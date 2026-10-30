#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Presented by KeJi
# Created Date ： 2026-10-29
# Modified Date ： 2026-10-29
"""Task 40 Step 2 — 验证：最新的小车能不能正常运动（**只验不改**）。

被测链路：`pleiades-sim` 车栈连续档执行器（`sim/ugv_sim/executor.rs`）→
`motion_cmd{action:"velocity", speed, turn}` → 世界 `DiffDrive.gd` 的 `"velocity"` 分支
（前进档位与转向档位**同时生效**）。
本脚本**一行不改被测代码、不改判据阈值**（§八 定值：`turn_speed=30`、`DEADBAND=5°`、
`ARC_MAX=10°`、`PASS_THROUGH=0.5 m`）；判据不通过就如实写 FAIL，不为了变绿改判据。

环境：`RobotSim_TestEnv/world_fast/`（源树整树副本 + `override.cfg` 的 `msaa_3d=0`；
llvmpipe 直跑 2 fps / 位姿龄 245 ms 会让判据失真）+ `xvfb-run -a` 起的 x11 显示
+ `RobotSim_TestEnv/simq_ugv`（控制口 9098）。

用例（6）：M0 静止 10 s｜M1 正东 20.25 m｜M2 L 形 18+18 m｜M3 正西 22 m（44 格）｜
M4 45° 斜线 19.80 m｜M5 窄通道 20+20+17.5+14 m（世界最窄可通行缝隙 3.5 m = 7 格，见 D2）。
判据（8）：到达 / 不抖 / 不打转 / 不撞墙 / 该停会停 / 停走对比（**只记录**）/ 日志可解析 / 无回归。

用法：
    python3 Tool/sim_task40_ugv_test.py --dry-run                  # 只跑前置检查（不起进程）
    python3 Tool/sim_task40_ugv_test.py --only M0,M1,M4            # 局部跑（调试/冒烟）
    python3 Tool/sim_task40_ugv_test.py --out-dir RobotSim_TestEnv/results/task40_<UTC>
    其他开关：--port PORT   --no-launch（连已在跑的控制口）   --no-json
退出码：0 = 8 条判据全 PASS（判据 6 只记录）；1 = 有 FAIL；2 = 环境/前置失败（产物不可信）

产物：`<out>/{trace.csv, world.log, sim.log, verdict.md, cases.json, config.toml, sim_stdout.log}`，
`verdict.md` 正文 ≤28 行、逐条给实测值 + PASS/FAIL。

--------------------------------------------------------------- 口径偏差登记（5 条）
D1 任务文档要求"复用 `sim_queue_test.py`、`sim_follow_test.py` 的既有设施"。本脚本**只复用前者**
   （`WS` / `Proc` / `Control` / `LogTail` / `port_busy` / `residual_procs` / `sha256_file` /
   `parse_log_ts` / `read_log_lines` / `clamp_world` / `dist` / `crit` / `SAMPLE_DT`）；
   显示层按 `step.md` 走 `xvfb-run -a` —— `sim_follow_test` 自带 `Xvfb :99` 类，与本 step 的规定冲突。
D2 用例 M5 要求"1 格宽窄通道 + 转角"。世界 `World.gd::OBSTACLES` 全表两两算过**不存在** 1 格
   （0.5 m）走廊：最小可通行缝隙是 **3.5 m（7 格）** = `wall_d` 北端 y=124 ↔ `wall_north` y=127.5，
   次小 5 m、7 m。不改世界是本 step 的红线，故 M5 用这条最窄缝隙（入射/出射各一次 90°）。
D3 任务文档要求"物理时间用位姿时间戳，不用墙钟"。控制口 `status` **不带**任何时间戳字段，
   世界也不广播 pose 事件 ⇒ 时间轴改用 `[Sim]` 日志行的 **µs 时间戳**（同进程单调），并用一次
   `[Sim] stop` 探针量出「日志钟 − 墙钟」偏移（写进 verdict 头部，实测量级 <50 ms）。
D4 判据 4 要求"全程最小 lidar 距离"。sim **不打印任何 range 值**、控制口无传感器字段、
   世界 WS 无观察者 API ⇒ 全向 lidar 无观测源。改为双口径：4a 仪器化（`[Sim] 前方障碍` 命中 0
   = 全程无急停）+ 4b 几何（轨迹点到 `World.gd::OBSTACLES` 全表矩形的最短距离，运行时解析源文件，
   不手抄）。几何净空是**保守替代**：中心距 ≥0.3 m ⇒ 车体边缘 ≥0.15 m（车体 0.30×0.25）。
D5 前置检查 #1「`git status` 里 `Task/`、`.github/`、`Cargo.toml`、`Cargo.lock` 无新条目」按字面
   会误报：进场基线里本来就有 `?? Task/task_40_ugv_motion.md`（Step 1 建的文档）。故改成与
   `tmp/task40/git_baseline.txt` 逐行比对这几个路径的**条目数不变**（等价于"本步没动它们"）。
"""

import argparse
import json
import math
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time
import traceback

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "Tool"))
import sim_queue_test as Q  # noqa: E402  （复用设施；`if __name__` 守护，import 安全）

WORLD_SRC = ROOT / "orion-simulation-world"
WORLD_FAST = ROOT / "RobotSim_TestEnv" / "world_fast"
INST = ROOT / "RobotSim_TestEnv" / "simq_ugv"
RESULTS = ROOT / "RobotSim_TestEnv" / "results"
TARGET_BIN = ROOT / "target" / "debug" / "pleiades-sim"
BASELINE_LOG = RESULTS / "queue_ugv_dev" / "sim_1.log"
GIT_BASELINE = ROOT / "tmp" / "task40" / "git_baseline.txt"
FROZEN_PATHS = ("Task/", ".github/", "Cargo.toml", "Cargo.lock")

HOST = "127.0.0.1"
WORLD_PORT = 9999
DEFAULT_PORT = 9098
WORLD_TIMEOUT_S = 1200                 # 世界自带 `timeout` 护栏（本轮预算 ~600 s）
OVERRIDE_TEXT = "[rendering]\nanti_aliasing/quality/msaa_3d=0\n"
WORLD_COPIED_MARK = ".copied_from"
OVERRIDE_TAG = "msaa_3d=0"

# ─── §八 已定参数（**一个字不改**，写进 config 并回读断言）────────────────────
FORWARD_SPEED = 30                     # §八 #1（前进档位 0~100）
TURN_SPEED = 30                        # §八 #1（转向档位 0~100 ⇒ 27 °/s）
DEADBAND_DEG = 5.0                     # §八 #2（只作日志记录，不参与判定）
ARC_MAX_DEG = 10.0                     # §八 #2
PASS_THROUGH_M = 0.5                   # §八 #3

# ─── 本步口径（§三 判据 + 代码事实）──────────────────────────────────────────
SPEED_MPS = 0.45                       # 世界 `_max_speed=1.5` × 30%
YAW_RATE_DPS = 27.0                    # 世界 `_max_turn_rate=90 °/s` × 30%
CELL = 0.5                             # `CELL_RESOLUTION`
TASK_TIMEOUT_S = 60                    # `goto.rs:31 TASK_TIMEOUT_MS`（用例分段的上限依据）
MAX_LEG_M = 20.0                       # 单段设计上限 = 60 s × 0.45 m/s 留 25% 余量
ARRIVE_TOL_M = 0.26                    # 判据 1 的逐轴口径（`navigator.rs:60`）
TURN_FLIP_MAX = {"M1": 2, "M4": 3}     # 判据 2（§三.3）
NO_SPIN_WIN_S = 10.0                   # 判据 3
NO_SPIN_MIN_M = 2.0                    # 判据 3
MIN_CLEAR_M = 0.3                      # 判据 4
STOP_LAT_S = 1.0                       # 判据 5
POSE_AGE_GATE_MS = 150                 # 环境闸门（位姿龄中位数上限）
M0_WINDOW_S = 10.0                     # M0 静止采样窗
SETTLE_S = 1.0                         # 每段之后的静置窗（判据 5(2) 的"任务后 1 s"）
HOME_TOL_M = 0.6                       # 归位判据（逐轴）
START_WAIT_S = 5.0                     # 命令 ack → 队首 task_id 非 None 的等待上限
SEG_TIMEOUT_S = 78.0                   # 单段等待上限 = 60 s Timeout + 建树/投递余量
SAMPLE_DT = Q.SAMPLE_DT                # 200 ms（复用 `sim_queue_test.py`）
CASE_BUDGET_S = {"M0": 15, "M1": 90, "M2": 130, "M3": 90, "M4": 90, "M5": 220}
CASE_ORDER = ["M0", "M1", "M2", "M3", "M4", "M5"]

# ================================================================ 日志正则（逐字对齐打点）
# `sim_handler.rs:804`（连续档）与 `:785`（旧离散档）、`navigator.rs:277/290/350`、
# `robot.rs:486`、`sim_handler.rs:859`、`emergency_stop.rs:132`。
EXEC_RE = re.compile(
    r"^(?P<ts>\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6})Z\s+INFO\s+"
    r"\[Sim\] UGV 位置=\((?P<x>-?\d+\.\d+),(?P<y>-?\d+\.\d+)\) "
    r"yaw=(?P<yaw>-?\d+\.\d+)° next=(?P<next>None|Some\(\((?P<gx>-?\d+), (?P<gy>-?\d+)\)\)) "
    r"→ speed=(?P<speed>\d+) turn=(?P<turn>-?\d+) 位姿龄=(?P<age>\d+)ms\s*$"
)
EXEC_TAG = "[Sim] UGV"
DISCRETE_RE = re.compile(r"\[Sim\] 位置=\(")          # 旧离散臂 ⇒ 必须 0 行
ARRIVE_PATS = ("[Nav] 到达目标（预测停点）", "[Nav] 已在目标格且逐轴误差 <= ")
DONE_PAT = "[Robot] 任务完成"
FAIL_PAT = "[Sim] 任务 "
FAIL_SUFFIX = " 失败，原因码 "
BLOCKED_PAT = "拍无路"
ESTOP_PAT = "[Sim] 前方障碍"
BASE_ACTION_RE = re.compile(r"\[Sim\] 位置=\([^)]*\) .*动作=(?P<act>\w+)")
OBSTACLE_RE = re.compile(
    r'\{"name":\s*"(?P<name>[A-Za-z_0-9]+)",\s*"origin":\s*Vector3\('
    r'(?P<ox>-?[\d.]+),\s*(?P<oy>-?[\d.]+),\s*(?P<oz>-?[\d.]+)\),\s*"size":\s*Vector3\('
    r'(?P<sx>-?[\d.]+),\s*(?P<sy>-?[\d.]+),\s*(?P<sz>-?[\d.]+)\)\}'
)


# ================================================================ 小工具

def Log(msg=""):
    print(msg, flush=True)


def cc(gx, gy):
    """网格坐标 → 世界坐标（**格心** `(g+0.5)*0.5`，与执行器/导航同口径）。"""
    return (round((gx + 0.5) * CELL, 2), round((gy + 0.5) * CELL, 2))


def utc_stamp():
    return time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())


def run_capture(argv, timeout=30):
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout, r.stderr
    except (subprocess.SubprocessError, OSError, FileNotFoundError) as exc:
        return -1, "", repr(exc)


def read_text(path):
    try:
        return pathlib.Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


# ================================================================ 用例表（坐标全部取格心）
# 出生点 `(64,64)`、`yaw=0`（`World.SPAWN_WORLD`，确定性，无 RNG）。
# 每段单独下发一次 mission（首段 `queue` = 清场，后续 `append` = 队列空时立即开始）。
CASES = [
    {"name": "M0", "title": "静止（不发任务）", "start": (64.0, 64.0), "segs": [],
     "desc": "10 s 采样"},
    {"name": "M1", "title": "直线 20 m", "start": (64.0, 64.0),
     "segs": [cc(168, 128)], "desc": "正东 20.25 m"},
    {"name": "M2", "title": "L 形（两次 90°）", "start": cc(168, 128),
     "segs": [cc(168, 164), cc(132, 164)], "desc": "正北 18 m → 正西 18 m"},
    {"name": "M3", "title": "长路径 40~50 格", "start": cc(132, 164),
     "segs": [cc(88, 164)], "desc": "正西 22 m（44 格）"},
    {"name": "M4", "title": "45° 斜线 20 m", "start": cc(88, 164),
     "segs": [cc(60, 136)], "desc": "正西南 19.80 m（dx=dy=−14）"},
    {"name": "M5", "title": "窄通道 + 转角（3.5 m 门洞，D2）", "start": cc(60, 136),
     "segs": [cc(60, 176), cc(60, 216), cc(60, 251), cc(88, 251)],
     "desc": "正北 20+20+17.5 m → 90° 右转东穿 14 m"},
]
CASE_BY_NAME = {c["name"]: c for c in CASES}


# ================================================================ 世界副本（同 task_39 Step 3）

def world_stamp(world_dir):
    """副本新鲜度标记：源树 HEAD + src/assets/tool 最新 (mtime,path) + project.godot sha。"""
    wd = pathlib.Path(world_dir)
    rc, out, _ = run_capture(["git", "-C", str(wd), "rev-parse", "HEAD"], timeout=60)
    head = out.strip()[:12] if rc == 0 else "?"
    newest = (0.0, "")
    for sub in ("src", "assets", "tool"):
        base = wd / sub
        if not base.is_dir():
            continue
        for dirpath, _dirnames, filenames in os.walk(str(base)):
            for fn in filenames:
                p = os.path.join(dirpath, fn)
                try:
                    st = os.stat(p)
                except OSError:
                    continue
                key = (st.st_mtime, p)
                if key > newest:
                    newest = key
    proj = wd / "project.godot"
    psha = Q.sha256_file(proj)[:16] if proj.is_file() else "?"
    return "HEAD=%s\nnewest=%s|%s\nproject=%s" % (head, int(newest[0]), newest[1], psha)


def world_marker_path():
    """标记放 `.godot/` 内：`diff -r` 排除 `.godot` 后，副本里只剩 `override.cfg` 一个非原文件。"""
    return WORLD_FAST / ".godot" / WORLD_COPIED_MARK


def ensure_world_fast(force=False):
    """幂等创建/刷新世界副本（源世界零改动；副本唯一差异 = `override.cfg`）。"""
    want = world_stamp(WORLD_SRC)
    marker = world_marker_path()
    try:
        have = marker.read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        have = ""
    ready = (WORLD_FAST / "project.godot").is_file() and (WORLD_FAST / ".godot" / "imported").is_dir()
    ent = {"copied": False, "reason": "", "seconds": 0.0, "stamp": want.replace("\n", " / ")}
    if (not force) and have == want.strip() and ready:
        ent["reason"] = "副本已存在且标记一致 → 复用"
        Log("[world_fast] %s" % ent["reason"])
        return ent
    ent["reason"] = "源树变化 → 重拷" if have else "首次创建"
    t0 = time.monotonic()
    if WORLD_FAST.exists():
        shutil.rmtree(WORLD_FAST)
    shutil.copytree(WORLD_SRC, WORLD_FAST,
                    ignore=shutil.ignore_patterns("override.cfg", WORLD_COPIED_MARK))
    (WORLD_FAST / "override.cfg").write_text(OVERRIDE_TEXT, encoding="utf-8")
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.write_text(want + "\n", encoding="utf-8")
    ent["copied"] = True
    ent["seconds"] = round(time.monotonic() - t0, 2)
    Log("[world_fast] %s（%.2f s，stamp=%s）" % (ent["reason"], ent["seconds"], ent["stamp"]))
    return ent


def world_copy_sync():
    """副本与源的 `DiffDrive.gd` 是否逐字节相同（A4：证明副本里有 `velocity` 分支）。"""
    a = WORLD_SRC / "src" / "chassis" / "DiffDrive.gd"
    b = WORLD_FAST / "src" / "chassis" / "DiffDrive.gd"
    if not (a.is_file() and b.is_file()):
        return False
    return Q.sha256_file(a) == Q.sha256_file(b)


def world_diff_extra():
    """`diff -r`（排除 `.godot`/`override.cfg`/标记）：期望空。"""
    rc, out, _ = run_capture(
        ["diff", "-r", "-q", "-x", ".godot", "-x", "override.cfg", "-x", WORLD_COPIED_MARK,
         str(WORLD_SRC), str(WORLD_FAST)], timeout=180)
    return rc, [ln for ln in out.splitlines() if ln.strip()]


# ================================================================ 配置 / 二进制

CONFIG_TEMPLATE = """\
# Presented by KeJi
# Created Date ： 2026-10-29
# Modified Date ： 2026-10-29
#
# Pleiades 配置（task_40 Step 2 小车运动验证；由 Tool/sim_task40_ugv_test.py 生成并幂等覆写，
# RobotSim_TestEnv/ 已被 .gitignore 忽略）
# §八 #1：forward_speed=30 / turn_speed=30（转向 27 °/s）
# **不加** [sim_world.chassis_params]（世界默认 max_speed=1.5 / max_turn_rate=90）

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
peer_name = "simq_ugv"
node_type = "car"

[sim_world]
enabled = true
url = "ws://127.0.0.1:{world_port}"
chassis = "diff_drive_ugv"

[chassis]
enabled = true
forward_speed = {forward_speed}
turn_speed = {turn_speed}

[control]
enabled = true
bind = "127.0.0.1"
port = {port}

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


def write_config(port, out=None):
    """自带模板写实例配置（**禁止**调用 `Q.write_config`：它会把 turn_speed 覆写成 10）。"""
    text = CONFIG_TEMPLATE.format(world_port=WORLD_PORT, forward_speed=FORWARD_SPEED,
                                  turn_speed=TURN_SPEED, port=port)
    cfg = INST / ".config" / "config.toml"
    cfg.parent.mkdir(parents=True, exist_ok=True)
    cfg.write_text(text, encoding="utf-8")
    if out is not None:                       # 产物副本（plan §2.3 附加项）
        (out / "config.toml").write_text(text, encoding="utf-8")
    # 世界速度不得被配置覆盖（改世界速度会让所有时间预算失真）
    for ln in text.splitlines():
        s = ln.strip()
        if s and not s.startswith("#") and "chassis_params" in s:
            Log("[CONFIG FAIL] 配置里出现 `chassis_params`：%s" % s)
            raise SystemExit(2)
    return cfg


def config_readback(cfg, port):
    """回读 §八 值（判据：写了什么就是什么，不允许被模板悄悄改掉）。"""
    want = {"forward_speed": str(FORWARD_SPEED), "turn_speed": str(TURN_SPEED),
            "port": str(port), "chassis": '"diff_drive_ugv"', "node_type": '"car"'}
    got, bad, has_params = {}, [], False
    sec = ""
    for ln in read_text(cfg).splitlines():
        s = ln.strip()
        if not s or s.startswith("#"):
            continue
        if s.startswith("["):
            sec = s
            if "chassis_params" in s:
                has_params = True
            continue
        if "chassis_params" in s:
            has_params = True
        if "=" in s:
            k, _, v = s.partition("=")
            got["%s.%s" % (sec, k.strip())] = v.strip()
    for k, v in want.items():
        real = got.get("[chassis].%s" % k) or got.get("[control].%s" % k) or got.get("[sim_world].%s" % k)
        if real is None:
            real = got.get("[Identity].%s" % k)
        if real != v:
            bad.append("%s=%r≠%r" % (k, real, v))
    if has_params:
        bad.append("出现了 chassis_params")
    return (not bad), bad


def binary_sources_newest():
    """`pleiades-sim/src/**/*.rs` 的最新 mtime（二进制新鲜度判据）。"""
    newest = (0.0, "")
    base = ROOT / "pleiades-sim" / "src"
    for dirpath, _d, filenames in os.walk(str(base)):
        for fn in filenames:
            if not fn.endswith(".rs"):
                continue
            p = os.path.join(dirpath, fn)
            try:
                st = os.stat(p)
            except OSError:
                continue
            if st.st_mtime > newest[0]:
                newest = (st.st_mtime, p)
    return newest


def ensure_binary():
    """实例二进制与 `target/debug/pleiades-sim` 同步（丢失/更旧才拷）。"""
    if not TARGET_BIN.exists():
        return False, "缺少 %s（先 cargo build -p pleiades-sim）" % TARGET_BIN
    dst = INST / "pleiades-sim"
    if (not dst.exists()) or dst.stat().st_mtime < TARGET_BIN.stat().st_mtime:
        shutil.copy2(TARGET_BIN, dst)
        dst.chmod(0o755)
        Log("  已同步二进制 → %s" % dst)
    same = Q.sha256_file(dst) == Q.sha256_file(TARGET_BIN)
    return same, "sha256 一致" if same else "实例二进制与 target 不一致"


# ================================================================ 起停

def world_argv():
    """世界 argv：`xvfb-run -a` 自动选号（不抢 `sim_follow_test` 的 :99）+ 非 headless + 副本路径。"""
    return ["timeout", str(WORLD_TIMEOUT_S), "xvfb-run", "-a", "-s", "-screen 0 1600x900x24",
            "godot", "--display-driver", "x11", "--rendering-driver", "opengl3",
            "--audio-driver", "Dummy", "--path", str(WORLD_FAST),
            "--resolution", "1600x900", "--position", "0,0"]


def wait_world_listen(log_path, timeout_s=150.0):
    needle = "[ws] listening on port %d" % WORLD_PORT
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        if needle in read_text(log_path):
            return time.time() - t0
        time.sleep(0.2)
    return None


def residual_all(with_zombie=True):
    """残留进程（含 Xvfb）：`Q.residual_procs` 只看 godot --headless/pleiades-sim，这里补 Xvfb/全量。

    `<defunct>`（僵尸）只占进程表 1 个槽、不持端口/文件，父进程退出即被回收，
    所以判据只看**活的**；数量单独记进 verdict（本机常年有几百个与本次无关的僵尸，不能当残留）。
    """
    out = list(Q.residual_procs())
    for pat in ("Xvfb", "xvfb-run", "godot"):
        rc, so, _ = run_capture(["pgrep", "-af", pat], timeout=5)
        if rc != 0:
            continue
        for ln in so.splitlines():
            ln = ln.strip()
            if not ln or "pgrep -af" in ln:
                continue
            if ln not in out:
                out.append(ln)
    if not with_zombie:
        out = [ln for ln in out if "<defunct>" not in ln]
    return out


def wait_clean(timeout=6.0):
    """等收尾干净：活进程 0 且 9999/控制口都空（最多等 `timeout` 秒）。返回 (残留, 僵尸数)。"""
    t0 = time.time()
    res = residual_all()
    while time.time() - t0 < timeout:
        res = residual_all()
        live = [x for x in res if "<defunct>" not in x]
        if not live and not Q.port_busy(WORLD_PORT) and not Q.port_busy(DEFAULT_PORT):
            break
        time.sleep(0.3)
    live = [x for x in res if "<defunct>" not in x]
    return live, len(res) - len(live)


# ================================================================ 采样

def is_drained(row):
    return row.get("task_id") is None and (row.get("queue_len") or 0) == 0


class Sampler:
    """200 ms 采样 `status` → trace 行；断线自动重连（单次失败不拖垮取证）。"""

    def __init__(self, ctl):
        self.ctl = ctl
        self.rows = []
        self.windows = []
        self.reconnects = 0

    def status(self):
        try:
            return self.ctl.status()
        except (OSError, Q.WsClosed, RuntimeError):
            self.ctl.close()
            if not self.ctl.connect(retries=8, delay=0.1):
                raise
            self.reconnects += 1
            return self.ctl.status()

    def row(self, case, phase, st):
        r = {
            "t_wall": round(time.time(), 3),
            "case": case,
            "phase": phase,
            "x": round(float(st.get("x", 0.0)), 3),
            "y": round(float(st.get("y", 0.0)), 3),
            "z": round(float(st.get("z", 0.0)), 3),
            "yaw": round(float(st.get("yaw", 0.0)), 4),
            "nav_status": st.get("nav_status"),
            "task_id": st.get("task_id"),
            "task_ticks": st.get("task_ticks"),
            "queue_len": st.get("queue_len"),
        }
        self.rows.append(r)
        return r

    def seg(self, seconds, case, phase, stop=None):
        """连续采样（200 ms/条）直到 seconds 或 stop(row)；返回本次行。"""
        rows = []
        t_s = time.time()
        while True:
            rows.append(self.row(case, phase, self.status()))
            if stop is not None and stop(rows[-1]):
                break
            if time.time() - t_s >= seconds:
                break
            time.sleep(SAMPLE_DT)
        return rows

    def flush_trace(self, out):
        write_trace(out, self.rows)


def write_trace(out, rows):
    import csv
    fields = ["t_wall", "case", "phase", "x", "y", "z", "yaw",
              "nav_status", "task_id", "task_ticks", "queue_len"]
    with (out / "trace.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        for r in rows:
            w.writerow({k: ("" if r.get(k) is None else r.get(k)) for k in fields})


# ================================================================ 用例执行

class Runner:
    def __init__(self, sampler, ctl, port):
        self.s = sampler
        self.ctl = ctl
        self.port = port
        self.case_time = {}
        self.case_abort = {}
        self.liveness = {}

    # ── 命令 ────────────────────────────────────────────────
    def cmd(self, obj, note):
        r = self.ctl.send_cmd(obj)
        if not r.get("ok"):
            Log("  [!] %s 回包失败：%s" % (note, r))
        return r

    def mission(self, cmd, target):
        return self.cmd({"cmd": cmd, "missions": [{"x": target[0], "y": target[1]}]}, cmd)

    # ── 归位（§3.9；归位任务不参与任何判据）──────────────────
    def ensure_home(self, name, target):
        st = self.s.status()
        d = max(abs(float(st.get("x", 0.0)) - target[0]), abs(float(st.get("y", 0.0)) - target[1]))
        if d <= HOME_TOL_M:
            return True
        dist = Q.dist((float(st.get("x", 0.0)), float(st.get("y", 0.0))), target)
        if dist > MAX_LEG_M:
            Log("  [%s] 未执行（前序污染）：当前 (%.2f,%.2f) 距期望起点 %.2f m > %.1f m"
                % (name, st["x"], st["y"], dist, MAX_LEG_M))
            return False
        Log("  [%s] 归位 → (%.2f,%.2f)（%.2f m，不计入判据）" % (name, target[0], target[1], dist))
        t0 = time.time()
        win = {"case": name, "seg": 0, "kind": "setup", "target": target, "t0": t0,
               "t1": None, "t_drain": None, "drain_row": None, "aborted": False}
        resp = self.mission("queue", target)
        if not resp.get("ok"):
            win["aborted"] = True
            win["t1"] = time.time()
            self.s.windows.append(win)
            return False
        self.s.seg(START_WAIT_S, name, "setup", stop=lambda r: r["task_id"] is not None)
        self.s.seg(SEG_TIMEOUT_S, name, "setup", stop=is_drained)
        win["t_drain"] = time.time()
        self.s.seg(SETTLE_S, name, "settle")
        win["t1"] = time.time()
        self.s.windows.append(win)
        return True

    # ── 单段 ────────────────────────────────────────────────
    def run_segment(self, name, idx, target, use_append):
        t0 = time.time()
        win = {"case": name, "seg": idx, "kind": "mission", "target": target, "t0": t0,
               "t1": None, "t_drain": None, "drain_row": None, "aborted": False}
        resp = self.mission("append" if use_append else "queue", target)
        if not resp.get("ok"):
            win["aborted"] = True
            win["t1"] = time.time()
            self.s.windows.append(win)
            return win
        started = self.s.seg(START_WAIT_S, name, "case", stop=lambda r: r["task_id"] is not None)
        if started[-1]["task_id"] is None:
            Log("  [%s.%d] 命令 ack 后 %.1f s 内没开始（队首仍是空）" % (name, idx, START_WAIT_S))
        rows = self.s.seg(SEG_TIMEOUT_S, name, "case", stop=is_drained)
        last = rows[-1]
        win["t_drain"] = last["t_wall"]
        win["drain_row"] = last
        if not is_drained(last):
            win["aborted"] = True
            Log("  [%s.%d] %.0f s 内未 drain（task_id=%s queue_len=%s）→ 发 stop 清场"
                % (name, idx, SEG_TIMEOUT_S, last["task_id"], last["queue_len"]))
            self.cmd({"cmd": "stop"}, "stop")
        # 任务后 1 s 的静置窗（判据 5(2) 的观测窗口）
        self.s.seg(SETTLE_S, name, "settle")
        win["t1"] = time.time()
        self.s.windows.append(win)
        Log("  [%s.%d] → (%.2f,%.2f) 用时 %.1f s%s"
            % (name, idx, target[0], target[1], win["t1"] - win["t0"],
               "（未 drain）" if win["aborted"] else ""))
        return win

    # ── 单个用例 ────────────────────────────────────────────
    def run_case(self, case, tail, out):
        name = case["name"]
        t_case = time.time()
        if case["segs"]:
            if not self.ensure_home(name, case["start"]):
                self.case_abort[name] = "未执行（前序污染）"
                return
        if not case["segs"]:
            # M0：不发任务，原地采 10 s
            win = {"case": name, "seg": 0, "kind": "idle", "target": case["start"],
                   "t0": time.time(), "t1": None, "t_drain": None, "drain_row": None,
                   "aborted": False}
            self.s.seg(M0_WINDOW_S, name, "case")
            win["t1"] = time.time()
            self.s.windows.append(win)
            Log("  [M0] 静止采样 %.0f s" % M0_WINDOW_S)
        else:
            for i, tgt in enumerate(case["segs"]):
                w = self.run_segment(name, i + 1, tgt, use_append=(i > 0))
                if w["aborted"]:
                    self.case_abort[name] = "第 %d 段未 drain" % (i + 1)
                    break
                if time.time() - t_case > CASE_BUDGET_S[name]:
                    self.case_abort[name] = "用例预算 %d s 用尽" % CASE_BUDGET_S[name]
                    break
        self.case_time[name] = round(time.time() - t_case, 1)
        self.s.flush_trace(out)

    # ── 执行器活体闸门（§3.8）──────────────────────────────
    def liveness_gate(self, tail):
        lines, _ = tail.all_lines()
        ugv = sum(1 for ln in lines if EXEC_TAG in ln)
        old = sum(1 for ln in lines if DISCRETE_RE.search(ln))
        self.liveness = {"ugv_lines": ugv, "discrete_lines": old}
        ok = ugv >= 5 and old == 0
        Log("  [活体闸门] `[Sim] UGV` %d 行 / 旧离散 `[Sim] 位置=(` %d 行 → %s"
            % (ugv, old, "OK（跑的是连续执行器）" if ok else "FAIL"))
        return ok


# ================================================================ 日志解析与切片

def parse_exec_lines(lines):
    """整份 sim 日志 → exec 行（ts/位姿/档位/位姿龄）+ 解析失败行。"""
    out, bad = [], []
    for ln in lines:
        if EXEC_TAG not in ln:
            continue
        m = EXEC_RE.match(ln)
        if not m:
            bad.append(ln.rstrip("\n"))
            continue
        ts = Q.parse_log_ts(ln)
        if ts is None:
            bad.append(ln.rstrip("\n"))
            continue
        out.append({
            "ts": ts, "line": ln.rstrip("\n"),
            "x": float(m.group("x")), "y": float(m.group("y")),
            "yaw": float(m.group("yaw")), "speed": int(m.group("speed")),
            "turn": int(m.group("turn")), "age": int(m.group("age")),
        })
    return out, bad


def parse_obstacles(world_gd=WORLD_SRC / "src" / "world" / "World.gd"):
    """运行时解析 `World.gd::OBSTACLES`（15 条 footprint），不手抄坐标。"""
    rects = []
    for m in OBSTACLE_RE.finditer(read_text(world_gd)):
        ox, oy = float(m.group("ox")), float(m.group("oy"))
        sx, sy = float(m.group("sx")), float(m.group("sy"))
        rects.append({"name": m.group("name"), "x0": ox, "y0": oy, "x1": ox + sx, "y1": oy + sy})
    return rects


def clear_of(p, rects):
    best = (float("inf"), "")
    for r in rects:
        dx = max(r["x0"] - p[0], 0.0, p[0] - r["x1"])
        dy = max(r["y0"] - p[1], 0.0, p[1] - r["y1"])
        d = math.hypot(dx, dy)
        if d < best[0]:
            best = (d, r["name"])
    return best


def baseline_metrics(path=BASELINE_LOG):
    """改前基线（§八 #8）：`queue_ugv_dev/sim_1.log`，`动作=Stop` 当"停"。同一套公式。"""
    rows = []
    for ln in Q.read_log_lines(path):
        if "[Sim] 位置=(" not in ln:
            continue
        ts = Q.parse_log_ts(ln)
        if ts is None:
            continue
        m = BASE_ACTION_RE.search(ln)
        rows.append((ts, (m.group("act") == "Stop") if m else False))
    rows.sort()
    n = len(rows)
    if n == 0:
        return {"lines": 0, "stop_lines": 0, "line_ratio": 0.0, "time_ratio": 0.0, "span_s": 0.0}
    stop_n = sum(1 for _, s in rows if s)
    total = stop_t = 0.0
    for i, (ts, s) in enumerate(rows):
        dt = (rows[i + 1][0] - ts) if i + 1 < n else SAMPLE_DT
        total += dt
        if s:
            stop_t += dt
    return {"lines": n, "stop_lines": stop_n, "line_ratio": stop_n / n,
            "time_ratio": (stop_t / total) if total > 0 else 0.0,
            "span_s": rows[-1][0] - rows[0][0]}


def ratio_metrics(execs):
    """同一套公式：行占比 + 时间加权占比（`(0,0)` 算"停"）。"""
    n = len(execs)
    if n == 0:
        return {"lines": 0, "stop_lines": 0, "line_ratio": 0.0, "time_ratio": 0.0, "span_s": 0.0}
    stop = [e for e in execs if e["speed"] == 0 and e["turn"] == 0]
    total = stop_t = 0.0
    for i, e in enumerate(execs):
        dt = (execs[i + 1]["ts"] - e["ts"]) if i + 1 < n else SAMPLE_DT
        total += dt
        if e["speed"] == 0 and e["turn"] == 0:
            stop_t += dt
    return {"lines": n, "stop_lines": len(stop), "line_ratio": len(stop) / n,
            "time_ratio": (stop_t / total) if total > 0 else 0.0,
            "span_s": execs[-1]["ts"] - execs[0]["ts"]}


def slice_execs(execs, ts0, ts1):
    return [e for e in execs if ts0 <= e["ts"] <= ts1]


# ================================================================ 判据 C1~C8

def lines_in(lines_ts, ts0, ts1):
    """按日志 µs 时间戳切片（`lines_ts` = [(ts, line)]，一次解析、处处复用）。"""
    return [ln for ts, ln in lines_ts if ts0 <= ts <= ts1]


def evaluate(ctx):
    """返回 8 条判据（判据 6 只记录）+ 供 verdict 用的关键数字。"""
    execs = ctx["execs"]
    lines = ctx["lines"]
    lts = ctx["lines_ts"]
    off = ctx["log_offset_s"]
    only = ctx["only"]
    windows = [w for w in ctx["windows"] if w["t1"] is not None]
    mission = [w for w in windows if w["kind"] == "mission"]
    c = []

    def wts(w):
        """窗口 → 日志时间轴（含 settle 尾窗，判据 5(2) 需要）。"""
        return (w["t0"] + off, w["t1"] + off)

    def wts_dry(w):
        """窗口的**任务期**（到 drain 为止）：判据 1/2/3/6 一律不含 settle 尾巴，
        否则「到达后停 1 s」会被算成抖动/停止时间。"""
        return (w["t0"] + off, (w["t_drain"] or w["t1"]) + off)

    # ── C1 到达 ─────────────────────────────────────────────
    seg_total = len(mission)
    arrived, done_cnt, fail_cnt, worst_err, worst_where = 0, 0, 0, 0.0, ""
    miss = []
    for w in mission:
        ts0, ts1 = wts_dry(w)
        seg_lines = lines_in(lts, ts0, ts1)
        hit_arr = any(p in ln for ln in seg_lines for p in ARRIVE_PATS)
        hit_done = sum(1 for ln in seg_lines if DONE_PAT in ln)
        hit_fail = any(FAIL_PAT in ln and FAIL_SUFFIX in ln for ln in seg_lines)
        dr = w["drain_row"] or {}
        tgt = w["target"] or (0.0, 0.0)
        err = max(abs(float(dr.get("x", 0.0)) - tgt[0]), abs(float(dr.get("y", 0.0)) - tgt[1]))
        if err > worst_err:
            worst_err, worst_where = err, "%s.%d" % (w["case"], w["seg"])
        ok = hit_arr and hit_done >= 1 and (not hit_fail) and err <= ARRIVE_TOL_M and not w["aborted"]
        if ok:
            arrived += 1
        else:
            miss.append("%s.%d" % (w["case"], w["seg"]))
        done_cnt += 1 if hit_done else 0
        fail_cnt += 1 if hit_fail else 0
    c.append(dict(n=1, name="到达",
                  detail="%d/%d 段 nav Arrived + `[Robot] 任务完成`；末位逐轴最大 %.2f m @ %s（≤%.2f）；"
                         "任务完成 %d / 失败 %d%s"
                         % (arrived, seg_total, worst_err, worst_where or "-", ARRIVE_TOL_M,
                            done_cnt, fail_cnt, ("；未过 " + ",".join(miss)) if miss else ""),
                  ok=(seg_total > 0 and arrived == seg_total and fail_cnt == 0)))

    # ── C2 不抖（M1 / M4）──────────────────────────────────
    flips_txt, flips_ok = [], True
    for name in ("M1", "M4"):
        ws = [w for w in mission if w["case"] == name]
        if not ws:
            if only and name not in only:
                flips_txt.append("%s 未跑(--only)" % name)
                continue
            flips_txt.append("%s 未跑" % name)
            flips_ok = False
            continue
        sub = slice_execs(execs, wts_dry(ws[0])[0], wts_dry(ws[-1])[1])
        nz = [e["turn"] for e in sub if e["turn"] != 0]
        flips = sum(1 for a, b in zip(nz, nz[1:]) if a * b < 0)
        ok = flips <= TURN_FLIP_MAX[name]
        flips_ok = flips_ok and ok
        flips_txt.append("%s %d（≤%d）" % (name, flips, TURN_FLIP_MAX[name]))
        ctx["flips"][name] = {"flips": flips, "nonzero": len(nz), "ok": ok}
    c.append(dict(n=2, name="不抖", detail="turn 反号 " + "／".join(flips_txt), ok=flips_ok))

    # ── C3 不打转（M1 / M4）────────────────────────────────
    spin_txt, spin_ok = [], True
    for name in ("M1", "M4"):
        ws = [w for w in mission if w["case"] == name]
        if not ws:
            if only and name not in only:
                spin_txt.append("%s 未跑(--only)" % name)
                continue
            spin_txt.append("%s 未跑" % name)
            spin_ok = False
            continue
        sub = slice_execs(execs, wts_dry(ws[0])[0], wts_dry(ws[-1])[1])
        i0 = next((i for i, e in enumerate(sub) if e["speed"] > 0 and e["turn"] == 0), None)
        if i0 is None or len(sub) - i0 < 2:
            spin_txt.append("%s 无直行段" % name)
            spin_ok = False
            continue
        base = sub[i0]
        span = sub[-1]["ts"] - base["ts"]
        nets = []
        t = 1.0
        while t + NO_SPIN_WIN_S <= span + 1e-6:
            lo, hi = base["ts"] + t - 1.0, base["ts"] + t - 1.0 + NO_SPIN_WIN_S
            win = [e for e in sub if lo <= e["ts"] <= hi]
            if len(win) >= 2:
                nets.append(math.hypot(win[-1]["x"] - win[0]["x"], win[-1]["y"] - win[0]["y"]))
            t += 1.0
        if not nets:
            spin_txt.append("%s 不足 %ds 直行" % (name, int(NO_SPIN_WIN_S)))
            spin_ok = False
            continue
        mn = min(nets)
        ok = mn >= NO_SPIN_MIN_M
        spin_ok = spin_ok and ok
        spin_txt.append("%s 最小 %.2f m／%d 窗（≥%.0f）" % (name, mn, len(nets), NO_SPIN_MIN_M))
        ctx["spin"][name] = {"min_net_m": round(mn, 3), "windows": len(nets), "ok": ok}
    c.append(dict(n=3, name="不打转", detail="10 s 净位移 " + "／".join(spin_txt), ok=spin_ok))

    # ── C4 不撞墙 ──────────────────────────────────────────
    estop = sum(1 for ln in lines if ESTOP_PAT in ln)
    rects = ctx["rects"]
    min_clear, min_at, min_case = float("inf"), None, ""
    for r in ctx["rows"]:
        d, nm = clear_of((r["x"], r["y"]), rects)
        if d < min_clear:
            min_clear, min_at, min_case = d, (r["x"], r["y"], nm), r["case"]
    c4ok = (estop == 0) and (min_clear >= MIN_CLEAR_M)
    c.append(dict(n=4, name="不撞墙",
                  detail="急停 %d 次（4a）；几何净空 min %.2f m @ %s(%s, %.2f,%.2f)（≥%.1f）"
                         % (estop, min_clear if min_at else -1.0, min_case,
                            min_at[2] if min_at else "-", min_at[0], min_at[1], MIN_CLEAR_M)
                         if min_at else "无轨迹",
                  ok=c4ok))
    ctx["estop"] = estop
    ctx["min_clear"] = (min_clear, min_at, min_case)

    # ── C5 该停会停 ────────────────────────────────────────
    m0ws = [w for w in windows if w["case"] == "M0" and w["kind"] == "idle"]
    m0_ok, m0_txt = False, "M0 未跑"
    if m0ws:
        sub = slice_execs(execs, *wts(m0ws[0]))
        allzero = bool(sub) and all(e["speed"] == 0 and e["turn"] == 0 for e in sub)
        rws = [r for r in ctx["rows"] if r["case"] == "M0"]
        move = math.hypot(rws[-1]["x"] - rws[0]["x"], rws[-1]["y"] - rws[0]["y"]) if len(rws) > 1 else 0.0
        m0_ok = allzero and move <= 0.01
        m0_txt = "M0 %d 行全 (0,0)、位移 %.3f m" % (len(sub), move)
        ctx["m0"] = {"lines": len(sub), "move_m": round(move, 3), "all_zero": allzero}
    lat_max, lat_where, lat_ok, lat_cnt = 0.0, "", True, 0
    for w in mission:
        ts0, td_log = wts_dry(w)
        full = slice_execs(execs, ts0, w["t1"] + off)   # 含 settle 尾窗：首个 (0,0) 通常落在 drain 之后
        mission_sub = slice_execs(execs, ts0, td_log)
        if not full:
            lat_ok = False
            lat_where = lat_where or "%s.%d 无 exec 行" % (w["case"], w["seg"])
            continue
        nz = [i for i, e in enumerate(mission_sub) if e["speed"] != 0 or e["turn"] != 0]
        if not nz:
            continue
        t_last = mission_sub[nz[-1]]["ts"]
        nxt = next((e for e in full
                    if e["ts"] > t_last and e["speed"] == 0 and e["turn"] == 0), None)
        if nxt is None:
            lat_ok = False
            lat_where = lat_where or "%s.%d 之后无 (0,0)" % (w["case"], w["seg"])
            continue
        lat = nxt["ts"] - t_last
        lat_cnt += 1
        if lat > lat_max:
            lat_max, lat_where = lat, "%s.%d" % (w["case"], w["seg"])
        if lat > STOP_LAT_S:
            lat_ok = False
        # 任务结束后 1 s 内全是 (0,0)
        after = [e for e in full if (w["t_drain"] + off) <= e["ts"] <= (w["t_drain"] + off) + STOP_LAT_S]
        bad = [e for e in after if e["speed"] != 0 or e["turn"] != 0]
        if bad or not after:
            lat_ok = False
            lat_where = lat_where or "%s.%d 任务后 1 s 非 (0,0)/无行" % (w["case"], w["seg"])
    c.append(dict(n=5, name="该停会停",
                  detail="%s；任务后首个 (0,0) 延迟 max %.2f s @ %s（≤%.1f，%d 段）"
                         % (m0_txt, lat_max, lat_where or "-", STOP_LAT_S, lat_cnt),
                  ok=(m0_ok and lat_ok)))
    ctx["stop_lat"] = {"cells": lat_cnt, "max_s": round(lat_max, 3)}

    # ── C6 停走对比（**只记录**）───────────────────────────
    per_case = {}
    for case in CASE_ORDER:
        ws = [w for w in windows if w["case"] == case and w["kind"] in ("mission", "idle")]
        if not ws:
            continue
        if ws[0]["kind"] == "idle":
            sub = slice_execs(execs, *wts(ws[0]))
        else:
            sub = slice_execs(execs, wts_dry(ws[0])[0], wts_dry(ws[-1])[1])
        per_case[case] = ratio_metrics(sub)
    base = ctx["baseline"]
    mission_sub = slice_execs(execs, wts_dry(mission[0])[0], wts_dry(mission[-1])[1]) if mission else []
    agg = ratio_metrics(mission_sub)
    ctx["ratio"] = {"per_case": per_case, "agg": agg, "baseline": base}
    c.append(dict(n=6, name="停走对比（记录）",
                  detail="任务期 (0,0) 时间占比 %.1f%%／行占比 %.1f%%（基线 时间 %.1f%%／行 %.1f%%；"
                         "离散日志事件驱动、连续日志 5 Hz ⇒ 只有时间加权同口径）"
                         % (agg["time_ratio"] * 100, agg["line_ratio"] * 100,
                            base["time_ratio"] * 100, base["line_ratio"] * 100),
                  ok=True, record=True))

    # ── C7 日志可解析 ──────────────────────────────────────
    rows_ugv = sum(1 for ln in lines if EXEC_TAG in ln)
    bad = ctx["exec_bad"]
    c7ok = rows_ugv > 0 and len(bad) == 0
    c.append(dict(n=7, name="日志可解析",
                  detail="`[Sim] UGV` %d 行 / 解析 %d（失败 %d）%s"
                         % (rows_ugv, rows_ugv - len(bad), len(bad),
                            ("；首条失败：" + bad[0][:110]) if bad else ""),
                  ok=c7ok))

    # ── C8 无回归 ──────────────────────────────────────────
    blocked = sum(1 for ln in lines if BLOCKED_PAT in ln)
    fails = sum(1 for ln in lines if FAIL_PAT in ln and FAIL_SUFFIX in ln)
    c8ok = blocked == 0 and estop == 0 and fails == 0
    c.append(dict(n=8, name="无回归",
                  detail="`拍无路` %d / 急停 %d / 任务失败 %d（全为 0 才 PASS）"
                         % (blocked, estop, fails), ok=c8ok))
    ctx["blocked"] = blocked
    ctx["fails"] = fails
    return c


# ================================================================ 前置检查

def git_boundary():
    """F6 的等价物：与进场基线比，`Task/`/`.github/`/`Cargo.*` 条目数不变。"""
    rc, so, _ = run_capture(["git", "status", "--porcelain", "-uall"], timeout=30)
    if rc != 0:
        return False, "git status 失败"
    now = [ln for ln in so.splitlines() if any(p in ln for p in FROZEN_PATHS)]

    def norm(ls):
        return sorted(ln.strip() for ln in ls if ln.strip())

    if GIT_BASELINE.is_file():
        base = [ln for ln in read_text(GIT_BASELINE).splitlines()
                if any(p in ln for p in FROZEN_PATHS)]
        if norm(base) == norm(now):
            return True, "与进场基线一致（%d 条，未动这几个路径）" % len(now)
        return False, "diff: 基线 %d 条 → 现在 %d 条" % (len(base), len(now))
    return True, "无基线文件，跳过（%d 条既有条目）" % len(now)


def preflight(port, out=None):
    """前置检查 #1~#9（#10~#12 只能起进程后验，在 `run()` 里）。"""
    checks = []

    def add(n, name, ok, detail):
        checks.append({"n": n, "name": name, "ok": bool(ok), "detail": detail})
        Log("  [%2d] %-44s %s  %s" % (n, name, "OK  " if ok else "FAIL", detail))

    ok, d = git_boundary()
    add(1, "边界：Task/.github/Cargo.* 无新条目", ok, d)

    b1 = Q.port_busy(port)
    b2 = Q.port_busy(WORLD_PORT)
    add(2, "端口 %d / %d 空闲" % (WORLD_PORT, port), (not b1) and (not b2),
        "busy: %s" % ([p for p, b in ((WORLD_PORT, b2), (port, b1)) if b]))

    res = residual_all()
    add(3, "无残留 godot/pleiades-sim/Xvfb", not res, "%d 条%s" % (len(res), ("：" + res[0][:80]) if res else ""))

    miss = [t for t in ("xvfb-run", "xauth", "Xvfb", "godot") if shutil.which(t) is None]
    rc, so, _ = run_capture(["godot", "--version"], timeout=60)
    add(4, "xvfb-run/xauth/Xvfb/godot 可用", (not miss) and rc == 0,
        ("缺 %s；" % miss if miss else "") + "godot %s" % so.strip()[:40])

    bt = TARGET_BIN.stat().st_mtime if TARGET_BIN.exists() else 0.0
    newest = binary_sources_newest()
    add(5, "二进制比 pleiades-sim/src 新", bt > newest[0],
        "bin=%s src=%s" % (time.strftime("%m-%d %H:%M", time.localtime(bt)) if bt else "缺失",
                           os.path.basename(newest[1]) + " " + time.strftime("%m-%d %H:%M", time.localtime(newest[0]))
                           if newest[0] else "-"))

    same, d = ensure_binary()
    add(6, "实例二进制 == target", same, d)

    cfg = write_config(port, out)
    ok, bad = config_readback(cfg, port)
    add(7, "config 回读 §八 值（turn_speed=30）", ok, "; ".join(bad) if bad else
        "forward_speed=30 turn_speed=30 port=%d chassis=diff_drive_ugv" % port)

    ent = ensure_world_fast()
    stamp_ok = world_marker_path().is_file() and \
        world_marker_path().read_text(encoding="utf-8", errors="replace").strip() == \
        world_stamp(WORLD_SRC).strip()
    rc2, extra = world_diff_extra()
    add(8, "world_fast 副本新鲜（diff 只 override.cfg）", stamp_ok and not extra,
        "%s；diff %s" % (ent["reason"], "干净" if not extra else extra[:2]))

    ov = read_text(WORLD_FAST / "override.cfg")
    add(9, "override.cfg == msaa_3d=0", ov == OVERRIDE_TEXT, repr(ov.replace("\n", "⏎")))
    return checks


# ================================================================ verdict / cases.json

def write_verdict(out, ctx, checks, residual, ports_busy, xlocks, ctl_port):
    cr = ctx["criteria"]
    by = {x["n"]: x for x in cr}
    fp = ctx["build"]
    env = ctx["env"]
    cases_txt = "｜".join(
        "%s %s %s" % (c["name"], ctx["case_desc"].get(c["name"], "-"),
                      ("%.0f s" % ctx["case_time"][c["name"]]) if c["name"] in ctx["case_time"] else "MISSING")
        for c in CASES if ctx["only"] is None or c["name"] in ctx["only"])
    L = []
    L.append("# task_40 Step 2 — 小车运动验证（只验不改）")
    L.append("- UTC %s｜控制口 %d｜世界 `RobotSim_TestEnv/world_fast`(msaa_3d=0, xvfb-run)｜%d 用例 8 判据%s"
             % (ctx["utc"], ctl_port, len(ctx["ran"]), "（`--only` 局部）" if ctx["only"] else ""))
    L.append("- 二进制 sha256[:12]=%s（=target %s）｜`forward_speed=30 turn_speed=30`（§八 #1）｜"
             "复用 `sim_queue_test.py` sha256[:12]=%s｜日志钟偏移 %+.0f ms（D3）"
             % (str(fp["binary_sha256"])[:12], fp["binary_same"], str(fp["reuse_script_sha256"])[:12],
                ctx["log_offset_s"] * 1000))
    L.append("- 环境闸门：位姿龄 中位/最大 = %d/%d ms（≤%d %s）｜执行器：`[Sim] UGV` %d 行 / 旧离散 "
             "`[Sim] 位置=(` %d 行"
             % (env["age_median"], env["age_max"], POSE_AGE_GATE_MS,
                "✓" if env["ok"] else "✗ FAIL(env)", env["ugv_lines"], env["discrete_lines"]))
    L.append("| # | 判据 | 实测 | 结论 |")
    L.append("|---|---|---|---|")
    for n in range(1, 9):
        x = by.get(n)
        if x is None:
            L.append("| %d | - | 未计算 | FAIL(env) |" % n)
            continue
        tag = "记录" if x.get("record") else ("PASS" if x["ok"] else "FAIL")
        L.append("| %d | %s | %s | %s |" % (n, x["name"], x["detail"], tag))
    L.append("- 用例：%s" % cases_txt)
    L.append("- 偏差：D1 只复用 `sim_queue_test.py`（显示走 `xvfb-run -a`）｜D2 M5 用 3.5 m 门洞（世界无 "
             "1 格走廊）｜D3 时间轴用日志 µs｜D4 判据 4 = 急停 0 + 几何净空｜D5 前置 #1 与进场基线比对")
    ok_all = all(x["ok"] for x in cr if not x.get("record")) and len(cr) == 8 and env["ok"]
    L.append("- 结论：%s（%s）"
             % ("8 条判据全 PASS" if ok_all else "有判据未过，见上表（只验不改，不调阈值）", out.name))
    L.append("- 收尾：sim/world 已停｜9999/%d 空闲=%s｜残留进程 %d｜`/tmp/.X*-lock` %d"
             % (ctl_port, not ports_busy, len(residual), xlocks))
    (out / "verdict.md").write_text("\n".join(L) + "\n", encoding="utf-8")
    return ok_all


def write_cases_json(out, ctx):
    data = {
        "task": "task_40 Step 2",
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ctx["t_start"])),
        "utc_dir": out.name,
        "only": sorted(ctx["only"]) if ctx["only"] else None,
        "build": ctx["build"],
        "env": ctx["env"],
        "log_offset_ms": round(ctx["log_offset_s"] * 1000, 2),
        "cases": [{"case": c["name"], "title": c["title"], "desc": c["desc"],
                   "segments": len(c["segs"]),
                   "elapsed_s": ctx["case_time"].get(c["name"]),
                   "abort": ctx["case_abort"].get(c["name"])} for c in CASES],
        "windows": [{k: v for k, v in w.items() if k != "drain_row"} for w in ctx["windows"]],
        "criteria": ctx["criteria"],
        "detail": {
            "flips": ctx["flips"], "spin": ctx["spin"], "m0": ctx.get("m0"),
            "stop_latency": ctx["stop_lat"], "ratio": ctx["ratio"],
            "min_clearance": {"m": ctx["min_clear"][0], "at": ctx["min_clear"][1],
                              "case": ctx["min_clear"][2]},
            "blocked_lines": ctx["blocked"], "mission_fail_lines": ctx["fails"],
            "estop_lines": ctx["estop"],
        },
        "preflight": ctx["preflight"],
        "residual_procs": ctx["residual"],
        "ports_busy": ctx["ports_busy"],
    }
    (out / "cases.json").write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")


# ================================================================ main

def pick_out(args):
    if args.out_dir:
        return pathlib.Path(args.out_dir).resolve()
    return (RESULTS / ("task40_" + utc_stamp())).resolve()


def main():
    ap = argparse.ArgumentParser(description="Task 40 Step 2 — 小车连续执行器运动验证（只验不改）")
    ap.add_argument("--out-dir", default=None)
    ap.add_argument("--port", type=int, default=DEFAULT_PORT)
    ap.add_argument("--dry-run", action="store_true", help="只跑前置检查（#1~#9），不起任何进程")
    ap.add_argument("--only", default=None, help="只跑这些用例，如 M0,M1,M4")
    ap.add_argument("--no-launch", action="store_true", help="连已在跑的控制口（不起世界/sim）")
    ap.add_argument("--no-json", dest="json", action="store_false", default=True)
    args = ap.parse_args()

    only = None
    if args.only:
        only = {x.strip().upper() for x in args.only.split(",") if x.strip()}
        bad = [x for x in only if x not in CASE_BY_NAME]
        if bad:
            Log("[ARGS FAIL] 未知用例 %s（可选 %s）" % (bad, CASE_ORDER))
            return 2

    out = pick_out(args)
    out.mkdir(parents=True, exist_ok=True)
    INST.mkdir(parents=True, exist_ok=True)
    Log("=" * 86)
    Log("task_40 Step 2 — 小车运动验证（只验不改）  产物 → %s" % out)
    Log("=" * 86)

    Log("\n── 前置检查 ─────────────────────────────────────────")
    checks = preflight(args.port, out)
    bad = [c for c in checks if not c["ok"]]
    if args.dry_run:
        Log("\nPREFLIGHT %s（%d/%d 通过；#10~#12 需起进程，dry-run 跳过）"
            % ("OK" if not bad and not args.no_launch else ("FAIL" if bad else "OK"), len(checks) - len(bad),
               len(checks)))
        for c in bad:
            Log("  [FAIL] #%d %s：%s" % (c["n"], c["name"], c["detail"]))
        return 0 if not bad else 2
    if bad:
        for c in bad:
            Log("[PREFLIGHT FAIL] #%d %s：%s" % (c["n"], c["name"], c["detail"]))
        return 2

    ctx = {
        "t_start": time.time(), "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "only": only, "ran": [], "windows": [], "rows": [], "case_time": {}, "case_abort": {},
        "flips": {}, "spin": {}, "stop_lat": {}, "ratio": {}, "blocked": 0, "fails": 0, "estop": 0,
        "criteria": [], "residual": [], "ports_busy": {}, "log_offset_s": 0.0,
        "build": {}, "env": {}, "case_desc": {c["name"]: c["desc"] for c in CASES},
    }
    world = Q.Proc("world")
    sim = Q.Proc("sim")
    ctl = Q.Control(args.port)
    world_log = out / "world.log"
    sim_stdout = out / "sim_stdout.log"
    os.environ["LIBGL_ALWAYS_SOFTWARE"] = "1"     # llvmpipe 软渲染（子进程继承）
    tail = Q.LogTail(INST / "Log")
    exit_code = 1
    fatal = None
    sampler = Sampler(ctl)
    runner = Runner(sampler, ctl, args.port)

    try:
        if not args.no_launch:
            argv = world_argv()
            Log("\n── 起世界（world_fast, xvfb-run -a）──\n  argv=%s" % " ".join(argv))
            world.start(argv, ROOT, world_log)
            dt = wait_world_listen(world_log, 150.0)
            if dt is None:
                Log("[PREFLIGHT FAIL] 世界 150 s 内没有监听 :%d（见 %s）" % (WORLD_PORT, world_log))
                return 2
            checks.append({"n": 10, "name": "世界 150 s 内监听 9999", "ok": True,
                           "detail": "%.1f s" % dt})
            Log("  [10] 世界监听 :%d（%.1f s）" % (WORLD_PORT, dt))

            sim.start([str(INST / "pleiades-sim"), "--headless"], INST, sim_stdout)
            Log("  等待控制口 %d 可连 …" % args.port)
        if not ctl.connect(retries=240 if not args.no_launch else 20, delay=0.25):
            Log("[PREFLIGHT FAIL] 控制口 %d 连不上" % args.port)
            return 2

        t0 = time.time()
        while time.time() - t0 < 120.0:
            if ctl.status().get("assembled"):
                break
            time.sleep(0.25)
        else:
            Log("[PREFLIGHT FAIL] 120 s 内未 assembled（世界没绑定/世界没起）")
            return 2
        checks.append({"n": 11, "name": "sim 120 s 内 assembled", "ok": True,
                       "detail": "%.1f s" % (time.time() - t0)})
        Log("  [11] 已装配（%.1f s）" % (time.time() - t0))

        st = ctl.status()
        spawn_ok = (abs(float(st["x"]) - 64.0) <= 0.3 and abs(float(st["y"]) - 64.0) <= 0.3
                    and abs(float(st["yaw"])) <= 0.1)
        checks.append({"n": 12, "name": "出生位姿 (64±0.3, 64±0.3) 且 |yaw|≤0.1", "ok": spawn_ok,
                       "detail": "(%.2f,%.2f) yaw=%.3f avatar=%s"
                                 % (st["x"], st["y"], st["yaw"], st.get("avatar_id"))})
        Log("  [12] 出生位姿 (%0.2f,%.2f) yaw=%.3f → %s"
            % (st["x"], st["y"], st["yaw"], "OK" if spawn_ok else "FAIL"))
        if not spawn_ok:
            Log("[PREFLIGHT FAIL] 出生位姿不是 (64,64)/yaw=0（残留 avatar？）")
            return 2

        # mode=auto 幂等（顺带 clean start）
        runner.cmd({"cmd": "mode", "value": "auto"}, "mode")
        tail.reset()

        # 日志钟偏移探针（D3）：控制口收命令即打点，Δ ≈ WS 往返（<10 ms）
        mark = tail.mark()
        t_before = time.time()
        ctl.send_cmd({"cmd": "queue", "missions": []})
        time.sleep(0.30)
        probe = [ln for ln in tail.since(mark) if "[control] queue（整体替换）" in ln]
        if probe:
            ts = Q.parse_log_ts(probe[-1])
            if ts:
                ctx["log_offset_s"] = ts - t_before
        Log("  日志钟偏移 = %+.0f ms（`[control] queue` 探针，%d 行命中）"
            % (ctx["log_offset_s"] * 1000, len(probe)))

        build = Q.build_fingerprint(INST / "pleiades-sim")
        inst_sha = Q.sha256_file(INST / "pleiades-sim")
        tgt_sha = Q.sha256_file(TARGET_BIN)
        build["binary_sha256"] = inst_sha
        build["binary_same"] = "一致" if inst_sha == tgt_sha else "不一致"
        build["reuse_script_sha256"] = Q.sha256_file(ROOT / "Tool" / "sim_queue_test.py")
        ctx["build"] = build
        Log("  构建指纹：binary=%s（%s）script=%s"
            % (inst_sha[:12], build["binary_same"], build["reuse_script_sha256"][:12]))

        # ── 用例 ────────────────────────────────────────────
        Log("\n── 用例（%s）──" % (",".join(sorted(only)) if only else "全部 6 个"))
        first_mission_done = False
        for case in CASES:
            if only and case["name"] not in only:
                continue
            Log("\n[%s] %s（%s）" % (case["name"], case["title"], case["desc"]))
            runner.run_case(case, tail, out)
            ctx["ran"].append(case["name"])
            if (case["segs"] and not first_mission_done):
                first_mission_done = True
                live = runner.liveness_gate(tail)
                if not live:
                    Log("[PREFLIGHT FAIL] 跑的不是连续执行器（二进制/装配不对）")
                    fatal = "活体闸门未过"
                    return 2

        # ── 判据 ────────────────────────────────────────────
        Log("\n── 判据 ────────────────────────────────────────────")
        lines, _ = tail.all_lines()
        execs, bad = parse_exec_lines(lines)
        ctx["lines"], ctx["execs"], ctx["exec_bad"] = lines, execs, bad
        ctx["lines_ts"] = [(ts, ln) for ts, ln in
                           ((Q.parse_log_ts(ln), ln) for ln in lines) if ts is not None]
        ctx["rows"] = sampler.rows
        ctx["windows"] = sampler.windows
        ctx["rects"] = parse_obstacles()
        ctx["baseline"] = baseline_metrics()
        ages = sorted(e["age"] for e in execs)
        ctx["env"] = {
            "age_median": ages[len(ages) // 2] if ages else -1,
            "age_max": ages[-1] if ages else -1,
            "ok": bool(ages) and ages[len(ages) // 2] <= POSE_AGE_GATE_MS,
            "ugv_lines": sum(1 for ln in lines if EXEC_TAG in ln),
            "discrete_lines": sum(1 for ln in lines if DISCRETE_RE.search(ln)),
            "gate": dict(runner.liveness),
            "exec_parsed": len(execs), "exec_bad": len(bad),
            "clone_obstacles": len(ctx["rects"]),
        }
        Log("  环境闸门：位姿龄 中位/最大 = %d/%d ms（≤%d）%s"
            % (ctx["env"]["age_median"], ctx["env"]["age_max"], POSE_AGE_GATE_MS,
               "OK" if ctx["env"]["ok"] else "FAIL(env)"))
        Log("  执行器：`[Sim] UGV` %d 行 / 解析 %d / 失败 %d；旧离散 %d 行；障碍表 %d 条"
            % (ctx["env"]["ugv_lines"], ctx["env"]["exec_parsed"], ctx["env"]["exec_bad"],
               ctx["env"]["discrete_lines"], len(ctx["rects"])))
        ctx["case_time"] = runner.case_time
        ctx["case_abort"] = runner.case_abort
        ctx["preflight"] = checks
        ctx["criteria"] = evaluate(ctx)
        for x in ctx["criteria"]:
            Log("  [%d] %-12s %s  %s" % (x["n"], x["name"],
                                         "记录" if x.get("record") else ("PASS" if x["ok"] else "FAIL"),
                                         x["detail"]))
        exit_code = 0 if all(x["ok"] for x in ctx["criteria"] if not x.get("record")) and \
            len(ctx["criteria"]) == 8 and ctx["env"]["ok"] else 1
    except SystemExit:
        raise
    except Exception:
        traceback.print_exc()
        fatal = "运行期异常（见上方 traceback）"
        exit_code = 2
    finally:
        # ── 收尾（顺序固定，§3.11）────────────────────────────
        Log("\n── 收尾 ───────────────────────────────────────────")
        try:
            ctl.close()
        except Exception:
            pass
        sim.stop()
        world.stop()
        sim_log = tail.newest()
        if sim_log is not None and sim_log.exists():
            shutil.copy2(sim_log, out / "sim.log")
            Log("  sim 日志 → %s（%s）" % (out / "sim.log", time.strftime("%H:%M:%S")))
        residual, zombie_n = wait_clean()
        ports = {"world": Q.port_busy(WORLD_PORT), "control": Q.port_busy(args.port)}
        xlocks = sorted(str(p) for p in pathlib.Path("/tmp").glob(".X*-lock"))
        Log("  pgrep 残留：%d 条活进程（另有 %d 个 <defunct>，父退出即回收）%s"
            % (len(residual), zombie_n, ("：" + residual[0][:100]) if residual else ""))
        Log("  端口占用：9999=%s %d=%s" % (ports["world"], args.port, ports["control"]))
        Log("  /tmp/.X*-lock：%s" % (xlocks or "无"))
        ctx["residual"] = residual
        ctx["ports_busy"] = ports
        if ctx["criteria"]:
            try:
                ok_all = write_verdict(out, ctx, checks, residual,
                                       ports["world"] or ports["control"], len(xlocks), args.port)
                if args.json:
                    write_cases_json(out, ctx)
                write_trace(out, sampler.rows)
                Log("  产物：verdict.md / trace.csv（%d 行）/ cases.json / sim.log / world.log"
                    % len(sampler.rows))
                if fatal is None:
                    exit_code = 0 if ok_all and not residual and not (ports["world"] or ports["control"]) else 1
            except Exception:
                traceback.print_exc()
                exit_code = 2
        else:
            Log("  [无判据] 本轮没有产出 verdict（前置/运行失败）；产物只有原始日志")

    Log("\nEXIT=%d" % exit_code)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
