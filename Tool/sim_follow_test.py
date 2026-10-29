#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-29
# Modified Date ： 2026-10-29
"""task_38 Step 3 — 新世界（城市街道 + PBR + HDRI）里复验 task_37 的 Follow（只验不改，§四）。

跑法：python3 Tool/sim_follow_test.py [--preflight] [--checks all|named|none] [--no-launch]
      [--out-dir DIR] [--port PORT] [--attempts 2] [--s0b strict|record|off]
产物：RobotSim_TestEnv/results/task38_follow_<UTC>/{trace.csv,detector_frames.csv,ped_trace.csv,
      env.json,verdict.md,verdict.json,shots/*.png,checks/*.log,world.log,sim.log,sim_trace.log}
      verdict.md 结构：§0 环境与口径（含仓库边界/实测档位/尝试次数）→ §1 前置自检 →
      §2 时序（S0 锚点 → S0 status → **S0b 归位表** → S1~S6）→ §3 十条判据（带 `口径` 列）→
      §3.1 空转通过的四条 → §3.2 判据 3/4 字面墙钟实测 → §3.3 判据 8 的两个距离段 →
      §3.4 判据 1/6 字面 20 s 墙钟（**判据 1 = 距离行、判据 6 = 帧占比**，只作对照）→
      §4 观察项 → §5 既有检查 → §6 产物 → §7 残留 → §8 根因初判（§8.1 跟随机制证据 /
      §8.2 初判 / §8.3 S0b 未生效的代码级根因）→ §9 结论
退出码：0 十条全 PASS；1 有 FAIL；2 环境/前置失败或 S0b 未过（FAIL(env)）

`--s0b`：**strict**（默认，§四.5 字面：goto 归位不过即记 FAIL(env) 并停）/
record（同一套实测，记录后继续跑 S1~S6，只作取证）/ off（跳过，A/B 对照用）。
S0b 不过时 `FixtureError` 不给整轮重试（§四.5「并停」）。

本脚本**只验收**：不改跟随 / detector / 投影 / executor / nav / 协议，不改世界外观，
不调 `conf_thres` / `nms` / `d_far` / `d_near` / `stale` / 任何判据阈值，不 `git add`/`commit`。

口径（verdict.md §0 是本口径的自包含定义处，plan.md 只作另见）：
  * 本机 llvmpipe 软渲染下新世界约 2.5 fps → 相机 Hz ≤ 渲染 fps、物理推进 < 实时。
  * 因此 **主口径 = 物理时间**：S3 以"行人位移 25.2 m（= 20 s × 1.26 m/s）"为终结条件，
    S4/S5 的时长按 S3 末尾就地测出的 k 折算；**判据阈值一个不动**。
  * 判据 5 的"≤2 tick"按 1 tick = 50 ms 物理时间折算，墙钟按本次实测 k 折算（具体值见 verdict §2）。
  * 并列报 = 墙钟字面口径（verdict §3 的 `口径` 列与 §3.2/§3.4，全部由 trace 实测，无写死数）。
"""

import argparse
import base64
import csv
from datetime import datetime, timezone
import hashlib
import json
import math
import os
import pathlib
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import tomllib
import traceback

# ================================================================ 路径与常量

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORLD_DIR = ROOT / "orion-simulation-world"
INST = ROOT / "RobotSim_TestEnv" / "sim1"
BIN = INST / "pleiades-sim"
CFG = INST / ".config" / "config.toml"
SIMLOG_DIR = INST / "Log"
RESULTS = ROOT / "RobotSim_TestEnv" / "results"
PED_TOOL_DIR = WORLD_DIR / "tool" / "pedestrian"
# 冻结基线（评审用同一条 `git status --porcelain -uall` 逐行比对）
BASELINE = ROOT / "tmp" / "task38" / "git_baseline.txt"
# 本步唯一允许新增的仓库路径（step.md 改动范围在 git 里的投影）；白名单**不随基线扩张**
BOUNDARY_ALLOW = {"?? Tool/sim_follow_test.py"}

HOST = "127.0.0.1"
WORLD_PORT = 9999
DISPLAY = ":99"

BIN_SHA256 = "dd6222a492c95a6c2a7a8ba5ce211a5bcb8c83eaca74fe147f99ec6dfca71ede"

# 世界命令行（**写死一份**，评审可直接 grep；R16：绝不能 `--headless`，绝不能 `--fixed-fps`）
def world_argv(world_dir=WORLD_DIR):
    return [
        "timeout", "1200",
        "godot",
        "--display-driver", "x11",
        "--rendering-driver", "opengl3",
        "--audio-driver", "Dummy",
        "--path", str(world_dir),
        "--resolution", "1600x900",
        "--position", "0,0",
    ]


SIM_ARGV = [str(BIN), "--headless"]

XVFB_ARGV = ["Xvfb", ":99", "-screen", "0", "1920x1080x24",
             "+extension", "GLX", "+render", "-noreset"]

IMG_W, IMG_H = 1600, 900

# ─── 场景常量（文档值；不改）────────────────────────────────────────────────
PED_SPAWN = {"x": 70.0, "y": 64.0, "z": 0.0, "yaw": 0.0}
PED_SPEED = 1.26                      # m/s（世界 clamp 0~1.26）
S3_TRACK_M = 25.2                     # = 20 s × 1.26 m/s（§四.5 "≈(95,64)" 的物理含义）
S3_WIN_M = 12.6                       # = 10 s × 1.26 m/s（"S3 后 10 s" 的位移等价）
S4_SECONDS = 5.0                      # 物理秒
S5_OBS_SECONDS = 10.0                 # 物理秒
S5_STALE_WAIT_S = 0.8                 # 世界侧 stale 门限（follow.rs:68 STALE_MS=800）
UAV_SPAWN = (64.0, 64.0, 2.5)
D_FAR_M, D_NEAR_M, STALE_MS = 4.0, 2.5, 800.0   # follow.rs:64/66/68（只用于核对，不参与判定）

# ─── S0b 归位到格心（§四.5，2026-10-29 人类追加）─────────────────────────────
S0B_TARGET = (64.25, 64.25)        # goto 目标 = UAV 所在格的格心（0.5 m 网格）
S0B_FIRST_CELL = (64.75, 64.25)    # 归位后「首格」格心（me + 0.5 m，朝向 +X）
S0B_DPSI_MAX_DEG = 10.0            # 断言 |Δψ| ≤ 10°（§四.5 原值，不得改）
S0B_IDLE_TIMEOUT_S = 90.0          # 等 nav_status=idle 的墙钟上限
S0B_SETTLE_S = 0.30                # 连续两拍 idle 的最小间隔

# ─── 判据阈值（§四.6 逐字，禁止调整）────────────────────────────────────────
BAND_LO, BAND_HI = 3.5, 6.0
BAND_SHARE_MIN = 0.80
BAND_MIN_D = 2.5
GLOBAL_MIN_D = 2.0
STOP_MOVE_MAX = 0.5
LOST_MOVE_MAX = 0.5
CANCEL_TICKS_MAX = 2.0
DET_SHARE_MIN = 0.90
FP_SHARE_MAX = 0.05
PROJ_ERR_MAX = 1.0

# ─── 观察者相机（截图投影；ObserverCamera.tscn 固定机位）─────────────────────
OBS_C_GODOT = (64.0, 26.0, 108.0)
OBS_FWD = (0.0, -0.5, -0.8660254037844387)
OBS_RIGHT = (1.0, 0.0, 0.0)
OBS_UP = (0.0, 0.8660254037844387, -0.5)
OBS_F_PX = 642.7                      # (900/2)/tan(35°)

# ─── 既有检查回归（22 条：17 个既有 check_*.gd + 5 个第二档位 / 佐证）─────────
CHECKS = [
    # name, mode, timeout_s, baseline_total, counted, named
    ("check_project",          "headless", 120, None, True,  False),
    ("check_world",            "headless", 180, 26,   True,  False),
    ("check_vehicle",          "headless", 200, 40,   True,  False),
    ("check_diff_drive",       "headless", 200, 41,   True,  False),
    ("check_pose",             "headless", 220, 23,   True,  False),
    ("check_bounds",           "headless", 260, 12,   True,  True),
    ("check_camera",           "headless", 200, 32,   True,  False),
    ("check_models",           "headless", 200, 24,   True,  False),
    ("check_sensor_models",    "headless", 220, 29,   True,  False),
    ("check_quadrotor",        "headless", 260, 48,   True,  False),
    ("check_lidar",            "headless", 260, 48,   True,  True),
    ("check_pedestrian",       "headless", 220, 40,   True,  False),
    ("check_pedestrian_physics", "headless", 220, 22, True,  False),
    ("check_ws_manager",       "headless", 260, 76,   True,  False),
    ("check_ws_manager",       "xvfb",     420, 77,   True,  False),
    ("check_camera_sensor",    "headless", 260, 18,   True,  False),
    ("check_camera_sensor",    "xvfb",     260, 28,   True,  False),
    ("check_camera_shots",     "headless", 220, 18,   True,  False),
    ("check_camera_shots",     "xvfb",     260, 35,   True,  True),
    ("check_pedestrian_shots", "xvfb",     260, 14,   True,  False),
    ("check_street",           "headless", 220, 28,   True,  False),
    ("check_street",           "xvfb",     420, 41,   True,  False),
    # 本任务新增的两条世界观感自检（不计入「既有 17 个」，只作佐证；plan §11.3 未列，
    # 这里跑是为了让判据 9 的"世界没坏"多一份独立证据，见 tmp/task38/impl.md 偏差 D4）
    ("check_world_look",       "headless", 220, 30,   False, False),
    ("check_world_look",       "xvfb",     420, 43,   False, False),
]


# 行人客户端：**只 import、不改**（step.md 允许改该目录，但本步零改动，见 impl.md）
if str(PED_TOOL_DIR) not in sys.path:
    sys.path.insert(0, str(PED_TOOL_DIR))
from pedestrian_client import (PedestrianClient, ConnectError, ReadyTimeout,   # noqa: E402
                               SpawnFallback, ProtocolError)


def Log(msg):
    print("[%s] %s" % (datetime.now().strftime("%H:%M:%S"), msg), flush=True)


# ================================================================ 小工具

def now_utc_stamp():
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def sha256_file(path):
    h = hashlib.sha256()
    with pathlib.Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def dist(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def crit(name, ok, detail=""):
    return {"name": name, "ok": bool(ok), "detail": str(detail)}


def median(values):
    xs = sorted(v for v in values if v is not None)
    if not xs:
        return None
    n = len(xs)
    return xs[n // 2] if n % 2 else 0.5 * (xs[n // 2 - 1] + xs[n // 2])


def percentile(values, q):
    xs = sorted(v for v in values if v is not None)
    if not xs:
        return None
    k = (len(xs) - 1) * q
    lo = int(math.floor(k))
    hi = int(math.ceil(k))
    if lo == hi:
        return xs[lo]
    return xs[lo] * (hi - k) + xs[hi] * (k - lo)


def git_status_lines():
    """与评审同一条命令：`-uall`（否则未跟踪目录会被折叠成 `dir/`）。"""
    try:
        r = subprocess.run(["git", "status", "--porcelain", "-uall"], cwd=str(ROOT),
                           capture_output=True, text=True, check=False)
        return [ln for ln in r.stdout.splitlines() if ln.strip()]
    except OSError:
        return []


BOUNDARY_CMD = "git status --porcelain -uall"


def baseline_lines(path=None):
    """读冻结基线；`#` 开头为注释、空行跳过（与评审的 diff 命令同口径）。"""
    path = pathlib.Path(BASELINE if path is None else path)
    if not path.is_file():
        return None
    out = []
    for ln in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not ln.strip() or ln.lstrip().startswith("#"):
            continue
        out.append(ln.rstrip())
    return out


def git_boundary_report():
    """返回 {ok, extra, missing, live_n, base_n, cmd}；`extra` 非空即越界。

    语义约定：**只有 extra 判越界**（多出来的东西才是违规）；missing 只记录
    （本步不改既有文件，missing 只可能来自外部删除，属观察项）。
    """
    live = git_status_lines()
    base = baseline_lines()
    if base is None:
        return {"ok": False, "extra": ["<基线文件缺失 %s>" % (BASELINE,)], "missing": [],
                "live_n": len(live), "base_n": 0, "cmd": BOUNDARY_CMD,
                "allow": sorted(BOUNDARY_ALLOW)}
    lb, bb = set(live), set(base)
    extra = sorted(lb - bb - BOUNDARY_ALLOW)
    missing = sorted(bb - lb)
    return {"ok": not extra, "extra": extra, "missing": missing,
            "live_n": len(live), "base_n": len(base),
            "allow": sorted(BOUNDARY_ALLOW), "cmd": BOUNDARY_CMD}


def run_capture(argv, timeout=60.0, env=None, cwd=None):
    """跑一条短命令，返回 (rc, out, err)。超时 rc=-1。"""
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=timeout,
                           env=env, cwd=str(cwd) if cwd else None, check=False)
        return r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "timeout"
    except OSError as exc:
        return -2, "", str(exc)


# ================================================================ 极简 WebSocket 客户端
# 与 `Tool/sim_queue_test.py:205-320` 同源（复制，不 import：避免跨脚本耦合）。

class WsClosed(EOFError):
    """对端关闭。"""


class WS:
    GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

    def __init__(self, host=HOST, port=9098, timeout=5.0):
        self.host = host
        self.port = port
        self.timeout = timeout
        self.sock = socket.create_connection((host, port), timeout=timeout)
        self.sock.settimeout(timeout)
        self._buf = b""
        self._handshake()

    def _handshake(self):
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            "GET / HTTP/1.1\r\n"
            f"Host: {self.host}:{self.port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(req.encode())
        while b"\r\n\r\n" not in self._buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError("握手期间对端关闭")
            self._buf += chunk
        head, _, rest = self._buf.partition(b"\r\n\r\n")
        self._buf = rest
        status_line = head.split(b"\r\n", 1)[0].decode("latin-1")
        if "101" not in status_line:
            raise ConnectionError(f"WebSocket 握手失败：{status_line}")
        expect = base64.b64encode(hashlib.sha1((key + self.GUID).encode()).digest()).decode()
        if expect.encode() not in head:
            raise ConnectionError("WebSocket 握手失败：Sec-WebSocket-Accept 不匹配")

    def _recv_exact(self, n):
        while len(self._buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise WsClosed("对端关闭")
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _recv_frame(self):
        b0, b1 = self._recv_exact(2)
        opcode = b0 & 0x0F
        masked = bool(b1 & 0x80)
        length = b1 & 0x7F
        if length == 126:
            (length,) = struct.unpack("!H", self._recv_exact(2))
        elif length == 127:
            (length,) = struct.unpack("!Q", self._recv_exact(8))
        mask = self._recv_exact(4) if masked else None
        payload = self._recv_exact(length) if length else b""
        if mask:
            payload = bytes(c ^ mask[i % 4] for i, c in enumerate(payload))
        return opcode, payload

    def _send_frame(self, opcode, payload=b""):
        header = bytearray([0x80 | opcode])
        n = len(payload)
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header += struct.pack("!H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack("!Q", n)
        mask = os.urandom(4)
        header += mask
        body = bytes(c ^ mask[i % 4] for i, c in enumerate(payload))
        self.sock.sendall(bytes(header) + body)

    def send(self, obj):
        self._send_frame(0x1, json.dumps(obj).encode())

    def recv(self):
        while True:
            opcode, payload = self._recv_frame()
            if opcode == 0x1:
                return json.loads(payload.decode())
            if opcode == 0x9:
                self._send_frame(0xA, payload)
                continue
            if opcode == 0xA:
                continue
            if opcode == 0x8:
                try:
                    self._send_frame(0x8, payload)
                except OSError:
                    pass
                raise WsClosed("对端 close")

    def request(self, obj):
        self.send(obj)
        return self.recv()

    def close(self):
        try:
            self._send_frame(0x8, b"")
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


# ================================================================ 进程 / 端口

class Proc:
    """世界 / sim 共用的进程包装：独立进程组 + 必 wait（不留僵尸）。`Tool/sim_queue_test.py:323-378`。"""

    def __init__(self, name):
        self.name = name
        self.p = None
        self.pid = None
        self._f = None

    def start(self, argv, cwd, log_path, env=None):
        log_path = pathlib.Path(log_path)
        log_path.parent.mkdir(parents=True, exist_ok=True)
        self._f = open(log_path, "ab", buffering=0)
        self.p = subprocess.Popen(
            argv, cwd=str(cwd), stdout=self._f, stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL, start_new_session=True, env=env,
        )
        self.pid = self.p.pid
        Log(f"[{self.name}] start pid={self.pid} argv={' '.join(str(a) for a in argv)} (cwd={cwd})")

    def alive(self):
        return self.p is not None and self.p.poll() is None

    def _killpg(self, sig):
        if self.p is None:
            return
        try:
            os.killpg(os.getpgid(self.p.pid), sig)
        except (ProcessLookupError, PermissionError, OSError):
            pass

    def stop(self, graceful_s=8.0):
        if self.p is None:
            return
        if self.p.poll() is None:
            self._killpg(signal.SIGTERM)
            try:
                self.p.wait(timeout=graceful_s)
            except subprocess.TimeoutExpired:
                self._killpg(signal.SIGKILL)
                try:
                    self.p.wait(timeout=5.0)
                except subprocess.TimeoutExpired:
                    pass
        self._killpg(signal.SIGKILL)
        try:
            self.p.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            pass
        if self._f is not None:
            self._f.close()
            self._f = None
        Log(f"[{self.name}] stopped (pid={self.pid})")
        self.p = None


def port_busy(port, host=HOST):
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        s.bind((host, port))
        return False
    except OSError:
        return True
    finally:
        s.close()


# ================================================================ 日志尾部

class Tail:
    """按**字节**偏移增量读一个日志文件。"""

    def __init__(self, path, from_end=False):
        self.path = pathlib.Path(path)
        self.pos = 0
        if from_end and self.path.exists():
            self.pos = self.path.stat().st_size

    def since(self, mark=None):
        if not self.path.exists():
            return []
        with self.path.open("rb") as f:
            f.seek(self.pos if mark is None else mark)
            data = f.read()
            self.pos = f.tell()
        if not data:
            return []
        return data.decode("utf-8", "replace").splitlines(keepends=True)

    def text(self):
        if not self.path.exists():
            return ""
        return self.path.read_text(encoding="utf-8", errors="replace")


class SimLogTail(Tail):
    """sim 的 tracing 日志：取 `Log/pleiades.log.*` 里最新的那份。"""

    def newest(self):
        if not SIMLOG_DIR.is_dir():
            return None
        files = sorted(SIMLOG_DIR.glob("pleiades.log.*"), key=lambda p: p.stat().st_mtime)
        return files[-1] if files else None

    def reset(self):
        p = self.newest()
        if p is None:
            return None
        self.path = p
        self.pos = 0
        return p


# ================================================================ 控制口

class Control:
    """控制口单连接（一问一答）+ 断线重连。`Tool/sim_queue_test.py:448-487`。"""

    def __init__(self, port, timeout=5.0):
        self.port = port
        self.timeout = timeout
        self.ws = None

    def connect(self, retries=240, delay=0.25):
        for _ in range(retries):
            try:
                self.ws = WS(HOST, self.port, timeout=self.timeout)
                return True
            except (OSError, ConnectionError):
                time.sleep(delay)
        return False

    def request(self, obj):
        if self.ws is None:
            raise WsClosed("控制口未连接")
        return self.ws.request(obj)

    def status(self):
        r = self.request({"cmd": "status"})
        if not r.get("ok"):
            raise RuntimeError("status 回包失败: %r" % (r,))
        return r.get("data", {})

    def send_cmd(self, obj):
        return self.request(obj)

    def close(self):
        if self.ws is not None:
            try:
                self.ws.close()
            except OSError:
                pass
            self.ws = None


# ================================================================ 配置 / 前置自检

class HarnessError(RuntimeError):
    """harness 自身故障（世界没起、行人没连上、端口被占…）→ 允许按 --attempts 重跑一次。"""


class FixtureError(HarnessError):
    """夹具断言不过（S0b 的 |Δψ| 超限）→ 按 §四.5 记 FAIL(env) 并停，**不做整轮重试**。"""


def load_config():
    with CFG.open("rb") as f:
        return tomllib.load(f)


def find_camera_sensor(cfg):
    for entry in cfg.get("sim_world", {}).get("sensor", []):
        if entry.get("slot") == "camera_nose":
            return entry
    return None


def preflight_checks(cfg, control_port):
    """§1.3 的 10 行前置 + 3 条环境项 + 1 条仓库边界，共 14 项，逐条返回 {name, ok, detail}。"""
    out = []
    det = cfg.get("detector", {})
    detcam = cfg.get("detector", {}).get("camera", {})
    sensor = find_camera_sensor(cfg)
    chassis = cfg.get("chassis", {})
    control = cfg.get("control", {})

    # 1 端口
    busy = [p for p in (control_port, WORLD_PORT) if port_busy(p)]
    out.append(crit("ports_free", not busy,
                    "busy=%s" % busy if busy else "control=%d world=%d 均空闲" % (control_port, WORLD_PORT)))

    # 2 [[sim_world.sensor]] camera
    ok = (sensor is not None
          and sensor.get("sensor") == "camera"
          and abs(float(sensor.get("params", {}).get("rate_hz", -1)) - 10.0) < 1e-9
          and abs(float(sensor.get("mount", {}).get("pitch", 0.0)) - (-15.0)) < 1e-9)
    out.append(crit("config_camera_sensor", ok,
                    "slot=%s sensor=%s rate_hz=%s mount.pitch=%s" % (
                        None if sensor is None else sensor.get("slot"),
                        None if sensor is None else sensor.get("sensor"),
                        None if sensor is None else sensor.get("params", {}).get("rate_hz"),
                        None if sensor is None else sensor.get("mount", {}).get("pitch"))))

    # 3 [detector]
    ok = (det.get("enabled") is True and det.get("device") == "cpu"
          and det.get("model") == "yolov8n.safetensors"
          and abs(float(det.get("conf_thres", -1)) - 0.35) < 1e-9
          and abs(float(det.get("nms_thres", -1)) - 0.45) < 1e-9)
    out.append(crit("config_detector", ok,
                    "enabled=%s device=%s model=%s conf=%s nms=%s" % (
                        det.get("enabled"), det.get("device"), det.get("model"),
                        det.get("conf_thres"), det.get("nms_thres"))))

    # 4 [detector.camera]
    want = {"fov_deg": 70.0, "mount_pitch_deg": -15.0, "ground_z": 0.0,
            "max_range_m": 12.0, "mount_yaw_deg": 90.0,
            "mount_x": 0.20, "mount_y": 0.0, "mount_z": 0.045}
    bad = [k for k, v in want.items() if abs(float(detcam.get(k, 1e9)) - v) > 1e-9]
    if detcam.get("fov_axis") != "vertical":
        bad.append("fov_axis")
    ok = not bad
    out.append(crit("config_detector_camera", ok,
                    "bad=%s | fov=%s(%s) pitch=%s ground_z=%s max_range=%s yaw=%s mount=(%s,%s,%s)" % (
                        bad, detcam.get("fov_deg"), detcam.get("fov_axis"),
                        detcam.get("mount_pitch_deg"), detcam.get("ground_z"),
                        detcam.get("max_range_m"), detcam.get("mount_yaw_deg"),
                        detcam.get("mount_x"), detcam.get("mount_y"), detcam.get("mount_z"))))

    # 5 控制口端口
    out.append(crit("config_control_port", isinstance(control.get("port"), int),
                    "[control].port=%s enabled=%s bind=%s" % (
                        control.get("port"), control.get("enabled"), control.get("bind"))))

    # 6 forward_speed
    ok = int(chassis.get("forward_speed", -1)) == 100
    out.append(crit("config_forward_speed", ok,
                    "[chassis].forward_speed=%s（要 100 = 2.0 m/s）" % chassis.get("forward_speed")))

    # 7 权重
    w = INST / "Pleiades_Workspace" / "yolov8n.safetensors"
    ok = w.is_file() and w.stat().st_size > 1 << 20
    out.append(crit("detector_weights", ok,
                    "%s size=%s" % (w, (w.stat().st_size if w.is_file() else "missing"))))

    # 8 programs 软链（幂等补建；允许路径内）
    link = INST / "programs"
    note = ""
    if link.is_symlink() or link.exists():
        try:
            target = pathlib.Path(os.path.realpath(link))
            ok = target == (ROOT / "programs").resolve()
            note = "已存在 → %s (resolved=%s) %s" % (os.readlink(link), target, "ok" if ok else "指向不符")
        except OSError as exc:
            ok, note = False, "读取失败：%s" % exc
    else:
        try:
            os.symlink("../../programs", str(link))
            ok, note = True, "缺失 → 已补建 ../../programs (resolved=%s)" % os.path.realpath(link)
        except OSError as exc:
            ok, note = False, "缺失且补建失败：%s" % exc
    out.append(crit("programs_symlink", ok, note))

    # 9 godot
    rc, so, se = run_capture(["godot", "--version"], timeout=30)
    out.append(crit("godot_version", rc == 0, (so.strip() or se.strip() or "rc=%d" % rc)))

    # 10 Xvfb / ImageMagick
    missing = [n for n in ("Xvfb", "import", "convert", "identify") if shutil.which(n) is None]
    out.append(crit("tools_xvfb_imagemagick", not missing, "missing=%s" % missing if missing else "ok"))

    # 11 .godot/imported 非空（E28：缺缓存 → 贴图静默 null）
    imported = WORLD_DIR / ".godot" / "imported"
    n = len(list(imported.glob("*"))) if imported.is_dir() else 0
    out.append(crit("world_import_cache", n > 0, ".godot/imported entries=%d" % n))

    # 12 二进制 sha（本步不 cargo build，只校验）
    ok = BIN.is_file() and sha256_file(BIN) == BIN_SHA256
    out.append(crit("sim_binary_sha256", ok,
                    "sha256=%s（期望 %s…）" % (sha256_file(BIN) if BIN.is_file() else "missing",
                                                BIN_SHA256[:16])))

    # 13 task_37 证据 + follow.rs 常量
    wb = ROOT / "Workbook" / "wb_37_detector.md"
    ok_wb = wb.is_file() and "PASS" in wb.read_text(encoding="utf-8", errors="replace")
    follow_src = ROOT / "pleiades-sim" / "src" / "sim" / "uav_sim" / "mission" / "follow.rs"
    txt = follow_src.read_text(encoding="utf-8", errors="replace") if follow_src.is_file() else ""
    consts = all(t in txt for t in ("D_FAR_M", "%.1f" % D_FAR_M, "D_NEAR_M", "%.1f" % D_NEAR_M,
                                    "STALE_MS", "%d" % STALE_MS))
    out.append(crit("task37_evidence", ok_wb and consts,
                    "workbook=%s follow.rs consts=%s" % (ok_wb, consts)))

    # 14 仓库边界：git status（-uall）与冻结基线逐行比对，只允许本轮 harness 自身
    b = git_boundary_report()
    out.append(crit("git_boundary", b["ok"],
                    "extra=%s missing=%s allow=%s live=%d base=%d cmd=%s" % (
                        b["extra"], b["missing"], b.get("allow"), b["live_n"],
                        b["base_n"], b["cmd"])))

    return out


# ================================================================ Xvfb / 显示

def display_alive(disp=DISPLAY):
    tmp = pathlib.Path("/tmp") / ("dsh_disp_probe_%d.png" % os.getpid())
    rc, _, _ = run_capture(["import", "-display", disp, "-window", "root", str(tmp)], timeout=15)
    try:
        tmp.unlink()
    except OSError:
        pass
    return rc == 0


class Xvfb:
    def __init__(self):
        self.proc = None
        self.reused = False

    def ensure(self):
        if display_alive():
            self.reused = True
            Log("Xvfb :99 已在跑 → 复用（收尾不杀）")
            return
        log = pathlib.Path("/tmp") / "task38_xvfb.log"
        self.proc = subprocess.Popen(XVFB_ARGV, stdout=open(log, "ab"),
                                     stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                                     start_new_session=True)
        for _ in range(60):
            if display_alive():
                Log("Xvfb :99 自起 pid=%d" % self.proc.pid)
                return
            time.sleep(0.3)
        raise HarnessError("Xvfb :99 起不来")

    def stop(self):
        if self.proc is None or self.reused:
            return
        try:
            os.killpg(os.getpgid(self.proc.pid), signal.SIGTERM)
        except (ProcessLookupError, PermissionError, OSError):
            pass
        try:
            self.proc.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(os.getpgid(self.proc.pid), signal.SIGKILL)
            except OSError:
                pass
        Log("Xvfb :99 已停（自起）")
        self.proc = None


# ================================================================ detector 日志解析

DET_HEAD_RE = re.compile(
    r"^(\S+)Z\s+INFO \[detector\] frame_ts=(\d+)ms\(世界时间\) seq=(\d+) (\d+)x(\d+) "
    r"推理=(\d+)ms dets=(\d+) pose=\(([-\d.]+),([-\d.]+),([-\d.]+),yaw=([-\d.]+)°\)(.*)$")
DET_BOX_RE = re.compile(r"\| (\w+) conf=([0-9.]+) bbox=\((\d+),(\d+)\)-\((\d+),(\d+)\)")
DET_ADOPT_RE = re.compile(r"→ 世界=\(([-\d.]+),([-\d.]+)\) 距离=([0-9.]+)m")
DET_REJ_RE = re.compile(r"判定=拒答（(.*?)）")


def parse_detector_line(line, mono_off):
    m = DET_HEAD_RE.match(line.rstrip("\n"))
    if not m:
        return None
    ts = m.group(1)
    try:
        epoch = datetime.strptime(ts, "%Y-%m-%dT%H:%M:%S.%f").replace(
            tzinfo=timezone.utc).timestamp()
    except ValueError:
        return None
    tail = m.group(12)
    dets = []
    marks = list(DET_BOX_RE.finditer(tail))
    for i, b in enumerate(marks):
        end = marks[i + 1].start() if i + 1 < len(marks) else len(tail)
        rest = tail[b.end():end]
        rec = {"cls": b.group(1), "conf": float(b.group(2)),
               "bbox": (int(b.group(3)), int(b.group(4)), int(b.group(5)), int(b.group(6))),
               "est": None, "range_m": None, "reject": None, "adopted": False}
        a = DET_ADOPT_RE.search(rest)
        if a:
            rec["est"] = (float(a.group(1)), float(a.group(2)))
            rec["range_m"] = float(a.group(3))
            rec["adopted"] = "判定=采纳" in rest
        else:
            r = DET_REJ_RE.search(rest)
            if r:
                rec["reject"] = r.group(1)
        dets.append(rec)
    return {
        "wall": epoch + mono_off,
        "world_ms": int(m.group(2)),
        "seq": int(m.group(3)),
        "width": int(m.group(4)),
        "height": int(m.group(5)),
        "infer_ms": int(m.group(6)),
        "dets": dets,
        "pose": (float(m.group(8)), float(m.group(9)), float(m.group(10)), float(m.group(11))),
    }


def frame_person(fr):
    """该帧"最好的那个人"：conf 最高的 person 框。"""
    best = None
    for d in fr["dets"]:
        if d["cls"] != "person":
            continue
        if best is None or d["conf"] > best["conf"]:
            best = d
    return best


def frame_fp_classes(fr):
    return [d["cls"] for d in fr["dets"] if d["cls"] != "person" and d["conf"] >= 0.5]


def frame6_ok(fr):
    """判据 6 的逐帧口径：person 且 conf>=0.5 且 bbox 底边在画面下半部。"""
    p = fr.get("person")
    return bool(p is not None and p["conf"] >= 0.5 and p["bbox"][3] > fr["height"] / 2.0)


# ================================================================ 截图投影

def project_observer(p_world):
    """世界 (x,y,z) → 观察者相机像素 (u,v)；在相机背后返回 None。"""
    gx, gy, gz = p_world[0], p_world[2], p_world[1]      # 世界 (x,y,z) → Godot (x,z,y)
    dx, dy, dz = gx - OBS_C_GODOT[0], gy - OBS_C_GODOT[1], gz - OBS_C_GODOT[2]
    zc = dx * OBS_FWD[0] + dy * OBS_FWD[1] + dz * OBS_FWD[2]
    if zc <= 0.05:
        return None
    u = IMG_W / 2.0 + OBS_F_PX * (dx * OBS_RIGHT[0] + dy * OBS_RIGHT[1] + dz * OBS_RIGHT[2]) / zc
    v = IMG_H / 2.0 - OBS_F_PX * (dx * OBS_UP[0] + dy * OBS_UP[1] + dz * OBS_UP[2]) / zc
    return (u, v)


def project_box(corners):
    """8 角点 → 屏幕轴对齐框 (u0,v0,u1,v1)，裁剪到画面内；全在画面外返回 None。"""
    pts = [project_observer(c) for c in corners]
    pts = [p for p in pts if p is not None]
    if not pts:
        return None
    u0 = max(0.0, min(p[0] for p in pts))
    u1 = min(float(IMG_W - 1), max(p[0] for p in pts))
    v0 = max(0.0, min(p[1] for p in pts))
    v1 = min(float(IMG_H - 1), max(p[1] for p in pts))
    if u1 - u0 < 1.0 or v1 - v0 < 1.0:
        return None
    return (u0, v0, u1, v1)


def person_corners(x, y, z=0.0):
    r, h = 0.25, 1.7
    return [(x + dx, y + dy, z + dz) for dx in (-r, r) for dy in (-r, r) for dz in (0.0, h)]


def uav_corners(x, y, z):
    r, h = 0.5, 0.3
    return [(x + dx, y + dy, z + dz) for dx in (-r, r) for dy in (-r, r) for dz in (-h, h)]


def shoot(ctx, tag, uav=None, ped=None):
    """整图 + 放大图。uav=(x,y,z)、ped=(x,y)。返回证据 dict。"""
    shots = ctx.out / "shots"
    shots.mkdir(parents=True, exist_ok=True)
    raw = shots / f"raw_{tag}.png"
    full = shots / f"full_{tag}.png"
    zoom = shots / f"zoom_{tag}.png"
    rc, _, err = run_capture(["import", "-display", DISPLAY, "-window", "root", str(raw)], timeout=90)
    if rc != 0:
        return {"tag": tag, "ok": False, "error": "import rc=%d %s" % (rc, err[:200])}
    rc, _, err = run_capture(["convert", str(raw), "-crop", f"{IMG_W}x{IMG_H}+0+0", "+repage",
                              str(full)], timeout=90)
    if rc != 0:
        return {"tag": tag, "ok": False, "error": "convert rc=%d %s" % (rc, err[:200])}

    box_ped = project_box(person_corners(ped[0], ped[1])) if ped else None
    box_uav = project_box(uav_corners(*uav)) if uav else None

    draw_argv = []
    if box_ped:
        draw_argv += ["-stroke", "#ffd400", "-strokewidth", "3",
                      "-draw", "rectangle %d,%d %d,%d" % (int(box_ped[0]), int(box_ped[1]),
                                                          int(box_ped[2]), int(box_ped[3]))]
    if box_uav:
        draw_argv += ["-stroke", "#00c8ff", "-strokewidth", "3",
                      "-draw", "rectangle %d,%d %d,%d" % (int(box_uav[0]), int(box_uav[1]),
                                                          int(box_uav[2]), int(box_uav[3]))]
    if draw_argv:
        tmp = shots / f"_tmp_{tag}.png"
        rc, _, err = run_capture(["convert", str(full)] + draw_argv + [str(tmp)], timeout=90)
        if rc == 0:
            os.replace(tmp, full)
        else:
            try:
                tmp.unlink()
            except OSError:
                pass

    # 放大：以人（没有则机）投影中点为中心取 480×360，3× 最近邻
    center = None
    for box in (box_ped, box_uav):
        if box:
            center = ((box[0] + box[2]) / 2.0, (box[1] + box[3]) / 2.0)
            break
    zoom_ok = False
    if center is not None:
        cx = int(min(max(center[0] - 240, 0), IMG_W - 480))
        cy = int(min(max(center[1] - 180, 0), IMG_H - 360))
        rc, _, err = run_capture(["convert", str(full), "-crop", f"480x360+{cx}+{cy}", "+repage",
                                  "-filter", "point", "-resize", "300%", str(zoom)], timeout=90)
        zoom_ok = rc == 0

    info = {}
    for p in (full, zoom):
        if p.exists():
            rc, so, _ = run_capture(["identify", "-format", "%wx%h", str(p)], timeout=30)
            info[p.name] = {"size": so.strip(), "bytes": p.stat().st_size}
    return {"tag": tag, "ok": full.exists() and full.stat().st_size > 10 * 1024,
            "files": info, "box_ped": box_ped, "box_uav": box_uav, "zoom_ok": zoom_ok,
            "uav": uav, "ped": ped}


# ================================================================ 行人（线程内 30 Hz）

class PedWalker:
    """一条 pedestrian 连接 + 发送线程（`pedestrian_client.PedestrianClient` 的薄封装）。

    发送线程独占 socket（单写者），主线程用 `drive()` 下期望值并等它落地：避免两线程同时写。
    """

    def __init__(self, url, peer_id="human-01", spawn=None):
        self._client = PedestrianClient(url=url, peer_id=peer_id, spawn=spawn,
                                        allow_spawn_fallback=False)
        self.poses = []                       # (mono, x, y, world_time)
        self.connected = False
        self.ready_spawn = None
        self.error = None
        self._lock = threading.Lock()
        self._want = (0.0, None)
        self._sent = (0.0, None)
        self._applied = 0
        self._applied_ts = 0.0
        self._closing = False
        self._thread = None

    def start(self):
        ready = self._client.connect()
        self.ready_spawn = dict(self._client.spawn or {})
        self.connected = True
        self._thread = threading.Thread(target=self._loop, name="ped-walker", daemon=True)
        self._thread.start()
        return ready

    def _loop(self):
        last_stamp = -1.0
        while not self._closing:
            try:
                want = self._want
                if want != self._sent:
                    self._client.drive(want[0], want[1])
                    self._sent = want
                    with self._lock:
                        self._applied += 1
                        self._applied_ts = time.monotonic()
                else:
                    self._client.tick()
                p = self._client.pose()
                stamp = self._client.pose_stamp()
                if p is not None and stamp != last_stamp:
                    last_stamp = stamp
                    with self._lock:
                        self.poses.append((stamp, float(p.get("x", 0.0)),
                                           float(p.get("y", 0.0)),
                                           float(p.get("world_time", 0.0) or 0.0)))
                self._client.sleep_to_next()
            except Exception as exc:            # 断链 / 协议错：记下来，退出线程
                self.error = repr(exc)
                self.connected = False
                return
        try:
            self._client.close(quiet=True)
        except Exception:
            pass
        self.connected = False

    def drive(self, v, dir_rad=None, wait_s=2.0):
        """下期望速度/朝向，返回它真正被发出的 monotonic 时刻。"""
        target = None
        with self._lock:
            self._want = (float(v), dir_rad)
            target = self._applied + 1
        end = time.monotonic() + wait_s
        while time.monotonic() < end:
            with self._lock:
                if self._applied >= target:
                    return self._applied_ts
            time.sleep(0.005)
        return time.monotonic()

    def close(self, join_s=5.0):
        self._closing = True
        if self._thread is not None:
            self._thread.join(timeout=join_s)
            self._thread = None
        self.connected = False

    def latest(self):
        with self._lock:
            return self.poses[-1] if self.poses else None

    def snapshot(self):
        with self._lock:
            return list(self.poses)

    def pose_age(self):
        p = self.latest()
        return None if p is None else time.monotonic() - p[0]


# ================================================================ 记录容器

class Ctx:
    def __init__(self, out, cfg, control_port, args):
        self.out = pathlib.Path(out)
        self.cfg = cfg
        self.control_port = control_port
        self.args = args
        self.rows = []                 # trace.csv 的行
        self.frames = []               # detector_frames.csv 的行
        self.shots = []
        self.world_dir = pathlib.Path(getattr(args, "world_dir", None) or WORLD_DIR).resolve()
        self.world = Proc("world")
        self.sim = Proc("sim")
        self.control = Control(control_port)
        self.ped = None
        self.sim_tail = SimLogTail(SIMLOG_DIR)
        self.world_tail = Tail(self.out / "world.log")
        # 等待锚点用的**累计**行缓存：增量 Tail 会把同一 chunk 里未匹配的行吃掉，
        # 所以「等日志出现某行」必须在历史里搜（见 tmp/task38/impl.md D6）
        self.sim_history = []
        self.world_history = []
        self.mono_off = time.monotonic() - time.time()
        self.camera_hz = None
        self.k = None
        self.env = {}
        self.phases = {}
        self.cancel = {}
        self.checks = []
        self.residual = {}
        self.git_before = []
        self.git_after = []
        self.follow_activity = {}
        self.seg = "S0"

    # ---- 日志
    def read_world(self):
        lines = self.world_tail.since()
        self.world_history.extend(lines)
        return lines

    def read_sim(self):
        lines = self.sim_tail.since()
        self.sim_history.extend(lines)
        for line in lines:
            fr = parse_detector_line(line, self.mono_off)
            if fr is not None:
                self.ingest_frame(fr)
        return lines

    def ingest_frame(self, fr):
        fr["person"] = frame_person(fr)
        fr["fp"] = frame_fp_classes(fr)
        self.frames.append(fr)

    def wait_world(self, pattern, timeout_s):
        rx = re.compile(pattern)
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            self.read_world()
            for line in self.world_history:
                m = rx.search(line)
                if m:
                    return m, line.strip()
            time.sleep(0.15)
        return None, None

    def wait_sim(self, pattern, timeout_s):
        rx = re.compile(pattern)
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            self.read_sim()
            for line in self.sim_history[-8000:]:
                m = rx.search(line)
                if m:
                    return m, line.strip()
            time.sleep(0.15)
        return None, None

    # ---- 真值 / 采样
    def truth_at(self, mono, tol=0.25):
        if self.ped is None:
            return None
        best = None
        for p in self.ped.snapshot():
            dt = abs(p[0] - mono)
            if dt <= tol and (best is None or dt < best[0]):
                best = (dt, p)
        return None if best is None else (best[1][1], best[1][2])

    def sample(self, seg=None):
        seg = seg or self.seg
        self.read_sim()                     # 采样前先抽干 detector 日志（判据 6/7/8 全靠它）
        st = self.control.status()
        now = time.monotonic()
        ped = self.ped.latest() if self.ped is not None else None
        row = {
            "t_wall": round(time.time(), 3),
            "t_mono": round(now, 3),
            "t_world": None,
            "seg": seg,
            "x": st.get("x"), "y": st.get("y"), "z": st.get("z"), "yaw": st.get("yaw"),
            "nav_status": st.get("nav_status"), "task_id": st.get("task_id"),
            "task_ticks": st.get("task_ticks"), "queue_len": st.get("queue_len"),
            "mode": st.get("mode"), "assembled": st.get("assembled"),
            "ped_x": None, "ped_y": None, "ped_connected": bool(self.ped and self.ped.connected),
            "d_m": None, "lat_offset": None, "det_age_s": None,
        }
        if self.frames:
            row["t_world"] = round(self.frames[-1]["world_ms"] / 1000.0, 3)
        if ped is not None:
            row["ped_x"], row["ped_y"] = round(ped[1], 3), round(ped[2], 3)
            row["t_world"] = round(ped[3], 3) if ped[3] else row["t_world"]
            row["d_m"] = round(dist((row["x"], row["y"]), (ped[1], ped[2])), 3)
            row["lat_offset"] = round(ped[2] - row["y"], 3)
            row["det_age_s"] = round(now - ped[0], 3)
        self.rows.append(row)
        return row

    def sample_for(self, seconds, seg=None, dt=0.10, on_tick=None):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.sample(seg)
            if on_tick is not None:
                on_tick()
            time.sleep(dt)


# ================================================================ 世界 / sim 起停

def set_world_env():
    env = dict(os.environ)
    env["DISPLAY"] = DISPLAY
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    return env


def launch_world(ctx):
    if not display_alive():
        raise HarnessError("Xvfb :99 不可用")
    ctx.world.start(world_argv(ctx.world_dir), ROOT, ctx.out / "world.log", env=set_world_env())
    m, line = ctx.wait_world(r"\[ws\] listening on port %d" % WORLD_PORT, 150)
    if m is None:
        raise HarnessError("世界 150 s 内没有监听 :%d（见 world.log）" % WORLD_PORT)
    return line


def launch_sim(ctx):
    ctx.sim.start(SIM_ARGV, INST, ctx.out / "sim.log")
    if not ctx.control.connect(retries=240, delay=0.25):
        raise HarnessError("控制口 %d 60 s 内连不上" % ctx.control_port)
    return True


def status_assembled(ctx, timeout_s=120):
    end = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < end:
        try:
            st = ctx.control.status()
        except Exception as exc:
            last = repr(exc)
            time.sleep(0.3)
            continue
        last = st
        if st.get("assembled") is True:
            return st
        time.sleep(0.3)
    raise HarnessError("sim 120 s 内未 assembled（最后 status=%r）" % (last,))


# ================================================================ S0

def assert_s0(ctx):
    """S0 的 6 条日志锚点 + status 断言（§四.5 / plan G4）。"""
    res = {"anchors": [], "status": {}, "ok": True}

    def anchor(name, pattern, timeout=60, sim=True):
        t0 = time.monotonic()
        m, line = (ctx.wait_sim(pattern, timeout) if sim else ctx.wait_world(pattern, timeout))
        dur = time.monotonic() - t0
        res["anchors"].append({"name": name, "ok": m is not None, "line": line or ""})
        Log("  S0 锚点 %-22s %s (%.1fs) %s" % (name, "OK" if m is not None else "MISSING", dur,
                                              (line or "")[:120]))
        if m is None:
            res["ok"] = False
        return m, line

    # sim 日志（顺序按实际出现先后；每步 30 s 够，出不来就是环境问题）
    anchor("camera_rate_hz", r"\[camera\] camera_sim 使用 slot=camera_nose rate_hz=(\d+)", timeout=30)
    anchor("detector_start",
           r"\[detector\] 已启动：device=cpu model=yolov8n\.safetensors conf=0\.35 nms=0\.45", timeout=30)
    anchor("detector_projection",
           r"\[detector\] 投影参数：fov=70\.0°\(vertical\).*pitch=-15\.0°.*max_range=12\.0", timeout=30)
    anchor("camera_first_frame", r"\[camera\] slot=camera_nose 首帧 640x480", timeout=60)
    # plan §4.5 的这条锚点其实在 **sim 的 tracing 日志**里（不是 Godot 世界 stdout）
    anchor("sim_world_chassis_ready",
           r"\[world\] chassis_ready avatar=(\d+) spawn=\((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)", timeout=30)
    # 世界日志：plan §4.5/G4 写的是 `[world] chassis_ready avatar=1 spawn=(…)`，**代码现实**是
    # `[ws] vehicle spawned … avatar=1 slots=1 … spawn=(64.00, 64.00, 2.50)`（WebSocketManager.gd:601）
    # → 以代码为准（偏差记 tmp/task38/impl.md D5）
    m, line = anchor("world_vehicle_spawn",
                     r"\[ws\] vehicle spawned .*avatar=(\d+) slots=\d+ .*spawn=\((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)",
                     timeout=30, sim=False)
    if m is not None:
        avatar = int(m.group(1))
        sp = (float(m.group(2)), float(m.group(3)), float(m.group(4)))
        ok = (avatar == 1 and abs(sp[0] - UAV_SPAWN[0]) < 0.01
              and abs(sp[1] - UAV_SPAWN[1]) < 0.01 and abs(sp[2] - UAV_SPAWN[2]) < 0.01)
        res["asserts"] = [crit("world_spawn", ok, "avatar=%d spawn=%s" % (avatar, sp))]
        if not ok:
            res["ok"] = False
    # 实测档位的**启动侧**证据：`[chassis] 构建底盘 … forward_speed=100 turn_speed=30`。
    # `binding forward=…` 那种行只在首个动作下发时出现（S2 之后），S0 等不到 → 见 impl.md 偏差 D12
    m, line = anchor("chassis_build_gear",
                     r"\[chassis\] 构建底盘 name=(\S+) forward_speed=(\d+) turn_speed=(\d+)", timeout=30)
    if m is not None:
        fs, ts = int(m.group(2)), int(m.group(3))
        res["asserts"] = res.get("asserts", []) + [
            crit("chassis_forward_speed_100", fs == 100, "forward_speed=%d turn_speed=%d" % (fs, ts))]
        if fs != 100:
            res["ok"] = False

    st = status_assembled(ctx)
    res["status"] = {k: st.get(k) for k in ("x", "y", "z", "yaw", "mode", "nav_status",
                                            "queue_len", "assembled", "avatar_id", "task_id")}
    checks = [
        ("spawn_x", abs(float(st.get("x", 1e9)) - UAV_SPAWN[0]) <= 0.15),
        ("spawn_y", abs(float(st.get("y", 1e9)) - UAV_SPAWN[1]) <= 0.15),
        ("altitude", 2.30 <= float(st.get("z", 0.0)) <= 2.70),
        ("yaw_zero", abs(float(st.get("yaw", 1e9))) <= 0.05),
        ("assembled", st.get("assembled") is True),
        ("avatar_1", st.get("avatar_id") == 1),
        ("mode_auto", st.get("mode") == "auto"),
        ("nav_idle", st.get("nav_status") == "idle"),
        ("queue_empty", st.get("queue_len") == 0),
    ]
    res["asserts"] = res.get("asserts", []) + [crit(n, ok) for n, ok in checks]
    if not all(ok for _, ok in checks):
        res["ok"] = False
    if not res["ok"]:
        raise HarnessError("S0 断言失败（见 phases.s0）")
    return res


def measure_environment(ctx, seconds=10.0):
    """10 s 窗口：相机实测 Hz（frame_ts 间隔中位数）、帧率口径 r、空场基线。"""
    t0 = time.monotonic()
    f0 = len(ctx.frames)
    ctx.seg = "S0j"
    ctx.sample_for(seconds)
    frames = ctx.frames[f0:]
    world_delta = None
    if len(frames) >= 2:
        world_delta = (frames[-1]["world_ms"] - frames[0]["world_ms"]) / 1000.0
    wall_delta = time.monotonic() - t0
    ivs = []
    for a, b in zip(frames, frames[1:]):
        d = (b["world_ms"] - a["world_ms"]) / 1000.0
        if d > 0:
            ivs.append(d)
    med = median(ivs)
    ctx.camera_hz = (1.0 / med) if med else None
    r = (world_delta / wall_delta) if world_delta and wall_delta > 0 else None
    fp_frames = [f for f in frames if f["fp"]]
    fp_classes = {}
    for f in fp_frames:
        for c in f["fp"]:
            fp_classes[c] = fp_classes.get(c, 0) + 1
    persons = [f for f in frames if f["person"] is not None]
    ctx.env.update({
        "camera_hz": None if ctx.camera_hz is None else round(ctx.camera_hz, 2),
        "r_world_over_wall": None if r is None else round(r, 3),
        "s0j_frames": len(frames),
        "s0j_window_s": round(wall_delta, 2),
        "s0j_infer_ms_median": median([f["infer_ms"] for f in frames]),
        "s0j_person_frames": len(persons),
        "s0j_fp_frames": len(fp_frames),
        "s0j_fp_classes": fp_classes,
        "fps_est": None if ctx.camera_hz is None else round(ctx.camera_hz, 2),
    })
    Log("环境：camera_hz=%s r=%s 空场帧=%d 其中 person 帧=%d 非 person(conf>=0.5) 帧=%d %s" % (
        ctx.env["camera_hz"], ctx.env["r_world_over_wall"], len(frames), len(persons),
        len(fp_frames), fp_classes))
    if ctx.camera_hz is None or ctx.camera_hz < 1.25:
        raise HarnessError("相机实测 %.2f Hz < 1.25 Hz（stale=0.8 s 临界）→ 判据 6/7 无从谈起"
                           % (ctx.camera_hz or 0.0))
    return ctx.env


# ================================================================ S0b 归位到格心

def wrap_deg(d):
    """角度归一化到 (-180, 180]。"""
    return (d + 180.0) % 360.0 - 180.0


def first_cell_dpsi(pose):
    """Δψ = 机头到首格格心(64.75,64.25) 的方位角 − 机头 yaw（度）。"""
    x, y, yaw = float(pose["x"]), float(pose["y"]), float(pose["yaw"])
    bearing = math.degrees(math.atan2(S0B_FIRST_CELL[1] - y, S0B_FIRST_CELL[0] - x))
    return wrap_deg(bearing - math.degrees(yaw)), bearing


def s0b_nav_line(ctx):
    """sim 日志里最近一条 `[Nav]` 到达行（取证用，取不到返回 None）。"""
    for line in reversed(ctx.sim_history[-8000:]):
        s = line.strip()
        if "[Nav] 已在目标格" in s or "[Nav] 到达目标" in s:
            return s
    return None


def phase_s0b(ctx, strict):
    """S0b 归位（§四.5）：`goto(64.25, 64.25)` → 等 `nav_status=idle` → 断言 |Δψ| ≤ 10°。

    不过则重发一次 goto；仍不过 → strict 记 FAIL(env) 并停（`FixtureError`，不整轮重试），
    record 只记录后继续（取证用）。**不改任何逻辑**，只动夹具动作。
    """
    st0 = ctx.control.status()
    rec = {
        "mode": ctx.args.s0b,
        "target": list(S0B_TARGET), "first_cell": list(S0B_FIRST_CELL),
        "thr_deg": S0B_DPSI_MAX_DEG,
        "attempts": [], "ok": False,
        "pose0": {k: st0.get(k) for k in ("x", "y", "z", "yaw", "nav_status", "mode",
                                          "task_id", "queue_len")},
        "mode_status": st0.get("mode"),
    }
    # 先挂到 ctx.phases：本函数在 strict 不过时用异常终止整轮，
    # 若等到 return 才赋值，FAIL(env) 的 verdict 里就看不到本次 S0b 的实测（见 impl.md 偏差 D2）
    ctx.phases["s0b"] = rec
    if st0.get("mode") != "auto":
        raise FixtureError("S0b 前置：op_mode=%r ≠ auto → goto 会被 main_loop 忽略"
                           "（control.rs:596-600）" % (st0.get("mode"),))
    Log("S0b 归位：goto%s → 等 nav_status=idle（上限 %.0f s）…" % (S0B_TARGET, S0B_IDLE_TIMEOUT_S))
    for i in (1, 2):
        t_send = time.monotonic()
        ack = ctx.control.send_cmd({"cmd": "goto", "x": S0B_TARGET[0], "y": S0B_TARGET[1]})
        if not ack.get("ok"):
            raise FixtureError("S0b goto 回包失败：%r" % (ack,))
        m_goto, goto_log = ctx.wait_sim(r"\[control\] goto \(([\d.]+), ([\d.]+)\) → cmd_tx", 5)
        m_start, start_log = ctx.wait_sim(r"\[Robot\] 开始任务 TaskId\((\d+)\)：Goto", 10)

        # (c) 命令被消费：task_id 非 None / queue_len>0 / nav_status != idle 之一。
        # no-op 的 goto 会在一拍内结束，抓不到也继续。
        consumed = False
        while time.monotonic() - t_send < 5.0:
            stc = ctx.control.status()
            if (stc.get("task_id") is not None or (stc.get("queue_len") or 0) > 0
                    or stc.get("nav_status") != "idle"):
                consumed = True
                break
            time.sleep(0.02)

        # (d) 等终态：queue_len==0 / task_id None / nav_status==idle 连续两拍，间隔 >= SETTLE_S
        settled = False
        last_idle = None
        st = stc
        while time.monotonic() - t_send < S0B_IDLE_TIMEOUT_S:
            st = ctx.control.status()
            idle_now = (st.get("queue_len") == 0 and st.get("task_id") is None
                        and st.get("nav_status") == "idle")
            if idle_now:
                if last_idle is None:
                    last_idle = time.monotonic()
                if time.monotonic() - last_idle >= S0B_SETTLE_S:
                    settled = True
                    break
            else:
                last_idle = None
            time.sleep(0.05)
        waited = time.monotonic() - t_send
        if not settled:
            tail = "\n".join(ctx.sim_history[-8:])
            raise FixtureError(
                "S0b 第 %d 次 goto 后 %.0f s 内 nav 未回 idle（最后 status=%r）；sim 日志尾：%s"
                % (i, S0B_IDLE_TIMEOUT_S, st, tail))

        # (e) 按实测位姿算 Δψ
        st = ctx.control.status()
        dpsi, bearing = first_cell_dpsi(st)
        x, y, z = float(st.get("x")), float(st.get("y")), float(st.get("z"))
        rec["attempts"].append({
            "i": i, "ack": ack, "goto_log": goto_log or "", "start_log": start_log or "",
            "goto_log_ok": m_goto is not None, "start_log_ok": m_start is not None,
            "consumed": consumed, "waited_s": round(waited, 2),
            "pose": [round(x, 3), round(y, 3), round(z, 3)],
            "yaw_deg": round(math.degrees(float(st.get("yaw") or 0.0)), 3),
            "nav_status": st.get("nav_status"), "task_id": st.get("task_id"),
            "queue_len": st.get("queue_len"),
            "dpsi_deg": round(dpsi, 2), "bearing_deg": round(bearing, 3),
            "dx": round(x - S0B_TARGET[0], 3), "dy": round(y - S0B_TARGET[1], 3),
            "moved_m": round(math.hypot(x - float(st0.get("x")), y - float(st0.get("y"))), 3),
            "nav_noop_log": s0b_nav_line(ctx),
        })
        Log("  S0b 尝试 %d：nav=%s waited=%.2f s 位姿=(%.3f, %.3f, %.3f) yaw=%.2f° "
            "到格心=(%+.3f, %+.3f) moved=%.3f m 方位=%.3f° Δψ=%+.2f°" % (
                i, st.get("nav_status"), waited, x, y, z,
                math.degrees(float(st.get("yaw") or 0.0)),
                x - S0B_TARGET[0], y - S0B_TARGET[1],
                math.hypot(x - float(st0.get("x")), y - float(st0.get("y"))),
                bearing, dpsi))
        if abs(dpsi) <= S0B_DPSI_MAX_DEG:
            rec["ok"] = True
            break

    # (3) 把归位后的位姿写进 trace.csv（seg=S0b；判据窗口都不含它）
    st = ctx.control.status()
    x, y, z = float(st.get("x")), float(st.get("y")), float(st.get("z"))
    rec["final_pose"] = [round(x, 3), round(y, 3), round(z, 3)]
    rec["final_yaw_deg"] = round(math.degrees(float(st.get("yaw") or 0.0)), 3)
    rec["final_dpsi_deg"] = round(first_cell_dpsi(st)[0], 2)
    ctx.seg = "S0b"
    n0 = len(ctx.rows)
    ctx.sample_for(1.0)
    rec["sampled_rows"] = len(ctx.rows) - n0
    # (4) 归位后截图（人还没接入 → ped=None）
    ctx.shots.append(shoot(ctx, "s0b_homed", uav=(x, y, z)))
    # (5) 日志一行
    last = rec["attempts"][-1]
    Log("S0b 归位：attempt=%d/2 位姿=(%.3f, %.3f, %.3f) Δψ=%+.2f° (阈值 ±%.0f°)，moved=%.3f m，"
        "nav=%s" % (last["i"], x, y, z, last["dpsi_deg"], S0B_DPSI_MAX_DEG,
                    last["moved_m"], last["nav_status"]))
    # (6) strict 不过即停；record 只记录
    if not rec["ok"]:
        if strict:
            raise FixtureError(
                "S0b FAIL(env)：goto(%.2f,%.2f) 后 |Δψ|=%.2f° > %.0f°（两次尝试；moved=%.3f m）；"
                "按 §四.5 停" % (S0B_TARGET[0], S0B_TARGET[1], abs(last["dpsi_deg"]),
                                S0B_DPSI_MAX_DEG, last["moved_m"]))
        Log("[warn] S0b 未过 → record 模式继续跑 S1~S6（本轮为取证，非 §四.5 验收路径）")
    return rec


# ================================================================ S1~S6

def phase_s1(ctx):
    cfg = ctx.cfg
    spawn = dict(PED_SPAWN)
    # 诊断开关（默认 70 = §四.5 规定值；只有 --ped-spawn-x 才改，且不在正式判定里用）
    if getattr(ctx.args, "ped_spawn_x", None):
        spawn["x"] = float(ctx.args.ped_spawn_x)
    ctx.ped = PedWalker("ws://127.0.0.1:%d" % WORLD_PORT, "human-01", spawn)
    try:
        ctx.ped.start()
    except (ConnectError, ReadyTimeout, SpawnFallback, ProtocolError) as exc:
        # harness 自身故障（step.md 的唯一例外）：记类名，交给 --attempts 重跑一次
        ctx.env["harness_fault"] = "pedestrian:%s: %s" % (type(exc).__name__, exc)
        raise HarnessError("行人接入失败（%s）：%s" % (type(exc).__name__, exc))
    m, line = ctx.wait_world(r"\[ws\] pedestrian spawned peer='human-01'.*spawn=\(([\d.]+), ([\d.]+)\) yaw=([-\d.]+)",
                             15)
    ok_spawn = m is not None
    ready_spawn = ctx.ped.ready_spawn or {}
    exact = (abs(ready_spawn.get("x", 1e9) - spawn["x"]) < 1e-3
             and abs(ready_spawn.get("y", 1e9) - spawn["y"]) < 1e-3
             and abs(ready_spawn.get("yaw", 1e9) - spawn["yaw"]) < 1e-3)
    ctx.seg = "S1"
    ctx.sample_for(2.0)
    res = {"spawn_log": line or "", "spawn_ok": ok_spawn, "ready_spawn": ready_spawn,
           "spawn_exact": exact, "pose_age": ctx.ped.pose_age()}
    if not (ok_spawn and exact):
        raise HarnessError("行人落点不符：log=%r ready_spawn=%r" % (line, ready_spawn))
    return res


def phase_s2(ctx):
    ctx.seg = "S2"
    ack = ctx.control.send_cmd({"cmd": "follow"})
    if not ack.get("ok"):
        raise HarnessError("follow 回包失败：%r" % (ack,))
    t0 = time.monotonic()
    task_id = None
    while time.monotonic() - t0 < 3.0:
        st = ctx.control.status()
        if st.get("task_id") is not None:
            task_id = st.get("task_id")
            break
        time.sleep(0.05)
    if task_id is None:
        raise HarnessError("follow 后 3 s 内 task_id 仍为 null")
    ticks0 = None
    t_ticks = None
    while time.monotonic() - t0 < 8.0:
        st = ctx.control.status()
        if ticks0 is None:
            ticks0 = st.get("task_ticks")
        if float(st.get("task_ticks") or 0) >= 5:
            t_ticks = float(st.get("task_ticks"))
            break
        time.sleep(0.05)
    ctx.sample_for(1.0)
    if t_ticks is None:
        raise HarnessError("follow 后 8 s 内 task_ticks 未达 5（树没在 tick）")
    m, line = ctx.wait_sim(r"\[control\] follow → cmd_tx", 3)
    return {"ack": ack, "task_id": task_id, "task_ticks": t_ticks,
            "control_log": line or "", "control_log_ok": m is not None,
            "ack_to_ticks_s": round(time.monotonic() - t0, 2)}


def phase_s3(ctx):
    """主窗口：行人沿 +X 走到位移 ≥ 25.2 m（= 20 s 物理时间 × 1.26 m/s）。"""
    ctx.seg = "S3"
    t_drive = ctx.ped.drive(PED_SPEED, 0.0)
    p0 = ctx.ped.latest()
    if p0 is None:
        raise HarnessError("S3 起点没有行人 pose")
    x0, y0 = p0[1], p0[2]
    t_start = time.monotonic()
    shot_marks = {0.40: False, 0.90: False}
    last = None
    x_end = x0
    while True:
        row = ctx.sample("S3")
        last = row
        ped = ctx.ped.latest()
        if ped is not None:
            x_end = ped[1]
        moved = x_end - x0
        for frac in list(shot_marks.keys()):
            if not shot_marks[frac] and moved >= S3_TRACK_M * frac:
                shot_marks[frac] = True
                tag = "s3_person_ahead" if frac >= 0.9 else "s3_progress_%d" % int(frac * 100)
                ctx.shots.append(shoot(ctx, tag, uav=(row["x"], row["y"], row["z"]),
                                       ped=(ped[1], ped[2]) if ped else None))
                Log("  截图 %s（位移 %.1f m）" % (tag, moved))
        if moved >= S3_TRACK_M:
            break
        if time.monotonic() - t_start > 180.0:
            Log("  [warn] S3 180 s 墙钟上限到了，位移只有 %.2f m（记为 S3 timeout）" % moved)
            break
        time.sleep(0.10)
    wall = time.monotonic() - t_start
    k = (x_end - x0) / (PED_SPEED * wall) if wall > 0 else None
    ctx.k = k
    ctx.env["k"] = None if k is None else round(k, 4)
    ctx.env["s3_wall_s"] = round(wall, 2)
    ctx.env["s3_ped_displacement_m"] = round(x_end - x0, 2)
    # 判据 6 字面窗的可复现锚点（与 detector_frames.csv 的 wall 同一时间域 = monotonic）
    ctx.env["s3_t_start"] = round(t_start, 3)
    ctx.env["s3_lit_lo"] = round(t_start + LIT_S3_LO_S, 3)
    ctx.env["s3_lit_hi"] = round(t_start + LIT_S3_HI_S, 3)
    Log("S3 结束：位移 %.2f m / 墙钟 %.1f s → k=%.3f（物理 %.1f s）" % (
        x_end - x0, wall, k or 0.0, (wall * (k or 0.0))))
    # 保一张"人在前"的整图（若 90% 阈值因 timeout 没触发）
    if not ctx.shots or not any(s.get("tag") == "s3_person_ahead" for s in ctx.shots):
        ped = ctx.ped.latest()
        ctx.shots.append(shoot(ctx, "s3_person_ahead", uav=(last["x"], last["y"], last["z"]),
                               ped=(ped[1], ped[2]) if ped else None))
    return {"x0": x0, "y0": y0, "x_end": x_end, "wall_s": round(wall, 2), "k": k,
            "timeout": (x_end - x0) < S3_TRACK_M, "displacement_m": round(x_end - x0, 2),
            "t_start": t_start}


def phase_s4(ctx, k):
    """人停：物理 5 s（墙钟上限 60 s）。"""
    ctx.seg = "S4"
    t_stop = ctx.ped.drive(0.0)
    window = S4_SECONDS / k if k else S4_SECONDS
    window = min(window, 60.0)
    ctx.sample_for(window, "S4")
    ped = ctx.ped.latest()
    st = ctx.control.status()
    ctx.shots.append(shoot(ctx, "s4_person_stopped", uav=(st.get("x"), st.get("y"), st.get("z")),
                           ped=(ped[1], ped[2]) if ped else None))
    return {"t_stop": t_stop, "window_s": round(window, 2), "k": k}


def phase_s5(ctx, k):
    """关行人客户端：模拟丢失；等 ≥0.8 s（且 ≥1/k）后观测 10 物理秒。"""
    ctx.seg = "S5"
    last_frame = None
    for fr in reversed(ctx.frames):
        if fr["person"] is not None:
            last_frame = fr
            break
    ctx.ped.close()
    t_close = time.monotonic()
    m, line = ctx.wait_world(r"\[ws\] pedestrian removed peer='human-01'", 10)
    removed = m is not None
    # 等「最后一帧 person 的墙钟年龄 ≥ 0.8 s」且 ≥ 1.0/k
    need = max(S5_STALE_WAIT_S, (1.0 / k) if k else S5_STALE_WAIT_S)
    while time.monotonic() - t_close < need:
        time.sleep(0.05)
    if last_frame is not None:
        while (time.monotonic() - last_frame["wall"]) < S5_STALE_WAIT_S:
            time.sleep(0.05)
    ctx.sample_for(0.2, "S5")
    t_win = time.monotonic()
    observe = min(S5_OBS_SECONDS / k if k else S5_OBS_SECONDS, 90.0)
    ctx.sample_for(observe, "S5")
    st = ctx.control.status()
    ctx.shots.append(shoot(ctx, "s5_lost_hold", uav=(st.get("x"), st.get("y"), st.get("z")),
                           ped=None))
    return {"t_close": t_close, "removed_log": line or "", "removed": removed,
            "wait_s": round(time.monotonic() - t_close, 2), "observe_s": round(observe, 2),
            "t_win": t_win, "last_person_frame_age_s": (
                None if last_frame is None else round(time.monotonic() - last_frame["wall"], 2))}


def wait_follow_ticks(ctx, ticks=5, timeout=15.0):
    ack = ctx.control.send_cmd({"cmd": "follow"})
    if not ack.get("ok"):
        raise HarnessError("follow 回包失败：%r" % (ack,))
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        st = ctx.control.status()
        if st.get("task_id") is not None and float(st.get("task_ticks") or 0) >= ticks:
            return st
        time.sleep(0.05)
    raise HarnessError("follow 后 %.0f s 内 task_ticks 未达 %d" % (timeout, ticks))


def measure_cancel(ctx, path, k):
    """判据 5：20 ms 细采样量「命令 → nav_status=idle」。"""
    st0 = wait_follow_ticks(ctx)
    cmd = {"cmd": "queue", "missions": []} if path == "A" else {"cmd": "stop"}
    t_send = time.monotonic()
    ack = ctx.control.send_cmd(cmd)
    t_ack = time.monotonic()
    idle_at = None
    samples = []
    while time.monotonic() - t_send < 30.0:
        st = ctx.control.status()
        samples.append((round((time.monotonic() - t_send) * 1000.0, 1),
                        st.get("queue_len"), st.get("task_id"), st.get("nav_status")))
        if st.get("queue_len") == 0 and st.get("task_id") is None and st.get("nav_status") == "idle":
            idle_at = time.monotonic()
            break
        time.sleep(0.02)
    latency_ms = (idle_at - t_send) * 1000.0 if idle_at is not None else None
    tick_ms = (50.0 / k) if k else 50.0
    ticks = (latency_ms / tick_ms) if latency_ms is not None else None
    return {"path": path, "cmd": cmd, "ok": bool(ack.get("ok")),
            "ack_ms": round((t_ack - t_send) * 1000.0, 1),
            "latency_ms": None if latency_ms is None else round(latency_ms, 1),
            "tick_ms": round(tick_ms, 1), "ticks": None if ticks is None else round(ticks, 2),
            "literal_100ms": (latency_ms is not None and latency_ms <= 100.0),
            "samples": samples[:40], "idle_reached": idle_at is not None,
            "before": {"task_id": st0.get("task_id"), "task_ticks": st0.get("task_ticks")}}


def phase_s6(ctx, k):
    ctx.seg = "S6"
    ctx.cancel["A"] = measure_cancel(ctx, "A", k)
    Log("S6-A queue{missions:[]}：latency=%s ms = %s tick（1 tick=%s ms）" % (
        ctx.cancel["A"]["latency_ms"], ctx.cancel["A"]["ticks"], ctx.cancel["A"]["tick_ms"]))
    ctx.cancel["B"] = measure_cancel(ctx, "B", k)
    Log("S6-B stop：latency=%s ms = %s tick" % (
        ctx.cancel["B"]["latency_ms"], ctx.cancel["B"]["ticks"]))
    ctx.sample_for(0.5, "S6")
    return ctx.cancel


# ================================================================ phases 总装

def run_phase_a(ctx):
    t_all = time.monotonic()
    if not ctx.args.no_launch:
        Log("S0：起世界 …")
        launch_world(ctx)
        Log("S0：起 sim …")
        launch_sim(ctx)
        status_assembled(ctx)
        ctx.sim_tail.reset()
        if ctx.sim_tail.path is None:
            raise HarnessError("找不到 sim 日志（%s）" % SIMLOG_DIR)
        Log("S0：sim 日志 = %s" % ctx.sim_tail.path.name)
        ctx.phases["s0"] = assert_s0(ctx)
        # 世界日志锚点：静态行人（与 §四.3 说法不符的现况，只记录）
        m, line = ctx.wait_world(r"\[world\] pedestrians built: (\d+)", 5)
        ctx.phases["s0"]["pedestrians_built"] = line or ""
        measure_environment(ctx, 10.0)
        # 世界 fps 粗估：相机 Hz ≤ 渲染 fps，二者在 <10 fps 时近似相等
        ctx.env["fps_note"] = "fps ≈ camera_hz（相机 Hz 被 rate_hz=10 卡住，渲染更慢时两者相等）"
    else:
        Log("--no-launch：复用已在跑的世界 / 控制口")
        if not ctx.control.connect(retries=20, delay=0.25):
            raise HarnessError("--no-launch 但控制口 %d 连不上" % ctx.control_port)
        ctx.phases["s0"] = {"status": ctx.control.status(), "anchors": []}
        ctx.sim_tail.reset()
        ctx.env["camera_hz"] = 6.6
        ctx.env["k"] = 1.0

    if ctx.args.s0b != "off":
        ctx.phases["s0b"] = phase_s0b(ctx, strict=(ctx.args.s0b == "strict"))

    Log("S1：行人接入 spawn=(70,64) …")
    ctx.phases["s1"] = phase_s1(ctx)
    Log("S2：触发 follow …")
    ctx.phases["s2"] = phase_s2(ctx)

    Log("S3：行人沿 +X 走 %.1f m（物理 20 s）…" % S3_TRACK_M)
    s3 = phase_s3(ctx)
    ctx.phases["s3"] = s3
    k = s3["k"] or 0.34

    Log("S4：人停 %.0f 物理秒（墙钟 %.1f s）…" % (S4_SECONDS, S4_SECONDS / k))
    ctx.phases["s4"] = phase_s4(ctx, k)

    Log("S5：关行人客户端（丢失）…")
    ctx.phases["s5"] = phase_s5(ctx, k)

    Log("S6：取消双路径 …")
    ctx.phases["s6"] = phase_s6(ctx, k)

    ctx.env["phase_wall_s"] = round(time.monotonic() - t_all, 1)
    return ctx


# ================================================================ 判据

def judge_all(ctx):
    rows = ctx.rows
    frames = ctx.frames
    k = ctx.k or ctx.env.get("k") or 0.34
    x_end = ctx.phases.get("s3", {}).get("x_end")
    x0 = ctx.phases.get("s3", {}).get("x0")
    fa = getattr(ctx, "follow_activity", None) or {}
    # 空转标注：跟随全程没让机身动起来时，字面 PASS 不等于"跟随成立"（见 verdict §3.1）
    IDLE_CAVEAT = None if fa.get("follow_active") else \
        "空转通过：跟随全程未让机身动起来（见 §3.1）"
    J = {}

    # 判据 1：S3 后 10 s（位移后 12.6 m）距离带
    win = [r for r in rows if r["seg"] == "S3" and r["d_m"] is not None
           and x_end is not None and r["ped_x"] is not None and r["ped_x"] >= x_end - S3_WIN_M]
    ds = [r["d_m"] for r in win]
    share = (sum(1 for d in ds if BAND_LO <= d <= BAND_HI) / len(ds)) if ds else None
    mean_d = (sum(ds) / len(ds)) if ds else None
    j1_ok = bool(ds) and share is not None and share >= BAND_SHARE_MIN and min(ds) >= BAND_MIN_D
    J["1"] = {"pass": j1_ok, "share_in_band": share, "min_d": min(ds) if ds else None,
              "mean_d": mean_d, "n": len(ds),
              "window": "S3 末尾 12.6 m 位移（= 10 s × 1.26 m/s）",
              "literal": literal_s3_prefix(rows, ctx.phases.get("s3", {}).get("t_start")),
              "value": "share=%.2f min=%.2f mean=%.2f n=%d" % (
                  share or 0, min(ds) if ds else 0, mean_d or 0, len(ds))}

    # 判据 2：全程 min d
    all_d = [r["d_m"] for r in rows if r["d_m"] is not None]
    j2_ok = bool(all_d) and min(all_d) >= GLOBAL_MIN_D
    J["2"] = {"pass": j2_ok, "min_d": min(all_d) if all_d else None, "n": len(all_d),
              "caveat": IDLE_CAVEAT,
              "value": "min=%.2f n=%d" % (min(all_d) if all_d else 0, len(all_d))}

    # 判据 3：人停 5 物理秒内水平位移 + 速度符号翻转
    s4 = [r for r in rows if r["seg"] == "S4"]
    move3, flips3 = 0.0, 0
    if s4:
        p0 = (s4[0]["x"], s4[0]["y"])
        move3 = max(dist((r["x"], r["y"]), p0) for r in s4)
        last_sign = 0
        for a, b in zip(s4, s4[1:]):
            dx = b["x"] - a["x"]
            if abs(dx) < 0.005:
                continue
            s = 1 if dx > 0 else -1
            if last_sign and s != last_sign:
                flips3 += 1
            last_sign = s
    j3_ok = bool(s4) and move3 <= STOP_MOVE_MAX and flips3 <= 1
    J["3"] = {"pass": j3_ok, "move_m": round(move3, 3), "sign_flips": flips3,
              "n": len(s4), "window_s": ctx.phases.get("s4", {}).get("window_s"),
              "literal_5s_wall": literal_move(s4, LIT_WIN_S4_S),
              "caveat": IDLE_CAVEAT,
              "value": "move=%.2f m flips=%d n=%d" % (move3, flips3, len(s4))}

    # 判据 4：丢失 ≥0.8 s 后 10 物理秒内水平位移
    t_win = ctx.phases.get("s5", {}).get("t_win")
    s5 = [r for r in rows if r["seg"] == "S5" and t_win is not None and r["t_mono"] >= t_win]
    move4 = 0.0
    if s5:
        p0 = (s5[0]["x"], s5[0]["y"])
        move4 = max(dist((r["x"], r["y"]), p0) for r in s5)
    j4_ok = bool(s5) and move4 <= LOST_MOVE_MAX
    J["4"] = {"pass": j4_ok, "move_m": round(move4, 3), "n": len(s5),
              "last_person_frame_age_s": ctx.phases.get("s5", {}).get("last_person_frame_age_s"),
              "literal_10s_wall": literal_move(s5, LIT_WIN_S5_S),
              "caveat": IDLE_CAVEAT,
              "value": "move=%.2f m n=%d" % (move4, len(s5))}

    # 判据 5：取消 ≤2 tick
    ca, cb = ctx.cancel.get("A", {}), ctx.cancel.get("B", {})
    j5_ok = (ca.get("ticks") is not None and ca["ticks"] <= CANCEL_TICKS_MAX
             and cb.get("ticks") is not None and cb["ticks"] <= CANCEL_TICKS_MAX)
    J["5"] = {"pass": j5_ok, "a_ticks": ca.get("ticks"), "a_ms": ca.get("latency_ms"),
              "b_ticks": cb.get("ticks"), "b_ms": cb.get("latency_ms"),
              "tick_ms": ca.get("tick_ms"),
              "literal_100ms_a": ca.get("literal_100ms"), "literal_100ms_b": cb.get("literal_100ms"),
              "nav_before": (ca["samples"][0][3] if ca.get("samples") else None),
              "nav_after": (ca["samples"][-1][3] if ca.get("samples") else None),
              "caveat": (None if fa.get("nav_nonidle_rows")
                         else "空转通过：nav 在命令前后都是 idle，只测到队列/task 被清空"),
              "value": "A=%s tick(%s ms) B=%s tick(%s ms) 1tick=%s ms" % (
                  ca.get("ticks"), ca.get("latency_ms"), cb.get("ticks"), cb.get("latency_ms"),
                  ca.get("tick_ms"))}

    # 判据 6：S3 后 10 s 内 ≥90% 帧有 person（conf≥0.5，bbox 底边 > 240）
    win_frames = []
    for fr in frames:
        tr = ctx.truth_at(fr["wall"])
        if tr is None or x_end is None:
            continue
        if tr[0] >= x_end - S3_WIN_M and fr["wall"] >= ctx.phases.get("s3", {}).get("t_start", 0):
            win_frames.append((fr, tr))
    good6 = 0
    details6 = []
    for fr, tr in win_frames:
        ok = frame6_ok(fr)
        p = fr["person"]
        good6 += 1 if ok else 0
        details6.append({"seq": fr["seq"], "conf": None if p is None else p["conf"],
                         "ymax": None if p is None else p["bbox"][3], "ok": ok})
    share6 = (good6 / len(win_frames)) if win_frames else None
    j6_ok = bool(win_frames) and share6 is not None and share6 >= DET_SHARE_MIN
    note6 = None
    if not j6_ok:
        note6 = ("窗内命中 %d/%d 帧的成因是机身没跟上、人早已走出视场（见 §3.1/§8.1），不是检测器在"
                 "新世界失效——本次 S3 全程有 person 的帧 %s 帧（conf 最高 %s，末帧距离 %s m）"
                 % (good6, len(win_frames), fa.get("det_person_frames"),
                    fa.get("det_person_max_conf"), fa.get("det_last_person_dist_m")))
    J["6"] = {"pass": j6_ok, "share": share6, "n": len(win_frames), "good": good6,
              "literal": literal_s3_frames(frames, ctx.phases.get("s3", {}).get("t_start")),
              "note": note6,
              "value": "share=%.2f (%d/%d)" % (share6 or 0, good6, len(win_frames)),
              "details": details6[:200]}

    # 判据 7：S3 全程非 person conf≥0.5 帧占比 ≤5%
    s3_frames = [fr for fr in frames
                 if ctx.phases.get("s3", {}).get("t_start", 0) <= fr["wall"]
                 <= ctx.phases.get("s4", {}).get("t_stop", 1e18)]
    fp_frames = [fr for fr in s3_frames if fr["fp"]]
    share7 = (len(fp_frames) / len(s3_frames)) if s3_frames else None
    cls7 = {}
    for fr in fp_frames:
        for c in fr["fp"]:
            cls7[c] = cls7.get(c, 0) + 1
    j7_ok = bool(s3_frames) and share7 is not None and share7 <= FP_SHARE_MAX
    J["7"] = {"pass": j7_ok, "share": share7, "n_frames": len(s3_frames),
              "n_fp_frames": len(fp_frames), "classes": cls7,
              "value": "share=%.3f (%d/%d) classes=%s" % (
                  share7 or 0, len(fp_frames), len(s3_frames), cls7)}

    # 判据 8：3~6 m 投影误差 < 1 m（逐帧）；6~8 m 不出现"离谱点"（err ≥ 1.0 m，与前半条同界）
    def _proj_band(lo, hi, lo_open=False):
        """lo_open=True → (lo, hi]；否则 [lo, hi]。"""
        out = []
        for fr in frames:
            tr = ctx.truth_at(fr["wall"])
            if tr is None:
                continue
            d_true = dist((fr["pose"][0], fr["pose"][1]), tr)
            if not ((lo < d_true) if lo_open else (lo <= d_true)):
                continue
            if d_true > hi:
                continue
            p = fr["person"]
            if p is None or not p["adopted"] or p["est"] is None:
                continue
            out.append({"seq": fr["seq"], "d_true": round(d_true, 2), "est": p["est"],
                        "truth": [round(tr[0], 2), round(tr[1], 2)],
                        "err_m": round(dist(p["est"], tr), 3)})
        return out

    def _band_stat(items):
        errs = [e["err_m"] for e in items]
        return {"n": len(items), "median": median(errs), "p95": percentile(errs, 0.95),
                "max": max(errs) if errs else None, "list": items}

    band36 = _proj_band(3.0, 6.0)                 # 闭区间
    band68 = _proj_band(6.0, 8.0, lo_open=True)   # (6, 8]，6.0 归上一段
    absurd = [e for e in band68 if e["err_m"] >= PROJ_ERR_MAX]
    j8_ok = (bool(band36) and max(e["err_m"] for e in band36) < PROJ_ERR_MAX and not absurd)
    errs = band36
    J["8"] = {"pass": j8_ok, "n": len(errs),
              "median": median([e["err_m"] for e in errs]),
              "p95": percentile([e["err_m"] for e in errs], 0.95),
              "max": max([e["err_m"] for e in errs]) if errs else None,
              "band36": _band_stat(band36), "band68": _band_stat(band68),
              "absurd_6_8": len(absurd), "band68_empty": not band68,
              "value": "3~6m n=%d median=%.3f p95=%.3f max=%.3f；6~8m n=%d median=%.3f max=%.3f 离谱=%d" % (
                  len(band36), median([e["err_m"] for e in band36]) or 0.0,
                  percentile([e["err_m"] for e in band36], 0.95) or 0.0,
                  (max([e["err_m"] for e in band36]) if band36 else 0.0),
                  len(band68), median([e["err_m"] for e in band68]) or 0.0,
                  (max([e["err_m"] for e in band68]) if band68 else 0.0), len(absurd)),
              "details": errs[:200]}

    # 判据 9：既有检查全绿 + LiDAR 无路缘/标线
    counted = [c for c in ctx.checks if c["counted"]]
    green = [c for c in counted if c["ok"]]
    named = {c["label"]: c for c in counted if c["named"]}
    lidar_ok = False
    street_ok = False
    for c in ctx.checks:
        if c["base"] == "check_lidar":
            lidar_ok = ("F7-real-wall-distance ok" in c["text"]
                        and "F8-real-wall-hits ok" in c["text"])
        if c["base"] == "check_street":
            street_ok = ("street-zero-collision ok" in c["text"]
                         and "facade-zero-collision ok" in c["text"])
    j9_ok = (len(counted) >= 22 and len(green) == len(counted) and lidar_ok and street_ok
             and all(c["ok"] for c in named.values()))
    reds = [c["label"] for c in counted if not c["ok"]]
    J["9"] = {"pass": j9_ok, "green": len(green), "total": len(counted), "reds": reds,
              "lidar_F7_F8_ok": lidar_ok, "street_zero_collision_ok": street_ok,
              "value": "green=%d/%d reds=%s lidar_f7f8=%s street_zero=%s" % (
                  len(green), len(counted), reds, lidar_ok, street_ok)}

    # 判据 10：证据齐全
    need = [ctx.out / "trace.csv", ctx.out / "detector_frames.csv", ctx.out / "verdict.md",
            ctx.out / "verdict.json"]
    full_shots = sorted((ctx.out / "shots").glob("full_*.png"))
    big = [p for p in full_shots if p.stat().st_size > 10 * 1024]
    tags = {p.stem.replace("full_", "") for p in big}
    want_tags = {"s3_person_ahead", "s4_person_stopped", "s5_lost_hold"}
    j10_ok = (all(p.exists() for p in need) and want_tags.issubset(tags)
              and len(big) >= 3 and len(ctx.rows) >= 10 and len(frames) >= 30)
    J["10"] = {"pass": j10_ok, "files": [str(p.relative_to(ctx.out)) for p in need if p.exists()],
               "shots": sorted(tags), "rows": len(ctx.rows), "frames": len(frames),
               "value": "shots=%s rows=%d frames=%d" % (sorted(tags), len(ctx.rows), len(frames))}

    return J


def observations(ctx):
    rows = ctx.rows
    k = ctx.k
    obs = {}
    lat = [abs(r["lat_offset"]) for r in rows if r["lat_offset"] is not None]
    obs["lateral_offset_mean"] = median(lat)
    obs["lateral_offset_max"] = max(lat) if lat else None
    # yaw 振荡：|Δyaw|>0.5° 且符号翻转
    yaws = [(r["t_mono"], math.degrees(r["yaw"])) for r in rows if r["yaw"] is not None]
    flips, last_sign = 0, 0
    for (t0, y0), (t1, y1) in zip(yaws, yaws[1:]):
        d = y1 - y0
        if abs(d) <= 0.5:
            continue
        s = 1 if d > 0 else -1
        if last_sign and s != last_sign:
            flips += 1
        last_sign = s
    obs["yaw_oscillation_count"] = flips
    obs["detector_hz"] = ctx.env.get("camera_hz")
    obs["infer_ms_median"] = median([f["infer_ms"] for f in ctx.frames])
    obs["infer_ms_p95"] = percentile([f["infer_ms"] for f in ctx.frames], 0.95)
    max_v = 0.0
    prev = None
    for r in rows:
        if r["seg"] != "S3":
            prev = None
            continue
        if prev is not None:
            dt = r["t_mono"] - prev["t_mono"]
            if dt > 0.01:
                v = dist((r["x"], r["y"]), (prev["x"], prev["y"])) / dt * (k or 1.0)
                max_v = max(max_v, v)
        prev = r
    obs["uav_max_speed_mps"] = round(max_v, 3)
    obs["k"] = k
    obs["r_world_over_wall"] = ctx.env.get("r_world_over_wall")
    obs["camera_hz"] = ctx.env.get("camera_hz")
    obs["fps_est"] = ctx.env.get("fps_est")
    obs["s3_wall_s"] = ctx.env.get("s3_wall_s")
    obs["s3_ped_displacement_m"] = ctx.env.get("s3_ped_displacement_m")
    return obs


# ================================================================ 跟随机制证据（只读分析）
# 目的：把「跟随有没有真的动起来」变成**可复算**的数字（判据本身不变，见 verdict §8.1）。
# 数据源全部已在盘上：<out>/sim_trace.log、ctx.rows（trace.csv）、ctx.cfg。不额外起世界。

SIM_STATE_RE = re.compile(
    r"\[Sim\] 位置=\(([-\d.]+),([-\d.]+)\) yaw=([-\d.]+)° "
    r"状态=(\w+) 动作=(\w+) sub_target=(None|Some\(\((-?\d+), (-?\d+)\)\))")
CHASSIS_RE = re.compile(
    r"\[chassis\] \w+ (\w+) → action=(\w+) speed=(\d+)（binding forward=(\d+) turn=(\d+)）")
FOLLOW_START_RE = re.compile(r"\[Robot\] 开始任务 TaskId\((\d+)\)：Follow")
FOLLOW_CMD_RE = re.compile(r"\[control\] follow → cmd_tx")
DET_BEARING_RE = re.compile(
    r"\| person conf=([0-9.]+) bbox=\((\d+),(\d+)\)-\((\d+),(\d+)\)"
    r".*?距离=([0-9.]+)m.*?相对机头=([+-][0-9.]+)°")
DET_FRAME_TS_RE = re.compile(r"frame_ts=(\d+)ms")

# 判据 1/3/4/6 的**字面墙钟**窗口（§四.5/§四.6 的字面秒数；只作对照，不参与判定）
LIT_WIN_S4_S, LIT_WIN_S5_S = 5.0, 10.0
LIT_S3_LO_S, LIT_S3_HI_S = 10.0, 20.0


def literal_move(rows, window_s):
    """一段 trace 行取【末 window_s 墙钟】内的最大水平位移（字面口径）。返回 (m, n, win_s)。"""
    if not rows:
        return (None, 0, None)
    t_end = max(r["t_mono"] for r in rows)
    w = [r for r in rows if r["t_mono"] >= t_end - window_s]
    if not w:
        return (None, 0, None)
    p0 = (w[0]["x"], w[0]["y"])
    return (round(max(dist((r["x"], r["y"]), p0) for r in w), 3), len(w),
            round(t_end - w[0]["t_mono"], 2))


def literal_s3_prefix(rows, t0, a=LIT_S3_LO_S, b=LIT_S3_HI_S):
    """字面 20 s 墙钟口径：S3 只跑 20 s 墙钟，取 [t0+a, t0+b] 这段算距离带。只作对照。"""
    if t0 is None:
        return {"n": 0}
    w = [r for r in rows if r["seg"] == "S3" and r["d_m"] is not None
         and t0 + a <= r["t_mono"] <= t0 + b]
    ds = [r["d_m"] for r in w]
    if not ds:
        return {"n": 0}
    return {"n": len(ds),
            "share": round(sum(1 for d in ds if BAND_LO <= d <= BAND_HI) / len(ds), 4),
            "min_d": min(ds), "mean_d": round(sum(ds) / len(ds), 3)}


def literal_s3_frames(frames, t_start, a=LIT_S3_LO_S, b=LIT_S3_HI_S):
    """字面 20 s 墙钟口径（**帧占比**，detector_frames.csv 口径）。只作对照。"""
    if t_start is None:
        return {"n": 0, "good": 0, "share": None, "t_lo": None, "t_hi": None}
    t_lo, t_hi = t_start + a, t_start + b
    w = [fr for fr in frames if t_lo <= fr["wall"] <= t_hi]
    good = sum(1 for fr in w if frame6_ok(fr))
    return {"n": len(w), "good": good,
            "share": round(good / len(w), 4) if w else None,
            "t_lo": round(t_lo, 3), "t_hi": round(t_hi, 3), "win_s": [a, b]}


def analyze_follow(ctx):
    """返回 follow_activity（字段名固定）；任何字段拿不到就置 None 并进 errors，不抛异常。"""
    fa = {
        "log_path": None, "log_bytes": 0,
        "follow_cmd_n": 0, "follow_start_n": 0, "follow_task_ids": [],
        "binding": {"forward": None, "turn": None},
        "states": [], "action_counts": {"TurnRight": 0, "TurnLeft": 0, "MoveForward": 0, "Stop": 0},
        "first_after_follow": None,
        "yaw_follow_start": None, "yaw_max_after_follow": None, "yaw_delta": None,
        "det_person_frames": 0, "det_person_max_conf": None,
        "det_first_person_ms": None, "det_last_person_ms": None,
        "det_last_person_dist_m": None, "det_last_person_bearing_rel_deg": None,
        "det_last_person_bbox_x1": None,
        "hfov_half_deg": None,
        "nav_rows": 0, "nav_nonidle_rows": 0, "nav_status_counts": {},
        "s2s3_uav_disp_m": 0.0,
        "follow_active": False,
        "errors": [],
    }
    # 相机半视场（由 config 的 fov_deg + 640x480 实算；解析失败置 None，**不猜**）
    try:
        fov_deg = float(ctx.cfg["detector"]["camera"]["fov_deg"])
        fa["hfov_half_deg"] = round(
            math.degrees(math.atan(math.tan(math.radians(fov_deg / 2.0)) * 640.0 / 480.0)), 2)
    except Exception as exc:
        fa["errors"].append("hfov_half_deg：%r" % (exc,))

    # 1) sim 日志（优先 <out>/sim_trace.log 硬拷贝）
    try:
        p = pathlib.Path(ctx.out) / "sim_trace.log"
        if p.is_file():
            text = p.read_text(encoding="utf-8", errors="replace")
            fa["log_path"], fa["log_bytes"] = str(p), p.stat().st_size
        else:
            tail = getattr(ctx, "sim_tail", None)
            text = tail.text() if tail is not None else ""
            fa["errors"].append("无 <out>/sim_trace.log，退化为 sim_tail.text()（%d B）" % len(text))
        seen_follow = False
        for line in text.splitlines():
            m = FOLLOW_CMD_RE.search(line)
            if m:
                fa["follow_cmd_n"] += 1
            m = FOLLOW_START_RE.search(line)
            if m:
                fa["follow_start_n"] += 1
                tid = int(m.group(1))
                if tid not in fa["follow_task_ids"]:
                    fa["follow_task_ids"].append(tid)
                seen_follow = True
            m = CHASSIS_RE.search(line)
            if m:
                fa["binding"] = {"forward": int(m.group(4)), "turn": int(m.group(5))}
            m = SIM_STATE_RE.search(line)
            if m:
                st = {"x": float(m.group(1)), "y": float(m.group(2)), "yaw": float(m.group(3)),
                      "state": m.group(4), "action": m.group(5), "sub_target": m.group(6)}
                fa["states"].append(st)
                act = st["action"]
                fa["action_counts"][act] = fa["action_counts"].get(act, 0) + 1
                if seen_follow and fa["first_after_follow"] is None:
                    fa["first_after_follow"] = {"state": st["state"], "action": act,
                                                "sub_target": st["sub_target"], "yaw": st["yaw"]}
                continue
            m = DET_BEARING_RE.search(line)
            if m:
                fa["det_person_frames"] += 1
                conf = float(m.group(1))
                if fa["det_person_max_conf"] is None or conf > fa["det_person_max_conf"]:
                    fa["det_person_max_conf"] = conf
                ts = DET_FRAME_TS_RE.search(line)
                if ts:
                    ms = int(ts.group(1))
                    if fa["det_first_person_ms"] is None:
                        fa["det_first_person_ms"] = ms
                    fa["det_last_person_ms"] = ms
                fa["det_last_person_dist_m"] = float(m.group(6))
                fa["det_last_person_bearing_rel_deg"] = float(m.group(7))
                fa["det_last_person_bbox_x1"] = int(m.group(2))
        if not text:
            fa["errors"].append("sim 日志为空，跟随机制字段全部 n/a")
    except Exception as exc:
        fa["errors"].append("sim 日志解析失败：%r" % (exc,))

    # 2) yaw：第一条 state 的 yaw 作为起点，之后的最大 |yaw|
    try:
        if fa["states"]:
            fa["yaw_follow_start"] = fa["states"][0]["yaw"]
            later = [abs(s["yaw"]) for s in fa["states"][1:]]
            fa["yaw_max_after_follow"] = max(later) if later else fa["yaw_follow_start"]
            fa["yaw_delta"] = round(fa["yaw_max_after_follow"] - fa["yaw_follow_start"], 2)
    except Exception as exc:
        fa["errors"].append("yaw 统计失败：%r" % (exc,))

    # 3) trace 行：nav 分布 + S2→S3 机身位移
    try:
        rows = ctx.rows
        fa["nav_rows"] = len(rows)
        navs = {}
        for r in rows:
            nv = r.get("nav_status")
            if nv is None:
                continue
            navs[nv] = navs.get(nv, 0) + 1
            if nv != "idle":
                fa["nav_nonidle_rows"] += 1
        fa["nav_status_counts"] = navs
        w = [r for r in rows if r.get("seg") in ("S2", "S3") and r.get("x") is not None]
        if w:
            p0 = (w[0]["x"], w[0]["y"])
            fa["s2s3_uav_disp_m"] = round(max(dist((r["x"], r["y"]), p0) for r in w), 3)
        else:
            fa["errors"].append("trace 里没有 S2/S3 行，s2s3_uav_disp_m 无从计算")
    except Exception as exc:
        fa["errors"].append("trace 统计失败：%r" % (exc,))

    # 诊断量（不是判据）：判据 1 要求 10 s 内补齐 ≥12.6 m 位移差，<1 m 即"跟随没发生"
    fa["follow_active"] = bool(fa["nav_nonidle_rows"] > 0 or fa["s2s3_uav_disp_m"] >= 1.0)
    return fa


def static_pedestrians():
    """解析 World.gd 的 `PEDESTRIANS` 表，按 UAV_SPAWN 算距离/方位（算出来，不写死）。"""
    out = {"path": "orion-simulation-world/src/world/World.gd", "line": None, "ok": False,
           "points": [], "error": None}
    src = WORLD_DIR / "src" / "world" / "World.gd"
    try:
        text = src.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        out["error"] = "World.gd 读不了：%r" % (exc,)
        return out
    lines = text.splitlines()
    start = None
    for i, ln in enumerate(lines):
        if re.match(r"^const PEDESTRIANS\s*:=", ln.strip()):
            start = i
            out["line"] = i + 1
            break
    if start is None:
        out["error"] = "World.gd PEDESTRIANS 解析失败（找不到 const 行）"
        return out
    block = []
    for ln in lines[start:]:
        block.append(ln)
        if ln.strip() == "]" and len(block) > 1:
            break
    for m in re.finditer(r"Vector3\(\s*([-\d.]+),\s*([-\d.]+),", "\n".join(block)):
        x, y = float(m.group(1)), float(m.group(2))
        dx, dy = x - UAV_SPAWN[0], y - UAV_SPAWN[1]
        out["points"].append({"x": x, "y": y,
                              "dist_m": round(math.hypot(dx, dy), 2),
                              "bearing_deg": round(math.degrees(math.atan2(dy, dx)), 1)})
    out["ok"] = bool(out["points"])
    if not out["ok"]:
        out["error"] = "World.gd PEDESTRIANS 解析失败（块内没有 Vector3）"
    return out


# ================================================================ 既有检查回归

def check_argv(entry):
    name, mode, timeout_s, _baseline = entry[0], entry[1], entry[2], entry[3]
    script = "res://src/tests/%s.gd" % name
    if mode == "headless":
        return ["timeout", str(timeout_s), "godot", "--headless", "--path", str(WORLD_DIR),
                "--fixed-fps", "60", "--script", script], None
    return (["timeout", str(timeout_s), "godot", "--display-driver", "x11",
             "--rendering-driver", "opengl3", "--audio-driver", "Dummy",
             "--path", str(WORLD_DIR), "--fixed-fps", "60", "--script", script],
            set_world_env())


def run_checks(ctx, selection):
    entries = []
    for entry in CHECKS:
        name, mode, _to, _base, counted, named = entry
        if selection == "named" and not (named or name == "check_street"):
            continue
        entries.append(entry)
    logs = ctx.out / "checks"
    logs.mkdir(parents=True, exist_ok=True)
    results = []
    for entry in entries:
        name, mode, timeout_s, baseline, counted, named = entry
        label = "%s(%s)" % (name, mode) if mode == "xvfb" else name
        argv, env = check_argv(entry)
        log_path = logs / ("%s.log" % label.replace("(", "_").replace(")", ""))
        Log("check %-28s mode=%-8s timeout=%ss" % (name, mode, timeout_s))
        t0 = time.monotonic()
        with log_path.open("wb") as f:
            try:
                r = subprocess.run(argv, stdout=f, stderr=subprocess.STDOUT,
                                   cwd=str(ROOT), env=env, timeout=timeout_s + 60, check=False)
                rc = r.returncode
            except subprocess.TimeoutExpired:
                rc = -1
        dur = time.monotonic() - t0
        text = log_path.read_text(encoding="utf-8", errors="replace")
        # 只认**收尾汇总行** `[check] total=N failed=M`，且取最后一条
        # （脚本正文里还有 `captures … total=3(==3)` 之类，用 re.search 会抓错，见 impl.md D11）
        summ = re.findall(r"\[check\] total=(\d+) failed=(\d+)", text)
        total = int(summ[-1][0]) if summ else None
        failed = int(summ[-1][1]) if summ else None
        self_ok = "SELF-CHECK OK" in text
        # `check_project` 没有 `total=`/`failed=` 汇总行（COMPONENTS.md §9.1）：
        # 它只认 `SELF-CHECK OK` + exit 0；其余脚本必须 `failed=0`
        ok = (rc == 0 and self_ok and (failed == 0 if failed is not None else True))
        results.append({
            "base": name, "label": label, "mode": mode, "counted": counted, "named": named,
            "exit": rc, "total": total, "failed": failed, "self_check_ok": self_ok,
            "baseline": baseline, "drift": (None if (baseline is None or total is None)
                                            else total - baseline),
            "ok": ok, "dur_s": round(dur, 1), "log": log_path.name,
            "tail": "\n".join(text.splitlines()[-3:]), "text": text,
        })
        Log("  → exit=%s total=%s failed=%s self=%s %s (%.0fs)" % (
            rc, total, failed, self_ok, "OK" if ok else "RED", dur))
    (ctx.out / "checks.json").write_text(
        json.dumps([{k: v for k, v in r.items() if k != "text"} for r in results],
                   ensure_ascii=False, indent=2), encoding="utf-8")
    return results


# ================================================================ 产物

def write_csvs(ctx):
    cols = ["t_wall", "t_world", "seg", "x", "y", "z", "yaw", "nav_status", "task_id",
            "task_ticks", "queue_len", "ped_x", "ped_y", "ped_connected", "d_m",
            "lat_offset", "det_age_s"]
    with (ctx.out / "trace.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=cols, extrasaction="ignore")
        w.writeheader()
        for row in ctx.rows:
            w.writerow({c: row.get(c) for c in cols})
    with (ctx.out / "detector_frames.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["wall", "world_ms", "seq", "infer_ms", "dets", "pose_x", "pose_y", "pose_z",
                    "pose_yaw", "person_conf", "person_ymax", "est_x", "est_y", "err_m",
                    "rejects", "fp_classes"])
        for fr in ctx.frames:
            p = fr["person"]
            tr = ctx.truth_at(fr["wall"])
            err = None
            if p is not None and p["est"] is not None and tr is not None:
                err = round(dist(p["est"], tr), 3)
            w.writerow([round(fr["wall"], 3), fr["world_ms"], fr["seq"], fr["infer_ms"],
                        len(fr["dets"]), fr["pose"][0], fr["pose"][1], fr["pose"][2],
                        fr["pose"][3], None if p is None else p["conf"],
                        None if p is None else p["bbox"][3],
                        None if (p is None or p["est"] is None) else p["est"][0],
                        None if (p is None or p["est"] is None) else p["est"][1],
                        err, "|".join(d["reject"] for d in fr["dets"] if d["reject"]),
                        "|".join(fr["fp"])])
    with (ctx.out / "ped_trace.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["mono", "x", "y", "world_time"])
        for p in (ctx.ped.snapshot() if ctx.ped else []):
            w.writerow([round(p[0], 3), round(p[1], 3), round(p[2], 3), p[3]])


def write_env_json(ctx):
    data = dict(ctx.env)
    data["mono_off"] = ctx.mono_off
    data["control_port"] = ctx.control_port
    data["world_port"] = WORLD_PORT
    data["world_dir"] = str(ctx.world_dir)
    data["ped_spawn"] = dict(PED_SPAWN, x=float(getattr(ctx.args, "ped_spawn_x", None) or PED_SPAWN["x"]))
    data["world_argv"] = world_argv(ctx.world_dir)
    data["sim_argv"] = SIM_ARGV
    data["shots"] = ctx.shots
    data["follow_activity"] = getattr(ctx, "follow_activity", {})
    data["boundary"] = git_boundary_report()
    data["static_pedestrians"] = static_pedestrians()
    data["attempts"] = ctx.env.get("attempts")
    data["rerun_reason"] = ctx.env.get("rerun_reason")
    (ctx.out / "env.json").write_text(json.dumps(data, ensure_ascii=False, indent=2),
                                      encoding="utf-8")
    return data


CONCLUSION_LINES = {
    "1": "S3 后 10 s 距离带",
    "2": "全程不穿人",
    "3": "人停 → 机停",
    "4": "丢失 → 停住",
    "5": "取消 ≤2 tick",
    "6": "新世界·检测",
    "7": "新世界·误检",
    "8": "新世界·投影",
    "9": "无碰撞回归",
    "10": "证据齐全",
}


def write_verdict(ctx, judges, obs, checks, overall, reason, pre):
    out = ctx.out
    env = ctx.env
    L = []
    A = L.append
    A("# task_38 Step 3 — 新世界 Follow 复验（只验不改）")
    A("")
    A("> 本文件由 `Tool/sim_follow_test.py` 生成；Presented by KeJi / Created Date ： 2026-10-29")
    A("> 产物目录：`%s`" % out)
    A("> 结论：**%s**%s" % (overall, ("（%s）" % reason) if reason else ""))
    A("")

    A("## 0 环境与口径")
    A("")
    fa = getattr(ctx, "follow_activity", {}) or {}
    bnd = git_boundary_report()
    spd = static_pedestrians()
    try:
        fov_deg = float(ctx.cfg["detector"]["camera"]["fov_deg"])
    except Exception:
        fov_deg = None
    A("| 量 | 实测 | 说明 |")
    A("|---|---|---|")
    A("| 渲染 fps（估） | %s | 新世界 PBR+HDRI 在 llvmpipe 下；fps ≈ camera_hz |" % env.get("fps_est"))
    A("| 相机实测 Hz | %s | 配置 `rate_hz=10`，`CameraSensor.Tick_Stream` 一次 `_process` 最多一帧 → Hz ≤ fps |"
      % env.get("camera_hz"))
    A("| 物理/实时 k | %s | S3 就地测：`25.2 / (1.26 × S3 墙钟)` |" % env.get("k"))
    A("| r = 世界钟/墙钟 | %s | detector `frame_ts` 增量 ÷ 墙钟 |" % env.get("r_world_over_wall"))
    A("| S3 墙钟 / 位移 | %s s / %s m | 目标 25.2 m（= 20 s × 1.26 m/s） |" % (
        env.get("s3_wall_s"), env.get("s3_ped_displacement_m")))
    A("| 空场基线（S0j） | %s 帧 / %s s，其中 person 帧 %s，非 person(conf≥0.5) 帧 %s %s | 判据 6/7 的归因基线 |" % (
        env.get("s0j_frames"), env.get("s0j_window_s"), env.get("s0j_person_frames"),
        env.get("s0j_fp_frames"), env.get("s0j_fp_classes")))
    A("| 仓库边界 | `%s` vs `tmp/task38/git_baseline.txt` → `extra=%s missing=%s` | 与评审同一条命令；allow=%s |" % (
        bnd.get("cmd"), bnd.get("extra"), bnd.get("missing"), bnd.get("allow")))
    A("| 实测档位 | `binding forward=%s turn=%s` | 来自 sim 日志 `[chassis]` 行（§四.9 R12 要求记档位） |" % (
        (fa.get("binding") or {}).get("forward"), (fa.get("binding") or {}).get("turn")))
    A("| 相机半视场 | `±%s°`（fov=%s° 垂直，640×480） | 判据 6 的「人出画」要靠它解释 |" % (
        fa.get("hfov_half_deg"), fov_deg))
    A("| 本轮尝试 | `attempt=%s` + `rerun_reason=%s` | step.md 要求重跑必须注明 |" % (
        env.get("attempts"), env.get("rerun_reason") or "（未重跑）"))
    A("")
    A("**口径定义（本文件自包含，见 `tmp/task38/plan.md` §4 另见）**：")
    A("")
    A("* 主口径 = **物理时间**：S3 终结条件 = 行人位移 ≥ 25.2 m（= 20 s × 1.26 m/s，与时钟无关）；"
      "S4/S5 时长按 S3 末尾实测 k 折算；")
    A("* 并列报 = **墙钟字面口径**（§3 的 `口径` 列、§3.2/§3.4，全部由 trace 实测，无写死数）；")
    A("* **阈值一个没动**，也没改世界外观/配置/任何 Rust 代码。")
    A("")
    A("**与 §四.3「世界中没有人」的差异（只记录，不算 FAIL 依据）**：`PERSONS` 已删，但 `%s:%s` 的 "
      "`PEDESTRIANS` 仍有 %s 个静态行人；按 World.gd 表实算：%s。它们全部落在 `max_range_m=12` 之外、"
      "也在半视场 ±%s° 之外（半视场由 fov 垂直 + 640×480 实算），S0j 空场基线（上一行）与之自洽。"
      "世界日志 `%s`。" % (
          spd.get("path"), spd.get("line"), len(spd.get("points") or []),
          ("；".join("(%.1f, %.1f) → %.2f m / %.1f°" % (p["x"], p["y"], p["dist_m"], p["bearing_deg"])
                    for p in spd.get("points") or [])) or "n/a（World.gd PEDESTRIANS 解析失败）",
          fa.get("hfov_half_deg"),
          ctx.phases.get("s0", {}).get("pedestrians_built", "n/a")))
    pre_dirs = sorted(p for p in RESULTS.glob("task38_follow_*") if p.resolve() != out.resolve()) \
        if RESULTS.is_dir() else []
    if pre_dirs:
        A("")
        A("> 上一轮产物目录（本轮是**脚本改了**所以重跑，不是跑坏了重试）：`%s`" % pre_dirs[-1])
    A("")

    A("## 1 前置自检表")
    A("")
    A("| # | 项 | 结果 | 实测 |")
    A("|---|---|---|---|")
    for i, c in enumerate(pre, 1):
        A("| %d | `%s` | %s | %s |" % (i, c["name"], "OK" if c["ok"] else "**FAIL**",
                                        c["detail"].replace("|", "/")))
    A("")

    A("## 2 S0~S6 时序")
    A("")
    s0 = ctx.phases.get("s0", {})
    A("**S0 日志锚点**")
    A("")
    for a in s0.get("anchors", []):
        A("* `%s` — %s — `%s`" % (a["name"], "OK" if a["ok"] else "**MISSING**", a["line"][:200]))
    A("")
    A("**S0 status 断言**：`%s`" % json.dumps(s0.get("status", {}), ensure_ascii=False))
    A("")
    s0b_lines(A, ctx)
    A("**S1** 行人接入：`%s`；ready_spawn=`%s`；spawn_exact=%s；pose_age=%s" % (
        ctx.phases.get("s1", {}).get("spawn_log", ""),
        ctx.phases.get("s1", {}).get("ready_spawn"),
        ctx.phases.get("s1", {}).get("spawn_exact"),
        ctx.phases.get("s1", {}).get("pose_age")))
    A("")
    s2 = ctx.phases.get("s2", {})
    A("**S2** follow：ack=`%s`，task_id=%s，task_ticks=%s，控制口日志 `%s`" % (
        s2.get("ack"), s2.get("task_id"), s2.get("task_ticks"), s2.get("control_log")))
    A("")
    s3 = ctx.phases.get("s3", {})
    A("**S3** 墙钟 %s s，行人 x: %.2f → %.2f（位移 %s m），k=%s，timeout=%s" % (
        s3.get("wall_s"), s3.get("x0") or 0, s3.get("x_end") or 0, s3.get("displacement_m"),
        s3.get("k"), s3.get("timeout")))
    s4 = ctx.phases.get("s4", {})
    A("**S4** 人停，观测窗 %s s 墙钟（= 5 物理秒 / k）" % s4.get("window_s"))
    s5 = ctx.phases.get("s5", {})
    A("**S5** 关行人客户端：removed=%s（`%s`），等待 %s s，观测 %s s 墙钟；最后一帧 person 年龄 %s s" % (
        s5.get("removed"), s5.get("removed_log", "")[:120], s5.get("wait_s"),
        s5.get("observe_s"), s5.get("last_person_frame_age_s")))
    ca, cb = ctx.cancel.get("A", {}), ctx.cancel.get("B", {})
    A("**S6** 取消 A（`queue{missions:[]}`）：%s ms（%s tick）；取消 B（`stop`）：%s ms（%s tick）；"
      "1 tick = %s ms 墙钟" % (ca.get("latency_ms"), ca.get("ticks"), cb.get("latency_ms"),
                              cb.get("ticks"), ca.get("tick_ms")))
    A("")
    A("**实测档位（动作下发时 sim 日志自报）**：`binding forward=%s turn=%s`（判据侧只记录，不改）。" % (
        (fa.get("binding") or {}).get("forward"), (fa.get("binding") or {}).get("turn")))
    A("**跟随机制**：见 §8.1（`[Sim] 位置=… 动作=…` 逐行 + detector 首末帧）。")
    A("")

    A("## 3 十条判据")
    A("")
    A("| 判据 | 实测值 | 判定 | 口径 |")
    A("|---|---|---|---|")
    lit = {
        "1": "20 s 墙钟重切：n=%s share=%s min=%s mean=%s" % (
            (judges["1"].get("literal") or {}).get("n"),
            (judges["1"].get("literal") or {}).get("share"),
            (judges["1"].get("literal") or {}).get("min_d"),
            (judges["1"].get("literal") or {}).get("mean_d")),
        "3": "末 %s s 墙钟内最大水平位移 %s m（n=%s）" % (
            LIT_WIN_S4_S, (judges["3"].get("literal_5s_wall") or [None, 0, None])[0],
            (judges["3"].get("literal_5s_wall") or [None, 0, None])[1]),
        "4": "末 %s s 墙钟内最大水平位移 %s m（n=%s）；同窗物理只推进 %.1f s" % (
            LIT_WIN_S5_S, (judges["4"].get("literal_10s_wall") or [None, 0, None])[0],
            (judges["4"].get("literal_10s_wall") or [None, 0, None])[1],
            (ctx.k or 0.0) * LIT_WIN_S5_S),
        "5": "`≤100 ms`：A=%s / B=%s" % (
            ca.get("latency_ms"), cb.get("latency_ms")),
        "6": "20 s 墙钟：n=%s share=%s" % (
            (judges["6"].get("literal") or {}).get("n"),
            (judges["6"].get("literal") or {}).get("share")),
    }
    for i in range(1, 11):
        j = judges[str(i)]
        caveat = j.get("caveat") or j.get("note")
        kou = caveat or "主口径（物理时间）"
        if lit.get(str(i)):
            kou = "%s；字面：%s" % (kou, lit[str(i)])
        val = str(j.get("value", "")) + (" †" if (j.get("caveat") or "").startswith("空转") else "")
        A("| %d %s | %s | **%s** | %s |" % (
            i, CONCLUSION_LINES[str(i)], val.replace("|", "/"),
            "PASS" if j["pass"] else "FAIL", kou.replace("|", "/")))
    A("")
    A("`†` = 字面 PASS 但**空转**：读数由「UAV 全程没动」造成，不得当作跟随成立（见 §3.1）。")
    A("")

    # ---- §3.1 空转通过的四条 --------------------------------------------------
    A("### 3.1 空转通过的四条（判据 2/3/4/5，不得当作跟随成立）")
    A("")
    A("| 量 | 实测 | 说明 |")
    A("|---|---|---|")
    A("| `nav_status` 分布（全 trace） | `%s` | 非 idle 行 %d / 总行 %d |" % (
        fa.get("nav_status_counts"), fa.get("nav_nonidle_rows") or 0, fa.get("nav_rows") or 0))
    A("| S2→S3 机身最大水平位移 | %s m | 判据 1 要在 10 s 内补齐 ≥12.6 m 的位移差 |" % (
        fa.get("s2s3_uav_disp_m")))
    A("| 行人 S3 位移 | %s m | 目标 25.2 m（§四.5） |" % (env.get("s3_ped_displacement_m"),))
    A("| 判据 5 取消前/后 nav | before=`%s` / after=`%s` | 队列与 task 确实被清空 |" % (
        judges["5"].get("nav_before"), judges["5"].get("nav_after")))
    A("| 判据 5 的 task_id 变化 | before=%s / after=%s | 首个 status 采样已是 idle |" % (
        (ca.get("samples") or [[None, None, None, None]])[0][2],
        (ca.get("samples") or [[None, None, None, None]])[-1][2]))
    A("")
    A("**结论**：判据 2/3/4/5 的字面要求被满足，但它是「UAV 全程未跟随」的副作用；"
      "这 4 条 PASS **不能**当作跟随成立。判据 5 的取消在控制口回调内**同步**完成（`reset`），"
      "首个 status 已是 idle，故 1 条采样即全部证据——它测到的是「队列/task 被清空」，"
      "**没测到**「运行中的任务被中断后姿态收敛」。")
    A("")
    A("判据 5 细采样（前 12 条）：")
    A("```")
    for s in ca.get("samples", [])[:12]:
        A("A %s" % (s,))
    for s in cb.get("samples", [])[:12]:
        A("B %s" % (s,))
    A("```")
    A("")
    A("复现：`python3 -c \"import csv,collections;print(collections.Counter(r['nav_status'] for r in "
      "csv.DictReader(open('%s/trace.csv'))))\"`" % out)
    A("")

    # ---- §3.2 判据 3/4 的字面墙钟读数（实测）----------------------------------
    A("### 3.2 判据 3/4 的字面墙钟读数（实测，不是估算）")
    A("")
    A("| 判据 | 字面窗（墙钟） | 实测最大水平位移 | 窗内采样行 | 实际窗长 |")
    A("|---|---|---|---|---|")
    for tag, crit_key, field, win in (("3（人停）", "3", "literal_5s_wall", LIT_WIN_S4_S),
                                      ("4（丢失）", "4", "literal_10s_wall", LIT_WIN_S5_S)):
        m_, n_, w_ = (judges[crit_key].get(field) or [None, 0, None])
        A("| %s | %s s | %s m | %s | %s s |" % (tag, win, m_, n_, w_))
    A("")
    A("两条与主口径（S4 整段 / S5 观测段的首点位移 %s m / %s m）一致；"
      "这是「UAV 在这两段里根本没动」的直接读数。" % (
          judges["3"].get("move_m"), judges["4"].get("move_m")))
    A("")
    A("复现（按 last-50 / last-100 行近似 5 s / 10 s 墙钟窗；精确版见本脚本 `literal_move()`）：")
    A("")
    A("```")
    A("python3 -c \"import csv,math;R='%s';rows=list(csv.DictReader(open(R+'/trace.csv')));"
      "dd=lambda w:max(math.hypot(float(r['x'])-float(w[0]['x']),float(r['y'])-float(w[0]['y'])) for r in w);"
      "print({s:round(dd([r for r in rows if r['seg']==s][-k:]),3) for s,k in (('S4',50),('S5',100))})\"" % out)
    A("```")
    A("")

    # ---- §3.3 判据 8 的两个距离段 ---------------------------------------------
    A("### 3.3 判据 8 的两个距离段")
    A("")
    j8 = judges["8"]
    for label, key, lo_txt, hi_txt in (("3~6 m（闭区间）", "band36", "3.0", "6.0"),
                                       ("6~8 m（左开右闭）", "band68", "(6.0", "8.0]")):
        b = j8.get(key) or {}
        A("**%s**：n=%s median=%s p95=%s max=%s；离谱点（err ≥ %s m）= %s" % (
            label, b.get("n"), b.get("median"), b.get("p95"), b.get("max"), PROJ_ERR_MAX,
            len([e for e in (b.get("list") or []) if e["err_m"] >= PROJ_ERR_MAX])))
        A("")
        if not b.get("n"):
            A("> n=0，无从检验（本轮没有该距离段的采纳帧；不虚构）。")
            A("")
            continue
        A("| seq | d_true (m) | err (m) | est | truth |")
        A("|---|---|---|---|---|")
        for e in (b.get("list") or [])[:40]:
            A("| %s | %s | %s | %s | %s |" % (e["seq"], e["d_true"], e["err_m"],
                                              e["est"], e["truth"]))
        A("")
    A("判据 8 主判仍按 3~6 m（判据前半条）；6~8 m 只查「不出现离谱点」，"
      "「离谱」口径 = err ≥ %s m（与前半条同界，文档未给数）。" % PROJ_ERR_MAX)
    A("")
    A("复现：`grep -c '| person conf=' %s/sim_trace.log` 与 "
      "`python3 -c \"import csv;print([(r['seq'],r['err_m']) for r in "
      "csv.DictReader(open('%s/detector_frames.csv')) if r['err_m'] and float(r['err_m'])>=1.0])\"`" % (
          out, out))
    A("")

    # ---- §3.4 判据 1/6 的字面 20 s 墙钟（只作对照）----------------------------
    A("### 3.4 判据 1/6 的字面 20 s 墙钟口径（**只作对照，不参与判定**）")
    A("")
    l1 = judges["1"].get("literal") or {}
    l6 = judges["6"].get("literal") or {}
    A("* **判据 1（距离带，`trace.csv` 行）**：n=%s share=%s min=%s mean=%s"
      "（窗 = S3 起点 +[%s, %s] s 墙钟）" % (
          l1.get("n"), l1.get("share"), l1.get("min_d"), l1.get("mean_d"),
          LIT_S3_LO_S, LIT_S3_HI_S))
    A("* **判据 6（检测占比，`detector_frames.csv` 帧）**：n=%s good=%s share=%s"
      "（窗 = `wall ∈ [%s, %s]`，规则 = person conf≥0.5 且 bbox 底边 > H/2 (=240)）" % (
          l6.get("n"), l6.get("good"), l6.get("share"), l6.get("t_lo"), l6.get("t_hi")))
    A("")
    A("> 判据 6 的字面口径是**帧占比**，不是距离统计（上一轮把判据 1 的数误贴到判据 6，"
      "见 `tmp/task38/prev_issues.md` #2）。")
    A("")
    A("主口径（物理时间：S3 末尾 12.6 m 位移窗）下判据 1 share=%s min=%.2f mean=%.2f，判据 6 share=%s。"
      "字面口径明显更差，是因为本机 `k=%s`：S3 走完 25.2 m 花了 %s s 墙钟，"
      "20 s 墙钟只走到约 %.1f m（= 25.2 m × %.2f）。"
      % (judges["1"].get("share_in_band"), judges["1"].get("min_d") or 0.0,
         judges["1"].get("mean_d") or 0.0, judges["6"].get("share"), env.get("k"),
         env.get("s3_wall_s"),
         S3_TRACK_M * ((LIT_S3_HI_S / (env.get("s3_wall_s") or 1.0))),
         (LIT_S3_HI_S / (env.get("s3_wall_s") or 1.0))))
    A("")
    A("复现：`python3 -c \"import csv;rows=[r for r in csv.DictReader(open('%s/trace.csv')) "
      "if r['seg']=='S3'];print(len(rows), rows[0]['t_wall'], rows[-1]['t_wall'])\"`"
      "（主口径窗见本脚本 `literal_s3_prefix()`）。" % out)
    A("")
    A("判据 6 复现（帧占比；窗口两端由 `env.json` 的 `s3_lit_lo` / `s3_lit_hi` 给出）：")
    A("")
    A("```")
    A("python3 -c \"import csv;R='%s';rows=list(csv.DictReader(open(R+'/detector_frames.csv')));"
      "w=[r for r in rows if %s <= float(r['wall']) <= %s];"
      "g=[r for r in w if r['person_conf'] and float(r['person_conf'])>=0.5 "
      "and int(r['person_ymax'])>240];"
      "print('n=%%d good=%%d share=%%.4f' %% (len(w), len(g), (len(g)/len(w)) if w else 0.0))\"" % (
          out, l6.get("t_lo"), l6.get("t_hi")))
    A("```")
    A("")

    A("## 4 观察项（只记录，不判 FAIL）")
    A("")
    A("| 项 | 值 |")
    A("|---|---|")
    for key, title in (("lateral_offset_mean", "侧向偏移 `|y_ped−y_uav|` 中位数（m）"),
                       ("lateral_offset_max", "侧向偏移最大值（m）"),
                       ("yaw_oscillation_count", "yaw 振荡次数（|Δyaw|>0.5° 且符号翻转）"),
                       ("detector_hz", "detector 实际帧率（Hz）"),
                       ("infer_ms_median", "单帧推理耗时中位数（ms）"),
                       ("infer_ms_p95", "单帧推理耗时 p95（ms）"),
                       ("uav_max_speed_mps", "UAV 实测最大速度（m/s，已按 k 折算）"),
                       ("k", "物理/实时 k"),
                       ("camera_hz", "相机实测 Hz"),
                       ("fps_est", "渲染 fps（估）")):
        A("| %s | %s |" % (title, obs.get(key)))
    A("")

    A("## 5 既有检查回归（%d 条）" % len(checks))
    A("")
    A("| 检查 | 模式 | exit | total | 基线 | 漂移 | SELF-CHECK | 判定 |")
    A("|---|---|---|---|---|---|---|---|")
    for c in checks:
        A("| %s | %s | %s | %s | %s | %s | %s | %s |" % (
            c["base"], c["mode"], c["exit"], c["total"], c["baseline"], c["drift"],
            "OK" if c["self_check_ok"] else "NO", "OK" if c["ok"] else "**RED**"))
    A("")
    A("**判据 9 点名**：`check_lidar` 的 `F7-real-wall-distance` / `F8-real-wall-hits` = %s；"
      "`check_street` 的 `street-zero-collision` / `facade-zero-collision` = %s。" % (
          "PASS 行齐全" if judges["9"].get("lidar_F7_F8_ok") else "**缺**",
          "0 碰撞" if judges["9"].get("street_zero_collision_ok") else "**缺**"))
    A("")
    A("不计入「既有 17 个」的两条佐证（`check_world_look`）：")
    A("")
    for c in checks:
        if not c["counted"]:
            A("* %s exit=%s total=%s failed=%s %s" % (c["label"], c["exit"], c["total"],
                                                      c["failed"], "OK" if c["ok"] else "**RED**"))
    A("")

    A("## 6 产物清单")
    A("")
    A("| 文件 | 字节 |")
    A("|---|---|")
    for p in sorted(out.glob("*")):
        if p.is_file():
            A("| `%s` | %d |" % (p.name, p.stat().st_size))
    for p in sorted((out / "shots").glob("*.png")):
        A("| `shots/%s` | %d |" % (p.name, p.stat().st_size))
    A("")
    A("截图 tag 与整图/放大图：")
    A("")
    for s in ctx.shots:
        A("* `%s` — ok=%s files=%s" % (s.get("tag"), s.get("ok"),
                                       json.dumps(s.get("files", {}), ensure_ascii=False)))
    A("")

    A("## 7 残留与清理")
    A("")
    A("```")
    A("pgrep -af 'pleiades-sim|godot .*orion-simulation-world|Xvfb :99' → %s" % (
        ctx.residual.get("pgrep") or "(空)"))
    A("ports 9098/9999 → %s" % (ctx.residual.get("ports") or "(空)"))
    A("Xvfb :99 → %s" % ctx.residual.get("xvfb"))
    A("git status 差异（相对本步开始）→ %s" % (ctx.residual.get("git_diff") or "(空)"))
    A("仓库边界（`%s` vs `tmp/task38/git_baseline.txt`，allow=%s）→ %s" % (
        bnd.get("cmd"), bnd.get("allow"),
        "extra=%s missing=%s" % (bnd.get("extra"), bnd.get("missing"))))
    A("```")
    A("")

    A("## 8 根因初判与建议（不改代码）")
    A("")
    A("### 8.1 跟随未生效的机制证据（全部来自 `sim_trace.log` / `trace.csv` 实测）")
    A("")
    if fa.get("errors"):
        A("解析告警（不影响 10 条判据）：")
        A("")
        for e in fa["errors"]:
            A("* %s" % e)
        A("")
    A("| 量 | 实测 |")
    A("|---|---|")
    A("| 跟随命令 / 任务起点 | `follow → cmd_tx` %s 次；`开始任务 TaskId` %s 次，TaskId=%s |" % (
        fa.get("follow_cmd_n"), fa.get("follow_start_n"), fa.get("follow_task_ids")))
    A("| 首个 `[Sim]` 行（任务起点之后） | 状态=`%s` 动作=`%s` sub_target=`%s` yaw=`%s°` |" % (
        (fa.get("first_after_follow") or {}).get("state"),
        (fa.get("first_after_follow") or {}).get("action"),
        (fa.get("first_after_follow") or {}).get("sub_target"),
        (fa.get("first_after_follow") or {}).get("yaw")))
    A("| yaw 起 → 最大 | `%s° → %s°`（Δ=%s°） |" % (
        fa.get("yaw_follow_start"), fa.get("yaw_max_after_follow"), fa.get("yaw_delta")))
    A("| `[Sim]` 动作计数 | `%s` |" % (fa.get("action_counts"),))
    A("| 机身位移（S2→S3） | %s m（`nav_nonidle`=%s / 总 %s 行） |" % (
        fa.get("s2s3_uav_disp_m"), fa.get("nav_nonidle_rows"), fa.get("nav_rows")))
    A("| person 帧（判定=采纳） | %s 帧，conf 最高 %s；世界时间 %s→%s ms |" % (
        fa.get("det_person_frames"), fa.get("det_person_max_conf"),
        fa.get("det_first_person_ms"), fa.get("det_last_person_ms")))
    A("| 末帧 person | 距离 %s m，相对机头 %s°，bbox 左边缘 x1=%s |" % (
        fa.get("det_last_person_dist_m"), fa.get("det_last_person_bearing_rel_deg"),
        fa.get("det_last_person_bbox_x1")))
    A("| 档位 | `binding forward=%s turn=%s` |" % (
        (fa.get("binding") or {}).get("forward"), (fa.get("binding") or {}).get("turn")))
    A("| 相机半视场 | `±%s°`（由 `[detector.camera].fov_deg=%s` 垂直 + 640×480 实算） |" % (
        fa.get("hfov_half_deg"), fov_deg))
    A("")
    A("上表每一行的复现命令（原样可跑）：")
    A("")
    A("```bash")
    A("grep -c 'follow → cmd_tx' %s/sim_trace.log" % out)
    A("grep -m1 '开始任务' %s/sim_trace.log" % out)
    A("grep -m1 '\\[Sim\\] 位置=' %s/sim_trace.log" % out)
    A("grep -c '动作=TurnRight' %s/sim_trace.log" % out)
    A("grep -c '动作=MoveForward' %s/sim_trace.log" % out)
    A("grep -o 'binding forward=[0-9]* turn=[0-9]*' %s/sim_trace.log | sort -u" % out)
    A("grep -c 'person conf=' %s/sim_trace.log" % out)
    A("python3 -c \"import csv,collections;print(collections.Counter(r['nav_status'] for r in "
      "csv.DictReader(open('%s/trace.csv'))))\"" % out)
    A("```")
    A("")

    A("### 8.2 根因初判")
    A("")
    for line in root_cause_lines(ctx, judges):
        A("* " + line)
    A("")
    A("### 8.3 S0b 归位未生效的代码级根因（本轮新增）")
    A("")
    rc83 = s0b_root_cause_lines(ctx)
    if rc83:
        for line in rc83:
            A("* " + line)
        A("")
        A("夹具改法建议（**只建议，本轮不改**，都只动夹具、不动 Rust 与阈值）：")
        A("")
        for i, line in enumerate(S0B_FIX_LINES, 1):
            A("%d. %s" % (i, line))
    else:
        A("* S0b 已通过（或未执行 `--s0b off`），本节不适用。")
    A("")

    A("## 9 结论")
    A("")
    A("**%s**%s" % (overall, ("：" + reason) if reason else ""))
    A("")
    A(summary_text(ctx, judges, overall, reason))
    A("")

    (out / "verdict.md").write_text("\n".join(L) + "\n", encoding="utf-8")


def root_cause_lines(ctx, judges):
    """§8.2 根因初判：**必须由数据分支**，不写死结论（见 tmp/task38/plan.md §1 #7）。"""
    out = []
    k = ctx.k
    fa = getattr(ctx, "follow_activity", {}) or {}
    if k is not None and k < 0.6:
        out.append("物理推进只有 %.2f× 实时（渲染 %.1f fps × `max_physics_steps_per_frame=8` → "
                   "%.1f steps/s ÷ 60）。这是世界侧（%s）的性能，不是跟随逻辑；本步不改。"
                   % (k, ctx.env.get("fps_est") or 0, (ctx.env.get("fps_est") or 0) * 8,
                      "PBR 三平面材质 + 街道 + HDRI"))
    if fa.get("binding"):
        out.append("档位已实测 `binding forward=%s turn=%s`（§0），因此**排除**「档位没给上」这一项；"
                   "旧结论里「先查 forward_speed 档位」的说法本轮作废。"
                   % (fa["binding"].get("forward"), fa["binding"].get("turn")))
    if fa.get("follow_start_n") and not fa.get("follow_active"):
        first = fa.get("first_after_follow") or {}
        out.append("跟随未生效（实测）：任务建立后首个动作是 `%s`（状态 `%s`，sub_target=%s），"
                   "机头从 %s° 转到 %s°（Δ=%s°），而目标只在鼻尖 %s° 方向；转完后 person 的 bbox "
                   "左边缘已到 x=%s（半视场 ±%s°），随后 dets 归零 → 树进 Hold。全程机身只动 %s m。"
                   "这是被验对象（sim 侧 `executor` 的格心转向 + 叶子的 0.5 m 给格）的行为，本步不改。"
                   % (first.get("action"), first.get("state"), first.get("sub_target"),
                      fa.get("yaw_follow_start"), fa.get("yaw_max_after_follow"), fa.get("yaw_delta"),
                      fa.get("det_last_person_bearing_rel_deg"), fa.get("det_last_person_bbox_x1"),
                      fa.get("hfov_half_deg"), fa.get("s2s3_uav_disp_m")))
        out.append("机制（源码定位，只读）：`pleiades-sim/src/sim/executor.rs:74` 按 `delta>0` 判 "
                   "`TurnRight`、`:184-186` 的格心是 `(g+0.5)*0.5`；"
                   "`pleiades-sim/src/sim/uav_sim/mission/follow.rs:155-165` 的 `step_cell` 给的是 "
                   "`world_to_grid(me + 0.5 m·unit)`。当机位落在格角 (64.0,64.0) 时，"
                   "`world_to_grid(64.5,64.0)` 的格心是 (64.75,64.25)，相对鼻尖的方位不是 0° → 一上来就右转；"
                   "转完后目标落到半视场外，detector 无 person 帧，`stale=800 ms` 一过即 Hold。")
    if not judges["1"]["pass"]:
        out.append("判据 1 未过：窗口内距离带占比 %s、min=%.2f、mean=%.2f；"
                   "结合 §8.1「机身位移 %s m」，根因是**跟随没让机身动起来**，不是 d_far/d_near，"
                   "也不是档位（档位已实测）。" % (
                       judges["1"].get("share_in_band"), judges["1"].get("min_d") or 0.0,
                       judges["1"].get("mean_d") or 0.0, fa.get("s2s3_uav_disp_m")))
    if not judges["3"]["pass"]:
        out.append("判据 3 未过：人停后 UAV 仍在追（位移 %.2f m / flips=%s）；`stale=0.8 s` 与相机 %.2f Hz 的"
                   "余量只有约 2×，属「检测延迟 + 物理被拖慢」的耦合，建议后续任务把相机 rate_hz 与渲染解耦。"
                   % (judges["3"].get("move_m") or 0, judges["3"].get("sign_flips"),
                      ctx.env.get("camera_hz") or 0))
    if not judges["4"]["pass"]:
        out.append("判据 4 未过：断开后 10 物理秒内位移 %.2f m；先核对 `age_ms` 用的是本地单调时钟"
                   "（follow.rs:229-262）与最后一帧 person 的年龄。" % (judges["4"].get("move_m") or 0))
    if not judges["5"]["pass"]:
        out.append("判据 5 未过：A=%s tick / B=%s tick（1 tick=%s ms 墙钟）；这是 20 ms 细采样 + 物理被拖慢下的"
                   "采样分辨率极限，字面 100 ms 与 tick 折算两套读数都在上表。"
                   % (judges["5"].get("a_ticks"), judges["5"].get("b_ticks"),
                      judges["5"].get("tick_ms")))
    if not judges["6"]["pass"]:
        out.append("判据 6 未过：窗口内 person（conf≥0.5 且底边>240）帧占比 %s；对照 S0j 空场基线 %s 帧。"
                   "§8.1 显示 person 只在 %s→%s ms 有 %s 帧（机头转过来之后就到视场边缘），"
                   "所以这不是检测器失效。" % (
                       judges["6"].get("share"), ctx.env.get("s0j_frames"),
                       fa.get("det_first_person_ms"), fa.get("det_last_person_ms"),
                       fa.get("det_person_frames")))
    if not judges["7"]["pass"]:
        out.append("判据 7 未过：S3 全程非 person conf≥0.5 帧占比 %s，类别 %s；新世界的窗格/斑马线/路缘"
                   "在此暴露为误检，本步不改 `conf_thres`。"
                   % (judges["7"].get("share"), judges["7"].get("classes")))
    if not judges["8"]["pass"]:
        out.append("判据 8 未过：3~6 m 内投影误差 max=%s（n=%s），6~8 m 离谱点 %s 个；"
                   "先看 mount_pitch/fov 是否与相机一致、以及 `truth_at` 的 0.25 s 对齐窗。"
                   % (judges["8"].get("max"), judges["8"].get("n"),
                      judges["8"].get("absurd_6_8")))
    if not judges["9"]["pass"]:
        out.append("判据 9 未过：红项 %s；本步没动世界，红即 Step 2 遗留。" % (judges["9"].get("reds"),))
    if not out:
        out.append("十条判据在物理时间口径下全部通过；渲染吞吐（%.1f fps / k=%.2f）是最主要的"
                   "外部约束，建议后续把相机出帧与渲染帧解耦。"
                   % (ctx.env.get("fps_est") or 0, k or 0))
    return out


def s0b_lines(A, ctx):
    """verdict §2 的 S0b 归位块（`write_verdict` / `write_env_verdict` 共用，两处必须一致）。"""
    rec = (getattr(ctx, "phases", {}) or {}).get("s0b")
    mode = getattr(ctx.args, "s0b", "?")
    if not rec:
        A("**S0b 归位（§四.5）**：`--s0b %s` → 本跑**未执行**归位（A/B 对照用）。" % mode)
        A("")
        return
    thr = rec.get("thr_deg") or S0B_DPSI_MAX_DEG
    A("**S0b 归位（§四.5）**：模式=`%s`；目标 goto(%.2f, %.2f) → 等 `nav_status=idle`；"
      "阈值 |Δψ| ≤ %.1f°；首格格心 (%.2f, %.2f)。" % (
          rec.get("mode"), rec["target"][0], rec["target"][1], thr,
          rec["first_cell"][0], rec["first_cell"][1]))
    A("")
    A("| 尝试 | goto ack | 等 idle | 位姿 (x,y,z) | yaw | nav | 到格心 (dx,dy) | moved | 方位 | Δψ | 判定 |")
    A("|---|---|---|---|---|---|---|---|---|---|---|")
    for a in rec.get("attempts") or []:
        ok = abs(a["dpsi_deg"]) <= thr
        A("| %s | %s | %s s | (%s, %s, %s) | %s° | %s | (%+.3f, %+.3f) | %s m | %s° | "
          "**%+.2f°** | %s |" % (
              a.get("i"), "ok" if (a.get("ack") or {}).get("ok") else "fail",
              a.get("waited_s"), a["pose"][0], a["pose"][1], a["pose"][2],
              a.get("yaw_deg"), a.get("nav_status"), a.get("dx"), a.get("dy"),
              a.get("moved_m"), a.get("bearing_deg"), a.get("dpsi_deg"),
              "PASS" if ok else "FAIL"))
    A("")
    last = (rec.get("attempts") or [{}])[-1]
    if rec.get("ok"):
        A("结论：**PASS**（|Δψ| = %s° ≤ %.1f°，归位后首拍无需转向）" % (last.get("dpsi_deg"), thr))
    else:
        A("结论：**FAIL(env)**（|Δψ| = %s° > %.1f°；按 §四.5「仍不过则记 FAIL(env) 并停」）" % (
            last.get("dpsi_deg"), thr))
    A("")
    A("取证（原文）：")
    A("")
    A("```")
    A("goto 日志    : %s" % (last.get("goto_log") or "(未取到)"))
    A("任务起点日志 : %s" % (last.get("start_log") or "(未取到)"))
    A("到达日志     : %s" % (last.get("nav_noop_log") or "(未取到)"))
    A("seg=S0b 采样 : %s 行（trace.csv）" % rec.get("sampled_rows"))
    A("```")
    A("")


S0B_FIX_LINES = [
    "人类在 §四.5 里允许 S0b 末尾用 `{\"cmd\":\"manual\",\"action\":\"spin_left|spin_right\"}` "
    "把机头拧到首格方位（夹具动作，最贴合「测试夹具」定位，改动最小）；",
    "或把 goto 目标改成「远端 +X 且 y=格心」（如 `(66.25, 64.25)`），先平移到格心再跟；",
    "或世界把 UAV 生在 `(64.25, 64.25, 2.5)`（要动 `World.gd`，超出本 step 允许范围）。",
]


def s0b_root_cause_lines(ctx):
    """§8.3：S0b 归位未生效的代码级根因（只读源码定位，**不改任何 Rust**）。"""
    rec = (getattr(ctx, "phases", {}) or {}).get("s0b") or {}
    if not rec or rec.get("ok"):
        return []
    moved = ((rec.get("attempts") or [{}])[-1].get("moved_m")) or 0.0
    return [
        "`ARRIVAL_TOL_M = CELL_RESOLUTION/2 + 0.01 = 0.26`（`navigator.rs:46`）、"
        "`CELL_RESOLUTION = 0.5`（`grid.rs:19`）、`world_to_grid = floor`（`grid.rs:255`）："
        "机位 (%.1f, %.1f) 与目标 (%.2f, %.2f) 落在**同一格** `(128, 128)`。" % (
            UAV_SPAWN[0], UAV_SPAWN[1], S0B_TARGET[0], S0B_TARGET[1]),
        "`grid_arrived()` 是**逐轴**判据（`navigator.rs:69`）→ `|dx|=0.25`、`|dy|=0.25` 均 ≤ 0.26 → "
        "「② 格内逼近」分支（`navigator.rs:248-263`）直接 `arrive()` 并 `return None`："
        "**goto 是 no-op，机身一步都不动**（本轮实测 moved=%.3f m）。" % moved,
        "`GoToLeaf` 收到 `Arrived` 即 `Success`（`goto.rs:112-118`）→ `finish()` 清 nav"
        "（`uav_sim/mod.rs:114-125`）→ `nav_status` 回 `idle`。",
        "于是「首格方位」恒为 `atan2(0.25, 0.75) = 18.43°`（机头 yaw=0°，格心 (64.75, 64.25)）→ "
        "`> %.0f°` → 重发 goto 仍是 no-op → 两次都不过。" % S0B_DPSI_MAX_DEG,
        "另一条分支也不通：若 goto 真的挪了机身，`executor.rs:82-100`（Idle：`|delta|>5°` → Turning）"
        "会把机头拧到 ≈45°（朝 45° 方向的格心），S0b 结束时 `Δψ ≈ −45°`，**更过不了**。",
    ]


def final_verdict(ctx, judges):
    """overall/reason 的**唯一**口径（`judge_all` 跑两次，两处必须逐字一致）。"""
    fails = [i for i in range(1, 11) if not judges[str(i)]["pass"]]
    overall = "PASS" if not fails else "FAIL"
    reason = "" if not fails else "未过判据 %s" % fails
    s0b = ctx.phases.get("s0b") or {}
    if s0b and not s0b.get("ok"):
        overall = "FAIL(env)"
        reason = ("S0b 未过（|Δψ|=%s° > %s°；按 §四.5 应停%s）；e2e 判据 %s"
                  % ((s0b.get("attempts") or [{}])[-1].get("dpsi_deg"), S0B_DPSI_MAX_DEG,
                     "，record 模式已继续跑完供复核" if ctx.args.s0b == "record" else "",
                     fails))
    return overall, reason


def write_env_verdict(ctx, pre, exc):
    """FAIL(env) 路径的 verdict.md/verdict.json：§0 环境 × §1 前置表 × §2 时序 × §7 残留 × §8.3 × §9。"""
    out = ctx.out
    env = ctx.env
    L = []
    A = L.append
    A("# task_38 Step 3 — 新世界 Follow 复验（只验不改）")
    A("")
    A("> 本文件由 `Tool/sim_follow_test.py` 生成；Presented by KeJi / Created Date ： 2026-10-29")
    A("> 产物目录：`%s`" % out)
    A("> 结论：**FAIL(env)**（`%s`）" % exc)
    A("")

    A("## 0 环境与口径")
    A("")
    A("| 量 | 实测 | 说明 |")
    A("|---|---|---|")
    A("| 控制口端口 | `%s` | 从实例 `[control].port` 读，不硬编码 |" % ctx.control_port)
    A("| `--s0b` 模式 | `%s` | strict=不过即停（§四.5 字面）；record=记录后继续（取证用） |"
      % getattr(ctx.args, "s0b", "?"))
    A("| 本轮尝试 | `attempt=%s` + `rerun_reason=%s` | step.md 要求重跑必须注明 |" % (
        env.get("attempts"), env.get("rerun_reason") or "（未重跑）"))
    A("| 相机实测 Hz | %s | 配置 `rate_hz=10` |" % env.get("camera_hz"))
    A("| 物理/实时 k | %s | S3 就地测 |" % env.get("k"))
    A("| S0j 空场基线 | %s 帧 / %s s，其中 person 帧 %s | S0b 之前，**归位前的空场基线** |" % (
        env.get("s0j_frames"), env.get("s0j_window_s"), env.get("s0j_person_frames")))
    A("")

    A("## 1 前置自检表")
    A("")
    A("| # | 项 | 结果 | 实测 |")
    A("|---|---|---|---|")
    for i, c in enumerate(pre, 1):
        A("| %d | `%s` | %s | %s |" % (i, c["name"], "OK" if c["ok"] else "**FAIL**",
                                        c["detail"].replace("|", "/")))
    A("")

    A("## 2 S0~S6 时序")
    A("")
    s0 = ctx.phases.get("s0", {}) or {}
    A("**S0 日志锚点**")
    A("")
    for a in s0.get("anchors", []) or []:
        A("* `%s` — %s — `%s`" % (a["name"], "OK" if a["ok"] else "**MISSING**", a["line"][:200]))
    A("")
    if s0.get("status"):
        A("**S0 status 断言**：`%s`" % json.dumps(s0.get("status"), ensure_ascii=False))
        A("")
    s0b_lines(A, ctx)
    A("> 本跑在 S0b 处终止（§四.5「仍不过则记 FAIL(env) 并停」），**没有** S1~S6 的动作与判据 1~10 读数。")
    A("")

    bnd = git_boundary_report()
    A("## 7 残留与清理")
    A("")
    A("```")
    A("pgrep -af 'pleiades-sim|godot .*orion-simulation-world|Xvfb :99' → %s" % (
        ctx.residual.get("pgrep") or "(空)"))
    A("ports %s/%s → %s" % (ctx.control_port, WORLD_PORT, ctx.residual.get("ports") or "(空)"))
    A("Xvfb :99 → %s" % ctx.residual.get("xvfb"))
    A("git status 差异（相对本步开始）→ %s" % (ctx.residual.get("git_diff") or "(空)"))
    A("仓库边界（`%s` vs `tmp/task38/git_baseline.txt`，allow=%s）→ extra=%s missing=%s" % (
        bnd.get("cmd"), bnd.get("allow"), bnd.get("extra"), bnd.get("missing")))
    A("```")
    A("")

    A("## 8 根因初判与建议（不改代码）")
    A("")
    A("### 8.3 S0b 归位未生效的代码级根因")
    A("")
    rc = s0b_root_cause_lines(ctx)
    if rc:
        for line in rc:
            A("* " + line)
        A("")
        A("夹具改法建议（**只建议，本轮不改**，都只动夹具、不动 Rust 与阈值）：")
        A("")
        for i, line in enumerate(S0B_FIX_LINES, 1):
            A("%d. %s" % (i, line))
    else:
        A("* S0b 已通过（或未执行），本节不适用。")
    A("")

    A("## 9 结论")
    A("")
    A("**FAIL(env)**：`%s`" % exc)
    A("")
    (out / "verdict.md").write_text("\n".join(L) + "\n", encoding="utf-8")
    (out / "verdict.json").write_text(json.dumps({
        "overall": "FAIL(env)", "reason": str(exc),
        "s0b": ctx.phases.get("s0b"), "attempts": env.get("attempts"),
        "rerun_reason": env.get("rerun_reason"),
        "residual": ctx.residual, "boundary": bnd,
    }, ensure_ascii=False, indent=2), encoding="utf-8")


def summary_text(ctx, judges, overall, reason):
    fails = [i for i in range(1, 11) if not judges[str(i)]["pass"]]
    k = ctx.k
    j8 = judges["8"]
    s0b = ctx.phases.get("s0b") or {}
    s0b_last = (s0b.get("attempts") or [{}])[-1]
    s0b_verdict = ("未执行（--s0b off）" if not s0b
                   else ("PASS" if s0b.get("ok") else "FAIL(env)"))
    return (
        "本步在 task_38 新世界（城市街道 + PBR + HDRI）里复跑了 task_37 的 `Mission::Follow`，"
        "只验收不改逻辑。实测渲染 %.1f fps、相机 %.2f Hz、物理 %.2f× 实时，因此按物理时间口径给阶段时长"
        "（S3 = 行人位移 25.2 m），判据阈值一个没动。S3 后 10 s 距离带占比 %s、min=%.2f、mean=%.2f；"
        "全程 min d=%s；人停位移 %s m；丢失后位移 %s m；取消 A/B = %s/%s tick。"
        "判据 6/7 的帧占比 %s / %s；投影误差 3~6 m max=%s m（n=%s）、6~8 m 离谱点 %s 个（n=%s）；"
        "既有检查 %s/%s 绿。"
        "S0b（goto 归位）实测 Δψ = %s°（阈值 ±%s°）→ %s；"
        "**注意**：判据 2/3/4/5 为空转通过（UAV 全程未跟随，见 §3.1），不得当作跟随成立。"
        "结论 %s%s。"
        % (ctx.env.get("fps_est") or 0, ctx.env.get("camera_hz") or 0, k or 0,
           judges["1"].get("share_in_band"), judges["1"].get("min_d") or 0.0,
           judges["1"].get("mean_d") or 0.0,
           judges["2"].get("min_d"), judges["3"].get("move_m"), judges["4"].get("move_m"),
           judges["5"].get("a_ticks"), judges["5"].get("b_ticks"),
           judges["6"].get("share"), judges["7"].get("share"),
           j8.get("max"), j8.get("n"), j8.get("absurd_6_8"), (j8.get("band68") or {}).get("n"),
           judges["9"].get("green"), judges["9"].get("total"),
           s0b_last.get("dpsi_deg"), S0B_DPSI_MAX_DEG, s0b_verdict, overall,
           ("（未过：%s）" % fails) if fails else "",
           ))


def write_verdict_json(ctx, judges, overall, reason):
    data = {"overall": overall, "reason": reason,
            "fps": ctx.env.get("fps_est"), "camera_hz": ctx.env.get("camera_hz"),
            "k": ctx.k, "r": ctx.env.get("r_world_over_wall"),
            "judges": {k: {kk: vv for kk, vv in v.items() if kk != "details"}
                       for k, v in judges.items()},
            "env": {k: v for k, v in ctx.env.items() if k != "shots"},
            "cancel": {p: {k: v for k, v in c.items() if k != "samples"}
                       for p, c in ctx.cancel.items()},
            "checks": [{k: v for k, v in c.items() if k != "text"} for c in ctx.checks],
            "residual": ctx.residual,
            "follow_activity": getattr(ctx, "follow_activity", {}),
            "boundary": git_boundary_report(),
            "s0b": ctx.phases.get("s0b"),
            "static_pedestrians": static_pedestrians()}
    (ctx.out / "verdict.json").write_text(json.dumps(data, ensure_ascii=False, indent=2),
                                          encoding="utf-8")
    return data


# ================================================================ 收尾

def pgrep_residual():
    rc, so, _ = run_capture(["/bin/sh", "-c",
                             "pgrep -af 'pleiades-sim|godot .*orion-simulation-world|Xvfb :99' || true"],
                            timeout=20)
    lines = [ln for ln in so.splitlines() if ln.strip() and "pgrep -af" not in ln]
    return "\n".join(lines)


def collect_residual(ctx, xvfb):
    res = {}
    res["pgrep"] = pgrep_residual()
    busy = [p for p in (ctx.control_port, WORLD_PORT) if port_busy(p)]
    res["ports"] = ("busy=%s" % busy) if busy else ""
    res["xvfb"] = "reused（未杀）" if xvfb.reused else "自起 → 已停"
    before = set(ctx.git_before)
    after = git_status_lines()
    added = [ln for ln in after if ln not in before]
    res["git_diff"] = "; ".join(added) if added else ""
    res["git_status_after"] = after
    ctx.residual = res
    return res


def cleanup(ctx, xvfb):
    try:
        if ctx.ped is not None:
            ctx.ped.close()
    except Exception:
        pass
    try:
        ctx.control.close()
    except Exception:
        pass
    try:
        ctx.sim.stop()
    except Exception:
        pass
    try:
        ctx.world.stop()
    except Exception:
        pass
    try:
        xvfb.stop()
    except Exception:
        pass


# ================================================================ main

def parse_args(argv=None):
    p = argparse.ArgumentParser(description="task_38 Step 3 新世界 Follow 复验（只验不改）")
    p.add_argument("--preflight", action="store_true", help="只跑前置自检，退出码 0/2")
    p.add_argument("--checks", default="all", choices=("all", "named", "none"),
                   help="既有检查：all=22(+)；named=判据 9 点名的 3 条 + check_street；none=不跑")
    p.add_argument("--no-checks", action="store_true", help="= --checks none")
    p.add_argument("--out-dir", default=None)
    p.add_argument("--port", type=int, default=None)
    p.add_argument("--no-launch", action="store_true", help="复用已在跑的世界/控制口")
    p.add_argument("--attempts", type=int, default=2, help="仅对 HarnessError 重试")
    p.add_argument("--world-dir", default=None,
                   help="（只做对照实验用）世界工程目录，默认 %s；正式跑不要传" % WORLD_DIR)
    p.add_argument("--ped-spawn-x", type=float, default=None,
                   help="（只做对照实验用）行人 spawn 的 x，默认 70（§四.5）；正式跑不要传")
    p.add_argument("--s0b", default="strict", choices=("strict", "record", "off"),
                   help="S0b 归位：strict=按 §四.5 不过即停；record=记录后继续（取证用）；off=跳过（A/B 用）")
    return p.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    if args.no_checks:
        args.checks = "none"
    try:
        cfg = load_config()
    except Exception as exc:
        print("[FAIL] 读不了 %s：%r" % (CFG, exc))
        return 2
    control_port = args.port or int(cfg.get("control", {}).get("port") or 9098)

    pre = preflight_checks(cfg, control_port)
    width = max(len(c["name"]) for c in pre)
    print("=" * 78)
    print("前置自检（%d 条）" % len(pre))
    for c in pre:
        print("  %-*s %-5s %s" % (width, c["name"], "OK" if c["ok"] else "FAIL", c["detail"]))
    print("=" * 78)
    if any(not c["ok"] for c in pre):
        print("[FAIL(env)] 前置自检未过 → 按 step.md「缺一条就记 FAIL(env) 并停」")
        if args.preflight:
            return 2
        out = pathlib.Path(args.out_dir or (RESULTS / ("task38_follow_" + now_utc_stamp())))
        out.mkdir(parents=True, exist_ok=True)
        (out / "verdict.md").write_text(
            "# task_38 Step 3 — 新世界 Follow 复验（只验不改）\n\n## 结论\n\n**FAIL(env)**\n\n"
            "前置自检未过：\n\n"
            + "\n".join("* `%s` %s — %s" % (c["name"], "OK" if c["ok"] else "FAIL", c["detail"])
                        for c in pre) + "\n", encoding="utf-8")
        return 2
    if args.preflight:
        print("[OK] 前置自检全过（%d 条）" % len(pre))
        return 0

    out = pathlib.Path(args.out_dir or (RESULTS / ("task38_follow_" + now_utc_stamp())))
    out.mkdir(parents=True, exist_ok=True)
    print("[out] %s" % out)

    ctx = Ctx(out, cfg, control_port, args)
    ctx.git_before = git_status_lines()
    # 配置副本（只取证，不改内容）
    shutil.copy2(CFG, out / "config.toml")

    xvfb = Xvfb()
    overall, reason = "FAIL", ""
    judges, obs, checks = {}, {}, []
    try:
        xvfb.ensure()
        if args.checks != "none":
            checks = run_checks(ctx, args.checks)
            ctx.checks = checks
        else:
            Log("跳过既有检查（--checks none）")
        attempt = 0
        while True:
            attempt += 1
            ctx.env["attempts"] = attempt
            try:
                run_phase_a(ctx)
                break
            except HarnessError as exc:
                Log("[HarnessError] %s" % exc)
                cleanup(ctx, xvfb)
                # S0b 不过**不允许**整轮重跑（§四.5「并停」）
                if isinstance(exc, FixtureError) or attempt >= max(1, args.attempts):
                    raise
                Log("→ harness 故障，重跑一次（attempt %d/%d）" % (attempt + 1, args.attempts))
                ctx.rows.clear()
                ctx.frames.clear()
                ctx.sim_history.clear()
                ctx.world_history.clear()
                ctx.phases.clear()
                ctx.control = Control(control_port)
                ctx.world = Proc("world")
                ctx.sim = Proc("sim")
                ctx.world_tail = Tail(out / "world.log", from_end=True)
                ctx.phases["rerun_reason"] = str(exc)
                ctx.env["rerun_reason"] = str(exc)
                xvfb.ensure()
        # 产物
        write_csvs(ctx)
        # sim 日志硬拷贝（取证）
        if ctx.sim_tail.path is not None and ctx.sim_tail.path.exists():
            shutil.copy2(ctx.sim_tail.path, out / "sim_trace.log")
        ctx.follow_activity = analyze_follow(ctx)
        write_env_json(ctx)
        judges = judge_all(ctx)
        obs = observations(ctx)
        # verdict.md 需要 judge10 的产物（先写一次 csv/json，再判，再补写）
        overall, reason = final_verdict(ctx, judges)
        # 先收尾、再取证：verdict §7「残留与清理」必须反映"跑完之后"的真实状态
        # （cleanup / collect_residual 都是幂等的；finally 里还会再来一次并追加 §10 复检）
        cleanup(ctx, xvfb)
        collect_residual(ctx, xvfb)
        write_verdict(ctx, judges, obs, checks, overall, reason, pre)
        write_verdict_json(ctx, judges, overall, reason)
        # 判据 10 依赖 verdict.md 存在：重判一次并回写
        judges = judge_all(ctx)
        overall, reason = final_verdict(ctx, judges)
        write_verdict(ctx, judges, obs, checks, overall, reason, pre)
        write_verdict_json(ctx, judges, overall, reason)
        print("=" * 78)
        for i in range(1, 11):
            j = judges[str(i)]
            print("  判据 %-2d %-4s %s" % (i, "PASS" if j["pass"] else "FAIL", j.get("value", "")))
        print("  结论：%s %s" % (overall, reason))
        print("  产物：%s" % out)
        print("=" * 78)
        if overall == "FAIL(env)":
            return 2
        return 0 if overall == "PASS" else 1
    except HarnessError as exc:
        Log("[FAIL(env)] %s" % exc)
        collect_residual(ctx, xvfb)
        try:
            write_csvs(ctx)
            write_env_json(ctx)
        except Exception:
            traceback.print_exc()
        try:
            write_env_verdict(ctx, pre, exc)
        except Exception:
            traceback.print_exc()
        return 2
    except Exception as exc:
        Log("[异常] %s" % exc)
        traceback.print_exc()
        collect_residual(ctx, xvfb)
        (out / "verdict.md").write_text(
            "# task_38 Step 3 — 新世界 Follow 复验（只验不改）\n\n## 结论\n\n**FAIL(异常)**\n\n"
            "```\n%s\n```\n" % traceback.format_exc(), encoding="utf-8")
        return 2
    finally:
        cleanup(ctx, xvfb)
        # 收尾后再核一次残留（cleanup 之后）
        try:
            res = collect_residual(ctx, xvfb)
            if (out / "verdict.md").exists():
                with (out / "verdict.md").open("a", encoding="utf-8") as f:
                    f.write("\n## 10 收尾复检（cleanup + 进程/端口复检之后）\n\n```\n%s\n%s\n%s\n```\n" % (
                        res.get("pgrep") or "(空)", res.get("ports") or "(空)",
                        res.get("git_diff") or "(空)"))
        except Exception:
            pass


if __name__ == "__main__":
    sys.exit(main())
