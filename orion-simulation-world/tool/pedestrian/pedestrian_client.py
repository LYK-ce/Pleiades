#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# 行人 WS 客户端（task_35 §二 Step 4 / COMPONENTS.md §8.1）：
#   连 ws://127.0.0.1:9999 -> chassis_hello{kind:"pedestrian"} -> 等 chassis_ready
#   -> 以 20~50 Hz 持续发 motion_cmd{v, dir} -> 读世界 10 Hz 下发的 pose -> 退出时发 v=0。
#
# 三条硬边界：
#   ① 只发控制、只读位姿：不读地图、不连 P2P、不认识任何 Pleiades 概念。
#   ② `dir` 用**世界系弧度**（0 = +X 东，逆时针为正）。世界内部 rotation.y = -dir，
#      这个取反只在世界侧做，客户端**不要**再做一次（否则左右镜像）。
#   ③ 世界 300 ms 收不到 motion_cmd 就自动停车（只停不断连）：所以「持续发」是硬要求，
#      发送节拍用单调时钟驱动，落后超过 2 个周期就重置节拍而不是补发突发帧
#      （世界只取最新一条，突发只会掩盖 300 ms 超时）。
#
# 传输层二选一（`--engine auto|websockets|stdlib`）：
#   · websockets（首选，自动回 Pong、帧校验交给库）
#   · ws_min（标准库按 RFC6455 手写，websockets 未安装时的兜底；显式回 Pong）
#   任何回落都会**打印一行**，绝不静默。
#
# 本文件也可单独当冒烟工具：
#   python3 pedestrian_client.py --v 1.0 --dir 1.5707963 --duration 3 --peer-id human-01

import argparse
import json
import math
import socket
import sys
import time

# 同目录直接 import（脚本方式运行时 sys.path[0] 就是本目录；被 -m 调起来时兜一下）
_HERE = __file__.rsplit("/", 1)[0]
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from ws_min import MiniWebSocket, WsClosed  # noqa: E402  （本地模块，必须在 sys.path 之后）

# ---------------------------------------------------------------- 常量（协议事实）
DEFAULT_URL = "ws://127.0.0.1:9999"
DEFAULT_PEER_ID = "human-01"
CHASSIS_NAME = "pedestrian"
KIND_PEDESTRIAN = "pedestrian"

MSG_CHASSIS_HELLO = "chassis_hello"
MSG_CHASSIS_READY = "chassis_ready"
MSG_MOTION_CMD = "motion_cmd"
MSG_POSE = "pose"

## step.md：20~50 Hz 持续发送；取 30 Hz（世界 300 ms 超时 = 9 个周期，抖动余量足够）。
DEFAULT_SEND_HZ = 30.0
MIN_SEND_HZ = 20.0
MAX_SEND_HZ = 50.0

## step.md 硬要求：等 chassis_ready 最多 8 s。
READY_TIMEOUT_S = 8.0
## pose 超过这么久没更新 -> 视为「世界没在回传」（本文件只暴露给调用方判断，不自己抛）。
POSE_STALE_S = 0.5

## 退出码：0 正常；2 连接失败；3 等不到 chassis_ready；4 协议/传输异常；5 spawn 被拒；130 Ctrl-C。
EXIT_OK = 0
EXIT_CONNECT = 2
EXIT_READY = 3
EXIT_PROTOCOL = 4
EXIT_SPAWN = 5
EXIT_INTERRUPTED = 130
## 附加（本客户端自用）：--linger 停车判据不通过。
EXIT_VERIFY = 6

## spawn 比对容差（米 / 弧度）：世界回执会把值过一遍 float，这里只防浮点噪声。
SPAWN_XY_EPS = 1.0e-3
SPAWN_YAW_EPS = 1.0e-3


# ---------------------------------------------------------------- 异常
class ClientError(RuntimeError):
    """所有「可读失败」的基类：CLI 抓到它只打印 `[error] …` 并按其 exit_code 退出。"""

    exit_code = EXIT_PROTOCOL


class ConnectError(ClientError):
    """TCP / WS 握手失败（端口没开、世界没起、URL 写错）。"""

    exit_code = EXIT_CONNECT


class ReadyTimeout(ClientError):
    """8 s 内没等到 chassis_ready。"""

    exit_code = EXIT_READY


class ProtocolError(ClientError):
    """握手后行为不符合协议（kind 不是 pedestrian、世界主动关连接等）。"""

    exit_code = EXIT_PROTOCOL


class SpawnFallback(ClientError):
    """spawn 请求被世界回退到默认点（世界**不报错**，只 push_warning）。"""

    exit_code = EXIT_SPAWN

    def __init__(self, requested: dict, actual: dict):
        self.requested = dict(requested or {})
        self.actual = dict(actual or {})
        super().__init__(
            "spawn 被拒：请求 (%.2f, %.2f)，世界实际落在 (%.2f, %.2f)（见世界日志 WARNING）"
            % (float(self.requested.get("x", 0.0)), float(self.requested.get("y", 0.0)),
               float(self.actual.get("x", 0.0)), float(self.actual.get("y", 0.0))))


# ---------------------------------------------------------------- 传输层
class _WebsocketsTransport:
    """`websockets.sync.client` 的薄封装：超时返回 None，断链统一抛 WsClosed。"""

    def __init__(self, ws, version: str):
        self._ws = ws
        self.version = version

    def send_text(self, text: str) -> None:
        try:
            self._ws.send(text)
        except Exception as exc:            # ConnectionClosed / OSError …
            raise WsClosed(1006, "发送失败：%s" % exc) from exc

    def recv_text(self, timeout: float = 0.0):
        """timeout=0 = 非阻塞取一帧；到点没有就返回 None（不抛 TimeoutError）。"""
        deadline = time.monotonic() + max(float(timeout), 0.0)
        first = True
        while True:
            remain = deadline - time.monotonic()
            if not first and remain <= 0.0:
                return None
            first = False
            try:
                raw = self._ws.recv(timeout=remain if remain > 0.0 else 0.0)
            except TimeoutError:
                return None
            except Exception as exc:
                raise WsClosed(1006, "接收失败：%s" % exc) from exc
            if isinstance(raw, str):
                return raw
            # 二进制帧（带 manifest 的连接才有）：本 step 不消费，丢弃后继续等文本
            continue

    def close(self, code: int = 1000, reason: str = "") -> None:
        try:
            self._ws.close(code, reason)
        except Exception:
            pass


class _MiniTransport:
    """`ws_min.MiniWebSocket` 的薄封装（接口与上面完全一致）。"""

    def __init__(self, sock: MiniWebSocket):
        self._sock = sock

    def send_text(self, text: str) -> None:
        self._sock.send_text(text)

    def recv_text(self, timeout: float = 0.0):
        return self._sock.recv_text(timeout)

    def close(self, code: int = 1000, reason: str = "") -> None:
        self._sock.close(code, reason)


def _Has_Websockets() -> tuple:
    """-> (可用?, 版本号, 失败原因)。只探测 import，不建连接。"""
    try:
        import websockets
        from websockets.sync.client import connect  # noqa: F401
    except ImportError as exc:
        return False, "", str(exc)
    return True, getattr(websockets, "__version__", "?"), ""


def open_transport(url: str, engine: str = "auto",
                   open_timeout: float = READY_TIMEOUT_S) -> tuple:
    """建一条 WS 连接 -> (transport, engine_name)。

    engine="auto"：优先 websockets；不可用则回落标准库并**打印一行**说明。
    engine="websockets"：不可用直接抛 ConnectError（附 pip install 提示）。
    engine="stdlib"：直接用 ws_min。
    """
    if engine not in ("auto", "websockets", "stdlib"):
        raise ValueError("engine 只接受 auto / websockets / stdlib，收到 %r" % (engine,))
    if engine in ("auto", "websockets"):
        ok, version, why = _Has_Websockets()
        if not ok:
            if engine == "websockets":
                raise ConnectError(
                    "websockets 不可用（%s）：请 `pip install websockets`，"
                    "或改用 --engine stdlib（标准库 RFC6455 客户端）" % why)
            print("[warn] websockets 不可用（%s），改用标准库 RFC6455 客户端"
                  "（pip install websockets 可更稳）" % why)
        else:
            from websockets.sync.client import connect as ws_connect
            try:
                ws = ws_connect(url, open_timeout=open_timeout, max_size=None)
            except ConnectionRefusedError as exc:
                raise ConnectError("连接 %s 失败：Connection refused（世界在跑吗？端口对吗？）"
                                   % url) from exc
            except (TimeoutError, socket.timeout) as exc:
                raise ConnectError("连接 %s 超时（%.0f s）：世界起来了但没接握？"
                                   % (url, open_timeout)) from exc
            except OSError as exc:
                raise ConnectError("连接 %s 失败：%s" % (url, exc)) from exc
            except Exception as exc:             # InvalidHandshake / InvalidStatus …
                raise ConnectError("连接 %s 失败：%s" % (url, exc)) from exc
            print("[client] engine=websockets %s" % version)
            return _WebsocketsTransport(ws, version), "websockets"

    sock = MiniWebSocket(url, open_timeout=open_timeout)
    try:
        sock.connect()
    except ConnectionRefusedError as exc:
        raise ConnectError("连接 %s 失败：Connection refused（世界在跑吗？端口对吗？）"
                           % url) from exc
    except (TimeoutError, socket.timeout) as exc:
        raise ConnectError("连接 %s 超时（%.0f s）：世界起来了但没接握？"
                           % (url, open_timeout)) from exc
    except WsClosed as exc:
        raise ConnectError("连接 %s 失败：%s" % (url, exc)) from exc
    except OSError as exc:
        raise ConnectError("连接 %s 失败：%s" % (url, exc)) from exc
    print("[client] engine=stdlib (RFC6455 自研)")
    return _MiniTransport(sock), "stdlib"


# ---------------------------------------------------------------- 小工具
def parse_spawn_spec(text: str, yaw_degrees: bool = True) -> dict:
    """`"64,56"` / `"64,56,0"` / `"64,56,0,90"` -> {x, y, z, yaw(弧度)}。

    第 4 段 yaw 默认按**度**收（命令行更自然），yaw_degrees=False 时按弧度。
    段数不足用默认值补：y=缺省非法（必须给 x,y），z=0，yaw=0。
    """
    raw = [p.strip() for p in str(text).replace(";", ",").split(",") if p.strip()]
    if len(raw) < 2:
        raise ValueError("spawn 至少要给两个数（x,y），收到 %r" % (text,))
    if len(raw) > 4:
        raise ValueError("spawn 最多四段（x,y[,z[,yaw]]），收到 %r" % (text,))
    try:
        values = [float(v) for v in raw]
    except ValueError as exc:
        raise ValueError("spawn 里有不是数字的段：%r" % (text,)) from exc
    x, y = values[0], values[1]
    z = values[2] if len(values) >= 3 else 0.0
    yaw = values[3] if len(values) >= 4 else 0.0
    if yaw_degrees:
        yaw = math.radians(yaw)
    return {"x": x, "y": y, "z": z, "yaw": Wrap_Pi(yaw)}


def Wrap_Pi(angle: float) -> float:
    """归一到 [-PI, PI)。不用 `% tau` 再减：180° 边界会抖（plan R12）。"""
    return math.atan2(math.sin(angle), math.cos(angle))


# ---------------------------------------------------------------- 客户端
class PedestrianClient:
    """一条 pedestrian 连接的全生命周期：hello / ready / 持续 drive / pose / 优雅停。"""

    def __init__(self, url: str = DEFAULT_URL, peer_id: str = DEFAULT_PEER_ID,
                 spawn: dict = None, params: dict = None,
                 send_hz: float = DEFAULT_SEND_HZ, ready_timeout: float = READY_TIMEOUT_S,
                 engine: str = "auto", verbose: bool = False,
                 allow_spawn_fallback: bool = False):
        self.url = str(url)
        self.peer_id = str(peer_id)
        self.send_hz = float(send_hz)
        if not (MIN_SEND_HZ - 1e-9 <= self.send_hz <= MAX_SEND_HZ + 1e-9):
            raise ValueError("send_hz 必须在 %.0f~%.0f Hz（世界 300 ms 超时），收到 %.1f"
                             % (MIN_SEND_HZ, MAX_SEND_HZ, self.send_hz))
        self.ready_timeout = float(ready_timeout)
        self.engine = str(engine)
        self.verbose = bool(verbose)
        self.allow_spawn_fallback = bool(allow_spawn_fallback)

        # 只读（调用方看，不写）
        self.ready = False
        self.ready_msg = {}
        self.spawn = {}
        self.engine_name = ""
        self.poses_received = 0
        self.sent_commands = 0
        self.last_dir = None
        self.stopped = False

        self._spawn_req = dict(spawn) if spawn else None
        self._params = dict(params) if params else {}
        self._transport = None
        self._seq = 0
        self._v = 0.0
        self._dir = None
        self._stop_sent = False
        self._closed = False
        self._pose = None
        self._pose_mono = 0.0
        self._next_send = 0.0
        self._period = 1.0 / self.send_hz

    # ------------------------------------------------------------ 握手
    def connect(self) -> dict:
        """发 hello -> 等 chassis_ready（≤ ready_timeout）-> 校验 kind / spawn -> 打印一行摘要。"""
        self._transport, self.engine_name = open_transport(
            self.url, self.engine, self.ready_timeout)

        hello = {
            "msg": MSG_CHASSIS_HELLO,
            "peer_id": self.peer_id,
            "chassis": CHASSIS_NAME,
            "kind": KIND_PEDESTRIAN,       # 两个都写：世界按 kind 分派，chassis 查目录
            "params": dict(self._params),
            "manifest": [],                # 不需要任何传感器：世界立刻回 assembly_ready
        }
        if self._spawn_req is not None:
            hello["spawn"] = dict(self._spawn_req)
        try:
            self._transport.send_text(json.dumps(hello, ensure_ascii=False))
        except WsClosed as exc:
            raise ProtocolError("发送 chassis_hello 失败：%s" % exc) from exc

        deadline = time.monotonic() + self.ready_timeout
        ready = None
        while ready is None:
            remain = deadline - time.monotonic()
            if remain <= 0.0:
                break
            try:
                text = self._transport.recv_text(remain)
            except WsClosed as exc:
                raise ProtocolError("等 chassis_ready 时连接被关闭：%s" % exc) from exc
            if text is None:
                continue
            msg = self._parse(text)
            if msg is None:
                continue
            if msg.get("msg") == MSG_CHASSIS_READY:
                ready = msg
                break
            # 抢跑帧（pose / assembly_ready）先消化掉，别丢；它们不影响握手
            self._dispatch(msg)

        if ready is None:
            self._shutdown_transport()
            raise ReadyTimeout(
                "%.0f s 内没等到 chassis_ready：世界在跑吗？peer_id 是否被别的进程抢了？"
                "（--url %s）" % (self.ready_timeout, self.url))

        self.ready_msg = dict(ready)
        kind = str(ready.get("kind", "")).strip()
        if kind != KIND_PEDESTRIAN:
            self._shutdown_transport()
            raise ProtocolError(
                "世界把本连接当成 %r 处理（kind=%r），不是 pedestrian：chassis/kind 是否都写了 "
                "'pedestrian'？" % (kind or "robot", kind))

        actual = ready.get("spawn", {})
        if not isinstance(actual, dict):
            actual = {}
        self.spawn = {
            "x": float(actual.get("x", 0.0)), "y": float(actual.get("y", 0.0)),
            "z": float(actual.get("z", 0.0)), "yaw": float(actual.get("yaw", 0.0)),
        }

        if self._spawn_req is not None:
            want = {
                "x": float(self._spawn_req.get("x", 0.0)),
                "y": float(self._spawn_req.get("y", 0.0)),
                "z": float(self._spawn_req.get("z", 0.0)),
                "yaw": float(self._spawn_req.get("yaw", 0.0)),
            }
            mismatch = (abs(want["x"] - self.spawn["x"]) > SPAWN_XY_EPS
                        or abs(want["y"] - self.spawn["y"]) > SPAWN_XY_EPS
                        or abs(Wrap_Pi(want["yaw"] - self.spawn["yaw"])) > SPAWN_YAW_EPS)
            if mismatch:
                if self.allow_spawn_fallback:
                    print("[warn] spawn 被拒：请求 (%.2f, %.2f)，世界实际落在 (%.2f, %.2f)"
                          "（--allow-spawn-fallback，继续）"
                          % (want["x"], want["y"], self.spawn["x"], self.spawn["y"]))
                else:
                    self._shutdown_transport()
                    raise SpawnFallback(want, self.spawn)

        print("[client] ready peer='%s' kind=pedestrian spawn=(%.2f, %.2f) yaw=%.2f"
              % (self.peer_id, self.spawn["x"], self.spawn["y"], self.spawn["yaw"]))
        if self.verbose:
            print("[client] ready_msg=%s" % json.dumps(ready, ensure_ascii=False, sort_keys=True))

        self.ready = True
        self._next_send = time.monotonic()
        return dict(ready)

    # ------------------------------------------------------------ 控制
    def drive(self, v: float, dir_rad: float = None) -> None:
        """设定目标速度 / 朝向并**立即补发一帧**（转弯对阶跃立刻响应）。dir=None 表示保持朝向。"""
        self._v = float(v)
        if dir_rad is not None:
            self._dir = float(dir_rad)
            self.last_dir = self._dir
        self._send_motion(include_dir=True)
        self._next_send = time.monotonic() + self._period

    def stop(self) -> None:
        """发一次 `{v: 0}`（不带 dir：保持朝向）。幂等：重复调用只发一次、只打一行。"""
        if self._stop_sent or not self.ready or self._transport is None:
            return
        self._stop_sent = True
        self._v = 0.0
        self._seq += 1
        try:
            self._transport.send_text(json.dumps(
                {"msg": MSG_MOTION_CMD, "seq": self._seq, "v": 0.0},
                separators=(",", ":")))
            self.sent_commands += 1
            self.stopped = True
            print("[client] stop v=0")
        except WsClosed as exc:
            print("[warn] stop 发送失败（连接已断）：%s" % exc)

    # ------------------------------------------------------------ 驱动循环三件套
    def tick(self) -> None:
        """一帧：先收（pose 不积压）后发（到点才发，20~50 Hz）。"""
        self.pump(0.0)
        now = time.monotonic()
        if now < self._next_send:
            return
        if now - self._next_send > self._period * 2.0:
            # 落后两个周期以上（调度抖动 / 长阻塞）：重置节拍，**不补发突发帧**
            self._next_send = now + self._period
        else:
            self._next_send += self._period
        self._send_motion(include_dir=True)

    def pump(self, timeout: float = 0.0) -> int:
        """收帧并分派，返回本轮处理帧数。timeout=0 = 只看已到达的（不阻塞）。"""
        if self._transport is None:
            return 0
        count = 0
        deadline = time.monotonic() + max(float(timeout), 0.0)
        while True:
            remain = max(deadline - time.monotonic(), 0.0)
            try:
                text = self._transport.recv_text(remain)
            except WsClosed as exc:
                # 世界关连接：交给调用方决定（patrol 会收尾退出；这里不吞成 0 帧无限转）
                raise ProtocolError("连接被世界关闭：%s" % exc) from exc
            if text is None:
                break
            msg = self._parse(text)
            if msg is None:
                continue
            self._dispatch(msg)
            count += 1
            if count >= 1000:            # 防一次 pump 被对端刷爆后卡死
                break
        return count

    def sleep_to_next(self) -> float:
        """睡到下一个发送节拍（最多睡一个周期），返回实际睡了秒数。"""
        delay = min(self._next_send - time.monotonic(), self._period)
        if delay <= 0.0:
            return 0.0
        time.sleep(delay)
        return delay

    # ------------------------------------------------------------ 位姿
    def pose(self) -> dict:
        """最近一条 pose 的**副本**；还没收到过 = None。"""
        return dict(self._pose) if self._pose is not None else None

    def pose_age(self):
        """距最近一条 pose 的秒数；没收到过 = None。"""
        if self._pose is None:
            return None
        return time.monotonic() - self._pose_mono

    def pose_stamp(self) -> float:
        """最近一条 pose 的到达时刻（monotonic），配合 poses_received 判「是不是新的一条」。"""
        return self._pose_mono

    # ------------------------------------------------------------ 收尾
    def close(self, quiet: bool = False) -> None:
        """停止 -> 关连接。幂等；收尾路径不许抛。"""
        if self._closed:
            return
        self._closed = True
        try:
            self.stop()
        except Exception:                  # 收尾时任何异常都只当噪音
            pass
        self._shutdown_transport()
        if not quiet:
            print("[client] close")

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, exc_type, exc, tb):
        self.close(quiet=exc_type is not None)
        return False

    # ------------------------------------------------------------ 私有
    def _send_motion(self, include_dir: bool = True) -> None:
        if self._transport is None:
            raise ProtocolError("还没 connect() 就 drive()")
        self._seq += 1
        msg = {"msg": MSG_MOTION_CMD, "seq": self._seq, "v": self._v}
        if include_dir and self._dir is not None:
            msg["dir"] = self._dir
        self._transport.send_text(json.dumps(msg, separators=(",", ":")))
        self.sent_commands += 1
        if self.verbose:
            print("[client] send#%d v=%.3f dir=%s" % (
                self._seq, self._v,
                "None" if msg.get("dir") is None else "%.4f" % float(msg["dir"])))

    def _dispatch(self, msg: dict) -> None:
        name = str(msg.get("msg", ""))
        if name == MSG_POSE:
            self._pose = dict(msg)
            self._pose_mono = time.monotonic()
            self.poses_received += 1
            return
        if name == MSG_CHASSIS_READY:
            self.ready_msg = dict(msg)
            return
        # assembly_ready / sensor_ready / 未来新增消息：**不抛**，verbose 才打一行
        if self.verbose:
            print("[client] recv msg='%s'（忽略）" % name)

    @staticmethod
    def _parse(text: str):
        try:
            parsed = json.loads(text)
        except (ValueError, TypeError):
            return None
        return parsed if isinstance(parsed, dict) else None

    def _shutdown_transport(self) -> None:
        if self._transport is not None:
            try:
                self._transport.close(1000, "")
            except Exception:
                pass
            self._transport = None


# ---------------------------------------------------------------- CLI（冒烟工具）
def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="行人 WS 客户端冒烟工具：连世界 -> hello -> 持续 motion_cmd{v,dir} -> 打印 pose")
    parser.add_argument("--v", type=float, default=0.0, help="线速度 m/s（世界 clamp 0~1.26）")
    parser.add_argument("--dir", type=float, default=None,
                        help="世界系朝向角**弧度**（0=+X，逆时针为正）；不给 = 保持朝向")
    parser.add_argument("--duration", type=float, default=3.0, help="驱动时长（秒，从 ready 起算）")
    parser.add_argument("--url", default=DEFAULT_URL)
    parser.add_argument("--peer-id", default=DEFAULT_PEER_ID)
    parser.add_argument("--spawn", default=None, help='出生点 "x,y[,z[,yaw_度]]"；缺省用世界默认点')
    parser.add_argument("--send-hz", type=float, default=DEFAULT_SEND_HZ)
    parser.add_argument("--ready-timeout", type=float, default=READY_TIMEOUT_S)
    parser.add_argument("--engine", default="auto", choices=("auto", "websockets", "stdlib"))
    parser.add_argument("--allow-spawn-fallback", action="store_true",
                        help="spawn 被世界回退时降级为 WARNING 继续跑（默认 exit 5）")
    parser.add_argument("-v", "--verbose", action="store_true", help="打印每条收发")
    return parser


def main(argv=None) -> int:
    args = _build_parser().parse_args(argv)
    spawn = None
    if args.spawn:
        try:
            spawn = parse_spawn_spec(args.spawn)
        except ValueError as exc:
            print("[error] %s" % exc)
            return EXIT_PROTOCOL
    try:
        client = PedestrianClient(url=args.url, peer_id=args.peer_id, spawn=spawn,
                                  send_hz=args.send_hz, ready_timeout=args.ready_timeout,
                                  engine=args.engine, verbose=args.verbose,
                                  allow_spawn_fallback=args.allow_spawn_fallback)
    except ValueError as exc:
        print("[error] %s" % exc)
        return EXIT_PROTOCOL

    try:
        client.connect()
    except SpawnFallback as exc:
        print("[error] %s" % exc)
        return EXIT_SPAWN
    except ClientError as exc:
        print("[error] %s" % exc)
        return exc.exit_code

    t0 = time.monotonic()
    next_print = t0 + 1.0
    interrupt = False
    exit_code = EXIT_OK
    try:
        while True:
            now = time.monotonic()
            if now - t0 >= args.duration:
                break
            client.drive(args.v, args.dir)
            client.tick()
            if now >= next_print:
                next_print += 1.0
                pose = client.pose()
                if pose is None:
                    print("[pose] t=%.1fs（还没有 pose：世界在跑吗？）" % (now - t0))
                else:
                    print("[pose] t=%.1fs pos=(%.2f, %.2f) yaw=%.1f° vx=%.2f vy=%.2f"
                          % (now - t0, float(pose.get("x", 0.0)), float(pose.get("y", 0.0)),
                             math.degrees(float(pose.get("yaw", 0.0))),
                             float(pose.get("vx", 0.0)), float(pose.get("vy", 0.0))))
            client.sleep_to_next()
    except ProtocolError as exc:
        print("[error] %s" % exc)
        exit_code = EXIT_PROTOCOL
    except KeyboardInterrupt:
        print("[warn] Ctrl-C：先发 v=0 再退出")
        interrupt = True
        exit_code = EXIT_INTERRUPTED
    finally:
        client.close()

    pose = client.pose()
    if pose is not None:
        print("[client] done: duration=%.1fs poses=%d cmds=%d last_pos=(%.2f, %.2f) yaw=%.1f°"
              % (time.monotonic() - t0, client.poses_received, client.sent_commands,
                 float(pose.get("x", 0.0)), float(pose.get("y", 0.0)),
                 math.degrees(float(pose.get("yaw", 0.0)))))
    else:
        print("[client] done: duration=%.1fs poses=0 cmds=%d（一条 pose 都没收到）"
              % (time.monotonic() - t0, client.sent_commands))
    return EXIT_INTERRUPTED if interrupt else exit_code


if __name__ == "__main__":
    raise SystemExit(main())
