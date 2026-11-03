#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-11-03
# Modified Date ： 2026-11-03

r"""行人常驻控制台（task_46 §二 Step 2）：一个进程 <-> 一个行人，启动即连世界、长期存活。

为什么需要它
    仿真里的行人 avatar 生命周期 = 它那条 WS 连接的生命周期。过去「每次让人走一下」都要重开一个
    pedestrian_client，于是人一会儿出现一会儿消失、peer_id 一变 Follow 就丢目标。本工具把
    「连世界 + 30 Hz 持续发 motion_cmd」做成常驻进程，动作只经本机控制口改写，人一直在场。

分工（不重写传输层）
    传输 / 握手 / 发送节拍 / pose 收帧全部交给同目录的 `PedestrianClient`；本文件只做三件事：
      ① 一个 20 Hz 控制环 + 一条 30 Hz 发送节拍（两条节拍解耦）：pump -> 到点才闭环决策 ->
         tick -> sleep_to_next。控制一拍发一帧，其余拍由 tick() 按 30 Hz 补齐保活；
      ② 一个 goal 状态机（闭环 / 阻塞 / 抢占 / 冻结检测）；
      ③ 一个 TCP 控制口（UTF-8 JSON-lines，一问一答）。

控制口契约
    监听 : TCP 127.0.0.1:9101（--port 可改；只回环，无鉴权）
    编码 : UTF-8，一行一条 JSON（以 \n 结尾），一问一答
    请求 : {"id": <任意 JSON 标量，缺省 null>, "cmd": "<name>", ...}
    成功 : {"id": <原样回显>, "ok": true,  "data": {...}}
    失败 : {"id": <原样回显>, "ok": false, "error": "<code>", "why": "<人类可读>"}

    cmd      参数（缺省）                               语义
    -------- ------------------------------------------ ------------------------------------------
    goto     x, y 必需；speed=1.0 tol=0.15 timeout=60   闭环走到点；阻塞到 到位 / 超时 / 冻结 / 被抢占
    face     x, y 必需                                  原地转向朝该点（v=0），立即返回
    walk     v, dir 必需                                开环直走，立即返回
    stop     —                                          v=0 并清 goal，立即返回
    pos      —                                          当前位姿 x / y / yaw / v / pose_age
    status   —                                          peer_id / connected / goal / pose_age / send_hz
    quit     —                                          drive(0) -> 关连接 -> 退出进程

    goto  返回 data: {"pos": [x, y], "d": <米>, "seconds": <秒>}
    face  返回 data: {"dir": <世界系弧度>}
    pos   返回 data: {"x", "y", "yaw", "v", "pose_age"}
    status返回 data: {"peer_id", "connected", "goal", "pose_age", "send_hz"}
          goal = null 或 {"x", "y", "speed", "tol", "remaining_s"}
    walk / stop / quit 返回 data: {}

    错误码：bad_json | unknown_cmd | bad_arg | not_ready | timeout | frozen | preempted | shutdown

    错误码优先级（一处定死，避免「同一请求两种答案」）：
        bad_json -> unknown_cmd -> bad_arg -> not_ready -> 执行
    `quit` 是唯一例外：过了 unknown_cmd 就直达执行（否则无世界时无法干净退出，只能 Ctrl-C）。
    这是任务文档 §二「行为细节 5」的**有意例外**：⑤ 的目的只是「指令不许把进程弄挂」，
    而 `quit` 恰是唯一的干净退出通道，把它挡成 not_ready 会让控制台在无世界时无法收尾。

用法
    # 1) 常驻起一个行人（先起它，再注入 Follow；实验期间不要退这个进程）
    python3 ped_console.py --peer-id human-01 --spawn 64,56,0,90

    # 2) 另一个终端随时指挥他
    echo '{"id":1,"cmd":"goto","x":70,"y":66}' | socat - TCP:127.0.0.1:9101
    echo '{"id":2,"cmd":"pos"}'                | socat - TCP:127.0.0.1:9101
    echo '{"id":3,"cmd":"face","x":64,"y":56}' | socat - TCP:127.0.0.1:9101

    # 3) 收工（先 drive(0) 再关连接，世界日志出现一次 pedestrian removed）
    echo '{"id":9,"cmd":"quit"}'               | socat - TCP:127.0.0.1:9101

边界与已知限制（任务文档 §二 要求写进交付说明）
    1. 进程即生命（设计如此）：一个 ped_console 对应一个行人，进程退出 = 这个人离场
       （世界日志 `pedestrian removed`）。实验约定是「先起 console、再注入 Follow，期间不退进程」；
       这不是缺陷，不需要归属解耦。
    2. v >= 0，不能倒着走；goto 目标在身后时靠 dir 先原地转（控制环 |Δθ| > 70° 门限）。
    3. dir 是**世界系绝对朝向**（0 = +X 东，逆时针为正），不是角速度；转向速率由世界侧
       max_turn_rate（180°/s）限速。取反只在世界侧做，本文件不做。
    4. 世界重启 / socket 断 -> 本进程退出 -> 人随之离场；重起 console 即为「换了一个人」
       （新 peer_id，Follow 需重新注入）。这属于环境重建，本工具**不实现断线重连**。
    5. 控制口无鉴权，只绑 127.0.0.1（--host 只接受回环地址）。
    6. 同一时刻只有一个 goal；任何写指令（goto / face / walk / stop）都会抢占前一个，
       被抢占者返回 error:"preempted"。
    7. 世界断链时底层 ws 的关闭异常统一翻译成 ProtocolError（不裸奔 traceback）：主循环打一行
       [error]、退出码 4，阻塞中的 goto 收口为 error:"shutdown"。
    8. CLI 先校验再连世界：--max-speed 必须 > 0、--port 限 1~65535，越界打一行 [error] 并 rc=2，
       既不监听也不连世界（否则非法值会被静默夹没，表现为「人不动且不报错」）。

退出码（沿用 pedestrian_client 常量）：0 正常（含 quit）/ 2 控制口或参数错误 /
    4 协议异常（世界断链）/ 5 spawn 被拒 / 130 SIGINT。
"""

import argparse
import json
import math
import signal
import socket
import sys
import threading
import time

# 同目录直接 import（脚本方式运行时 sys.path[0] 就是本目录；被 -m 调起来时兜一下）
_HERE = __file__.rsplit("/", 1)[0]
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from pedestrian_client import (  # noqa: E402  （本地模块，必须在 sys.path 之后）
    DEFAULT_SEND_HZ, DEFAULT_URL, EXIT_CONNECT, EXIT_INTERRUPTED, EXIT_OK, EXIT_PROTOCOL, EXIT_SPAWN,
    READY_TIMEOUT_S, ClientError, ConnectError, PedestrianClient, ProtocolError,
    ReadyTimeout, SpawnFallback, Wrap_Pi, parse_spawn_spec,
)
from ws_min import WsClosed  # noqa: E402  （发送失败要按协议异常收口，不裸奔 traceback）

# ---------------------------------------------------------------- 常量
DEFAULT_HOST = "127.0.0.1"
## 只允许回环：本工具不对外监听，也不做鉴权。
LOOPBACK_HOSTS = ("127.0.0.1", "localhost", "::1")
DEFAULT_PORT = 9101
DEFAULT_PEER_ID = "human-01"

## 世界硬上限（Pedestrian.gd:47 DEFAULT_MAX_SPEED）：负值一律当 0，本文件所有入口先自行 clamp。
WORLD_MAX_SPEED = 1.26
DEFAULT_GOTO_SPEED = 1.0
DEFAULT_GOTO_TOL = 0.15
DEFAULT_GOTO_TIMEOUT_S = 60.0

## 闭环增益：v = min(speed, d * POS_GAIN)（任务文档 §二「形态」逐字，不加 v 下限）。
POS_GAIN = 2.0
## 控制决策节拍（任务文档 §二「行为细节 1」= 20 Hz）；发送节拍仍是 client 的 30 Hz。
## 两条节拍解耦：控制一拍发一帧，其余拍由 tick() 补齐，世界 300 ms 空闲停车永远打不到。
CONTROL_HZ = 20.0
CONTROL_PERIOD_S = 1.0 / CONTROL_HZ
## |Δθ| 超过它就先原地转（v 不能为负，只能靠转把目标掰到身前）。
TURN_FIRST_DEG = 70.0
TURN_FIRST_RAD = math.radians(TURN_FIRST_DEG)

## 冻结检测：下过 v>0 但位姿这么久没位移 -> goto 返回 frozen（step3_driver.walk_to 同款判据）。
FROZEN_WINDOW_S = 3.0
FROZEN_MOVE_EPS_M = 0.05
## 一行上限，防对端刷爆（超了直接 bad_json 并断这条连接）。
MAX_LINE_BYTES = 64 * 1024
## goto 等待者的轮询粒度（同时用于发现 shutdown）。
WAIT_POLL_S = 0.2
## 等待兜底：deadline 过这么久还没结果 -> timeout 自保（控制环意外停了也不会永远挂着）。
WAIT_GUARD_S = 5.0
LOG_PERIOD_S = 1.0
ACCEPT_TIMEOUT_S = 0.2
## 未就绪（客户端没有 transport）时的空转粒度：此时**不许** tick()。
IDLE_POLL_S = 0.1

ERR_BAD_JSON = "bad_json"
ERR_UNKNOWN_CMD = "unknown_cmd"
ERR_BAD_ARG = "bad_arg"
ERR_NOT_READY = "not_ready"
ERR_TIMEOUT = "timeout"
ERR_FROZEN = "frozen"
ERR_PREEMPTED = "preempted"
ERR_SHUTDOWN = "shutdown"

CMDS = ("goto", "face", "walk", "stop", "pos", "status", "quit")

## SIGINT / SIGTERM：处理函数里只置标志，收尾仍在主循环的 finally 里做（patrol.py 同款）。
_INTERRUPTED = False


def _On_Signal(signum, frame) -> None:
    global _INTERRUPTED
    _INTERRUPTED = True


# ---------------------------------------------------------------- 类型
class BadArg(ValueError):
    """参数语法非法 -> bad_arg。纯语法，与世界无关，因此未就绪时也能先报出来。"""


class Goal:
    """一个 goto 的全部状态。result 只写一次（锁内），命中即 ev.set()。"""

    __slots__ = ("gid", "x", "y", "speed", "tol", "timeout", "t0", "deadline",
                 "ev", "result", "freeze_ref")

    def __init__(self, gid, x, y, speed, tol, timeout, t0):
        self.gid = int(gid)
        self.x, self.y = float(x), float(y)
        self.speed, self.tol = float(speed), float(tol)
        self.timeout = float(timeout)
        self.t0 = float(t0)
        self.deadline = self.t0 + self.timeout
        self.ev = threading.Event()
        self.result = None                  # {"ok":True,"data":{...}} 或 {"ok":False,"error","why"}
        self.freeze_ref = None              # (t, x, y)：首次下 v>0 时建立，位移 >= 0.05 m 就刷新


# ---------------------------------------------------------------- 参数解析（纯语法）
def _num(req, key, *, default=None, required=True, ctx=""):
    """从请求里取一个有限实数；类型 / 缺失 / 非有限都抛 BadArg（不碰世界）。"""
    if key not in req:
        if required:
            raise BadArg("缺少参数 %s（%s）" % (key, ctx))
        return default
    val = req[key]
    if isinstance(val, bool) or not isinstance(val, (int, float)):
        raise BadArg("%s 必须是数字，收到 %r" % (key, val))
    val = float(val)
    if not math.isfinite(val):
        raise BadArg("%s 必须是有限数，收到 %r" % (key, req[key]))
    return val


def parse_goto(req, max_speed) -> dict:
    """goto x y [speed=1.0] [tol=0.15] [timeout=60]；speed 先验 > 0 再夹到世界上限。"""
    x = _num(req, "x", ctx="goto 的目标点")
    y = _num(req, "y", ctx="goto 的目标点")
    speed = _num(req, "speed", default=DEFAULT_GOTO_SPEED, required=False, ctx="goto")
    if speed <= 0.0:
        raise BadArg("speed 必须 > 0（speed=0 的 goto 等于 stop，只会白等满 timeout），收到 %r"
                     % (req.get("speed"),))
    speed = min(float(speed), float(max_speed))
    tol = _num(req, "tol", default=DEFAULT_GOTO_TOL, required=False, ctx="goto")
    if tol <= 0.0:
        raise BadArg("tol 必须 > 0，收到 %r" % (req.get("tol"),))
    timeout = _num(req, "timeout", default=DEFAULT_GOTO_TIMEOUT_S, required=False, ctx="goto")
    if timeout <= 0.0:
        raise BadArg("timeout 必须 > 0，收到 %r" % (req.get("timeout"),))
    return {"x": x, "y": y, "speed": speed, "tol": tol, "timeout": timeout}


def parse_face(req) -> dict:
    """face x y：原地转向朝该点。"""
    return {"x": _num(req, "x", ctx="face 的朝向点"),
            "y": _num(req, "y", ctx="face 的朝向点")}


def parse_walk(req, max_speed) -> dict:
    """walk v dir：开环直走。v 负值夹 0（世界同口径，不打日志噪音），dir 归一到 [-π, π)。"""
    v = _num(req, "v", ctx="walk 的速度")
    if v < 0.0:
        v = 0.0
    v = min(float(v), float(max_speed))
    dir_rad = Wrap_Pi(_num(req, "dir", ctx="walk 的朝向（世界系弧度）"))
    return {"v": v, "dir": dir_rad}


def parse_noarg(cmd, req) -> dict:
    """stop / pos / status：忽略多余字段（不因未知键失败）。"""
    return {}


# ---------------------------------------------------------------- 控制台
class PedConsole:
    """常驻控制台：30 Hz 主循环兼控制环 + 本机 JSON-lines 控制口。"""

    def __init__(self, client, host, port, max_speed, quiet=False, verbose=False):
        self.client = client
        self.host, self.port = str(host), int(port)
        self.max_speed = float(max_speed)
        self.quiet, self.verbose = bool(quiet), bool(verbose)
        self._lock = threading.RLock()      # 控制状态锁；RLock（_do_goto 持锁时还会再进 _finish_goal）
        self._io = threading.RLock()        # 传输层锁：PedestrianClient 非线程安全，调用一律串行化
        self._goal = None                   # Goal | None；None = 空闲（只保活）
        self._gid = 0
        self._stop = threading.Event()
        self._server = None
        self._listener = None
        self._conns = set()
        self._next_control = 0.0            # 下一拍控制决策的 monotonic 时刻；0.0 = 第一拍立即执行

    # -------------------------------------------------- 小工具
    @staticmethod
    def _err(rid, code, why):
        return {"id": rid, "ok": False, "error": code, "why": why}

    def _log_ctl(self, msg: str) -> None:
        """写指令日志（--quiet 抑制）；错误 / 告警行不经过这里，永远打。"""
        if not self.quiet:
            print("[ctl] %s" % msg)

    def _log_verbose(self, msg: str) -> None:
        if self.verbose:
            print("[ctl] %s" % msg)

    def _drive(self, v, dir_rad=None):
        """所有发帧都走这里：_io 串行化（PedestrianClient 非线程安全）+ 统一日志。"""
        with self._io:
            try:
                self.client.drive(v, dir_rad)
            except WsClosed as exc:         # 世界在这一拍关连接：统一成协议异常
                raise ProtocolError("发送 motion_cmd 失败（世界已断开）：%s" % exc) from exc

    def _pose_xyz(self):
        with self._io:                      # client.pose()/spawn 与主循环的 pump() 串行
            pose = self.client.pose()
            if pose is not None:
                return (float(pose.get("x", 0.0)), float(pose.get("y", 0.0)),
                        float(pose.get("yaw", 0.0)))
            sp = self.client.spawn or {}    # pose 还没到（<=100 ms）：用出生点，别浪费第一拍
            return float(sp.get("x", 0.0)), float(sp.get("y", 0.0)), float(sp.get("yaw", 0.0))

    def _goal_snapshot(self):
        with self._lock:
            return self._goal

    def _due(self, now: float) -> bool:
        """控制节拍（20 Hz）：到点返回 True 并推进；落后 >2 拍重置节拍，不补发突发决策。"""
        if now < self._next_control:
            return False
        if now - self._next_control > CONTROL_PERIOD_S * 2.0:
            self._next_control = now + CONTROL_PERIOD_S
        else:
            self._next_control += CONTROL_PERIOD_S
        return True

    def _parse(self, cmd, req):
        if cmd == "goto":
            return parse_goto(req, self.max_speed)
        if cmd == "face":
            return parse_face(req)
        if cmd == "walk":
            return parse_walk(req, self.max_speed)
        return parse_noarg(cmd, req)        # stop / pos / status

    # -------------------------------------------------- 监听
    def start(self) -> None:
        self._server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._server.bind((self.host, self.port))
        self._server.listen(8)
        self._server.settimeout(ACCEPT_TIMEOUT_S)
        self._listener = threading.Thread(target=self._accept_loop,
                                          name="ped-ctl-accept", daemon=True)
        self._listener.start()
        print("[ctl] 监听 %s:%d（UTF-8 JSON-lines，一问一答；只绑本机）" % (self.host, self.port))

    def _accept_loop(self) -> None:
        while not self._stop.is_set():
            try:
                conn, addr = self._server.accept()
            except socket.timeout:
                continue
            except OSError:
                break                       # 服务器 socket 已关（close_all）
            conn.settimeout(None)           # 阻塞读：一问一答，靠对端换行推进
            with self._lock:
                self._conns.add(conn)
            threading.Thread(target=self._serve_conn, args=(conn, addr),
                             name="ped-ctl-conn", daemon=True).start()

    # -------------------------------------------------- 控制环（主线程）
    def run(self) -> int:
        exit_code = EXIT_OK
        t0 = time.monotonic()
        next_log = t0 + LOG_PERIOD_S
        if not self.quiet:
            print("[ped] 控制环起：peer='%s' send_hz=%.0f port=%d max_speed=%.2f"
                  % (self.client.peer_id, self.client.send_hz, self.port, self.max_speed))
        try:
            while not self._stop.is_set() and not _INTERRUPTED:
                now = time.monotonic()
                try:
                    with self._io:
                        self.client.pump(0.0)              # ① 收 pose（不积压）
                    x, y, yaw = self._pose_xyz()
                    goal = self._goal_snapshot()
                    if goal is not None and self._due(now):
                        with self._io:
                            self._step_goal(goal, x, y, yaw, now)  # ② 有 goal：闭环（内部 _drive 发帧）
                    if self.client.ready:
                        with self._io:
                            try:
                                self.client.tick()             # ③ 保活：无 goal 时由它按 30 Hz 发
                            except WsClosed as exc:             # 世界在 tick 里关连接：同上收口
                                raise ProtocolError("发送 motion_cmd 失败（世界已断开）：%s" % exc) from exc
                        self.client.sleep_to_next()            # ④ 睡到下一拍（只读 _next_send，不持锁）
                    else:
                        # 未就绪：**禁止 tick**。没有 transport 时 _send_motion 会抛 ProtocolError，
                        # 把进程顶出 not_ready 模式（世界没起时本进程必须继续监听）。
                        time.sleep(IDLE_POLL_S)
                except ProtocolError as exc:
                    # 世界在任何一拍里关掉连接（pump / drive / tick 都会抛）：干净退出，不留 traceback，
                    # 阻塞中的 goto 由 finally 的 request_shutdown 收口为 error:"shutdown"。
                    print("[error] 连接被世界关闭：%s" % exc)
                    exit_code = EXIT_PROTOCOL
                    break
                if not self.quiet and self.client.ready and now >= next_log:
                    next_log += LOG_PERIOD_S                          # 未就绪不打进度（只留启动告警）
                    self._log_progress(now - t0, x, y, yaw, goal)
        except KeyboardInterrupt:
            print("[warn] Ctrl-C：先 drive(0) 再关连接")
            exit_code = EXIT_INTERRUPTED
        finally:
            if _INTERRUPTED and exit_code == EXIT_OK:
                exit_code = EXIT_INTERRUPTED
                print("[warn] SIGINT/SIGTERM：先 drive(0) 再关连接")
            self.request_shutdown("进程退出")
            self.close_all()
        return exit_code

    def _log_progress(self, elapsed, x, y, yaw, goal):
        print("[ped] t=%.1fs pos=(%.2f, %.2f) yaw=%.1f° goal=%s poses=%d cmds=%d"
              % (elapsed, x, y, math.degrees(yaw),
                 "无" if goal is None else "(%.2f, %.2f)" % (goal.x, goal.y),
                 self.client.poses_received, self.client.sent_commands))

    # -------------------------------------------------- goal 状态机
    def _step_goal(self, goal, x, y, yaw, now):
        d = math.hypot(goal.x - x, goal.y - y)

        if d <= goal.tol:                                     # ① 到位
            if not self._finish_goal(goal, True, data={
                    "pos": [round(x, 2), round(y, 2)], "d": round(d, 3),
                    "seconds": round(now - goal.t0, 1)}):
                return                                        # 已被抢占：别再发帧盖掉新指令
            self._drive(0.0)                            # 停住，dir 省略 = 保持朝向
            return

        if now >= goal.deadline:                              # ② 超时
            if not self._finish_goal(goal, False, error=ERR_TIMEOUT,
                                     why="%.0f s 内没走到 (%.2f, %.2f)，还差 %.2f m"
                                         % (goal.timeout, goal.x, goal.y, d)):
                return                                        # 已被抢占：同上
            self._drive(0.0)
            return

        err = math.atan2(goal.y - y, goal.x - x)              # 世界系绝对朝向
        if abs(Wrap_Pi(err - yaw)) > TURN_FIRST_RAD:          # ③ 大偏差：原地转（v 不能为负）
            cmd_v, cmd_dir = 0.0, err
        else:
            cmd_v, cmd_dir = min(goal.speed, d * POS_GAIN), err

        if cmd_v > 0.0:                                       # ④ 冻结检测：只统计连续 v>0 区间
            ref = goal.freeze_ref
            if ref is None:
                goal.freeze_ref = (now, x, y)
            elif math.hypot(x - ref[1], y - ref[2]) >= FROZEN_MOVE_EPS_M:
                goal.freeze_ref = (now, x, y)                 # 动了 -> 重新计时
            elif now - ref[0] >= FROZEN_WINDOW_S:
                if self._finish_goal(goal, False, error=ERR_FROZEN,
                                     why="下过 v=%.2f 但 %.0f s 内位姿没动（世界卡住？），"
                                         "还差 %.2f m" % (cmd_v, FROZEN_WINDOW_S, d)):
                    self._drive(0.0)
                return
        else:
            goal.freeze_ref = None                            # 原地转不产生位移：清窗，别把转身当冻结

        self._drive(cmd_v, cmd_dir)                     # ⑤ 发帧（drive 立即发出，tick 本轮不会重发）

    def _finish_goal(self, goal, ok, data=None, error=None, why=None) -> bool:
        with self._lock:
            if goal.result is not None:
                return False                                  # 只写一次
            if ok:
                goal.result = {"ok": True, "data": dict(data or {})}
            else:
                goal.result = {"ok": False, "error": error or ERR_SHUTDOWN, "why": why or ""}
            if self._goal is goal:
                self._goal = None
            goal.ev.set()
            return True

    def _wait_goal(self, goal):
        guard = goal.deadline + WAIT_GUARD_S
        while not goal.ev.wait(WAIT_POLL_S):
            if self._stop.is_set():
                self._finish_goal(goal, False, error=ERR_SHUTDOWN, why="控制台正在退出")
                break
            if time.monotonic() >= guard:
                self._finish_goal(goal, False, error=ERR_TIMEOUT,
                                  why="等待兜底超时（控制环可能已停）")
                break
        with self._lock:
            return dict(goal.result or {"ok": False, "error": ERR_SHUTDOWN,
                                        "why": "goal 没有结果（内部异常）"})

    # -------------------------------------------------- 指令实现
    def _do_goto(self, rid, a):
        now = time.monotonic()
        with self._lock:
            self._gid += 1
            goal = Goal(gid=self._gid, x=a["x"], y=a["y"], speed=a["speed"],
                        tol=a["tol"], timeout=a["timeout"], t0=now)
            old, self._goal = self._goal, goal
            if old is not None:
                self._finish_goal(old, False, error=ERR_PREEMPTED,
                                  why="被 goto#%d 抢占（同一时刻只有一个 goal）" % goal.gid)
        self._log_ctl("goto#%d -> (%.2f, %.2f) speed=%.2f tol=%.2f timeout=%.0fs"
                      % (goal.gid, goal.x, goal.y, goal.speed, goal.tol, goal.timeout))
        res = self._wait_goal(goal)
        rep = {"id": rid, "ok": bool(res.get("ok"))}
        if rep["ok"]:
            rep["data"] = dict(res.get("data") or {})
        else:
            rep["error"] = res.get("error") or ERR_SHUTDOWN
            rep["why"] = res.get("why") or ""
        self._log_ctl("goto#%d %s" % (goal.gid, "到位" if rep["ok"] else "失败：" + rep["error"]))
        return rep

    def _do_face(self, rid, a):
        x, y, _yaw = self._pose_xyz()                 # pose 还没到就用 spawn，不算 not_ready
        dir_w = Wrap_Pi(math.atan2(a["y"] - y, a["x"] - x))
        with self._lock:                              # face 会清掉正在跑的 goal
            old, self._goal = self._goal, None
            if old is not None:
                self._finish_goal(old, False, error=ERR_PREEMPTED, why="被 face 抢占")
        self._drive(0.0, dir_w)                 # v=0 + 绝对朝向；世界按 max_turn_rate 转过去
        self._log_ctl("face -> dir=%.1f°（v=0 原地转）" % math.degrees(dir_w))
        return {"id": rid, "ok": True, "data": {"dir": round(dir_w, 6)}}

    def _do_walk(self, rid, a):
        with self._lock:
            old, self._goal = self._goal, None
            if old is not None:
                self._finish_goal(old, False, error=ERR_PREEMPTED, why="被 walk 抢占")
        self._drive(a["v"], a["dir"])
        self._log_ctl("walk v=%.2f dir=%.1f°" % (a["v"], math.degrees(a["dir"])))
        return {"id": rid, "ok": True, "data": {}}

    def _do_stop(self, rid, _a):
        with self._lock:
            old, self._goal = self._goal, None
            if old is not None:
                self._finish_goal(old, False, error=ERR_PREEMPTED, why="被 stop 清除")
        self._drive(0.0)                        # 不用 client.stop()：那个是幂等一次性收尾
        self._log_ctl("stop v=0")
        return {"id": rid, "ok": True, "data": {}}

    def _do_pos(self, rid, _a):
        with self._io:                          # 与主循环的 pump() 串行（transport 一律持 _io）
            pose = self.client.pose()
            age = self.client.pose_age()
        if pose is None:
            return self._err(rid, ERR_NOT_READY, "还没收到 pose（世界 10 Hz 下发，稍后再试）")
        data = {
            "x": round(float(pose.get("x", 0.0)), 3),
            "y": round(float(pose.get("y", 0.0)), 3),
            "yaw": round(float(pose.get("yaw", 0.0)), 4),
            "v": round(math.hypot(float(pose.get("vx", 0.0)),
                                  float(pose.get("vy", 0.0))), 3),
            "pose_age": None if age is None else round(age, 3)}
        self._log_verbose("pos %s" % json.dumps(data, ensure_ascii=False))
        return {"id": rid, "ok": True, "data": data}

    def _do_status(self, rid, _a):
        with self._io:
            age = self.client.pose_age()
        with self._lock:
            g = self._goal
            goal = None if g is None else {
                "x": round(g.x, 2), "y": round(g.y, 2), "speed": g.speed,
                "tol": g.tol, "remaining_s": round(max(g.deadline - time.monotonic(), 0.0), 1)}
        data = {
            "peer_id": self.client.peer_id, "connected": bool(self.client.ready),
            "goal": goal, "pose_age": None if age is None else round(age, 3),
            "send_hz": float(self.client.send_hz)}
        self._log_verbose("status %s" % json.dumps(data, ensure_ascii=False))
        return {"id": rid, "ok": True, "data": data}

    def _do_quit(self, rid, _a):
        # 只回 reply；后置动作 "quit" 由 _handle_line 追加，保证「回复先 flush 再退出」
        self._log_ctl("quit：drive(0) -> 关连接 -> 退出")
        return {"id": rid, "ok": True, "data": {}}

    # -------------------------------------------------- 分派（错误码优先级在此定死）
    def _handle_line(self, raw: bytes):
        """一行请求 -> (reply, post)。post ∈ {None, "quit"}；所有 _do_* 只返回 reply。"""
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError as exc:
            return self._err(None, ERR_BAD_JSON, "不是合法 UTF-8：%s" % exc), None
        try:
            req = json.loads(text)
        except ValueError as exc:
            return self._err(None, ERR_BAD_JSON, "JSON 解析失败：%s" % exc), None
        if not isinstance(req, dict):
            return self._err(None, ERR_BAD_JSON, "请求必须是 JSON 对象"), None

        rid = req.get("id")                      # 原样回显（缺省 null）；不做类型强制
        cmd = req.get("cmd")
        if not isinstance(cmd, str) or cmd not in CMDS:            # ② unknown_cmd
            return self._err(rid, ERR_UNKNOWN_CMD,
                             "未知指令 %r；可用：%s" % (cmd, " / ".join(CMDS))), None
        if cmd == "quit":                                          # 例外：跳过 3/4
            return self._do_quit(rid, {}), "quit"                  # post="quit"
        try:
            args = self._parse(cmd, req)                           # ③ bad_arg（纯语法）
        except BadArg as exc:
            return self._err(rid, ERR_BAD_ARG, str(exc)), None
        if not self.client.ready:                                  # ④ not_ready
            return self._err(rid, ERR_NOT_READY,
                             "世界未连接（%s）：先起世界再重起本进程" % self.client.url), None
        try:
            reply = getattr(self, "_do_" + cmd)(rid, args)          # ⑤ 执行（goto 阻塞在本线程）
        except ProtocolError as exc:                                # 世界已断：回 shutdown，主循环随即退出
            self.request_shutdown("指令执行时连接已断：%s" % exc)
            return self._err(rid, ERR_SHUTDOWN, str(exc)), None
        return reply, None

    # -------------------------------------------------- 连接服务（一问一答）
    def _serve_conn(self, conn, addr):
        post = None
        rfile = None
        try:
            rfile = conn.makefile("rb")
            while not self._stop.is_set():
                raw = rfile.readline(MAX_LINE_BYTES)
                if not raw:                         # 对端半关 / EOF：必须主动关，否则 socat 挂住
                    break
                if not raw.endswith(b"\n"):
                    self._write_reply(conn, self._err(None, ERR_BAD_JSON,
                                        "一行超过 %d 字节或没有换行" % MAX_LINE_BYTES))
                    break
                reply, post = self._handle_line(raw)
                self._write_reply(conn, reply)
                if post == "quit":
                    break
        except OSError as exc:
            if not self._stop.is_set():
                print("[warn] 控制口连接异常（%s）：%s" % (addr, exc))
        finally:
            for step in (lambda: rfile and rfile.close(),
                         lambda: conn.shutdown(socket.SHUT_RDWR),
                         conn.close):
                try:
                    step()
                except (OSError, ValueError):
                    pass
            with self._lock:
                self._conns.discard(conn)
            if post == "quit":                      # 回复已 flush + FIN 已发，再退
                self.request_shutdown("quit")

    def _write_reply(self, conn, obj) -> None:
        line = json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8") + b"\n"
        if self.verbose:
            print("[ctl] -> %s" % line.decode("utf-8").rstrip("\n"))
        conn.sendall(line)                  # 一问一答：写完即 flush（sendall 无缓冲层）

    # -------------------------------------------------- 收尾（幂等）
    def request_shutdown(self, why: str) -> None:
        with self._lock:
            if self._stop.is_set():
                return                                  # 幂等：quit 与 finally 会各来一次
            self._stop.set()
            old, self._goal = self._goal, None
            if old is not None:
                self._finish_goal(old, False, error=ERR_SHUTDOWN, why="控制台退出：%s" % why)
        print("[ctl] shutdown：%s" % why)

    def close_all(self) -> None:
        if self.client.ready:                           # 没连上世界时没有 transport，发帧必抛
            try:
                self._drive(0.0)                        # ⑤ 先 drive(0)
            except Exception as exc:                    # 收尾不许抛
                print("[warn] 退出前 drive(0) 失败：%s" % exc)
        try:
            with self._io:
                self.client.close()                     # 关连接（世界日志 pedestrian removed）
        except Exception as exc:
            print("[warn] 关连接失败：%s" % exc)
        if self._server is not None:
            try:
                self._server.close()
            except OSError:
                pass
        for conn in list(self._conns):
            try:
                conn.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                conn.close()
            except OSError:
                pass
        if self._listener is not None:
            self._listener.join(timeout=1.0)


# ---------------------------------------------------------------- CLI
def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="行人常驻控制台：一个进程 <-> 一个行人；30 Hz 持续驱动 + 本机 JSON-lines 控制口")
    parser.add_argument("--url", default=DEFAULT_URL, help="世界 WS 地址")
    parser.add_argument("--peer-id", default=DEFAULT_PEER_ID,
                        help="WS 上的 peer_id（一个 id 一个进程；重连会摘掉旧 avatar）")
    parser.add_argument("--spawn", default=None,
                        help='出生点 "x,y[,z[,yaw_度]]"；缺省不带 spawn = 世界默认点 (64,56)')
    parser.add_argument("--host", default=DEFAULT_HOST,
                        help="控制口监听地址：只允许回环（%s）" % " / ".join(LOOPBACK_HOSTS))
    parser.add_argument("--port", type=int, default=DEFAULT_PORT, help="控制口端口（默认 9101）")
    parser.add_argument("--send-hz", type=float, default=DEFAULT_SEND_HZ,
                        help="motion_cmd 发送频率（硬限 20~50 Hz；世界 300 ms 超时）")
    parser.add_argument("--max-speed", type=float, default=WORLD_MAX_SPEED,
                        help="goto / walk 的速度上限（世界硬上限 %.2f，不许越过）" % WORLD_MAX_SPEED)
    parser.add_argument("--ready-timeout", type=float, default=READY_TIMEOUT_S,
                        help="等 chassis_ready 的窗口（秒）")
    parser.add_argument("--engine", default="auto", choices=("auto", "websockets", "stdlib"),
                        help="WS 传输层实现（auto = 优先 websockets）")
    parser.add_argument("--allow-spawn-fallback", action="store_true",
                        help="spawn 被世界回退时降级为 WARNING 继续（默认 exit 5）")
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="打印每条请求/应答 + 客户端逐帧收发")
    parser.add_argument("--quiet", action="store_true", help="不打每秒进度行与指令行（告警/错误仍打）")
    return parser


def main(argv=None) -> int:
    global _INTERRUPTED
    args = build_parser().parse_args(argv)

    if args.host not in LOOPBACK_HOSTS:
        print("[error] --host 只允许本机回环（%s），收到 %r：本工具不对外监听"
              % (" / ".join(LOOPBACK_HOSTS), args.host))
        return EXIT_CONNECT                   # 2 = 参数 / 控制口错误（见文件头「退出码」）

    if not (1 <= int(args.port) <= 65535):
        print("[error] --port 必须在 1~65535，收到 %r" % (args.port,))
        return EXIT_CONNECT
    if not (args.max_speed > 0.0):
        print("[error] --max-speed 必须 > 0，收到 %r（goto 的 v 不能为 0/负）" % (args.max_speed,))
        return EXIT_CONNECT
    if args.max_speed > WORLD_MAX_SPEED:
        print("[warn] --max-speed %.2f 超过世界硬上限，按 %.2f 生效"
              % (args.max_speed, WORLD_MAX_SPEED))

    spawn = None
    if args.spawn:
        try:
            spawn = parse_spawn_spec(args.spawn)
        except ValueError as exc:
            print("[error] %s" % exc)
            return EXIT_CONNECT

    signal.signal(signal.SIGINT, _On_Signal)
    signal.signal(signal.SIGTERM, _On_Signal)

    try:
        client = PedestrianClient(url=args.url, peer_id=args.peer_id, spawn=spawn,
                                  send_hz=args.send_hz, ready_timeout=args.ready_timeout,
                                  engine=args.engine, verbose=args.verbose,
                                  allow_spawn_fallback=args.allow_spawn_fallback)
    except ValueError as exc:
        print("[error] %s" % exc)
        return EXIT_CONNECT

    console = PedConsole(client, host=args.host, port=args.port,
                         max_speed=min(args.max_speed, WORLD_MAX_SPEED),
                         quiet=args.quiet, verbose=args.verbose)
    # ① 控制口先起来：世界没起也要能连、能回 not_ready
    try:
        console.start()
    except OSError as exc:
        print("[error] 无法监听 %s:%d：%s（换 --port，或先停掉占用该端口的进程）"
              % (args.host, args.port, exc))
        return EXIT_CONNECT

    # ② 再连世界；连不上不退出（控制口照旧监听），但要把「为什么」打清楚
    try:
        client.connect()
    except (ConnectError, ReadyTimeout) as exc:
        print("[warn] 未连上世界：%s" % exc)
        print("[warn] 控制口仍在 %s:%d 监听，所有指令返回 not_ready；"
              "世界起好后请重起本进程（本工具不做断线重连，见文件头「边界」）"
              % (args.host, args.port))
    except ClientError as exc:                     # ProtocolError / SpawnFallback：配置或代码问题，直接退
        print("[error] %s" % exc)
        console.request_shutdown("连接世界失败")
        console.close_all()
        return EXIT_SPAWN if isinstance(exc, SpawnFallback) else EXIT_PROTOCOL

    return console.run()


if __name__ == "__main__":
    raise SystemExit(main())
