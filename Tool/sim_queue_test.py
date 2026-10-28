#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-27
# Modified Date ： 2026-10-27
"""Task 36 Step 8 — sim 端到端验证：队列语义（replace / append / preempt）+ 双栈（§12.8）。

真跑：`godot --headless --path orion-simulation-world`（端口 9999）
   + `RobotSim_TestEnv/simq_<stack>/pleiades-sim --headless`（控制口 9098 / 9099），
只通过控制口注入（`queue` / `append` / `preempt` / `stop` / `mode`），
每 200 ms 采一次 `status` 到 `<out-dir>/trace.csv`（§12.8.4）。

用例（§12.8.3 的 10 条；本轮每栈跑 1~9，用例 10 = 双栈）：
    1 逐条执行        replace [A(近), B(远)] → A 到 → 出队 → B 开始 → B 到 → 队列空
    2 append 不打断   A 行驶中 append B：不换头、task_ticks 逐帧递增、位置连续前进
    3 append 空队列   空队列 append → 立即开始
    4 preempt 让位    A 行驶中 preempt P：立即换头、queue_len=2、A 让位
                      （§12.8.6 第 3 条「≤1 tick」→ 换头后位移 ≤ 1 tick + 惯性 + 位姿量化，
                       位姿 10 Hz 量不出 50 ms，见 impl.md D3）
    5 preempt 恢复    P 到 → task_id 回到 A，**task_ticks 从暂停值继续（不清零）**
    6 preempt 嵌套    A→B→C 三次换头，C→B→A 恢复，三者的 task_ticks 都不归零
    7 replace 丢弃    A 行驶中 replace [C]：C 从 0 起、A 不再回来
    8 取消            replace []：立即停车、queue_len=0、nav_status=idle
    9 不可达          超远目标 → 60 s Timeout → 失败上报 → 出队 → 后续 append 仍可执行
   10 双栈            本栈 1~9 全 PASS；两栈终表由 `--pair` 聚合（不起进程）

判定口径（§12.8.4）：抢占是否"从暂停点继续"**只认 `task_ticks`**（暂停期间不 tick → 冻结）。
UAV 用例只判"到达 + 不超时"，不卡厘米级。

日志域判据（tracing 行首是 µs 时间戳，两条打点同进程 → Δ 即真实延迟，不受位姿 10 Hz 限制）：
    用例 2 ③b   append 之后设备**零动作**（无 `[Sim] reset` / `[Sim] suspend`）：A 不停、不重来
    用例 4 ④b   `[control] preempt` → `[Sim] suspend` Δ ≤ 50 ms
                （§12.8.6 第 3 条「停车延迟 ≤ 1 tick」的**直接**度量；位移上界 ④ 降为物理佐证）
    用例 7 ③b   `[control] queue（整体替换）1 条` → `[Sim] reset` Δ ≤ 50 ms（立即生效）
    用例 8 ⑤b   `[control] queue（整体替换）0 条` → `[Sim] reset` Δ ≤ 50 ms（立即生效）
    用例 9 ③b   失败任务末次 `task_ticks` ≥ 1100（≈55 s：Timeout 在树内真走完，不是快速失败）
后三条（④b / 用例 9 ③b）同时是**硬检查**第 6、7 条；`--pair` 要求每栈 hard ≥8 条且全过。

用法：
    python3 Tool/sim_queue_test.py --stack ugv             # 起世界 + sim，跑 1~9
    python3 Tool/sim_queue_test.py --stack uav             # 同上（quadrotor_uav）
    python3 Tool/sim_queue_test.py --stack ugv --no-launch # 连已在跑的控制口
    python3 Tool/sim_queue_test.py --stack ugv --smoke     # 只跑用例 1 的单任务版（R1 前置冒烟）
    python3 Tool/sim_queue_test.py --pair                  # 用例 10：读两栈 cases.json 出 10 行终表
                                                           # 校验：fatal 空 / 1~9 全 PASS / 采样达标 /
                                                           # hard ≥8 全过 / 两栈同一二进制与同一脚本
    其他开关：--out-dir DIR   --json / --no-json（默认写）   --port PORT
退出码：0 = 9 条全 PASS；1 = 有 FAIL / 硬检查不过；2 = 环境/前置失败（产物缺失、装配失败等）

产物（`RobotSim_TestEnv/` 已被 .gitignore 忽略）：
    <out-dir>/trace.csv        `t,case,queue_len,task_id,task_ticks,nav_status,x,y,z,yaw`
    <out-dir>/cases.json       `--json` 时写出（机器可读的同源数字）
    <out-dir>/conclusion.md    总表 + 逐条判据 + 硬检查 + 暂停/恢复 ticks 证据 + 残留进程检查
    <out-dir>/sim_stdout.log   sim 的 stdout/stderr（tracing 写 `Log/` 文件，这里只有零散 eprintln）
    <out-dir>/sim_<CASE>.log   该用例期间的 sim 日志切片
    <out-dir>/world.log        世界实例日志
    <out-dir>/config.toml      本轮使用的配置副本
"""

import argparse
import base64
import csv
from datetime import datetime
import hashlib
import json
import math
import os
import pathlib
import re
import signal
import socket
import struct
import subprocess
import sys
import time
import traceback

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORLD_DIR = ROOT / "orion-simulation-world"
TARGET_BIN = ROOT / "target" / "debug" / "pleiades-sim"
TESTENV = ROOT / "RobotSim_TestEnv"
RESULTS = TESTENV / "results"

HOST = "127.0.0.1"
WORLD_PORT = 9999
WORLD_TIMEOUT_S = 2400          # 世界进程护栏（step.md 准备段「后台起世界 timeout 2400」）

# ─── 采样 / 判据常量（§12.8.4 与 plan.md §2.7 的值；**不得为通过而改**）───
# 时间预算（plan.md §2.7 的初值按 0.45 m/s 直线估；实测本世界 9°/s 的转向才是主成本
# —— 一次 2 m 的 Goto 实测 28.5 s，其中 17 s 是到达后的对齐转向 → 预算按实测放大，
# 判据（ticks 不清零 / 让位位移上界 / 队列账面）一个不放。偏差已记 tmp/task36/impl.md）
SAMPLE_DT = 0.20                # §12.8.4：每 200 ms 采一次 status
POLL_DT = 0.05                  # 等待条件的轮询周期（只轮询、不入 trace）
ARRIVE_TOL_M = 0.35             # 到达佐证（逐轴；UAV 不卡厘米，只作信息项）
STOP_TOL_M = {"ugv": 0.05, "uav": 0.08}      # 取消/抢占后 0.6→1.6 s 窗口内的位移上限
CASE_BUDGET_S = {1: 150, 2: 200, 3: 140, 4: 120, 5: 180, 6: 320, 7: 180, 8: 90, 9: 150}
TICKS_MIN_BEFORE_PAUSE = 20     # 抢占前该任务至少已 tick 1 s（这样"重建从 0"才可判别）
TICKS_RESUME_SLACK = 10         # 恢复值比暂停值最多多出的 tick 数（≤0.5 s：命令投递 + 1~2 拍）
                                # 上限意义：暂停期间树不 tick → 只允许"命令还没送达"的那几拍；
                                # 实测 +1~+3（见 tmp/task36/run_{ugv,uav}.log）。放宽到 +60(=3 s)
                                # 会让"冻结"退化成"没冻太久"，判据失效。

# ─── 用例 4 的 tick 级口径（§12.8.6 第 3 条：停车延迟 ≤ 1 个 tick）───────────
# 位姿只有 10 Hz（世界 POSE_BROADCAST_INTERVAL_S = 0.1），**无法直接量 50 ms**，
# 故把"≤1 tick"换成等价且可测的位移上界：
#   1 个 tick 的位移 + 惯性消解位移 + 1 个位姿周期的量化位移，再乘余量。
POSE_DT_S = 0.10
AUTO_TICK_S = 0.05
CHASSIS_DECEL_MPS2 = 3.0
CASE4_FINE_DT = 0.02
CASE4_FINE_WIN_S = 0.60
CASE4_BOUND_MARGIN = 1.5
CASE9_TIMEOUT_BUDGET_S = 100    # 60 s Timeout + 建树/采样余量
AUTO_PROBE_S = 0.4              # 命令 ack 后等 main_loop 消费队列的上限（poll 到条件成立为止）
SMOKE_BUDGET_S = 90             # R1：单任务冒烟（实测 2 m 一次约 28 s）

# ─── 日志域判据（§12.8.6 第 3 条「停车延迟 ≤ 1 个 tick」的直接度量；plan.md C3/C4/C5）───
# tracing 行首是**µs** 时间戳：`2026-10-27T15:12:47.724655Z  INFO ...`。
# 两条打点在同一进程，Δ 即「命令进队 → 设备动手」的真实延迟（实测 0.10~0.13 ms）。
LOG_TICK_TOL_MS = AUTO_TICK_S * 1000            # 50.0 ms = 1 个 tick
LOG_TS_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6})Z\s")
CASE9_MIN_FAIL_TICKS = 1100                     # 60 s / 50 ms ≈ 1200；留 8% 余量
PAIR_MIN_SAMPLES = {n: 3 for n in range(1, 10)}
PAIR_MIN_SAMPLES.update({4: 10, 5: 10, 6: 10})  # 判据核心用例，采样不得太少

PORT = {"ugv": 9098, "uav": 9099}
CHASSIS = {"ugv": "diff_drive_ugv", "uav": "quadrotor_uav"}
NODE_TYPE = {"ugv": "car", "uav": "uav"}
STATUS_NEW_KEYS = ["queue_len", "task_id", "task_ticks", "nav_status"]

CASE_TITLES = {
    1: "逐条执行", 2: "append 不打断", 3: "append 空队列", 4: "preempt 让位",
    5: "preempt 恢复", 6: "preempt 嵌套", 7: "replace 丢弃", 8: "取消",
    9: "不可达（60 s Timeout）", 10: "双栈",
}

CONFIG_TEMPLATE = """\
# Presented by KeJi
# Created Date ： 2026-10-27
# Modified Date ： 2026-10-27
#
# Pleiades 配置（Task 36 Step 8 队列语义 e2e；由 Tool/sim_queue_test.py 生成并幂等覆写，
# RobotSim_TestEnv/ 已被 .gitignore 忽略）
# 与世界默认参数保持一致：**不加** [sim_world.chassis_params]（max_speed=1.5 / max_turn_rate=90）

[Log]
level = "info"
log_file_path = "Log/"

[Network]
LAN = true
WAN = false
Transport_Protocol = "TCP"
listen_port = 0
dht_namespace = "pleiades-nodes"
bootstrap_peers = []

[Storage]
workspace_dir = "Pleiades_Workspace"
quota_gb = 0
kvcache_dir = ".kvcache"

[Identity]
peer_name = "simq_{stack}"
node_type = "{node_type}"

[sim_world]
enabled = true
url = "ws://127.0.0.1:{world_port}"
chassis = "{chassis}"

[chassis]
enabled = true
forward_speed = 30
turn_speed = 10

[control]
enabled = true
bind = "127.0.0.1"
port = {port}
"""

# 车栈声明一个 lidar（急停 + 建图）；机栈**不声明任何 sensor**（机无雷达，§12.5.2）
SENSOR_BLOCK = """
[[sim_world.sensor]]
slot = "lidar_front"
sensor = "lidar_2d"

[sim_world.sensor.mount]
x = 0.1
y = 0.0
z = 0.2
yaw = 0.0

[sim_world.sensor.params]
range = 12.0
rays = 360
rate_hz = 10
"""


# ================================================================ 极简 WebSocket 客户端
# 与 `RobotSim_TestEnv/tools/ws.py` 同源（该文件在 gitignored 目录、不能 import，故内联一份）。
# 只做控制口的一问一答：JSON 文本帧、客户端掩码、126/127 扩展长度、ping→pong、close→EOFError。

class WsClosed(EOFError):
    """对端关闭（用来区分「连接断了」与其它异常）。"""


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
    """世界 / sim 共用的进程包装：独立进程组 + 必 wait（不留僵尸）。"""

    def __init__(self, name):
        self.name = name
        self.p = None
        self.pid = None
        self._f = None

    def start(self, argv, cwd, log_path):
        log_path.parent.mkdir(parents=True, exist_ok=True)
        self._f = open(log_path, "ab", buffering=0)
        self.p = subprocess.Popen(
            argv, cwd=str(cwd), stdout=self._f, stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL, start_new_session=True,
        )
        self.pid = self.p.pid
        print(f"  [{self.name}] start pid={self.pid} argv={' '.join(argv)} (cwd={cwd})")

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
        # 兜底：组内可能还有子进程（`timeout` 包的 godot 就是两个进程）
        self._killpg(signal.SIGKILL)
        try:
            self.p.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            pass
        if self._f is not None:
            self._f.close()
            self._f = None
        print(f"  [{self.name}] stopped (pid={self.pid})")
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


def read_log_lines(path):
    if path is None or not path.exists():
        return []
    with path.open("r", encoding="utf-8", errors="replace") as f:
        return f.readlines()


class LogTail:
    """增量读 sim 的 tracing 日志文件（按**字节**偏移切片，供逐用例取证）。

    注意（与 plan.md §2.7 的偏差，见 tmp/task36/impl.md）：tracing 只写文件
    （`bootstrap.rs` 的 `log_dir.join("pleiades.log.<ts>")`），**不写 stdout**，
    所以切片对象是 `simq_<stack>/Log/pleiades.log.*` 的最新一份，而不是 `sim_stdout.log`。
    """

    def __init__(self, log_dir):
        self.log_dir = log_dir
        self.path = None
        self.pos = 0

    def newest(self):
        if not self.log_dir.is_dir():
            return None
        files = sorted(self.log_dir.glob("pleiades.log.*"), key=lambda p: p.stat().st_mtime)
        return files[-1] if files else None

    def reset(self):
        self.path = self.newest()
        self.pos = 0
        return self.path

    def mark(self):
        return self.pos

    def since(self, mark):
        """读 `mark` 之后新增的行（并推进游标）。"""
        if self.path is None or not self.path.exists():
            return []
        with self.path.open("rb") as f:
            f.seek(mark)
            data = f.read()
            end = f.tell()
        self.pos = end
        if not data:
            return []
        return data.decode("utf-8", "replace").splitlines(keepends=True)

    def all_lines(self):
        """整份日志（硬检查「无静默丢弃」用）。返回 (行, 当前字节长度)。"""
        if self.path is None or not self.path.exists():
            return [], 0
        with self.path.open("rb") as f:
            data = f.read()
        lines = data.decode("utf-8", "replace").splitlines(keepends=True)
        return lines, len(data)


# ================================================================ 控制口

class Control:
    """控制口单连接（一问一答）+ 断线重连。"""

    def __init__(self, port, timeout=5.0):
        self.port = port
        self.timeout = timeout
        self.ws = None

    def connect(self, retries=40, delay=0.25):
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
            raise RuntimeError(f"status 回包失败: {r}")
        return r.get("data", {})

    def send_cmd(self, obj):
        """发一条命令，返回回包（`ok` / `err`）。"""
        return self.request(obj)

    def close(self):
        if self.ws is not None:
            self.ws.close()
            self.ws = None


# ================================================================ 几何

def fwd(st, d):
    """沿当前航向 d 米（世界系）。"""
    return (round(st["x"] + d * math.cos(st["yaw"]), 2),
            round(st["y"] + d * math.sin(st["yaw"]), 2))


def lat(st, d):
    """垂直航向 d 米（左转为正）。"""
    return (round(st["x"] - d * math.sin(st["yaw"]), 2),
            round(st["y"] + d * math.cos(st["yaw"]), 2))


def clamp_world(p):
    """夹到世界 [0,128)（控制口对越界点会直接拒绝）。"""
    return (round(min(127.9, max(0.1, p[0])), 2), round(min(127.9, max(0.1, p[1])), 2))


def dist(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def crit(name, ok, detail=""):
    return {"name": name, "ok": bool(ok), "detail": str(detail)}


# ================================================================ 日志域 / 构建指纹
# 「命令 → 设备动作」的**直接**度量：日志两条打点在同一进程、µs 精度，不受位姿 10 Hz 限制。
# 用途：用例 4 的 §12.8.6 第 3 条（≤1 tick）、用例 7/8 的「立即生效」（plan.md C3/C4）。

def parse_log_ts(line):
    """tracing 行开头的 µs 时间戳 → epoch 秒；解析不出/不是时间戳行 → None。"""
    m = LOG_TS_RE.match(line)
    if not m:
        return None
    try:
        return datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S.%f").timestamp()
    except ValueError:
        return None


def log_action_delay_ms(lines, start_pat, end_pat):
    """日志切片：`start_pat` 的**最后一次**命中 → 其后**第一次** `end_pat` 的 Δ(ms)。

    两条打点在同一进程、µs 精度，故 Δ 就是「命令进队 → 设备动手」的真实延迟。
    返回 (Δms | None, 明细文本)；任一未命中 → (None, 说明)。
    """
    i0 = None
    for i, ln in enumerate(lines):
        if start_pat in ln:
            i0 = i
    if i0 is None:
        return None, "未命中起点 %r" % start_pat
    t0 = parse_log_ts(lines[i0])
    if t0 is None:
        return None, "起点行无 µs 时间戳"
    for ln in lines[i0 + 1:]:
        if end_pat in ln:
            t1 = parse_log_ts(ln)
            if t1 is None:
                return None, "终点行无 µs 时间戳"
            dt = (t1 - t0) * 1000.0
            return dt, "%r → %r：Δ=%.2f ms" % (start_pat, end_pat, dt)
    return None, "起点之后未见 %r" % end_pat


def sha256_file(path):
    """文件 sha256（1 MiB 分块；492 MB 的二进制约 1~2 s）。"""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for blk in iter(lambda: f.read(1 << 20), b""):
            h.update(blk)
    return h.hexdigest()


def build_fingerprint(bin_path):
    """构建指纹：`--pair` 用它卡「两栈来自同一构建、同一脚本」。"""
    fp = {"binary_sha256": None, "binary_bytes": None, "source_sha256": None,
          "script_sha256": sha256_file(pathlib.Path(__file__).resolve()),
          "git_head": None, "git_dirty": None}
    for p, key in ((bin_path, "binary_sha256"), (TARGET_BIN, "source_sha256")):
        p = pathlib.Path(p) if p else None
        if p and p.exists():
            fp[key] = sha256_file(p)
    if bin_path and pathlib.Path(bin_path).exists():
        fp["binary_bytes"] = pathlib.Path(bin_path).stat().st_size
    try:
        fp["git_head"] = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=ROOT,
            capture_output=True, text=True, timeout=10).stdout.strip() or None
        st = subprocess.run(
            ["git", "status", "--porcelain"], cwd=ROOT,
            capture_output=True, text=True, timeout=10).stdout
        fp["git_dirty"] = len([x for x in st.splitlines() if x.strip()])
    except (subprocess.SubprocessError, OSError):
        pass
    return fp


# ================================================================ 主夹具

class Harness:
    def __init__(self, stack, out, port, log_dir):
        self.stack = stack
        self.out = out
        self.port = port
        self.ctl = Control(port)
        self.rows = []            # trace 行
        self.cases = {}           # case -> {pass, criteria, samples}
        self.cur = 0
        self.t0 = time.time()
        self.log = LogTail(log_dir)
        self.evidence = []        # 暂停/恢复 ticks 对（conclusion 里单独成表）
        self.verdicts = {}        # 各 case 的关键数字（供用例 5/6 承接）

    # ── 采样 ────────────────────────────────────────────────
    def _row(self, case, st):
        r = {
            "t": round(time.time() - self.t0, 2),
            "case": case,
            "queue_len": st.get("queue_len"),
            "task_id": st.get("task_id"),
            "task_ticks": st.get("task_ticks"),
            "nav_status": st.get("nav_status"),
            "x": round(float(st.get("x", 0.0)), 3),
            "y": round(float(st.get("y", 0.0)), 3),
            "z": round(float(st.get("z", 0.0)), 3),
            "yaw": round(float(st.get("yaw", 0.0)), 4),
        }
        self.rows.append(r)
        return r

    def snap(self, case=None):
        """发一次 status，记进 trace，返回 (row, status)。"""
        st = self.ctl.status()
        row = self._row(case if case is not None else self.cur, st)
        return row, st

    def seg(self, seconds, stop=None, case=None):
        """连续采样（200 ms 一条，进 trace）直到 seconds 或 stop(row) 为真。返回 rows。"""
        c = case if case is not None else self.cur
        t_start = time.time()
        rows = []
        while True:
            try:
                row, _ = self.snap(c)
            except (OSError, WsClosed, RuntimeError) as e:
                print(f"  [case {c}] status 失败（{type(e).__name__}），尝试重连")
                self.ctl.close()
                if not self.ctl.connect(retries=8, delay=0.1):
                    print(f"  [case {c}] 重连失败，采样中止")
                    break
                time.sleep(SAMPLE_DT)
                continue
            rows.append(row)
            if stop is not None and stop(row):
                break
            if time.time() - t_start >= seconds:
                break
            time.sleep(SAMPLE_DT)
        return rows

    def fine(self, seconds, dt=None, case=None):
        """细采样（默认 20 ms），行照常入 trace。

        只给「让位 / 停车」这种 tick 级判定用：`seg` 的 200 ms 会把 1 个 tick（50 ms）抹平。
        注意：位姿是 10 Hz，采样再细也不能分辨 50 ms —— 判据按位移上界写（见 case4）。
        """
        step = CASE4_FINE_DT if dt is None else dt
        c = case if case is not None else self.cur
        t_start = time.time()
        rows = []
        while time.time() - t_start < seconds:
            row, _ = self.snap(c)
            rows.append(row)
            time.sleep(step)
        return rows

    def speed_est(self, rows, tail=8):
        """最近 `tail` 对采样的速度中位数（m/s）；样本不足 → 0.0。"""
        sp = []
        for a, b in zip(rows, rows[1:]):
            dt = b["t"] - a["t"]
            d = dist((a["x"], a["y"]), (b["x"], b["y"]))
            if dt > 0 and d > 0.01:
                sp.append(d / dt)
        sp = sp[-tail:] if sp else []
        sp.sort()
        return sp[len(sp) // 2] if sp else 0.0

    def poll(self, pred, timeout, note=""):
        """50 ms 轮询 status 直到 pred(row) 为真；返回命中的 row（None = 超时）。不入 trace。"""
        t_start = time.time()
        last = None
        while time.time() - t_start < timeout:
            try:
                st = self.ctl.status()
            except (OSError, WsClosed, RuntimeError):
                self.ctl.close()
                if not self.ctl.connect(retries=6, delay=0.1):
                    return None
                continue
            row = self._row(self.cur, st)
            self.rows.pop()  # 轮询的采样不入 trace（§R5：只轮询、不记）
            last = row
            if pred(row):
                return row
            time.sleep(POLL_DT)
        if note:
            print(f"  [case {self.cur}] poll 超时（{timeout}s）：{note}；最后={last}")
        return None

    def await_cond(self, pred, timeout, note=""):
        """长等待（用例级）：**边采样边等**（200 ms 一条进 trace，提供轨迹证据）。

        与 [`Harness.poll`] 的分工：`poll` 只用于 ack 级（≤0.5 s）判定，不入 trace；
        分钟级的「到到达 / 到出队」必须走这里，否则 trace 只剩首尾两行、无法取证。
        """
        rows = self.seg(timeout, stop=pred)
        if rows and pred(rows[-1]):
            return rows[-1]
        if note:
            print(f"  [case {self.cur}] 等待超时（{timeout}s）：{note}；最后={rows[-1] if rows else None}")
        return None

    # ── 便捷 ────────────────────────────────────────────────
    def command(self, obj, name):
        r = self.ctl.send_cmd(obj)
        if not r.get("ok"):
            print(f"  [!] {name} 回包失败：{r}")
        return r

    def queue(self, pts):
        return self.command({"cmd": "queue", "missions": [{"x": p[0], "y": p[1]} for p in pts]}, "queue")

    def append(self, pts):
        return self.command({"cmd": "append", "missions": [{"x": p[0], "y": p[1]} for p in pts]}, "append")

    def preempt(self, pts):
        return self.command({"cmd": "preempt", "missions": [{"x": p[0], "y": p[1]} for p in pts]}, "preempt")

    def stop(self):
        return self.command({"cmd": "stop"}, "stop")

    def ensure_idle(self, timeout=20.0):
        """用例前置：清空队列 + 停车 + 等 nav 回到 idle（不参与判据，只保证起点干净）。"""
        self.stop()
        return self.poll(lambda r: r["queue_len"] == 0 and r["task_id"] is None, timeout,
                         "ensure_idle") is not None

    def moving(self, rows, min_speed=0.2, span=1):
        """最近 `span` 帧的平均速度 ≥ min_speed → 已经在走（span=1 即相邻两帧，200 ms）。"""
        if len(rows) < span + 1:
            return False
        a, b = rows[-1 - span], rows[-1]
        dt = b["t"] - a["t"]
        if dt <= 0:
            return False
        return dist((a["x"], a["y"]), (b["x"], b["y"])) / dt >= min_speed

    def wait_move(self, timeout, min_ticks=0, span=1, min_speed=0.2):
        """等到「已经在走」且（可选）队首 ticks ≥ min_ticks。返回命中的 row。

        `span`/`min_speed` 用来区分「真在直行」与「原地转向」：本世界 UGV 的 nav 是
        「转到对准 → 直行」交替（9°/s），逐帧判速度会被转向里的短促位移骗过。
        用例 2 要量「append 后 3 s 的位移」，前提必须是**行驶中**（§12.8.3），故用
        span=10（2 s）平均 ≥0.35 m/s 卡「已经在直行」；其余用例保持默认（span=1）。
        """
        t_start = time.time()
        buf = []
        while time.time() - t_start < timeout:
            row, _ = self.snap()
            buf.append(row)
            if self.moving(buf, min_speed, span) and (row["task_ticks"] or 0) >= min_ticks:
                return row
            time.sleep(SAMPLE_DT)
        return None

    def slice(self, mark, case):
        lines = self.log.since(mark)
        (self.out / f"sim_{case}.log").write_text("".join(lines), encoding="utf-8")
        return lines

    def finish_case(self, n, criteria):
        ok = all(c["ok"] for c in criteria)
        samples = len([r for r in self.rows if r["case"] == n])
        self.cases[n] = {"case": n, "title": CASE_TITLES.get(n, ""), "pass": ok,
                         "criteria": criteria, "samples": samples}
        print(f"  用例 {n} {CASE_TITLES.get(n, '')}：{'PASS' if ok else 'FAIL'}（{samples} 采样）")
        for c in criteria:
            if not c["ok"]:
                print(f"    ✗ {c['name']}：{c['detail']}")
        return ok

    # ── 用例 ────────────────────────────────────────────────
    def case1(self):
        """逐条执行：A(近) → 自动出队 → B(远) → 队列空停车（D1：同航向递增，避免 180° 掉头）。"""
        self.cur = 1
        mark = self.log.mark()
        crits = []
        self.ensure_idle()
        row, st = self.snap()
        A = clamp_world(fwd(st, 2.0))
        B = clamp_world(fwd(st, 4.0))
        ack = self.queue([A, B])
        crits.append(crit("queue 回包 ok", ack.get("ok") is True, ack.get("err", "")))
        r1 = self.poll(lambda r: r["queue_len"] == 2 and r["task_id"] is not None, 0.5, "A 开始")
        a1 = r1["task_id"] if r1 else None
        crits.append(crit("① ≤0.5 s 内 task_id=a1、queue_len=2", r1 is not None,
                          f"a1={a1} queue_len={r1['queue_len'] if r1 else None}"))
        r2 = self.await_cond(lambda r: a1 is not None and r["task_id"] not in (None, a1) and r["queue_len"] == 1,
                       CASE_BUDGET_S[1], "A 到达后 B 开始")
        a2 = r2["task_id"] if r2 else None
        crits.append(crit(f"② ≤{CASE_BUDGET_S[1]} s 内 A 出队、B 开始（queue_len=1）", r2 is not None,
                          f"a2={a2} a1={a1}"))
        r3 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE_BUDGET_S[1], "队列跑空")
        crits.append(crit("③ 队列跑空（queue_len=0、task_id=None）", r3 is not None,
                          f"{r3}"))
        row, st = self.snap()
        err_b = dist((st["x"], st["y"]), B)
        crits.append(crit("④ 末位姿 ≈ B（逐轴 ≤%.2f m）" % ARRIVE_TOL_M,
                          abs(st["x"] - B[0]) <= ARRIVE_TOL_M and abs(st["y"] - B[1]) <= ARRIVE_TOL_M,
                          f"pos=({st['x']:.2f},{st['y']:.2f}) B={B} dist={err_b:.3f}"))
        lines = "".join(self.slice(mark, 1))
        n_done = len(re.findall(r"任务完成", lines))
        crits.append(crit("⑤ 日志切片「任务完成」≥2", n_done >= 2, f"count={n_done}"))
        return self.finish_case(1, crits)

    def case2(self):
        """append 不打断：不换头、ticks 逐帧递增、位置连续前进。"""
        self.cur = 2
        mark = self.log.mark()
        crits = []
        self.ensure_idle()
        row, st = self.snap()
        A = clamp_world(fwd(st, 6.0))
        self.queue([A])
        r1 = self.poll(lambda r: r["task_id"] is not None, 0.5, "A 开始")
        a1 = r1["task_id"] if r1 else None
        crits.append(crit("① A 开始（task_id 非空）", r1 is not None, f"a1={a1}"))
        w = self.wait_move(40.0, TICKS_MIN_BEFORE_PAUSE, span=10, min_speed=0.35)
        crits.append(crit(f"① 抢占前 A 已 tick ≥{TICKS_MIN_BEFORE_PAUSE} 拍且已在直行"
                          f"（2 s 平均 ≥0.35 m/s；转向段不算）",
                          w is not None, f"row={w}"))
        if w is None:
            return self.finish_case(2, crits + [crit("后续判据", False, "前置未成立")])
        yaw0 = w["yaw"]
        row0, st0 = self.snap()
        origin = (st0["x"], st0["y"])
        B = clamp_world(fwd(st0, 8.0))
        ack = self.append([B])
        crits.append(crit("② append 回包 ok", ack.get("ok") is True, ack.get("err", "")))
        win = self.seg(3.0)
        heads = {r["task_id"] for r in win}
        crits.append(crit("③ 3 s 窗口内 task_id 恒为 a1（不换头）",
                          heads == {a1}, f"窗口内 task_id 集合={sorted(heads, key=lambda v: (v is None, v))}"))
        ticks = [r["task_ticks"] for r in win if r["task_ticks"] is not None]
        mono = all(b > a for a, b in zip(ticks, ticks[1:])) if len(ticks) >= 2 else False
        crits.append(crit("④ task_ticks 逐帧严格递增且末-首 ≥40",
                          mono and ticks and (ticks[-1] - ticks[0]) >= 40,
                          f"ticks 首/末={ticks[0] if ticks else None}/{ticks[-1] if ticks else None} 递增={mono}"))
        proj = [((r["x"] - origin[0]) * math.cos(yaw0) + (r["y"] - origin[1]) * math.sin(yaw0)) for r in win]
        back = 0
        for a, b in zip(proj, proj[1:]):
            if a - b > 0.10:
                back += 1
        crits.append(crit("⑤ 沿航向位移 ≥0.35 m 且无 >0.10 m 回退",
                          proj and (proj[-1] - proj[0]) >= 0.35 and back == 0,
                          f"位移={proj[-1] - proj[0] if proj else 0:.3f} 回退次数={back}"))
        # ③b（plan.md C2）：append 之后设备**零动作** —— A 不停、不重来（设备层证据；
        # 只读一次日志：`since()` 会推进游标，见计划 R2）
        lines = self.slice(mark, 2)
        i_app = max((i for i, ln in enumerate(lines) if "[control] append" in ln), default=-1)
        dev_after = [ln for ln in (lines[i_app + 1:] if i_app >= 0 else [])
                     if "[Sim] reset" in ln or "[Sim] suspend" in ln]
        crits.append(crit("③b append 之后设备零动作（无 [Sim] reset / [Sim] suspend）",
                          i_app >= 0 and not dev_after,
                          f"append 行#{i_app}；之后设备动作 {len(dev_after)} 条"))
        r4 = self.await_cond(lambda r: a1 is not None and r["task_id"] not in (None, a1) and r["queue_len"] == 1,
                       CASE_BUDGET_S[2], "A 完成 → B 开始")
        a2 = r4["task_id"] if r4 else None
        crits.append(crit("⑥ A 完成后 task_id=a2、queue_len=1", r4 is not None,
                          f"a2={a2} queue_len={r4['queue_len'] if r4 else None}"))
        r5 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE_BUDGET_S[2], "队列跑空")
        row, st = self.snap()
        crits.append(crit("⑦ 队列跑空且末位姿 ≈ B",
                          r5 is not None and abs(st["x"] - B[0]) <= ARRIVE_TOL_M
                          and abs(st["y"] - B[1]) <= ARRIVE_TOL_M,
                          f"pos=({st['x']:.2f},{st['y']:.2f}) B={B}"))
        return self.finish_case(2, crits)

    def case3(self):
        """append 空队列 → 立即开始。"""
        self.cur = 3
        mark = self.log.mark()
        crits = []
        r0 = self.poll(lambda r: r["queue_len"] == 0 and r["task_id"] is None, 2.0, "前置：队列空")
        crits.append(crit("① 前置：队列空（承接用例 2 收尾）", r0 is not None, f"{r0}"))
        row, st = self.snap()
        C = clamp_world(fwd(st, 10.0))
        ack = self.append([C])
        crits.append(crit("② append 回包 ok", ack.get("ok") is True, ack.get("err", "")))
        r1 = self.poll(lambda r: r["task_id"] is not None and r["queue_len"] == 1, 0.5, "C 立即开始")
        crits.append(crit("③ ≤0.5 s 内 task_id 非空、queue_len=1", r1 is not None, f"{r1}"))
        r2 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE_BUDGET_S[3], "C 完成")
        row, st = self.snap()
        crits.append(crit("④ C 完成且末位姿 ≈ C",
                          r2 is not None and abs(st["x"] - C[0]) <= ARRIVE_TOL_M
                          and abs(st["y"] - C[1]) <= ARRIVE_TOL_M,
                          f"pos=({st['x']:.2f},{st['y']:.2f}) C={C}"))
        self.slice(mark, 3)
        return self.finish_case(3, crits)

    def case4(self):
        """preempt 让位：立即换头 + queue_len=2 + 换头后位移 ≤ 1 tick + 惯性 + 位姿量化。"""
        self.cur = 4
        mark = self.log.mark()
        crits = []
        self.ensure_idle()
        row, st = self.snap()
        A = clamp_world(fwd(st, 4.0))
        self.queue([A])
        r1 = self.poll(lambda r: r["task_id"] is not None, 0.5, "A 开始")
        a1 = r1["task_id"] if r1 else None
        w = self.wait_move(25.0, 30)
        crits.append(crit("① A 已 tick ≥30 拍且已在走", w is not None, f"{w}"))
        if w is None:
            return self.finish_case(4, crits + [crit("后续判据", False, "前置未成立")])
        row0, st0 = self.snap()
        self.verdicts["pause_ticks_a"] = row0["task_ticks"]
        self.verdicts["a1"] = a1
        P = clamp_world(lat(st0, 1.2))
        v_est = max(self.speed_est([r for r in self.rows if r["case"] == 4]), 0.2)
        self.verdicts["case4_v_est"] = round(v_est, 3)
        ack = self.preempt([P])
        crits.append(crit("② preempt 回包 ok", ack.get("ok") is True, ack.get("err", "")))
        r2 = self.poll(lambda r: a1 is not None and r["task_id"] not in (None, a1) and r["queue_len"] == 2,
                       0.5, "P 抢到队首")
        p_id = r2["task_id"] if r2 else None
        self.verdicts["p_id"] = p_id
        crits.append(crit("③ ≤0.5 s 内 task_id=P、queue_len=2（A 退到第二位）", r2 is not None,
                          f"p={p_id} queue_len={r2['queue_len'] if r2 else None}"))
        fine_rows = self.fine(CASE4_FINE_WIN_S)
        win = fine_rows + self.seg(1.5 - CASE4_FINE_WIN_S)
        # ④ 口径（§12.8.6 第 3 条）：从**换头生效那一帧**起量位移 —— 命令投递延迟不算在
        #   "停车延迟"里（投递延迟已由 ③ 的 ≤0.5 s 卡住）。换头生效帧 = 首个 task_id==p_id 的采样。
        base = next((r for r in fine_rows if r["task_id"] == p_id), fine_rows[0])
        last = fine_rows[-1]
        moved_eff = dist((base["x"], base["y"]), (last["x"], last["y"]))
        moved_ack = dist((st0["x"], st0["y"]), (last["x"], last["y"]))
        tick_d = v_est * AUTO_TICK_S
        inertia_d = v_est * v_est / (2 * CHASSIS_DECEL_MPS2)
        quant_d = v_est * POSE_DT_S
        bound = (tick_d + inertia_d + quant_d) * CASE4_BOUND_MARGIN
        # 停车延迟（位姿域，量化 100 ms，只作取证；判定用位移上界）
        changes = [r for r in fine_rows
                   if abs(r["x"] - last["x"]) > 1e-3 or abs(r["y"] - last["y"]) > 1e-3]
        stop_lat = (changes[-1]["t"] - base["t"]) if changes else 0.0
        self.verdicts["case4_stop_latency_ms"] = int(round(stop_lat * 1000))
        self.verdicts["case4_moved_eff"] = round(moved_eff, 3)
        self.verdicts["case4_bound"] = round(bound, 3)
        crits.append(crit(
            "④ 让位：换头后位移 %.3f m ≤ %.3f m（1 tick %.3f + 惯性 %.3f + 位姿量化 %.3f，"
            "×%.1f；v≈%.2f m/s；实测停车延迟 %d ms，位姿 10 Hz 量化）"
            % (moved_eff, bound, tick_d, inertia_d, quant_d, CASE4_BOUND_MARGIN, v_est,
               int(round(stop_lat * 1000))),
            moved_eff <= bound,
            "moved_eff=%.3f moved_from_ack=%.3f bound=%.3f lat=%.2fs v=%.2f"
            % (moved_eff, moved_ack, bound, stop_lat, v_est)))
        # ④b（plan.md C3）：§12.8.6 第 3 条「停车延迟 ≤ 1 个 tick」的**直接**度量 ——
        # 日志 µs 时间戳不受位姿 10 Hz 限制；④ 的位移上界降为物理佐证（判据不删、不放宽）。
        # 只读一次日志（计划 R2：`since()` 会推进游标）。
        lines = self.slice(mark, 4)
        text = "".join(lines)
        dt_ms, dt_detail = log_action_delay_ms(lines, "[control] preempt", "[Sim] suspend")
        self.verdicts["case4_cmd_to_suspend_ms"] = None if dt_ms is None else round(dt_ms, 2)
        crits.append(crit("④b `[control] preempt` → `[Sim] suspend` Δ ≤ %.0f ms（1 个 tick）"
                          % LOG_TICK_TOL_MS,
                          dt_ms is not None and dt_ms <= LOG_TICK_TOL_MS, dt_detail))
        crits.append(crit("⑤ 窗口内 task_id 从不回到 a1",
                          all(r["task_id"] != a1 for r in win),
                          f"a1={a1}"))
        earn = row0["task_ticks"] or 0
        self.evidence.append({"case": 4, "task": "A", "phase": "pause", "task_id": a1, "ticks": earn})
        crits.append(crit("⑥ 日志切片含「抢占队头」", "抢占队头" in text, ""))
        return self.finish_case(4, crits)

    def case5(self):
        """preempt 恢复：P 到 → A 回来，暂停值 ≤ ticks ≤ 暂停值+slack（不清零 = 同一实例）。"""
        self.cur = 5
        mark = self.log.mark()
        crits = []
        a1 = self.verdicts.get("a1")
        p_id = self.verdicts.get("p_id")
        pause_ticks = self.verdicts.get("pause_ticks_a") or 0
        crits.append(crit(f"① 暂停时 A 的 task_ticks ≥30（实测 {pause_ticks}）", pause_ticks >= 30, ""))
        r1 = self.await_cond(lambda r: p_id is not None and r["task_id"] != p_id,
                             CASE_BUDGET_S[4], "P 完成")
        crits.append(crit("② P 完成（task_id 不再为 P）", r1 is not None, f"{r1}"))
        r2 = self.await_cond(lambda r: a1 is not None and r["task_id"] == a1, 5.0, "A 恢复")
        resume_ticks = r2["task_ticks"] if r2 else None
        crits.append(crit("③ ≤5 s 内 task_id 回到 a1、queue_len=1",
                          r2 is not None and r2["queue_len"] == 1,
                          f"a1={a1} row={r2}"))
        crits.append(crit("④ %d ≤ 恢复值 ≤ %d + %d（暂停期间不 tick；重建则从 0 起）"
                          % (pause_ticks, pause_ticks, TICKS_RESUME_SLACK),
                          resume_ticks is not None
                          and pause_ticks <= resume_ticks <= pause_ticks + TICKS_RESUME_SLACK,
                          f"pause={pause_ticks} resume={resume_ticks} slack={TICKS_RESUME_SLACK}"))
        self.evidence.append({"case": 5, "task": "A", "phase": "resume", "task_id": a1,
                              "ticks": resume_ticks, "pause": pause_ticks})
        r3 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE_BUDGET_S[5], "A 完成")
        crits.append(crit("⑤ A 完成、队列空", r3 is not None, f"{r3}"))
        self.slice(mark, 5)
        return self.finish_case(5, crits)

    def case6(self):
        """preempt 嵌套：a→b→c 换头、c→b→a 恢复，三者 ticks 都不归零。"""
        self.cur = 6
        mark = self.log.mark()
        crits = []
        self.ensure_idle()
        row, st = self.snap()
        A = clamp_world(fwd(st, 3.5))
        self.queue([A])
        r1 = self.poll(lambda r: r["task_id"] is not None, 0.5, "A 开始")
        a_id = r1["task_id"] if r1 else None
        w = self.wait_move(25.0, TICKS_MIN_BEFORE_PAUSE)
        crits.append(crit("① A 已 tick ≥%d 拍" % TICKS_MIN_BEFORE_PAUSE, w is not None, f"{w}"))
        if w is None:
            return self.finish_case(6, crits + [crit("后续判据", False, "前置未成立")])
        a_pause = w["task_ticks"]
        self.evidence.append({"case": 6, "task": "A", "phase": "pause", "task_id": a_id, "ticks": a_pause})
        row, st = self.snap()
        B = clamp_world(lat(st, 1.5))
        self.preempt([B])
        r2 = self.poll(lambda r: r["task_id"] not in (None, a_id) and r["queue_len"] == 2, 0.5, "B 抢到队首")
        b_id = r2["task_id"] if r2 else None
        crits.append(crit("② 换头 a→b，queue_len=2", r2 is not None,
                          f"a={a_id} b={b_id} queue_len={r2['queue_len'] if r2 else None}"))
        r3 = self.await_cond(lambda r: b_id is not None and r["task_id"] == b_id and (r["task_ticks"] or 0) >= TICKS_MIN_BEFORE_PAUSE,
                       30.0, "B 已 tick ≥20")
        crits.append(crit("③ B 已 tick ≥%d 拍" % TICKS_MIN_BEFORE_PAUSE, r3 is not None, f"{r3}"))
        b_pause = r3["task_ticks"] if r3 else None
        self.evidence.append({"case": 6, "task": "B", "phase": "pause", "task_id": b_id, "ticks": b_pause})
        row, st = self.snap()
        C = clamp_world(lat(st, 0.8))
        self.preempt([C])
        r4 = self.poll(lambda r: r["task_id"] not in (None, b_id) and r["queue_len"] == 3, 0.5, "C 抢到队首")
        c_id = r4["task_id"] if r4 else None
        crits.append(crit("④ 换头 b→c，queue_len=3", r4 is not None,
                          f"c={c_id} queue_len={r4['queue_len'] if r4 else None}"))
        # C 完成 → B 恢复
        r5 = self.await_cond(lambda r: r["task_id"] == b_id, CASE_BUDGET_S[6], "C 完成 → B 恢复")
        b_resume = r5["task_ticks"] if r5 else None
        crits.append(crit("⑤ C 完成 → task_id=b，%s ≤ b_ticks ≤ %s + %d（不清零）"
                          % (b_pause, b_pause, TICKS_RESUME_SLACK),
                          r5 is not None and b_pause is not None
                          and b_pause <= b_resume <= b_pause + TICKS_RESUME_SLACK,
                          f"b_pause={b_pause} b_resume={b_resume}"))
        self.evidence.append({"case": 6, "task": "B", "phase": "resume", "task_id": b_id,
                              "ticks": b_resume, "pause": b_pause})
        # B 完成 → A 恢复
        r6 = self.await_cond(lambda r: r["task_id"] == a_id and r["queue_len"] == 1,
                       CASE_BUDGET_S[6], "B 完成 → A 恢复")
        a_resume = r6["task_ticks"] if r6 else None
        crits.append(crit("⑥ B 完成 → task_id=a、queue_len=1，%s ≤ a_ticks ≤ %s + %d（不清零）"
                          % (a_pause, a_pause, TICKS_RESUME_SLACK),
                          r6 is not None and a_pause <= a_resume <= a_pause + TICKS_RESUME_SLACK,
                          f"a_pause={a_pause} a_resume={a_resume} row={r6}"))
        self.evidence.append({"case": 6, "task": "A", "phase": "resume", "task_id": a_id,
                              "ticks": a_resume, "pause": a_pause})
        r7 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE_BUDGET_S[6], "A 完成")
        crits.append(crit("⑦ A 完成、队列空", r7 is not None, f"{r7}"))
        self.slice(mark, 6)
        return self.finish_case(6, crits)

    def case7(self):
        """replace 丢弃：C 从 0 起、A 不再回来。"""
        self.cur = 7
        mark = self.log.mark()
        crits = []
        self.ensure_idle()
        row, st = self.snap()
        A = clamp_world(fwd(st, 5.0))
        self.queue([A])
        r1 = self.poll(lambda r: r["task_id"] is not None, 0.5, "A 开始")
        a_id = r1["task_id"] if r1 else None
        w = self.wait_move(25.0, 30)
        crits.append(crit("① A 已 tick ≥30 拍且已在走", w is not None, f"{w}"))
        if w is None:
            return self.finish_case(7, crits + [crit("后续判据", False, "前置未成立")])
        row, st = self.snap()
        C = clamp_world(fwd(st, 2.0))
        ack = self.queue([C])
        crits.append(crit("② replace 回包 ok", ack.get("ok") is True, ack.get("err", "")))
        r2 = self.poll(lambda r: a_id is not None and r["task_id"] not in (None, a_id) and r["queue_len"] == 1,
                       0.5, "C 抢到队首")
        c_id = r2["task_id"] if r2 else None
        crits.append(crit("③ ≤0.5 s 内 task_id=C（≠A）、queue_len=1", r2 is not None,
                          f"c={c_id} a={a_id}"))
        crits.append(crit("④ C 首个采样 task_ticks ≤10（新实例从 0 起）",
                          (r2["task_ticks"] if r2 else 999) <= 10,
                          f"ticks={r2['task_ticks'] if r2 else None}"))
        win = self.seg(3.0)
        crits.append(crit("⑤ 3 s 内 task_id 从不回到 a",
                          all(r["task_id"] != a_id for r in win), f"a={a_id}"))
        # ③b（plan.md C4）：§12.8.6 第 4 条再加「立即生效」的时序证据。
        # 切片放在 3 s 窗口之后（不是紧接 ③）：tracing 是 non_blocking writer，
        # 紧接着 ack 读文件可能还没落盘；Δ 由两条日志行自身决定，读的时刻不影响数值。
        lines = self.slice(mark, 7)
        dt7, dt7_detail = log_action_delay_ms(lines, "[control] queue（整体替换）1 条", "[Sim] reset")
        self.verdicts["case7_cmd_to_reset_ms"] = None if dt7 is None else round(dt7, 2)
        crits.append(crit("③b `[control] queue（整体替换）1 条` → `[Sim] reset` Δ ≤ %.0f ms（立即生效）"
                          % LOG_TICK_TOL_MS,
                          dt7 is not None and dt7 <= LOG_TICK_TOL_MS, dt7_detail))
        r3 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE_BUDGET_S[7], "C 完成")
        crits.append(crit("⑥ C 完成 → queue_len=0、task_id=None", r3 is not None, f"{r3}"))
        return self.finish_case(7, crits)

    def case8(self):
        """取消（replace []）：立即停车、queue_len=0、nav_status=idle。"""
        self.cur = 8
        mark = self.log.mark()
        crits = []
        self.ensure_idle()
        row, st = self.snap()
        A = clamp_world(fwd(st, 5.0))
        self.queue([A])
        r1 = self.poll(lambda r: r["task_id"] is not None, 0.5, "A 开始")
        crits.append(crit("① A 开始", r1 is not None, f"{r1}"))
        w = self.wait_move(25.0, 10)
        crits.append(crit("② A 已在走", w is not None, f"{w}"))
        ack = self.queue([])
        ack_t = time.time()
        crits.append(crit("③ 取消回包 ok", ack.get("ok") is True, ack.get("err", "")))
        r2 = self.poll(lambda r: r["queue_len"] == 0 and r["task_id"] is None, 0.5, "取消生效")
        crits.append(crit("④ ≤0.5 s 内 queue_len=0、task_id=None", r2 is not None, f"{r2}"))
        r_idle = self.poll(lambda r: r["nav_status"] == "idle", 1.0, "nav 回 idle")
        crits.append(crit("⑤ ≤1.0 s 内 nav_status=idle（reset 清了 nav）", r_idle is not None,
                          f"nav={r_idle['nav_status'] if r_idle else None}"))
        # 停车：ack 后 0.6 s 与 1.6 s 两帧位移 ≤ STOP_TOL_M
        while time.time() - ack_t < 0.6:
            time.sleep(0.02)
        row_a, _ = self.snap()
        while time.time() - ack_t < 1.6:
            time.sleep(0.02)
        row_b, _ = self.snap()
        moved = dist((row_a["x"], row_a["y"]), (row_b["x"], row_b["y"]))
        crits.append(crit("⑥ 停车：ack+0.6 s → ack+1.6 s 位移 ≤ %.2f m" % STOP_TOL_M[self.stack],
                          moved <= STOP_TOL_M[self.stack], f"位移={moved:.4f} m"))
        # ⑤b（plan.md C4）：§12.8.6 第 4 条「取消后 queue_len / task_id 与预期一致」的
        # 时序证据（`[control] queue（整体替换）0 条` → `[Sim] reset`，同一 µs 时间轴）。
        # 切片放在 ⑥ 的 1.6 s 窗口之后：non_blocking writer 需要落盘时间（Δ 不受影响）。
        lines = self.slice(mark, 8)
        text = "".join(lines)
        dt8, dt8_detail = log_action_delay_ms(lines, "[control] queue（整体替换）0 条", "[Sim] reset")
        self.verdicts["case8_cmd_to_reset_ms"] = None if dt8 is None else round(dt8, 2)
        crits.append(crit("⑤b `[control] queue（整体替换）0 条` → `[Sim] reset` Δ ≤ %.0f ms（立即生效）"
                          % LOG_TICK_TOL_MS,
                          dt8 is not None and dt8 <= LOG_TICK_TOL_MS, dt8_detail))
        crits.append(crit("⑦ 日志切片含「队列已替换（0 条）」", "队列已替换（0 条）" in text, ""))
        return self.finish_case(8, crits)

    def case9(self):
        """不可达：超远目标 → 60 s Timeout → 失败上报 → 出队 → 后续 append 仍可执行。"""
        self.cur = 9
        mark = self.log.mark()
        crits = []
        self.ensure_idle()
        # v_est = 本轮 trace 的沿航向速度中位数（用例 1~8 的采样）
        speeds = []
        for a, b in zip(self.rows, self.rows[1:]):
            dt = b["t"] - a["t"]
            if dt <= 0:
                continue
            d = dist((a["x"], a["y"]), (b["x"], b["y"]))
            if d > 0.02:
                speeds.append(d / dt)
        speeds.sort()
        v_est = speeds[len(speeds) // 2] if speeds else 0.45
        row, st = self.snap()
        d_need = max(30.0, min(58.0, v_est * 70 * 1.2))
        U = fwd(st, d_need)
        note_geom = "沿航向"
        if not (0.0 <= U[0] < 128.0 and 0.0 <= U[1] < 128.0):
            U = fwd(st, -d_need)          # 越界 → 取反航向，仍保证 ≥30 m 不可达
            note_geom = "反航向（正航向越界）"
        U = clamp_world(U)
        ack = self.queue([U])
        crits.append(crit("① queue 回包 ok", ack.get("ok") is True, ack.get("err", "")))
        r1 = self.poll(lambda r: r["task_id"] is not None, 0.5, "U 开始")
        crits.append(crit("② U 开始", r1 is not None, f"{r1}"))
        r2 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE9_TIMEOUT_BUDGET_S, "U 失败出队")
        crits.append(crit("③ ≤%d s 内出现失败终值（queue_len=0、task_id=None）" % CASE9_TIMEOUT_BUDGET_S,
                          r2 is not None, f"{r2}"))
        # ③b（plan.md C5）：§12.8.3 用例 9「60 s Timeout → 失败上报 → 出队」——
        # 失败任务在 trace 里的**末次 task_ticks** 必须≥1100（≈55 s），证明 Timeout 在树内
        # 真的走完，排除「快速失败 / 被静默丢弃」。只用 trace 的 `task_ticks`，不读日志。
        u_id = r1["task_id"] if r1 else None
        t_series = [r["task_ticks"] for r in self.rows
                    if r["case"] == 9 and u_id is not None and r["task_id"] == u_id
                    and r["task_ticks"] is not None]
        last_ticks = t_series[-1] if t_series else None
        self.verdicts["case9_fail_ticks"] = last_ticks
        crits.append(crit("③b 失败任务末次 task_ticks ≥ %d（≈%.0f s：Timeout 真走完，不是快速失败）"
                          % (CASE9_MIN_FAIL_TICKS, CASE9_MIN_FAIL_TICKS * AUTO_TICK_S),
                          last_ticks is not None and last_ticks >= CASE9_MIN_FAIL_TICKS,
                          f"u_id={u_id} 末次 ticks={last_ticks}（{len(t_series)} 采样）"))
        lines = "".join(self.slice(mark, 9))
        m = re.search(r"任务 .* 失败，原因码 (\d+)", lines)
        code = int(m.group(1)) if m else None
        crits.append(crit("④ 日志命中「任务 … 失败，原因码 (1|3)」（期望 1=Timeout）",
                          code in (1, 3), f"实测码={code}"))
        self.verdicts["fail_code"] = code
        self.verdicts["case9_target"] = U
        self.verdicts["case9_v_est"] = round(v_est, 3)
        # 失败后 append：队列仍可用、新任务立即开始
        row, st = self.snap()
        S = clamp_world(fwd(st, 1.0))
        ack = self.append([S])
        crits.append(crit("⑤ append 回包 ok", ack.get("ok") is True, ack.get("err", "")))
        r3 = self.poll(lambda r: r["task_id"] is not None and r["queue_len"] == 1, 0.5, "S 立即开始")
        crits.append(crit("⑥ ≤0.5 s 内 task_id 非空、queue_len=1（出队后队列仍可用）",
                          r3 is not None, f"{r3}"))
        r4 = self.await_cond(lambda r: r["queue_len"] == 0 and r["task_id"] is None,
                       CASE_BUDGET_S[9], "S 完成")
        row, st = self.snap()
        crits.append(crit("⑦ S ≤%d s 内完成（设备未卡死）且末位姿 ≈ S" % CASE_BUDGET_S[9],
                          r4 is not None and abs(st["x"] - S[0]) <= ARRIVE_TOL_M
                          and abs(st["y"] - S[1]) <= ARRIVE_TOL_M,
                          f"S={S} pos=({st['x']:.2f},{st['y']:.2f})"))
        print(f"  用例 9 几何：v_est={v_est:.3f} m/s 目标={U}（{note_geom}，{d_need:.1f} m）"
              f" 失败码={code}")
        return self.finish_case(9, crits)

    def case10(self):
        """双栈：本栈 1~9 全 PASS；两栈合起来的 10 行终表由 `--pair` 出（§12.8.6 第 1 条）。"""
        ok = all(self.cases.get(n, {}).get("pass") for n in range(1, 10))
        self.cur = 10
        return self.finish_case(10, [crit(
            "本栈 1~9 全 PASS（双栈结论请跑 `--pair`：读两栈 cases.json 出 10 行终表）", ok,
            f"本轮 {self.stack}：{sum(1 for n in range(1, 10) if self.cases.get(n, {}).get('pass'))}/9")])


# ================================================================ 配置 / 前置

def write_config(stack, out, port):
    text = CONFIG_TEMPLATE.format(stack=stack, node_type=NODE_TYPE[stack], chassis=CHASSIS[stack],
                                  port=port, world_port=WORLD_PORT)
    if stack == "ugv":
        text += SENSOR_BLOCK
    cfg = out / ".config" / "config.toml"
    cfg.parent.mkdir(parents=True, exist_ok=True)
    cfg.write_text(text, encoding="utf-8")
    # 与世界默认参数一致：出现 chassis_params 即拒跑（改世界速度会让 9 条用例的时间预算失真）
    for ln in text.splitlines():
        s = ln.strip()
        if s and not s.startswith("#") and "chassis_params" in s:
            print(f"[CONFIG FAIL] 配置里出现 `chassis_params`：{s}")
            raise SystemExit(2)
    return cfg


def ensure_binary(out):
    """实例目录里的二进制：缺失或比 `target/debug/pleiades-sim` 旧 → 覆盖（幂等，便于复跑）。"""
    dst = out / "pleiades-sim"
    if not TARGET_BIN.exists():
        print(f"[PREFLIGHT FAIL] 缺少 {TARGET_BIN}（先 cargo build -p pleiades-sim）")
        return False
    if (not dst.exists()) or dst.stat().st_mtime < TARGET_BIN.stat().st_mtime:
        import shutil
        shutil.copy2(TARGET_BIN, dst)
        dst.chmod(0o755)
        print(f"  已同步二进制 → {dst}")
    return True


def wait_world_listen(log_path, timeout_s=60.0):
    t0 = time.time()
    needle = f"[ws] listening on port {WORLD_PORT}"
    while time.time() - t0 < timeout_s:
        if needle in "".join(read_log_lines(log_path)):
            print(f"  世界已在 {time.time() - t0:.1f}s 内监听 {WORLD_PORT}")
            return True
        time.sleep(0.2)
    return False


# ================================================================ 产物

def write_trace(out, rows):
    path = out / "trace.csv"
    fields = ["t", "case", "queue_len", "task_id", "task_ticks", "nav_status", "x", "y", "z", "yaw"]
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        for r in rows:
            w.writerow({k: ("" if r.get(k) is None else r.get(k)) for k in fields})
    return path


def residual_procs():
    """残留进程检查（pgrep 自检；无匹配 → 空列表）。"""
    out = []
    for pat in ("godot --headless", "pleiades-sim"):
        try:
            r = subprocess.run(["pgrep", "-af", pat], capture_output=True, text=True, timeout=5)
            for ln in r.stdout.splitlines():
                ln = ln.strip()
                if ln and "pgrep -af" not in ln:
                    out.append(ln)
        except (subprocess.SubprocessError, FileNotFoundError):
            pass
    return out


def write_conclusion(out, stack, h, hard, residual, extra):
    path = out / "conclusion.md"
    L = []
    L.append("# Task 36 Step 8 — sim 队列语义 e2e（%s 栈）\n" % stack)
    L.append(f"- 开始时间：{time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(h.t0))}")
    L.append(f"- 产物目录：`{out}`")
    L.append(f"- 控制口：`ws://{HOST}:{h.port}`；世界：`ws://{HOST}:{WORLD_PORT}`；栈：`{CHASSIS[stack]}`\n")
    L.append("## 一、用例总表（§12.8.3 的 10 条）\n")
    L.append("| # | 场景 | 结果 | 采样数 |")
    L.append("|---|---|---|---|")
    for n in range(1, 11):
        c = h.cases.get(n)
        if c is None:
            L.append(f"| {n} | {CASE_TITLES[n]} | MISSING | 0 |")
        else:
            L.append(f"| {n} | {CASE_TITLES[n]} | **{'PASS' if c['pass'] else 'FAIL'}** | {c['samples']} |")
    L.append("\n## 二、逐条判据\n")
    for n in range(1, 11):
        c = h.cases.get(n)
        if c is None:
            continue
        L.append(f"### 用例 {n} — {CASE_TITLES[n]}：{'PASS' if c['pass'] else 'FAIL'}\n")
        L.append("| 判据 | 结果 | 明细 |")
        L.append("|---|---|---|")
        for cr in c["criteria"]:
            L.append(f"| {cr['name']} | {'✅' if cr['ok'] else '❌'} | `{cr['detail']}` |")
        L.append("")
    L.append("## 三、硬检查\n")
    L.append("| # | 检查 | 结果 | 明细 |")
    L.append("|---|---|---|---|")
    for i, hc in enumerate(hard, 1):
        L.append(f"| {i} | {hc['name']} | {'✅' if hc['ok'] else '❌'} | `{hc['detail']}` |")
    L.append("")
    L.append("## 四、暂停 / 恢复 ticks 证据（用例 5 / 6；判定只认 `task_ticks`，§12.8.4）\n")
    L.append("| 用例 | 任务 | 阶段 | task_id | task_ticks | 暂停值 | 结论 |")
    L.append("|---|---|---|---|---|---|---|")
    for e in h.evidence:
        verdict = ""
        if e["phase"] == "resume" and e.get("pause") is not None and e.get("ticks") is not None:
            verdict = "不清零（同一实例）" if e["ticks"] >= e["pause"] else "**被清零（疑似重建）**"
        L.append(f"| {e['case']} | {e['task']} | {e['phase']} | {e.get('task_id')} | "
                 f"{e.get('ticks')} | {e.get('pause')} | {verdict} |")
    L.append("")
    if extra:
        L.append("## 五、本轮关键数字\n")
        for k, v in extra.items():
            L.append(f"- {k}：`{v}`")
        L.append("")
    L.append("## 六、残留进程检查\n")
    if residual:
        L.append("**有残留**：")
        for ln in residual:
            L.append(f"- `{ln}`")
    else:
        L.append("`pgrep -af 'godot --headless|pleiades-sim'` 为空 ✅")
    L.append("")
    path.write_text("\n".join(L), encoding="utf-8")
    return path


def print_table(h):
    print("\n=== 用例总表（%s） ===" % h.stack)
    for n in range(1, 11):
        c = h.cases.get(n)
        print(f"  {n:2d} {CASE_TITLES[n]:<22} {'PASS' if c and c['pass'] else ('FAIL' if c else 'MISSING')}")


# ================================================================ 双栈聚合（用例 10）

def pair_summary(dir_ugv=None, dir_uav=None):
    """用例 10（双栈，§12.8.6 第 1 条）：读两栈 cases.json 出 10 行终表 + 硬化校验（只读，不起进程）。

    任一栈不满足下列任一条 → 用例 10 FAIL（`problems` 写明原因）：
      1. `fatal` 为空；
      2. 用例 1~9 全 PASS；
      3. 用例 1~9 采样数 ≥ `PAIR_MIN_SAMPLES[n]`（"不接受 0 采样"这种软判据）；
      4. 该栈 `hard` 非空、全 ok、且条数 ≥ 8；
      5. 该栈 `build.binary_sha256` 非空（旧脚本/旧产物直接 FAIL，不给"配对到旧文件"留缝）；
      6. 两栈 `build.binary_sha256` 与 `build.script_sha256` 相同（同一构建 / 同一脚本）。

    终端打印 md 全文，并回写 `results/queue_pair.{md,json}`（json 含每栈 samples /
    samples_total / hard_count / build / source / mtime，供评审核对是否同一轮取证）。
    """
    dirs = {"ugv": pathlib.Path(dir_ugv) if dir_ugv else RESULTS / "queue_ugv",
            "uav": pathlib.Path(dir_uav) if dir_uav else RESULTS / "queue_uav"}
    data, problems = {}, []
    for st in ("ugv", "uav"):
        p = dirs[st] / "cases.json"
        if not p.exists():
            problems.append(f"缺 {p}（先跑 `python3 Tool/sim_queue_test.py --stack {st}`）")
            continue
        d = json.loads(p.read_text(encoding="utf-8"))
        cases = {c["case"]: c for c in d.get("cases", [])}
        fails = [n for n in range(1, 10) if not cases.get(n, {}).get("pass")]
        thin = [n for n in range(1, 10)
                if (cases.get(n, {}).get("samples") or 0) < PAIR_MIN_SAMPLES[n]]
        hard = d.get("hard") or []
        badhard = [hc.get("name") for hc in hard if not hc.get("ok")]
        build = d.get("build") or {}
        if d.get("fatal"):
            problems.append(f"{st} 栈 fatal={d['fatal']}")
        if fails:
            problems.append(f"{st} 栈用例 {fails} 未 PASS")
        if thin:
            problems.append(f"{st} 栈用例 {thin} 采样不足（下限 {PAIR_MIN_SAMPLES}）—— 不接受 0 采样")
        if not build.get("binary_sha256"):
            problems.append(f"{st} 栈产物无构建指纹（旧脚本/旧产物？重跑该栈）")
        if not hard:
            problems.append(f"{st} 栈 cases.json 无 hard 数组（旧脚本产物？）")
        elif badhard:
            problems.append(f"{st} 栈硬检查未过：{badhard}")
        elif len(hard) < 8:
            problems.append(f"{st} 栈硬检查只有 {len(hard)} 条（期望 ≥8）")
        data[st] = {
            "started_at": d.get("started_at"), "pass_1_9": not fails, "fails": fails,
            "thin": thin, "fatal": d.get("fatal"),
            "hard_ok": bool(hard) and not badhard, "hard_count": len(hard),
            "samples": {str(n): cases.get(n, {}).get("samples", 0) for n in range(1, 10)},
            "samples_total": sum((cases.get(n, {}).get("samples") or 0) for n in range(1, 10)),
            "build": build, "source": str(p),
            "mtime": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(p.stat().st_mtime)),
            "verdicts": d.get("verdicts") or {},
        }
    if len(data) == 2:
        b0, b1 = data["ugv"]["build"], data["uav"]["build"]
        for key, why in (("binary_sha256", "两栈二进制不同（sha256 不一致）"),
                         ("script_sha256", "两栈脚本版本不同（sha256 不一致）")):
            if b0.get(key) != b1.get(key):
                problems.append(f"{why}：ugv={str(b0.get(key))[:12]} uav={str(b1.get(key))[:12]}"
                                " → 用同一构建重跑两栈")
    ok = not problems and len(data) == 2
    pair = {"case10_pass": ok, "stacks": data, "problems": problems,
            "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime())}
    (RESULTS / "queue_pair.json").write_text(json.dumps(pair, ensure_ascii=False, indent=2),
                                             encoding="utf-8")

    L = ["# Task 36 Step 8 — 双栈终表（用例 10）\n",
         f"- 生成时间：{pair['generated_at']}",
         "- 判定：两栈各跑一轮，各自 1~9 全 PASS + 采样达标（下限 %s）+ 硬检查全过（≥8 条）"
         % PAIR_MIN_SAMPLES,
         "  + 两栈同一构建/同一脚本 ⇒ 用例 10 PASS（§12.8.6 第 1 条）\n",
         "| # | 场景 | ugv_sim | uav_sim | ugv 采样 | uav 采样 |",
         "|---|---|---|---|---|---|"]
    for n in range(1, 10):
        cells = []
        for st in ("ugv", "uav"):
            v = data.get(st) or {}
            samples = (v.get("samples") or {}).get(str(n))
            good = v.get("pass_1_9") and samples is not None and samples >= PAIR_MIN_SAMPLES[n]
            cells.append(("PASS" if good else "MISSING/FAIL", samples))
        L.append(f"| {n} | {CASE_TITLES[n]} | {cells[0][0]} | {cells[1][0]} | "
                 f"{cells[0][1]} | {cells[1][1]} |")
    L.append(f"| 10 | {CASE_TITLES[10]} | {'PASS' if ok else 'FAIL'} | {'PASS' if ok else 'FAIL'} | — | — |")
    L.append("\n## 用例 4 命令 → 设备动作延迟（日志 µs 时间戳；§12.8.6 第 3 条）\n")
    L.append("| 栈 | 命令→停车 | 命令→reset（用例 7/8） | 换头后位移 | 位移上界 |")
    L.append("|---|---|---|---|---|")
    for st in ("ugv", "uav"):
        v = data.get(st) or {}
        vd = v.get("verdicts") or {}
        L.append(f"| {st} | {vd.get('case4_cmd_to_suspend_ms')} ms | "
                 f"{vd.get('case7_cmd_to_reset_ms')}/{vd.get('case8_cmd_to_reset_ms')} ms | "
                 f"{vd.get('case4_moved_eff')} m | {vd.get('case4_bound')} m |")
    L.append("\n## 取证来源\n")
    L.append("| 栈 | cases.json | mtime | started_at | binary sha256[:12] | script sha256[:12] |")
    L.append("|---|---|---|---|---|---|")
    for st in ("ugv", "uav"):
        v = data.get(st) or {}
        b = v.get("build") or {}
        L.append(f"| {st} | `{v.get('source')}` | {v.get('mtime')} | {v.get('started_at')} | "
                 f"{str(b.get('binary_sha256'))[:12]} | {str(b.get('script_sha256'))[:12]} |")
    if problems:
        L.append("\n## 问题\n")
        L.extend(f"- {x}" for x in problems)
    text = "\n".join(L) + "\n"
    (RESULTS / "queue_pair.md").write_text(text, encoding="utf-8")

    print(text)
    print(f"产物：{RESULTS / 'queue_pair.json'}、{RESULTS / 'queue_pair.md'}")
    print(f"case10_pass = {ok}")
    return 0 if ok else 1

# ================================================================ main

def main():
    ap = argparse.ArgumentParser(description="Task 36 Step 8 — sim 队列语义 e2e（§12.8）")
    ap.add_argument("--stack", choices=["ugv", "uav"], default="ugv")
    ap.add_argument("--out-dir", default=None)
    ap.add_argument("--json", dest="json", action="store_true", default=True,
                    help="写 cases.json（默认开；--pair 聚合依赖它）")
    ap.add_argument("--no-json", dest="json", action="store_false")
    ap.add_argument("--pair", action="store_true",
                    help="用例 10（双栈）聚合：只读两栈 cases.json，不起任何进程；校验 fatal 为空 / "
                         "1~9 全 PASS / 采样下限 / hard ≥8 条且全过 / 两栈同一构建与同一脚本")
    ap.add_argument("--pair-dir-ugv", default=None,
                    help="--pair 的 ugv 产物目录（默认 results/queue_ugv；用 --out-dir 跑过就指过来）")
    ap.add_argument("--pair-dir-uav", default=None,
                    help="--pair 的 uav 产物目录（默认 results/queue_uav）")
    ap.add_argument("--port", type=int, default=None)
    ap.add_argument("--no-launch", action="store_true", help="连已在跑的控制口（不起世界/sim）")
    ap.add_argument("--smoke", action="store_true", help="只跑单任务冒烟（R1 前置）")
    args = ap.parse_args()

    if args.pair:
        return pair_summary(args.pair_dir_ugv, args.pair_dir_uav)

    stack = args.stack
    port = args.port if args.port is not None else PORT[stack]
    out = pathlib.Path(args.out_dir) if args.out_dir else (RESULTS / f"queue_{stack}")
    out = out.resolve()
    out.mkdir(parents=True, exist_ok=True)

    inst = TESTENV / f"simq_{stack}"
    inst.mkdir(parents=True, exist_ok=True)

    if not args.no_launch:
        if not ensure_binary(inst):
            return 2
        if not WORLD_DIR.is_dir():
            print(f"[PREFLIGHT FAIL] 缺少世界工程 {WORLD_DIR}")
            return 2
        if port_busy(port):
            print(f"[PREFLIGHT FAIL] 控制口 {port} 已被占用（有残留进程？）")
            return 2
        if port_busy(WORLD_PORT):
            print(f"[PREFLIGHT FAIL] 世界端口 {WORLD_PORT} 已被占用（有残留进程？）")
            return 2

    cfg = write_config(stack, inst, port)
    (out / "config.toml").write_text(cfg.read_text(encoding="utf-8"), encoding="utf-8")
    print(f"  配置：{cfg}（副本 → {out / 'config.toml'}）")

    # 构建指纹（plan.md C6）：`--pair` 用它卡「两栈来自同一构建、同一脚本」（§12.8.6 第 1 条）
    build = build_fingerprint(inst / "pleiades-sim")
    print("  构建指纹：binary=%s bytes=%s source=%s script=%s git=%s dirty=%s"
          % (str(build["binary_sha256"])[:12], build["binary_bytes"],
             str(build["source_sha256"])[:12], str(build["script_sha256"])[:12],
             str(build["git_head"])[:8], build["git_dirty"]))

    h = Harness(stack, out, port, inst / "Log")
    world = Proc("world")
    sim = Proc("sim")
    world_log = out / "world.log"
    sim_stdout = out / "sim_stdout.log"
    hard = []
    fatal = None
    exit_code = 1

    try:
        if not args.no_launch:
            world.start(["timeout", str(WORLD_TIMEOUT_S), "godot", "--headless", "--path", str(WORLD_DIR)],
                        ROOT, world_log)
            if not wait_world_listen(world_log, 60.0):
                print("[PREFLIGHT FAIL] 世界 60 s 内未监听 9999")
                return 2
            sim.start([str(inst / "pleiades-sim"), "--headless"], inst, sim_stdout)
            print("  等待控制口可连 …")
        if not h.ctl.connect(retries=240 if not args.no_launch else 20, delay=0.25):
            print(f"[PREFLIGHT FAIL] 控制口 {port} 连不上")
            return 2
        h.log.reset()

        # 首帧硬检查：4 个新键必须在（旧二进制会立刻暴露）
        st0 = h.ctl.status()
        missing = [k for k in STATUS_NEW_KEYS if k not in st0]
        if missing:
            print(f"[PREFLIGHT FAIL] status 缺键 {missing} —— 请先 cargo build -p pleiades-sim 并复制产物"
                  f"（当前二进制可能早于 Step 8）")
            return 2
        # 等装配（世界绑定 avatar）
        t0 = time.time()
        while time.time() - t0 < 120.0:
            if h.ctl.status().get("assembled"):
                break
            time.sleep(0.25)
        else:
            print("[PREFLIGHT FAIL] 120 s 内未 assembled（世界没绑定/世界没起）")
            return 2
        print(f"  已装配（{time.time() - t0:.1f}s）")

        # mode=auto 幂等：顺带 device.reset() 让起点干净
        h.command({"cmd": "mode", "value": "auto"}, "mode")
        h.ensure_idle()
        h.log.reset()

        if args.smoke:
            h.cur = 1
            mark = h.log.mark()
            row, st = h.snap()
            A = clamp_world(fwd(st, 2.0))
            h.queue([A])
            # 先等「已开始」，再等「已完成」：命令是异步投递的，直接等「队列空」会被
            # main_loop 消费之前的那一帧误判成完成（§R5）
            started = h.poll(lambda x: x["task_id"] is not None, 0.5, "冒烟开始")
            r = h.poll(lambda x: x["queue_len"] == 0 and x["task_id"] is None,
                       SMOKE_BUDGET_S, "冒烟到达") if started else None
            row, st = h.snap()
            ok = r is not None and abs(st["x"] - A[0]) <= ARRIVE_TOL_M and abs(st["y"] - A[1]) <= ARRIVE_TOL_M
            h.finish_case(1, [crit("单任务 queue → 到达 → 出队（≤%d s）" % SMOKE_BUDGET_S, ok,
                                   f"A={A} pos=({st['x']:.2f},{st['y']:.2f}) started={started is not None}")])
            h.slice(mark, 1)
        else:
            print("\n── 用例 1~9 ──────────────────────────────────")
            for fn in (h.case1, h.case2, h.case3, h.case4, h.case5,
                       h.case6, h.case7, h.case8, h.case9):
                print(f"\n[用例 {fn.__name__[4:]}]")
                try:
                    fn()
                except Exception as exc:  # 单条用例异常不能拖垮整轮取证
                    traceback.print_exc()
                    n = int(fn.__name__[4:])
                    h.cases[n] = {"case": n, "title": CASE_TITLES[n], "pass": False,
                                  "criteria": [crit("用例执行未抛异常", False, repr(exc))],
                                  "samples": len([r for r in h.rows if r["case"] == n])}
            print("\n[用例 10]")
            h.case10()

            # ── 硬检查 ──
            all_pass = all(h.cases.get(n, {}).get("pass") for n in range(1, 10))
            hard.append({"name": "1. 本轮用例 1~9 全 PASS", "ok": all_pass,
                         "detail": f"{sum(1 for n in range(1, 10) if h.cases.get(n, {}).get('pass'))}/9"})
            ticks_ok = True
            detail = []
            for e in h.evidence:
                if e["phase"] == "resume" and e.get("pause") is not None and e.get("ticks") is not None:
                    good = e["ticks"] >= e["pause"]
                    ticks_ok = ticks_ok and good
                    detail.append(f"case{e['case']}/{e['task']}: {e['pause']}→{e['ticks']}{'' if good else '(FAIL)'}")
            hard.append({"name": "2. 用例 5/6 的 task_ticks 恢复值 ≥ 暂停值（不清零）",
                         "ok": ticks_ok and bool(detail), "detail": "; ".join(detail) or "无采样"})
            c4 = h.cases.get(4, {}).get("criteria", [])
            yield_c = next((c for c in c4 if c["name"].startswith("④ 让位")), None)
            hard.append({"name": "3. 用例 4 停车延迟（换头后位移 ≤ 1 tick + 惯性 + 位姿量化）",
                         "ok": bool(yield_c and yield_c["ok"]),
                         "detail": yield_c["detail"] if yield_c else "无判据"})
            c78 = all(h.cases.get(n, {}).get("pass") for n in (7, 8))
            hard.append({"name": "4. 用例 7/8 之后 queue_len / task_id 符合预期", "ok": c78,
                         "detail": f"7={'PASS' if h.cases.get(7, {}).get('pass') else 'FAIL'} "
                                   f"8={'PASS' if h.cases.get(8, {}).get('pass') else 'FAIL'}"})
            lines, _ = h.log.all_lines()
            bad = [ln for ln in lines if "WARN" in ln or "warn" in ln
                   if ("丢弃" in ln) or ("忽略 Auto 命令" in ln)]
            hard.append({"name": "5. 全程日志无静默丢弃（WARN 含「丢弃 / 忽略 Auto 命令」= 0）",
                         "ok": len(bad) == 0, "detail": f"命中 {len(bad)} 条" + (f"：{bad[:2]}" if bad else "")})
            # 第 6、7 条（plan.md C6）：把 §12.8.6 第 3 条与用例 9 的时延从「普通判据」
            # 提升为**硬检查**（--pair 要求 hard ≥8 且全过）。
            c4c = h.cases.get(4, {}).get("criteria", [])
            cmd4 = next((c for c in c4c if c["name"].startswith("④b")), None)
            hard.append({"name": "6. 用例 4 命令→停车 Δ ≤ 1 tick（日志 µs 时间戳；§12.8.6 第 3 条）",
                         "ok": bool(cmd4 and cmd4["ok"]),
                         "detail": cmd4["detail"] if cmd4 else "无判据"})
            c9c = h.cases.get(9, {}).get("criteria", [])
            tick9 = next((c for c in c9c if c["name"].startswith("③b")), None)
            hard.append({"name": "7. 用例 9 失败任务末次 task_ticks ≥ %d（Timeout 真走完）"
                                 % CASE9_MIN_FAIL_TICKS,
                         "ok": bool(tick9 and tick9["ok"]),
                         "detail": tick9["detail"] if tick9 else "无判据"})

        write_trace(out, h.rows)
    except SystemExit:
        raise
    except Exception:
        traceback.print_exc()
        fatal = "运行期异常（见上方 traceback）"
    finally:
        try:
            h.ctl.close()
        except Exception:
            pass
        sim.stop()
        world.stop()

    residual = residual_procs()
    hard.append({"name": "8. 进程收尾（pgrep godot/pleiades-sim 为空）", "ok": len(residual) == 0,
                 "detail": f"{len(residual)} 条残留" + (f"：{residual[:2]}" if residual else "")})

    extra = {
        "v_est（用例 1~8 沿航向速度中位数）": h.verdicts.get("case9_v_est"),
        "用例 9 目标点 / 几何": h.verdicts.get("case9_target"),
        "用例 9 实测失败码（期望 1=Timeout；3=Unreachable 也算过）": h.verdicts.get("fail_code"),
        "暂停/恢复 ticks 对数": len([e for e in h.evidence if e['phase'] == 'resume']),
        "用例 4 命令→停车延迟 ms（日志 µs；④b 判据 ≤%.0f）" % LOG_TICK_TOL_MS:
            h.verdicts.get("case4_cmd_to_suspend_ms"),
        "用例 7 命令→reset 延迟 ms（③b 判据 ≤%.0f）" % LOG_TICK_TOL_MS:
            h.verdicts.get("case7_cmd_to_reset_ms"),
        "用例 8 命令→reset 延迟 ms（⑤b 判据 ≤%.0f）" % LOG_TICK_TOL_MS:
            h.verdicts.get("case8_cmd_to_reset_ms"),
        "用例 9 失败任务末次 task_ticks（③b 判据 ≥%d）" % CASE9_MIN_FAIL_TICKS:
            h.verdicts.get("case9_fail_ticks"),
    }
    write_conclusion(out, stack, h, hard, residual, extra)
    print_table(h)

    if args.json:
        (out / "cases.json").write_text(json.dumps({
            "stack": stack,
            "started_at": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(h.t0)),
            "out_dir": str(out),
            "cases": [h.cases[n] for n in sorted(h.cases)],
            "hard": hard,
            "evidence": h.evidence,
            "verdicts": h.verdicts,
            "build": build,
            "fatal": fatal,
        }, ensure_ascii=False, indent=2), encoding="utf-8")

    print("\n=== 硬检查 ===")
    for hc in hard:
        print(f"  {'PASS' if hc['ok'] else 'FAIL'}  {hc['name']}：{hc['detail']}")
    print(f"\n产物：{out}（trace.csv / conclusion.md{' / cases.json' if args.json else ''}）")

    if fatal:
        return 1
    if args.smoke:
        ok = h.cases.get(1, {}).get("pass", False)
        exit_code = 0 if ok else 1
    else:
        exit_code = 0 if all(hc["ok"] for hc in hard) and all(
            h.cases.get(n, {}).get("pass") for n in range(1, 11)) else 1
    print(f"\nEXIT={exit_code}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
