#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Presented by KeJi
# Created Date ： 2026-10-30
# Modified Date ： 2026-10-30
"""task_41 Step 2 — sim 机栈 containment 端到端验证（tree v1，**只验不改**）。

被测链路：Pictor `task_set{type:3}` ⇒ 变长 14 B 帧 ⇒ sim 机栈 `Containment { x, y, half_side }`
⇒ `containment::build`（`ReactiveFallback[ ReactiveSequence[InsideSquare, follow子树], 巡逻 ]`）
⇒ 正方形巡逻 / 人在界内则跟随 / 出界回巡逻。
**一行不改被测代码、不改判据阈值、不改用例坐标、不重跑求绿**（plan.md §0 重跑纪律）：
判据不过就如实写 FAIL；证据指向环境就写 `FAIL(env)`。

执行轮次：**只验不改，仅 1 轮**（FAIL 即终止，不重试）。

环境：`RobotSim_TestEnv/world_fast/`（源树整树副本 + `override.cfg` 的 `msaa_3d=0`）+ `xvfb-run -a`
+ 机栈实例 `RobotSim_TestEnv/simq_uav_containment/`（控制口 9122，libp2p 9125，detector + 机头相机，
`vel_fwd=0.8`、`yaw_rate_deg=30`、`model=yolov8s.safetensors`）+ Pictor（`--headless --path Pictor --
--ws-port=9100`，CWD=`Pictor`（kernel 读 `Pictor/.config/config.toml`，见 step §8；脚本只读校验，不写
Pictor/），libp2p 9123）+ 行人
`Tool/sim_task41_pedestrian.py`（`python3 -u`）。

用例（3 段，总 ~420-600 s）：
  S2-A 无行人 150 s 巡逻｜S2-B 行人 0.5 m/s (64,80)→(64,71) 停 20 s（放人时机见 D3）→(64,64) 3 s→
  S2-C 行人走向 (64,50) 出界，观察 20 s 内回巡逻。

判据（P1~P7，Task §4.4 原值，一个不改）：
  P1 任务贯通｜P2 巡逻到 ≥2 角（150 s 窗）｜P3 5 s 内切跟随 + 10 s 位移 ≥3 m + 距离不拉大
  （t0 只认 class=person 且 判定=采纳）
  P4 出界 20 s 内回巡逻 + 朝角点位移 ≥2 m｜P5 安全（越界/突跳/卡死）
  P6 配置生效（v=0.8±0.05 / 位姿龄 ≤120 ms / detector ≥1 Hz（CPU+yolov8s））｜P7 无异常 + 收尾 + 边界

用法：
    python3 Tool/sim_task41_containment_test.py --dry-run      # 只跑前置检查（不起进程）
    python3 Tool/sim_task41_containment_test.py --selfcheck    # 起停一轮，不发任务（产物 tmp/task41/）
    python3 Tool/sim_task41_containment_test.py                # **唯一一次**验证运行
    其他开关：--out DIR   --only P1,P3（只裁剪 stdout 打印）   --max-s 900   --keep
退出码：0 = P1~P7 全 PASS；1 = 有 FAIL；2 = 环境/前置失败（无法判定，记 FAIL(env)）

产物：`RobotSim_TestEnv/results/task41_s2_<UTC>/`
    verdict.md / sim.log / world.log / ped.log / pictor.log / trace.csv / cases.json
    + ped_trace.csv / status.jsonl / ctl_A.json / driver.log / configs/

--------------------------------------------------------------- 口径偏差登记（D1~D11）
D1 Task §4.5 要求「复用 `sim_task40_ugv_test.py`、`sim_follow_test.py` 的既有设施」。本脚本只复用
   前者（`ensure_world_fast` / `world_argv` / `world_diff_extra` / `world_copy_sync` / `residual_all` /
   `run_capture` / `utc_stamp`）+ `sim_queue_test`（`WS` / `Proc` / `LogTail` / `Control` /
   `port_busy` / `sha256_file` / `SAMPLE_DT`），**不 import `sim_follow_test`**：它自带 `Xvfb :99`
   管理，与 step.md「显示一律 `xvfb-run -a`」冲突。
D2 Task §4.4 的 `next_cell 属于/离开角点集合`：`next_cell` 实为 D* 的**相邻格**（`goto.rs:105`、
   `navigator.rs:265-300`）或跟随叶的 0.5 m 几何格（`follow.rs:154-165`），**不是任务目标格**
   ⇒ 字面判据对任何正确实现都近乎恒真/恒假。改用 `[Nav] 新目标 (x,y)`（`navigator.rs:229`，
   = 叶子交给导航器的**目标**）∈ 角点集合 作为等价实证（P2b / P4a）；P3 用「无新目标 + 朝人 +
   不越过 x=62」；**字面计数照录**。
D3 Task §4.3 的放人条件「`|y−72|≤0.5` ∧ 朝 `+x` ∧ `x∈[56,60]`」在 v1 巡逻顺序下**不可满足**
   （`containment.rs:52` 的角点序：y≈72 那条边是 (72,72)→(56,72)，朝 **−x**）。保留该条件并行
   监测（`trigger_used=L`，预期 0 命中），实际用**上行段相位锁**：x≈72 朝 +y 且
   `y ∈ [y_target, y_target+3]` 时放人（`y_target = 74 − 20·v_eff`，`armed` 规则见 `wait_release`）。
   ⇒ 行人 20 s 后停在 (64,71)、UAV 正好进西行段 x≈70（首检 5~7 m、相对机头 10~30°）。
   与文档「等 UAV 到位再放人、保证人在机头前方 4~8 m 的 FOV 内」同构。
D4 Task §4.3 行人停点 (64,72) **恰在正方形边界线上**（`|y−64| = 8.00`），检测世界坐标误差
   0.3~0.6 m 会让 `InsideSquare` 在边界抖动。停点取 **(64,71)**（边界内 1 m，Task 原文即写
   「南边界**内**」）；`cases.json` 同时记 `doc_stop` 与 `stop`，verdict 记实测边界余量。
D5 Task P5 的「连续 5 s 位置变化 < 0.05 m 且任务未结束」会把**合法的 Follow Hold**（`d≤2.5 m`
   悬停、`v=0`，`follow.rs:66`）误判成卡死。卡死 = 该 5 s 窗内**有动作指令**（`|v| ≥ 0.5`）
   却几乎不动。
D6 Task 未规定物理时间轴来源。`t_world = a + k·t_log`，主标定用 `[detector] frame_ts`（世界时间
   ms），交叉校验用 `ped_trace.csv`，退化路径用相对轴（verdict 记 D6）。
D7 `[Sim] UAV` 行**不含 z**。`trace.csv` 的 `z` 由 200 ms `status` 采样近邻填充（±0.5 s），
   无则留空；`status.jsonl` 全量留证。
D8 kernel 的实际 CWD 是 `Pictor/`（证据：kernel 日志落 `Pictor/Log`、keypair 只在 `Pictor/.config`、
   04:22 轮绑定端口 38005 = 当时 `Pictor/.config` 的 `listen_port` 值）。step §8 已由人类固定
   `Pictor/.config/config.toml`：listen_port=9123 / peer_name=pictor_s2 / node_type=ground_station。
   本脚本**只读校验**该文件 + 从 kernel 日志实证「监听地址 …/tcp/9123/」，**不写 Pictor/**（越界）。
   旧的 `RobotSim_TestEnv/pictor_ws/.config/config.toml`（task_39 遗留 9101/pictor_ws）不再使用，保持原样。
D9 Task §4.2 行人写「`pedestrian_client.py`（`python3 -u`）」。S2-B 需要「到点驻留 20 s」，而
   `patrol.py` 无 dwell 能力 ⇒ 新增 `Tool/sim_task41_pedestrian.py`（`python3 -u` 启动），
   **内部 import 同一个 `PedestrianClient`**，只加「路点 + 驻留」驱动层，不复制协议代码。
D10 Task §4.4 P3 的「5 s 内 `next_cell` 离开角点集合」：follow 叶**不调用 `nav.set_goal`**
   （`follow.rs` 文件头：直接给 `next_cell = me + 0.5 m·unit(target−me)`），所以 follow 期间
   **不会有** `[Nav] 新目标`。实证改为 ① `v>0` 且机头朝人 ±20°，② `next_cell` 格心距机体
   ≤ 1.0 m 且方位差 ≤25°（= 朝人一步）。二者其一致即判「已切跟随」；上一轮自造的
   `min_x` / 「驻留期无 c4 到达」降为**辅助量**，不进判定。
D11 行人命令 0.5 m/s，实测中位速度见 verdict「行人口径」行与
    `cases.json.person.measured_speed_median`（世界侧起步转弯 + 0.30 m 到点容差；不改用例速度）。
"""

import argparse
import csv
import json
import math
import os
import pathlib
import re
import shutil
import socket
import sys
import time
import traceback
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "Tool"))
import sim_queue_test as Q          # noqa: E402  （复用设施；`if __name__` 守护，import 安全）
import sim_task40_ugv_test as S2    # noqa: E402  （**只读复用**它的公共函数，不改它）

# ================================================================ §2 常量
WORLD_FAST = ROOT / "RobotSim_TestEnv" / "world_fast"
INST = ROOT / "RobotSim_TestEnv" / "simq_uav_containment"
RESULTS = ROOT / "RobotSim_TestEnv" / "results"
PICTOR = ROOT / "Pictor"
PICTOR_CWD = PICTOR                          # D8：kernel 的 CWD 就是 Pictor/（--path 同一目录）
PICTOR_CFG = PICTOR / ".config" / "config.toml"   # 只读校验对象（gitignored）
PICTOR_LOG = PICTOR / "Log"                  # kernel 日志目录（监听地址/连接建立证据）
CTL = PICTOR / "tool" / "pictor_ws_ctl.py"
PED_TOOL = ROOT / "Tool" / "sim_task41_pedestrian.py"
RELEASE_BIN = ROOT / "target" / "release" / "pleiades-sim"
# §二.14：detector 用 yolov8s.safetensors（HF `lmz/candle-yolo-v8`，355 keys 与 yolov8n 一致；
# 已拷入本仓 `Pleiades_Workspace/`，与实例内副本内容 sha256 一致）
YOLO_SRC = ROOT / "Pleiades_Workspace" / "yolov8s.safetensors"
KERNEL_SO = PICTOR / "bin" / "libpleiades_terminal.so"
GIT_BASELINE = ROOT / "tmp" / "task41" / "git_baseline.txt"

WORLD_PORT = 9999
WS_PORT = 9100
PIC_NET_PORT = 9123
CTL_PORT = 9122
UAV_NET_PORT = 9125
PORTS = (WORLD_PORT, WS_PORT, CTL_PORT, PIC_NET_PORT, UAV_NET_PORT)
PEER_UAV = "simq_uav_containment"
PEER_PED = "s2_ped"

PICTOR_TIMEOUT_S = 2100
WORLD_LISTEN_S = 90.0          # R7 闸门
SPAWN_S = 60.0                 # R7 闸门
DETECTOR_S = 120.0             # plan §8 step5 闸门
CAMERA_S = 60.0                # plan §8 step5 闸门
PICTOR_LISTEN_S = 40.0         # plan §8 step5 闸门
PICTOR_KERNEL_S = 30.0         # 起进程后的启动闸门：等 kernel 绑定 9123 并打印监听地址
REGISTER_S = 40.0              # plan §4.1「ctl wait --timeout 40」

# ─── 封控正方形（Task §4.3；世界轴对齐）────────────────────────────────────
CX, CY, HALF = 64.0, 64.0, 8.0
CELL = 0.5                     # `CELL_RESOLUTION`（只用于网格换算与字面计数）
CORNERS = [(CX - HALF, CY - HALF), (CX + HALF, CY - HALF),
           (CX + HALF, CY + HALF), (CX - HALF, CY + HALF)]
CORNER_GRID = [(int(x / CELL), int(y / CELL)) for x, y in CORNERS]

# ─── 行人路线（D4：(64,71) 而非文档的 (64,72)；§二.13：行人 0.5 m/s）─────────
PED_SPAWN = (64.0, 80.0)
PED_LEGS = "64,71@20;64,64@3;64,50@0"
PED_STOP = (64.0, 71.0)
PED_DOC_STOP = (64.0, 72.0)
PED_SPEED = 0.5
PED_ARRIVE_TOL = 0.30
PED_DURATION_S = 300.0

# ─── 判据阈值常量（Task §4.4 原值，**一个不改**）──────────────────────────────
CORNER_TOL_M = 1.0
P2_CORNERS_MIN = 2
SWITCH_S = 5.0
DISP_MIN_M = 3.0
DIST_SLACK_M = 2.0
P4_S = 20.0
P4_DISP_MIN_M = 2.0
JUMP_M = 1.0
JUMP_DT_MAX_S = 0.6
DEAD_WIN_S = 5.0
DEAD_EPS_M = 0.05
DEAD_MIN_V = 0.5
BOUND_M = 128.0
V_TARGET = 0.8
V_TOL = 0.05
POSE_AGE_MAX_MS = 120
DET_MIN_HZ = 1.0
PERSON_CLASS = "person"        # §二.15：detector 只把 person 放进 Target；t0 也只认它
ACCEPT_VERDICT = "采纳"        # `detector/mod.rs:547` 的另一取值是 `非person（忽略）`（只计数留证）
NAV_CORNER_TOL_M = 0.2         # D2：`[Nav] 新目标` 命中角点的容差（导航层就打在 2 位小数）
P3A_YAW_TOL_DEG = 20.0
P3A_MIN_ROWS = 1

# ─── 放人相位锁 / 跟随实证（plan §2.2(e)(f) / §3.2 / §4.3）──────────────────
NEXT_CELL_STEP_M = 1.0         # ② follow 叶「朝人 0.5 m 一步」的格心半径（`follow.rs::step_cell`）
P3A2_ANGLE_DEG = 25.0          # ② `next_cell` 方位差（detector 世界坐标 0.3~0.6 m 误差 → 放宽 5°）
T_PED_WALK_S = 20.0            # 0.5 m/s 下 (64,80)→(64,71)：8.65 m + ~2 s 起步转向
REL_WEST_MARGIN_M = 2.0        # 人停稳时 UAV 已进西行段 2 m ⇒ 首检 5~7 m（P3b 需 d_首 ≥5.5 m）
REL_Y_FALLBACK = 63.0          # `v_eff` 测不出来时的 y 目标
V_EFF_CLAMP = (0.25, 0.90)     # 防路径速度测量抖动把 y 目标推到离谱
REL_BAND_M = 3.0               # 相位窗带宽：`y ∈ [y_target, y_target+3]`
ASCENT_X_TOL_M = 0.8           # 「上行段」= x 贴着东边
ASCENT_YAW_TOL_DEG = 25.0      # 「上行段」= 机头朝 +y（±25°）

# ─── 用例时间（Task §4.3 + plan §4）────────────────────────────────────────
A_OBSERVE_S = 150.0            # S2-A 观察窗（= P2 窗口）
RELEASE_WAIT_S = 120.0         # S2-B 放人等待上限（plan §2.2(f) / R10）
B_OBSERVE_S = 90.0             # S2-B 观察上限（等 leg1 起：走 20 s + 停 20 s）
C_OBSERVE_S = 45.0             # S2-C 观察上限
SAMPLE_DT = Q.SAMPLE_DT        # 200 ms
LOG_POLL_DT = 0.40             # 日志轮询周期
MISSION_TIMEOUT_S = 120        # ctl containment 的 --timeout

GOLDEN_HEX = "0103428000004280000041000000"      # count=1 | type=3 | 64 | 64 | 8
GOLDEN_LEN = 14
GOLDEN_TYPE = 3

# ================================================================ §3 配置模板
UAV_CONFIG_TEMPLATE = """\
# Presented by KeJi
# Created Date ： 2026-10-30
# Modified Date ： 2026-10-30
#
# task_41 Step 2：sim 机栈封控验证（相机 + detector + containment v1）。
# 与 `RobotSim_TestEnv/simq_s4_uav`（即 `sim1`）同源，只换身份 / 端口 / 速度（§二.3 vel_fwd=0.8）
# 与 detector 权重（§二.14 yolov8s）。
[Log]
level = "info"
log_file_path = "Log/"

[Network]
LAN = true
WAN = false
Transport_Protocol = "TCP"
listen_port = {net_port}
dht_namespace = "pleiades-nodes"
bootstrap_peers = []
subscribe_topics = []

[Storage]
workspace_dir = "Pleiades_Workspace"
quota_gb = 0
kvcache_dir = ".kvcache"

[Identity]
peer_name = "{peer_name}"
node_type = "uav"

[sim_world]
enabled = true
url = "ws://127.0.0.1:{world_port}"
chassis = "quadrotor_uav"
reconnect_ms_min = 200
reconnect_ms_max = 3000
handshake_timeout_ms = 8000

# 机头相机（真渲染；pitch 负 = 低头 15°，保证 2~3 m 时脚在画面内）
[[sim_world.sensor]]
slot = "camera_nose"
sensor = "camera"

[sim_world.sensor.mount]
x = 0.20
y = 0.0
z = 0.045
yaw = 1.5708        # 弧度：光轴 = 局部 -Z，前视要 +90°
pitch = -15.0       # 度：负 = 低头

[sim_world.sensor.params]
width = 640
height = 480
fov_deg = 70.0
quality = 0.8
rate_hz = 10.0

[chassis]
enabled = true
forward_speed = 100
turn_speed = 30

[detector]
enabled = true
model = "yolov8s.safetensors"
device = "cpu"
conf_thres = 0.35
nms_thres = 0.45

[detector.camera]
fov_deg = 70.0
fov_axis = "vertical"
mount_x = 0.20
mount_y = 0.0
mount_z = 0.045
mount_yaw_deg = 90.0
mount_pitch_deg = -15.0
ground_z = 0.0
max_range_m = 12.0

[control]
enabled = true
bind = "127.0.0.1"
port = {ctl_port}

# §二.3：sim 飞机速度统一 0.8 m/s（只改 sim 实例配置；1.5 实测巡逻仅 0.33 m/s，已回退）
[flight_ctrl]
vel_fwd = 0.8
yaw_rate_deg = 30.0
"""

# ================================================================ §3.7 日志正则
LOG_TS = r"(?P<ts>\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6})Z"
TS_RE = re.compile(r"^" + LOG_TS + r"\s")
UAV_RE = re.compile(
    r"^" + LOG_TS + r"\s+\w+\s+"
    r"\[Sim\] UAV 位置=\((?P<x>-?\d+\.\d+),(?P<y>-?\d+\.\d+)\) yaw=(?P<yaw>-?\d+\.\d+)° "
    r"next=(?:None|Some\(\((?P<gx>-?\d+), (?P<gy>-?\d+)\)\)) → "
    r"v=(?P<v>-?\d+(?:\.\d+)?) omega=(?P<omega>-?\d+(?:\.\d+)?) 位姿龄=(?P<age>\d+)ms"
)
DET_RE = re.compile(
    r"^" + LOG_TS + r"\s+\w+\s+"
    r"\[detector\] frame_ts=(?P<fts>\d+)ms\(世界时间\) seq=(?P<seq>\d+) "
    r"\d+x\d+ 推理=(?P<infer>\d+)ms dets=(?P<ndet>\d+) "
    r"pose=\((?P<px>-?\d+\.\d+),(?P<py>-?\d+\.\d+),(?P<pz>-?\d+\.\d+),yaw=(?P<pyaw>-?\d+\.\d+)°\)"
)
# §二.15：一行可能有多个框。只有 `class=person` 且 `判定=采纳` 才算目标；bench/dog 等写
# `判定=非person（忽略）`（`detector/mod.rs:547` 只有这两个取值，拒答行不带 `世界=`，天然不匹配），
# 这里把 verdict 也抓出来：非 person 的只进 `others` 计数留证，**绝不进 `targets`**。
TGT_RE = re.compile(
    r"\| (?P<cls>[A-Za-z_][A-Za-z0-9_]*) conf=(?P<conf>\d+\.\d+) "
    r"bbox=\((?P<bx1>-?\d+),(?P<by1>-?\d+)\)-\((?P<bx2>-?\d+),(?P<by2>-?\d+)\) "
    r"→ 世界=\((?P<tx>-?\d+\.\d+),(?P<ty>-?\d+\.\d+)\) 距离=(?P<rng>\d+\.\d+)m "
    r"方位=(?P<br>[+-]\d+\.\d+)° 相对机头=(?P<rel>[+-]\d+\.\d+)° 判定=(?P<verdict>采纳|非person（忽略）)"
)
# D8 / step §8：kernel 真正绑的端口（`Pictor/Log/pleiades.log.*` 的启动行）
KERNEL_LISTEN_RE = re.compile(r"监听地址: .*/tcp/(?P<port>\d+)/p2p/")
GOAL_RE = re.compile(r"\[Nav\] 新目标 \((?P<x>-?\d+\.\d+), (?P<y>-?\d+\.\d+)\)")
ARR_RE = re.compile(r"\[Nav\].*到达.*?\((?P<x>-?\d+\.\d+), (?P<y>-?\d+\.\d+)\)")
START_RE = re.compile(r"\[Robot\] 开始任务")
BUILD_RE = re.compile(r"\[Sim/uav\] 任务 .* 建树完成（实例数 (?P<n>\d+)）")
FAIL_RE = re.compile(r"\[Sim\] 任务 .* 失败，原因码")
PANIC_RE = re.compile(r"panicked at|thread '.*' panicked|\bpanic!\(")
ERROR_RE = re.compile(r"\sERROR\s")
KNOWN_NOISE = ("resources still in use at exit", "ObjectDB instances were leaked", "<defunct>")

FROZEN = ("Task/", ".github/", "Cargo.toml", "Cargo.lock",
          "pleiades-base/src/", "pleiades-sim/src/", "pleiades-ugv/src/", "pleiades-uav/src/")

# ================================================================ §4 小工具


def Log(msg=""):
    print(msg, flush=True)


def utc_stamp():
    return time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())


def log_epoch(ts):
    """tracing 行首的 UTC 时间戳 → epoch 秒（**显式 UTC**，不用 `Q.parse_log_ts` 的本地时区假设）。"""
    try:
        return datetime.strptime(ts, "%Y-%m-%dT%H:%M:%S.%f").replace(tzinfo=timezone.utc).timestamp()
    except (ValueError, TypeError):
        return None


def line_epoch(line):
    m = TS_RE.match(line)
    return log_epoch(m.group("ts")) if m else None


def wrap180(deg):
    return (deg + 180.0) % 360.0 - 180.0


def dist(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def median(xs):
    ys = sorted(xs)
    if not ys:
        return None
    n = len(ys)
    return ys[n // 2] if n % 2 else (ys[n // 2 - 1] + ys[n // 2]) / 2.0


def percentile(xs, p):
    ys = sorted(xs)
    if not ys:
        return None
    k = (len(ys) - 1) * float(p)
    lo, hi = int(math.floor(k)), int(math.ceil(k))
    return ys[lo] if lo == hi else ys[lo] + (ys[hi] - ys[lo]) * (k - lo)


def read_text(path):
    try:
        return pathlib.Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def write_json(path, obj):
    pathlib.Path(path).write_text(json.dumps(obj, ensure_ascii=False, indent=2), encoding="utf-8")


def fmt(v, n=2):
    return "-" if v is None else ("%.*f" % (n, v))


def hit_corner(xy, tol=NAV_CORNER_TOL_M):
    for c in CORNERS:
        if dist(xy, c) <= tol:
            return c
    return None


def outside_square(xy):
    return (xy[0] < CX - HALF or xy[0] > CX + HALF
            or xy[1] < CY - HALF or xy[1] > CY + HALF)


# ================================================================ §5 实例 / 二进制 / 权重


def instance_dirs():
    for sub in (".config", "Log", "Pleiades_Workspace"):
        (INST / sub).mkdir(parents=True, exist_ok=True)


def write_uav_config(out=None):
    instance_dirs()
    text = UAV_CONFIG_TEMPLATE.format(net_port=UAV_NET_PORT, ctl_port=CTL_PORT,
                                      peer_name=PEER_UAV, world_port=WORLD_PORT)
    cfg = INST / ".config" / "config.toml"
    cfg.write_text(text, encoding="utf-8")
    if out is not None:
        (out / "configs").mkdir(parents=True, exist_ok=True)
        (out / "configs" / "uav_config.toml").write_text(text, encoding="utf-8")
    return cfg


PICTOR_CFG_INFO = {"ok": False, "sha": "", "detail": "未校验"}   # verify_pictor_config() 回填


def verify_pictor_config(out=None):
    """只读校验 kernel 实际读取的 `Pictor/.config/config.toml`（D8 / step §8）。

    **禁止写 Pictor/**（独立仓；本 step 允许路径只有 Tool/、RobotSim_TestEnv/、Workbook/）。
    返回 (ok, detail, sha256)。缺文件/键值不符 → ok=False，由 preflight #12 记 FAIL(env)。
    """
    text = read_text(PICTOR_CFG)
    want = {"listen_port": str(PIC_NET_PORT),
            "peer_name": '"pictor_s2"',
            "node_type": '"ground_station"'}
    flat = {}
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#") or "=" not in s:
            continue
        k, _, v = s.partition("=")
        flat[k.strip()] = v.split("#")[0].strip()
    bad = ["%s=%r≠%r" % (k, flat.get(k), v) for k, v in want.items() if flat.get(k) != v]
    sha = Q.sha256_file(PICTOR_CFG) if PICTOR_CFG.is_file() else ""
    if out is not None:
        (out / "configs").mkdir(parents=True, exist_ok=True)
        (out / "configs" / "pictor_config.readback.txt").write_text(
            "# %s\n# sha256=%s\n\n%s" % (PICTOR_CFG, sha, text), encoding="utf-8")
    ok = PICTOR_CFG.is_file() and not bad
    PICTOR_CFG_INFO.update({"ok": ok, "sha": sha,
                            "detail": "; ".join(bad) if bad else "9123/pictor_s2/ground_station"})
    return ok, ("%s sha256=%s" % (PICTOR_CFG.relative_to(ROOT), sha[:16]) if ok
                else ("缺 %s" % PICTOR_CFG) if not PICTOR_CFG.is_file() else "; ".join(bad)), sha


def config_readback(cfg):
    """逐键回读（plan §3.2 的 11 项）；返回 (ok, bad_list, flat)。"""
    want = {
        "peer_name": '"%s"' % PEER_UAV,
        "node_type": '"uav"',
        "listen_port": str(UAV_NET_PORT),
        "port": str(CTL_PORT),
        "chassis": '"quadrotor_uav"',
        "url": '"ws://127.0.0.1:%d"' % WORLD_PORT,
        "vel_fwd": "0.8",
        "yaw_rate_deg": "30.0",
        "enabled": "true",
        "model": '"yolov8s.safetensors"',
        "mount_pitch_deg": "-15.0",
    }
    flat, bad, has_params = {}, [], False
    for line in read_text(cfg).splitlines():
        s = line.strip()
        if not s or s.startswith("#") or s.startswith("[["):
            continue
        if s.startswith("["):
            if "chassis_params" in s:
                has_params = True
            continue
        if "=" not in s:
            continue
        k, _, v = s.partition("=")
        flat[k.strip()] = v.split("#")[0].strip() if k.strip() != "url" else v.strip()
    # 同名键在多段里出现（`enabled` / `port` / `model`）：逐个段查，任一命中即算满足
    for key, expected in want.items():
        got = None
        for line in read_text(cfg).splitlines():
            s = line.strip()
            if s.startswith(key) and "=" in s:
                got = s.partition("=")[2].split("#")[0].strip()
                break
        if got != expected:
            bad.append("%s=%r≠%r" % (key, got, expected))
    if has_params:
        bad.append("出现了 [sim_world.chassis_params] 段")
    return (not bad), bad, flat


def ensure_binary():
    """实例二进制 == `target/release/pleiades-sim`（sha256 闸门）。"""
    if not RELEASE_BIN.is_file():
        return False, "缺少 %s" % RELEASE_BIN, ""
    dst = INST / "pleiades-sim"
    instance_dirs()
    if (not dst.exists()) or dst.stat().st_mtime < RELEASE_BIN.stat().st_mtime:
        shutil.copy2(RELEASE_BIN, dst)
        dst.chmod(0o755)
    want, have = Q.sha256_file(RELEASE_BIN), Q.sha256_file(dst)
    return want == have, ("sha256 一致 %s" % have[:16]) if want == have else \
        "实例二进制 sha 不一致（%s vs %s）" % (have[:12], want[:12]), have


def ensure_yolo():
    if not YOLO_SRC.is_file():
        return False, "缺少模型 %s" % YOLO_SRC, ""
    dst = INST / "Pleiades_Workspace" / "yolov8s.safetensors"
    dst.parent.mkdir(parents=True, exist_ok=True)
    if (not dst.exists()) or Q.sha256_file(dst) != Q.sha256_file(YOLO_SRC):
        shutil.copy2(YOLO_SRC, dst)
    have = Q.sha256_file(dst)
    ok = have == Q.sha256_file(YOLO_SRC)
    return ok, "yolov8s.safetensors sha256=%s（%d B）" % (have[:12], dst.stat().st_size), have


def binary_sources_newest():
    newest = (0.0, "")
    for dirpath, _d, filenames in os.walk(str(ROOT / "pleiades-sim" / "src")):
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


# ================================================================ §6 进程


def start_proc(name, argv, cwd, log_path):
    proc = Q.Proc(name)
    proc.start(argv, cwd, log_path)
    return proc


def wait_text(path, needle, timeout_s):
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        if needle in read_text(path):
            return round(time.time() - t0, 2)
        time.sleep(0.2)
    return None


def wait_clean(timeout=10.0):
    """等收尾干净：活进程 0 且 5 个端口全空（最多等 `timeout` 秒）。返回 (活残留, 忙端口, 僵尸数)。"""
    t0 = time.time()
    res, busy, zombie = [], [], 0
    while time.time() - t0 < timeout:
        allres = S2.residual_all()
        zombie = len([x for x in allres if "<defunct>" in x])
        res = [x for x in allres if "<defunct>" not in x]
        busy = [p for p in PORTS if Q.port_busy(p)]
        if not res and not busy:
            break
        time.sleep(0.4)
    allres = S2.residual_all()
    zombie = len([x for x in allres if "<defunct>" in x])
    res = [x for x in allres if "<defunct>" not in x]
    busy = [p for p in PORTS if Q.port_busy(p)]
    return res, busy, zombie


# ================================================================ §7 前置


def git_frozen_now():
    rc, so, _ = S2.run_capture(["git", "status", "--porcelain", "-uall"], timeout=30)
    if rc != 0:
        return None
    return [ln.strip() for ln in so.splitlines() if any(p in ln for p in FROZEN)]


def git_frozen_baseline():
    if not GIT_BASELINE.is_file():
        return None
    return [ln.strip() for ln in read_text(GIT_BASELINE).splitlines()
            if any(p in ln for p in FROZEN) and ln.strip()]


def git_pictor_now():
    rc, so, _ = S2.run_capture(["git", "-C", str(PICTOR), "status", "--porcelain", "-uall"], timeout=30)
    return None if rc != 0 else [ln.strip() for ln in so.splitlines() if ln.strip()]


def preflight(out=None):
    checks = []

    def add(n, name, ok, detail):
        checks.append({"n": n, "name": name, "ok": bool(ok), "detail": detail})
        Log("  [%2d] %-46s %s  %s" % (n, name, "OK  " if ok else "FAIL", detail))

    busy = [p for p in PORTS if Q.port_busy(p)]
    add(1, "端口 %s 全空" % (list(PORTS),), not busy, "busy=%s" % busy)

    res = [x for x in S2.residual_all() if "<defunct>" not in x]
    add(2, "无残留 godot/pleiades-sim/Xvfb", not res, "%d 条%s" % (len(res), ("：" + res[0][:80]) if res else ""))

    miss = [t for t in ("xvfb-run", "xauth", "Xvfb", "godot") if shutil.which(t) is None]
    rc, so, _ = S2.run_capture(["godot", "--version"], timeout=60)
    add(3, "xvfb-run/xauth/Xvfb/godot 可用", (not miss) and rc == 0,
        ("缺 %s；" % miss if miss else "") + "godot %s" % so.strip()[:40])

    ent = S2.ensure_world_fast()
    extra = S2.world_diff_extra()[1]
    stamp_ok = (WORLD_FAST / "project.godot").is_file()
    add(4, "world_fast 副本新鲜（diff 只 override.cfg）", stamp_ok and not extra,
        "%s；diff %s" % (ent["reason"], "干净" if not extra else extra[:2]))

    ov = read_text(WORLD_FAST / "override.cfg")
    add(5, "override.cfg 含 msaa_3d=0", "msaa_3d=0" in ov, repr(ov.replace("\n", "⏎")))

    bin_ok = RELEASE_BIN.is_file()
    newest = binary_sources_newest()
    fresh = bin_ok and RELEASE_BIN.stat().st_mtime > newest[0]
    add(6, "release 二进制存在且比 src 新", fresh,
        ("mtime=%s sha=%s；src 最新 %s %s" % (
            time.strftime("%F_%T", time.localtime(RELEASE_BIN.stat().st_mtime)),
            Q.sha256_file(RELEASE_BIN)[:16], os.path.basename(newest[1]),
            time.strftime("%F_%T", time.localtime(newest[0]))) if bin_ok else "缺少 %s" % RELEASE_BIN))

    ok, detail, _sha = ensure_binary()
    add(7, "实例二进制 == target/release", ok, detail)

    yok, ydetail, _ysha = ensure_yolo()
    add(8, "yolo 权重就位", yok, ydetail)

    add(9, "Pictor kernel 库", KERNEL_SO.is_file(),
        ("%s %d B" % (KERNEL_SO.name, KERNEL_SO.stat().st_size)) if KERNEL_SO.is_file() else "缺少")

    add(10, "Pictor 项目 / ctl 脚本 / 行人脚本",
        (PICTOR / "project.godot").is_file() and CTL.is_file() and PED_TOOL.is_file(),
        "project.godot=%s ctl=%s ped=%s" % ((PICTOR / "project.godot").is_file(), CTL.is_file(),
                                            PED_TOOL.is_file()))

    cfg = write_uav_config(out)
    ok, bad, _flat = config_readback(cfg)
    add(11, "机栈 config 回读 11 键", ok, "; ".join(bad) if bad else
        "peer=%s net=%d ctl=%d chassis=quadrotor_uav vel_fwd=0.8 yaw=30.0 detector=on/yolov8s"
        % (PEER_UAV, UAV_NET_PORT, CTL_PORT))

    ok_pic, pic_detail, _pic_sha = verify_pictor_config(out)
    add(12, "Pictor config（Pictor/.config：9123 / pictor_s2）", ok_pic, pic_detail)

    now = git_frozen_now()
    base = git_frozen_baseline()
    same = now is not None and base is not None and sorted(now) == sorted(base)
    add(13, "git 冻结路径 == 基线", same,
        "冻结路径 %d 条" % (len(now or [])) if same else "基线 %s vs 现在 %s" % (base, now))

    pst = git_pictor_now()
    add(14, "Pictor 独立仓状态可读", pst is not None, "%d 条改动（快照入 cases.json）" % len(pst or []))
    return checks


# ================================================================ §8 采样


class LogSampler:
    """增量读 sim 的 tracing 日志（`Log/pleiades.log.*`），解析 5 类行；`wait_for` 供实时触发。"""

    def __init__(self, log_dir):
        self.log_dir = pathlib.Path(log_dir)
        self.tail = Q.LogTail(log_dir)
        self.path = None
        self.recs = []
        self.t_start = 0.0
        self.preexisting = set()

    def snapshot_preexisting(self):
        """起 sim **之前**记下已有的日志文件：`tracing` 每次运行新建一份，不能读错旧的那份。"""
        try:
            self.preexisting = {str(p) for p in self.log_dir.glob("pleiades.log.*")}
        except OSError:
            self.preexisting = set()
        return len(self.preexisting)

    def start(self):
        files = []
        try:
            files = sorted(self.log_dir.glob("pleiades.log.*"), key=lambda p: p.stat().st_mtime)
        except OSError:
            files = []
        fresh = [p for p in files if str(p) not in self.preexisting]
        self.path = (fresh or files or [None])[-1]
        self.tail.path = self.path
        self.tail.pos = 0
        self.t_start = time.time()
        return self.path

    def poll(self):
        new = self.tail.since(self.tail.mark())
        for ln in new:
            rec = parse_line(ln)
            if rec is not None:
                self.recs.append(rec)
        return new

    def wait_for(self, pred, timeout, idx0=0, note=""):
        t0 = time.time()
        while True:
            self.poll()
            hit = pred(self.recs[idx0:])
            if hit:
                return hit
            if time.time() - t0 >= timeout:
                return None
            time.sleep(LOG_POLL_DT)

    def full_text(self):
        path = self.path
        if path is None:
            try:
                files = sorted(self.log_dir.glob("pleiades.log.*"), key=lambda p: p.stat().st_mtime)
            except OSError:
                files = []
            path = files[-1] if files else None
        if path is None or not path.exists():
            return ""
        try:
            data = path.read_bytes()
        except OSError:
            return ""
        return data.decode("utf-8", "replace")


class StatusSampler:
    """200 ms 采 `status`（D7：`[Sim] UAV` 行不含 z，靠它填 trace 的 z + 全量留证）。"""

    def __init__(self, ctl):
        self.ctl = ctl
        self.rows = []
        self.reconnects = 0
        self.errors = 0

    def status(self):
        try:
            return self.ctl.status()
        except (OSError, Q.WsClosed, RuntimeError):
            self.errors += 1
            try:
                self.ctl.close()
            except Exception:
                pass
            if not self.ctl.connect(retries=20, delay=0.1):
                raise
            self.reconnects += 1
            return self.ctl.status()

    def row(self, phase):
        st = self.status()
        r = {
            "t_wall": round(time.time(), 3),
            "phase": phase,
            "x": float(st.get("x", 0.0)), "y": float(st.get("y", 0.0)),
            "z": float(st.get("z", 0.0)), "yaw": float(st.get("yaw", 0.0)),
            "task_id": st.get("task_id"), "queue_len": st.get("queue_len"),
            "task_ticks": st.get("task_ticks"), "nav_status": st.get("nav_status"),
            "assembled": bool(st.get("assembled")), "mode": st.get("mode"),
        }
        self.rows.append(r)
        return r

    def seg(self, seconds, phase, stop=None, deadline=None):
        t0 = time.time()
        while True:
            try:
                r = self.row(phase)
            except Exception:
                return
            if stop is not None and stop(r):
                return
            if deadline is not None and time.time() >= deadline:
                return
            if time.time() - t0 >= seconds:
                return
            time.sleep(SAMPLE_DT)


# ================================================================ §8.5 行解析


def parse_line(line):
    """一行 sim 日志 → 结构化记录（只认判据要用到的几类；其它返回 None）。"""
    ln = line.rstrip("\n")
    m = UAV_RE.match(ln)
    if m:
        ts = log_epoch(m.group("ts"))
        if ts is None:
            return None
        gx = int(m.group("gx")) if m.group("gx") is not None else None
        gy = int(m.group("gy")) if m.group("gy") is not None else None
        return {"kind": "uav", "t": ts, "raw": ln,
                "x": float(m.group("x")), "y": float(m.group("y")),
                "yaw": float(m.group("yaw")), "gx": gx, "gy": gy,
                "v": float(m.group("v")), "omega": float(m.group("omega")),
                "age": int(m.group("age"))}
    m = DET_RE.match(ln)
    if m:
        ts = log_epoch(m.group("ts"))
        if ts is None:
            return None
        targets, others = [], []
        for t in TGT_RE.finditer(ln):
            cls = t.group("cls")
            one = {"cls": cls, "conf": float(t.group("conf")),
                   "x": float(t.group("tx")), "y": float(t.group("ty")),
                   "range_m": float(t.group("rng")),
                   "bearing": float(t.group("br")), "rel": float(t.group("rel"))}
            if cls == PERSON_CLASS and t.group("verdict") == ACCEPT_VERDICT:
                targets.append(one)
            else:
                others.append(one)
        return {"kind": "det", "t": ts, "raw": ln, "fts_ms": int(m.group("fts")),
                "seq": int(m.group("seq")), "infer_ms": int(m.group("infer")),
                "ndet": int(m.group("ndet")), "targets": targets,
                "others": [o["cls"] for o in others],
                "px": float(m.group("px")), "py": float(m.group("py")),
                "pz": float(m.group("pz")), "pyaw": float(m.group("pyaw"))}
    ts = line_epoch(ln)
    m = GOAL_RE.search(ln)
    if m:
        return {"kind": "goal", "t": ts, "raw": ln,
                "x": float(m.group("x")), "y": float(m.group("y"))}
    m = ARR_RE.search(ln)
    if m:
        return {"kind": "arrive", "t": ts, "raw": ln,
                "x": float(m.group("x")), "y": float(m.group("y"))}
    m = BUILD_RE.search(ln)
    if m:
        return {"kind": "build", "t": ts, "raw": ln, "n": int(m.group("n"))}
    if START_RE.search(ln):
        return {"kind": "start", "t": ts, "raw": ln}
    if FAIL_RE.search(ln):
        return {"kind": "fail", "t": ts, "raw": ln}
    return None


def parse_sim_text(text):
    recs = []
    for ln in text.splitlines():
        r = parse_line(ln)
        if r is not None:
            recs.append(r)
    return recs


# ================================================================ §9 用例


class Stage:
    def __init__(self, ctx, sampler, sctl, tail):
        self.ctx = ctx
        self.sampler = sampler
        self.sctl = sctl
        self.tail = tail
        self.ped_proc = None

    # ── 发封控帧
    def send_containment(self):
        argv = [sys.executable, str(CTL), "containment", "--target", PEER_UAV,
                "--cx", str(int(CX)), "--cy", str(int(CY)), "--half", str(int(HALF)),
                "--port", str(WS_PORT), "--json", "--timeout", str(MISSION_TIMEOUT_S)]
        t_before = time.time()
        rc, so, se = S2.run_capture(argv, timeout=MISSION_TIMEOUT_S + 60)
        data = {}
        for line in reversed(so.splitlines()):
            line = line.strip()
            if line.startswith("{"):
                try:
                    data = json.loads(line)
                    break
                except json.JSONDecodeError:
                    continue
        t_after = time.time()
        sent_at = data.get("sent_at")
        if not isinstance(sent_at, (int, float)):
            sent_at = t_before
        info = {"argv": argv, "rc": rc, "json": data, "stderr": se[-4000:],
                "t_before": t_before, "t_after": t_after, "sent_at": sent_at}
        write_json(self.ctx["out"] / "ctl_A.json", info)
        Log("  [ctl] containment → rc=%s %.1fs ack_ok=%s payload=%s(%s B) type_on_wire=%s"
            % (rc, t_after - t_before, data.get("ack_ok"), data.get("payload_hex"),
               data.get("payload_len"), data.get("type_on_wire")))
        return info

    # ── S2-A
    def stage_a(self):
        ctx = self.ctx
        Log("\n[S2-A] 无行人巡逻 %.0f s" % A_OBSERVE_S)
        idx0 = len(self.sampler.recs)
        info = self.send_containment()
        ctx["ctl_A"] = info
        ctx["anchors"]["t_send_A_wall"] = info["sent_at"]

        def has_start(recs):
            for r in recs:
                if r.get("kind") == "start" and "Containment {" in r["raw"]:
                    return r
            return None
        r_start = self.sampler.wait_for(has_start, 60.0, idx0)
        ctx["p1"]["start_line"] = r_start["raw"] if r_start else None

        def has_build(recs):
            for r in recs:
                if r.get("kind") == "build" and r.get("n") == 1:
                    return r
            return None
        r_build = self.sampler.wait_for(has_build, 20.0, idx0)
        ctx["p1"]["build_line"] = r_build["raw"] if r_build else None
        ctx["p1"]["t_build_wall"] = r_build["t"] if r_build else None

        t_end = time.time() + A_OBSERVE_S
        self.sctl.seg(max(0.0, t_end - time.time()), "A", deadline=t_end)
        ctx["anchors"]["t_A_end_wall"] = time.time()
        Log("  [S2-A] 结束（发送 %s；开始任务=%s；建树完成=%s）"
            % (time.strftime("%H:%M:%S", time.gmtime(ctx["anchors"]["t_send_A_wall"])),
               "有" if r_start else "无", "有" if r_build else "无"))

    # ── S2-B
    def wait_release(self):
        """plan §2.2(f)：L 字面（预期 0 命中）+ A 相位锁（上行段 `y_target..y_target+3`），只放一次。

        - `armed`：只有「本圈还没越过 y_target」才允许放（非上行行或 y<y_target → re-arm）；
        - `v_eff`：最近 6 s 内 `|x−72|≤1.0` 的行的路径速度；`y_target = release_target_y(v_eff)`；
        - 超 `RELEASE_WAIT_S` → `trigger_used="timeout"`：**不放人**，记证据（P3/P4 记 FAIL）。
        """
        ctx = self.ctx
        rel = ctx["release"]

        def v_eff_of(rows):
            if not rows:
                return None, 0
            t_end = rows[-1]["t"]
            east = [r for r in rows
                    if abs(r["x"] - (CX + HALF)) <= 1.0 and r["t"] >= t_end - 6.0]
            return path_speed(east, 6.0)

        # 先把 S2-A 期间积压的行全部吃掉：放人窗口内**只认新出现的**触发
        self.sampler.poll()
        idx0 = len(self.sampler.recs)
        t0 = time.time()
        deadline = min(t0 + RELEASE_WAIT_S, ctx["hard_deadline"])
        armed = False
        recent = []            # 最近 3 条 uav 行（判 Δy>0）
        uav_seen = []          # 本阶段见过的全部 uav 行（算 v_eff）
        ascent_ys = []         # 上行段 y 序列（超时证据）
        skip_passes = 0
        v_eff_last, y_target_last = None, release_target_y(None)
        Log("  [S2-B] 等待放人触发（上限 %.0f s）：L=南边朝 +x 且 x∈[56,60]（预期 0）/ "
            "A=上行段 y∈[y_target, y_target+%.0f]" % (RELEASE_WAIT_S, REL_BAND_M))
        while time.time() < deadline:
            self.sampler.poll()
            while idx0 < len(self.sampler.recs):
                r = self.sampler.recs[idx0]
                idx0 += 1
                if r.get("kind") != "uav":
                    continue
                # 1) L 字面（Task §4.3 原文；v1 巡逻顺序下不可满足，仅记录）
                if (abs(r["y"] - (CY + HALF)) <= 0.5 and abs(wrap180(r["yaw"])) <= 15.0
                        and (CX - HALF) <= r["x"] <= (CX - HALF) + 4.0):
                    rel["literal_hits"] += 1
                    rel["literal_wait_s"] = round(time.time() - t0, 2)
                    rel["trigger_used"] = "L"
                    rel["wait_s"] = round(time.time() - t0, 2)
                    rel["v_eff"] = v_eff_last
                    rel["y_target"] = round(y_target_last, 2)
                    rel["trigger_row"] = {"x": r["x"], "y": r["y"], "yaw": r["yaw"]}
                    rel["skip_passes"] = skip_passes
                    return "L", r
                uav_seen.append(r)
                recent = (recent + [r])[-3:]
                v_eff, n_eff = v_eff_of(uav_seen)
                v_eff_last = v_eff
                y_target_last = release_target_y(v_eff)
                y_target = y_target_last
                ascent = (is_ascent_row(r) and len(recent) >= 2
                          and recent[-1]["y"] > recent[0]["y"])
                # 2) 维护 armed
                if (not ascent) or r["y"] < y_target:
                    armed = True
                elif r["y"] >= y_target + REL_BAND_M and not armed:
                    skip_passes += 1          # 本圈上行段已越过 y_target，等下一圈
                if ascent:
                    ascent_ys = (ascent_ys + [round(r["y"], 2)])[-40:]
                # 3) A 相位锁（主）
                if armed and ascent and y_target <= r["y"] <= y_target + REL_BAND_M:
                    rel["trigger_used"] = "A"
                    rel["wait_s"] = round(time.time() - t0, 2)
                    rel["v_eff"] = v_eff
                    rel["y_target"] = round(y_target, 2)
                    rel["ascent_rows"] = n_eff
                    rel["trigger_row"] = {"x": r["x"], "y": r["y"], "yaw": r["yaw"]}
                    rel["skip_passes"] = skip_passes
                    return "A", r
            self.sctl.seg(0.0, "B_wait")
            time.sleep(LOG_POLL_DT)
        rel["trigger_used"] = "timeout"
        rel["wait_s"] = round(time.time() - t0, 2)
        rel["v_eff"] = v_eff_last
        rel["y_target"] = round(y_target_last, 2)
        rel["skip_passes"] = skip_passes
        rel["ascent_ys"] = ascent_ys[-20:]
        return "timeout", None

    def start_ped(self):
        ctx = self.ctx
        out = ctx["out"]
        argv = ["timeout", str(int(PED_DURATION_S + 60)), sys.executable, "-u", str(PED_TOOL),
                "--spawn", "%.1f,%.1f" % PED_SPAWN, "--legs", PED_LEGS,
                "--speed", "%.2f" % PED_SPEED, "--peer-id", PEER_PED,
                "--csv", str(out / "ped_trace.csv"), "--arrive-tol", "%.2f" % PED_ARRIVE_TOL,
                "--duration", "%.0f" % PED_DURATION_S, "--quiet"]
        Log("  起行人：%s" % " ".join(argv[2:]))
        self.ped_proc = start_proc("ped", argv, ROOT, out / "ped.log")
        ctx["procs"]["ped"] = self.ped_proc
        dt = wait_text(out / "ped.log", "[ped] ready peer=", 40.0)
        if dt is None:
            Log("  [FAIL] 行人 40 s 内没有 ready（见 ped.log）")
            return False
        Log("  [ped] ready（%.1f s）" % dt)
        return True

    def ped_rows(self):
        return read_ped_csv(self.ctx["out"] / "ped_trace.csv")

    def stage_b(self):
        ctx = self.ctx
        Log("\n[S2-B] 进入跟随")
        trigger, rec = self.wait_release()
        t_release = time.time()
        ctx["release"]["t_release_wall"] = t_release
        ctx["anchors"]["t_release_wall"] = t_release
        if trigger == "timeout":
            Log("  [S2-B] 放人等待 %.0f s 超时（L 命中 %d 次）→ 不放人，P3/P4 记 FAIL 并给证据"
                % (RELEASE_WAIT_S, ctx["release"]["literal_hits"]))
            self.sctl.seg(min(B_OBSERVE_S, max(0.0, ctx["hard_deadline"] - time.time())), "B")
            return False
        Log("  [S2-B] 触发=%s（等待 %.1f s，L 命中 %d 次）：%s"
            % (trigger, ctx["release"]["wait_s"], ctx["release"]["literal_hits"],
               (rec or {}).get("raw", "")[:120]))
        if not self.start_ped():
            self.sctl.seg(min(B_OBSERVE_S, max(0.0, ctx["hard_deadline"] - time.time())), "B")
            return False

        t_end = min(t_release + B_OBSERVE_S, ctx["hard_deadline"])
        leg1 = None
        while time.time() < t_end:
            self.sctl.seg(0.4, "B")
            rows = self.ped_rows()
            if rows:
                got = [r for r in rows if r["leg"] >= 1]
                if got:
                    leg1 = got[0]
                    break
        if leg1 is None:
            Log("  [S2-B] %.0f s 内行人未进入 leg 1（P3 记 FAIL）" % B_OBSERVE_S)
            return False
        Log("  [S2-B] 行人 leg1 起（world_time=%.2f）→ 已随动" % leg1["world_time"])
        return True

    def stage_c(self):
        """t_out = 行人首次走出 [56,72]²；观察 [t_out, t_out+45]。"""
        ctx = self.ctx
        Log("\n[S2-C] 离开回巡逻")
        t_end = min(time.time() + C_OBSERVE_S + 60.0, ctx["hard_deadline"])
        t_out_row = None
        boundary = None
        while time.time() < t_end:
            self.sctl.seg(0.4, "C")
            rows = self.ped_rows()
            if boundary is None:
                for r in rows:
                    if dist((r["x"], r["y"]), PED_STOP) <= PED_ARRIVE_TOL + 0.1:
                        boundary = r["world_time"]
                        ctx["anchors"]["t_ped_arrive_boundary"] = boundary
                        break
            if boundary is not None:
                for r in rows:
                    if r["world_time"] > boundary and outside_square((r["x"], r["y"])):
                        t_out_row = r
                        break
                if t_out_row is not None:
                    break
        if t_out_row is None:
            Log("  [S2-C] 未观测到行人出界（P4 记 FAIL）")
            return False
        ctx["anchors"]["t_out"] = t_out_row["world_time"]
        Log("  [S2-C] 出界 world_time=%.2f pos=(%.2f, %.2f)"
            % (t_out_row["world_time"], t_out_row["x"], t_out_row["y"]))
        t_end = min(time.time() + C_OBSERVE_S, ctx["hard_deadline"])
        self.sctl.seg(max(0.0, t_end - time.time()), "C")
        return True


# ================================================================ §9.5 行人 CSV


def read_ped_csv(path):
    rows = []
    try:
        with open(path, newline="", encoding="utf-8") as f:
            for r in csv.DictReader(f):
                try:
                    rows.append({"t_wall": float(r["t_wall"]),
                                 "world_time": float(r["world_time"]),
                                 "x": float(r["x"]), "y": float(r["y"]),
                                 "z": float(r.get("z") or 0.0),
                                 "yaw_deg": float(r.get("yaw_deg") or 0.0),
                                 "v": float(r.get("v") or 0.0),
                                 "leg": int(float(r.get("leg") or 0)),
                                 "phase": (r.get("phase") or "")})
                except (KeyError, TypeError, ValueError):
                    continue
    except OSError:
        return []
    rows.sort(key=lambda r: r["world_time"])
    return rows


def ped_pos_at(rows, w):
    """按 world_time 线性插值（与 `step4_driver.py:979` 同法）；**spawn 之前无值（返回 None）**。

    上一轮在 `w < rows[0].world_time` 时返回 spawn 点，导致 `trace.csv` 在行人还没 spawn 时
    就填上 (64,80)。这里改成「只有 spawn 之后才有值」；P3 的距离窗口都在 `t_spawn` 之后，
    判定结果不受影响。
    """
    if not rows:
        return None
    if w < rows[0]["world_time"]:
        return None
    if w >= rows[-1]["world_time"]:
        return (rows[-1]["x"], rows[-1]["y"])
    for i in range(1, len(rows)):
        w1 = rows[i]["world_time"]
        if w <= w1:
            w0 = rows[i - 1]["world_time"]
            if w1 == w0:
                return (rows[i]["x"], rows[i]["y"])
            k = (w - w0) / (w1 - w0)
            return (rows[i - 1]["x"] + k * (rows[i]["x"] - rows[i - 1]["x"]),
                    rows[i - 1]["y"] + k * (rows[i]["y"] - rows[i - 1]["y"]))
    return (rows[-1]["x"], rows[-1]["y"])


def nearest_z(status_rows, t_wall, tol=0.5):
    best = None
    for r in status_rows:
        dt = abs(r["t_wall"] - t_wall)
        if dt <= tol and (best is None or dt < best[0]):
            best = (dt, r["z"])
    return None if best is None else best[1]


# ================================================================ §10 分析


def fit_line(pts):
    """最小二乘 `y = a + k·x`；样本 <3 → None。"""
    n = len(pts)
    if n < 3:
        return None
    mx = sum(p[0] for p in pts) / n
    my = sum(p[1] for p in pts) / n
    var = sum((p[0] - mx) ** 2 for p in pts)
    if var <= 0:
        return None
    k = sum((p[0] - mx) * (p[1] - my) for p in pts) / var
    a = my - k * mx
    resid = max(abs(y - (a + k * x)) for x, y in pts)
    return {"k": k, "a": a, "n": n, "max_resid": resid}


# ---------------------------------------------------------------- 放人 / 跟随实证（纯函数）


def path_speed(rows, span_s=6.0):
    """最近 `span_s` 秒的**路径速度**（m/s）：Σ|Δpos| / Δt。返回 (v, n)；n<3 或 Δt<1.5 s → (None, n)。

    `rows` 的每行要有 `t`（本地单调/日志时间均可，只要单调）与 `x`/`y`。
    """
    rs = [r for r in rows if r.get("t") is not None]
    if len(rs) < 2:
        return None, len(rs)
    t_end = rs[-1]["t"]
    win = [r for r in rs if t_end - r["t"] <= span_s]
    if len(win) < 3:
        return None, len(win)
    dt = win[-1]["t"] - win[0]["t"]
    if dt < 1.5:
        return None, len(win)
    length = sum(dist((a["x"], a["y"]), (b["x"], b["y"])) for a, b in zip(win, win[1:]))
    return length / dt, len(win)


def release_target_y(v_eff):
    """放人相位锁的 y 目标：`y_target = (CY+HALF) + REL_WEST_MARGIN_M − T_PED_WALK_S·v_eff`。

    夹到 `[CY−HALF, CY+HALF]` = [56, 72]；`v_eff` 为 None → `REL_Y_FALLBACK`(63.0)；
    `v_eff` 先夹到 `V_EFF_CLAMP=(0.25, 0.90)`（防路径速度测量抖动把目标推离谱）。
    """
    if v_eff is None:
        return REL_Y_FALLBACK
    v = min(max(float(v_eff), V_EFF_CLAMP[0]), V_EFF_CLAMP[1])
    y = (CY + HALF) + REL_WEST_MARGIN_M - T_PED_WALK_S * v
    return min(max(y, CY - HALF), CY + HALF)


def is_ascent_row(r):
    """上行段：`|x−(CX+HALF)| ≤ 0.8` 且 `|wrap180(yaw−90°)| ≤ 25°`（Δy>0 由调用方按最近几行判）。"""
    return (abs(r["x"] - (CX + HALF)) <= ASCENT_X_TOL_M
            and abs(wrap180(r["yaw"] - 90.0)) <= ASCENT_YAW_TOL_DEG)


def ped_median_speed(rows):
    """行人实测路径速度中位（m/s）：相邻两行 Δs/Δt，取 0.01<v<1.5 的中位（§二.13 命令值 0.5）。"""
    vs = []
    for a, b in zip(rows, rows[1:]):
        dt = b["world_time"] - a["world_time"]
        if dt <= 0:
            continue
        v = dist((a["x"], a["y"]), (b["x"], b["y"])) / dt
        if 0.01 < v < 1.5:
            vs.append(v)
    return median(vs), len(vs)


def pick_p3_t0(dets, t_ped_spawn, inside_pred):
    """返回第一条 `t_world ≥ t_ped_spawn` 且**至少一个 person 目标**在正方形内的 det 记录（否则 None）。

    §二.15 / step §9：t0 必须来自 `class=person` 且 `判定=采纳` 的行；bench/dog 等类别**不得计入**
    （上一轮 04:49 的 t0 被 `class=bench` 的采纳框锚定在 (71.45,64.09)）。spawn 之前的 phantom 同样不认
    （03:23 轮 t0 被 spawn 前 4 条 phantom 拉到 2.2 s 的教训）。
    """
    if t_ped_spawn is None:
        return None
    for r in dets:
        if r["t_world"] < t_ped_spawn:
            continue
        for t in r.get("targets") or []:
            if t.get("cls", PERSON_CLASS) != PERSON_CLASS:
                continue
            if inside_pred(t):
                return dict(r, inside_target=t)
    return None


def follow_switch_evidence(uav_rows, goals, ped_rows, t0, switch_s=None):
    """P3a 的两条实证（D10）+ 字面计数；返回 (ok, sub)。

    ① `v>0` ∧ 机头朝人 ±`P3A_YAW_TOL_DEG`；② `next_cell` 格心距机体 ≤`NEXT_CELL_STEP_M`
    且方位差 ≤`P3A2_ANGLE_DEG`（= follow 叶的「朝人一步」，它**不调** `nav.set_goal`）。
    二者其一致即判「5 s 内已切跟随」。
    """
    span = SWITCH_S if switch_s is None else float(switch_s)
    win = [r for r in uav_rows if t0 <= r["t_world"] <= t0 + span]
    toward, step = [], []
    for r in win:
        p = ped_pos_at(ped_rows, r["t_world"])
        if p is not None:
            brg = math.degrees(math.atan2(p[1] - r["y"], p[0] - r["x"]))
            err = abs(wrap180(brg - r["yaw"]))
            if r["v"] > 0 and err <= P3A_YAW_TOL_DEG:
                toward.append({"t": r["t_world"] - t0, "err": err, "v": r["v"]})
        if r["gx"] is not None and r["gy"] is not None:
            cc = ((r["gx"] + 0.5) * CELL, (r["gy"] + 0.5) * CELL)
            d = dist((r["x"], r["y"]), cc)
            brg = math.degrees(math.atan2(cc[1] - r["y"], cc[0] - r["x"]))
            aerr = abs(wrap180(brg - r["yaw"]))
            if d <= NEXT_CELL_STEP_M and aerr <= P3A2_ANGLE_DEG:
                step.append({"t": r["t_world"] - t0, "d": d, "err": aerr,
                             "cell": [r["gx"], r["gy"]]})
    lit = sum(1 for r in win if r["gx"] is not None and (r["gx"], r["gy"]) in CORNER_GRID)
    goals_in = len([r for r in goals if t0 <= r["t_world"] <= t0 + span])
    ok = len(toward) >= P3A_MIN_ROWS or len(step) >= P3A_MIN_ROWS
    sub = {"toward_rows": len(toward), "step_rows": len(step),
           "literal_hits": lit, "goals_in_5s": goals_in,
           "first_toward": toward[:1], "first_step": step[:1]}
    return ok, sub


def setup_clock(recs, ped_rows):
    """t_world = a + k·t_log（D6）：主标定 detector frame_ts，交叉校验 ped，退化用相对轴。"""
    det_pts = [(r["t"], r["fts_ms"] / 1000.0) for r in recs if r["kind"] == "det"]
    main = fit_line(det_pts)
    ped_pts = [(r["t_wall"], r["world_time"]) for r in ped_rows]
    ped_fit = fit_line(ped_pts)
    source = "detector_frame_ts"
    if main is None or main["n"] < 10:
        if ped_fit is not None and ped_fit["n"] >= 10:
            main, source = ped_fit, "ped_trace"
        else:
            t_first = min([p[0] for p in det_pts] + [p[0] for p in ped_pts]) if (det_pts or ped_pts) else 0.0
            main = {"k": 1.0, "a": -t_first, "n": 0, "max_resid": 0.0}
            source = "relative"
    cross = None
    if ped_fit is not None and source == "detector_frame_ts":
        t_mid = (min(p[0] for p in ped_pts) + max(p[0] for p in ped_pts)) / 2.0
        cross = (ped_fit["a"] + ped_fit["k"] * t_mid) - (main["a"] + main["k"] * t_mid)
    return {"k": main["k"], "a": main["a"], "n": main["n"], "max_resid": main["max_resid"],
            "source": source, "cross_check_s": cross}


def make_to_world(clock):
    return lambda t: clock["a"] + clock["k"] * t


def evaluate(ctx, recs, ped_rows, clock):
    tw = make_to_world(clock)
    out = ctx["out"]
    uav = [dict(r, t_world=tw(r["t"])) for r in recs if r["kind"] == "uav"]
    det = [dict(r, t_world=r["fts_ms"] / 1000.0) for r in recs if r["kind"] == "det"]
    goals = [dict(r, t_world=tw(r["t"])) for r in recs if r["kind"] == "goal"]
    arrives = [dict(r, t_world=tw(r["t"])) for r in recs if r["kind"] == "arrive"]
    fails = [dict(r, t_world=tw(r["t"])) for r in recs if r["kind"] == "fail"]
    t_send = tw(ctx["anchors"]["t_send_A_wall"])
    ctx["counts"] = {"uav_rows": len(uav), "det_rows": len(det),
                     "nav_goal_rows": len(goals), "nav_arrive_rows": len(arrives),
                     "accepted_person": sum(len(r.get("targets") or []) for r in det),
                     "accepted_nonperson": sum(len(r.get("others") or []) for r in det)}
    ctx["ped_speed"] = ped_median_speed(ped_rows)

    crit = []

    # ── P1 任务贯通
    p1 = ctx["p1"]
    cj = (ctx.get("ctl_A") or {}).get("json") or {}
    p1a = cj.get("ack_ok") is True
    p1b = (cj.get("payload_len") == GOLDEN_LEN and cj.get("payload_hex") == GOLDEN_HEX
           and cj.get("type_on_wire") == GOLDEN_TYPE)
    sline = p1.get("start_line") or ""
    p1c = bool(sline) and "Containment {" in sline and ("x: 64.0" in sline or "x: 64," in sline) \
        and ("y: 64.0" in sline or "y: 64," in sline) and "half_side: 8.0" in sline
    bline = p1.get("build_line")
    bt = p1.get("t_build_wall")
    dt_build = (bt - (ctx["anchors"]["t_send_A_wall"] or bt)) if bt else None
    p1d = bool(bline) and ("实例数 1" in bline) and (dt_build is not None and dt_build <= 10.0)
    unknown_type = sum(1 for ln in (ctx["sim_text"] or "").splitlines()
                       if "[Cmd] 未知 mission type" in ln)
    no_det_hold = sum(1 for ln in (ctx["sim_text"] or "").splitlines()
                      if "Containment：未启用 detector" in ln)
    p1e = unknown_type == 0
    p1f = no_det_hold == 0
    ok1 = p1a and p1b and p1c and p1d and p1e and p1f
    crit.append({"n": 1, "name": "任务贯通", "ok": ok1,
                 "detail": "ack_ok=%s；payload=%s(%s B,type=%s)；开始任务=%s；建树完成 实例数 1=%s(+%s s)；"
                           "未知 type=%d 行；未启用 detector Hold=%d 行"
                           % (cj.get("ack_ok"), cj.get("payload_hex"), cj.get("payload_len"),
                              cj.get("type_on_wire"), "有" if p1c else "无",
                              "是" if p1d else "否", fmt(dt_build, 2),
                              unknown_type, no_det_hold),
                 "sub": {"a": p1a, "b": p1b, "c": p1c, "d": p1d, "e": p1e, "f": p1f,
                         "build_delay_s": dt_build}})

    # ── P2 巡逻（窗 [t_send, t_send+A_OBSERVE_S] = 150 s）
    w0, w1 = t_send, t_send + A_OBSERVE_S
    uw = [r for r in uav if w0 <= r["t_world"] <= w1]
    corner_min = []
    for c in CORNERS:
        d = min((dist((r["x"], r["y"]), c) for r in uw), default=None)
        corner_min.append({"corner": c, "min_d": d,
                           "t": next((r["t_world"] - t_send for r in uw
                                      if dist((r["x"], r["y"]), c) == d), None)})
    hit_corners = [x for x in corner_min if x["min_d"] is not None and x["min_d"] <= CORNER_TOL_M]
    p2a = len(hit_corners) >= P2_CORNERS_MIN
    gw = [r for r in goals if w0 <= r["t_world"] <= w1]
    goal_corners = {}
    for r in gw:
        hit = hit_corner((r["x"], r["y"]))
        if hit is not None:
            goal_corners.setdefault(hit, []).append(r["t_world"] - t_send)
    p2b = len(goal_corners) >= P2_CORNERS_MIN
    fail_w = [r for r in fails if w0 <= r["t_world"] <= w1]
    timeout_cnt = sum(1 for ln in (ctx["sim_text"] or "").splitlines() if "Timeout" in ln)
    p2c = len(fail_w) == 0 and timeout_cnt == 0
    lit_rows = sum(1 for r in uw if r["gx"] is not None and (r["gx"], r["gy"]) in CORNER_GRID)
    ok2 = p2a and p2b and p2c
    crit.append({"n": 2, "name": "巡逻到 ≥2 角", "ok": ok2,
                 "detail": "窗 [0,%.0f] s：到角点 %d 个≤%.1f m（%s）；新目标命中角点 %d 个（%s）；"
                           "失败=%d；Timeout=%d 行；next_cell 字面命中 %d 行"
                           % (A_OBSERVE_S, len(hit_corners), CORNER_TOL_M,
                              "；".join("c%d %.2f m @%.1f s" % (i + 1, x["min_d"], x["t"] or 0.0)
                                       for i, x in enumerate(corner_min) if x["min_d"] is not None),
                              len(goal_corners),
                              "、".join("%s @%.1f s" % (str(k), v[0]) for k, v in goal_corners.items()),
                              len(fail_w), timeout_cnt, lit_rows),
                 "sub": {"a": p2a, "b": p2b, "c": p2c,
                         "corner_min": [{"corner": x["corner"], "min_d": x["min_d"], "t": x["t"]}
                                        for x in corner_min],
                         "goal_corners": {str(k): v for k, v in goal_corners.items()},
                         "next_cell_literal": lit_rows,
                         "timeout_lines": timeout_cnt,
                         "goal_seq": [(r["x"], r["y"], r["t_world"] - t_send) for r in gw]}})

    # ── P3 进入跟随（t0 必须 ≥ 行人 spawn；plan §4.3 照抄）
    arm = ctx["anchors"]
    t_ped_spawn = ped_rows[0]["world_time"] if ped_rows else None
    arm["t_ped_spawn"] = t_ped_spawn
    inside_pred = lambda t: not outside_square((t["x"], t["y"]))          # noqa: E731
    hit0 = pick_p3_t0(det, t_ped_spawn, inside_pred)
    acc = ctx.get("counts") or {}
    if hit0 is None:
        crit.append({"n": 3, "name": "进入跟随", "ok": False,
                     "detail": "行人 spawn 之后从未识别到 class=person 且 判定=采纳 的界内目标（t0 不存在）→ FAIL"
                               "（spawn=%s s world；person 采纳目标 %s 个、非 person 采纳 %s 个）"
                               % (fmt(t_ped_spawn), acc.get("accepted_person", "-"),
                                  acc.get("accepted_nonperson", "-")),
                     "sub": {"t_ped_spawn": t_ped_spawn,
                             "accepted_person": acc.get("accepted_person"),
                             "accepted_nonperson": acc.get("accepted_nonperson")}})
    else:
        t0 = hit0["t_world"]
        det_in = hit0["inside_target"]
        arm["t_first_in_square_detect"] = t0
        _p = ped_pos_at(ped_rows, t0)
        d_t0_ped = dist((det_in["x"], det_in["y"]), _p) if _p is not None else None
        p3a, sub3a = follow_switch_evidence(uav, goals, ped_rows, t0)
        win10 = [r for r in uav if t0 <= r["t_world"] <= t0 + 10.0]
        disp = None
        if len(win10) >= 2:
            disp = dist((win10[0]["x"], win10[0]["y"]), (win10[-1]["x"], win10[-1]["y"]))
        p3b = disp is not None and disp >= DISP_MIN_M
        d_first, d_last = [], []
        for r in uav:
            p = ped_pos_at(ped_rows, r["t_world"])
            if p is None:
                continue
            d = dist((r["x"], r["y"]), p)
            if t0 <= r["t_world"] <= t0 + 5.0:
                d_first.append(d)
            elif t0 + 5.0 < r["t_world"] <= t0 + 10.0:
                d_last.append(d)
        m_first = sum(d_first) / len(d_first) if d_first else None
        m_last = sum(d_last) / len(d_last) if d_last else None
        p3c = (m_first is not None and m_last is not None and m_last <= m_first + DIST_SLACK_M)
        # 以下只作辅助量（D10：上一轮自造的 P3d 不进判定）
        leg1 = next((r for r in ped_rows if r["leg"] >= 1), None)
        hold_end = leg1["world_time"] if leg1 else t0 + 20.0
        hold_rows = [r for r in uav if t0 <= r["t_world"] <= hold_end]
        c4_arrive = [r for r in arrives
                     if t0 <= r["t_world"] <= hold_end and dist((r["x"], r["y"]), CORNERS[3]) <= NAV_CORNER_TOL_M]
        min_x = min((r["x"] for r in hold_rows), default=None)
        goal_in = len([r for r in goals if t0 <= r["t_world"] <= t0 + 10.0])
        ok3 = p3a and p3b and p3c
        crit.append({"n": 3, "name": "进入跟随", "ok": ok3,
                     "detail": "t0=%.1f s（world）/%.1f s（rel t_send）；行 class=%s conf=%.2f、世界=(%.2f,%.2f)、"
                               "该点与行人实测位置相距 %s m；spawn=%s s（world）；"
                               "5 s 内 ①朝人 v>0=%d 行 / ②next_cell 朝人一步=%d 行；"
                               "10 s 位移 %s m；首/末 5 s 均距 %s→%s m；采纳类别 person=%s / 非person=%s；"
                               "辅助：min x=%s、驻留期 c4 到达=%d、10 s 内新目标=%d、next_cell 字面命中=%d"
                               % (t0, (t0 - t_send) if t_send is not None else float("nan"),
                                  det_in.get("cls", "-"), det_in.get("conf", float("nan")),
                                  det_in["x"], det_in["y"], fmt(d_t0_ped), fmt(t_ped_spawn),
                                  sub3a["toward_rows"], sub3a["step_rows"], fmt(disp),
                                  fmt(m_first), fmt(m_last),
                                  acc.get("accepted_person", "-"), acc.get("accepted_nonperson", "-"),
                                  fmt(min_x), len(c4_arrive), goal_in, sub3a["literal_hits"]),
                     "sub": {"a": p3a, "b": p3b, "c": p3c, "t0": t0,
                             "t_ped_spawn": t_ped_spawn, "t0_rel": (t0 - t_send) if t_send is not None else None,
                             "t0_cls": det_in.get("cls"), "t0_conf": det_in.get("conf"),
                             "t0_target": [det_in["x"], det_in["y"]], "t0_distance_to_ped_m": d_t0_ped,
                             "accepted_person": acc.get("accepted_person"),
                             "accepted_nonperson": acc.get("accepted_nonperson"),
                             "toward_rows": sub3a["toward_rows"],
                             "step_rows": sub3a["step_rows"],
                             "literal_hits": sub3a["literal_hits"],
                             "disp10_m": disp, "mean_d_first": m_first, "mean_d_last": m_last,
                             "min_x_hold": min_x, "c4_arrive_in_hold": len(c4_arrive),
                             "goals_in_10s": goal_in}})

    # ── P4 离开回巡逻（窗 [t_out, t_out+P4_S] = 20 s）
    t_out = ctx["anchors"].get("t_out")
    if t_out is None:
        crit.append({"n": 4, "name": "离开回巡逻", "ok": False,
                     "detail": "t_out 不存在（人没有出界）→ FAIL", "sub": {}})
    else:
        rows4 = [r for r in uav if t_out <= r["t_world"] <= t_out + P4_S]
        gw4 = [r for r in goals if t_out <= r["t_world"] <= t_out + P4_S]
        hits = [(r, hit_corner((r["x"], r["y"]))) for r in gw4]
        hits = [(r, c) for r, c in hits if c is not None]
        p4a = len(hits) > 0
        p4b = False
        p4detail_d = None
        if p4a and len(rows4) >= 2:
            target = hits[0][1]
            start = (rows4[0]["x"], rows4[0]["y"])
            end = (rows4[-1]["x"], rows4[-1]["y"])
            d0, d1 = dist(start, target), dist(end, target)
            p4detail_d = (d0, d1)
            p4b = d1 <= d0 - P4_DISP_MIN_M
        lit4 = sum(1 for r in rows4 if r["gx"] is not None and (r["gx"], r["gy"]) in CORNER_GRID)
        ok4 = p4a and p4b
        crit.append({"n": 4, "name": "离开回巡逻", "ok": ok4,
                     "detail": "t_out=%.1f s（world）/%.1f s（rel t_send）；窗 %.0f s；+%.1f s 新目标 %s；"
                               "该窗内朝该角点 %s→%s m（−%s m）；next_cell 字面命中 %d 行"
                               % (t_out, (t_out - t_send) if t_send is not None else float("nan"), P4_S,
                                  (hits[0][0]["t_world"] - t_out) if hits else float("nan"),
                                  str(hits[0][1]) if hits else "-",
                                  fmt(p4detail_d[0]) if p4detail_d else "-",
                                  fmt(p4detail_d[1]) if p4detail_d else "-",
                                  fmt(p4detail_d[0] - p4detail_d[1]) if p4detail_d else "-",
                                  lit4),
                     "sub": {"a": p4a, "b": p4b, "t_out": t_out, "t_out_rel": t_out - t_send,
                             "goal_corner": str(hits[0][1]) if hits else None,
                             "d_start": p4detail_d[0] if p4detail_d else None,
                             "d_end": p4detail_d[1] if p4detail_d else None,
                             "next_cell_literal": lit4}})

    # ── P5 安全（全程）
    out_rows = [r for r in uav if not (0.0 < r["x"] < BOUND_M and 0.0 < r["y"] < BOUND_M)]
    jumps = []
    max_rate = 0.0
    for a, b in zip(uav, uav[1:]):
        dt = b["t_world"] - a["t_world"]
        if dt <= 0:
            continue
        d = dist((a["x"], a["y"]), (b["x"], b["y"]))
        if dt <= JUMP_DT_MAX_S and d > JUMP_M:
            jumps.append({"t": b["t_world"] - t_send, "d": d, "dt": dt,
                          "from": [a["x"], a["y"]], "to": [b["x"], b["y"]]})
        if d / dt > max_rate:
            max_rate = d / dt
    p5a = len(out_rows) == 0
    p5b = len(jumps) == 0
    dead_wins = []
    i = 0
    while i < len(uav):
        j = i
        while j + 1 < len(uav) and uav[j + 1]["t_world"] - uav[i]["t_world"] <= DEAD_WIN_S:
            j += 1
        win = uav[i:j + 1]
        if len(win) >= 3 and win[-1]["t_world"] - win[0]["t_world"] >= DEAD_WIN_S - 0.4:
            span = max(dist((win[0]["x"], win[0]["y"]), (r["x"], r["y"])) for r in win)
            cmd = all(abs(r["v"]) >= DEAD_MIN_V for r in win)
            if span < DEAD_EPS_M and cmd:
                dead_wins.append({"t": win[0]["t_world"] - t_send, "span": span})
                i = j
        i += 1
    p5c = len(dead_wins) == 0
    bbox = (min((r["x"] for r in uav), default=None), max((r["x"] for r in uav), default=None),
            min((r["y"] for r in uav), default=None), max((r["y"] for r in uav), default=None))
    ok5 = p5a and p5b and p5c
    crit.append({"n": 5, "name": "安全", "ok": ok5,
                 "detail": "bbox x[%s,%s] y[%s,%s]；越界 %d 行；突跳 %d 次（max %s m/s）；卡死 %d 窗"
                           % (fmt(bbox[0]), fmt(bbox[1]), fmt(bbox[2]), fmt(bbox[3]),
                              len(out_rows), len(jumps), fmt(max_rate), len(dead_wins)),
                 "sub": {"a": p5a, "b": p5b, "c": p5c, "bbox": bbox,
                         "jumps": jumps[:5], "dead": dead_wins[:5], "max_speed_mps": max_rate,
                         "out_rows": len(out_rows)}})

    # ── P6 配置生效（v=0.8±0.05 / 位姿龄 ≤120 ms / detector ≥1 Hz / model=yolov8s）
    vs = [r["v"] for r in uav]
    vs_pos = [v for v in vs if v > 0]
    v_med = median(vs_pos)
    v_zero_ratio = (len(vs) - len(vs_pos)) / len(vs) if vs else None
    v_hit = any(abs(v - V_TARGET) <= V_TOL for v in vs)
    p6a = v_hit and v_med is not None and (V_TARGET - V_TOL) <= v_med <= (V_TARGET + V_TOL)
    ages = [r["age"] for r in uav]
    age_med = median(ages)
    p6b = age_med is not None and age_med <= POSE_AGE_MAX_MS
    hz = None
    if len(det) >= 2:
        span_ms = det[-1]["fts_ms"] - det[0]["fts_ms"]
        hz = (len(det) - 1) / (span_ms / 1000.0) if span_ms > 0 else None
    p6c = hz is not None and hz >= DET_MIN_HZ
    infer_med = median([r["infer_ms"] for r in det])
    det_lines = [ln for ln in (ctx["sim_text"] or "").splitlines() if "[detector] 已启动" in ln]
    p6d = any("model=yolov8s.safetensors" in ln for ln in det_lines)
    ok6 = p6a and p6b and p6c and p6d
    crit.append({"n": 6, "name": "配置生效", "ok": ok6,
                 "detail": "v 中位 %s（min %s/max %s，v=0 占 %s%%；∃|v−%.1f|≤%.2f=%s）；"
                           "位姿龄 中位 %s ms / p90 %s ms；detector %s Hz（%d 帧，推理中位 %s ms）；"
                           "model=yolov8s：%s（%d 行 `[detector] 已启动`）"
                           % (fmt(v_med, 3), fmt(min(vs) if vs else None), fmt(max(vs) if vs else None),
                              fmt((v_zero_ratio or 0.0) * 100.0, 0), V_TARGET, V_TOL, v_hit,
                              fmt(age_med, 0), fmt(percentile(ages, 0.9), 0),
                              fmt(hz, 2), len(det), fmt(infer_med, 0),
                              "有" if p6d else "无", len(det_lines)),
                 "sub": {"a": p6a, "b": p6b, "c": p6c, "d": p6d, "v_median": v_med,
                         "v_min": min(vs) if vs else None, "v_max": max(vs) if vs else None,
                         "v_target_hit": v_hit,
                         "v_zero_ratio": v_zero_ratio, "age_median": age_med,
                         "age_p90": percentile(ages, 0.9), "det_hz": hz, "det_rows": len(det),
                         "infer_median_ms": infer_med,
                         "det_start_line": (det_lines[0] if det_lines else None)}})

    # ── P7 无异常（a）/ 收尾（b）/ 边界（c）——b/c 在收尾后由 finalize 填写
    crit.append({"n": 7, "name": "无异常", "ok": False, "detail": "（待收尾）", "sub": {}})
    ctx["criteria"] = crit
    ctx["uav_rows"] = uav
    ctx["det_rows"] = det
    ctx["goal_rows"] = goals
    ctx["arrive_rows"] = arrives
    return crit


def count_anomalies(texts):
    """P7a：四份日志里的异常计数（允许清单不算）。"""
    res = {"panic": 0, "task_fail": 0, "blocked": 0, "script_error": 0, "parse_error": 0, "error": 0}
    samples = []
    for name, text in texts.items():
        for ln in text.splitlines():
            if any(n in ln for n in KNOWN_NOISE):
                continue
            if PANIC_RE.search(ln):
                res["panic"] += 1
                samples.append("%s: %s" % (name, ln[:160]))
            if FAIL_RE.search(ln):
                res["task_fail"] += 1
                samples.append("%s: %s" % (name, ln[:160]))
            if "拍无路" in ln or " Blocked" in ln or "Blocked(" in ln:
                res["blocked"] += 1
                samples.append("%s: %s" % (name, ln[:160]))
            if "SCRIPT ERROR" in ln:
                res["script_error"] += 1
                samples.append("%s: %s" % (name, ln[:160]))
            if "Parse Error" in ln:
                res["parse_error"] += 1
                samples.append("%s: %s" % (name, ln[:160]))
            if ERROR_RE.search(ln):
                res["error"] += 1
                samples.append("%s: %s" % (name, ln[:160]))
    return res, samples[:10]


# ================================================================ §11 产物


TRACE_FIELDS = ["t_world", "x", "y", "z", "yaw", "v", "omega", "next_cell",
                "person_x", "person_y", "dist"]


def write_trace(out, uav, ped_rows, status_rows, clock):
    n = 0
    k = clock.get("k") or 0.0
    a = clock.get("a") or 0.0
    with (out / "trace.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=TRACE_FIELDS)
        w.writeheader()
        for r in uav:
            p = ped_pos_at(ped_rows, r["t_world"])
            d = dist((r["x"], r["y"]), p) if p else None
            # D7：`[Sim] UAV` 行不含 z → 用 200 ms status 采样的近邻（±0.5 s）填
            t_wall = (r["t_world"] - a) / k if k else r["t_world"]
            z = nearest_z(status_rows, t_wall)
            w.writerow({
                "t_world": "%.4f" % r["t_world"],
                "x": "%.2f" % r["x"], "y": "%.2f" % r["y"],
                "z": "" if z is None else "%.3f" % z,
                "yaw": "%.1f" % r["yaw"], "v": r["v"], "omega": r["omega"],
                "next_cell": "" if r["gx"] is None else "%d,%d" % (r["gx"], r["gy"]),
                "person_x": "" if p is None else "%.3f" % p[0],
                "person_y": "" if p is None else "%.3f" % p[1],
                "dist": "" if d is None else "%.3f" % d,
            })
            n += 1
    return n


def write_status_jsonl(out, status_rows):
    with (out / "status.jsonl").open("w", encoding="utf-8") as f:
        for r in status_rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    return len(status_rows)


def ensure_artifacts(out, ctx):
    """保证 Task §4.5 的 7 件主产物**一定存在**（早退时写占位，注明原因）。

    上一轮 04:22 的评审 issue #3：放人前终止时 `ped.log` 缺失 ⇒ 邮件附件里的人类看不到产物全貌。
    占位文本刻意不含 ERROR/panic/失败 等异常关键词（避免污染 P7a 计数）。
    """
    made = []
    why = ctx.get("fatal") or "本轮到该阶段前终止（见 driver.log / verdict 的致命闸门段）"
    for name in ("sim.log", "world.log", "pictor.log", "ped.log"):
        p = out / name
        if not p.exists():
            p.write_text("[%s] 未启动：%s\n" % (name.split(".")[0], why), encoding="utf-8")
            made.append(name)
    if not (out / "trace.csv").exists():
        (out / "trace.csv").write_text(",".join(TRACE_FIELDS) + "\n", encoding="utf-8")
        made.append("trace.csv")
    if not (out / "status.jsonl").exists():
        (out / "status.jsonl").write_text("", encoding="utf-8")
    return made


def refresh_placeholders(out, names):
    """只保留「内容仍是占位」的产物名（`ensure_artifacts` 之后被真实内容覆盖的不算）。"""
    keep = []
    for n in names:
        p = out / n
        try:
            head = p.read_text(encoding="utf-8").splitlines()[0] if p.is_file() else ""
        except OSError:
            head = ""
        if "未启动" in head:
            keep.append(n)
    return keep


def write_verdict(out, ctx, residual, busy, zombie, xlocks):
    crit = ctx["criteria"]
    by = {c["n"]: c for c in crit}
    L = []
    L.append("# task_41 Step 2 verdict — sim 机栈 containment（tree v1，只验不改）")
    L.append("- 口径：UAV 0.8 m/s（`vel_fwd`）/ detector `yolov8s.safetensors`（CPU）/ 行人 0.5 m/s；"
             "判据窗口 P2=%.0f s、P3=%.0f+%.0f s、P4=%.0f s"
             % (A_OBSERVE_S, SWITCH_S, 10.0, P4_S))
    L.append("- 运行：%s；rc=%s；总时长 %s s；用例 A/B/C 时长 %s / %s / %s s"
             % (ctx["utc"], ctx.get("rc", "-"), fmt(ctx.get("elapsed_s"), 1),
                fmt(ctx.get("dur_A"), 1), fmt(ctx.get("dur_B"), 1), fmt(ctx.get("dur_C"), 1)))
    _sp = ctx.get("ped_speed") or (None, 0)
    L.append("- 行人口径：命令 %.2f m/s（§二.13）；实测中位 %s m/s（n=%s，`ped_trace.csv` 相邻行 Δs/Δt）"
             % (PED_SPEED, fmt(_sp[0], 3), _sp[1]))
    L.append("- 环境：world_fast（diff=空）；`xvfb-run -a`；%s（vel_fwd=0.8，yaw_rate=30.0，"
             "listen=%d，ctl=%d，detector=on/model=yolov8s）；Pictor ws=%d（CWD=Pictor（kernel 读 "
             "Pictor/.config，D8），net=%d）；"
             "行人 spawn (%.0f,%.0f) → 停 (%.0f,%.0f) 20 s → (64,64) → (64,50)，%.1f m/s（D4/D9）"
             % (PEER_UAV, UAV_NET_PORT, CTL_PORT, WS_PORT, PIC_NET_PORT,
                PED_SPAWN[0], PED_SPAWN[1], PED_STOP[0], PED_STOP[1], PED_SPEED))
    cj = (ctx.get("ctl_A") or {}).get("json") or {}
    L.append("- 任务帧：`%s`（%s B，type=%s，cx=64 cy=64 half=8），Pictor ack ok=%s"
             % (cj.get("payload_hex", "-"), cj.get("payload_len", "-"),
                cj.get("type_on_wire", "-"), cj.get("ack_ok", "-")))
    ck = ctx["clock"]
    L.append("- 时钟：t_world = a + k·t_log（k=%.6f，n=%d，max_resid=%.4f s，源=%s；ped 交叉校验差 %s s）"
             % (ck["k"], ck["n"], ck["max_resid"], ck["source"], fmt(ck.get("cross_check_s"), 3)))
    L.append("")
    L.append("| # | 判据 | 实测 | 阈值 | 结果 |")
    L.append("|---|---|---|---|---|")
    thr = {1: "ack+14 B+建树", 2: "≥2 角 ≤1.0 m（150 s）", 3: "5 s / ≥3 m / ≤+2 m",
           4: "20 s / ≥2 m", 5: "0 越界/0 突跳/0 卡死",
           6: "0.8±0.05 / ≤120 ms / ≥1 Hz / yolov8s", 7: "0 异常 / 0 残留"}
    for n in range(1, 8):
        c = by.get(n)
        if c is None:
            L.append("| P%d | - | 未计算 | %s | FAIL(env) |" % (n, thr.get(n, "-")))
            continue
        tag = "PASS" if c["ok"] else ("FAIL(env)" if c.get("env") else "FAIL")
        detail = c["detail"].replace("|", "/")
        L.append("| P%d | %s | %s | %s | %s |" % (n, c["name"], detail, thr.get(n, "-"), tag))
    L.append("")
    ok_all = all(c["ok"] for c in crit) and len(crit) == 7 and not residual and not busy
    L.append("**结论：%s**" % ("PASS（P1~P7 全过）" if ok_all else "FAIL（见上表；只验不改：未调参、未改判据、未重跑）"))
    L.append("")
    if ctx.get("fatal") or any(c.get("env") for c in crit):
        L.append("## 致命闸门与证据（FAIL(env) 时人类先看这段）")
        L.append("- 致命：%s" % (ctx.get("fatal") or "（判据取不到实测值）"))
        L.append("")
        L.append("| 闸门 | 结果 | 证据 |")
        L.append("|---|---|---|")
        for g in ctx.get("gates", []):
            L.append("| %s | %s | %s |" % (g["name"], "OK" if g["ok"] else "FAIL",
                                           str(g["detail"]).replace("|", "/")))
        for title, path, n in (("sim.log", out / "sim.log", 15),
                               ("pictor.log", out / "pictor.log", 10),
                               ("pictor_kernel.log", out / "pictor_kernel.log", 15),
                               ("driver.log", out / "driver.log", 10)):
            tail = [ln for ln in read_text(path).splitlines() if ln.strip()][-n:]
            if not tail:
                continue
            L.append("")
            L.append("### %s 尾部 %d 行" % (title, n))
            L.append("```text")
            L.extend(tail)
            L.append("```")
        L.append("")
    L.append("## 锚点（全部由脚本生成；world = 绝对世界时间，rel = 相对 t_send）")
    L.append("| 锚点 | world (s) | rel t_send (s) |")
    L.append("|---|---|---|")
    tw = make_to_world(ck)
    t_send_ref = ctx["anchors"].get("t_send_A_wall")
    t_send_w = tw(t_send_ref) if t_send_ref is not None else None
    for label, key, kind in (("t_send", "t_send_A_wall", "wall"),
                             ("t_A_end", "t_A_end_wall", "wall"),
                             ("t_release", "t_release_wall", "wall"),
                             ("t_ped_spawn", "t_ped_spawn", "world"),
                             ("t0", "t_first_in_square_detect", "world"),
                             ("t_out", "t_out", "world"),
                             ("t_run_end", "t_run_end", "wall")):
        val = ctx["anchors"].get(key)
        if val is None:
            L.append("| %s | - | - |" % label)
            continue
        if kind == "wall":
            val = tw(val)
        L.append("| %s | %.3f | %s |"
                 % (label, val, "-" if t_send_w is None else "%.3f" % (val - t_send_w)))
    L.append("")
    L.append("## 偏差登记（D1~D11）")
    L.append("- D1 只复用 `sim_queue_test.py` / `sim_task40_ugv_test.py`，不 import `sim_follow_test`（显示走 `xvfb-run -a`）")
    L.append("- D2 `next_cell` 是相邻格 ⇒ P2b/P4a 用 `[Nav] 新目标`（§6.2/§6.4）；字面计数：P2 %s 行、P4 %s 行"
             % (str((by.get(2) or {}).get("sub", {}).get("next_cell_literal", "-")),
                str((by.get(4) or {}).get("sub", {}).get("next_cell_literal", "-"))))
    L.append("- D3 放人：trigger=%s（L 字面命中 %s 次，预期 0），等待 %s s；v_eff=%s m/s、y_target=%s、"
             "trigger_row=%s、skip_passes=%s"
             % (ctx["release"].get("trigger_used"), ctx["release"].get("literal_hits"),
                fmt(ctx["release"].get("wait_s"), 1), fmt(ctx["release"].get("v_eff"), 3),
                fmt(ctx["release"].get("y_target"), 2),
                ctx["release"].get("trigger_row"), ctx["release"].get("skip_passes")))
    L.append("- D4 行人停点 (64,71) vs 文档 (64,72)；实测边界余量 y=%.2f（界内 %.2f m）"
             % (PED_STOP[1], (CY + HALF) - PED_STOP[1]))
    L.append("- D5 卡死 = 5 s 窗内 `|v|≥0.5` 且位移 <0.05 m（Follow Hold 的 v=0 不算）")
    L.append("- D6 时间轴 = %s（退化路径见 §6.0）" % ck["source"])
    L.append("- D7 `[Sim] UAV` 无 z ⇒ trace 的 z 由 200 ms status 近邻填充（status.jsonl 全量留证）")
    L.append("- D8 kernel 的 CWD=`Pictor/`：只读校验 `Pictor/.config/config.toml`（9123/pictor_s2/ground_station，"
             "sha=%s），并从 `Pictor/Log` 实证「监听地址 …/tcp/%d/」；**不写 Pictor/**"
             % ((ctx.get("pictor_cfg_sha_before") or "-")[:16], PIC_NET_PORT))
    L.append("- D9 行人用 `Tool/sim_task41_pedestrian.py`（内部 import 同一个 `PedestrianClient`）")
    L.append("- D10 P3a 用 ① 朝人 v>0 ±20° / ② `next_cell` 朝人一步 ≤1.0 m ±25°（follow 叶不调 "
             "`nav.set_goal`，故无 `[Nav] 新目标`）；`min_x` / c4 到达降为辅助量")
    L.append("- D11 行人命令 0.5 m/s，实测中位 %s m/s（世界侧起步转弯 + 0.30 m 到点容差；不改用例速度）"
             % fmt((ctx.get("ped_speed") or (None,))[0], 3))
    L.append("")
    L.append("## 关键日志（≤10 行）")
    if not ctx.get("key_lines"):
        ctx["key_lines"] = ([ln for ln in read_text(out / "pictor.log").splitlines() if ln.strip()][-5:]
                            + ["（本轮未发出 task_set / 无 sim 日志：见上节致命闸门与证据）"])
    L.append("```text")
    for ln in ctx.get("key_lines", [])[:10]:
        L.append(ln.rstrip())
    L.append("```")
    L.append("")
    L.append("## 收尾")
    L.append("- 残留进程：%d（活）/%d（<defunct> 忽略）；端口 %s：%s；`/tmp/.X*-lock`：%d"
             % (len(residual), zombie, list(PORTS), "全空" if not busy else "占用 %s" % busy, xlocks))
    if residual:
        L.extend("- 残留：%s" % x for x in residual[:5])
    (out / "verdict.md").write_text("\n".join(L) + "\n", encoding="utf-8")
    return ok_all


def write_cases_json(out, ctx, preflight_checks, pictor_snapshot):
    cj = (ctx.get("ctl_A") or {}).get("json") or {}
    data = {
        "run_tag": out.name,
        "started_utc": ctx.get("started_utc"),
        "finished_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exit_code": ctx.get("rc"),
        "env": {
            "world_fast_stamp": read_text(WORLD_FAST / ".godot" / ".copied_from").strip(),
            "world_diff": S2.world_diff_extra()[1],
            "world_copy_sync": S2.world_copy_sync(),
            "sim_bin": {"path": str(RELEASE_BIN), "sha256": Q.sha256_file(RELEASE_BIN)},
            "yolo_sha256": Q.sha256_file(YOLO_SRC) if YOLO_SRC.is_file() else None,
            "pictor_so": KERNEL_SO.is_file(),
            "ports_free_at_start": {str(p): (not Q.port_busy(p)) for p in PORTS},
            "git_baseline_frozen_equal": all(c["ok"] for c in preflight_checks if c["n"] == 13),
            "pictor_status_snapshot": pictor_snapshot,
        },
        "ports": {"world": WORLD_PORT, "pictor_ws": WS_PORT, "pictor_net": PIC_NET_PORT,
                  "uav_ctl": CTL_PORT, "uav_net": UAV_NET_PORT},
        "pictor": {"cwd": str(PICTOR_CWD),
                   "config": str(PICTOR_CFG.relative_to(ROOT)),
                   "config_sha256_before": ctx.get("pictor_cfg_sha_before"),
                   "kernel_log": ctx.get("pictor_kernel_log"),
                   "kernel_listen_port": PIC_NET_PORT},
        "gates": ctx.get("gates", []),
        "fatal": ctx.get("fatal"),
        "placeholders": ctx.get("placeholders", []),
        "mission": {"type": GOLDEN_TYPE, "x": CX, "y": CY, "half_side": HALF,
                    "payload_hex": cj.get("payload_hex"), "payload_len": cj.get("payload_len"),
                    "ack_ok": cj.get("ack_ok"), "sent_at_wall": ctx["anchors"].get("t_send_A_wall")},
        "square": {"cx": CX, "cy": CY, "half": HALF, "corners": [list(c) for c in CORNERS],
                   "corner_grid": [list(g) for g in CORNER_GRID]},
        "person": {"spawn": list(PED_SPAWN), "stop": list(PED_STOP), "doc_stop": list(PED_DOC_STOP),
                   "speed": PED_SPEED,
                   "measured_speed_median": (ctx.get("ped_speed") or (None,))[0],
                   "legs": [{"to": [64, 71], "dwell_s": 20}, {"to": [64, 64], "dwell_s": 3},
                            {"to": [64, 50], "dwell_s": 0}],
                   "peer_id": PEER_PED, "arrive_tol_m": PED_ARRIVE_TOL},
        "release": ctx["release"],
        "anchors": ctx["anchors"],
        "clock": ctx["clock"],
        "windows": {"P2": [0, A_OBSERVE_S], "P3a_s": SWITCH_S, "P3_disp_s": 10.0,
                    "P4_s": P4_S, "P5": "全程",
                    "P3_t0_gate": "> t_ped_spawn"},
        "counts": ctx.get("counts"),
        "caps": {"release_wait_s": RELEASE_WAIT_S, "B_s": B_OBSERVE_S, "C_s": C_OBSERVE_S,
                 "max_s": ctx.get("max_s")},
        "deviations": ["D1 只复用 sim_queue_test / sim_task40_ugv_test（显示走 xvfb-run -a）",
                       "D2 next_cell 是相邻格 → P2b/P4a 用 [Nav] 新目标，字面计数照录",
                       "D3 放人：L 字面条件（预期 0）+ A 上行段相位锁（y_target = 74 − 20·v_eff）",
                       "D4 行人停点 (64,71) vs 文档 (64,72)",
                       "D5 卡死 = 5 s 窗内 |v|≥0.5 且位移 <0.05 m",
                       "D6 时间轴 = a + k·t_log（detector frame_ts 主标定）",
                       "D7 [Sim] UAV 无 z → status 近邻填充",
                       "D8 kernel CWD=Pictor：只读校验 Pictor/.config（9123/pictor_s2/ground_station），"
                       "不写 Pictor/；pictor_ws 旧配置不用",
                       "D9 行人 = Tool/sim_task41_pedestrian.py（import 同一个 PedestrianClient）",
                       "D10 P3a = ① 朝人 v>0 ±20° 或 ② next_cell 朝人一步 ≤1.0 m ±25°；"
                       "min_x / c4 到达只作辅助量",
                       "D11 行人命令 0.5 m/s，世界侧实测中位速度见 person.measured_speed_median"
                       "（起步转弯 + 0.30 m 到点容差）"],
        "preflight": preflight_checks,
        "criteria": ctx["criteria"],
        "residual_procs": ctx.get("residual", []),
        "ports_busy": ctx.get("busy_ports", []),
        "duration": {"A": ctx.get("dur_A"), "B": ctx.get("dur_B"), "C": ctx.get("dur_C"),
                     "total": ctx.get("elapsed_s")},
    }
    write_json(out / "cases.json", data)


# ================================================================ §12 主流程


def pick_out(args):
    if args.out:
        return pathlib.Path(args.out).resolve()
    return (RESULTS / ("task41_s2_" + utc_stamp())).resolve()


def run_selfcheck(out, max_s):
    """只起停环境（等 spawn/detector/相机/Pictor 注册），**不发任务、不算判据**。"""
    Log("=" * 92)
    Log("task_41 Step 2 — 环境自检（不发 task_set、不写 results/）  产物 → %s" % out)
    Log("=" * 92)
    out.mkdir(parents=True, exist_ok=True)
    checks = preflight(out)
    bad = [c for c in checks if not c["ok"]]
    if bad:
        for c in bad:
            Log("[PREFLIGHT FAIL] #%d %s：%s" % (c["n"], c["name"], c["detail"]))
        return 2
    t_start = time.time()
    deadline = t_start + max_s
    procs = {}
    rc = 0
    try:
        procs["world"] = start_proc("world", S2.world_argv(), ROOT, out / "world.log")
        dt = wait_text(out / "world.log", "[ws] listening on port %d" % WORLD_PORT, WORLD_LISTEN_S)
        Log("  [自检] 世界监听 :%d → %s" % (WORLD_PORT, "%.1f s" % dt if dt else "超时"))
        if dt is None:
            rc = 2
            return rc
        before = read_text(out / "world.log").count("vehicle spawned")
        # step §8 / D8：Pictor 先起；WS 9100 与 kernel libp2p 9123（**实证监听端口**）都就绪后才起 sim
        pre_kernel = ({p.name for p in PICTOR_LOG.glob("pleiades.log.*")}
                      if PICTOR_LOG.is_dir() else set())
        procs["pictor"] = start_proc(
            "pictor",
            ["timeout", str(PICTOR_TIMEOUT_S), "godot", "--headless", "--path", str(PICTOR),
             "--", "--ws-port=%d" % WS_PORT], PICTOR_CWD, out / "pictor.log")
        dt_pic = wait_text(out / "pictor.log", "[WsCmd] listening on 127.0.0.1:%d" % WS_PORT,
                           PICTOR_LISTEN_S)
        Log("  [自检] Pictor listening → %s" % ("%.1f s" % dt_pic if dt_pic is not None else "超时"))
        klog, kports, t_k = None, [], time.time()
        while time.time() - t_k < PICTOR_KERNEL_S:
            fresh = ([p for p in PICTOR_LOG.glob("pleiades.log.*") if p.name not in pre_kernel]
                     if PICTOR_LOG.is_dir() else [])
            fresh.sort(key=lambda p: p.stat().st_mtime)
            if fresh:
                klog = fresh[-1]
                kports = sorted({int(m.group("port"))
                                 for m in KERNEL_LISTEN_RE.finditer(read_text(klog))})
                if kports:
                    break
            time.sleep(0.2)
        Log("  [自检] kernel 监听端口 → %s（日志 %s）"
            % (kports or "无", klog.name if klog is not None else "<无>"))
        if kports != [PIC_NET_PORT]:
            rc = 2
            return rc
        procs["sim"] = start_proc("sim", [str(INST / "pleiades-sim"), "--headless"], INST,
                                  out / "sim_stdout.log")
        tail = Q.LogTail(INST / "Log")
        tail.reset()

        def sim_has(needle, timeout):
            t0 = time.time()
            while time.time() - t0 < timeout:
                path = tail.newest()
                if path is not None and needle in read_text(path):
                    return round(time.time() - t0, 2)
                time.sleep(0.3)
            return None
        dt_spawn = None
        t0 = time.time()
        while time.time() - t0 < SPAWN_S:
            if read_text(out / "world.log").count("vehicle spawned") > before:
                dt_spawn = round(time.time() - t0, 2)
                break
            time.sleep(0.2)
        Log("  [自检] vehicle spawned → %s" % ("%.1f s" % dt_spawn if dt_spawn is not None else "超时"))
        dt_det = sim_has("[detector] 已启动", DETECTOR_S)
        Log("  [自检] `[detector] 已启动` → %s" % ("%.1f s" % dt_det if dt_det is not None else "超时"))
        dt_cam = sim_has("slot=camera_nose 首帧", CAMERA_S)
        Log("  [自检] `[camera] slot=camera_nose 首帧` → %s" % ("%.1f s" % dt_cam if dt_cam is not None else "超时"))
        ctl = Q.Control(CTL_PORT)
        ok_ctl = ctl.connect(retries=240, delay=0.25)
        Log("  [自检] 控制口 %d → %s" % (CTL_PORT, "可连" if ok_ctl else "连不上"))
        if ok_ctl:
            st = ctl.status()
            Log("  [自检] status assembled=%s pos=(%.2f,%.2f,%.2f) yaw=%.1f° "
                % (st.get("assembled"), st.get("x", 0.0), st.get("y", 0.0), st.get("z", 0.0),
                   math.degrees(float(st.get("yaw", 0.0)))))
            ctl.close()
        rc, so, se = S2.run_capture([sys.executable, str(CTL), "wait", "--target", PEER_UAV,
                                     "--port", str(WS_PORT), "--json",
                                     "--timeout", str(REGISTER_S)], timeout=REGISTER_S + 30)
        Log("  [自检] ctl wait %s → rc=%s %s" % (PEER_UAV, rc, so.strip().splitlines()[-1:] or se[:120]))
        if any(x is None for x in (dt_spawn, dt_det, dt_cam)) or not ok_ctl or rc != 0:
            rc = 2
        else:
            rc = 0
    finally:
        Log("\n  [自检] 收尾 …")
        for name in ("pictor", "sim", "world"):
            p = procs.pop(name, None)
            if p is not None:
                p.stop()
        residual, busy, zombie = wait_clean(15.0)
        Log("  残留 %d 活 / %d 僵尸；端口 %s；rc=%d" % (len(residual), zombie, busy or "全空", rc))
        write_json(out / "selfcheck.json",
                   {"utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                    "rc": rc, "residual": residual, "busy_ports": busy,
                    "elapsed_s": round(time.time() - t_start, 1)})
    return rc


def new_ctx(out, max_s):
    """主流程的 ctx 字面量（plan §2 E6）；`hard_deadline` 由调用方按进程起点覆盖。"""
    t = time.time()
    return {
        "out": out, "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "hard_deadline": t + max_s, "max_s": max_s,
        "procs": {}, "residual": [], "busy_ports": [], "gates": [], "fatal": None,
        "anchors": {}, "release": {"trigger_used": None, "literal_hits": 0,
                                   "literal_wait_s": None, "wait_s": None,
                                   "v_eff": None, "y_target": None, "trigger_row": None,
                                   "skip_passes": 0, "t_release_wall": None},
        "p1": {}, "clock": {"k": 1.0, "a": 0.0, "n": 0, "max_resid": 0.0,
                            "source": "n/a", "cross_check_s": None},
        "counts": {}, "criteria": [], "key_lines": [], "ctl_A": None, "sim_text": "",
        "rc": None, "dur_A": None, "dur_B": None, "dur_C": None,
        "pictor_kernel_log": None, "pictor_cfg_sha_before": None,
    }


def main():
    ap = argparse.ArgumentParser(
        description="task_41 Step 2 — sim 机栈 containment 验证（tree v1，只验不改）")
    ap.add_argument("--out", default=None, help="产物目录（默认 RobotSim_TestEnv/results/task41_s2_<UTC>）")
    ap.add_argument("--dry-run", action="store_true", help="只跑前置检查，不起任何进程")
    ap.add_argument("--selfcheck", action="store_true", help="起停一轮环境自检（不发 task_set）")
    ap.add_argument("--only", default=None, help="只裁剪 stdout 打印的判据，如 P1,P3")
    ap.add_argument("--max-s", type=int, default=900, help="全脚本时长上限（默认 900）")
    ap.add_argument("--keep", action="store_true", help="结束时**不**停进程（调试用）")
    args = ap.parse_args()

    only = None
    if args.only:
        only = {x.strip().upper() for x in args.only.split(",") if x.strip()}

    out = pick_out(args)
    out.mkdir(parents=True, exist_ok=True)
    t_process = time.time()
    log_path = out / "driver.log"
    fh = open(log_path, "w", buffering=1, encoding="utf-8")

    def Log2(msg=""):
        print(msg, flush=True)
        try:
            fh.write(msg + "\n")
        except Exception:
            pass

    global Log
    Log = Log2

    if args.selfcheck:
        rc = run_selfcheck(out, args.max_s)
        fh.close()
        Log("\nEXIT=%d" % rc)
        return rc

    Log("=" * 92)
    Log("task_41 Step 2 — sim 机栈 containment 验证（tree v1，只验不改）  产物 → %s" % out)
    Log("=" * 92)
    Log("\n── 前置检查 ─────────────────────────────────────────")
    checks = preflight(out)
    bad = [c for c in checks if not c["ok"]]
    if args.dry_run:
        Log("\nPREFLIGHT %s（%d/%d 通过；起进程后的闸门 dry-run 跳过）"
            % ("FAIL" if bad else "OK", len(checks) - len(bad), len(checks)))
        for c in bad:
            Log("  [FAIL] #%d %s：%s" % (c["n"], c["name"], c["detail"]))
        fh.close()
        return 0 if not bad else 2
    if bad:
        for c in bad:
            Log("[PREFLIGHT FAIL] #%d %s：%s" % (c["n"], c["name"], c["detail"]))
        fh.close()
        return 2

    ctx = new_ctx(out, args.max_s)
    ctx["hard_deadline"] = t_process + args.max_s
    ctx["pictor_cfg_sha_before"] = PICTOR_CFG_INFO.get("sha") or None
    pictor_snapshot = git_pictor_now() or []
    os.environ["LIBGL_ALWAYS_SOFTWARE"] = "1"
    world = sim = pictor = None
    sampler = None
    sctl = None
    exit_code = 2
    fatal = None
    recs = []
    try:
        def gate(name, ok, detail):
            ctx["gates"].append({"name": name, "ok": bool(ok), "detail": detail,
                                 "t_wall": round(time.time(), 3)})
            Log("  闸门 %-22s %s  %s" % (name, "OK" if ok else "FAIL", detail))
            return ok

        # ── 起世界
        Log("\n── 起世界（world_fast, xvfb-run -a）──")
        world = start_proc("world", S2.world_argv(), ROOT, out / "world.log")
        dt = wait_text(out / "world.log", "[ws] listening on port %d" % WORLD_PORT, WORLD_LISTEN_S)
        gate("world_listen", dt is not None, "%.1f s" % dt if dt is not None else "timeout")
        if dt is None:
            fatal = "世界 %.0f s 内没有监听 :%d（FAIL(env)）" % (WORLD_LISTEN_S, WORLD_PORT)
            exit_code = 2
            return 2
        Log("  世界监听 :%d（%.1f s）" % (WORLD_PORT, dt))

        # ── 起 Pictor（先起；等 WS + kernel libp2p 端口就绪，避免两边同时 dial）
        Log("\n── 起 Pictor ──")
        # D8：kernel 每次运行新建一份日志；起进程前快照旧文件，之后只认「新出现的」那份
        pre_kernel = ({p.name for p in PICTOR_LOG.glob("pleiades.log.*")}
                      if PICTOR_LOG.is_dir() else set())
        t_pic0 = time.time()
        pictor = start_proc("pictor",
                            ["timeout", str(PICTOR_TIMEOUT_S), "godot", "--headless",
                             "--path", str(PICTOR), "--", "--ws-port=%d" % WS_PORT],
                            PICTOR_CWD, out / "pictor.log")
        dt_pic0 = wait_text(out / "pictor.log", "[WsCmd] listening on 127.0.0.1:%d" % WS_PORT,
                            PICTOR_LISTEN_S)
        gate("pictor_ws_9100", dt_pic0 is not None,
             "%.1f s" % dt_pic0 if dt_pic0 is not None else "timeout")
        if dt_pic0 is None:
            fatal = "Pictor WS %.0f s 内未监听 :%d（FAIL(env)）" % (PICTOR_LISTEN_S, WS_PORT)
            exit_code = 2
            return 2

        # D8/step §8：kernel 必须真的绑在 9123（不是「本地能连上」就算）
        klog, kports, t_k = None, [], time.time()
        while time.time() - t_k < PICTOR_KERNEL_S:
            fresh = ([p for p in PICTOR_LOG.glob("pleiades.log.*") if p.name not in pre_kernel]
                     if PICTOR_LOG.is_dir() else [])
            fresh.sort(key=lambda p: p.stat().st_mtime)
            if fresh:
                klog = fresh[-1]
                kports = sorted({int(m.group("port"))
                                 for m in KERNEL_LISTEN_RE.finditer(read_text(klog))})
                if kports:
                    break
            time.sleep(0.2)
        ctx["pictor_kernel_log"] = str(klog) if klog is not None else None
        if kports != [PIC_NET_PORT]:
            fatal = ("Pictor kernel 未绑定 :%d（实测 监听地址 端口=%s，日志=%s）→ FAIL(env)；"
                     "检查 Pictor/.config/config.toml 与 CWD=Pictor"
                     % (PIC_NET_PORT, kports or "无", klog))
            gate("kernel_libp2p_9123", False, "ports=%s" % (kports or "无"))
            exit_code = 2
            return 2
        gate("kernel_libp2p_9123", True, "监听地址 …/tcp/%d/（%s）" % (PIC_NET_PORT, klog.name))
        # 再确认「可连」（step §8 原文：都可连后才起 sim）
        try:
            with socket.create_connection(("127.0.0.1", PIC_NET_PORT), 0.5):
                pass
        except OSError as exc:
            fatal = "Pictor kernel :%d 已打印监听但 TCP 不可连（%r）→ FAIL(env)" % (PIC_NET_PORT, exc)
            gate("kernel_tcp_connect", False, repr(exc))
            exit_code = 2
            return 2
        gate("kernel_tcp_connect", True, "127.0.0.1:%d 可连" % PIC_NET_PORT)
        Log("  Pictor WS :%d + kernel :%d 就绪（%.1f s）"
            % (WS_PORT, PIC_NET_PORT, time.time() - t_pic0))

        Log("\n── 起机栈 ──")
        before = read_text(out / "world.log").count("vehicle spawned")
        sampler = LogSampler(INST / "Log")
        sampler.snapshot_preexisting()
        sim = start_proc("sim", [str(INST / "pleiades-sim"), "--headless"], INST,
                         out / "sim_stdout.log")
        t0 = time.time()
        dt_spawn = None
        while time.time() - t0 < SPAWN_S:
            if read_text(out / "world.log").count("vehicle spawned") > before:
                dt_spawn = round(time.time() - t0, 2)
                break
            time.sleep(0.2)
        if dt_spawn is None:
            fatal = "世界 %.0f s 内没有为机栈新增 `vehicle spawned`（FAIL(env)）" % SPAWN_S
            gate("world_vehicle_spawned", False, "timeout")
            exit_code = 2
            return 2
        gate("world_vehicle_spawned", True, "%.1f s" % dt_spawn)
        Log("  vehicle spawned（%.1f s）" % dt_spawn)

        ctl = Q.Control(CTL_PORT)
        if not ctl.connect(retries=240, delay=0.25):
            fatal = "控制口 %d 连不上（FAIL(env)）" % CTL_PORT
            gate("sim_assembled", False, "ctl :%d 连不上" % CTL_PORT)
            exit_code = 2
            return 2
        t0 = time.time()
        while time.time() - t0 < 120.0:
            if ctl.status().get("assembled"):
                break
            time.sleep(0.25)
        else:
            fatal = "120 s 内未 assembled（FAIL(env)）"
            gate("sim_assembled", False, "timeout")
            exit_code = 2
            return 2
        gate("sim_assembled", True, "%.1f s" % (time.time() - t0))
        Log("  已装配（%.1f s）" % (time.time() - t0))

        # 三道就绪闸门：detector / 相机首帧 / Pictor listening
        sampler.start()
        Log("  sim 日志 → %s" % (sampler.path.name if sampler.path is not None else "<无>"))

        def sim_has(needle, timeout):
            t0 = time.time()
            while time.time() - t0 < timeout:
                if needle in sampler.full_text():
                    return round(time.time() - t0, 2)
                time.sleep(0.3)
            return None
        dt_det = sim_has("[detector] 已启动", DETECTOR_S)
        gate("detector_started", dt_det is not None,
             "%.1f s" % dt_det if dt_det is not None else "timeout")
        Log("  `[detector] 已启动` → %s" % ("%.1f s" % dt_det if dt_det is not None else "超时"))
        dt_cam = sim_has("slot=camera_nose 首帧", CAMERA_S)
        gate("camera_first_frame", dt_cam is not None,
             "%.1f s" % dt_cam if dt_cam is not None else "timeout")
        Log("  `[camera] slot=camera_nose 首帧` → %s" % ("%.1f s" % dt_cam if dt_cam is not None else "超时"))
        dt_pic = wait_text(out / "pictor.log", "[WsCmd] listening on 127.0.0.1:%d" % WS_PORT,
                           PICTOR_LISTEN_S)
        gate("pictor_ws_recheck", dt_pic is not None,
             "%.1f s" % dt_pic if dt_pic is not None else "timeout")
        Log("  Pictor listening :%d → %s" % (WS_PORT, "%.1f s" % dt_pic if dt_pic is not None else "超时"))
        if dt_det is None or dt_cam is None or dt_pic is None:
            fatal = "就绪闸门未过（detector=%s 相机=%s pictor=%s）→ FAIL(env)" % (dt_det, dt_cam, dt_pic)
            exit_code = 2
            return 2

        st = ctl.status()
        Log("  出生位姿 (%.2f,%.2f,%.2f) yaw=%.1f°"
            % (st.get("x", 0.0), st.get("y", 0.0), st.get("z", 0.0),
               math.degrees(float(st.get("yaw", 0.0)))))
        rc, so, se = S2.run_capture([sys.executable, str(CTL), "wait", "--target", PEER_UAV,
                                     "--port", str(WS_PORT), "--json",
                                     "--timeout", str(REGISTER_S)], timeout=REGISTER_S + 30)
        Log("  ctl wait %s → rc=%s %s" % (PEER_UAV, rc, so.strip().splitlines()[-1:] or se[:120]))
        gate("pictor_registered", rc == 0, "ctl wait rc=%s" % rc)
        if rc != 0:
            fatal = "机栈 %.0f s 内未在 Pictor 注册（FAIL(env)）" % REGISTER_S
            exit_code = 2
            return 2

        sctl = StatusSampler(ctl)
        stage = Stage(ctx, sampler, sctl, sampler.tail)

        # ── S2-A
        t_a = time.time()
        stage.stage_a()
        ctx["dur_A"] = round(time.time() - t_a, 1)

        # ── S2-B
        t_b = time.time()
        stage.stage_b()
        ctx["dur_B"] = round(time.time() - t_b, 1)

        # ── S2-C
        t_c = time.time()
        stage.stage_c()
        ctx["dur_C"] = round(time.time() - t_c, 1)
        ctx["anchors"]["t_run_end"] = time.time()
        exit_code = 1
    except SystemExit:
        raise
    except Exception:
        traceback.print_exc()
        fatal = "运行期异常（见上方 traceback）"
        exit_code = 2
    finally:
        # ── 收尾：ped → pictor → sim → world
        Log("\n── 收尾 ───────────────────────────────────────────")
        ctx["fatal"] = fatal
        if not args.keep:
            for name, p in (("ped", ctx["procs"].get("ped")), ("pictor", pictor),
                            ("sim", sim), ("world", world)):
                if p is not None:
                    try:
                        p.stop()
                    except Exception:
                        traceback.print_exc()
            residual, busy, zombie = wait_clean(15.0)
        else:
            residual, busy, zombie = [], [], 0
        xlocks = sorted(str(p) for p in pathlib.Path("/tmp").glob(".X*-lock"))
        Log("  残留：%d 活进程（另有 %d 个 <defunct>，父退出即回收）%s"
            % (len(residual), zombie, ("：" + residual[0][:100]) if residual else ""))
        Log("  端口 %s：%s" % (list(PORTS), "全空" if not busy else busy))
        Log("  /tmp/.X*-lock：%s" % (xlocks or "无"))
        ctx["residual"] = residual
        ctx["busy_ports"] = busy
        if ctx.get("pictor_kernel_log"):
            try:
                shutil.copy2(ctx["pictor_kernel_log"], out / "pictor_kernel.log")
            except OSError:
                pass

        # ── 日志与取数
        try:
            ctx["sim_text"] = sampler.full_text() if sampler is not None else ""
        except Exception:
            ctx["sim_text"] = ""
        ped_rows = read_ped_csv(out / "ped_trace.csv")
        ctx["placeholders"] = ensure_artifacts(out, ctx)
        recs = parse_sim_text(ctx["sim_text"])
        ctx["clock"] = setup_clock(recs, ped_rows)
        is_env_fail = exit_code == 2
        try:
            if recs and ctx["anchors"].get("t_send_A_wall"):
                evaluate(ctx, recs, ped_rows, ctx["clock"])
            else:
                ctx["criteria"] = [{"n": n, "name": nm, "ok": False, "env": True,
                                    "detail": "无 sim 日志（FAIL(env)），无法判定",
                                    "sub": {}} for n, nm in
                                   ((1, "任务贯通"), (2, "巡逻到 ≥2 角"), (3, "进入跟随"),
                                    (4, "离开回巡逻"), (5, "安全"), (6, "配置生效"), (7, "无异常"))]
        except Exception:
            traceback.print_exc()
            if not ctx["criteria"]:
                ctx["criteria"] = [{"n": n, "name": nm, "ok": False, "env": True,
                                    "detail": "分析异常（见 driver.log），无法判定", "sub": {}}
                                   for n, nm in ((1, "任务贯通"), (2, "巡逻到 ≥2 角"),
                                                 (3, "进入跟随"), (4, "离开回巡逻"),
                                                 (5, "安全"), (6, "配置生效"), (7, "无异常"))]

        # ── P7（b）收尾 + （c）边界 + （a）异常计数
        texts = {"sim": ctx["sim_text"], "world": read_text(out / "world.log"),
                 "pictor": read_text(out / "pictor.log"), "ped": read_text(out / "ped.log")}
        anomalies, samples = count_anomalies(texts)
        p7a = all(v == 0 for v in anomalies.values())
        p7b = (not residual) and (not busy)
        now_frozen = git_frozen_now()
        base_frozen = git_frozen_baseline()
        p7c = now_frozen is not None and base_frozen is not None and \
            sorted(now_frozen) == sorted(base_frozen)
        pictor_now = git_pictor_now() or []
        p7c = p7c and (sorted(pictor_now) == sorted(pictor_snapshot))
        sub7 = {"a": p7a, "b": p7b, "c": p7c, "anomalies": anomalies,
                "residual": residual, "busy_ports": busy,
                "frozen_now": now_frozen, "frozen_base": base_frozen,
                "pictor_now": pictor_now, "pictor_snapshot": pictor_snapshot}
        env7 = is_env_fail and not p7b
        new7 = {"n": 7, "name": "无异常", "ok": p7a and p7b and p7c, "env": env7,
                "detail": "ERROR/panic/失败/Blocked/SCRIPT ERROR/Parse Error = %s；残留 %d 活进程；"
                          "端口 5/5 %s；冻结路径=%s；Pictor status=%s"
                          % ("/".join(str(anomalies[k]) for k in
                                      ("error", "panic", "task_fail", "blocked",
                                       "script_error", "parse_error")),
                             len(residual), "全空" if not busy else "占用 %s" % busy,
                             "基线一致" if p7c else "不一致", "一致" if p7c else "不一致"),
                "sub": sub7}
        crit = [c for c in ctx["criteria"] if c["n"] != 7]
        crit.append(new7)
        crit.sort(key=lambda c: c["n"])
        ctx["criteria"] = crit

        # 关键日志（≤10 行）
        key = []
        for r in recs:
            if r["kind"] in ("start", "build", "fail"):
                key.append(r["raw"])
        for r in recs:
            if r["kind"] == "arrive" and hit_corner((r["x"], r["y"])) is not None:
                key.append(r["raw"])
                if len(key) >= 10:
                    break
        if len(key) < 10:
            for r in recs:
                if r["kind"] == "det" and r["targets"]:
                    key.append(r["raw"])
                    break
        if len(key) < 10 and samples:
            key.append(samples[0])
        ctx["key_lines"] = key[:10]

        # ── 产物
        try:
            ntr = write_trace(out, ctx.get("uav_rows", []), ped_rows,
                              sctl.rows if sctl is not None else [], ctx["clock"])
            nst = write_status_jsonl(out, sctl.rows if sctl is not None else [])
            if sampler is not None and sampler.path is not None and sampler.path.exists():
                shutil.copy2(sampler.path, out / "sim.log")
            # 占位剔除：`ensure_artifacts` 之后已被真实内容覆盖的，不再记进 cases.json.placeholders
            ctx["placeholders"] = refresh_placeholders(out, ctx.get("placeholders") or [])
            ctx["elapsed_s"] = round(time.time() - t_process, 1)
            ok_pre = all(c["ok"] for c in ctx["criteria"]) and not residual and not busy
            if exit_code != 2:
                exit_code = 0 if ok_pre else 1
            ctx["rc"] = exit_code
            ok_all = write_verdict(out, ctx, residual, busy, zombie, len(xlocks))
            if ok_all != ok_pre:
                Log("  [WARN] verdict 的 ok_all(%s) 与预计算(%s) 不一致" % (ok_all, ok_pre))
            write_cases_json(out, ctx, checks, pictor_snapshot)
            Log("\n  产物：verdict.md / sim.log / world.log / ped.log / pictor.log / "
                "trace.csv（%d 行）/ cases.json（+ped_trace.csv %d 行 / status.jsonl %d 行）"
                % (ntr, len(ped_rows), nst))
        except Exception:
            traceback.print_exc()
            exit_code = 2

        for c in ctx["criteria"]:
            if only is not None and ("P%d" % c["n"]) not in only:
                continue
            Log("  [P%d] %-10s %s  %s" % (c["n"], c["name"],
                                         "PASS" if c["ok"] else ("FAIL(env)" if c.get("env") else "FAIL"),
                                         c["detail"]))
        if fatal:
            Log("  [致命] %s" % fatal)
        Log("\nEXIT=%d" % exit_code)
        if args.keep:
            Log("  --keep：进程保持运行（world=%s sim=%s pictor=%s）"
                % (world and world.pid, sim and sim.pid, pictor and pictor.pid))
        try:
            fh.close()
        except Exception:
            pass
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
