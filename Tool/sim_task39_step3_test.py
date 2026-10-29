#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-29
# Modified Date ： 2026-10-29
"""task_39 Step 3 — 验证（常规移动 + 跟随）。**只验不改**。

跑法：python3 -B Tool/sim_task39_step3_test.py [--preflight] [--probe-env]
                                            [--no-launch] [--reuse-world]
                                            [--out-dir DIR] [--port 9098]
                                            [--world-dir DIR]
                                            [--cases f1,f2,f3,m1,m2,m3]
                                            [--regrade DIR]
产物：RobotSim_TestEnv/results/task39_<UTC>/{
        trace.csv, detector_frames.csv, ped_trace.csv, env_probe.csv, script.log,
        sim.log, sim_trace.log, world.log, env.json, verdict.json, verdict.md, shots/*.png}
      verdict.md 结构：§0 环境与口径 → §1 前置自检（21 条）+ §1.1 任务级遗留门 → §2 时序 →
      §2.1 归位与目视几何 → §3 判据（28 行 = 计划 27 + M2 的 L9b 子项，带实测值/判定/口径）→
      §4 实测数据 → §5 观察项/偏差 → §6 结论（含 §6.4/§6.5 遗留登记）→ §7 产物清单 →
      §8 残留检查 → §9 复现命令 → §10 收尾复检；
      `--regrade` 模式再加 §11 与取证轮的差异 / §12 `prev_issues` 逐条处置 / §13 复算方法与等价性
退出码：0 全 PASS；1 有 FAIL（即使同时有 FAIL(env)）；2 仅 FAIL(env) / 前置不过 / 脚本异常

第 3 轮补充：`--regrade DIR` 只读已有 results 产物、重建 ctx 后**复算** 28 条判据
（不启动世界/仿真，源目录零写入）；判据状态机新增 `N-A（真空）`（前置不成立时的恒真断言
不计通过）。

第 2 轮补充：世界用 `RobotSim_TestEnv/world_fast/`（源世界全量副本 + 唯一 `override.cfg`
关掉 4× MSAA）。不碰 `orion-simulation-world/`；副本由 `ensure_world_fast()` 幂等创建。

本脚本**只验收**，不改任何 Rust / GDScript / Lua / config.toml，不改
`Tool/sim_follow_test.py`（sha256 钉死在 REUSE_SHA256），不调 `conf_thres` / `nms` /
`d_far` / `d_near` / `stale` / 任何判据阈值，不 `git add` / `git commit`，不 `cargo fmt`。

基础设施（世界/sim 起停、控制口 WS、行人客户端、detector 日志解析、截图）**全部 import**
自已提交的 `Tool/sim_follow_test.py`（一个字节都不改）；本文件只写
「16 条前置 + S0 锚点 + 3 个常规移动用例 + 3 个跟随用例（F4 聚合）+ 27 条判据 + verdict」。

口径（verdict.md §0 是本口径的自包含定义处）：
  * 本机 llvmpipe 软渲染 → 物理推进 < 实时（task_38 实测 k≈0.25~0.4）。
    **所有时长判据一律用物理时间**（世界 `frame_ts_ms` / 行人 pose 的 `world_time`），
    墙钟值在 verdict 里并列但不参与判定。
  * `[Sim] UAV … → v=… omega=…`（`sim_handler.rs` 的 `v={}`/`omega={}`，5 Hz 限流）里
    **`v`/`omega` 都是 `f32` 的物理量（m/s、rad/s），必须按小数解析**（旧解析只认整数 ⇒
    只看到 30% 的行）；**`|v|<5e-3 且 |omega|<5e-3` = 停**（`next=None`）；
    `v≈0 且 omega≠0` = 原地转（不算停）；`v>0` = 走。「停段」= 连续若干条「停」行的极大段。
    Step 2 之后日志里已无 `动作=Stop` 字段（旧离散执行器），本口径是它的语义等价（verdict §0）。
    相关判据带**覆盖率门**（`uav_coverage()`：已解析数必须等于日志里的行数，否则 `FAIL(env)`）。
  * **`[flight_ctrl]`（`vel_fwd` m/s / `yaw_rate_deg` °/s）是机栈速度的唯一来源**
    （task_39 §四 修订）；`[chassis]` 的 `forward_speed`/`turn_speed` 仍存在，但只服务
    `manual` 遥控与车栈，**机栈执行器已不读**。verdict 里所有角度/速度都由
    `executor_consts()` + `flight_ctrl_cfg()` + `world_caps()` 现算，不再写死。
  * `[Sim] UAV` 行只有 5 Hz 墙钟限流 → 看不到 <200 ms 的瞬停，这是**分辨率上限**（属被测代码）。
"""

import argparse
import ast
import bisect
import csv
from datetime import datetime, timezone
import json
import math
import os
import pathlib
import re
import shutil
import sys
import time
import traceback

# ================================================================ 复用基线（只 import，不改）

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOOL_DIR = ROOT / "Tool"
sys.path.insert(0, str(TOOL_DIR))
import sim_follow_test as FX                       # noqa: E402

REUSE_PATH = TOOL_DIR / "sim_follow_test.py"
REUSE_SHA256 = "2e4a54d39dcdf4cb9635430c1bcc9469b8eae4360d4395f874a7d3f1028ef76b"

SimBinary = ROOT / "target" / "release" / "pleiades-sim"
BASELINE = ROOT / "tmp" / "task39" / "git_baseline.txt"
BOUNDARY_ALLOW = {"?? Tool/sim_task39_step3_test.py"}
RESULTS = ROOT / "RobotSim_TestEnv" / "results"
INST = FX.INST
CFG = FX.CFG
HOST = FX.HOST
WORLD_PORT = FX.WORLD_PORT
DISPLAY = FX.DISPLAY
UAV_SPAWN = FX.UAV_SPAWN
DEFAULT_PORT = 9098

# ================================================================ 世界副本（第 2 轮环境修复）
# 源世界一个字节不改；副本只多一个 override.cfg（关 4× MSAA），其余 `diff -r` 必须为空。
WORLD_SRC = ROOT / "orion-simulation-world"
WORLD_FAST = ROOT / "RobotSim_TestEnv" / "world_fast"
OVERRIDE_TEXT = "[rendering]\nanti_aliasing/quality/msaa_3d=0\n"
WORLD_TIMEOUT_S = 3000          # 世界自带 `timeout`（预算 1800 s，不能让世界先死）
WORLD_COPIED_MARK = ".copied_from"

# ================================================================ 场景常量（世界坐标米，网格 0.5 m）

# 出生点 (64,64,2.5) 来自世界 S0（本步 S0 复验）；格心 = (n+0.5)×0.5
M1_TARGET = (84.25, 64.25)      # 直线：正东 ≈20 m
M2_TARGET = (84.25, 84.25)      # L 形：一次 90°（东→北）
M3_TARGET = (106.25, 84.25)     # 长路径：正东 22 m = 44 格
REPOS_W = (56.25, 64.25)        # F1 前置第一跳（人西侧，到达时机头朝西）
F1_START = (64.25, 64.25)       # F1 前置第二跳（8 m 东向 → 到达时机头朝东）
F2_PREP = (64.25, 64.25)
F2_START = (68.25, 64.25)
F3_START = (64.25, 64.25)

PED_F1_SPAWN = {"x": 70.0, "y": 64.5, "z": 0.0, "yaw": 0.0}
PED_F1_SPEED, PED_F1_DIR, PED_F1_TRACK_M = 1.26, 0.0, 25.2        # 20 物理 s × 1.26 m/s
PED_F2_SPAWN = {"x": 72.0, "y": 60.0, "z": 0.0, "yaw": math.pi / 2}
PED_F2_SPEED, PED_F2_DIR, PED_F2_TRACK_M = 0.60, math.pi / 2, 8.0  # (72,60) → (72,68) @0.6 m/s
PED_F3_SPAWN = {"x": 70.0, "y": 64.5, "z": 0.0, "yaw": 0.0}
PED_F3_SPEED, PED_F3_DIR = 1.26, 0.0
PEER_F1, PEER_F2, PEER_F3 = "human-f1", "human-f2", "human-f3"

# ================================================================ 判据阈值（§五 字面或由世界常数推导）

MOVE10_MIN = 5.0            # §五.3「10 s 内位移 ≥ 5 m」
DIR_COS_MIN = 0.707         # 「方向与人的移动方向一致」= 夹角 ≤45°
OOD_MAX_S = 1.0             # §五.3 F2①「最长连续出画 ≤ 1 s」
STOP_EP_MAX_F2 = 3          # §五.3 F2③「动作=Stop ≤ 3 次」
MIN_D = 2.0                 # §五.3 F2④ / §五.3 F4
F1_D_MED_MAX = 5.0          # F1「不掉队」末 5 物理 s 距离中位数
F1_D_END_MAX = 5.0          # F1「不掉队」末距离
M_STOP_EP_MAX = 2           # M1~M3 合法停段只允许 1 段（到达）+1 段余量
M_MICRO_CYCLE_MAX = 0       # 长度 ≤3 条且两侧都在走的停段必须为 0
M_OMEGA_FLIP_MAX = 3        # 仅 M2：拐角处 omega 符号翻转次数
M_ARRIVE_TOL = 0.30         # navigator.rs:35 ARRIVAL_THRESHOLD_M，逐轴
M_PATH_LO, M_PATH_HI = 0.90, 2.00
M_STEP_MAX = 0.50           # 相邻 10 Hz 采样单步位移上限
STEP_DT_MAX_S = 0.20        # path_metrics 只统计 Δt ≤ 0.2 s 的相邻对（正常采样 10 Hz；
                            # >0.2 s = 采样空洞，实测 M3 有 1 对 Δt=2.748 s ⇒ 0.693 m 是 0.25 m/s×2.75 s）
X_LO, X_HI = 0.5, 127.5
Z_LO, Z_HI = 2.0, 3.2
F3_DRIFT_MAX = 0.70         # 满速 2.0 m/s、MAX_ACCEL 3.0 ⇒ v²/2a = 0.667
F3_YAW_MAX = 10.0
F3_SETTLE_S = 3.0
F3_LAST1S_MAX = 0.10
F3_PRE_MOVE_MIN = 1.5
F3_PRE_V_LINES = 3
STALE_S = 0.8               # follow.rs:68 STALE_MS=800
D_FAR, D_NEAR = 4.0, 2.5    # follow.rs:64/66（只用于解释，不参与判定）

# 归位 / 目视几何 / 真人口径（第 2 轮新增；**都不是 §五 判据阈值**）
PREP_ARRIVE_LOOSE_M = 1.00    # 归位「宽松到达」：只作记录 + vantage 输入，不作跳过用例的理由
VANTAGE_RANGE_MAX_M = 11.0    # max_range_m=12 留 1 m 余量
VANTAGE_BEARING_MAX_DEG = 40.0  # 水平半 FOV 43.5°（fov=70 垂直 / 4:3）留余量
REAL_PERSON_TOL_M = 1.00      # adopted 帧算「对真人」的 est↔truth 距离阈值
POSE_AGE_RT_MAX_MS = 120.0    # 环境实时门（只 warn，不中止）
F2_UNDECIDABLE_IV_S = 0.6     # 帧间隔中位 >0.6 s ⇒ 出画判据不可判（实测 0.412 s 的 1.5× 余量）

# ── 任务级遗留门（`task_gates()`；**不进 §3 判据、不阻断运行**）
CARGO_LOG = ROOT / "tmp" / "task39" / "cargo_test.log"
NO_PATH_FILES = [
    "pleiades-uav/src/uav/nav/pathfinder.rs",
    "pleiades-ugv/src/ugv/nav/pathfinder.rs",
    "pleiades-sim/src/sim/uav_sim/nav/pathfinder.rs",
    "pleiades-sim/src/sim/ugv_sim/nav/pathfinder.rs",
]
# 复算等价性契约（`--regrade`；只用于断言，不用于渲染）
#
# 上一轮这里钉的是**上一轮的实测数字**（帧数 / FAIL 集合 / verdict 变化表），harness 一改
# 必然全红，而且会把「复算」变成「往旧答案上凑」。现在换成两条可核的契约：
#   ① 对**历史目录**复算 ⇒ 只把差异写进 §11，不判错（`REGRADE_LEGACY`）；
#   ② 对**当轮/自洽目录**复算 ⇒ `judge_diff` 必须为空 + `REGRADE_PIN` 若已填则逐项比对。
REGRADE_LEGACY = (
    "task39_20261029T081753Z",          # 第 1 轮（16 条前置、旧 Stop 口径）
    "task39_20261029T085418Z",          # 第 2 轮（夹具朝向修正前）
    "task39_20261029T091203Z",          # 第 2 轮最终证据（旧 `90°/s` 口径）
    "task39_s4_m1f2_20261029T094918Z",  # 口径修订后 2 用例 probe（解析只到 30%）
)
# 当轮复算的钉死值：**跑完取证轮后**把实测（帧数/计数/fail 集）填进来，再复算须 0 diff。
# 实测（取证轮 `task39_20261029T100655Z`，2026-10-29T10:06:55Z~10:10:55Z）：
#   帧数 590；PASS 23 / FAIL 5 / FAIL(env) 0 / N-A 0；
#   FAIL = L9b, F1-2, F2-1, F2-2, F2-3。
# 注意：`REGRADE_PIN` 只对应**这一个**取证轮。要对别的（既不在 `REGRADE_LEGACY`、也不是
# 取证轮）目录复算，先清空它或把该目录加进 `REGRADE_LEGACY`，否则会按设计判 `FAIL(env)`
# ——这条是防「往旧答案上凑」，不是误报。
REGRADE_PIN = {
    "frames": 590,
    "counts": {"PASS": 23, "FAIL": 5, "FAIL(env)": 0, "N-A": 0, "total": 28},
    "fail_ids": ["L9b", "F1-2", "F2-1", "F2-2", "F2-3"],
}

# 预算（墙钟秒）；世界自带 `timeout 3000`
CASE_TIMEOUT_WALL = {"m1": 90.0, "m2": 90.0, "m3": 90.0,
                     "f1": 240.0, "f2": 150.0, "f3": 150.0}
REPOS_TIMEOUT_WALL = 70.0       # 单跳归位上限 = 树 Timeout(60 s) + 10 s 出队余量
TOTAL_BUDGET_WALL = 1800.0
PROBE_WORLD_ITERS = 40          # --probe-env：各跑 40 帧
PROBE_TIMEOUT_WALL = 600.0

# ================================================================ 日志解析契约

UAV_RE = re.compile(
    r"\[Sim\] UAV 位置=\(([-\d.]+),([-\d.]+)\) yaw=([-\d.]+)° "
    r"next=(None|Some\(\((-?\d+), (-?\d+)\)\)) → v=(-?[\d.]+) omega=(-?[\d.]+) 位姿龄=(\d+)ms")
V_EPS = 5e-3                    # `v`/`omega` 的「零」容差：`f32` Display 给 `0`/`0.8`/`-0.5235988`
NAV_RE = re.compile(r"\[Nav\] (到达目标|已在目标格.+?到达)")
BLOCKED_RE = re.compile(r"\[Nav\] 连续 \d+ 拍无路")
NOTREADY_RE = re.compile(r"\[Nav\] 连续 \d+ 拍未算完")
CTRL_GOTO_RE = re.compile(r"\[control\] goto \(([-\d.]+), ([-\d.]+)\) → cmd_tx")
CTRL_FOLLOW_RE = re.compile(r"\[control\] follow → cmd_tx")
MISSION_FAIL_RE = re.compile(r"\[Sim\] 任务 TaskId\(\d+\) 失败，原因码 (\d+)")
ESTOP_RE = re.compile(r"\[Sim\] 前方障碍")
NO_LIDAR_RE = re.compile(r"\[Sim\] 无 lidar（未配置 lidar_2d）：on_tick 跳过急停")
LOG_TS_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d+)Z")
SPAWN_RE = r"\[ws\] pedestrian spawned peer='%s'.*spawn=\(([-\d.]+), ([-\d.]+)\) yaw=([-\d.]+)"
REMOVED_RE = r"\[ws\] pedestrian removed peer='%s'"

Log = FX.Log
now_utc_stamp = FX.now_utc_stamp
sha256_file = FX.sha256_file
dist = FX.dist
median = FX.median


# ================================================================ 小工具

def fmt(v, n=3):
    if v is None:
        return "—"
    if isinstance(v, bool):
        return "是" if v else "否"
    if isinstance(v, float):
        return ("%%.%df" % n) % v
    return str(v)


MONO_OFF = 0.0          # ctx.mono_off = monotonic − time.time()，main 里赋值


def log_wall(line):
    """tracing 行首的 UTC 时间戳 → monotonic 秒（+ MONO_OFF，与 detector 帧 `wall` 同基）。"""
    m = LOG_TS_RE.match(line)
    if not m:
        return None
    try:
        return datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S.%f").replace(
            tzinfo=timezone.utc).timestamp() + MONO_OFF
    except ValueError:
        return None


def frame_person_adopted(fr):
    """该帧里「跟随树真正吃到的目标」：cls=person 且 adopted=True 的最高 conf 框。"""
    best = None
    for d in fr["dets"]:
        if d["cls"] != "person" or not d.get("adopted"):
            continue
        if best is None or d["conf"] > best["conf"]:
            best = d
    return best


def crit(name, ok, detail="", **extra):
    d = {"name": name, "ok": bool(ok), "detail": str(detail)}
    d.update(extra)
    return d


def wrap_deg(d):
    return (d + 180.0) % 360.0 - 180.0


# ================================================================ 物理时间钟（wall → world）

class WorldClock:
    """把墙钟时刻映射到世界（物理）时间：detector 帧自带 `frame_ts_ms`，按 wall 排序插值。

    为什么要它：相机帧间隔 0.3~0.5 物理 s，直接拿「最新一帧」当采样时刻会把
    F3-2/F3-3 这种 1 s 量级的窗口糊成一格。线性插值给出亚帧分辨率。
    """

    def __init__(self, max_extrap_s=120.0):
        self.pts = []                 # (wall, world)
        self._ws = []
        self._i = 0
        self._max_extrap = max_extrap_s
        self.rewinds = 0

    def sync(self, ctx):
        while self._i < len(ctx.frames):
            fr = ctx.frames[self._i]
            self._i += 1
            self._add(float(fr["wall"]), fr["world_ms"] / 1000.0)

    def _add(self, w, t):
        if self.pts and w <= self.pts[-1][0]:
            return
        if self.pts and t < self.pts[-1][1]:
            self.rewinds += 1
            return
        self.pts.append((w, t))
        self._ws.append(w)

    def _slope(self, i):
        lo = max(0, i - 10)
        hi = min(len(self.pts) - 1, i + 10)
        if hi <= lo:
            return 1.0
        dw = self.pts[hi][0] - self.pts[lo][0]
        dt = self.pts[hi][1] - self.pts[lo][1]
        if dw <= 0.0:
            return 1.0
        return min(max(dt / dw, 0.02), 2.0)

    def at(self, wall):
        if wall is None or not self.pts:
            return None
        if len(self.pts) == 1:
            return self.pts[0][1]
        i = bisect.bisect_left(self._ws, wall)
        if i <= 0:
            return self.pts[0][1] + self._slope(0) * (wall - self.pts[0][0])
        if i >= len(self.pts):
            w, t = self.pts[-1]
            dt = min(max(wall - w, 0.0), self._max_extrap)
            return t + self._slope(len(self.pts) - 1) * dt
        w0, t0 = self.pts[i - 1]
        w1, t1 = self.pts[i]
        if w1 <= w0:
            return t1
        return t0 + (t1 - t0) * (wall - w0) / (w1 - w0)

    def span(self, wall0, wall1):
        """返回 (world0, world1, k)；k = Δ物理/Δ墙钟。"""
        w0, w1 = self.at(wall0), self.at(wall1)
        k = None
        if w0 is not None and w1 is not None and wall1 > wall0:
            k = (w1 - w0) / (wall1 - wall0)
        return w0, w1, k


# ================================================================ 采样（复用 FX.Ctx.sample + 物理时间）

def sample(ctx, seg=None):
    """采一拍：status + 最新 detector 帧 + 行人位姿；`t_world` 统一用物理时钟插值。"""
    row = ctx.sample(seg or ctx.seg)
    ctx.clock.sync(ctx)
    tw = ctx.clock.at(row["t_mono"])
    if tw is not None:
        row["t_world"] = round(tw, 3)
    row["pose_age_ms"] = latest_pose_age(ctx)
    return row


def sample_phys(ctx, seconds, seg=None, dt=0.10, deadline=None):
    """按**物理时间**采 seconds 秒。"""
    sample(ctx, seg)                      # 先采一拍，确保物理钟已 sync
    t_now = ctx.clock.at(time.monotonic())
    if t_now is None:
        t_now = ctx.rows[-1].get("t_world") if ctx.rows else 0.0
    t_end = (t_now or 0.0) + seconds
    while True:
        sample(ctx, seg)
        now = ctx.clock.at(time.monotonic())
        if now is not None and now >= t_end:
            return
        if deadline is not None and time.monotonic() > deadline:
            return
        time.sleep(dt)


def seg_rows(ctx, seg):
    return [r for r in ctx.rows if r.get("seg") == seg and r.get("x") is not None]


def pose_at(rows, tw, keys=("x", "y", "z", "yaw")):
    """trace 行按 t_world 线性插值到位姿；返回 dict（含 interp=True/False）。"""
    pts = [(r["t_world"], r) for r in rows
           if r.get("t_world") is not None and r.get("x") is not None]
    if not pts:
        return None
    pts.sort(key=lambda p: p[0])
    ts = [p[0] for p in pts]
    i = bisect.bisect_left(ts, tw)
    if i <= 0:
        out = {k: pts[0][1].get(k) for k in keys}
        out["interp"] = False
        return out
    if i >= len(pts):
        out = {k: pts[-1][1].get(k) for k in keys}
        out["interp"] = False
        return out
    t0, r0 = pts[i - 1]
    t1, r1 = pts[i]
    if t1 <= t0:
        out = {k: r1.get(k) for k in keys}
        out["interp"] = False
        return out
    a = (tw - t0) / (t1 - t0)
    out = {}
    for k in keys:
        v0, v1 = r0.get(k), r1.get(k)
        if k == "yaw" and v0 is not None and v1 is not None:
            out[k] = v0 + math.radians(wrap_deg(math.degrees(v1 - v0))) * a
        else:
            out[k] = v0 if v0 is None or v1 is None else v0 + (v1 - v0) * a
    out["interp"] = True
    return out


def disp(rows, tw0, tw1):
    """[tw0, tw1] 的首末位移（米）+ 起末位姿。"""
    p0 = pose_at(rows, tw0)
    p1 = pose_at(rows, tw1)
    if p0 is None or p1 is None:
        return None
    return {"p0": p0, "p1": p1, "d": dist((p0["x"], p0["y"]), (p1["x"], p1["y"])),
            "dx": p1["x"] - p0["x"], "dy": p1["y"] - p0["y"],
            "dyaw_deg": abs(wrap_deg(math.degrees(p1["yaw"] - p0["yaw"])))
            if p0.get("yaw") is not None and p1.get("yaw") is not None else None}


# ================================================================ 轨迹 / 日志分析

def uav_lines(ctx, mark, mark_end=None):
    """解析 `[mark, mark_end)` 内的 `[Sim] UAV` 行（含物理时间）。"""
    out = []
    for line in ctx.sim_history[mark:mark_end]:
        m = UAV_RE.search(line)
        if not m:
            continue
        wall = log_wall(line)
        v = float(m.group(7))
        omega = float(m.group(8))
        out.append({
            "wall": wall,
            "t_world": None if wall is None else ctx.clock.at(wall),
            "x": float(m.group(1)), "y": float(m.group(2)), "yaw": float(m.group(3)),
            "next": None if m.group(4) == "None" else (int(m.group(5)), int(m.group(6))),
            "v": v, "omega": omega, "age_ms": int(m.group(9)),
            "kind": "stop" if (abs(v) < V_EPS and abs(omega) < V_EPS) else
                    ("turn" if abs(v) < V_EPS else "move"),
            "line": line.strip(),
        })
    return out


def uav_coverage(ctx, mark, mark_end=None):
    """(已解析, 日志里的 `[Sim] UAV` 行总数)：防「解析不到 ⇒ 零行 ⇒ 真空 PASS」。

    `sim_handler.rs` 的 `v={}`/`omega={}` 是 `f32` Display（`0.8`、`-0.5235988`），
    旧正则只认整数 ⇒ 440 行只解析到 133 行（30%），停段/翻转/pre_v 全部建在残样上。
    覆盖率不足的判据一律 `FAIL(env)`，不许当成「没问题」。
    """
    sl = ctx.sim_history[mark:mark_end]
    total = sum(1 for ln in sl if "[Sim] UAV" in ln)
    ok = sum(1 for ln in sl if UAV_RE.search(ln))
    return ok, total


def uav_re_selftest():
    """4 条**逐字取自 `sim_handler.rs` 格式串**的样本，断言解析出正确数值与 `next`。

    格式串：`"[Sim] UAV 位置=({:.2},{:.2}) yaw={:.1}° next={:?} → v={} omega={} 位姿龄={}ms"`
    （`v`/`omega` 是 `f32` Display，`next` 是 `Option<(i32,i32)>` 的 `Debug`）。
    """
    cases = [
        ("[Sim] UAV 位置=(64.00,64.00) yaw=0.0° next=None → v=0 omega=0 位姿龄=8ms",
         (64.0, 64.0, 0.0, None, 0.0, 0.0, 8)),
        ("[Sim] UAV 位置=(64.11,64.01) yaw=5.8° next=Some((129, 128)) → "
         "v=0.8 omega=-0.5235988 位姿龄=51ms",
         (64.11, 64.01, 5.8, (129, 128), 0.8, -0.5235988, 51)),
        ("[Sim] UAV 位置=(64.53,64.11) yaw=21.8° next=Some((130, 128)) → "
         "v=0 omega=0.5235988 位姿龄=148ms",
         (64.53, 64.11, 21.8, (130, 128), 0.0, 0.5235988, 148)),
        ("[Sim] UAV 位置=(65.02,64.29) yaw=-0.7° next=Some((131, 128)) → "
         "v=0.8 omega=0 位姿龄=102ms",
         (65.02, 64.29, -0.7, (131, 128), 0.8, 0.0, 102)),
    ]
    bad = []
    for line, want in cases:
        m = UAV_RE.search(line)
        if not m:
            bad.append("匹配不到：%s" % line[:48])
            continue
        got = (float(m.group(1)), float(m.group(2)), float(m.group(3)),
               None if m.group(4) == "None" else (int(m.group(5)), int(m.group(6))),
               float(m.group(7)), float(m.group(8)), int(m.group(9)))
        ok_vals = all((abs(a - b) <= 1e-6 if isinstance(a, float) and isinstance(b, float)
                       else a == b) for a, b in zip(got, want))
        if not ok_vals:
            bad.append("数值不符：%r ≠ %r" % (got, want))
    return {"ok": not bad, "n": len(cases), "bad": bad}


def stop_segments(lines):
    """极大「停」段：返回 [{i0,i1,n,prev,next,t0,t1}]。

    输入是 `uav_lines()` 的 float 行，零值容差 `V_EPS` 已在那边吸收，这里只比较 `kind`。
    """
    segs = []
    i = 0
    n = len(lines)
    while i < n:
        if lines[i]["kind"] != "stop":
            i += 1
            continue
        j = i
        while j + 1 < n and lines[j + 1]["kind"] == "stop":
            j += 1
        prev = lines[i - 1]["kind"] if i > 0 else None
        nxt = lines[j + 1]["kind"] if j + 1 < n else None
        segs.append({"i0": i, "i1": j, "n": j - i + 1, "prev": prev, "next": nxt,
                     "t0": lines[i]["t_world"], "t1": lines[j]["t_world"],
                     "micro": (j - i + 1) <= 3 and prev == "move" and nxt == "move"})
        i = j + 1
    return segs


def omega_flips(lines):
    """`omega` 符号翻转次数（输入为 float 行，零值容差已在 `uav_lines` 吸收）。"""
    signs = [1 if ln["omega"] > 0 else -1 for ln in lines if ln["omega"] != 0]
    return sum(1 for a, b in zip(signs, signs[1:]) if a != b)


def window_lines(lines, t_lo, t_hi):
    return [ln for ln in lines
            if ln["t_world"] is not None and t_lo <= ln["t_world"] <= t_hi]


def path_metrics(rows):
    """路径长 / 单步最大位移（**物理时间口径**）/ 越界计数。

    `prev_issues.md` 第 2 条：相邻采样必须校验 Δt。trace 里存在 Δt=2.748 s 的采样空洞
    （主循环被 detector 推理阻塞），2.75 s × 0.25 m/s = 0.693 m 会被误判成「单步跳变」。
    正常 10 Hz 采样（Δt≈0.05~0.1 s）⇒ 只统计 0 < Δt ≤ STEP_DT_MAX_S 的相邻对；
    未过滤最大值与其 Δt 一并返回，写进实测值备核。
    """
    pts = [(r.get("t_world"), r["x"], r["y"], r.get("z")) for r in rows
           if r.get("x") is not None]
    if len(pts) < 2:
        return {"n": len(pts), "path_m": None, "straight_m": None, "ratio": None,
                "step_max": None, "step_max_raw": None, "step_dt_raw": None,
                "step_pairs": 0, "step_pairs_dropped": 0, "oob": 0, "z_oob": 0}
    path = sum(dist((a[1], a[2]), (b[1], b[2])) for a, b in zip(pts, pts[1:]))
    straight = dist((pts[0][1], pts[0][2]), (pts[-1][1], pts[-1][2]))
    pairs = [(b[0] - a[0], dist((a[1], a[2]), (b[1], b[2])))
             for a, b in zip(pts, pts[1:]) if a[0] is not None and b[0] is not None]
    good = [p for p in pairs if 0.0 < p[0] <= STEP_DT_MAX_S]
    raw = max(pairs, key=lambda p: p[1]) if pairs else (None, None)
    oob = sum(1 for p in pts if not (X_LO <= p[1] <= X_HI and X_LO <= p[2] <= X_HI))
    z_oob = sum(1 for p in pts if p[3] is not None and not (Z_LO <= p[3] <= Z_HI))
    return {"n": len(pts), "path_m": round(path, 3), "straight_m": round(straight, 3),
            "ratio": round(path / straight, 3) if straight > 1e-6 else None,
            "step_max": None if not good else round(max(p[1] for p in good), 3),
            "step_max_raw": None if raw[1] is None else round(raw[1], 3),
            "step_dt_raw": None if raw[0] is None else round(raw[0], 3),
            "step_pairs": len(good), "step_pairs_dropped": len(pairs) - len(good),
            "oob": oob, "z_oob": z_oob}


def frames_in(ctx, tw_lo, tw_hi):
    ctx.clock.sync(ctx)
    out = []
    for fr in ctx.frames:
        tw = ctx.clock.at(float(fr["wall"]))
        if tw is None or not (tw_lo <= tw <= tw_hi):
            continue
        out.append((tw, fr))
    out.sort(key=lambda p: p[0])
    return out


def oob_runs(framelist, presence, tw_lo, tw_hi):
    """窗内「连续出画」段（物理时间）：返回 [{start,end,dur,n}]，按 dur 降序。

    出画段 = 连续 presence==False 的帧；时长 = 后一「在画」帧的物理时刻 − 前一「在画」帧的物理时刻
    （窗端点截断），这正是 §五.4 的「连续无 person 的最长间隔」。
    """
    if not framelist:
        return []
    flags = [(tw, bool(presence(fr))) for tw, fr in framelist]
    runs = []
    i, n = 0, len(flags)
    while i < n:
        if flags[i][1]:
            i += 1
            continue
        j = i
        while j + 1 < n and not flags[j + 1][1]:
            j += 1
        # 出画段的两端：上一「在画」帧 / 下一「在画」帧；窗外则截断到窗边界
        start = tw_lo if i == 0 else flags[i - 1][0]
        end = flags[j + 1][0] if j + 1 < n else tw_hi
        runs.append({"start": start, "end": end, "dur": max(0.0, end - start),
                     "kind": "head" if i == 0 else "mid", "frames": j - i + 1})
        i = j + 1
    runs.sort(key=lambda r: r["dur"], reverse=True)
    return runs


# ================================================================ 世界副本（第 2 轮）

def world_argv_fast(world_dir=None):
    """世界 argv：与上一轮逐字一致，只有 `--path` 指向副本、`timeout` 1200→3000。"""
    wd = pathlib.Path(world_dir or WORLD_FAST)
    return ["timeout", str(WORLD_TIMEOUT_S), "godot",
            "--display-driver", "x11", "--rendering-driver", "opengl3",
            "--audio-driver", "Dummy", "--path", str(wd),
            "--resolution", "1600x900", "--position", "0,0"]


def world_stamp(world_dir):
    """副本新鲜度标记：源树 HEAD + src/assets/tool 最新 (mtime,path) + project.godot sha。"""
    wd = pathlib.Path(world_dir)
    try:
        head = FX.run_capture(
            ["git", "-C", str(wd), "rev-parse", "HEAD"], timeout=60)[1].strip()[:12]
    except Exception:
        head = "?"
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
    psha = sha256_file(proj)[:16] if proj.is_file() else "?"
    return "HEAD=%s\nnewest=%s|%s\nproject=%s" % (head, int(newest[0]), newest[1], psha)


def world_marker_path():
    """标记放在 `.godot/` 内：`diff -r` 排除 `.godot`，副本里就只剩 `override.cfg` 一个非原文件。"""
    return WORLD_FAST / ".godot" / WORLD_COPIED_MARK


def ensure_world_fast(force=False, reuse=False):
    """幂等创建/刷新世界副本（源世界零改动；副本唯一差异 = override.cfg）。

    `reuse=True`（`--reuse-world`）时无条件复用现有副本，不校验源树是否已变。
    """
    want = world_stamp(WORLD_SRC)
    marker = world_marker_path()
    try:
        have = marker.read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        have = ""
    ent = {"copied": False, "reason": "", "seconds": 0.0,
           "stamp": want.replace("\n", " / "), "force": bool(force)}
    ready = (WORLD_FAST / "project.godot").is_file() and \
        (WORLD_FAST / ".godot" / "imported").is_dir()
    if reuse and ready:
        ent["reason"] = "复用（--reuse-world，跳过刷新）"
        Log("[world_fast] %s" % ent["reason"])
        return ent
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
    Log("[world_fast] %s：%s（%.2f s，stamp=%s）" % (
        ent["reason"], WORLD_FAST.relative_to(ROOT), ent["seconds"], ent["stamp"]))
    return ent


def launch_world_fast(ctx):
    """起副本世界并等 `[ws] listening on port 9999`（150 s）。"""
    if not FX.display_alive():
        raise FX.HarnessError("Xvfb :99 不可用")
    argv = world_argv_fast(ctx.world_dir)
    ctx.world.start(argv, ROOT, ctx.out / "world.log", env=FX.set_world_env())
    m, line = ctx.wait_world(r"\[ws\] listening on port %d" % WORLD_PORT, 150)
    if m is None:
        raise FX.HarnessError("副本世界 150 s 内没有监听 :%d（见 world.log）" % WORLD_PORT)
    ctx.env["world_argv"] = " ".join(argv)
    return line


def probe_env(ctx):
    """两次世界（源世界 MSAA 4× / 副本 MSAA 0），各 `--print-fps --quit-after 40`。"""
    rows = []
    for label, wdir in (("msaa4x_orig_world", WORLD_SRC), ("msaa0_world_fast", WORLD_FAST)):
        log = ctx.out / ("probe_%s.log" % label)
        argv = ["timeout", str(int(PROBE_TIMEOUT_WALL)), "godot",
                "--display-driver", "x11", "--rendering-driver", "opengl3",
                "--audio-driver", "Dummy", "--path", str(wdir),
                "--resolution", "1600x900", "--position", "0,0",
                "--print-fps", "--quit-after", str(PROBE_WORLD_ITERS)]
        t0 = time.monotonic()
        ctx.world.start(argv, ROOT, log, env=FX.set_world_env())
        rc, timed_out = None, False
        try:
            rc = ctx.world.p.wait(timeout=PROBE_TIMEOUT_WALL)
        except Exception:
            timed_out = True
        wall = round(time.monotonic() - t0, 2)
        try:
            ctx.world.stop()
        except Exception:
            pass
        try:
            text = log.read_text(encoding="utf-8", errors="replace")
        except OSError:
            text = ""
        fps = [float(x) for x in re.findall(r"Project FPS: ([0-9.]+)", text)]
        rows.append({
            "knob": "rendering/anti_aliasing/quality/msaa_3d",
            "value": "2（世界默认 4× 抗锯齿）" if "msaa4x" in label else "0",
            "world": "orion-simulation-world" if "msaa4x" in label else "RobotSim_TestEnv/world_fast",
            "wall_40f": wall,
            "fps_printed": fps[-1] if fps else None,
            "fps_min": min(fps) if fps else None,
            "fps_max": max(fps) if fps else None,
            "pose_age_ms_median": "",
            "rc": rc, "timed_out": timed_out,
        })
        Log("[probe-env] %-18s wall=%.2f s fps=%s（%d 次打印）" % (
            label, wall, rows[-1]["fps_printed"], len(fps)))
    with (ctx.out / "env_probe.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["knob", "value", "world", "wall_40f",
                                          "fps_printed", "fps_min", "fps_max",
                                          "pose_age_ms_median", "rc", "timed_out"],
                           extrasaction="ignore")
        w.writeheader()
        for r in rows:
            w.writerow(r)
    ctx.env["probe_env"] = rows
    return rows


# ================================================================ 前置自检（21 条，自己写）

def _eq(a, b, tol=1e-9):
    return abs(float(a) - float(b)) <= tol


def harness_lint(path=None):
    """AST 自检：① 模块级 `def` 必须至少被调用一次；② 禁用标识符不得复活。

    这是对 `tmp/task39/prev_issues.md` 第 3 条（4 处死代码 + 门禁未接线）的反向锁：
    只要有人再写一个「定义了从不调用」的函数，或把 `shot_mark` / `_idle_settled` / `pct`
    写回来，`--preflight` 第 21 条就红。禁用名用拼接构造，避免本检查自己命中自己。
    """
    src = (pathlib.Path(path) if path else pathlib.Path(__file__)).read_text(
        encoding="utf-8", errors="replace")
    tree = ast.parse(src)
    defs = [n.name for n in tree.body
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))]
    banned = ("shot" + "_mark", "_idle" + "_settled", "pct")
    calls, hits = {}, {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            f = node.func
            nm = f.id if isinstance(f, ast.Name) else (
                f.attr if isinstance(f, ast.Attribute) else None)
            if nm:
                calls[nm] = calls.get(nm, 0) + 1
        elif isinstance(node, ast.Name) and node.id in banned:
            hits[node.id] = hits.get(node.id, 0) + 1
        elif isinstance(node, ast.Attribute) and node.attr in banned:
            hits[node.attr] = hits.get(node.attr, 0) + 1
        elif isinstance(node, ast.arg) and node.arg in banned:
            hits[node.arg] = hits.get(node.arg, 0) + 1
    dead = [d for d in defs if calls.get(d, 0) == 0]
    return {"ok": not dead and not hits, "dead_defs": dead, "banned": hits,
            "defs": len(defs), "dead": dead, "hits": sorted(hits)}


def executor_consts():
    """从 `executor.rs` 读 `DEADBAND_DEG` / `SPIN_DEG`（**不写死度数**）。

    `prev_issues.md` 第 3 条：旧叙述把「6.8°」说成「大于 `SPIN_DEG=20°`」，与常量自相矛盾。
    判据文案里出现的每一档阈值都必须来自源码，读不到就返回 `(None, None)` 并在文本里说明。
    """
    path = ROOT / "pleiades-sim" / "src" / "sim" / "uav_sim" / "executor.rs"
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None, None
    out = []
    for name in ("DEADBAND_DEG", "SPIN_DEG"):
        m = re.search(r"%s:\s*f32\s*=\s*([\d.]+)" % name, text)
        out.append(None if m is None else float(m.group(1)))
    return tuple(out)


def flight_ctrl_cfg(cfg):
    """`[flight_ctrl]`（task_39 §四 修订）：`vel_fwd` m/s、`yaw_rate_deg` °/s；缺键返回 None。

    这是**机栈速度的唯一来源**（`sim_handler.rs` 读它构造 `UavExecutor`）；`[chassis]` 的
    `forward_speed`/`turn_speed` 仍存在但只服务 `manual` 遥控与车栈，不再进机栈执行器。
    """
    fc = (cfg or {}).get("flight_ctrl") or {}

    def f(k):
        try:
            return float(fc.get(k))
        except (TypeError, ValueError):
            return None
    return {"vel_fwd": f("vel_fwd"), "yaw_rate_deg": f("yaw_rate_deg")}


def world_caps():
    """世界侧速度/角速度上限（`Quadrotor.gd` 生效参数），用于判「是否被世界截断」。

    返回 `(max_speed_mps, max_yaw_rate_rps)`；读不到返回 `(None, None)`。
    """
    path = ROOT / "orion-simulation-world" / "src" / "chassis" / "Quadrotor.gd"
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None, None
    ms = re.search(r"var\s+_max_speed\s*:=\s*([\d.]+)", text)
    my = re.search(r"var\s+_max_yaw_rate\s*:=\s*PI\s*\*\s*([\d.]+)", text)
    return (None if ms is None else float(ms.group(1)),
            None if my is None else math.pi * float(my.group(1)))


def turn_zone_text(age_s, fc=None):
    """位姿龄 × `[flight_ctrl].yaw_rate_deg` → 每拍转角落在执行器的哪一档。

    `executor.rs` 三档全有全无：`|Δθ| ≤ DEADBAND_DEG` ⇒ 直行（omega=0）；
    `DEADBAND_DEG < |Δθ| ≤ SPIN_DEG` ⇒ 边走边转（`v=vel_fwd`、`omega=±yaw_rate`）；
    `|Δθ| > SPIN_DEG` ⇒ 原地转（`v=0`）。比较句由源码常量 + 配置现算，**不写死结论**；
    `fc` 缺省时现场读 `[flight_ctrl]`（复算/取证两条路径共用本函数）。
    """
    if fc is None:
        try:
            fc = flight_ctrl_cfg(FX.load_config())
        except Exception:                       # 配置读不到 ⇒ 档位不可判（照实写）
            fc = None
    fc = fc or {}
    db, spin = executor_consts()
    consts = "`executor.rs` `DEADBAND_DEG=%s°`、`SPIN_DEG=%s°`；" \
             "`[flight_ctrl]` `vel_fwd=%s m/s`、`yaw_rate_deg=%s °/s`" % (
                 fmt(db, 1), fmt(spin, 1),
                 fmt(fc.get("vel_fwd"), 2), fmt(fc.get("yaw_rate_deg"), 1))
    if age_s is None or db is None or spin is None or fc.get("yaw_rate_deg") is None:
        return {"deg": None, "band": "位姿龄/常量/`[flight_ctrl]` 读不到 ⇒ 档位不可判",
                "cmp": consts}
    deg = fc["yaw_rate_deg"] * age_s
    if deg <= db:
        band = "直行档（v=vel_fwd、omega=0）"
        cmp = "%.1f° ≤ DEADBAND_DEG=%.1f°" % (deg, db)
    elif deg <= spin:
        band = "边走边转档（v=vel_fwd、omega=±yaw_rate）"
        cmp = "DEADBAND_DEG=%.1f° < %.1f° ≤ SPIN_DEG=%.1f°" % (db, deg, spin)
    else:
        band = "原地转档（v=0、omega=±yaw_rate）"
        cmp = "%.1f° > SPIN_DEG=%.1f°" % (deg, spin)
    return {"deg": round(deg, 1), "band": band, "cmp": cmp}


def _seg_of(ctx, tw):
    """物理时刻 → 所属用例段（`M1`~`M3` / `F1`~`F3`）；窗外返回「（用例窗外）」。"""
    if tw is None:
        return "（无物理时间）"
    for k in ("m1", "m2", "m3", "f1", "f2", "f3"):
        rec = (ctx.cases_done or {}).get(k)
        if not rec:
            continue
        t0, t1 = rec.get("t_world_start"), rec.get("t_world_end")
        if t0 is not None and t1 is not None and t0 <= tw <= t1:
            return rec.get("name") or k.upper()
    return "（用例窗外）"


def person_frame_breakdown(ctx):
    """§5.1 的三桶量化（`prev_issues.md` 第 8 条）——**所有数字都由数据算出，不写死**。

    ① 有 `est` 的帧 = `follow.rs` 会吃的帧：真人 / bbox 触底幻影 / 无真值 / est↔truth 偏差>1 m；
    ② 仅 bbox 的帧 = 无 `est`/`adopted` ⇒ 进不了 follow，按拒答原因直方图；
    ③ 关事件之后仍有 `est` 的帧（真值已消失仍能给出 2.5 m 幻影）。
    分桶互斥且穷尽：`real + bottom_edge + no_truth + len(est_off) == with_est`（断言在返回值里）。
    """
    cached = getattr(ctx, "_pfb", None)
    if cached is not None:
        return cached
    ctx.clock.sync(ctx)
    close = ((ctx.cases_done or {}).get("f3") or {}).get("t_close_world")
    out = {"frames": len(ctx.frames or []), "person": 0, "with_est": 0, "adopted": 0,
           "real": 0, "est_off": [], "bottom_edge": 0, "no_truth": 0,
           "bbox_only": 0, "reject": {}, "bbox_t0": None, "bbox_t1": None,
           "bbox_before_close": 0, "post_close_est": [], "by_case": {},
           "close_world": close, "invariant_ok": True}
    for fr in (ctx.frames or []):
        p = FX.frame_person(fr)
        if p is None:
            continue
        out["person"] += 1
        tw = ctx.clock.at(float(fr["wall"]))
        seg = _seg_of(ctx, tw)
        out["by_case"][seg] = out["by_case"].get(seg, 0) + 1
        if p.get("est") is None:
            out["bbox_only"] += 1
            reason = (p.get("reject") or "（无原因码）").split("（")[0]
            out["reject"][reason] = out["reject"].get(reason, 0) + 1
            if tw is not None:
                out["bbox_t0"] = tw if out["bbox_t0"] is None else min(out["bbox_t0"], tw)
                out["bbox_t1"] = tw if out["bbox_t1"] is None else max(out["bbox_t1"], tw)
                if close is not None and tw <= close:
                    out["bbox_before_close"] += 1
            continue
        out["with_est"] += 1
        if p.get("adopted"):
            out["adopted"] += 1
        truth = ped_truth_at(ctx, float(fr["wall"]))
        if (truth is not None and p["bbox"][3] < fr["height"] - 1
                and dist(p["est"], truth) <= REAL_PERSON_TOL_M):
            out["real"] += 1
        elif p["bbox"][3] >= fr["height"] - 1:          # ① 框底边触图像底边 y=479
            out["bottom_edge"] += 1
        elif truth is None:                              # ② 真值不存在
            out["no_truth"] += 1
        else:                                            # ③ est 偏离真人 >1 m
            out["est_off"].append({"d_m": round(dist(p["est"], truth), 2),
                                   "range_m": p.get("range_m"),
                                   "t_world": None if tw is None else round(tw, 2),
                                   "seg": seg})
        if close is not None and tw is not None and tw > close:
            pos = pose_at(ctx.rows, tw)
            out["post_close_est"].append({
                "t_world": round(tw, 2), "range_m": p.get("range_m"),
                "adopted": bool(p.get("adopted")),
                "est": (round(p["est"][0], 2), round(p["est"][1], 2)),
                "uav": None if pos is None else (round(pos["x"], 2), round(pos["y"], 2))})
    out["invariant_ok"] = (out["real"] + out["bottom_edge"] + out["no_truth"]
                           + len(out["est_off"]) == out["with_est"])
    ctx._pfb = out
    return out


def task_gates():
    """任务级遗留门（**不进 §3 判据、不阻断运行**）。

    只登记「本轮无权修、必须由允许改对应 crate 的 step 补齐」的既有缺口，防它被当成
    「这轮该修」或「已经修好」：
      ① 1a `NoPath` 单测 4/4（`Task/…:121-127` §二 1.5 #3）；
      ② `cargo test -p pleiades-sim --bins` 是否全绿（HEAD 即红也要如实登记）。
    """
    out = []
    have, miss = [], []
    for rel in NO_PATH_FILES:
        try:
            text = (ROOT / rel).read_text(encoding="utf-8", errors="replace")
        except OSError:
            miss.append("%s（读不到）" % rel)
            continue
        if re.search(r"fn test_[a-z0-9_]*no_path[a-z0-9_]*\s*\(", text):
            have.append(rel)
        else:
            miss.append(rel)
    out.append(crit("no_path_unit_tests_4of4", not miss,
                    "%d/%d 份带 `NoPath` 用例%s" % (
                        len(have), len(NO_PATH_FILES),
                        "" if not miss else "；缺：%s" % "、".join("`%s`" % m for m in miss))))
    try:
        text = CARGO_LOG.read_text(encoding="utf-8", errors="replace")
    except OSError:
        out.append(crit("existing_unit_tests", False,
                        "未跑（缺 `%s`）" % CARGO_LOG.relative_to(ROOT)))
    else:
        hits = re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed", text)
        if not hits:
            out.append(crit("existing_unit_tests", False,
                            "`%s` 里没有 `test result:` 行" % CARGO_LOG.relative_to(ROOT)))
        else:
            _state, n_pass, n_fail = hits[-1]
            names = []
            if int(n_fail):
                idx = text.rfind("failures:")
                block = text[idx:] if idx >= 0 else ""
                names = re.findall(r"^\s{4}([\w:]+)$", block, re.M)[:3]
            out.append(crit("existing_unit_tests", int(n_fail) == 0,
                            "`cargo test -p pleiades-sim --bins` 末条 %s passed / %s failed%s" % (
                                n_pass, n_fail,
                                "" if not names else "；红：%s" % "、".join(
                                    "`%s`" % n for n in names))))
    return out


def overall_of(judges):
    """`overall` + `reason` 的单点实现（新增 `N-A` 分支，`main` 与 `regrade_run` 共用）。"""
    def ids(v):
        return [i for i in JUDGE_ORDER if (judges.get(i) or {}).get("verdict") == v]

    real, env, na = ids("FAIL"), ids("FAIL(env)"), ids("N-A")
    if real:
        reason = "不过的判据：%s" % ", ".join(real)
        if env:
            reason += "（另有 %d 条 FAIL(env)：%s）" % (len(env), ", ".join(env))
        if na:
            reason += "（另有 %d 条 N-A（真空）：%s）" % (len(na), ", ".join(na))
        return "FAIL", reason
    if env:
        return "FAIL(env)", "不过的判据：%s" % ", ".join(env)
    if na:
        return "FAIL(env)", "%d 条判据因前置不成立而不可判（N-A）：%s" % (len(na), ", ".join(na))
    return "PASS", "%d 条判据全 PASS" % len(judges)


def judge_diff(old, new):
    """两次判据表逐条比较（`verdict`/`pass`/`measured`），返回变化项。"""
    out = []
    for ident in JUDGE_ORDER:
        a = (old or {}).get(ident) or {}
        b = (new or {}).get(ident) or {}
        fields = [f for f in ("verdict", "pass", "measured") if a.get(f) != b.get(f)]
        if fields:
            out.append({"id": ident, "old": a.get("verdict"), "new": b.get("verdict"),
                        "fields": fields, "old_measured": a.get("measured"),
                        "new_measured": b.get("measured")})
    return out


def load_artifacts_ctx(src, out):
    """从产物目录**只读**重建 ctx（与实时跑同一批 judge 函数；原型已验证逐条等价）。

    `sim_trace.log` 比 `detector_frames.csv` 多几帧（`write_csvs()` 之后才到的推理行），
    按 CSV 的 `seq` 集合过滤 ⇒ 帧数与判据时刻的快照一致（实测 1067→1060）。
    """
    src = pathlib.Path(src)
    need_env = ("cases", "detectors", "judges", "preflight", "s0", "sim_binary_sha256",
                "target_binary_sha256", "reuse_script", "control_port", "world_dir",
                "world_copy")
    try:
        env = json.loads((src / "env.json").read_text(encoding="utf-8"))
    except OSError as exc:
        raise FX.HarnessError("读不了 %s/env.json：%r" % (src, exc))
    miss = [k for k in need_env if k not in env]
    if miss:
        raise FX.HarnessError("%s/env.json 缺键：%s" % (src, miss))
    need_cols = ("t_wall", "t_mono", "t_world", "seg", "x", "y", "z", "yaw", "d_m")
    try:
        tr = list(csv.DictReader((src / "trace.csv").open(encoding="utf-8")))
        det = list(csv.DictReader((src / "detector_frames.csv").open(encoding="utf-8")))
    except OSError as exc:
        raise FX.HarnessError("读不了 %s 的 trace.csv / detector_frames.csv：%r" % (src, exc))
    if not tr or not det:
        raise FX.HarnessError("%s 的 trace.csv 或 detector_frames.csv 为空" % src)
    missc = [c for c in need_cols if c not in tr[0]]
    if missc:
        raise FX.HarnessError("%s/trace.csv 缺列：%s" % (src, missc))
    rows = []
    for r in tr:
        row = dict(r)
        for k in ("t_wall", "t_mono", "t_world", "x", "y", "z", "yaw", "d_m",
                  "ped_x", "ped_y", "pose_age_ms", "task_ticks", "queue_len"):
            row[k] = None if row.get(k) in (None, "") else float(row[k])
        row["task_id"] = None if row.get("task_id") in (None, "") else int(row["task_id"])
        row["mode"] = row.get("mode") or None
        row["nav_status"] = row.get("nav_status") or None
        rows.append(row)
    global MONO_OFF
    MONO_OFF = (rows[0]["t_mono"] or 0.0) - (rows[0]["t_wall"] or 0.0)
    keep = {int(r["seq"]) for r in det}
    frames = []
    with (src / "sim_trace.log").open(encoding="utf-8", errors="replace") as fh:
        for line in fh:
            fr = FX.parse_detector_line(line, MONO_OFF)
            if fr is not None and fr["seq"] in keep:
                fr["fp"] = FX.frame_fp_classes(fr)
                frames.append(fr)
    if not frames:
        raise FX.HarnessError("%s/sim_trace.log 里没有与 CSV `seq` 匹配的 detector 帧" % src)

    class _Ctx:
        pass

    ctx = _Ctx()
    ctx.out = pathlib.Path(out)
    ctx.env = {k: v for k, v in env.items()
               if k not in ("cases", "detectors", "judges", "overall", "reason",
                            "preflight", "s0")}
    ctx.cases_done = {}
    for k, v in (env.get("cases") or {}).items():
        rec = dict(v)
        dets = (env.get("detectors") or {}).get(k)
        if dets:
            rec["det"] = dets
        if rec.get("wall_s") is None and rec.get("t_wall_start") is not None \
                and rec.get("t_wall_end") is not None:
            rec["wall_s"] = round(rec["t_wall_end"] - rec["t_wall_start"], 2)
        ctx.cases_done[k] = rec
    ctx.cases_failed = {}
    ctx.frames = frames
    ctx.rows = rows
    ctx.sim_history = (src / "sim_trace.log").read_text(
        encoding="utf-8", errors="replace").splitlines(True)
    ctx.clock = WorldClock()
    ctx.clock.sync(ctx)
    ctx.control_port = env.get("control_port")
    ctx.world_dir = pathlib.Path(env.get("world_dir") or WORLD_FAST)
    ctx.world_copy = env.get("world_copy") or {}
    ctx.residual = {}
    ctx.shots = env.get("shots") or []
    ctx.xvfb_reused = False
    ctx.sim_tail = None
    ctx.ped = None
    ctx.ped_peer = None
    ctx.seg = "END"
    ctx.regrade_src = str(src)
    ctx.regrade_env = env
    ctx.git_before = FX.git_status_lines()
    ctx._truth_all = []
    for k in ("f1", "f2", "f3"):
        for p in ((ctx.cases_done.get(k) or {}).get("ped_poses") or []):
            ctx._truth_all.append(tuple(p))
    return ctx


def copy_evidence(src, out):
    """把取证轮产物**只读**复制到复算目录，并写 `source_manifest.json/.sha256`。"""
    src, out = pathlib.Path(src), pathlib.Path(out)
    names = ["trace.csv", "detector_frames.csv", "ped_trace.csv", "sim_trace.log",
             "sim.log", "world.log", "script.log", "config.toml", "env_probe.csv"]
    names += sorted(p.name for p in src.glob("probe_*.log"))
    copied = []
    for n in names:
        p = src / n
        if p.is_file():
            shutil.copy2(p, out / n)
            copied.append(n)
    shots_src = src / "shots"
    if shots_src.is_dir():
        if (out / "shots").is_dir():
            shutil.rmtree(out / "shots")
        shutil.copytree(shots_src, out / "shots")
    for a, b in (("env.json", "source_env.json"), ("verdict.md", "source_verdict.md"),
                 ("verdict.json", "source_verdict.json")):
        if (src / a).is_file():
            shutil.copy2(src / a, out / b)
    manifest, lines = {}, []
    for p in sorted(src.rglob("*")):
        if not p.is_file():
            continue
        rel = str(p.relative_to(src))
        h = sha256_file(p)
        manifest[rel] = {"sha256": h, "size": p.stat().st_size}
        lines.append("%s  %s" % (h, rel))
    (out / "source_manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True), encoding="utf-8")
    (out / "source_manifest.sha256").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return {"copied": copied, "files": len(manifest), "manifest": manifest}


def write_regrade_tail(ctx, src, old, diffs):
    """在 `verdict.md` 末尾追加 §11（差异）/ §12（prev_issues 处置）/ §13（等价性）。"""
    pfb = person_frame_breakdown(ctx)
    judges = getattr(ctx, "regrade_judges", {}) or {}
    L = []
    A = L.append
    A("\n## 11 与取证轮的差异（逐条）\n")
    rows = []
    for d in diffs:
        rows.append([d["id"], d["old"], d["new"], ", ".join(d["fields"]),
                     "判据/口径修正" if "verdict" in d["fields"] else "实测文案变化",
                     (d.get("new_measured") or "")[:110]])
    A(_table(["判据", "取证轮", "本轮", "变化字段", "性质", "本轮实测"], rows) if rows
      else "两次判据表**完全一致**（0 条变化）。\n")
    n_same_v = sum(1 for i in JUDGE_ORDER
                   if ((old.get(i) or {}).get("verdict")
                       == (judges.get(i) or {}).get("verdict")))
    n_same_m = sum(1 for i in JUDGE_ORDER
                   if ((old.get(i) or {}).get("measured")
                       == (judges.get(i) or {}).get("measured")))
    A("\n共 %d 行判据：其中 **%d/%d 行 verdict 与取证轮逐字节相同**、"
      "**%d/%d 行 measured 逐字节相同**（差异全部落在上表，且都是判据/口层修正，"
      "采集端一行未改）。\n" % (len(JUDGE_ORDER), n_same_v, len(JUDGE_ORDER),
                              n_same_m, len(JUDGE_ORDER)))
    A("\n## 12 本轮 `prev_issues.md` 处置 + 自查\n")
    try:
        pi_raw = (ROOT / "tmp" / "task39" / "prev_issues.md").read_text(
            encoding="utf-8", errors="replace").strip()
        pi_sha = sha256_file(ROOT / "tmp" / "task39" / "prev_issues.md")[:32]
    except OSError as exc:
        pi_raw, pi_sha = "（读不到：%r）" % (exc,), "—"
    A("`tmp/task39/prev_issues.md`（sha256 `%s…`）原文：\n\n```\n%s\n```\n" % (
        pi_sha, pi_raw or "(空)"))
    # 当轮 `[flight_ctrl]`（优先读产物目录里那一份 = 取证轮实际生效的配置）
    fc_run = None
    try:
        import tomllib                                  # noqa: PLC0415  本机 py3.11+
        with (pathlib.Path(ctx.out) / "config.toml").open("rb") as fh:
            fc_run = flight_ctrl_cfg(tomllib.load(fh))
    except (OSError, ImportError, ValueError):
        try:
            fc_run = flight_ctrl_cfg(FX.load_config())
        except Exception:                               # noqa: BLE001  读不到就照实写
            fc_run = None
    cov_all = uav_coverage(ctx, 0)
    cov_full = cov_all[1] > 0 and cov_all[0] == cov_all[1]
    n_have = sum(1 for i in JUDGE_ORDER if i in (judges or {}))
    src_name = pathlib.Path(src).name
    rows = [
        ["`[Sim] UAV` 行解析覆盖率", "%d/%d" % cov_all,
         "OK" if cov_full else "**不足 ⇒ 相关判据 FAIL(env)**"],
        ["`[flight_ctrl]`（机栈速度唯一来源）",
         "vel_fwd=%s m/s、yaw_rate_deg=%s °/s" % (
             fmt((fc_run or {}).get("vel_fwd"), 2), fmt((fc_run or {}).get("yaw_rate_deg"), 1)),
         "OK" if (fc_run or {}).get("vel_fwd") is not None else "**读不到**"],
        ["每拍转角 = `yaw_rate_deg` × 位姿龄中位",
         "%s°（位姿龄中位 %s ms）" % (
             fmt(turn_zone_text(_age_median_of(ctx), fc_run)["deg"], 1),
             fmt(_age_median_of(ctx) * 1000.0, 0)),
         turn_zone_text(_age_median_of(ctx), fc_run)["band"]],
        ["28 行判据齐全", "%d/%d" % (n_have, len(JUDGE_ORDER)),
         "OK" if n_have == len(JUDGE_ORDER) else "**缺 %d 行**" % (len(JUDGE_ORDER) - n_have)],
        ["与源目录（`%s`）的判据差异" % src_name, "%d 条（%s）" % (
            len(diffs), "、".join(d["id"] for d in diffs) or "无"),
         "OK（当轮复算须 0 条）" if not diffs else
         ("LEGACY 目录：只登记差异，不判错" if src_name in REGRADE_LEGACY
          else "**当轮复算不为 0 ⇒ 判据不确定**")],
    ]
    A(_table(["自查项", "实测", "判定"], rows))
    A("\n## 13 复算方法与等价性\n")
    files = ["trace.csv", "detector_frames.csv", "sim_trace.log"]
    rows = [[n, sha256_file(pathlib.Path(src) / n)[:32] + "…"] for n in files]
    A(_table(["文件", "sha256"], rows))
    A("\n- 源目录：`%s`（只读；本轮前后 sha256 清单见 `source_manifest.sha256`，"
      "`sha256sum -c` 应全 OK）；新目录：`%s`。" % (src, ctx.out))
    A("- 帧集合：`sim_trace.log` 按 `detector_frames.csv` 的 `seq` 过滤 ⇒ **%d 帧**，"
      "与取证轮判据时刻的快照一致。" % len(ctx.frames or []))
    A("- 等价性：**%d/%d 行 verdict 相同**、**%d/%d 行 measured 相同**；变化的只有 %s。"
      "契约：源目录 ∈ `REGRADE_LEGACY`（%d 个历史目录）时只登记差异；否则"
      "`judge_diff` 必须为空、`REGRADE_PIN` 若已填须逐项相等（当前 %s）。" % (
          n_same_v, len(JUDGE_ORDER), n_same_m, len(JUDGE_ORDER),
          "、".join(d["id"] for d in diffs) or "无", len(REGRADE_LEGACY),
          "已填" if REGRADE_PIN else "未填（首次复算）"))
    A("- **`--regrade` 的用途**：证明 §3 的 28 行是**判据对同一批产物的确定性输出**，"
      "而不是手改过的表——采集端（世界/sim/行人客户端）一个字没改，"
      "改的只有解析（`[Sim] UAV` 的 `f32` 小数）与报告层文案。")
    A("- **为什么不重跑世界/仿真**：① 复算的输入是取证轮已落盘的 `trace.csv` / "
      "`detector_frames.csv` / `sim_trace.log`，重跑只会换一批环境噪声；"
      "② 重跑无法证明「同一批产物算两次结果相同」，而复算能；"
      "③ 复算不产生新的世界/仿真状态，避免把环境抖动记到被测代码头上。"
      "正式取证轮仍是一次真实世界 + sim + 行人的完整跑（见 §2/§9）。")
    A("- 边界自查：本轮未起世界/sim/行人客户端；源目录零写入（`source_manifest.sha256`）；"
      "`git status` 与基线之差仍只有本脚本一行。")
    with (ctx.out / "verdict.md").open("a", encoding="utf-8") as f:
        f.write("\n".join(L) + "\n")
    return L


def _age_median_of(ctx):
    """§5 用的位姿龄中位（第一个 goto 用例的 `[Sim] UAV` 行）。"""
    first = next((ctx.cases_done[k] for k in ("m1", "m2", "m3")
                  if k in (ctx.cases_done or {})), None)
    return (((first or {}).get("analysis") or {}).get("pose_age_ms_median") or 0) / 1000.0


def _mv(judges, ident, key):
    """从某条判据的实测串里取 `key=数值`（供根因叙述引用，不重复写死实测值）。"""
    text = ((judges or {}).get(ident) or {}).get("measured") or ""
    m = re.search(r"%s=(-?[\d.]+)" % re.escape(key), text)
    return None if m is None else float(m.group(1))


def extra_observations(ctx, judges):
    """附加观测（**不进 §3 判据**）：回答「跟住没有」而不是「判据绿没绿」。

    `§五` 的 F1-2/F2-3 与场景几何不自洽（机速上限 < 人速；切线通过时机的净位移方向必然
    由「指向人」的方位决定），这两条不是代码缺陷、也不在本轮权限内改，所以另给一组
    可直接核对的量：初/末距离、`+y` 投影 `dy`、机速 vs 人速的算术差值、M2 的路径形状。
    """
    out = []
    try:
        fc = flight_ctrl_cfg(FX.load_config()) or {}
    except Exception:                                   # noqa: BLE001  读不到就写「—」
        fc = {}
    v_fwd = fc.get("vel_fwd")
    plan = (("F1", "f1", PED_F1_SPEED, 20.0, PED_F1_SPAWN),
            ("F2", "f2", PED_F2_SPEED, 13.0, PED_F2_SPAWN))
    for tag, key, ped_v, dur, spawn in plan:
        rec = (ctx.cases_done or {}).get(key)
        if not rec:
            continue
        rows = seg_rows(ctx, tag)
        tw0, tw1 = rec.get("t_world_start"), rec.get("t_world_end")
        q0 = pose_at(rows, tw0, keys=("x", "y", "d_m")) if tw0 is not None else None
        q1 = pose_at(rows, tw1, keys=("x", "y", "d_m")) if tw1 is not None else None
        d10 = disp(rows, tw0, tw0 + 10.0) if tw0 is not None else None
        st = rec.get("start") or {}
        try:
            d_init = dist((float(st.get("x")), float(st.get("y"))),
                          (spawn["x"], spawn["y"]))
        except (TypeError, ValueError):
            d_init = None
        gap = None if v_fwd is None else round(ped_v - v_fwd, 3)
        theory = None if (d_init is None or gap is None) else round(d_init + gap * dur, 2)
        out.append(
            "- **附加观测 %s（不进 §3）**：机速上限 `vel_fwd=%s m/s` vs 人速 `%s m/s`"
            "（人速−机速=%s m/s）；起点间距 %s m ⇒ %.0f 物理 s 后理论间距 ≈ **%s m**%s；"
            "实测距离 %s m → %s m（末距离口径=truth）；10 s 位移 `dy`=%s m、`dx`=%s m%s。" % (
                tag, fmt(v_fwd, 2), fmt(ped_v, 2), fmt(gap, 2), fmt(d_init, 2), dur,
                fmt(theory, 2),
                "（`F1-2` 要求末距离 ≤%.1f m ⇒ 该判据在几何上不可能过）" % F1_D_END_MAX
                if tag == "F1" else "",
                fmt((q0 or {}).get("d_m"), 2),
                fmt((q1 or {}).get("d_m"), 2),
                fmt(None if d10 is None else d10["dy"], 2),
                fmt(None if d10 is None else d10["dx"], 2),
                "" if tag != "F2" else
                "（`F2-3` 要求 cos(位移,+y)≥%.3f；人沿 +y 走、机从侧向切入 ⇒ "
                "净位移方向由「指向人」的方位决定，判据与切向几何不自洽）" % DIR_COS_MIN))
    m2 = (ctx.cases_done or {}).get("m2")
    if m2:
        an = m2.get("analysis") or {}
        paths = an.get("paths") or {}
        rows2 = seg_rows(ctx, "M2")
        ts = [r["t_world"] for r in rows2 if r.get("t_world") is not None]
        worst = None
        if ts:
            t = min(ts) + 1.0
            while t <= max(ts):
                a = pose_at(rows2, t - 1.0, keys=("yaw",))
                b = pose_at(rows2, t, keys=("yaw",))
                if (a and b and a.get("yaw") is not None and b.get("yaw") is not None):
                    d = abs(wrap_deg(math.degrees(b["yaw"] - a["yaw"])))
                    worst = d if worst is None else max(worst, d)
                t += 1.0
        span = an.get("yaw_span")
        shape = ("yaw 跨度 %s° ⇒ **出现过一次真转向**（拐角语义成立）" % fmt(span, 1)
                 if (span or 0.0) >= 60.0 else
                 "yaw 跨度仅 %s° ⇒ 无 ≥60° 转向，`L9b` 的「拐角」语义**退化**"
                 "（8 连通下 `(64.25,64.25)→(84.25,84.25)` 可能被走成对角直线）" % fmt(span, 1))
        out.append(
            "- **附加观测 M2 路径形状（不进 §3）**：路径长/直线=%s、逐秒最大 `|Δyaw|`=%s°；"
            "%s。真 L 形需要障碍或两段 goto（属下一步），本轮**不改用例、不改判据**。" % (
                fmt(paths.get("ratio"), 3), fmt(worst, 1), shape))
    out.append(
        "- **附加观测 `F3_DRIFT_MAX`（不进 §3）**：门值 %.2f m 是按旧满速推的；新满速 "
        "`vel_fwd=%s m/s`、`MAX_ACCEL=3.0` ⇒ `v²/2a`=%s m ⇒ 该门比设计意图**宽**。"
        "保留不动（改了就是调阈值），只记明。" % (
            F3_DRIFT_MAX, fmt(v_fwd, 2),
            fmt(None if v_fwd is None else v_fwd ** 2 / 2.0 / 3.0, 3)))
    _ = judges                      # 参数留给后续按判据编号引用实测（本轮不消费）
    return out


def regrade_run(args):
    """`--regrade DIR`：对已有产物目录做 28 条判据复算（**不启动世界/仿真，源目录零写入**）。"""
    src = pathlib.Path(args.regrade).resolve()
    if not src.is_dir():
        print("[FAIL(env)] --regrade 目录不存在：%s" % src)
        return 2
    if (src / "source_manifest.json").exists():
        print("[FAIL(env)] %s 本身是复算目录（含 source_manifest.json）"
              "→ 请对取证轮目录复算" % src)
        return 2
    out = pathlib.Path(args.out_dir).resolve() if args.out_dir else \
        (RESULTS / ("task39_" + now_utc_stamp() + "_regrade"))
    if out == src:
        print("[FAIL(env)] --out-dir 不能等于 --regrade（会毁掉取证轮证据）")
        return 2
    out.mkdir(parents=True, exist_ok=True)
    tee = Tee(str(out / "script.log"), sys.stdout)
    sys.stdout = tee
    print("[out] %s" % out)
    print("[regrade] 源目录 %s（只读）" % src)
    problems = []
    judges, old = {}, {}
    diffs = []
    pre, s0 = [], None
    ctx = None
    overall, reason = "FAIL(env)", ""
    try:
        try:
            env = json.loads((src / "env.json").read_text(encoding="utf-8"))
        except OSError as exc:
            raise FX.HarnessError("读不了 %s/env.json：%r" % (src, exc))
        # ── 硬门：二进制与复用脚本与取证轮逐字节一致
        try:
            sh_sim1 = sha256_file(INST / "pleiades-sim")
            sh_tgt = sha256_file(SimBinary)
            sh_reuse = sha256_file(REUSE_PATH)
        except OSError as exc:
            problems.append("二进制/复用脚本读不了：%r" % (exc,))
            sh_sim1 = sh_tgt = sh_reuse = None
        if sh_sim1 is not None:
            if not (sh_sim1 == env.get("sim_binary_sha256") == sh_tgt):
                problems.append(
                    "硬门 sim 二进制不一致：sim1=%s… 取证轮=%s… target=%s…" % (
                        sh_sim1[:16], (env.get("sim_binary_sha256") or "")[:16],
                        sh_tgt[:16]))
        if sh_reuse is not None and sh_reuse != REUSE_SHA256:
            problems.append("硬门 `Tool/sim_follow_test.py` sha256=%s… ≠ 钉死值 %s…" % (
                sh_reuse[:16], REUSE_SHA256[:16]))
        # ── 软门：git HEAD 与取证轮比对（取不到只记一句，不算失败）
        head_now = FX.run_capture(["git", "-C", str(ROOT), "rev-parse", "--short",
                                   "HEAD"])[1].strip()
        head_old = ""
        try:
            m = re.search(r"\|\s*git HEAD\s*\|\s*([0-9a-fA-F]+)\s*\|",
                          (src / "verdict.md").read_text(encoding="utf-8", errors="replace"))
            head_old = m.group(1) if m else ""
        except OSError:
            head_old = ""
        print("[regrade] git HEAD 本轮=%s 取证轮=%s（软门）" % (
            head_now, head_old or "未取到（跳过）"))
        if head_old and head_old != head_now:
            print("[warn] HEAD 与取证轮不同（软门，不阻断）：%s ≠ %s" % (head_old, head_now))

        ctx = load_artifacts_ctx(src, out)
        pre = env.get("preflight") or []
        s0 = env.get("s0")
        old = env.get("judges") or {}
        man = copy_evidence(src, out)
        print("[regrade] 复制证据 %d 个文件（manifest 见 source_manifest.json）" % man["files"])
        if CARGO_LOG.is_file():
            shutil.copy2(CARGO_LOG, out / "cargo_test.log")
        judges = fill_missing_judges(ctx, judge_all(ctx))
        ctx.regrade_judges = judges
        diffs = judge_diff(old, judges)
        # ── 等价性断言（不满足 ⇒ FAIL(env)，但仍照写 verdict，不许手改掩盖）
        problems += regrade_equiv(ctx, old, judges, diffs)
        overall, reason = overall_of(judges)
        if problems:
            reason = "复算门/等价性不满足：%s（判据表原样保留）" % "；".join(problems)
            overall = "FAIL(env)"
        ctx.residual = residual()
        ctx.residual["git_status_after"] = FX.git_status_lines()
        write_json_artifacts(ctx, judges, pre, s0, overall, reason)
        write_verdict(ctx, judges, pre, s0, overall, reason,
                      env_fail=(overall == "FAIL(env)"))
        write_regrade_tail(ctx, src, old, diffs)
    except Exception as exc:
        traceback.print_exc()
        overall, reason = "FAIL(env)", "复算异常：%s: %s" % (type(exc).__name__, exc)
    finally:
        if ctx is not None:
            try:
                write_json_artifacts(ctx, judges, pre, s0, overall, reason)
                if not (out / "verdict.md").exists():
                    write_verdict(ctx, judges, pre, s0, overall, reason, env_fail=True)
            except Exception:
                traceback.print_exc()
        sys.stdout.flush()
        sys.stdout = tee.stream
        tee.close()
    print("=" * 92)
    for ident in JUDGE_ORDER:
        j = judges.get(ident)
        if j:
            print("  %-5s %-10s %s" % (ident, j["verdict"], (j["measured"] or "")[:120]))
    print("  结论：%s — %s" % (overall, reason))
    print("  产物：%s" % out)
    print("=" * 92)
    if overall == "PASS":
        return 0
    return 2 if overall == "FAIL(env)" else 1


def regrade_equiv(ctx, old, judges, diffs):
    """复算等价性断言（**不再钉上一轮的数字**，见 `REGRADE_LEGACY`/`REGRADE_PIN`）。

    ① 28 行齐全；② 源目录 ∈ `REGRADE_LEGACY` ⇒ 差异只进 §11，不计 problem；
    ③ 否则（当轮/自洽）⇒ `judge_diff` 必须为空 + `REGRADE_PIN` 若已填则逐项比对；
    ④ 源目录的 `sim_trace.log` 可读 ⇒ `uav_coverage` 覆盖率必须 100%（防旧解析重演）。
    """
    probs = []
    cnt = {"PASS": 0, "FAIL": 0, "FAIL(env)": 0, "N-A": 0}
    for j in judges.values():
        cnt[j.get("verdict")] = cnt.get(j.get("verdict"), 0) + 1
    cnt["total"] = len(judges)
    src_name = pathlib.Path(getattr(ctx, "regrade_src", "") or "").name
    legacy = src_name in REGRADE_LEGACY
    ids = [i for i in JUDGE_ORDER if i not in judges]
    if ids:
        probs.append("判据行数 %d/%d（缺 %s）" % (len(judges), len(JUDGE_ORDER),
                                                 ", ".join(ids)))
    # ④ 解析覆盖率（源目录可读才算）
    try:
        hist = (pathlib.Path(ctx.regrade_src) / "sim_trace.log").read_text(
            encoding="utf-8", errors="replace").splitlines(True)
    except OSError:
        hist = []
    if hist:
        total = sum(1 for ln in hist if "[Sim] UAV" in ln)
        ok_n = sum(1 for ln in hist if UAV_RE.search(ln))
        if total and ok_n != total:
            probs.append("`[Sim] UAV` 解析覆盖率 %d/%d（<100%% ⇒ 分辨率不足）" % (ok_n, total))
        print("[regrade] `[Sim] UAV` 行解析 %d/%d" % (ok_n, total))
    else:
        print("[regrade] 源目录没有 sim_trace.log ⇒ 跳过 `[Sim] UAV` 覆盖率门")
    if legacy:
        print("[regrade] 源目录 %s ∈ `REGRADE_LEGACY`（历史轮次）⇒ 差异只写 §11，"
              "不计 problem；差异 %d 条（%s）" % (
                  src_name, len(diffs), ", ".join(d["id"] for d in diffs) or "无"))
    else:
        if diffs:
            probs.append("当轮复算 diff=%d 条（%s）⇒ 判据不确定或被手改" % (
                len(diffs), ", ".join(d["id"] for d in diffs)))
        if REGRADE_PIN:
            for key in ("frames", "counts", "fail_ids"):
                got = (len(ctx.frames or []) if key == "frames"
                       else cnt if key == "counts"
                       else [i for i in JUDGE_ORDER
                             if (judges.get(i) or {}).get("verdict") == "FAIL"])
                if REGRADE_PIN.get(key) != got:
                    probs.append("`REGRADE_PIN[%s]`=%r ≠ 实测 %r" % (
                        key, REGRADE_PIN.get(key), got))
        else:
            print("[regrade] `REGRADE_PIN` 未填 ⇒ 跳过钉死值比对（首次复算，填完须 0 diff）")
    print("[regrade] 判据 %d 条：PASS %d / FAIL %d / FAIL(env) %d / N-A %d；"
          "差异 %d 条（%s）" % (cnt["total"], cnt["PASS"], cnt["FAIL"], cnt["FAIL(env)"],
                              cnt["N-A"], len(diffs),
                              ", ".join(d["id"] for d in diffs) or "无"))
    for p in probs:
        print("[regrade][等价性] %s" % p)
    return probs


def preflight(cfg, port):
    out = []

    def add(name, ok, detail):
        out.append(crit(name, ok, detail))

    busy = [p for p in (port, WORLD_PORT) if FX.port_busy(p)]
    add("ports_free", not busy,
        "%d/%d 空闲" % (port, WORLD_PORT) if not busy else "被占用：%s" % (busy,))

    cam = FX.find_camera_sensor(cfg) or {}
    cp, cm = cam.get("params", {}), cam.get("mount", {})
    add("config_camera_sensor",
        cam.get("slot") == "camera_nose" and cam.get("sensor") == "camera"
        and _eq(cp.get("rate_hz", -1), 10.0) and _eq(cm.get("pitch", 0), -15.0),
        "slot=%r sensor=%r rate_hz=%r mount.pitch=%r" % (
            cam.get("slot"), cam.get("sensor"), cp.get("rate_hz"), cm.get("pitch")))

    det = cfg.get("detector", {})
    add("config_detector",
        det.get("enabled") is True and det.get("device") == "cpu"
        and det.get("model") == "yolov8n.safetensors"
        and _eq(det.get("conf_thres", -1), 0.35) and _eq(det.get("nms_thres", -1), 0.45),
        "enabled=%r device=%r model=%r conf=%r nms=%r" % (
            det.get("enabled"), det.get("device"), det.get("model"),
            det.get("conf_thres"), det.get("nms_thres")))

    dc = det.get("camera", {})
    want = {"fov_deg": 70.0, "mount_x": 0.20, "mount_y": 0.0, "mount_z": 0.045,
            "mount_yaw_deg": 90.0, "mount_pitch_deg": -15.0, "ground_z": 0.0,
            "max_range_m": 12.0}
    ok_dc = all(_eq(dc.get(k, 1e9), v) for k, v in want.items()) and \
        dc.get("fov_axis") == "vertical"
    add("config_detector_camera", ok_dc,
        " ".join("%s=%r" % (k, dc.get(k)) for k in want) + " fov_axis=%r" % (dc.get("fov_axis"),))

    gear = cfg.get("chassis", {})
    add("config_chassis_gear",
        int(gear.get("forward_speed", -1)) == 100 and int(gear.get("turn_speed", -1)) >= 30,
        "forward_speed=%r turn_speed=%r（机栈已不读，仅 `manual` 遥控/车栈用）" % (
            gear.get("forward_speed"), gear.get("turn_speed")))

    fc = flight_ctrl_cfg(cfg)
    add("config_flight_ctrl",
        fc["vel_fwd"] is not None and fc["yaw_rate_deg"] is not None
        and _eq(fc["vel_fwd"], 0.8, 1e-3) and _eq(fc["yaw_rate_deg"], 30.0, 1e-3),
        "`[flight_ctrl]` vel_fwd=%r m/s yaw_rate_deg=%r °/s（期望 0.8 / 30.0；"
        "机栈速度的唯一来源）" % (fc["vel_fwd"], fc["yaw_rate_deg"]))

    ctl = cfg.get("control", {})
    add("config_control_port",
        int(ctl.get("port", -1)) == port and ctl.get("bind") == "127.0.0.1",
        "port=%r bind=%r（期望 %d/127.0.0.1）" % (ctl.get("port"), ctl.get("bind"), port))

    sw = cfg.get("sim_world", {})
    idn = cfg.get("Identity", {})
    add("config_stack_kind",
        sw.get("chassis") == "quadrotor_uav" and idn.get("node_type") == "uav",
        "[sim_world].chassis=%r [Identity].node_type=%r" % (
            sw.get("chassis"), idn.get("node_type")))

    weights = INST / "Pleiades_Workspace" / "yolov8n.safetensors"
    add("detector_weights",
        weights.is_file() and weights.stat().st_size > 1 << 20,
        "%s bytes=%s" % (weights.relative_to(ROOT),
                         weights.stat().st_size if weights.is_file() else "缺失"))

    prog = INST / "programs"
    add("programs_present", prog.exists(),
        "存在（%s）→ 不新建" % ("软链 → %s" % os.readlink(prog) if prog.is_symlink() else "实目录")
        if prog.exists() else "缺失（需 ln -s ../../programs）")

    rc, so, _ = FX.run_capture(["godot", "--version"], timeout=60)
    add("godot_version", rc == 0, "rc=%d %s" % (rc, so.strip()[:60]))

    missing = [t for t in ("Xvfb", "import", "convert", "identify")
               if shutil.which(t) is None]
    add("tools_imagemagick_xvfb", not missing,
        "全部在 PATH" if not missing else "缺：%s" % (missing,))

    imported = FX.WORLD_DIR / ".godot" / "imported"
    n_imp = len(list(imported.iterdir())) if imported.is_dir() else 0
    add("world_import_cache", n_imp > 0,
        "%s 文件数=%d" % (imported.relative_to(ROOT), n_imp))

    sh_bin, sh_rel = None, None
    try:
        sh_bin = sha256_file(INST / "pleiades-sim")
        sh_rel = sha256_file(SimBinary)
    except OSError as exc:
        add("sim_binary_fresh", False, "读不了二进制：%r" % (exc,))
    else:
        add("sim_binary_fresh", sh_bin == sh_rel,
            "sim1=%s… target=%s… %s" % (sh_bin[:16], sh_rel[:16],
                                        "相同" if sh_bin == sh_rel else "**不同（需 cp）**"))

    consts = []
    fr_path = ROOT / "pleiades-sim" / "src" / "sim" / "uav_sim" / "mission" / "follow.rs"
    ex_path = ROOT / "pleiades-sim" / "src" / "sim" / "uav_sim" / "executor.rs"
    for path, pats in ((fr_path, [r"D_FAR_M: f32 = 4\.0", r"D_NEAR_M: f32 = 2\.5",
                                 r"STALE_MS: u64 = 800"]),
                       (ex_path, [r"DEADBAND_DEG: f32 = 5\.0", r"SPIN_DEG: f32 = 20\.0"])):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError as exc:
            consts.append("%s 读不了" % path.name)
            continue
        for p in pats:
            if not re.search(p, text):
                consts.append("%s 缺 %s" % (path.name, p))
    add("source_consts", not consts,
        "follow.rs 3 常量 + executor.rs 2 常量齐全" if not consts else "; ".join(consts))

    live = set(FX.git_status_lines())
    try:
        base = set(ln.rstrip() for ln in BASELINE.read_text(
            encoding="utf-8", errors="replace").splitlines() if ln.strip())
    except OSError as exc:
        add("git_boundary", False, "基线读不了：%r" % (exc,))
    else:
        extra = sorted(live - base - BOUNDARY_ALLOW)
        missing_l = sorted(base - live)
        add("git_boundary", not extra,
            "多出 %d 行 / 缺 %d 行%s" % (
                len(extra), len(missing_l),
                "" if not extra else "；越界：" + "; ".join(extra[:5])))

    try:
        sha = sha256_file(REUSE_PATH)
    except OSError as exc:
        sha = "<读不了 %r>" % (exc,)
    add("reuse_pin",
        sha == REUSE_SHA256 and getattr(FX, "PedWalker", None) is not None,
        "sim_follow_test.py sha256=%s…%s；import 成功" % (
            sha[:16], "" if sha == REUSE_SHA256 else "（**与钉死值不符**）"))

    # ── 17 副本存在（含 .godot 导入缓存）
    need = ["project.godot", "override.cfg", "src", "assets", "tool", ".godot"]
    miss = [n for n in need if not (WORLD_FAST / n).exists()]
    imp_fast = WORLD_FAST / ".godot" / "imported"
    imp_src = WORLD_SRC / ".godot" / "imported"
    n_fast = len(list(imp_fast.iterdir())) if imp_fast.is_dir() else 0
    n_src = len(list(imp_src.iterdir())) if imp_src.is_dir() else 0
    add("world_fast_present", (not miss) and n_fast > 0 and n_fast == n_src,
        "%s 子项缺 %s；`.godot/imported` 副本=%d 源=%d" % (
            WORLD_FAST.relative_to(ROOT), miss or "无", n_fast, n_src))

    # ── 18 override.cfg 只能有 MSAA 一个键
    try:
        ovr = (WORLD_FAST / "override.cfg").read_text(encoding="utf-8", errors="replace")
        ovr_lines = [ln.strip() for ln in ovr.splitlines() if ln.strip()]
    except OSError as exc:
        ovr_lines, ovr = ["<读不了 %r>" % (exc,)], ""
    add("world_override_only_msaa",
        ovr_lines == ["[rendering]", "anti_aliasing/quality/msaa_3d=0"],
        "override.cfg=%r" % (ovr_lines,))

    # ── 19 副本与原世界逐字节相同（除 override.cfg / .godot）
    rc_d, so_d, se_d = FX.run_capture(
        ["diff", "-r", str(WORLD_SRC), str(WORLD_FAST),
         "--exclude=override.cfg", "--exclude=.godot"], timeout=300)
    try:
        same_proj = sha256_file(WORLD_SRC / "project.godot") == \
            sha256_file(WORLD_FAST / "project.godot")
    except OSError:
        same_proj = False
    add("world_copy_identical", rc_d == 0 and not so_d.strip() and same_proj,
        "diff rc=%d 输出 %d 行%s；project.godot sha 相同=%s" % (
            rc_d, len(so_d.splitlines()), "" if not so_d.strip() else "：%s" % so_d[:200],
            same_proj))

    # ── 20 副本新鲜度（源树被改过就自动重拷）
    want_stamp = world_stamp(WORLD_SRC)
    try:
        have_stamp = world_marker_path().read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        have_stamp = ""
    add("world_rt_refresh", have_stamp == want_stamp.strip(),
        "标记一致" if have_stamp == want_stamp.strip()
        else "不一致（源树已变）→ ensure_world_fast 应已重拷")

    # ── 21 harness 自检（死代码 / 禁用标识符）
    try:
        lint = harness_lint()
        add("harness_lint", lint["ok"],
            "模块级 def %d 个；未调用=%s；禁用名命中=%s" % (
                lint["defs"], lint["dead_defs"] or "无", lint["hits"] or "无"))
    except (OSError, SyntaxError) as exc:
        add("harness_lint", False, "AST 扫描失败：%r" % (exc,))

    # ── 23 `[Sim] UAV` 行解析自检（f32 Display 小数，样本逐字取自 `sim_handler.rs` 格式串）
    try:
        sel = uav_re_selftest()
        add("uav_log_formats", sel["ok"],
            "%d 条样本全解析" % sel["n"] if sel["ok"] else "；".join(sel["bad"]))
    except Exception as exc:                        # noqa: BLE001  自检失败只记账
        add("uav_log_formats", False, "自检异常：%r" % (exc,))
    return out


# ================================================================ S0 锚点

S0_ANCHORS = [
    ("camera_rate_hz", r"\[camera\] camera_sim 使用 slot=camera_nose rate_hz=(\d+)", 30, True),
    ("detector_start",
     r"\[detector\] 已启动：device=cpu model=yolov8n\.safetensors conf=0\.35 nms=0\.45", 30, True),
    ("detector_projection",
     r"\[detector\] 投影参数：fov=70\.0°\(vertical\).*pitch=-15\.0°.*max_range=12\.0", 30, True),
    ("camera_first_frame", r"\[camera\] slot=camera_nose 首帧 640x480", 60, True),
    ("sim_world_chassis_ready",
     r"\[world\] chassis_ready avatar=(\d+) spawn=\((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)", 30, True),
    ("world_vehicle_spawn",
     r"\[ws\] vehicle spawned .*avatar=(\d+) slots=\d+ .*spawn=\((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)",
     30, False),
    ("chassis_build_gear",
     r"\[chassis\] 构建底盘 name=(\S+) forward_speed=(\d+) turn_speed=(\d+)", 30, True),
    ("stack_uav", r"\[Sim\] 任务栈 = Uav（底盘 quadrotor_uav）", 30, True),
    ("nav_plan_budget", r"\[Nav\] 规划时间预算 = 20ms", 30, True),
    ("no_lidar_estop_skipped", r"\[Sim\] 无 lidar（未配置 lidar_2d）：on_tick 跳过急停", 30, True),
    ("control_listen", r"\[control\] 监听 ws://127\.0\.0\.1:(\d+)", 30, True),
]

S0_STATUS = [
    ("spawn_x", lambda st: abs(float(st.get("x", 1e9)) - UAV_SPAWN[0]) <= 0.15),
    ("spawn_y", lambda st: abs(float(st.get("y", 1e9)) - UAV_SPAWN[1]) <= 0.15),
    ("altitude", lambda st: 2.30 <= float(st.get("z", 0.0)) <= 2.70),
    ("yaw_zero", lambda st: abs(float(st.get("yaw", 1e9))) <= 0.05),
    ("assembled", lambda st: st.get("assembled") is True),
    ("avatar_1", lambda st: st.get("avatar_id") == 1),
    ("mode_auto", lambda st: st.get("mode") == "auto"),
    ("nav_idle", lambda st: st.get("nav_status") == "idle"),
    ("queue_empty", lambda st: st.get("queue_len") == 0),
]


def assert_s0(ctx):
    """11 条日志锚点 + 9 条 status 断言（§五 起跑环境），任一不过 → HarnessError。"""
    res = {"anchors": [], "status": {}, "asserts": [], "ok": True}
    for name, pattern, timeout, is_sim in S0_ANCHORS:
        t0 = time.monotonic()
        m, line = (ctx.wait_sim(pattern, timeout) if is_sim
                   else ctx.wait_world(pattern, timeout))
        ctx.clock.sync(ctx)
        res["anchors"].append({"name": name, "ok": m is not None, "line": line or "",
                               "seconds": round(time.monotonic() - t0, 2)})
        Log("  S0 锚点 %-24s %-8s %s" % (name, "OK" if m is not None else "MISSING",
                                         (line or "")[:110]))
        if m is None:
            res["ok"] = False

    for name, ok in (("chassis_forward_speed_100",
                      re.search(r"forward_speed=100", res["anchors"][6]["line"] or "") is not None),
                     ("world_spawn_64_64_2p5",
                      "(64.00, 64.00, 2.50)" in (res["anchors"][5]["line"] or ""))):
        # `chassis_forward_speed_100` 仍然保留：`[chassis]` 只服务 `manual` 遥控/车栈，
        # 机栈速度改由 `[flight_ctrl]`（preflight `config_flight_ctrl`）提供。
        res["asserts"].append(crit(name, ok))

    st = FX.status_assembled(ctx, 120)
    res["status"] = {k: st.get(k) for k in ("x", "y", "z", "yaw", "mode", "nav_status",
                                            "queue_len", "assembled", "avatar_id", "task_id")}
    for name, fn in S0_STATUS:
        try:
            ok = bool(fn(st))
        except (TypeError, ValueError):
            ok = False
        res["asserts"].append(crit(name, ok, "st=%r" % (res["status"].get(name.split("_")[0]),)))
        if not ok:
            res["ok"] = False
    for c in res["asserts"]:
        if not c["ok"]:
            res["ok"] = False
    Log("S0 status: %s" % json.dumps(res["status"], ensure_ascii=False))
    if not res["ok"]:
        raise FX.HarnessError("S0 断言失败（见 verdict §2）")
    return res


def pose_ages(ctx):
    """`[Sim] UAV` 行里的 `位姿龄=Nms` 序列（环境实时性 / 控制环新鲜度）。"""
    out = []
    for line in ctx.sim_history:
        m = UAV_RE.search(line)
        if m:
            out.append(int(m.group(9)))
    return out


def latest_pose_age(ctx):
    """增量版：只扫新到的日志行，返回最近一条 `[Sim] UAV` 的位姿龄（ms）。"""
    hist = ctx.sim_history
    i = getattr(ctx, "_age_scan_i", 0)
    best = getattr(ctx, "_age_last", None)
    while i < len(hist):
        m = UAV_RE.search(hist[i])
        if m:
            best = int(m.group(9))
        i += 1
    ctx._age_scan_i = i
    ctx._age_last = best
    return best


def measure_env(ctx, seconds=10.0):
    """10 物理 s 窗口：相机实测 Hz、帧间隔中位、空场 person 帧数（必须 0）、k、位姿龄。"""
    ctx.seg = "S0j"
    t_wait = time.monotonic()
    while not ctx.clock.pts and time.monotonic() - t_wait < 10.0:
        ctx.read_sim()
        ctx.clock.sync(ctx)
        time.sleep(0.10)
    if not ctx.clock.pts:
        raise FX.HarnessError("10 s 内没有 detector 帧入钟（相机/detector 没跑起来）")
    w0 = ctx.clock.at(time.monotonic())
    t0 = time.monotonic()
    f0 = len(ctx.frames)
    n0 = len(ctx.rows)
    sample_phys(ctx, seconds, seg="S0j")
    frames = ctx.frames[f0:]
    wall_d = time.monotonic() - t0
    ctx.clock.sync(ctx)
    w1 = ctx.clock.at(time.monotonic())
    ivs = [(b["world_ms"] - a["world_ms"]) / 1000.0 for a, b in zip(frames, frames[1:])]
    ivs = [d for d in ivs if d > 0]
    med = median(ivs)
    persons = [f for f in frames if FX.frame_person(f) is not None]
    ctx.camera_hz = (1.0 / med) if med else None
    k = ((w1 - w0) / wall_d) if (w0 is not None and w1 is not None and wall_d > 0) else None
    ctx.k = k
    ages = pose_ages(ctx)
    age_med = median(ages) if ages else None
    ctx.env.update({
        "camera_hz": None if ctx.camera_hz is None else round(ctx.camera_hz, 2),
        "frame_interval_median_s": None if med is None else round(med, 3),
        "k_world_over_wall": None if k is None else round(k, 4),
        "s0j_frames": len(frames),
        "s0j_wall_s": round(wall_d, 2),
        "s0j_person_frames": len(persons),
        "s0j_infer_ms_median": median([f["infer_ms"] for f in frames]),
        "s0j_rows": len(ctx.rows) - n0,
        "pose_age_ms_median": None if age_med is None else round(age_med, 1),
        "pose_age_ms_max": max(ages) if ages else None,
        "pose_age_n": len(ages),
        "rt_ok": bool(age_med is not None and age_med <= POSE_AGE_RT_MAX_MS),
    })
    Log("环境：camera_hz=%s 帧间隔中位=%s s k=%s 空场 person 帧=%d/%d "
        "位姿龄中位=%s ms/最大=%s ms（%d 行）detector 推理中位=%s ms" % (
            ctx.env["camera_hz"], ctx.env["frame_interval_median_s"],
            ctx.env["k_world_over_wall"], len(persons), len(frames),
            ctx.env["pose_age_ms_median"], ctx.env["pose_age_ms_max"],
            ctx.env["pose_age_n"], ctx.env["s0j_infer_ms_median"]))
    if ctx.env["s0j_person_frames"] != 0:
        Log("[warn] 空场窗口里有 %d 个 person 帧（见 §5 detector 幻影）" % len(persons))
    if not ctx.env["rt_ok"]:
        Log("[warn] S0j 位姿龄中位 %s ms > %.0f ms（环境实时门，只记 warn 不中止）"
            % (ctx.env["pose_age_ms_median"], POSE_AGE_RT_MAX_MS))
    if ctx.camera_hz is None or ctx.camera_hz < 1.00:
        raise FX.HarnessError("相机实测 %.2f Hz < 1.00 Hz（stale=0.8 s 临界）"
                              % (ctx.camera_hz or 0.0))
    return ctx.env


def s0_env_anchors(ctx, s0):
    """S0 追加 2 条环境锚点：副本 argv + MSAA 关掉的间接证据（位姿龄）。"""
    argv = ctx.env.get("world_argv") or " ".join(world_argv_fast(ctx.world_dir))
    ok_path = ("world_fast" in argv) and str(ctx.world_dir).endswith("world_fast")
    age = ctx.env.get("pose_age_ms_median")
    ok_age = age is not None and age <= POSE_AGE_RT_MAX_MS
    s0["asserts"].append(crit("world_path_fast", ok_path, argv))
    s0["asserts"].append(crit(
        "world_msaa_off", ok_age,
        "S0j 位姿龄中位=%s ms（门 ≤%.0f ms）⇒ 副本 override 生效的间接证据"
        % (fmt(age, 1), POSE_AGE_RT_MAX_MS)))
    for c in s0["asserts"][-2:]:
        Log("  S0 环境锚点 %-18s %-8s %s" % (c["name"], "OK" if c["ok"] else "FAIL",
                                             c["detail"][:110]))
    if not ok_path:
        s0["ok"] = False
        raise FX.HarnessError("世界 argv 不含 world_fast：%s" % argv)
    if not ok_age:
        Log("[warn] 位姿龄门未过（%.1f ms），按 step.md「如实记」继续" % (age or -1.0))
    return s0


# ================================================================ goto 用例

def _pose(st):
    return {k: st.get(k) for k in ("x", "y", "z", "yaw", "nav_status", "task_id",
                                   "queue_len", "mode", "assembled")}


def goto_case(ctx, name, target, timeout):
    """一个 goto 用例（**进判据**）：发 goto → 采样到底 → 返回实测 dict。"""
    rec = {"name": name, "kind": "goto", "target": list(target), "mark": len(ctx.sim_history)}
    st0 = ctx.control.status()
    rec["start"] = _pose(st0)
    t_send = time.monotonic()
    rec["t_wall_start"] = time.monotonic()
    ctx.clock.sync(ctx)
    rec["t_world_start"] = ctx.clock.at(rec["t_wall_start"])
    ack = ctx.control.send_cmd({"cmd": "goto", "x": target[0], "y": target[1]})
    rec["ack"] = ack
    if not ack.get("ok"):
        raise FX.HarnessError("goto 回包失败：%r" % (ack,))
    m, line = ctx.wait_sim(
        r"\[control\] goto \(([-\d.]+), ([-\d.]+)\) → cmd_tx", 8)
    rec["cmd_log"] = line or ""
    rec["nav_mark"] = len(ctx.sim_history)      # 「到达」只看这条命令之后的行
    ctx.clock.sync(ctx)
    Log("%s：goto%s → 等到达（上限 %.0f s 墙钟）" % (name, target, timeout))
    nav_hit = None
    settle_from = None
    done = False
    fail_line = None
    n_rows0 = len(ctx.rows)
    shot_mid = False
    while True:
        sample(ctx, name)
        if nav_hit is None:
            for ln in ctx.sim_history[rec["nav_mark"]:]:
                mm = NAV_RE.search(ln)
                if mm:
                    nav_hit = ln.strip()
                    break
        if fail_line is None:
            for ln in ctx.sim_history[rec["nav_mark"]:]:
                if MISSION_FAIL_RE.search(ln):
                    fail_line = ln.strip()
                    break
        if fail_line is not None and (ctx.control.status().get("task_id") is None):
            rec["fail_line"] = fail_line          # 任务已失败出队：不必再空转
            break
        st = ctx.control.status()
        idle_now = (st.get("queue_len") == 0 and st.get("task_id") is None
                    and st.get("nav_status") == "idle")
        if idle_now and nav_hit is not None:
            settle_from = settle_from or time.monotonic()
            if time.monotonic() - settle_from >= 0.30:
                done = True
                break
        else:
            settle_from = None
        # 中段截图（长路径 M3 用）：走了一半就拍一张
        if not shot_mid and name == "M3":
            x, y = float(st.get("x") or 0), float(st.get("y") or 0)
            if dist((x, y), (rec["start"]["x"], rec["start"]["y"])) >= 0.5 * max(
                    1e-6, dist((rec["start"]["x"], rec["start"]["y"]), target)):
                shot_mid = True
                ctx.shots.append(
                    FX.shoot(ctx, "m3_mid", uav=(x, y, float(st.get("z") or 2.5))))
        if time.monotonic() - t_send > timeout:
            rec["timeout"] = True
            break
        time.sleep(0.05)
    rec["mark_end"] = len(ctx.sim_history)      # 用例日志窗右界
    rec["done"] = done
    rec["timeout"] = rec.get("timeout", False)
    m_fail = MISSION_FAIL_RE.search(fail_line or "")
    rec["fail_code"] = int(m_fail.group(1)) if m_fail else None
    rec["fail_line"] = fail_line or ""
    st1 = ctx.control.status()
    rec["end"] = _pose(st1)
    rec["nav_line"] = nav_hit or ""
    rec["t_wall_end"] = time.monotonic()
    ctx.clock.sync(ctx)
    rec["t_world_end"] = ctx.clock.at(rec["t_wall_end"])
    rec["wall_s"] = round(rec["t_wall_end"] - rec["t_wall_start"], 2)
    w0, w1, k = ctx.clock.span(rec["t_wall_start"], rec["t_wall_end"])
    rec["phys_s"] = None if (w0 is None or w1 is None) else round(w1 - w0, 2)
    rec["k"] = None if k is None else round(k, 3)
    rec["rows"] = len(ctx.rows) - n_rows0
    rec["shot"] = ("m3_mid" if shot_mid else None)
    Log("%s：done=%s timeout=%s 墙钟=%.1fs 物理=%s s k=%s 末位姿=(%.3f, %.3f) nav=%s" % (
        name, done, rec["timeout"], rec["wall_s"], rec["phys_s"], rec["k"],
        float(st1.get("x") or 0), float(st1.get("y") or 0), st1.get("nav_status")))
    return rec


def goto_reposition(ctx, name, target):
    """归位 goto（**不进判据**）：**永不抛异常**，只返回实测值。

    第 2 轮：上一轮在这里抛 `FixtureError` 导致 F1~F3 一行没跑（`prev_issues.md` 第 1 条）。
    现在归位失败只落记录（`arrive_ok_hard` = Δ≤0.30 m，`arrive_ok_loose` = Δ≤1.00 m），
    跟随用例照跑；判据按实测判。
    """
    rec = goto_case(ctx, name, target, REPOS_TIMEOUT_WALL)
    rec["kind"] = "reposition"
    st = rec["end"]
    dx = abs(float(st.get("x") or 1e9) - target[0])
    dy = abs(float(st.get("y") or 1e9) - target[1])
    hard = (not rec.get("timeout")) and rec.get("done") and \
        dx <= M_ARRIVE_TOL and dy <= M_ARRIVE_TOL
    loose = (not rec.get("timeout")) and rec.get("done") and \
        dx <= PREP_ARRIVE_LOOSE_M and dy <= PREP_ARRIVE_LOOSE_M
    rec.update({"arrive_ok": hard, "arrive_ok_hard": hard, "arrive_ok_loose": loose,
                "dxy": (round(dx, 3), round(dy, 3)),
                "reached": (st.get("x"), st.get("y")),
                "nav_line": (rec.get("nav_line") or "")[:120],
                "fail_code": rec.get("fail_code")})
    Log("%s：归位 %s Δ=(%.3f, %.3f) hard=%s loose=%s 原因码=%s" % (
        name, target, dx, dy, "OK" if hard else "**不过**",
        "OK" if loose else "**不过**", rec.get("fail_code")))
    return rec


def _clear_queue(ctx):
    """发 `stop` 清队列（避免上一例 follow 残留），失败不抛。"""
    try:
        ctx.control.send_cmd({"cmd": "stop"})
        ctx.wait_sim(r"\[control\] stop", 5)
    except Exception:
        pass
    time.sleep(0.5)


def prep_ladder(ctx, case, targets, budget_s=None):
    """归位梯子：每个目标最多 2 次 `goto_reposition`；**任何失败都不抛**。

    返回 {"tries": [...], "best_dxy", "arrive_ok_loose", "hard_ok", "vantage": {...}}
    """
    t_start = time.monotonic()
    deadline = None if budget_s is None else t_start + budget_s
    tries = []
    best = None
    for tgt in targets:
        got_loose = False
        for attempt in range(2):
            if deadline is not None and time.monotonic() > deadline:
                tries.append({"target": list(tgt), "attempt": attempt + 1,
                              "result": "skipped_budget"})
                break
            _clear_queue(ctx)
            rec = goto_reposition(ctx, "%s-prep%d" % (case, len(tries) + 1), tgt)
            tries.append({"target": list(tgt), "attempt": attempt + 1,
                          "result": "ok" if rec["arrive_ok_hard"] else (
                              "loose" if rec["arrive_ok_loose"] else "fail"),
                          "dxy": rec["dxy"], "reached": list(rec["reached"]),
                          "hard": rec["arrive_ok_hard"], "loose": rec["arrive_ok_loose"],
                          "fail_code": rec.get("fail_code"), "wall_s": rec.get("wall_s")})
            if best is None or max(rec["dxy"]) < max(best):
                best = rec["dxy"]
            if rec["arrive_ok_loose"]:
                got_loose = True
                break
        if not got_loose:
            Log("%s：目标 %s 两次归位都没到（宽松 1.00 m），继续往下跑" % (case, tgt))
    return {"tries": tries, "best_dxy": best,
            "arrive_ok_loose": any(t.get("loose") for t in tries),
            "hard_ok": any(t.get("hard") for t in tries),
            "wall_s": round(time.monotonic() - t_start, 2)}


def vantage_gate(ctx, ped_spawn):
    """目视几何门（**只作记录**）：机到人起点 ≤11 m 且方位角与机头夹角 ≤40°。

    ⚠️ 单位：control `status` 的 `yaw` 是**弧度**（`[Sim] UAV` 日志行才是度），
    本函数内部统一换算成度再比阈值。
    """
    st = ctx.control.status()
    x, y = float(st.get("x") or 0.0), float(st.get("y") or 0.0)
    yaw_deg = math.degrees(float(st.get("yaw") or 0.0))
    dx, dy = ped_spawn["x"] - x, ped_spawn["y"] - y
    rng = math.hypot(dx, dy)
    bearing = abs(wrap_deg(math.degrees(math.atan2(dy, dx)) - yaw_deg))
    ok = rng <= VANTAGE_RANGE_MAX_M and bearing <= VANTAGE_BEARING_MAX_DEG
    rec = {"range_m": round(rng, 3), "bearing_deg": round(bearing, 1),
           "ok": ok, "uav": (round(x, 3), round(y, 3)),
           "yaw_deg": round(yaw_deg, 1)}
    Log("目视几何：人起点距机 %.2f m，方位夹角 %.1f°（yaw=%.1f°）→ %s（只记录，不跳过用例）" % (
        rng, bearing, yaw_deg, "OK" if ok else "**不足**"))
    return rec


def prep_face(ctx, case, ped_spawn, tol_deg=20.0, max_s=25.0):
    """把机头转到行人方向（**manual 闭环**：spin + status yaw 反馈，最多 2 轮）。

    为什么不用 `goto` 摆朝向（实测证据，见 §5 根因）：`goto` 的到达是**按位置**判的，
    最后一格还没转到位就可能 `next=None`，实测 yaw 在 −83.8° → +97.5° 之间甩
    ⇒ 人落在 43° 半 FOV 之外，detector 一帧都看不到人，follow 原地不动，
    F1~F3 会变成「测不到」而不是「跟不住」。夹具需要的是「相机对着人」。

    两个坑（都实测过）：
      * `status.yaw` 是**弧度**（`[Sim] UAV` 日志行才是度）；
      * 切回 auto 时**队列里残留的旧任务会复活**（实测机头继续转到 −77.4°、并南移 1 m），
        所以每轮开头先 `stop`（Auto→清空任务），切回 auto 后再核对一次朝向。
    **不改任何被测代码、不改判据阈值**，只用控制口已有的命令。
    """
    rec = {"rounds": [], "ok": False, "yaw_end": None, "bearing_end": None}
    for rnd in (1, 2):
        _clear_queue(ctx)                      # Auto(Set([]))：清空队列，防旧任务复活
        st = ctx.control.status()
        x0, y0 = float(st.get("x") or 0.0), float(st.get("y") or 0.0)
        yaw0 = math.degrees(float(st.get("yaw") or 0.0))      # status.yaw 是弧度
        b0 = wrap_deg(math.degrees(
            math.atan2(ped_spawn["y"] - y0, ped_spawn["x"] - x0)) - yaw0)
        ctx.control.send_cmd({"cmd": "mode", "value": "manual"})
        t0 = time.monotonic()
        prev, right_when_positive, worsen, n = None, True, 0, 0
        while time.monotonic() - t0 < max_s:
            st = ctx.control.status()
            yaw = math.degrees(float(st.get("yaw") or 0.0))    # status.yaw 是弧度
            x, y = float(st.get("x") or 0.0), float(st.get("y") or 0.0)
            b = wrap_deg(math.degrees(
                math.atan2(ped_spawn["y"] - y, ped_spawn["x"] - x)) - yaw)
            if abs(b) <= tol_deg:
                break
            if prev is not None:
                if abs(b) > abs(prev) + 3.0:
                    worsen += 1
                    if worsen >= 3:                # 转向反了：翻转一次符号约定
                        right_when_positive = not right_when_positive
                        worsen = 0
                        Log("[warn] %s：朝向闭环方向疑似反了 → 翻转 spin 符号" % case)
                else:
                    worsen = 0
            prev = b
            act = "spin_right" if (b > 0) == right_when_positive else "spin_left"
            ctx.control.send_cmd({"cmd": "manual", "action": act})
            n += 1
            time.sleep(0.05)
        try:
            ctx.control.send_cmd({"cmd": "manual", "action": "stop"})
        except Exception:
            pass
        ctx.control.send_cmd({"cmd": "mode", "value": "auto"})
        time.sleep(0.3)
        st = ctx.control.status()
        x, y = float(st.get("x") or 0.0), float(st.get("y") or 0.0)
        yaw = math.degrees(float(st.get("yaw") or 0.0))
        b = wrap_deg(math.degrees(math.atan2(ped_spawn["y"] - y, ped_spawn["x"] - x)) - yaw)
        rec["rounds"].append({
            "round": rnd, "yaw_start": round(yaw0, 1), "bearing_start": round(b0, 1),
            "yaw_end": round(yaw, 1), "bearing_end": round(b, 1), "steps": n,
            "wall_s": round(time.monotonic() - t0, 2), "mode": st.get("mode"),
            "queue_len": st.get("queue_len")})
        rec["yaw_end"], rec["bearing_end"] = round(yaw, 1), round(b, 1)
        Log("%s：朝向闭环 第%d轮 yaw %.1f°→%.1f° 方位差 %.1f°→%.1f°（门 ≤%.0f°，%d 步 %.1f s，mode=%s）" % (
            case, rnd, yaw0, yaw, b0, b, tol_deg, n, rec["rounds"][-1]["wall_s"],
            st.get("mode")))
        if abs(b) <= tol_deg and st.get("mode") == "auto":
            rec["ok"] = True
            break
    return rec


# ================================================================ 行人客户端

def ped_open(ctx, peer, spawn):
    """接一条行人客户端并等世界 `[ws] pedestrian spawned`。"""
    before = ped_alive_count(ctx)                    # 门禁：开之前世界动态行人必须为 0
    ctx.env["ped_leak_before_%s" % peer] = before
    if before:
        Log("[warn] %s 接入前世界仍有动态行人 %s（上一例残留，见 §5）" % (peer, before))
    inst = FX.PedWalker("ws://127.0.0.1:%d" % WORLD_PORT, peer, dict(spawn))
    try:
        inst.start()
    except Exception as exc:                      # 行人接入失败 = harness 故障
        ctx.env["harness_fault"] = "pedestrian:%s: %s" % (type(exc).__name__, exc)
        raise FX.HarnessError("行人接入失败（%s）：%s" % (type(exc).__name__, exc))
    m, line = ctx.wait_world(SPAWN_RE % re.escape(peer), 15)
    ready = inst.ready_spawn or {}
    rec = {"peer": peer, "spawn_req": dict(spawn), "ready_spawn": ready,
           "spawn_log": line or "", "spawn_ok": m is not None,
           "alive_before": before,
           "spawn_matches": (abs(float(ready.get("x", 1e9)) - spawn["x"]) < 1e-3
                             and abs(float(ready.get("y", 1e9)) - spawn["y"]) < 1e-3)}
    if not rec["spawn_ok"]:
        inst.close()
        raise FX.HarnessError("世界 15 s 内没有 pedestrian spawned peer='%s'" % peer)
    ctx.ped = inst
    ctx.ped_peer = peer
    ctx.read_world()
    rec["alive_after"] = ped_alive_count(ctx)
    Log("行人 %s 接入：spawn=(%.2f, %.2f) yaw=%.2f %s（世界动态行人 %d→%d）" % (
        peer, float(ready.get("x", 0)), float(ready.get("y", 0)),
        float(ready.get("yaw", 0)), "与请求一致" if rec["spawn_matches"] else "**与请求不符**",
        len(before), len(rec["alive_after"])))
    return rec


def ped_close_wait(ctx, peer):
    """关客户端并等世界 `[ws] pedestrian removed`（防残影污染下一例）。"""
    rec = {"peer": peer}
    if ctx.ped is None:
        rec["ok"] = True
        return rec
    rec["poses"] = len(ctx.ped.snapshot())
    rec["auto_lost"] = not ctx.ped.connected
    try:
        ctx.ped.close()
    except Exception as exc:
        rec["close_error"] = repr(exc)
    m, line = ctx.wait_world(REMOVED_RE % re.escape(peer), 10)
    rec["removed_log"] = line or ""
    rec["ok"] = m is not None
    ctx.read_world()
    rec["alive_after"] = ped_alive_count(ctx)     # 门禁：关之后世界动态行人必须为 []
    rec["leak"] = rec["alive_after"]
    if rec["alive_after"]:
        Log("[warn] 关掉 %s 后世界仍有动态行人 %s" % (peer, rec["alive_after"]))
    Log("行人 %s 关闭：removed=%s，世界动态行人=%s" % (
        peer, "OK" if rec["ok"] else "**10s 内未见 removed**", rec["alive_after"] or "无"))
    return rec


def ped_halt(ctx):
    """让行人停下。

    ⚠️ `FX.PedWalker` **没有 `stop()` 方法**（只有 `start/_loop/drive/close/latest/snapshot`）；
    首轮脚本里的 `ped.stop()` 是死代码路径（首轮 F1 归位就失败，没跑到这里所以没暴露）。
    v=0 即停（朝向保持）。
    """
    if ctx.ped is None:
        return False
    try:
        ctx.ped.drive(0.0, None)
        Log("行人已停车（v=0）")
        return True
    except Exception as exc:
        Log("[warn] 行人停车失败：%r" % (exc,))
        return False


def ped_alive_count(ctx):
    """从世界日志的 spawned/removed 行数出「世界当前动态行人数」。"""
    alive = set()
    for line in ctx.world_history:
        m = re.search(r"\[ws\] pedestrian spawned peer='([^']+)'", line)
        if m:
            alive.add(m.group(1))
            continue
        m = re.search(r"\[ws\] pedestrian removed peer='([^']+)'", line)
        if m:
            alive.discard(m.group(1))
    return sorted(alive)


def follow_on(ctx, timeout=20.0):
    """发 follow 并等到 `task_ticks >= 5`（证明树在 tick，不信 nav_status）。"""
    t0 = time.monotonic()
    ack = ctx.control.send_cmd({"cmd": "follow"})
    m, line = ctx.wait_sim(CTRL_FOLLOW_RE.pattern, 8)
    while time.monotonic() - t0 < timeout:
        st = ctx.control.status()
        if (st.get("task_ticks") or 0) >= 5 and st.get("task_id") is not None:
            Log("follow 已进树：task_id=%s ticks=%s nav=%s" % (
                st.get("task_id"), st.get("task_ticks"), st.get("nav_status")))
            return {"ack": ack, "cmd_log": line or "", "status": _pose(st),
                    "wall_s": round(time.monotonic() - t0, 2), "ok": True}
        ctx.read_sim()
        ctx.clock.sync(ctx)
        time.sleep(0.1)
    raise FX.HarnessError("follow 20 s 内没有 task_ticks>=5（follow 任务没进树）")


# ================================================================ 跟随用例

def _follow_loop(ctx, seg, end_cond, timeout, dt=0.10, extra_phys=0.0):
    """采样直到 end_cond() 为真（或墙钟超时）；可选再多采 extra_phys 物理秒。"""
    t0 = time.monotonic()
    timed_out = False
    while True:
        sample(ctx, seg)
        if end_cond():
            break
        if time.monotonic() - t0 > timeout:
            timed_out = True
            break
        time.sleep(dt)
    if extra_phys > 0:
        sample_phys(ctx, extra_phys, seg=seg)
    return {"timeout": timed_out, "wall_s": round(time.monotonic() - t0, 2)}


def case_f1(ctx):
    """F1 直线跟随：人沿 y≈64.5 向东 20 物理 s（位移 25.2 m）。"""
    rec = {"name": "F1", "kind": "follow", "mark": len(ctx.sim_history),
           "prep": [], "seg": "F1"}
    rec["prep"] = prep_ladder(ctx, "F1", [REPOS_W, F1_START], CASE_TIMEOUT_WALL["f1"] * 0.7)
    rec["face"] = prep_face(ctx, "F1", PED_F1_SPAWN)
    rec["vantage"] = vantage_gate(ctx, PED_F1_SPAWN)
    rec["ped"] = ped_open(ctx, PEER_F1, PED_F1_SPAWN)
    rec["follow"] = follow_on(ctx)
    ped = ctx.ped
    t0_mono = ped.drive(PED_F1_SPEED, PED_F1_DIR)
    ctx.clock.sync(ctx)
    tw0 = ctx.clock.at(t0_mono)
    rec["t_wall_start"] = t0_mono
    rec["t_world_start"] = tw0
    p_start = ctx.control.status()
    rec["start"] = _pose(p_start)
    Log("F1：人起走 (%.2f, %.2f) v=%.2f → 追 %.1f m" % (
        p_start.get("x", 0), p_start.get("y", 0), PED_F1_SPEED, PED_F1_TRACK_M))
    shot_done = False
    t_send = t0_mono

    def cond():
        latest = ped.latest()
        row_now = ctx.rows[-1] if ctx.rows else {}
        nonlocal shot_done
        if not shot_done and (row_now.get("t_world") or 0) - (tw0 or 0) >= 8.0:
            shot_done = True
            ctx.shots.append(FX.shoot(ctx, "f1_mid", uav=(row_now.get("x"), row_now.get("y"),
                                                          row_now.get("z")),
                                      ped=(latest[1], latest[2]) if latest else None))
        if latest is None:
            return False
        return (latest[1] - PED_F1_SPAWN["x"]) >= PED_F1_TRACK_M

    loop = _follow_loop(ctx, "F1", cond, CASE_TIMEOUT_WALL["f1"])
    ped_halt(ctx)
    rec["loop"] = loop
    rec["mark_end"] = len(ctx.sim_history)
    rec["t_wall_end"] = time.monotonic()
    ctx.clock.sync(ctx)
    rec["t_world_end"] = ctx.clock.at(rec["t_wall_end"])
    rec["end"] = _pose(ctx.control.status())
    latest = ped.latest()
    rec["ped_end"] = None if latest is None else [round(latest[1], 3), round(latest[2], 3)]
    rec["ped_track_m"] = None if latest is None else round(latest[1] - PED_F1_SPAWN["x"], 3)
    w0, w1, k = ctx.clock.span(rec["t_wall_start"], rec["t_wall_end"])
    rec["phys_s"] = None if (w0 is None or w1 is None) else round(w1 - w0, 2)
    rec["k"] = None if k is None else round(k, 3)
    rec["wall_s"] = round(rec["t_wall_end"] - rec["t_wall_start"], 2)
    rec["ped_close"] = ped_close_wait(ctx, PEER_F1)
    record_ped_poses(ctx, rec)
    Log("F1：结束 人位移=%s m 物理=%s s 墙钟=%.1f s k=%s" % (
        rec["ped_track_m"], rec["phys_s"], rec["loop"]["wall_s"], rec["k"]))
    return rec


def case_f2(ctx):
    """F2 切线通过（重点）：人 (72,60)→(72,68)，0.6 m/s。"""
    rec = {"name": "F2", "kind": "follow", "mark": len(ctx.sim_history),
           "prep": [], "seg": "F2"}
    rec["prep"] = prep_ladder(ctx, "F2", [F2_PREP], CASE_TIMEOUT_WALL["f2"] * 0.5)
    rec["face"] = prep_face(ctx, "F2", PED_F2_SPAWN)
    rec["vantage"] = vantage_gate(ctx, PED_F2_SPAWN)
    rec["ped"] = ped_open(ctx, PEER_F2, PED_F2_SPAWN)
    ped = ctx.ped
    # F2：人起走前先采 2 物理 s（「已在画面里」的基线）
    # ⚠️ 顺序在**第 2 轮修正**：基线采样必须在 `follow` **之前**。原顺序（先 follow 再采基线）
    # 会让 follow 在这 2 s 里就把机带走并把目标丢掉（实测 1.3 m / yaw −109.6°），
    # 于是「10 s 位移」从一个人已经出画的起点开始测 ⇒ 测的是退化初态而不是跟随能力。
    ped_halt(ctx)
    sample_phys(ctx, 2.0, seg="F2")
    shot_before = FX.shoot(ctx, "f2_before", uav=None,
                           ped=(PED_F2_SPAWN["x"], PED_F2_SPAWN["y"]))
    ctx.shots.append(shot_before)
    rec["follow"] = follow_on(ctx)
    t0_mono = ped.drive(PED_F2_SPEED, PED_F2_DIR)
    ctx.clock.sync(ctx)
    tw0 = ctx.clock.at(t0_mono)
    rec["t_wall_start"] = t0_mono
    rec["t_world_start"] = tw0
    rec["start"] = _pose(ctx.control.status())
    Log("F2：人起走 (%.2f, %.2f) v=%.2f dir=%s → 走 %.1f m" % (
        PED_F2_SPAWN["x"], PED_F2_SPAWN["y"], PED_F2_SPEED, "北(+y)", PED_F2_TRACK_M))
    shot_after = {"done": False}

    def cond():
        latest = ped.latest()
        row_now = ctx.rows[-1] if ctx.rows else {}
        if not shot_after["done"] and (row_now.get("t_world") or 0) - (tw0 or 0) >= 7.0:
            shot_after["done"] = True
            ctx.shots.append(FX.shoot(ctx, "f2_after", uav=(row_now.get("x"), row_now.get("y"),
                                                            row_now.get("z")),
                                      ped=(latest[1], latest[2]) if latest else None))
        if latest is None:
            return False
        return (latest[2] - PED_F2_SPAWN["y"]) >= PED_F2_TRACK_M

    loop = _follow_loop(ctx, "F2", cond, CASE_TIMEOUT_WALL["f2"], extra_phys=2.0)
    ped_halt(ctx)
    rec["loop"] = loop
    rec["mark_end"] = len(ctx.sim_history)
    rec["t_wall_end"] = time.monotonic()
    ctx.clock.sync(ctx)
    rec["t_world_end"] = ctx.clock.at(rec["t_wall_end"])
    rec["end"] = _pose(ctx.control.status())
    latest = ped.latest()
    rec["ped_end"] = None if latest is None else [round(latest[1], 3), round(latest[2], 3)]
    rec["ped_track_m"] = None if latest is None else round(latest[2] - PED_F2_SPAWN["y"], 3)
    w0, w1, k = ctx.clock.span(rec["t_wall_start"], rec["t_wall_end"])
    rec["phys_s"] = None if (w0 is None or w1 is None) else round(w1 - w0, 2)
    rec["k"] = None if k is None else round(k, 3)
    rec["wall_s"] = round(rec["t_wall_end"] - rec["t_wall_start"], 2)
    rec["ped_close"] = ped_close_wait(ctx, PEER_F2)
    record_ped_poses(ctx, rec)
    Log("F2：结束 人位移=%s m 物理=%s s 墙钟=%.1f s k=%s" % (
        rec["ped_track_m"], rec["phys_s"], rec["loop"]["wall_s"], rec["k"]))
    return rec


def case_f3(ctx):
    """F3 丢失：机在动时关掉行人客户端 → 800 ms 后停住。"""
    rec = {"name": "F3", "kind": "follow", "mark": len(ctx.sim_history),
           "prep": [], "seg": "F3"}
    rec["prep"] = prep_ladder(ctx, "F3", [F3_START], CASE_TIMEOUT_WALL["f3"] * 0.5)
    rec["face"] = prep_face(ctx, "F3", PED_F3_SPAWN)
    rec["vantage"] = vantage_gate(ctx, PED_F3_SPAWN)
    rec["ped"] = ped_open(ctx, PEER_F3, PED_F3_SPAWN)
    rec["follow"] = follow_on(ctx)
    ped = ctx.ped
    t0_mono = ped.drive(PED_F3_SPEED, PED_F3_DIR)
    ctx.clock.sync(ctx)
    rec["t_wall_start"] = t0_mono
    rec["t_world_start"] = ctx.clock.at(t0_mono)
    rec["start"] = _pose(ctx.control.status())
    # 等「连续 >=3 条 v>0」且「自 F3 起点位移 ≥1.5 m」（≤60 s 墙钟）
    t_wait = time.monotonic()
    moved = None
    last3 = []
    while time.monotonic() - t_wait < 60.0:
        sample(ctx, "F3")
        ul = uav_lines(ctx, rec["mark"])
        last3 = ul[-F3_PRE_V_LINES:]
        row = ctx.rows[-1]
        moved = dist((row["x"], row["y"]),
                     (rec["start"]["x"], rec["start"]["y"]))
        if (len(last3) >= F3_PRE_V_LINES and all(ln["v"] > 0 for ln in last3)
                and moved >= F3_PRE_MOVE_MIN):
            break
        time.sleep(0.10)
    rec["pre_v_ok"] = bool(len(last3) >= F3_PRE_V_LINES
                           and all(ln["v"] > 0 for ln in last3))
    rec["pre_v"] = [ln["v"] for ln in last3]
    rec["pre_omega"] = [ln["omega"] for ln in last3]      # float 解析后才有值（旧口径恒 None）
    cov_ok, cov_tot = uav_coverage(ctx, rec["mark"])
    rec["pre_uav_parsed"] = "%d/%d" % (cov_ok, cov_tot)
    rec["pre_moved_m"] = None if moved is None else round(moved, 3)
    rec["pre_ok"] = bool(rec["pre_v_ok"] and (moved or 0.0) >= F3_PRE_MOVE_MIN)
    ctx.shots.append(FX.shoot(ctx, "f3_before_close",
                              uav=(ctx.rows[-1]["x"], ctx.rows[-1]["y"], ctx.rows[-1]["z"]),
                              ped=(ped.latest()[1], ped.latest()[2]) if ped.latest() else None))
    Log("F3：关客户端前 v=%s 位移=%.3f m → %s" % (
        rec["pre_v"], moved or 0.0, "OK" if rec["pre_ok"] else "**前置不过**"))
    rec["t_close_wall"] = time.monotonic()
    ctx.clock.sync(ctx)
    rec["t_close_world"] = ctx.clock.at(rec["t_close_wall"])
    # 最后一条 adopted person 帧的物理时刻（t_lost）
    t_lost = None
    for fr in reversed(ctx.frames):
        if frame_person_adopted(fr) is not None:
            t_lost = ctx.clock.at(float(fr["wall"]))
            break
    rec["t_lost_world"] = t_lost if t_lost is not None else rec["t_close_world"]
    ctx.read_world()
    rec["alive_before_close"] = ped_alive_count(ctx)      # 门禁：关之前应恰好 1 人（human-f3）
    Log("F3：关客户端前世界动态行人=%s" % (rec["alive_before_close"] or "无",))
    rec["ped_close"] = ped_close_wait(ctx, PEER_F3)
    sample_phys(ctx, 5.0, seg="F3")
    ctx.shots.append(FX.shoot(ctx, "f3_after_close",
                              uav=(ctx.rows[-1]["x"], ctx.rows[-1]["y"], ctx.rows[-1]["z"]),
                              ped=None))
    rec["mark_end"] = len(ctx.sim_history)
    rec["t_wall_end"] = time.monotonic()
    ctx.clock.sync(ctx)
    rec["t_world_end"] = ctx.clock.at(rec["t_wall_end"])
    rec["end"] = _pose(ctx.control.status())
    w0, w1, k = ctx.clock.span(rec["t_wall_start"], rec["t_wall_end"])
    rec["phys_s"] = None if (w0 is None or w1 is None) else round(w1 - w0, 2)
    rec["k"] = None if k is None else round(k, 3)
    rec["wall_s"] = round(rec["t_wall_end"] - rec["t_wall_start"], 2)
    record_ped_poses(ctx, rec)
    Log("F3：t_lost=%s 关后采样 5 物理 s 结束=%s 墙钟=%.1f s" % (
        rec["t_lost_world"], rec["t_world_end"], rec["wall_s"]))
    return rec


# （原 `_prep()` 已删除：归位逻辑全部并入 `prep_ladder`；保留薄封装会命中 harness_lint 的死代码门。）

# ================================================================ 判据（27 行）

def _mk(ident, case, text, measured, ok, basis, env_fail=False, vacuous=False):
    """一条判据。`vacuous=True` ⇒ `N-A（真空）`：断言在前置不成立时恒真，不计通过。

    `prev_issues.md` 第 5 条：F3-1 前置不过时 F3-2/F3-3 是「机本来就没动」下的真值恒真
    断言，判 PASS 等于伪造通过；现在单列 `N-A`（`pass=False`，但不算 FAIL）。
    """
    if vacuous:
        verdict = "N-A"
    elif env_fail:
        verdict = "FAIL(env)"
    else:
        verdict = "PASS" if ok else "FAIL"
    return {"id": ident, "case": case, "text": text, "measured": measured,
            "pass": bool(ok) and not env_fail and not vacuous, "verdict": verdict,
            "vacuous": bool(vacuous), "basis": basis}


def _goto_judges(ctx, rec, ids, with_omega=False):
    """M1/M2/M3 共用：L{n}~L{n+4}。ids = [arrive, blocked, estop, stopidle, bounds]。"""
    name = rec["name"]
    mark = rec["mark"]
    mark_end = rec.get("mark_end")
    nav_mark = rec.get("nav_mark", mark)
    new = ctx.sim_history[mark:mark_end]
    nav_hits = [ln.strip() for ln in ctx.sim_history[nav_mark:mark_end] if NAV_RE.search(ln)]
    blocked = [ln.strip() for ln in new if BLOCKED_RE.search(ln)]
    notready = [ln.strip() for ln in new if NOTREADY_RE.search(ln)]
    estop = [ln.strip() for ln in new if ESTOP_RE.search(ln)]
    no_lidar = any(NO_LIDAR_RE.search(ln) for ln in ctx.sim_history)
    rows = seg_rows(ctx, name)
    paths = path_metrics(rows)
    st = rec["end"]
    dxt = abs(float(st.get("x") or 1e9) - rec["target"][0])
    dyt = abs(float(st.get("y") or 1e9) - rec["target"][1])
    ul = uav_lines(ctx, mark, mark_end)
    cov_ok, cov_tot = uav_coverage(ctx, mark, mark_end)
    cov_full = cov_tot > 0 and cov_ok == cov_tot
    segs = stop_segments(ul)
    micro = [s for s in segs if s["micro"]]
    block_status = any((r.get("nav_status") == "blocked") for r in rows)
    out = []
    ok_arrive = (not rec.get("timeout")) and bool(nav_hits) and dxt <= M_ARRIVE_TOL \
        and dyt <= M_ARRIVE_TOL and st.get("task_id") is None
    out.append(_mk(ids[0], name, "nav 判 Arrived（`[Nav] 到达` + 末位逐轴 ≤0.30 m）",
                   "nav 行 %d 条；末位 (%.3f, %.3f) Δ=(%.3f, %.3f)；task_id=%s；"
                   "timeout=%s；任务失败原因码=%s" % (
                       len(nav_hits), float(st.get("x") or 0), float(st.get("y") or 0),
                       dxt, dyt, st.get("task_id"), rec.get("timeout"),
                       rec.get("fail_code")),
                   ok_arrive, "sim.log / status"))
    out.append(_mk(ids[1], name, "无 `Blocked`",
                   "`[Nav] 连续 N 拍无路`=%d 条；status.nav_status=blocked 的采样=%d 条" % (
                       len(blocked), sum(1 for r in rows if r.get("nav_status") == "blocked")),
                   (not blocked) and (not block_status), "sim.log / status"))
    out.append(_mk(ids[2], name, "无急停（`[Sim] 前方障碍` 计数=0）",
                   "`[Sim] 前方障碍`=%d 条；环境 `[Sim] 无 lidar…跳过急停`=%s（弱判据，见 §5）" % (
                       len(estop), "存在" if no_lidar else "缺失"),
                   len(estop) == 0, "sim.log（弱判据）"))
    out.append(_mk(ids[3], name, "无连续 `Stop + Idle` 交替（停段 ≤2、微循环=0）",
                   "`[Sim] UAV` 行 %d 条（解析 %d/%d）；停段 %d 段 %s；"
                   "微循环（≤3 条且两侧在走）%d 段；"
                   "`[Nav] 连续 N 拍未算完` %d 条" % (
                       len(ul), cov_ok, cov_tot, len(segs),
                       [(s["n"], s["prev"], s["next"]) for s in segs],
                       len(micro), len(notready)),
                   len(segs) <= M_STOP_EP_MAX and len(micro) <= M_MICRO_CYCLE_MAX
                   and cov_full,
                   "`[Sim] UAV` 行（5 Hz）", env_fail=not cov_full))
    ok_bounds = (paths["oob"] == 0 and paths["z_oob"] == 0
                 and (paths["step_max"] is None or paths["step_max"] <= M_STEP_MAX)
                 and (paths["ratio"] is None or M_PATH_LO <= paths["ratio"] <= M_PATH_HI))
    out.append(_mk(ids[4], name, "无越界/撞墙（边界 + 路径长比 + 单步跳变）",
                   "采样 %d 点；越界 %d；z 越界 %d；单步最大 %s m"
                   "（Δt≤%.2f s 口径，%d/%d 对参与，丢弃 %d 对；"
                   "未过滤最大 %s m 出现在 Δt=%s s）；路径长/直线=%s" % (
                       paths["n"], paths["oob"], paths["z_oob"], fmt(paths["step_max"]),
                       STEP_DT_MAX_S, paths["step_pairs"],
                       paths["step_pairs"] + paths["step_pairs_dropped"],
                       paths["step_pairs_dropped"], fmt(paths["step_max_raw"]),
                       fmt(paths["step_dt_raw"]), fmt(paths["ratio"])),
                   ok_bounds, "trace.csv（物理时间；Δt 过滤见 §5）"))
    if with_omega:
        flips = omega_flips(ul)
        out.append(_mk("L9b", name, "拐角不反复停走：omega 符号翻转 ≤3",
                       "omega≠0 的行数=%d；符号翻转=%d；解析 %d/%d" % (
                           sum(1 for ln in ul if ln["omega"] != 0), flips,
                           cov_ok, cov_tot),
                       flips <= M_OMEGA_FLIP_MAX and cov_full, "`[Sim] UAV` 行",
                       env_fail=not cov_full))
    ages = [ln["age_ms"] for ln in ul]
    speeds = []
    pts = [(r["t_mono"], r["x"], r["y"]) for r in rows if r.get("x") is not None]
    j = 0
    for i in range(len(pts)):
        while j + 1 < len(pts) and pts[j + 1][0] - pts[i][0] <= 1.0:
            j += 1
        dt = pts[j][0] - pts[i][0]
        if dt >= 0.9:
            speeds.append(dist((pts[i][1], pts[i][2]), (pts[j][1], pts[j][2])) / dt)
    ys = [ln["yaw"] for ln in ul]
    rec["analysis"] = {"nav_hits": len(nav_hits), "blocked": len(blocked),
                       "yaw_min": None if not ys else round(min(ys), 1),
                       "yaw_max": None if not ys else round(max(ys), 1),
                       "yaw_span": None if not ys else round(max(ys) - min(ys), 1),
                       "fail_code": rec.get("fail_code"),
                       "pose_age_ms_median": None if not ages else round(median(ages), 1),
                       "pose_age_ms_max": None if not ages else max(ages),
                       "speed_max_1s": None if not speeds else round(max(speeds), 3),
                       "path_total_m": None if not rows else round(
                           sum(dist((a["x"], a["y"]), (b["x"], b["y"]))
                               for a, b in zip(rows, rows[1:])), 3),
                       "notready": len(notready), "estop": len(estop),
                       "uav_lines": len(ul), "uav_parsed": "%d/%d" % (cov_ok, cov_tot),
                       "uav_coverage_ok": cov_full, "stop_segments": [
                           {"n": s["n"], "prev": s["prev"], "next": s["next"],
                            "micro": s["micro"],
                            "t0": None if s["t0"] is None else round(s["t0"], 2),
                            "t1": None if s["t1"] is None else round(s["t1"], 2)}
                           for s in segs],
                       "micro": len(micro), "paths": paths,
                       "omega_flips": omega_flips(ul),
                       "dxy": [round(dxt, 3), round(dyt, 3)]}
    return out


def record_ped_poses(ctx, rec):
    """把本用例的行人真值快照并入全局真值索引。

    为什么要合并：判据在**所有用例跑完之后**统一算，那时 `ctx.ped` 已经被下一例的
    `PedWalker` 替换（F3 之后就是 None）⇒ 不能拿 `ctx.ped` 当 F1/F2 的真值。
    各用例的行人位姿时间戳单调不重叠，合并成一个有序索引即可。
    """
    poses = ctx.ped.snapshot() if ctx.ped is not None else []
    rec["ped_poses"] = [[round(p[0], 3), round(p[1], 3), round(p[2], 3), round(p[3], 3)]
                        for p in poses]
    lst = getattr(ctx, "_truth_all", None)
    if lst is None:
        lst = []
        ctx._truth_all = lst
    lst.extend(poses)
    Log("%s：行人真值 %d 条并入索引（累计 %d 条）" % (rec["name"], len(poses), len(lst)))
    return rec["ped_poses"]


def ped_truth_at(ctx, wall, tol=0.6):
    """某个 detector 帧的墙钟时刻最邻近的行人真值 (x, y)；无真值时 None。"""
    poses = getattr(ctx, "_truth_all", None)
    if not poses:
        poses = ctx.ped.snapshot() if ctx.ped is not None else []
    if not poses:
        return None
    n = len(poses)
    if getattr(ctx, "_truth_n", -1) != n:
        pts = sorted((p[0], p[1], p[2]) for p in poses)
        ctx._truth_pts = pts
        ctx._truth_ts = [p[0] for p in pts]
        ctx._truth_n = n
    ts = getattr(ctx, "_truth_ts", [])
    if not ts:
        return None
    i = bisect.bisect_left(ts, wall)
    best = None
    for j in (i - 1, i):
        if 0 <= j < len(ts):
            dt = abs(ts[j] - wall)
            if dt <= tol and (best is None or dt < best[0]):
                best = (dt, ctx._truth_pts[j])
    return None if best is None else (best[1][1], best[1][2])


def frame_person_real(ctx, fr):
    """真人框集合：`est` 与行人真值 ≤REAL_PERSON_TOL_M 且 bbox 底边不触图像底边。

    `prev_issues.md` 第 2 条：空场时 detector 会把画面底部同一 bbox 判成 person
    （y1=479=底边、距离≈2.5 m），follow.rs 取 range 最小 ⇒ 幻影会抢在真人前面。
    「检测到人」「不撞人」两句话里的「人」只能是真人，所以主口径用本函数。
    """
    truth = ped_truth_at(ctx, float(fr["wall"]))
    if truth is None:
        return []
    out = []
    for d in fr["dets"]:
        if d["cls"] != "person" or d.get("est") is None:
            continue
        if d["bbox"][3] >= fr["height"] - 1:
            continue
        if dist(d["est"], truth) <= REAL_PERSON_TOL_M:
            out.append(d)
    return out


def _judge_follow_window(ctx, rec, tag, tw_lo, tw_hi):
    """F1/F2/F4 共用：窗内 `距离=` 的**双口径**（real 主 / adopted 并列）+ 幻影帧数。"""
    fl = frames_in(ctx, tw_lo, tw_hi)
    ad, re_, fp_n = [], [], 0
    for _tw, fr in fl:
        pa = frame_person_adopted(fr)
        if pa is not None and pa.get("range_m") is not None:
            ad.append(pa["range_m"])
        real = frame_person_real(ctx, fr)
        if real:
            d = min(real, key=lambda x: 1e9 if x.get("range_m") is None else x["range_m"])
            if d.get("range_m") is not None:
                re_.append(d["range_m"])
        elif any(x["cls"] == "person" for x in fr["dets"]):
            fp_n += 1                     # 有 person 框但不对真人 = 幻影帧
    rec.setdefault("det", {})[tag] = {
        "frames": len(fl), "person_frames": len(ad),
        "min_range_m": None if not ad else round(min(ad), 3),
        "median_range_m": None if not ad else round(median(ad), 3),
        "hit_rate": round(len(ad) / len(fl), 3) if fl else None,
        "real_person_frames": len(re_),
        "real_min_range_m": None if not re_ else round(min(re_), 3),
        "real_hit_rate": round(len(re_) / len(fl), 3) if fl else None,
        "fp_frames": fp_n,
    }
    return re_, ad, fl


def judge_f1(ctx, rec):
    out = []
    tw0, tw1 = rec.get("t_world_start"), rec.get("t_world_end")
    rows = seg_rows(ctx, "F1")
    d10 = disp(rows, tw0, tw0 + 10.0) if tw0 is not None else None
    ok1 = d10 is not None and d10["d"] >= MOVE10_MIN
    out.append(_mk("F1-1", "F1", "10 物理 s 内机位移 ≥5 m",
                   "位移=%s m（窗 [t0, t0+10] 物理）" % fmt(None if d10 is None else d10["d"]),
                   ok1, "trace.csv（物理时间）"))
    d_end = None
    med = None
    if tw1 is not None:
        tail = [r for r in rows if r.get("t_world") is not None
                and r["t_world"] >= tw1 - 5.0 and r.get("d_m") is not None]
        med = median([r["d_m"] for r in tail])
        d_end = tail[-1]["d_m"] if tail else None
    ok2 = (med is not None and med <= F1_D_MED_MAX and d_end is not None
           and d_end <= F1_D_END_MAX)
    out.append(_mk("F1-2", "F1", "不掉队（末 5 物理 s 距离中位 ≤5.0 m 且末距离 ≤5.0 m）",
                   "末 5 物理 s 中位=%s m；末距离=%s m；样本 %d" % (
                       fmt(med), fmt(d_end),
                       len([r for r in rows if r.get("d_m") is not None])),
                   ok2, "trace.csv（truth：status x/y − 行人 pose）"))
    real, adopted, fl = _judge_follow_window(ctx, rec, "F1", tw0, tw1)
    truth = [r["d_m"] for r in rows if r.get("t_world") is not None
             and tw0 <= r["t_world"] <= tw1 and r.get("d_m") is not None]
    det = rec["det"]["F1"]
    min_real = det.get("real_min_range_m")
    min_truth = min(truth) if truth else None
    ok3 = (min_real is not None and min_real >= MIN_D
           and (min_truth is None or min_truth >= MIN_D))
    out.append(_mk("F1-3", "F1", "不撞人：窗内最小距离 ≥2.0 m（主口径=真人）",
                   "真人帧 min=%s m（%d/%d 帧，幻影帧 %d）；adopted min=%s m（%d 帧）；"
                   "truth min=%s m（%d 点）" % (
                       fmt(min_real), len(real), len(fl), det.get("fp_frames"),
                       fmt(det.get("min_range_m")), len(adopted), fmt(min_truth),
                       len(truth)),
                   ok3, "detector `距离=`（真人 est↔truth≤1 m）+ trace truth"))
    return out


def judge_f2(ctx, rec):
    out = []
    tw0 = rec.get("t_world_start")
    tw1 = rec.get("t_world_end")
    rows = seg_rows(ctx, "F2")
    med_iv = ctx.env.get("frame_interval_median_s")
    # ① 出画（主口径=真人；raw/adopted 并列）
    lo = (tw0 - 2.0) if tw0 is not None else None
    hi = (tw0 + 15.4) if tw0 is not None else None
    fl = frames_in(ctx, lo, hi) if lo is not None else []
    if med_iv is not None and med_iv > F2_UNDECIDABLE_IV_S:
        out.append(_mk("F2-1", "F2", "① 最长连续出画 ≤1.0 s（真人口径）",
                       "相机帧间隔中位=%s s > %.1f s → 本判据不可判（分辨率不够）"
                       % (fmt(med_iv), F2_UNDECIDABLE_IV_S),
                       False, "detector_frames", env_fail=True))
    elif not fl:
        out.append(_mk("F2-1", "F2", "① 最长连续出画 ≤1.0 s（真人口径）",
                       "窗内 0 帧", False, "detector_frames", env_fail=True))
    else:
        runs_r = oob_runs(fl, lambda fr: bool(frame_person_real(ctx, fr)), lo, hi)
        runs_a = oob_runs(fl, frame_person_adopted, lo, hi)
        runs_raw = oob_runs(fl, FX.frame_person, lo, hi)
        worst_r = runs_r[0]["dur"] if runs_r else 0.0
        worst_a = runs_a[0]["dur"] if runs_a else 0.0
        worst_raw = runs_raw[0]["dur"] if runs_raw else 0.0
        rec["oob"] = {
            "real": [{"dur": round(r["dur"], 3), "start": round(r["start"], 2),
                      "end": round(r["end"], 2)} for r in runs_r[:5]],
            "adopted": [{"dur": round(r["dur"], 3)} for r in runs_a[:5]],
            "raw": [{"dur": round(r["dur"], 3)} for r in runs_raw[:5]],
            "worst_real_s": round(worst_r, 3), "worst_adopted_s": round(worst_a, 3),
            "worst_raw_s": round(worst_raw, 3), "frames": len(fl),
            "frame_interval_median_s": med_iv}
        out.append(_mk("F2-1", "F2", "① 最长连续出画 ≤1.0 s（真人口径，adopted/raw 并列）",
                       "真人最长=%s s（%d 段）；adopted 最长=%s s；raw 最长=%s s；"
                       "窗内 %d 帧（帧间隔中位 %s s）" % (
                           fmt(worst_r), len(runs_r), fmt(worst_a), fmt(worst_raw),
                           len(fl), fmt(med_iv)),
                       worst_r <= OOD_MAX_S, "detector_frames（物理时间）"))
    # ② 位移 + 方向
    d10 = disp(rows, tw0, tw0 + 10.0) if tw0 is not None else None
    ok2 = d10 is not None and d10["d"] >= MOVE10_MIN
    out.append(_mk("F2-2", "F2", "② 10 物理 s 内位移 ≥5 m",
                   "位移=%s m（Δ=(%s, %s)）" % (
                       fmt(None if d10 is None else d10["d"]),
                       fmt(None if d10 is None else d10["dx"]),
                       fmt(None if d10 is None else d10["dy"])),
                   ok2, "trace.csv（物理时间）"))
    cos = None
    if d10 is not None and d10["d"] > 1e-6:
        cos = (d10["dy"] * 1.0 + d10["dx"] * 0.0) / d10["d"]     # 人的方向 = +y
    ok3 = cos is not None and cos >= DIR_COS_MIN
    out.append(_mk("F2-3", "F2", "② 位移方向与人的移动方向一致（cos ≥0.707）",
                   "cos=%s（人方向=(0,+1)；机位移角=%s°）" % (
                       fmt(cos),
                       "—" if (d10 is None or d10["d"] <= 1e-6) else
                       fmt(math.degrees(math.atan2(d10["dx"], d10["dy"])))),
                   ok3, "trace.csv（物理时间）"))
    # ③ 停段（带覆盖率门：解析不到 ⇒ 零行 ⇒ 真空 PASS 是禁止的）
    ul = uav_lines(ctx, rec["mark"], rec.get("mark_end"))
    cov_ok, cov_tot = uav_coverage(ctx, rec["mark"], rec.get("mark_end"))
    cov_full = cov_tot > 0 and cov_ok == cov_tot
    win = window_lines(ul, (tw0 - 2.0) if tw0 is not None else 0.0,
                       tw1 if tw1 is not None else 1e18)
    segs = stop_segments(win)
    stops = [ln for ln in win if ln["kind"] == "stop"]
    out.append(_mk("F2-4", "F2", "③ `动作=Stop` ≤3 次（停段数；字面条数并列）",
                   "停段 %d 段 %s；停行条数=%d（窗内 `[Sim] UAV` 行 %d 条；解析 %d/%d）" % (
                       len(segs), [(s["n"], s["prev"], s["next"]) for s in segs],
                       len(stops), len(win), cov_ok, cov_tot),
                   len(segs) <= STOP_EP_MAX_F2 and cov_full,
                   "`[Sim] UAV` 行（|v|<5e-3 且 |omega|<5e-3）", env_fail=not cov_full))
    rec["f2_stop_segments"] = [{"n": s["n"], "prev": s["prev"], "next": s["next"],
                                "t0": None if s["t0"] is None else round(s["t0"], 2)}
                               for s in segs]
    # ④ 最小距离（主口径=真人）
    real, adopted, fl2 = _judge_follow_window(
        ctx, rec, "F2", (tw0 - 2.0) if tw0 is not None else 0.0,
        tw1 if tw1 is not None else 1e18)
    det = rec["det"]["F2"]
    truth = [r["d_m"] for r in rows if r.get("t_world") is not None
             and tw0 - 2.0 <= r["t_world"] <= (tw1 or 1e18) and r.get("d_m") is not None]
    min_real = det.get("real_min_range_m")
    min_truth = min(truth) if truth else None
    ok5 = (min_real is not None and min_real >= MIN_D
           and (min_truth is None or min_truth >= MIN_D))
    out.append(_mk("F2-5", "F2", "④ 最小距离 ≥2.0 m（主口径=真人）",
                   "真人帧 min=%s m（%d/%d 帧，幻影帧 %d）；adopted min=%s m（%d 帧）；"
                   "truth min=%s m（%d 点）" % (
                       fmt(min_real), len(real), len(fl2), det.get("fp_frames"),
                       fmt(det.get("min_range_m")), len(adopted), fmt(min_truth),
                       len(truth)),
                   ok5, "detector `距离=`（真人 est↔truth≤1 m）+ trace truth"))
    return out


def judge_f3(ctx, rec):
    """F3 丢失：**口径已按 prev_issues 第 5 条修正**。

    - F3-1 前置不过 = 跟随在关客户端前就把机停住 ⇒ 这是**被测代码**行为（环境门 §0 已过：
      位姿龄 70 ms 中位 / 相机 11 fps / 检测链 3.31 Hz），判 `FAIL`，不再判 `FAIL(env)`。
    - 800 ms 从 **`t_close_world`（关事件）** 起算，不再从「最后一次 adopted 帧」起算
      （后者与关事件可差几十秒，窗会落在「早就停了」的段上）。
    - 前置不过时 F3-2/F3-3 记 `N-A`（真空断言恒真，不计通过）。
    """
    out = []
    close = rec.get("t_close_world")
    lost = rec.get("t_lost_world")
    tw_end = rec.get("t_world_end")
    pre_ok = bool(rec.get("pre_ok"))
    gap = None if (close is None or lost is None) else round(close - lost, 2)
    out.append(_mk("F3-1", "F3", "前置：关客户端前机在动（末 3 条 v>0 且自起点位移 ≥1.5 m）",
                   "末 3 条 v=%s；omega=%s（解析 %s）；自 F3 起点位移=%s m；"
                   "最后一次 adopted 帧 t=%s → 关事件 t=%s"
                   "（其间 %s s 无目标、机已静止）%s" % (
                       rec.get("pre_v"), rec.get("pre_omega"),
                       rec.get("pre_uav_parsed") or "—",
                       fmt(rec.get("pre_moved_m")),
                       fmt(lost, 2), fmt(close, 2), fmt(gap),
                       "（环境门见 §0）" if pre_ok else
                       "⇒ 前置不过 ⇒ **FAIL**（跟随丢目标后 Hold，属被测代码）"),
                   pre_ok, "`[Sim] UAV` 行 + trace.csv"))
    rows = seg_rows(ctx, "F3")
    base = close if close is not None else lost
    vac = not pre_ok
    d2, t_lo, t_hi = None, None, None
    if base is not None:
        t_lo, t_hi = base + STALE_S, base + STALE_S + F3_SETTLE_S
        d2 = disp(rows, t_lo, t_hi)
    ok2 = (d2 is not None and d2["d"] <= F3_DRIFT_MAX
           and (d2["dyaw_deg"] is None or d2["dyaw_deg"] <= F3_YAW_MAX))
    out.append(_mk("F3-2", "F3", "800 ms 后机停住（[t_close+0.8, t_close+0.8+3.0] 位移 ≤0.70 m "
                                 "且 `abs(Δyaw)` ≤10°）",
                   "窗 [%s, %s] 位移=%s m；`abs(Δyaw)`=%s°；t_close=%s；%s" % (
                       fmt(t_lo, 2), fmt(t_hi, 2),
                       fmt(None if d2 is None else d2["d"]),
                       fmt(None if d2 is None else d2["dyaw_deg"]),
                       fmt(close, 2),
                       "**真空断言**：F3-1 不过（关前已静止 %s s）⇒ 断恒真，判 N-A" % fmt(gap)
                       if vac else "前置成立"),
                   ok2, "trace.csv（物理时间）", vacuous=vac))
    d3 = None
    if tw_end is not None:
        d3 = disp(rows, tw_end - 1.0, tw_end)
    ok3 = d3 is not None and d3["d"] <= F3_LAST1S_MAX
    out.append(_mk("F3-3", "F3", "末 1 物理 s 位移 ≤0.10 m（真的停住，不是慢飘）",
                   "末 1 物理 s 位移=%s m；%s" % (
                       fmt(None if d3 is None else d3["d"]),
                       "**真空断言**（同 F3-1）⇒ N-A" if vac else "前置成立"),
                   ok3, "trace.csv（物理时间）", vacuous=vac))
    _judge_follow_window(ctx, rec, "F3", rec.get("t_world_start"), tw_end)
    return out


def judge_all(ctx):
    judges = {}

    def put(lst):
        for j in lst:
            judges[j["id"]] = j

    if "m1" in ctx.cases_done:
        put(_goto_judges(ctx, ctx.cases_done["m1"], ["L1", "L2", "L3", "L4", "L5"]))
    if "m2" in ctx.cases_done:
        put(_goto_judges(ctx, ctx.cases_done["m2"], ["L6", "L7", "L8", "L9", "L10"],
                         with_omega=True))
    if "m3" in ctx.cases_done:
        put(_goto_judges(ctx, ctx.cases_done["m3"], ["L11", "L12", "L13", "L14", "L15"]))
    if "f1" in ctx.cases_done:
        put(judge_f1(ctx, ctx.cases_done["f1"]))
    if "f2" in ctx.cases_done:
        put(judge_f2(ctx, ctx.cases_done["f2"]))
    if "f3" in ctx.cases_done:
        put(judge_f3(ctx, ctx.cases_done["f3"]))
    # F4：三个跟随窗合起来的最小距离（主口径=真人）
    mins, mins_a = [], []
    for k in ("f1", "f2", "f3"):
        det = (ctx.cases_done.get(k) or {}).get("det", {}).get(k.upper())
        if det and det.get("real_min_range_m") is not None:
            mins.append(det["real_min_range_m"])
        if det and det.get("min_range_m") is not None:
            mins_a.append(det["min_range_m"])
    if mins:
        judges["F4-1"] = _mk("F4-1", "F1~F3", "全程最小距离 ≥2.0 m（三窗 min，主口径=真人）",
                             "各窗真人 min=%s；合并 min=%s m（%d 窗有真人帧）；"
                             "adopted 各窗 min=%s" % (
                                 mins, fmt(min(mins)), len(mins), mins_a),
                             min(mins) >= MIN_D, "detector `距离=`（真人 est↔truth≤1 m）")
    elif any(k in (ctx.cases_done or {}) for k in ("f1", "f2", "f3")):
        judges["F4-1"] = _mk("F4-1", "F1~F3", "全程最小距离 ≥2.0 m（三窗 min，主口径=真人）",
                             "跑过的跟随窗里没有一帧真人（adopted 各窗 min=%s）" % (mins_a,),
                             False, "detector")
    # 一个跟随用例都没跑：交给 fill_missing_judges 写 missing_reason（不伪造 FAIL）
    return judges


JUDGE_ORDER = ["L1", "L2", "L3", "L4", "L5", "L6", "L7", "L8", "L9", "L9b", "L10",
               "L11", "L12", "L13", "L14", "L15",
               "F1-1", "F1-2", "F1-3", "F2-1", "F2-2", "F2-3", "F2-4", "F2-5",
               "F3-1", "F3-2", "F3-3", "F4-1"]

JUDGE_TEXT = {
    "L1": "nav 判 Arrived（`[Nav] 到达` + 末位逐轴 ≤0.30 m）",
    "L2": "无 `Blocked`", "L3": "无急停", "L4": "无连续 `Stop + Idle` 交替",
    "L5": "无越界/撞墙", "L6": "nav 判 Arrived", "L7": "无 `Blocked`", "L8": "无急停",
    "L9": "无连续 `Stop + Idle` 交替", "L9b": "拐角不反复停走：omega 符号翻转 ≤3",
    "L10": "无越界/撞墙", "L11": "nav 判 Arrived", "L12": "无 `Blocked`",
    "L13": "无急停", "L14": "无连续 `Stop + Idle` 交替", "L15": "无越界/撞墙",
    "F1-1": "10 物理 s 内机位移 ≥5 m",
    "F1-2": "不掉队（末 5 物理 s 距离中位 ≤5.0 m 且末距离 ≤5.0 m）",
    "F1-3": "不撞人：窗内最小距离 ≥2.0 m（主口径=真人）",
    "F2-1": "① 最长连续出画 ≤1.0 s（真人口径）",
    "F2-2": "② 10 物理 s 内位移 ≥5 m",
    "F2-3": "② 位移方向与人的移动方向一致（cos ≥0.707）",
    "F2-4": "③ `动作=Stop` ≤3 次（停段数）",
    "F2-5": "④ 最小距离 ≥2.0 m（主口径=真人）",
    "F3-1": "前置：关客户端前机在动", "F3-2": "800 ms 后机停住",
    "F3-3": "末 1 物理 s 位移 ≤0.10 m",
    "F4-1": "全程最小距离 ≥2.0 m（三窗 min，主口径=真人）",
}


def judge_case_of(ident):
    """判据编号 → 用例键（用于「用例没跑完」的兜底行）。"""
    if ident.startswith("L"):
        n = int(re.sub(r"\D", "", ident) or 0)
        return "m1" if n <= 5 else ("m2" if n <= 10 else "m3")
    if ident.startswith("F1"):
        return "f1"
    if ident.startswith("F2"):
        return "f2"
    if ident.startswith("F3"):
        return "f3"
    return "f4"


def fill_missing_judges(ctx, judges):
    """兜底：`JUDGE_ORDER` 里任何缺失 id 都要生成一行（实测值 = missing_reason）。

    这是对 `prev_issues.md` 第 1 条（27 行只出 17 行）的结构性锁：只要某个用例在某一步
    抛了异常/超时/没被 `--cases` 选中，§3 仍恒有 `len(JUDGE_ORDER)` 行。
    """
    failed = getattr(ctx, "cases_failed", {}) or {}
    for ident in JUDGE_ORDER:
        if ident in judges:
            continue
        case = judge_case_of(ident)
        if case == "f4":
            why = "；".join("%s=%s" % (k.upper(), (failed.get(k) or {}).get("error")
                                         or ("已跑" if k in (ctx.cases_done or {}) else "未运行"))
                            for k in ("f1", "f2", "f3"))
        else:
            rec = failed.get(case) or {}
            why = rec.get("error") or (
                "用例 %s 未运行（--cases 未包含 / 总预算超时）" % case.upper())
        judges[ident] = _mk(ident, "F1~F3" if case == "f4" else case.upper(),
                            JUDGE_TEXT.get(ident, "（判据）"),
                            "未取到实测值：%s" % why, False, "harness", env_fail=True)
    return judges


# ================================================================ 产物

TRACE_COLS = ["t_wall", "t_mono", "t_world", "seg", "x", "y", "z", "yaw", "nav_status",
              "task_id", "task_ticks", "queue_len", "mode", "ped_x", "ped_y", "d_m",
              "ped_connected", "pose_age_ms"]


def write_csvs(ctx):
    with (ctx.out / "trace.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=TRACE_COLS, extrasaction="ignore")
        w.writeheader()
        for row in ctx.rows:
            w.writerow({c: row.get(c) for c in TRACE_COLS})
    with (ctx.out / "detector_frames.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["wall", "t_world", "world_ms", "seq", "infer_ms", "dets",
                    "pose_x", "pose_y", "pose_z", "pose_yaw",
                    "person_conf", "person_adopted", "person_range_m",
                    "est_x", "est_y", "fp_classes", "bbox_bottom", "is_real_person"])
        for fr in ctx.frames:
            tw = ctx.clock.at(float(fr["wall"]))
            p = FX.frame_person(fr)
            pa = frame_person_adopted(fr)
            w.writerow([round(fr["wall"], 3), None if tw is None else round(tw, 3),
                        fr["world_ms"], fr["seq"], fr["infer_ms"], len(fr["dets"]),
                        fr["pose"][0], fr["pose"][1], fr["pose"][2], fr["pose"][3],
                        None if p is None else p["conf"],
                        pa is not None,
                        None if pa is None else pa["range_m"],
                        None if (p is None or p["est"] is None) else p["est"][0],
                        None if (p is None or p["est"] is None) else p["est"][1],
                        "|".join(fr["fp"]),
                        1 if (p is not None and p["bbox"][3] >= fr["height"] - 1) else 0,
                        1 if frame_person_real(ctx, fr) else 0])
    with (ctx.out / "ped_trace.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["mono", "x", "y", "world_time", "t_world_calc"])
        for p in (ctx.ped.snapshot() if ctx.ped else []):
            tw = ctx.clock.at(p[0])
            w.writerow([round(p[0], 3), round(p[1], 3), round(p[2], 3), p[3],
                        None if tw is None else round(tw, 3)])


def write_json_artifacts(ctx, judges, pre, s0, overall, reason):
    data = dict(ctx.env)
    data.update({
        "utc": now_utc_stamp(),
        "cwd": str(ROOT),
        "sim1_config": str(CFG),
        "sim_binary_sha256": sha256_file(INST / "pleiades-sim"),
        "target_binary_sha256": sha256_file(SimBinary),
        "reuse_script": {"path": "Tool/sim_follow_test.py", "sha256": REUSE_SHA256},
        "world_argv": " ".join(world_argv_fast(ctx.world_dir)),
        "world_dir": str(ctx.world_dir),
        "world_override": OVERRIDE_TEXT,
        "world_copy": getattr(ctx, "world_copy", {}),
        "sim_argv": FX.SIM_ARGV,
        "display": DISPLAY,
        "control_port": ctx.control_port,
        "world_port": WORLD_PORT,
        "clock": {"points": len(ctx.clock.pts), "rewinds": ctx.clock.rewinds,
                  "k_env": ctx.env.get("k_world_over_wall")},
        "stop_semantics": "`[Sim] UAV` 行 v==0 且 omega==0 = 停（旧 `动作=Stop` 的语义等价）",
        "static_pedestrians": FX.static_pedestrians(),
        "s0": s0,
        "preflight": pre,
        "cases": {k: {kk: vv for kk, vv in v.items() if kk not in ("det",)}
                  for k, v in (ctx.cases_done or {}).items()},
        "detectors": {k: v.get("det") for k, v in (ctx.cases_done or {}).items()},
        "judges": judges,
        "overall": overall,
        "reason": reason,
        "shots": ctx.shots,
        "residual": ctx.residual,
    })
    if getattr(ctx, "regrade_src", None):
        man = {}
        try:
            man = json.loads((pathlib.Path(ctx.out) / "source_manifest.json").read_text(
                encoding="utf-8"))
        except (OSError, ValueError):
            man = {}
        data["regrade"] = {"source": str(ctx.regrade_src), "manifest": man}
    (ctx.out / "env.json").write_text(json.dumps(data, ensure_ascii=False, indent=2),
                                      encoding="utf-8")
    (ctx.out / "verdict.json").write_text(
        json.dumps({"overall": overall, "reason": reason, "judges": judges,
                    "utc": data["utc"]}, ensure_ascii=False, indent=2), encoding="utf-8")
    return data


def _table(headers, rows):
    out = ["| " + " | ".join(headers) + " |",
           "|" + "|".join(["---"] * len(headers)) + "|"]
    for r in rows:
        out.append("| " + " | ".join(str(c) for c in r) + " |")
    return "\n".join(out)


def write_verdict(ctx, judges, pre, s0, overall, reason, env_fail=None):
    L = []
    A = L.append
    A("# task_39 Step 3 — 验证（常规移动 + 跟随）\n")
    A("> 生成时间（UTC）：%s　　脚本：`Tool/sim_task39_step3_test.py`（只验不改）\n" % now_utc_stamp())
    A("## 0 环境与口径\n")
    wcopy = getattr(ctx, "world_copy", {}) or {}
    probe = (ctx.env.get("probe_env") or [])
    rows0 = []
    regrade_src = getattr(ctx, "regrade_src", None)
    if regrade_src:
        rows0 += [
            ["数据来源（**复算**，未重跑世界/仿真）",
             "`%s`（trace.csv/detector_frames.csv/sim_trace.log 的 sha256 见 §13）"
             % regrade_src],
            ["复算等价性",
             "见 §11（逐行差异）与 §12（覆盖率/`[flight_ctrl]`/28 行齐全/差异条数）；"
             "契约 = 当轮 0 diff，历史目录只登记差异（`REGRADE_LEGACY`）"],
        ]
    rows0 += [
        ["git HEAD", FX.run_capture(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"])[1].strip()],
        ["git dirty 行数", len(ctx.git_before)],
        ["sim1 配置", str(CFG.relative_to(ROOT))],
        ["sim1 二进制 sha256", sha256_file(INST / "pleiades-sim")[:32] + "…"],
        ["target 二进制 sha256", sha256_file(SimBinary)[:32] + "…"],
        ["复用脚本", "Tool/sim_follow_test.py sha256=%s…（本步未改）" % REUSE_SHA256[:24]],
        ["world argv", " ".join(world_argv_fast(ctx.world_dir))],
        ["world 目录", "%s（%s）" % (ctx.world_dir.relative_to(ROOT),
                                  wcopy.get("reason") or "—")],
        ["world 副本来源", "%s（%.2f s，stamp=%s）" % (
            wcopy.get("reason") or "—", wcopy.get("seconds") or 0.0,
            (wcopy.get("stamp") or "—").replace("|", "/"))],
        ["world override.cfg", OVERRIDE_TEXT.replace("\n", " ⏎ ")],
        ["sim argv", " ".join(FX.SIM_ARGV) + "（cwd=RobotSim_TestEnv/sim1）"],
        ["DISPLAY", DISPLAY],
        ["Xvfb", "复用已在跑的 :99（收尾不杀）" if getattr(ctx, "xvfb_reused", False)
         else "自起 :99（收尾 killpg）"],
        ["相机实测 Hz", fmt(ctx.env.get("camera_hz"))],
        ["相机帧间隔中位", "%s s" % fmt(ctx.env.get("frame_interval_median_s"))],
        ["detector 推理中位", "%s ms ⇒ 相机/检测链上限 %.2f Hz（属被测代码，不可调）" % (
            fmt(ctx.env.get("s0j_infer_ms_median"), 0),
            1000.0 / ctx.env["s0j_infer_ms_median"]
            if ctx.env.get("s0j_infer_ms_median") else 0.0)],
        ["位姿龄 中位/最大（S0j）", "%s / %s ms（%s 行；实时门 ≤%.0f ms → %s）" % (
            fmt(ctx.env.get("pose_age_ms_median"), 1), fmt(ctx.env.get("pose_age_ms_max"), 0),
            ctx.env.get("pose_age_n"), POSE_AGE_RT_MAX_MS,
            "过" if ctx.env.get("rt_ok") else "**未过（只记 warn）**")],
        ["k = Δ物理/Δ墙钟", fmt(ctx.env.get("k_world_over_wall"), 4)],
        ["空场 person 帧", "%s / %s（S0j，空场无 person 检测 ⇒ 幻影只在有人的用例里出现）" % (
            ctx.env.get("s0j_person_frames"), ctx.env.get("s0j_frames"))],
        ["控制口 / 世界口", "%d / %d" % (ctx.control_port, WORLD_PORT)],
        ["物理时间源", "detector `frame_ts_ms`（按墙钟插值成 wall→world 映射，%d 点）"
         % len(ctx.clock.pts)],
    ]
    A(_table(["项", "值"], rows0))
    if probe:
        A("\n**渲染旋钮实测（`--probe-env`，同一台机器、各 40 帧、Xvfb :99 + llvmpipe）**\n")
        A(_table(["世界", "msaa_3d", "墙钟/40 帧", "末秒 FPS", "FPS min", "FPS max"], [
            [r.get("world"), r.get("value"), "%s s" % fmt(r.get("wall_40f"), 2),
             fmt(r.get("fps_printed"), 1), fmt(r.get("fps_min"), 1),
             fmt(r.get("fps_max"), 1)] for r in probe]))
        f4 = next((r.get("fps_printed") for r in probe
                   if str(r.get("world")) == "orion-simulation-world"), None)
        f0 = next((r.get("fps_printed") for r in probe
                   if str(r.get("world")) == "RobotSim_TestEnv/world_fast"), None)
        ratio = (f0 / f4) if (f0 and f4) else None
        A("\nMSAA 0 vs 4×：FPS %s → %s（%s×）。这是本轮唯一的改动，且只影响画质，"
          "判据不依赖画面质量（`env_probe.csv`）。\n" % (
              fmt(f4, 1), fmt(f0, 1), fmt(ratio, 1)))
    A("\n**Stop 口径映射**（Step 2 之后日志已无 `动作=Stop` 字段）："
      "`[Sim] UAV … → v=… omega=…`（`sim_handler.rs:750`，5 Hz 限流）里 "
      "**`v==0 且 `omega==0` = 停**（`next=None`）；`v==0 且 omega!=0` = 原地转（不算停）。"
      "「停段」= 连续「停」行的极大段；`动作=Stop` 的**字面条数**并列给出。\n")
    A("**口径**：所有时长判据用**物理时间**（§五 的「10 s」「800 ms」同 task_38 口径，"
      "本机 llvmpipe 软渲染下物理推进 k=%s，墙钟值只并列不参判）；"
      "**`[Sim] UAV` 行 5 Hz 墙钟限流 ⇒ 看不到 <200 ms 的瞬停**，这是分辨率上限（属被测代码）；"
      "**detector 双口径**：`真人`（`est` 与行人真值 ≤%.1f m 且 bbox 底边不触图像底边）为主口径、"
      "`adopted`（follow.rs 实际吃到的框）并列——空场幻影见 §5。\n"
      % (fmt(ctx.env.get("k_world_over_wall"), 4), REAL_PERSON_TOL_M))
    if env_fail:
        A("## 结论\n\n**%s** — %s\n" % (overall, reason))
    A("## 1 前置自检（%d 条%s）\n" % (
        len(pre), "，取证轮" if getattr(ctx, "regrade_src", None) else ""))
    A(_table(["#", "name", "ok", "detail"],
             [[i + 1, c["name"], "OK" if c["ok"] else "**FAIL**", c["detail"]]
              for i, c in enumerate(pre)]))
    gates = task_gates()
    A("\n### 1.1 任务级遗留门（**不进 §3 判据、不阻断运行**）\n")
    A(_table(["#", "门", "状态", "实测"],
             [[i + 1, g["name"], "OK" if g["ok"] else "**未过**", g["detail"]]
              for i, g in enumerate(gates)]))
    A("\n> 这两条是「本轮无权修、必须由允许改对应 crate 的 step 补齐」的既有缺口；"
      "§6.5 给出补法与断言原文。\n")
    A("\n## 2 时序\n")
    rows = []
    if s0:
        for a in s0.get("anchors", []):
            rows.append(["S0 锚点", a["name"], "%.2f s" % a["seconds"],
                         "OK" if a["ok"] else "**MISSING**", (a["line"] or "")[:90]])
        rows.append(["S0 status", "9 条断言",
                     "—", "%d/%d" % (sum(1 for c in s0["asserts"] if c["ok"]),
                                     len(s0["asserts"])), ""])
    for k in ("f1", "f2", "f3", "m1", "m2", "m3"):
        rec = (ctx.cases_done or {}).get(k)
        if not rec:
            continue
        rows.append([rec["name"], "墙钟",
                     "%s s" % fmt(rec.get("wall_s"), 1), "—",
                     "物理 %s s，k=%s" % (fmt(rec.get("phys_s"), 1), fmt(rec.get("k")))])
        if rec.get("kind") == "goto":
            rows.append([rec["name"], "goto %s" % (rec["target"],), "—",
                         "到达" if rec.get("done") else "**未到达**",
                         str(rec.get("analysis", {}).get("dxy"))])
        else:
            rows.append([rec["name"], "人位移",
                         "%s m" % fmt(rec.get("ped_track_m")), "—",
                         "人终点=%s" % (rec.get("ped_end"),)])
    A(_table(["阶段", "项", "墙钟", "结果", "备注"], rows))
    A("\n## 2.1 归位与目视几何（不进判据，只解释 F 用例的起跑条件）\n")
    grow = []
    for k in ("f1", "f2", "f3"):
        rec = (ctx.cases_done or {}).get(k)
        if not rec:
            continue
        for t in ((rec.get("prep") or {}).get("tries") or []):
            grow.append([rec["name"], "#%d→%s" % (t.get("attempt"), t.get("target")),
                         t.get("result"), str(t.get("dxy")),
                         "是" if t.get("hard") else ("宽松是" if t.get("loose") else "**否**"),
                         str(t.get("fail_code")), t.get("wall_s")])
        fc = rec.get("face") or {}
        for r in (fc.get("rounds") or []):
            grow.append([rec["name"], "朝向闭环 第%s轮" % r.get("round"),
                         "收敛" if abs(r.get("bearing_end") or 999) <= 20.0 else "**未收敛**",
                         "yaw %s°→%s°；方位差 %s°→%s°" % (
                             r.get("yaw_start"), r.get("yaw_end"),
                             r.get("bearing_start"), r.get("bearing_end")),
                         "mode=%s" % r.get("mode"), "", r.get("wall_s")])
        v = rec.get("vantage") or {}
        grow.append([rec["name"], "目视几何", "OK" if v.get("ok") else "**不足**",
                     "range=%s m bearing=%s°" % (fmt(v.get("range_m")), fmt(v.get("bearing_deg"), 1)),
                     "机=(%s, %s)" % (fmt(v.get("uav", [None])[0]), fmt(v.get("uav", [None, None])[1])),
                     "", ""])
    A(_table(["用例", "归位跳", "结果", "Δ/几何", "硬到达", "原因码", "墙钟 s"], grow))
    A("\n> `PREP_ARRIVE_LOOSE_M=%.2f`、`VANTAGE_RANGE_MAX_M=%.1f`、`VANTAGE_BEARING_MAX_DEG=%.1f`："
      "归位失败/几何不足**不作为跳过用例的理由**，F1~F3 照跑、判据照实判。\n" % (
          PREP_ARRIVE_LOOSE_M, VANTAGE_RANGE_MAX_M, VANTAGE_BEARING_MAX_DEG))
    A("\n## 3 判据（%d 行）\n" % len(JUDGE_ORDER))
    jrows = []
    for ident in JUDGE_ORDER:
        j = judges.get(ident)
        if not j:
            continue
        jrows.append([ident, j["case"], j["text"], j["measured"],
                      "PASS" if j["pass"] else (
                          "**N-A（真空）**" if j.get("vacuous")
                          else "**%s**" % j["verdict"]), j["basis"]])
    A(_table(["编号", "用例", "判据", "实测值", "判定", "口径"], jrows))
    A("\n判据行数：%d（计划 27：M 15 + F1 3 + F2 5 + F3 3 + F4 1；"
      "M2 的 `L9b` 是 `L9` 的 omega 子项，单独成行便于评审核对 ⇒ 实际 %d 行）\n" % (
          len(jrows), len(JUDGE_ORDER)))
    if len(jrows) != len(JUDGE_ORDER):
        raise RuntimeError("§3 判据行数 %d ≠ %d（兜底失败）" % (len(jrows), len(JUDGE_ORDER)))
    A("## 4 实测数据（逐用例）\n")
    for k in ("f1", "f2", "f3", "m1", "m2", "m3"):
        rec = (ctx.cases_done or {}).get(k)
        if not rec:
            continue
        A("### %s\n" % rec["name"])
        an = rec.get("analysis") or {}
        if rec.get("kind") == "goto":
            A(_table(["项", "值"], [
                ["目标", str(rec["target"])],
                ["起点", "(%s, %s)" % (fmt(rec["start"].get("x")), fmt(rec["start"].get("y")))],
                ["终点", "(%s, %s) yaw=%s" % (fmt(rec["end"].get("x")), fmt(rec["end"].get("y")),
                                              fmt(rec["end"].get("yaw")))],
                ["Δ 到目标", str(an.get("dxy"))],
                ["nav 到达行", rec.get("nav_line") or "（无）"],
                ["停段", str(an.get("stop_segments"))],
                ["微循环段数", an.get("micro")],
                ["omega 符号翻转", an.get("omega_flips")],
                ["`[Nav] 连续 N 拍未算完`", an.get("notready")],
                ["路径长 / 直线 / 比", "%s / %s / %s" % (
                    fmt((an.get("paths") or {}).get("path_m")),
                    fmt((an.get("paths") or {}).get("straight_m")),
                    fmt((an.get("paths") or {}).get("ratio")))],
                ["单步最大位移", "%s m（Δt≤%.2f s；未过滤 %s m @ Δt=%s s）" % (
                    fmt((an.get("paths") or {}).get("step_max")), STEP_DT_MAX_S,
                    fmt((an.get("paths") or {}).get("step_max_raw")),
                    fmt((an.get("paths") or {}).get("step_dt_raw")))],
                ["任务失败原因码（1=Timeout）", an.get("fail_code")],
                ["位姿龄 中位/最大（ms）", "%s / %s" % (
                    fmt(an.get("pose_age_ms_median"), 1), an.get("pose_age_ms_max"))],
                ["实测最大速度（1 s 窗）", "%s m/s" % fmt(an.get("speed_max_1s"))],
                ["路径总长", "%s m" % fmt(an.get("path_total_m"))],
                ["yaw 跨度（min~max）", "%s° ~ %s°（%s°）" % (
                    an.get("yaw_min"), an.get("yaw_max"), an.get("yaw_span"))],
            ]))
        else:
            det = (rec.get("det") or {})
            A(_table(["项", "值"], [
                ["起点", "(%s, %s)" % (fmt(rec["start"].get("x")), fmt(rec["start"].get("y")))],
                ["终点", "(%s, %s)" % (fmt(rec["end"].get("x")), fmt(rec["end"].get("y")))],
                ["人位移", "%s m" % fmt(rec.get("ped_track_m"))],
                ["物理时长", "%s s" % fmt(rec.get("phys_s"), 1)],
                ["墙钟时长", "%s s" % fmt(rec.get("wall_s"), 1)],
                ["detector 真人命中率", str({kk: vv.get("real_hit_rate")
                                              for kk, vv in det.items()})],
                ["detector 真人 min 距离", str({kk: vv.get("real_min_range_m")
                                                for kk, vv in det.items()})],
                ["detector adopted 命中率", str({kk: vv.get("hit_rate")
                                                  for kk, vv in det.items()})],
                ["幻影帧（有 person 框但不对真人）", str({kk: vv.get("fp_frames")
                                                         for kk, vv in det.items()})],
                ["出画（真人）", str((rec.get("oob") or {}).get("worst_real_s"))],
                ["出画（adopted）", str((rec.get("oob") or {}).get("worst_adopted_s"))],
                ["出画（raw）", str((rec.get("oob") or {}).get("worst_raw_s"))],
                ["F2 停段", str(rec.get("f2_stop_segments"))],
                ["关客户端前 v", str(rec.get("pre_v"))],
                ["关前位移", fmt(rec.get("pre_moved_m"))],
                ["关客户端前后世界动态行人", "%s → %s" % (
                    rec.get("alive_before_close"),
                    (rec.get("ped_close") or {}).get("alive_after"))],
            ]))
        A("")
    A("## 5 观察项 / 偏差\n")
    obs = []
    first_goto = next((ctx.cases_done[k] for k in ("m1", "m2", "m3")
                       if k in (ctx.cases_done or {})), None)
    # ── 5.1 detector 幻影（三桶；数字全部来自 person_frame_breakdown，不写死）
    pfb = person_frame_breakdown(ctx)
    if pfb["person"]:
        off = pfb["est_off"]
        off_rng = ("%s~%s m" % (fmt(min(x["d_m"] for x in off)), fmt(max(x["d_m"] for x in off)))
                   if off else "—")
        off_case = {}
        for x in off:
            off_case[x["seg"]] = off_case.get(x["seg"], 0) + 1
        bc = pfb["by_case"]
        m_sum = sum(v for k, v in bc.items() if k in ("M1", "M2", "M3"))
        obs.append(
            "- **detector `person` 框总量**：全轮 %d 帧里有 `person` 框 **%d** 帧"
            "（%d 帧无 person）；按用例：%s；**M1~M3 合计 %d 帧**"
            "（常规移动用例几乎不出现 `person` 框）⇒ 幻影集中在有行人的用例里，"
            "`trace.csv`/`detector_frames.csv` 可核。" % (
                pfb["frames"], pfb["person"], pfb["frames"] - pfb["person"],
                "、".join("%s=%d" % (k, v) for k, v in sorted(bc.items())), m_sum))
        obs.append(
            "- **① 有 `est` 的 %d 帧 = follow 会吃的帧**（`adopted` %d 帧）：真人 **%d**、"
            "`est`↔真值偏差>1 m 的 **%d**（%s；分布 %s）、bbox 触图像底边 y=%d 的 **%d**"
            "（≈2.5 m、无近处真值 ⇒ 真幻影）、无真值 %d。分桶互斥且穷尽：%d+%d+%d+%d=%d"
            "（不变式=%s）。" % (
                pfb["with_est"], pfb["adopted"], pfb["real"], len(off), off_rng,
                "、".join("%s=%d" % (k, v) for k, v in sorted(off_case.items())) or "—",
                479, pfb["bottom_edge"], pfb["no_truth"],
                pfb["real"], pfb["bottom_edge"], pfb["no_truth"], len(off),
                pfb["with_est"], "成立" if pfb["invariant_ok"] else "**不成立**"))
        obs.append(
            "- **② 仅 bbox 的 %d 帧 = 无 `est`/`adopted` ⇒ 进不了 follow**：拒答原因 %s；"
            "时间窗 [%s, %s]，其中 %d 帧在 F3 关事件 t=%s **之前**"
            "（纠正上一轮「关后仍有 86 帧」的读法）。成因：行人沿 +x 走远、框底边升到画面"
            "中线以上 ⇒ `框底边不在画面下半部` 被投影门拒答。" % (
                pfb["bbox_only"],
                "、".join("%s %d" % (k, v) for k, v in sorted(pfb["reject"].items())) or "—",
                fmt(pfb["bbox_t0"], 2), fmt(pfb["bbox_t1"], 2),
                pfb["bbox_before_close"], fmt(pfb["close_world"], 2)))
        if pfb["post_close_est"]:
            q = pfb["post_close_est"][0]
            obs.append(
                "- **③ 关事件之后仍有 `est` 的帧：%d 帧**（首帧 t=%s、range=%s m、"
                "底边触底=%s、adopted=%s、est=%s、机位=%s）⇒ 真值消失后仍能给出 ≈2.5 m 的幻影框。"
                "本例落在 F3 采样窗之后、未污染判据；但 `follow.rs` 取 `range_m` 最小者 ⇒ "
                "下一轮必须防。" % (
                    len(pfb["post_close_est"]), fmt(q["t_world"], 1), fmt(q["range_m"]),
                    "是", "是" if q["adopted"] else "否", q["est"], q["uav"]))
        else:
            obs.append("- **③ 关事件之后没有新的 `est` 帧**（如实记录）。")
        obs.append(
            "- **④ 静态世界行人 2 人**（World.gd `PEDESTRIANS`：(44,96)/(52,102)）："
            "S0j 空场 24 帧 `person` 框 **%s** 个（`env.json`）⇒ 空场无 person 误检；"
            "两人距所有用例轨迹 >12 m = detector `max_range_m` ⇒ 即使进画也被 `超出最大距离` "
            "拒答、不可能被 adopted。" % ctx.env.get("s0j_person_frames"))
    else:
        obs.append("- **detector 幻影**：本轮没有出现 `person` 框（如实记录）。")
    if first_goto:
        an = first_goto.get("analysis") or {}
        fc_v = (flight_ctrl_cfg(FX.load_config()) if not getattr(ctx, "regrade_src", None)
                else (ctx.regrade_env or {}).get("flight_ctrl"))
        if not fc_v:
            try:
                fc_v = flight_ctrl_cfg(FX.load_config())
            except Exception:                            # noqa: BLE001  读不到就照实写
                fc_v = None
        zt = turn_zone_text((an.get("pose_age_ms_median") or 0) / 1000.0, fc_v)
        cap_v, cap_w = world_caps()
        obs.append(
            "- **`[Sim] UAV` 行与动力学**：`%s` 实测最大速度 %s m/s（1 s 窗）、"
            "位姿龄中位 %s ms；机栈执行器读 `[flight_ctrl]` `vel_fwd=%s m/s`、"
            "`yaw_rate_deg=%s °/s`（世界侧 `Quadrotor.gd` 上限 `clampf(v,±%s)` m/s、"
            "`clampf(omega,±%s)` rad/s ⇒ **%s**），执行器 20 Hz 闭环 ⇒ "
            "`yaw_rate_deg` × 位姿龄 ≈ **%s°/拍**，%s ⇒ **%s**（来源：`uav_sim/executor.rs` + "
            "`[flight_ctrl]` + `chassis/Quadrotor.gd`）。" % (
                first_goto["name"], fmt(an.get("speed_max_1s")),
                fmt(an.get("pose_age_ms_median"), 1),
                fmt((fc_v or {}).get("vel_fwd"), 2), fmt((fc_v or {}).get("yaw_rate_deg"), 1),
                fmt(cap_v, 1), fmt(cap_w, 2),
                "不截断" if (cap_v is None or (fc_v or {}).get("vel_fwd") is None
                            or abs((fc_v or {}).get("vel_fwd")) <= cap_v) else "**被截断**",
                fmt(zt["deg"], 1), zt["cmp"], zt["band"]))
    # ── 5.1b 附加观测（不进 §3）：初/末距离、dy 投影、机速 vs 人速、M2 路径形状
    obs.extend(extra_observations(ctx, judges))
    # ── 5.2 detector 2.43 Hz 上限
    inf = ctx.env.get("s0j_infer_ms_median")
    if inf:
        obs.append(
            "- **detector 推理 %s ms ⇒ 相机/检测链上限 %.2f Hz**（实测帧间隔中位 %s s）："
            "这是**被测代码**（`sim/detector` 单帧 CPU 推理）的硬约束，不是渲染问题，"
            "本步只记不改。F2-1「最长连续出画 ≤1 s」在该帧率下只有约 %.1f 帧的判定余量；"
            "帧间隔中位 >%.1f s 时该判据记为不可判（`FAIL(env)`）。" % (
                fmt(inf, 0), 1000.0 / inf, fmt(ctx.env.get("frame_interval_median_s")),
                1.0 / max(1e-9, ctx.env.get("frame_interval_median_s") or 1.0),
                F2_UNDECIDABLE_IV_S))
    # ── 5.3 MSAA 旋钮
    if probe:
        f4 = next((r.get("fps_printed") for r in probe
                   if str(r.get("world")) == "orion-simulation-world"), None)
        f0 = next((r.get("fps_printed") for r in probe
                   if str(r.get("world")) == "RobotSim_TestEnv/world_fast"), None)
        obs.append(
            "- **MSAA 旋钮（本轮唯一的环境改动）**：上一轮把 2.2 fps 归因为「llvmpipe 不可救」，"
            "实际只是世界的 4× MSAA；副本 `override.cfg` 关掉后同机 FPS %s → %s"
            "（%.1f×；`env_probe.csv`，§0 表）。世界源码零改动（preflight 19 `diff -r` 为空）。" % (
                fmt(f4, 1), fmt(f0, 1), (f0 / f4) if (f0 and f4) else 0.0))
    obs.append("- **「无急停」是弱判据**：本环境无 lidar（S0 锚点 `[Sim] 无 lidar（未配置 lidar_2d）"
               "：on_tick 跳过急停`），机栈本就不挂 2D 急停 ⇒ L3/L8/L13 恒真。")
    obs.append("- **`[Sim] UAV` 5 Hz 限流** ⇒ 停段判据（L4/L9/L14、F2-4）分辨率上限 200 ms 墙钟。")
    if (ctx.clock.rewinds or 0) > 0:
        obs.append("- 物理钟出现 %d 次世界时间回退（已忽略该点）。" % ctx.clock.rewinds)
    leaks = [k for k in (ctx.env or {}) if k.startswith("ped_leak_before_") and (ctx.env or {})[k]]
    if leaks:
        obs.append("- **行人客户端残留**：%s（`ped_alive_count()` 门禁记录，见 §2.1/§4）。" % (
            {k: ctx.env[k] for k in leaks},))
    A("\n".join(obs) + "\n")
    A("## 6 结论\n")
    n_pass = sum(1 for j in judges.values() if j["pass"])
    n_fail = sum(1 for j in judges.values() if j["verdict"] == "FAIL")
    n_env = sum(1 for j in judges.values() if j["verdict"] == "FAIL(env)")
    n_na = sum(1 for j in judges.values() if j.get("vacuous"))
    A("**%s** — %d 行：PASS %d / FAIL %d / FAIL(env) %d / N-A %d。%s\n" % (
        overall, len(judges), n_pass, n_fail, n_env, n_na, reason))
    for ident in JUDGE_ORDER:
        j = judges.get(ident)
        if j and not j["pass"]:
            A("* `%s` %s — 实测：%s" % (
                ident, "N-A（真空）" if j.get("vacuous") else j["verdict"], j["measured"]))
    A("")
    A("**根因初判（代码级，只作初判不改代码）**\n")
    an0 = (first_goto or {}).get("analysis") or {}
    age_s = (an0.get("pose_age_ms_median") or 0) / 1000.0
    try:
        fc6 = flight_ctrl_cfg(FX.load_config()) or {}
    except Exception:                                    # noqa: BLE001  读不到就照实写
        fc6 = {}
    zt = turn_zone_text(age_s, fc6)
    cap_v, cap_w = world_caps()
    A("1. **一拍的转角取决于 `[flight_ctrl].yaw_rate_deg`，不是 `turn_speed`**："
      "`yaw_rate_deg=%s °/s` × `位姿龄` 中位 %.3f s ≈ **%s°**，%s ⇒ **%s**"
      "（见 `pleiades-sim/src/sim/uav_sim/executor.rs` 的三档全有全无控制律）。"
      "上一轮在 `90°/s` 口径下得到的「每拍 8.6° > 死区 ⇒ 极限环」结论**在本轮口径下不成立**；"
      "连带的极限环叙述随口径修正一并撤销。" % (
          fmt(fc6.get("yaw_rate_deg"), 1), age_s, fmt(zt["deg"], 1), zt["cmp"], zt["band"]))
    r_arc = None
    if fc6.get("vel_fwd") and fc6.get("yaw_rate_deg"):
        r_arc = abs(fc6["vel_fwd"]) / math.radians(abs(fc6["yaw_rate_deg"]))
    A("2. **最小转弯半径是几何量**：`r = v/ω = %s m`（`vel_fwd=%s m/s` ÷ `yaw_rate_deg=%s °/s`），"
      "而 `follow.rs` 一拍只给**下一格格心**（≤0.5 m）；`r` 远大于格距 ⇒ 每次换格都要画一段"
      "比 0.5 m 大得多的弧，`goto` 的 `Timeout` 装饰器 60 s（`FailCode::Timeout` 码值 1）"
      "在长路径上必然到期。世界侧上限 `clampf(v,±%s)` / `clampf(omega,±%s)` m/s ⇒ **%s**。" % (
          fmt(r_arc, 2), fmt(fc6.get("vel_fwd"), 2), fmt(fc6.get("yaw_rate_deg"), 1),
          fmt(cap_v, 1), fmt(cap_w, 2),
          "不截断" if (cap_v is None or fc6.get("vel_fwd") is None
                      or abs(fc6["vel_fwd"]) <= cap_v) else "**被世界截断**"))
    A("3. 跟随链上还有两处**属被测代码**的硬约束：detector 单帧推理 %s ms（上限 %.2f Hz）、"
      "`[Sim] UAV` 行 5 Hz 墙钟限流。二者与执行器 20 Hz 闭环不匹配，"
      "是 F2「切线通过」跟不住的直接来源；本步按 step.md「不过就如实记」，不调阈值、不改代码。\n" % (
          fmt(inf, 0), (1000.0 / inf) if inf else 0.0))
    # ── §6.4 follow.rs 机制与实测的因果
    iv = ctx.env.get("frame_interval_median_s")
    A("4. **`follow.rs` 的取值机制也在同一条链上**：目标 = 人所在 0.5 m 格的格心（`step_cell`），"
      "只有 `range > D_FAR=%.1f m` 才下发、`range < D_NEAR=%.1f m` 就停（常量见 `%s`），"
      "且 `STALE_MS=%d` 判过期。实测帧间隔中位 %s s ⇒ 800 ms 内只有约 %.1f 帧，"
      "在 %.2f Hz 检测链上「跟住」的字面含义就是每 %s s 才更新一次目标；"
      "叠加第 2 条的弧线几何，F1 的 10 s 位移只有 %s m、F2 只有 %s m、方向 cos=%s。\n" % (
          D_FAR, D_NEAR, "`follow.rs:64/66/68`", int(STALE_S * 1000),
          fmt(iv), (STALE_S / iv) if iv else 0.0, (1.0 / iv) if iv else 0.0, fmt(iv),
          _mv(judges, "F1-1", "位移"), _mv(judges, "F2-2", "位移"),
          _mv(judges, "F2-3", "cos")))
    A("5. **判据与场景几何的两处不自洽**（不是代码缺陷，本轮按权限只记录）："
      "① `F1-2` 要求末距离 ≤%.1f m，而机速上限 `vel_fwd=%s m/s` < 人速 `%s m/s` ⇒ "
      "间距单调拉大（算术见 §5 附加观测）；② `F2-3` 要求 10 s 位移方向与人的 +y 一致"
      "（cos ≥%.3f），而人沿 +y 横穿、机从侧向切入 ⇒ 净位移方向由「指向人」的方位决定。"
      "两条都要人类决定（提高 `vel_fwd` / 降人速 / 改判据口径），**不在本 step 权限内**。\n" % (
          F1_D_END_MAX, fmt(fc6.get("vel_fwd"), 2), fmt(PED_F1_SPEED, 2), DIR_COS_MIN))
    A("\n### 6.5 遗留/越界登记（**本 step 无权修**）\n")
    gates = {c["name"]: c for c in task_gates()}
    gp = gates.get("no_path_unit_tests_4of4") or {}
    gu = gates.get("existing_unit_tests") or {}
    A("- **1a `NoPath` 单测覆盖**：%s。期望断言：**收敛且无路 → `StepResult::NoPath`**"
      "（不是 `NotReady`），4 份 `pathfinder.rs` 各一条 `fn test_*no_path*()`；"
      "缺口必须由允许改 `pleiades-uav/`、`pleiades-ugv/` 的 step 补齐。" % (gp.get("detail") or "—"))
    A("- **既有单测**：%s；两条红在 HEAD 即红（`MountOut` 现有 5 键含 `pitch`，"
      "断言仍期望 4 键），`parts.rs` 本任务零改动，非本任务引入。" % (gu.get("detail") or "—"))
    if getattr(ctx, "regrade_src", None):
        A("- **本轮为复算、未重跑世界/仿真**（方法与等价性见 §13）；只验不改："
          "`pleiades-sim/`、世界、判据阈值一行未动。")
    A("")
    A("\n## 7 产物清单\n")
    prow = []
    for p in sorted(ctx.out.iterdir()):
        if p.is_file():
            n = sum(1 for _ in p.open("rb")) if p.suffix in (".csv", ".md", ".json", ".log") else None
            prow.append([p.name, "%d B" % p.stat().st_size, n if n is not None else "—"])
    for p in sorted((ctx.out / "shots").glob("*.png")) if (ctx.out / "shots").is_dir() else []:
        prow.append(["shots/%s" % p.name, "%d B" % p.stat().st_size, "—"])
    A(_table(["文件", "大小", "行数"], prow))
    A("\n## 8 残留检查\n")
    if getattr(ctx, "regrade_src", None):
        A("> 本轮未起世界/仿真（只复算）；下列为复算结束时现场检查。\n")
    res = ctx.residual or {}
    A("```\npgrep:\n%s\n\nports:\n%s\n\nxvfb: %s\n\ngit_status_after: %d 行\n```\n" % (
        res.get("pgrep") or "(空)", res.get("ports") or "(空)", res.get("xvfb") or "—",
        len(res.get("git_status_after") or [])))
    A("## 9 复现命令\n")
    A("```bash\ncd %s\n"
      "# ① 世界副本（幂等；源世界零改动）\n"
      "python3 -B Tool/sim_task39_step3_test.py --preflight   # 前置自检 + 任务级遗留门 + 自动建副本\n"
      "python3 -B Tool/sim_task39_step3_test.py --probe-env   # env_probe.csv（MSAA 4x vs 0）\n"
      "# ② 逐 crate 编译门禁\n"
      "for c in pleiades-ugv pleiades-uav pleiades-terminal pleiades-sim; do cargo check -p \"$c\" --all-targets; done\n"
      "cargo check -p pleiades-base            # --all-targets 为已知红（t01_storage_integration）\n"
      "# ③ 单测（全量；2 条红在 HEAD 即红）\n"
      "cargo test -p pleiades-sim --bins 2>&1 | tee tmp/task39/cargo_test.log\n"
      "# ④ 正式跑（6 用例，顺序 f1,f2,f3,m1,m2,m3）\n"
      "python3 -B Tool/sim_task39_step3_test.py --cases f1,f2,f3,m1,m2,m3\n"
      "# ⑤ 判据复算（只读已有产物，不启动世界/仿真）\n"
      "python3 -B Tool/sim_task39_step3_test.py --regrade RobotSim_TestEnv/results/<取证轮目录>\n"
      "```\n" % ROOT)
    (ctx.out / "verdict.md").write_text("\n".join(L), encoding="utf-8")


# ================================================================ 收尾 / 残留

def cleanup_all(ctx, xvfb):
    try:
        ctx.control.send_cmd({"cmd": "stop"})
    except Exception:
        pass
    try:
        if ctx.ped is not None:
            ctx.ped.close()
    except Exception:
        pass
    try:
        ctx.control.close()
    except Exception:
        pass
    for p in (ctx.sim, ctx.world):
        try:
            p.stop()
        except Exception:
            pass
    try:
        xvfb.stop()
    except Exception:
        pass
    ctx.xvfb_reused = bool(xvfb.reused)


def residual():
    out = {"pgrep": FX.pgrep_residual()}
    busy = [p for p in (DEFAULT_PORT, WORLD_PORT) if FX.port_busy(p)]
    out["ports"] = ("忙：%s" % busy) if busy else "9098/9999 均空闲"
    out["x_display"] = os.path.exists("/tmp/.X11-unix/X99")
    return out


# ================================================================ main

class Tee:
    def __init__(self, path, stream):
        self.stream = stream
        self.f = open(path, "a", encoding="utf-8", buffering=1)

    def write(self, s):
        self.stream.write(s)
        try:
            self.f.write(s)
        except (OSError, ValueError):
            pass
        return len(s)

    def flush(self):
        try:
            self.stream.flush()
        except (OSError, ValueError):
            pass
        try:
            self.f.flush()
        except (OSError, ValueError):
            pass

    def isatty(self):
        return False

    def close(self):
        try:
            self.f.close()
        except (OSError, ValueError):
            pass


def parse_args(argv=None):
    p = argparse.ArgumentParser(description="task_39 Step 3 验证（常规移动 + 跟随，只验不改）")
    p.add_argument("--preflight", action="store_true", help="只跑前置自检 + 任务级遗留门，退出码 0/2")
    p.add_argument("--probe-env", action="store_true",
                   help="只跑渲染旋钮实测（源世界 MSAA 4× vs 副本 0）→ env_probe.csv")
    p.add_argument("--with-probe", action="store_true",
                   help="正式跑之前先跑渲染旋钮实测，把 env_probe.csv 放进本次结果目录")
    p.add_argument("--no-launch", action="store_true", help="复用已在跑的世界/控制口（仅排障）")
    p.add_argument("--reuse-world", action="store_true", help="跳过副本刷新（默认按 .copied_from 自动判定）")
    p.add_argument("--out-dir", default=None)
    p.add_argument("--port", type=int, default=None)
    p.add_argument("--world-dir", default=None, help="世界目录（默认 RobotSim_TestEnv/world_fast）")
    p.add_argument("--cases", default="f1,f2,f3,m1,m2,m3")
    p.add_argument("--regrade", default=None, metavar="DIR",
                   help="对已有 results 目录做判据复算（不启动世界/仿真）")
    return p.parse_args(argv)


def close_ped_quietly(ctx):
    """用例异常退出时兜底关掉行人客户端（否则残影会污染下一例的 detector 判据）。"""
    if ctx.ped is None:
        return
    peer = ctx.ped_peer
    try:
        ctx.ped.close()
    except Exception:
        pass
    ctx.ped = None
    if peer:
        try:
            ctx.wait_world(REMOVED_RE % re.escape(peer), 10)
        except Exception:
            pass
    ctx.read_world()
    Log("兜底关掉行人 %s（世界动态行人=%s）" % (peer, ped_alive_count(ctx) or "无"))


def run_case(ctx, name):
    """跑一个用例；**任何异常只记账不中断整轮**（`prev_issues.md` 第 1 条）。"""
    try:
        if name == "m1":
            rec = goto_case(ctx, "M1", M1_TARGET, CASE_TIMEOUT_WALL["m1"])
            ctx.shots.append(FX.shoot(ctx, "m1_arrive",
                                      uav=(rec["end"]["x"], rec["end"]["y"], rec["end"]["z"])))
        elif name == "m2":
            rec = goto_case(ctx, "M2", M2_TARGET, CASE_TIMEOUT_WALL["m2"])
            ctx.shots.append(FX.shoot(ctx, "m2_corner",
                                      uav=(rec["end"]["x"], rec["end"]["y"], rec["end"]["z"])))
        elif name == "m3":
            rec = goto_case(ctx, "M3", M3_TARGET, CASE_TIMEOUT_WALL["m3"])
            ctx.shots.append(FX.shoot(ctx, "m3_arrive",
                                      uav=(rec["end"]["x"], rec["end"]["y"], rec["end"]["z"])))
        elif name == "f1":
            rec = case_f1(ctx)
        elif name == "f2":
            rec = case_f2(ctx)
        elif name == "f3":
            rec = case_f3(ctx)
        else:
            Log("[warn] 未知用例 %r → 跳过" % name)
            return None
    except FX.HarnessError as exc:
        ctx.cases_failed[name] = {"error": "%s: %s" % (type(exc).__name__, exc),
                                  "phase": "run_case"}
        Log("[FAIL(env)] 用例 %s 中止：%s（继续下一例）" % (name.upper(), exc))
        close_ped_quietly(ctx)
        return None
    except Exception as exc:
        traceback.print_exc()
        ctx.cases_failed[name] = {"error": "%s: %s" % (type(exc).__name__, exc),
                                  "phase": "run_case"}
        Log("[FAIL(env)] 用例 %s 脚本异常：%r（继续下一例）" % (name.upper(), exc))
        close_ped_quietly(ctx)
        return None
    ctx.cases_done[name] = rec
    ctx.seg = "END"
    return rec


def main(argv=None):
    args = parse_args(argv)
    if args.regrade:
        return regrade_run(args)
    want = [c.strip().lower() for c in args.cases.split(",") if c.strip()]
    try:
        cfg = FX.load_config()
    except Exception as exc:
        print("[FAIL] 读不了 %s：%r" % (CFG, exc))
        return 2
    port = args.port or int(cfg.get("control", {}).get("port") or DEFAULT_PORT)
    world_dir = pathlib.Path(args.world_dir).resolve() if args.world_dir else WORLD_FAST

    world_copy = ensure_world_fast(reuse=args.reuse_world) if world_dir == WORLD_FAST \
        else {"copied": False, "reason": "外部 --world-dir（{0}）".format(world_dir),
              "seconds": 0.0, "stamp": "—"}

    if args.probe_env:
        out = pathlib.Path(args.out_dir) if args.out_dir else \
            (RESULTS / ("task39_probe_" + now_utc_stamp()))
        out.mkdir(parents=True, exist_ok=True)
        tee = Tee(str(out / "script.log"), sys.stdout)
        sys.stdout = tee
        print("[out] %s" % out)
        ctx = FX.Ctx(out, cfg, port, args)
        ctx.world_dir = world_dir
        ctx.world_copy = world_copy
        ctx.clock = WorldClock()
        ctx.cases_done = {}
        ctx.cases_failed = {}
        ctx.git_before = FX.git_status_lines()
        xvfb = FX.Xvfb()
        rc = 0
        try:
            xvfb.ensure()
            probe_env(ctx)
        except Exception:
            traceback.print_exc()
            rc = 2
        finally:
            try:
                ctx.world.stop()
            except Exception:
                pass
            try:
                xvfb.stop()
            except Exception:
                pass
            sys.stdout.flush()
            sys.stdout = tee.stream
            tee.close()
        print("env_probe.csv → %s" % (out / "env_probe.csv"))
        return rc

    pre = preflight(cfg, port)
    width = max(len(c["name"]) for c in pre)
    print("=" * 92)
    print("前置自检（%d 条）" % len(pre))
    for c in pre:
        print("  %-*s %-5s %s" % (width, c["name"], "OK" if c["ok"] else "FAIL", c["detail"]))
    print("=" * 92)
    if args.preflight:
        return 0 if all(c["ok"] for c in pre) else 2
    if any(not c["ok"] for c in pre):
        print("[FAIL(env)] 前置自检未过（缺口见上表）")
        return 2

    out = pathlib.Path(args.out_dir) if args.out_dir else \
        (RESULTS / ("task39_" + now_utc_stamp()))
    out.mkdir(parents=True, exist_ok=True)
    tee = Tee(str(out / "script.log"), sys.stdout)
    sys.stdout = tee
    print("[out] %s" % out)
    shutil.copy2(CFG, out / "config.toml")

    ctx = FX.Ctx(out, cfg, port, args)
    ctx.world_dir = world_dir
    ctx.world_copy = world_copy
    ctx.clock = WorldClock()
    ctx.cases_done = {}
    ctx.cases_failed = {}
    ctx.ped_peer = None
    ctx.xvfb_reused = False
    ctx.git_before = FX.git_status_lines()
    ctx.control = FX.Control(port, timeout=10.0)
    global MONO_OFF
    MONO_OFF = float(ctx.mono_off)      # tracing 行首 UTC → monotonic（与 detector 帧同基）

    xvfb = FX.Xvfb()
    s0 = None
    judges = {}
    overall, reason = "FAIL(env)", ""
    t_boot = time.monotonic()
    try:
        xvfb.ensure()
        ctx.xvfb_reused = bool(xvfb.reused)
        if args.with_probe:
            Log("先跑渲染旋钮实测（源世界 MSAA 4× vs 副本 0）…")
            probe_env(ctx)
        if not args.no_launch:
            Log("起世界（副本）…")
            launch_world_fast(ctx)
            Log("起 sim …")
            FX.launch_sim(ctx)
            FX.status_assembled(ctx, 120)
        ctx.sim_tail.reset()
        Log("sim tracing 日志：%s" % ctx.sim_tail.path)
        s0 = assert_s0(ctx)
        measure_env(ctx, 10.0)
        s0_env_anchors(ctx, s0)

        for idx, case in enumerate(want):
            if time.monotonic() - t_boot > TOTAL_BUDGET_WALL:
                for rest in want[idx:]:
                    ctx.cases_failed[rest] = {
                        "error": "总预算 %.0f s 超时（R5），该用例未运行" % TOTAL_BUDGET_WALL,
                        "phase": "budget"}
                Log("[FAIL(env)] 总预算 %.0f s 超时，剩余用例写 missing_reason"
                    % TOTAL_BUDGET_WALL)
                break
            run_case(ctx, case)

        if ctx.env.get("k_world_over_wall") is None:
            ks = [v.get("k") for v in ctx.cases_done.values() if v.get("k")]
            if ks:
                ctx.env["k_world_over_wall"] = round(median(ks), 4)
                ctx.env["k_source"] = "逐用例实测 k 的中位数（measure_env 窗口无帧）"
        judges = fill_missing_judges(ctx, judge_all(ctx))
        overall, reason = overall_of(judges)
    except FX.HarnessError as exc:
        Log("[FAIL(env)] %s" % exc)
        overall, reason = "FAIL(env)", str(exc)
        judges = fill_missing_judges(ctx, judge_all(ctx))
    except Exception:
        Log("[异常] 见 traceback")
        traceback.print_exc()
        overall, reason = "FAIL(env)", "脚本异常（见 script.log）"
        judges = fill_missing_judges(ctx, judge_all(ctx))
    finally:
        try:
            cleanup_all(ctx, xvfb)
        except Exception:
            traceback.print_exc()
        try:
            ctx.residual = residual()
            ctx.residual["git_status_after"] = FX.git_status_lines()
        except Exception:
            ctx.residual = {}
        try:
            if not hasattr(ctx, "cases_done"):
                ctx.cases_done = {}
            if not getattr(ctx, "cases_failed", None):
                ctx.cases_failed = {}
            write_csvs(ctx)
            write_json_artifacts(ctx, judges, pre, s0, overall, reason)
            write_verdict(ctx, judges, pre, s0, overall, reason,
                          env_fail=(overall == "FAIL(env)"))
        except Exception:
            traceback.print_exc()
        try:
            if ctx.sim_tail.path is not None and ctx.sim_tail.path.exists():
                shutil.copy2(ctx.sim_tail.path, out / "sim_trace.log")
            if (out / "verdict.md").exists():
                with (out / "verdict.md").open("a", encoding="utf-8") as f:
                    f.write("\n## 10 收尾复检（cleanup 之后）\n\n```\n%s\n%s\n%s\n```\n" % (
                        ctx.residual.get("pgrep") or "(空)",
                        ctx.residual.get("ports") or "(空)",
                        "X99 socket 存在" if ctx.residual.get("x_display") else "X99 socket 已消失"))
        except Exception:
            pass
        sys.stdout.flush()
        sys.stdout = tee.stream
        tee.close()

    print("=" * 92)
    for ident in JUDGE_ORDER:
        j = judges.get(ident)
        if j:
            print("  %-5s %-10s %s" % (ident, j["verdict"], j["measured"][:120]))
    print("  结论：%s — %s" % (overall, reason))
    print("  产物：%s" % out)
    print("=" * 92)
    if overall == "PASS":
        return 0
    return 2 if overall == "FAIL(env)" else 1


if __name__ == "__main__":
    sys.exit(main())
