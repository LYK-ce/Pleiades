#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Presented by KeJi
# Created Date ： 2026-10-30
# Modified Date ： 2026-10-30
"""task_41 Step 4 — 端到端验证（1 UAV + 4 UGV + Pictor 协同控制，**只验不改**）。

被测链路（Task §六 / §二.9~§二.12）：
  Pictor `containment{type:3}` ⇒ sim 机栈 tree v2（`ReactiveFallback[ ReactiveSequence[InsideSquare,
  follow], 巡逻 ]`）⇒ `InsideSquare` 成功时发 `pleiades/robot/assist_request`（msgid 7，8 B）
  ⇒ Pictor kernel → MessageParser → kernel_bridge → EventBus → `CoopController`：人四周 3 m 环的 4 个
  等角点 + **最近一对一**分配 ⇒ 逐车一帧 `TASK_SET{goto}` ⇒ 4 台 sim UGV 各自 `Goto`。

执行轮次：**只验不改，仅 1 轮**（FAIL 即终止，不重试）。
`--dry-run`（静态前置）与 `--selftest`（纯离线）各可跑一次；真跑只跑 1 次。
崩溃后的 `--reanalyze <同一产物目录>` 只重渲染 `verdict.md`（同一批产物、不重启任何进程、不改判据），
不算重跑。

环境（Task §6.2 + step.md:11）：
  世界 `RobotSim_TestEnv/world_fast/`（源树整树副本 + `override.cfg` 的 `msaa_3d=0`）+ `xvfb-run -a`
  + `--print-fps`（世界 fps 闸门）；1×UAV `simq_uav_containment`（沿用 Step 2：ctl 9122 / net 9125 /
  `vel_fwd=0.8` / `yaw_rate_deg=30` / `model=yolov8s.safetensors`）；
  4×UGV `simq_ugv_c1..c4`（ctl 9131..9134 / net 9135..9138 / `chassis=diff_drive_ugv` /
  `chassis_params.max_speed=2.0` / `forward_speed=100` / `turn_speed=35` / 一个 `lidar_front`）；
  Pictor：**项目镜像** `RobotSim_TestEnv/pictor_ws/` 作 `--path`（godot 强制 CWD = `--path` ⇒ kernel 读
  镜像的 `.config`，其中 `[Network].subscribe_topics` 含 `pleiades/robot/assist_request`）；
  行人 `Tool/sim_task41_pedestrian.py`（`python3 -u`，0.5 m/s，4 段路点 + 驻留）。

用例（Task §6.3 + step.md:18；总 ~600 s）：
  S4-A 无人 30 s 巡逻（4 车静止、不发任何车命令）｜S4-A2 4 车经各自 control 口 `goto` 到四角
  (52,52)/(76,52)/(76,76)/(52,76)，等 ≥15 s 且全部停稳（上限 90 s）后再放人｜S4-B 行人
  (64,80)→(64,71) 停 **120 s**（等 UAV 巡逻贴近 ≤5 m、person 被采纳）→(64,64) 停 20 s
  （触发 assist → 4 车去 3 m 环）｜S4-C 行人 (64,64)→(66,64) 停 10 s（重分配、无同点）
  ｜S4-D 行人走向 (64,50) 出界（UAV 回巡逻、4 车停在最后目标点）。

为什么先摆位（step.md:18）：4 车出生被世界排在 (64,64) 的 2 m 环上，正好压在行人 x=64 的路线与
  3 m 环的点位上（上一轮 `leg1` 被 `simq_ugv_c3` 卡死在 (64.0000,66.3751) ⇒ `t_out` 不存在 ⇒
  Q7/P4 无法判定）。摆到四角后：① 人不再被车挡；② 3 m 环的四点在 40 s 内可达。

判据（Q1~Q8，Task §6.4；阈值一个不改）：
  Q1 链路贯通｜Q2 点位正确（3.0±0.05 m / 距人 3.0±1.0 m / 四点不重复）｜Q3 到达（40 s、≤1.0 m、
  车间距 ≥1.0 m）｜Q4 四象限覆盖｜Q5 分配稳定（序号变化 ≤2、无同点）｜Q6 车运动（≤2.05 m/s、无
  Blocked、急停段 ≤1/车、不越界）｜Q7 UAV 回归（P1~P4；P3 位移 ≥**2.5 m**，人类裁决 §4.4 末）｜
  Q8 无异常 + 收尾 + 边界。

用法：
    python3 Tool/sim_task41_coop_test.py --selftest              # 纯离线（合成数据跑通 evaluate+产物）
    python3 Tool/sim_task41_coop_test.py --dry-run               # 静态前置（含镜像 --import/--quit 冒烟）
    python3 Tool/sim_task41_coop_test.py --max-s 1500            # **唯一一次**真跑
    python3 Tool/sim_task41_coop_test.py --reanalyze DIR         # 同批产物离线重渲染 verdict.md
退出码：0 = Q1~Q8 全 PASS；1 = 有 FAIL；2 = 环境/前置失败（FAIL(env)）

产物：`RobotSim_TestEnv/results/task41_s4_<UTC>/`
    verdict.md / sim_uav.log / sim_ugv_c1..c4.log / pictor.log / ped.log / world.log / trace.csv /
    cases.json（+ ped_trace.csv / driver.log / pictor_kernel.log / ctl_A.json / ctl_vehicles.json /
    configs/）

--------------------------------------------------------------- 口径偏差登记（D1~D17）
D1 Task §6.5 允许「扩展 `sim_task41_containment_test.py` 或新 `sim_task41_coop_test.py`」⇒ 新建本脚本，
   只 import 复用（`sim_queue_test` / `sim_task40_ugv_test` / `sim_task41_containment_test`）。Step 2 的
   判据是 P1~P7、Step 4 是 Q1~Q8（口径不同，尤其 Q7 的 2.5 m）⇒ **不 import `S2C.main`/`S2C.evaluate`**。
D2 `godot --path <P>` 会把进程 CWD 强制切到 `<P>`（本步探针：`tmp/task41/cwdprobe/`）⇒ Pictor 的
   kernel 配置只能落在 `--path` 目录里；`Pictor/` 不在本步允许路径内 ⇒ 用**项目镜像**
   `RobotSim_TestEnv/pictor_ws/`（R1）。镜像每次真跑前重新同步（`diff -r` 闸门）。
D3 镜像同步在 plan 的 4 个排除项之外**多排除 `.git`**：Pictor 是独立仓，git 元数据对运行无用，
   且 40 MB 里大头是 `.git`；`diff -r`（同样排除 `.git`）与关键文件 sha 仍是闸门。
D4 行人停点 `(64,71)` vs 文档 `(64,72)`（沿用 Step 2 D4）：`(64,72)` 恰在正方形边界线上
   （`|y−64| = 8.00`），`--arrive-tol 0.30` 会让它停在界外 ⇒ 整条跟随/协助链路失效。Task 原文即
   「南边界**内**」。`cases.json` 同时记 `doc_stops` 与 `stops`。
D5 急停行文本按**代码**取 `前方障碍`（`pleiades-ugv/src/ugv/emergency_stop.rs:47` 打印
   `[Robot] 前方障碍 …m < …m，急停`），不是 plan 里的 `[Sim] 前方障碍`（该串在代码里不存在）。
D6 `--print-fps` 的 `Project FPS: N` 行**没有时间戳** ⇒ 世界 fps 取全程样本中位（无法精确切窗），
   逐样本值入 `cases.json.fps`。
D7 `[Coop]` 行没有时间戳（GDScript `print`）⇒ 事件时刻 = 读文件时的墙钟（100 ms 泵），再
   `t_world = a + k·t_wall`（`S2C.setup_clock` 的 detector `frame_ts` 标定）；交叉校验用 4 车
   `[Robot] 开始任务 … Goto {x,y}` 的 tracing 时间戳中位，差 >1.0 s 时改用车侧时间并标 `time_src=car`（M1）。
D8 Q1d 的「坐标 2 位小数相等」用 `|Δ| ≤ 0.01 m` 判（与 2 位小数相等等价，容忍 f32/`%.2f` 的舍入）。
D9 Q2c 的下界 = `3.0 − 1.0 = 2.0 m`（Task §6.4 原文「距人真实位置 3.0 ± 1.0 m」）。
D10 Q8a 的异常计数覆盖 8 份文本（UAV + 4×UGV + pictor + ped + world）；plan 的「7 份」少算
   `world.log`，本脚本把世界也计入（更严，不放松判据）。
D11 `[Sim] UGV` 行是 **5 Hz**（`pleiades-sim/src/sim/sim_handler.rs` 的 `Should_Log("sim.ugv.exec",200)`）
   ⇒ Q3b 的车间距按 100 ms 网格线性插值后取最小，Q6a 的逐段速度只用 `Δt ∈ [0.15,0.6] s` 的相邻行。
D12 行人 `leg` 从 **0** 起、只有 leg0/1/2 有 `phase=dwell` 行；S4-D 的 leg3 无 dwell ⇒ 不入 Q4；
   `t_leg*_arrive` 取该 leg 首条 `dwell` 行的 `world_time`。
D13 Q8b 的解析率按「判据用到的行」算：`[Nav]` 的 Blocked/急停行进独立计数器（`拍无路`/`前方障碍`），
    不进 `parse_line` 的记录流（Step 4 第 1 次真跑已登记，本轮保留）。
D14 Q4 的边界样本（|dx| 或 |dy| < 0.05 m）同时计入相邻两象限；3 m 环的 4 点落在世界轴上，
    单值硬判会把两个点并进同一象限（保留）。
D15 **已撤销**：Q8b 恢复定稿口径 —— `[Sim]`/`[Nav]`/`[Coop]` 三族**各 ≥1 行**且解析率 100%
    （上一轮「`[Coop]` 零样本时不硬判」是事后放宽，违反 step.md:19，本轮删掉）。
D16 UAV 实例 `[Log] level="debug"`（**只改实例配置**，不改源码/判据/阈值）：为了看见
    `command_handler.rs:59,62` 的 `gossipsub 发布成功/失败`（info 级不可见，上一轮 Q1 因此无法定位）。
    证据只进 verdict 的「协助链路断点证据」段，**不参与任何判据的 ok 计算**。
D17 4 车先摆位到四角（step.md:18）⇒ Q4 的「四象限」在「车根本没动」时也可能成立（四角天然分布
    在人的四个象限）。**故 Q4 单独不能证明协同链路**；链路结论以 Q1/Q2/Q3/Q5 为准，Q4 只判覆盖。
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

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "Tool"))
import sim_queue_test as Q                     # noqa: E402  （Proc / Control / LogTail / port_busy / sha256_file）
import sim_task40_ugv_test as T40              # noqa: E402  （world_argv / ensure_world_fast / residual_all / run_capture）
import sim_task41_containment_test as S2C      # noqa: E402  （LogSampler/parse_line/ped CSV/clock/Stage.wait_release）

# ================================================================ §2 常量

WORLD_FAST = ROOT / "RobotSim_TestEnv" / "world_fast"
PICTOR = ROOT / "Pictor"
PICTOR_WS = ROOT / "RobotSim_TestEnv" / "pictor_ws"
PICTOR_CFG = PICTOR / ".config" / "config.toml"          # 只读校验对象（不改 Pictor/）
PICTOR_WS_CFG = PICTOR_WS / ".config" / "config.toml"    # 镜像里我们自己写的那份
PICTOR_WS_LOG = PICTOR_WS / "Log"                        # kernel 的日志目录（CWD = 镜像）
CTL = PICTOR / "tool" / "pictor_ws_ctl.py"
PED_TOOL = ROOT / "Tool" / "sim_task41_pedestrian.py"
RELEASE_BIN = ROOT / "target" / "release" / "pleiades-sim"
RESULTS = ROOT / "RobotSim_TestEnv" / "results"
GIT_BASELINE = ROOT / "tmp" / "task41" / "git_baseline.txt"
YOLO_SRC = ROOT / "Pleiades_Workspace" / "yolov8s.safetensors"
KERNEL_SO = PICTOR / "bin" / "libpleiades_terminal.so"
# `Pictor/.config/config.toml` 的进场 sha256（Step 2 起固定；本步**只读**校验，任何变化即 FAIL(env)）
PICTOR_CFG_SHA = "82c95ea70194e13e9175c3a766886838076cf8c61d7b37e385da8beb20451cf3"

WORLD_PORT = 9999
WS_PORT = 9100
PIC_NET_PORT = 9123
PORTS = (9999, 9100, 9122, 9123, 9125, 9131, 9132, 9133, 9134, 9135, 9136, 9137, 9138)

PEER_UAV = "simq_uav_containment"
PEERS_UGV = ("simq_ugv_c1", "simq_ugv_c2", "simq_ugv_c3", "simq_ugv_c4")
UGV_CTL = {"simq_ugv_c1": 9131, "simq_ugv_c2": 9132,
           "simq_ugv_c3": 9133, "simq_ugv_c4": 9134}
UGV_NET = {"simq_ugv_c1": 9135, "simq_ugv_c2": 9136,
           "simq_ugv_c3": 9137, "simq_ugv_c4": 9138}
ALL_PEERS = (PEER_UAV,) + PEERS_UGV
PEER_PED = "s4_ped"

CX, CY, HALF = 64.0, 64.0, 8.0
CORNERS = S2C.CORNERS
CORNER_GRID = S2C.CORNER_GRID
BOUND_M = 128.0

# 用例时长（Task §6.3）
A_OBSERVE_S = 30.0             # S4-A 无行人巡逻
RELEASE_WAIT_S = 120.0         # S2C 相位锁放人等待上限（复用 Step 2 的 D3）
OBSERVE_CAP_S = 340.0          # S4-B/C/D 连续观测总上限（new：release→spawn 5 + 220 走/停 + 25 settle）
P4_SETTLE_S = 25.0             # 人出界后再观测（P4 窗 + 余量，plan §4.5）

# S4-A2 起步摆位（step.md:18）：4 车 goto 四角，≥15 s 停稳后再放人
CAR_STAGE = {                       # 车 → 四角（世界系，米；c1..c4 与 PEERS_UGV 同序）
    "simq_ugv_c1": (52.0, 52.0), "simq_ugv_c2": (76.0, 52.0),
    "simq_ugv_c3": (76.0, 76.0), "simq_ugv_c4": (52.0, 76.0),
}
STAGE_MIN_S = 15.0                  # step.md:18「等 15 s 停稳」（下限，不得更短）
STAGE_MAX_S = 90.0                  # 上限：超时未停稳 ⇒ FAIL(env)
STAGE_TOL_M = 1.5                   # 「到点」判据（Goto 到点 + 车体尺寸余量；不是判据阈值）
STAGE_CONNECT_S = 20.0              # 4 个 control 口的连接窗口

# 行人（D4：停点 (64,71)）
PED_SPAWN = (64.0, 80.0)
PED_LEGS = "64,71@120;64,64@20;66,64@10;64,50@0"
PED_STOPS = [(64.0, 71.0), (64.0, 64.0), (66.0, 64.0), (64.0, 50.0)]
PED_DOC_STOPS = [(64.0, 72.0), (64.0, 64.0), (66.0, 64.0), (64.0, 50.0)]
PED_SPEED = 0.5
PED_ARRIVE_TOL = 0.30
PED_DURATION_S = 420.0

# 判据阈值（Task §6.4，一个不改）
RING_R = 3.0                   # 3 m 环（§二.10）
RING_TOL = 0.05                # Q2b：3.0 ± 0.05 m
PERSON_TOL = 1.0               # Q2c：距人 3.0 ± 1.0 m（D9）
PERSON_TOL_FLOOR = RING_R - PERSON_TOL
ARRIVE_S = 40.0                # Q3a 窗口
ARRIVE_TOL_M = 1.0             # Q3a
CAR_GAP_M = 1.0                # Q3b / Q5b
IDX_CHANGE_MAX = 2             # Q5a
CAR_V_MAX = 2.05               # Q6a
ESTOP_MAX = 1                  # Q6c
ESTOP_GAP_S = 2.0              # 急停分段（同段重复是 1 s 限流造成的）
COORD_EQ_M = 0.01              # D8：2 位小数相等的容差
Q7_DISP_MIN_M = 2.5            # Q7/P3b：**人类裁决 2.5 m**（不是 S2C.DISP_MIN_M=3.0）
Q7 = dict(CORNER_TOL=S2C.CORNER_TOL_M, SWITCH_S=S2C.SWITCH_S, P3_SLACK=S2C.DIST_SLACK_M,
          P4_S=S2C.P4_S, P4_MIN=S2C.P4_DISP_MIN_M, P2_MIN=S2C.P2_CORNERS_MIN)
WORLD_FPS_MIN = 5.0            # step.md:12 前置
POSE_AGE_MAX_MS = 200          # step.md:12 前置（UAV `位姿龄` 中位）
DET_MIN_HZ = 1.0               # 机栈 detector 帧率下限（§二.14 的 CPU 口径）
# D16 的证据串（`pleiades-base/src/Network/command_handler.rs:59,62`、
# `swarm_events.rs:384`）；只在 UAV 实例 `[Log] level="debug"` 时出现
DIAG_PAT = {
    "pub_ok_assist": "gossipsub 发布成功: topic=pleiades/robot/assist_request",
    "pub_err_assist": "gossipsub 发布失败 (pleiades/robot/assist_request)",
    "sub_assist": "节点订阅 gossipsub topic: pleiades/robot/assist_request",
    "pub_ok_pose": "gossipsub 发布成功: topic=pleiades/robot/pose",
    "pub_err_pose": "gossipsub 发布失败 (pleiades/robot/pose)",
}
UAV_LOG_LEVEL = "debug"             # D16：只改 UAV 实例配置，用于区分「发布侧失败」与「地面站丢弃」
BOUNDARY_TOL_M = 0.05         # Q4：|dx|/|dy| < 0.05 m 的样本记边界，同时计入相邻两象限
R3_STALL_M = 0.05              # R3：连续 5 s 位移 < 0.05 m
R3_STALL_S = 5.0
R3_CAR_NEAR_M = 0.8

# 闸门超时（plan §4.6）
PICTOR_TIMEOUT_S = 2100
WORLD_LISTEN_S = 90.0
SPAWN_S = 60.0
ASSEMBLED_S = 120.0
DETECTOR_S = 120.0
CAMERA_S = 60.0
PICTOR_LISTEN_S = 40.0
PICTOR_KERNEL_S = 30.0
REGISTER_S = 90.0
CARS_TYPED_S = 60.0
MISSION_TIMEOUT_S = 120
SAMPLE_DT = Q.SAMPLE_DT        # 200 ms
PUMP_DT = 0.10                 # 泵周期（plan R10）
TRACE_DT = 0.10                # trace.csv 网格

GOLDEN_HEX = "0103428000004280000041000000"
GOLDEN_LEN = 14
GOLDEN_TYPE = 3

# ================================================================ §3 配置模板


def inst_dir(peer):
    return ROOT / "RobotSim_TestEnv" / peer


def instance_dirs(peer):
    base = inst_dir(peer)
    for sub in (".config", "Log", "Pleiades_Workspace"):
        (base / sub).mkdir(parents=True, exist_ok=True)
    return base


FE = "# Presented by KeJi\n# Created Date ： 2026-10-30\n# Modified Date ： 2026-10-30\n"


def ugv_config_text(peer):
    """4 份 UGV 实例配置（Task §6.2 + §二.12；plan §2.3）——只有端口/peer_name 不同。"""
    return (
        "# Presented by KeJi\n# Created Date ： 2026-10-30\n# Modified Date ： 2026-10-30\n#\n"
        "# task_41 Step 4：sim UGV（c1..c4 之一），由 Tool/sim_task41_coop_test.py 幂等生成\n"
        "[Log]\n"
        'level = "info"\n'
        'log_file_path = "Log/"\n'
        "\n[Network]\n"
        "LAN = true\n"
        "WAN = false\n"
        'Transport_Protocol = "TCP"\n'
        "listen_port = %(net_port)d\n"
        'dht_namespace = "pleiades-nodes"\n'
        "bootstrap_peers = []\n"
        "subscribe_topics = []             # §二：车不订阅 assist_request\n"
        "\n[Storage]\n"
        'workspace_dir = "Pleiades_Workspace"\n'
        "quota_gb = 0\n"
        'kvcache_dir = ".kvcache"\n'
        "\n[Identity]\n"
        'peer_name = "%(peer)s"\n'
        'node_type = "car"\n'
        "\n[sim_world]\n"
        "enabled = true\n"
        'url = "ws://127.0.0.1:%(world_port)d"\n'
        'chassis = "diff_drive_ugv"\n'
        "reconnect_ms_min = 200\n"
        "reconnect_ms_max = 3000\n"
        "handshake_timeout_ms = 8000\n"
        "\n# §二.12：车速 2 m/s（世界里 max_speed × 档位 100%%）；turn_speed=35 ⇒ 31.5 °/s\n"
        "[sim_world.chassis_params]\n"
        "max_speed = 2.0\n"
        "\n[[sim_world.sensor]]\n"
        'slot = "lidar_front"\n'
        'sensor = "lidar_2d"\n'
        "\n[sim_world.sensor.mount]\n"
        "x = 0.1\n"
        "y = 0.0\n"
        "z = 0.2\n"
        "yaw = 0.0\n"
        "\n[sim_world.sensor.params]\n"
        "range = 12.0\n"
        "rays = 360\n"
        "rate_hz = 10\n"
        "\n[chassis]\n"
        "enabled = true\n"
        "forward_speed = 100\n"
        "turn_speed = 35\n"
        "\n[control]\n"
        "enabled = true\n"
        'bind = "127.0.0.1"\n'
        "port = %(ctl_port)d\n"
    ) % {"net_port": UGV_NET[peer], "peer": peer, "world_port": WORLD_PORT,
         "ctl_port": UGV_CTL[peer]}


def pictor_config_text():
    """Pictor 内核配置（写进**镜像**；`Pictor/.config` 只读校验）。plan §2.4。"""
    return (
        "# Presented by KeJi\n# Created Date ： 2026-10-30\n# Modified Date ： 2026-10-30\n#\n"
        "# task_41 Step 4：Pictor 内核配置（Pictor 项目镜像 → CWD = RobotSim_TestEnv/pictor_ws）\n"
        "[Log]\n"
        'level = "info"\n'
        'log_file_path = "Log/"\n'
        "\n[Network]\n"
        "LAN = true\n"
        "WAN = false\n"
        'Transport_Protocol = "TCP"\n'
        "listen_port = %(net)d\n"
        'dht_namespace = "pleiades-nodes"\n'
        "bootstrap_peers = []\n"
        "# §六：地面站是唯一订阅 assist_request 的节点\n"
        'subscribe_topics = ["pleiades/robot/assist_request"]\n'
        "\n[Storage]\n"
        'workspace_dir = "Pleiades_Workspace"\n'
        "quota_gb = 0\n"
        'kvcache_dir = ".kvcache"\n'
        "\n[Identity]\n"
        'peer_name = "pictor_s4"\n'
        'node_type = "ground_station"\n'
    ) % {"net": PIC_NET_PORT}


# ================================================================ §4 小工具

LOG_FH = [None]


def Log(msg=""):
    print(msg, flush=True)
    fh = LOG_FH[0]
    if fh is not None:
        try:
            fh.write(str(msg) + "\n")
        except (OSError, ValueError):
            pass


def utc_stamp():
    return time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())


def utc_iso():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def read_text(path):
    return S2C.read_text(path)


def write_json(path, obj):
    pathlib.Path(path).write_text(json.dumps(obj, ensure_ascii=False, indent=2), encoding="utf-8")


def fmt(v, n=2):
    return S2C.fmt(v, n)


def dist(a, b):
    return S2C.dist(a, b)


def median(xs):
    return S2C.median(xs)


def percentile(xs, p):
    return S2C.percentile(xs, p)


def outside_square(xy):
    return (xy[0] < CX - HALF or xy[0] > CX + HALF
            or xy[1] < CY - HALF or xy[1] > CY + HALF)


def parse_any(line):
    """一行 sim 日志 → 记录（UGV 位姿 / `开始任务 Goto` 优先，其余回落 `S2C.parse_line`）。"""
    ln = line.rstrip("\n")
    m = CAR_RE.match(ln)
    if m:
        ts = S2C.log_epoch(m.group("ts"))
        if ts is None:
            return None
        gx = int(m.group("gx")) if m.group("gx") is not None else None
        gy = int(m.group("gy")) if m.group("gy") is not None else None
        return {"kind": "ugv", "t": ts, "raw": ln, "x": float(m.group("x")), "y": float(m.group("y")),
                "yaw": float(m.group("yaw")), "gx": gx, "gy": gy,
                "speed": int(m.group("speed")), "turn": int(m.group("turn")),
                "age": int(m.group("age"))}
    m = START_GOTO.search(ln)
    if m:
        ts = S2C.log_epoch(m.group("ts"))
        if ts is None:
            return None
        return {"kind": "goto_start", "t": ts, "raw": ln,
                "x": float(m.group("x")), "y": float(m.group("y"))}
    return S2C.parse_line(line)


def parse_text(text):
    return [r for r in (parse_any(ln) for ln in text.splitlines()) if r is not None]


def interp(series, t):
    """`[(t, x, y)]` 按 t 线性插值（越界取端点；空 → None）。"""
    if not series:
        return None
    if t <= series[0][0]:
        return (series[0][1], series[0][2])
    if t >= series[-1][0]:
        return (series[-1][1], series[-1][2])
    lo, hi = 0, len(series) - 1
    while hi - lo > 1:
        mid = (lo + hi) // 2
        if series[mid][0] <= t:
            lo = mid
        else:
            hi = mid
    t0, x0, y0 = series[lo]
    t1, x1, y1 = series[hi]
    if t1 == t0:
        return (x1, y1)
    k = (t - t0) / (t1 - t0)
    return (x0 + k * (x1 - x0), y0 + k * (y1 - y0))


def git_frozen_now():
    rc, so, _ = T40.run_capture(["git", "status", "--porcelain", "-uall"], timeout=60)
    if rc != 0:
        return None
    return [ln.strip() for ln in so.splitlines()
            if any(p in ln for p in ("Task/", ".github/", "Cargo.toml", "Cargo.lock"))]


def git_frozen_baseline():
    if not GIT_BASELINE.is_file():
        return None
    return [ln.strip() for ln in read_text(GIT_BASELINE).splitlines()
            if any(p in ln for p in ("Task/", ".github/", "Cargo.toml", "Cargo.lock")) and ln.strip()]


def git_pictor_now():
    rc, so, _ = T40.run_capture(["git", "-C", str(PICTOR), "status", "--porcelain", "-uall"],
                                timeout=60)
    return None if rc != 0 else [ln.strip() for ln in so.splitlines() if ln.strip()]


# ================================================================ §4.5 正则（全部“照日志原样”）

CAR_RE = T40.EXEC_RE                                        # `[Sim] UGV 位置=(x,y) yaw=…° next=… → speed=… turn=… 位姿龄=…ms`
START_GOTO = re.compile(r"(?P<ts>\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6})Z.*"
                        r"\[Robot\] 开始任务 .*Goto \{ x: (?P<x>-?\d+(?:\.\d+)?), "
                        r"y: (?P<y>-?\d+(?:\.\d+)?) \}")
COOP_REQ = re.compile(r"\[Coop\] 协助请求 requester=(?P<vid>[^ ]+) "
                      r"目标=\((?P<x>-?\d+\.\d+),(?P<y>-?\d+\.\d+)\) 车数=(?P<n>\d+)")
COOP_NOCAR = re.compile(r"\[Coop\] 协助请求 requester=(?P<vid>[^ ]+) "
                        r"目标=\((?P<x>-?\d+\.\d+),(?P<y>-?\d+\.\d+)\) 无在线车")
COOP_ASG = re.compile(r"\[Coop\] 指派 车=(?P<car>[^ ]+) 点#(?P<idx>\d+) "
                      r"目标=\((?P<x>-?\d+\.\d+),(?P<y>-?\d+\.\d+)\) 当前距=(?P<d>\d+\.\d+) m")
BLOCKED_RE = re.compile(r"\[Nav\] 连续 \d+ 拍无路 → Blocked")
ESTOP_PAT = "前方障碍"                                       # D5
FPS_RE = re.compile(r"Project FPS:\s*(?P<f>\d+(?:\.\d+)?)")
UNKNOWN_PATS = ("unknown msgid", "parse_orion_frame failed")
# ================================================================ §5 实例 / 镜像 / 二进制

MIRROR_IGNORE = (".config", "Log", ".godot", "Pleiades_Workspace", ".git")   # D3
MIRROR_DIFF_EXCLUDES = (".config", "Log", ".godot", "Pleiades_Workspace", ".git")
MIRROR_KEY_FILES = ("src/cooperation/coop_controller.gd", "src/main/main.tscn")


def ensure_dirs_and_configs(out=None):
    """建 4 个 UGV 实例目录 + 幂等写模板（`out/configs/ugv_c*.toml` 留档）。返回 {peer: cfg 路径}。"""
    cfgs = {}
    for peer in PEERS_UGV:
        base = instance_dirs(peer)
        cfg = base / ".config" / "config.toml"
        text = ugv_config_text(peer)
        cfg.write_text(text, encoding="utf-8")
        cfgs[peer] = cfg
        if out is not None:
            (out / "configs").mkdir(parents=True, exist_ok=True)
            (out / "configs" / ("ugv_%s.toml" % peer.split("_")[-1])).write_text(text, encoding="utf-8")
    return cfgs


def ugv_config_readback(cfg, peer):
    """逐键回读（plan §2.3 的 11 项）；返回 (ok, bad_list)。"""
    text = read_text(cfg)
    want = {
        "peer_name": '"%s"' % peer,
        "node_type": '"car"',
        "listen_port": str(UGV_NET[peer]),
        "port": str(UGV_CTL[peer]),
        "chassis": '"diff_drive_ugv"',
        "url": '"ws://127.0.0.1:%d"' % WORLD_PORT,
        "max_speed": "2.0",
        "forward_speed": "100",
        "turn_speed": "35",
        "enabled": "true",
        "slot": '"lidar_front"',
    }
    flat = {}
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#") or "=" not in s:
            continue
        k, _, v = s.partition("=")
        flat.setdefault(k.strip(), v.split("#")[0].strip())
    bad = ["%s=%r≠%r" % (k, flat.get(k), v) for k, v in want.items() if flat.get(k) != v]
    return (not bad), bad


def write_pictor_config(out=None):
    """幂等写镜像里的 kernel 配置（D2）。返回 config 路径。"""
    (PICTOR_WS / ".config").mkdir(parents=True, exist_ok=True)
    (PICTOR_WS / "Log").mkdir(parents=True, exist_ok=True)
    (PICTOR_WS / "Pleiades_Workspace").mkdir(parents=True, exist_ok=True)
    text = pictor_config_text()
    PICTOR_WS_CFG.write_text(text, encoding="utf-8")
    if out is not None:
        (out / "configs").mkdir(parents=True, exist_ok=True)
        (out / "configs" / "pictor_config.toml").write_text(text, encoding="utf-8")
    return PICTOR_WS_CFG


def pictor_config_readback(cfg):
    """镜像 config 的 4 键回读：9123 / pictor_s4 / ground_station / subscribe 含 assist。"""
    text = read_text(cfg)
    flat = {}
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#") or "=" not in s:
            continue
        k, _, v = s.partition("=")
        flat.setdefault(k.strip(), v.split("#")[0].strip())
    want = {"listen_port": str(PIC_NET_PORT), "peer_name": '"pictor_s4"',
            "node_type": '"ground_station"'}
    bad = ["%s=%r≠%r" % (k, flat.get(k), v) for k, v in want.items() if flat.get(k) != v]
    sub = flat.get("subscribe_topics", "")
    if "pleiades/robot/assist_request" not in sub:
        bad.append("subscribe_topics=%r 不含 assist_request" % sub)
    return (not bad), bad


def force_uav_debug(cfg, out=None):
    """把 UAV 实例 `[Log] level` 改成 `debug`（**只改实例配置**，D16）。

    依据：`pleiades-base/src/bootstrap.rs:107-108` —— `EnvFilter::try_from_default_env()` 优先，
    没有 `RUST_LOG` 时才用 config 的 `[Log] level`。故 `run_real` 开头会 `os.environ.pop("RUST_LOG")`，
    保证这条配置真的生效；改完回读确认，并把最终文件覆盖到 `out/configs/uav_config.toml` 留档。
    """
    if os.environ.get("RUST_LOG"):
        return False, "环境里有 RUST_LOG=%r（会覆盖 [Log] level）" % os.environ["RUST_LOG"]
    text = read_text(cfg)
    lines = text.splitlines()
    idx = next((i for i, ln in enumerate(lines) if ln.strip() == 'level = "info"'), None)
    if idx is None:
        return False, "UAV config 里找不到 `level = \"info\"` 行"
    lines[idx] = 'level = "%s"' % UAV_LOG_LEVEL
    text = "\n".join(lines) + "\n"
    cfg.write_text(text, encoding="utf-8")
    if out is not None:
        (out / "configs").mkdir(parents=True, exist_ok=True)
        (out / "configs" / "uav_config.toml").write_text(text, encoding="utf-8")
    ok = ('level = "%s"' % UAV_LOG_LEVEL) in read_text(cfg)
    return ok, ('[Log] level="%s"（D16：用来看 command_handler.rs:59,62 的发布结果）'
                % UAV_LOG_LEVEL if ok else "写回后仍未生效")


def mirror_diff():
    """`diff -r`（排除运行期目录）→ (ok, 差异行列表)。"""
    argv = ["diff", "-r"]
    for pat in MIRROR_DIFF_EXCLUDES:
        argv += ["--exclude", pat]
    argv += [str(PICTOR), str(PICTOR_WS)]
    rc, so, se = T40.run_capture(argv, timeout=180)
    lines = [ln for ln in (so + se).splitlines() if ln.strip()]
    return (rc == 0 and not lines), lines


def mirror_shas():
    """关键文件 sha256（Pictor 源 vs 镜像）。"""
    out = {}
    for rel in MIRROR_KEY_FILES:
        a = PICTOR / rel
        b = PICTOR_WS / rel
        out[rel] = {"src": Q.sha256_file(a) if a.is_file() else None,
                    "mirror": Q.sha256_file(b) if b.is_file() else None}
    return out


def sync_pictor_mirror(out=None):
    """整树重同步 Pictor → `RobotSim_TestEnv/pictor_ws/`（D2/D3）+ 冒烟。

    顺序固定（plan §2.4）：copytree（排除运行期目录）→ `--import` → 写 config → `--quit` 冒烟
    → 再写一次 config → `diff -r` 闸门。返回 dict（`ok` / `diff` / `shas` / 冒烟证据）。
    """
    res = {"ok": False, "detail": "", "diff": [], "shas": {}, "import_rc": None,
           "quit_rc": None, "quit_ready": False, "quit_script_errors": 0}
    if PICTOR_WS.parent != (ROOT / "RobotSim_TestEnv"):
        res["detail"] = "镜像路径异常：%s" % PICTOR_WS
        return res
    if PICTOR_WS.exists():
        shutil.rmtree(PICTOR_WS)
    shutil.copytree(PICTOR, PICTOR_WS, ignore=shutil.ignore_patterns(*MIRROR_IGNORE))
    rc, so, se = T40.run_capture(["godot", "--headless", "--path", str(PICTOR_WS), "--import"],
                                 timeout=900)
    res["import_rc"] = rc
    if rc != 0:
        res["detail"] = "镜像 --import rc=%d：%s" % (rc, (se or so)[-300:])
        return res
    write_pictor_config(out)
    rc, so, se = T40.run_capture(["timeout", "180", "godot", "--headless", "--path",
                                  str(PICTOR_WS), "--quit"], timeout=240)
    res["quit_rc"] = rc
    txt = (so or "") + (se or "")
    res["quit_ready"] = "[Main] ready: 10 children" in txt
    res["quit_script_errors"] = sum(1 for ln in txt.splitlines() if "SCRIPT ERROR" in ln)
    write_pictor_config(out)
    ok_diff, diff = mirror_diff()
    res["diff"] = diff
    res["shas"] = mirror_shas()
    shas_ok = all(v["src"] is not None and v["src"] == v["mirror"] for v in res["shas"].values())
    ok = ok_diff and shas_ok and res["quit_rc"] == 0 and res["quit_ready"] \
        and res["quit_script_errors"] == 0
    res["ok"] = ok
    res["detail"] = ("镜像同步：diff 空 + %d 个关键文件 sha 相同 + `--import` rc=%d + `--quit` ready=%s"
                     % (len(res["shas"]), res["import_rc"], res["quit_ready"])
                     if ok else "diff %d 行 / shas_ok=%s / quit rc=%s ready=%s SCRIPT ERROR=%d"
                     % (len(diff), shas_ok, res["quit_rc"], res["quit_ready"],
                        res["quit_script_errors"]))
    return res


def ensure_binary_all():
    """`target/release/pleiades-sim` → 5 个实例（UAV + 4 UGV），sha 全等是闸门。返回 (ok, shas, detail)。"""
    if not RELEASE_BIN.is_file():
        return False, {}, "缺少 %s（先 cargo build -p pleiades-sim --release）" % RELEASE_BIN
    want = Q.sha256_file(RELEASE_BIN)
    shas = {}
    for peer in ALL_PEERS:
        base = instance_dirs(peer)
        dst = base / "pleiades-sim"
        need = (not dst.exists()) or Q.sha256_file(dst) != want
        if need:
            shutil.copy2(RELEASE_BIN, dst)
            dst.chmod(0o755)
        shas[peer] = Q.sha256_file(dst)
    ok = all(v == want for v in shas.values())
    return ok, shas, ("5 个实例 sha256 == release %s" % want[:16] if ok else
                      "sha 不一致：%s" % {k: v[:12] for k, v in shas.items()})


def ensure_yolo(out=None):
    """UAV 实例的 detector 权重（复用 `S2C.ensure_yolo`；4 个 UGV 实例只建空 workspace）。"""
    for peer in PEERS_UGV:
        (inst_dir(peer) / "Pleiades_Workspace").mkdir(parents=True, exist_ok=True)
    return S2C.ensure_yolo()


# ================================================================ §6 进程


def start_proc(name, argv, cwd, log_path):
    proc = Q.Proc(name)
    proc.start(argv, cwd, log_path)
    return proc


def wait_text(path, needle, timeout_s):
    return S2C.wait_text(path, needle, timeout_s)


def wait_clean(timeout=20.0):
    """等收尾干净：0 活残留 + 13 端口全空。返回 (活残留, 忙端口, 僵尸数)。"""
    t0 = time.time()
    res, busy, zombie = [], [], 0
    while time.time() - t0 < timeout:
        allres = T40.residual_all()
        zombie = len([x for x in allres if "<defunct>" in x])
        res = [x for x in allres if "<defunct>" not in x]
        busy = [p for p in PORTS if Q.port_busy(p)]
        if not res and not busy:
            break
        time.sleep(0.4)
    allres = T40.residual_all()
    zombie = len([x for x in allres if "<defunct>" in x])
    res = [x for x in allres if "<defunct>" not in x]
    busy = [p for p in PORTS if Q.port_busy(p)]
    return res, busy, zombie


# ================================================================ §7 前置（17 条）

UVG_CHECK_KEYS = ("peer_name", "node_type", "listen_port", "port", "chassis", "url",
                  "max_speed", "forward_speed", "turn_speed", "enabled", "slot")


def preflight(out=None):
    """静态前置 18 条（`--dry-run` 与真跑同一套）。返回 checks 列表。"""
    checks = []

    def add(n, name, ok, detail):
        checks.append({"n": n, "name": name, "ok": bool(ok), "detail": detail})
        Log("  [%2d] %-48s %s  %s" % (n, name, "OK  " if ok else "FAIL", detail))

    busy = [p for p in PORTS if Q.port_busy(p)]
    add(1, "13 个端口全空", not busy, "busy=%s" % busy)

    res = [x for x in T40.residual_all() if "<defunct>" not in x]
    add(2, "无残留 godot/pleiades-sim/Xvfb", not res,
        "%d 条%s" % (len(res), ("：" + res[0][:80]) if res else ""))

    miss = [t for t in ("xvfb-run", "xauth", "Xvfb", "godot") if shutil.which(t) is None]
    rc, so, _ = T40.run_capture(["godot", "--version"], timeout=60)
    add(3, "xvfb-run/xauth/Xvfb/godot 可用", (not miss) and rc == 0,
        ("缺 %s；" % miss if miss else "") + "godot %s" % (so.strip().splitlines() or [""])[0][:40])

    ent = T40.ensure_world_fast()
    extra = T40.world_diff_extra()[1]
    add(4, "world_fast 副本新鲜（diff 只 override.cfg）",
        (WORLD_FAST / "project.godot").is_file() and not extra,
        "%s；diff %s" % (ent.get("reason", "?"), "干净" if not extra else str(extra)[:60]))

    ov = read_text(WORLD_FAST / "override.cfg")
    add(5, "override.cfg 含 msaa_3d=0", "msaa_3d=0" in ov, repr(ov.replace("\n", "⏎")))

    bin_ok = RELEASE_BIN.is_file()
    newest = S2C.binary_sources_newest()
    fresh = bin_ok and RELEASE_BIN.stat().st_mtime > newest[0]
    add(6, "release 二进制存在且比 src 新", fresh,
        ("mtime=%s sha=%s；src 最新 %s" % (time.strftime("%F_%T", time.localtime(
            RELEASE_BIN.stat().st_mtime)), Q.sha256_file(RELEASE_BIN)[:16],
            os.path.basename(newest[1])) if bin_ok else "缺少 %s" % RELEASE_BIN))

    ok_bin, shas, detail_bin = ensure_binary_all()
    add(7, "5 个实例二进制 sha == release", ok_bin, detail_bin)

    yok, ydetail, _ysha = ensure_yolo(out)
    add(8, "yolo 权重就位（UAV 实例）", yok, ydetail)

    add(9, "Pictor kernel 库", KERNEL_SO.is_file(),
        ("%s %d B" % (KERNEL_SO.name, KERNEL_SO.stat().st_size)) if KERNEL_SO.is_file() else "缺少")

    add(10, "Pictor 项目 / ctl 脚本 / 行人脚本",
        (PICTOR / "project.godot").is_file() and CTL.is_file() and PED_TOOL.is_file(),
        "project.godot=%s ctl=%s ped=%s" % ((PICTOR / "project.godot").is_file(), CTL.is_file(),
                                            PED_TOOL.is_file()))

    uav_cfg = S2C.write_uav_config(out)
    ok_uav, bad_uav, _flat = S2C.config_readback(uav_cfg)
    add(11, "UAV config 回读 11 键", ok_uav, "; ".join(bad_uav) if bad_uav else
        "peer=%s net=%d ctl=%d chassis=quadrotor_uav vel_fwd=0.8 yaw=30.0 detector=on/yolov8s"
        % (PEER_UAV, 9125, 9122))

    cfgs = ensure_dirs_and_configs(out)
    bad_all = []
    for peer in PEERS_UGV:
        ok_one, bad_one = ugv_config_readback(cfgs[peer], peer)
        if not ok_one:
            bad_all.append("%s: %s" % (peer, "; ".join(bad_one)))
    add(12, "4×UGV config 回读 11 键", not bad_all,
        "; ".join(bad_all) if bad_all else
        "c1..c4 chassis=diff_drive_ugv max_speed=2.0 fwd=100 turn=35 lidar_front 端口 9131..9134/9135..9138")

    mirror = sync_pictor_mirror(out)
    add(13, "Pictor 镜像 diff 空 + 关键文件 sha", mirror["ok"], mirror["detail"])
    if out is not None:
        (out / "mirror_diff.txt").write_text(
            "\n".join(mirror["diff"]) or "# diff -r（排除 %s）：空\n" % (list(MIRROR_DIFF_EXCLUDES),),
            encoding="utf-8")

    ok_pcfg, bad_pcfg = pictor_config_readback(PICTOR_WS_CFG)
    add(14, "镜像 config 回读 4 键（9123/pictor_s4/订阅 assist）", ok_pcfg,
        "; ".join(bad_pcfg) if bad_pcfg else
        "listen_port=%d peer_name=pictor_s4 ground_station subscribe_topics 含 assist_request" % PIC_NET_PORT)

    sha_now = Q.sha256_file(PICTOR_CFG) if PICTOR_CFG.is_file() else ""
    add(15, "Pictor/.config/config.toml sha 未变（只读）", sha_now == PICTOR_CFG_SHA,
        "sha256=%s（期望 %s…）" % (sha_now[:16] or "<缺文件>", PICTOR_CFG_SHA[:16]))

    now_f, base_f = git_frozen_now(), git_frozen_baseline()
    same_f = now_f is not None and base_f is not None and sorted(now_f) == sorted(base_f)
    add(16, "git 冻结路径 == 基线", same_f,
        "冻结 %d 条" % len(now_f or []) if same_f else "基线 %s vs 现在 %s" % (base_f, now_f))

    pst = git_pictor_now()
    add(17, "Pictor 独立仓状态可读", pst is not None,
        "%d 条改动（快照入 cases.json）" % len(pst or []))

    # #18（D16）：只把 UAV 实例日志提到 debug —— 链路断点唯一可取证处（不改源码/判据）
    ok_dbg, dbg_detail = force_uav_debug(uav_cfg, out)
    add(18, "UAV 实例 [Log] level=debug（D16 诊断）", ok_dbg, dbg_detail)
    return checks


# ================================================================ §8 采样泵


class TextTail:
    """增量读文本日志（Pictor 的 stdout）：每次 `poll_lines` 返回 (新行, 读到时的墙钟)。

    M1/D7：`[Coop]` 行没有时间戳 ⇒ 事件时刻只能“读文件时打戳”。
    """

    def __init__(self, path):
        self.path = pathlib.Path(path)
        self.pos = 0
        self.buf = ""

    def poll_lines(self):
        out = []
        try:
            if not self.path.exists():
                return out
            with self.path.open("rb") as f:
                f.seek(self.pos)
                data = f.read()
                self.pos = f.tell()
        except OSError:
            return out
        if not data:
            return out
        self.buf += data.decode("utf-8", "replace")
        parts = self.buf.split("\n")
        self.buf = parts.pop()
        now = time.time()
        for ln in parts:
            out.append((ln, now))
        return out

    def full_text(self):
        return read_text(self.path)


class Pump:
    """单线程采样泵（plan §4.4）：5 台 sim 的日志 + Pictor 的 stdout，100 ms 一轮。

    **无后台线程**（Step 2 的做法）：`S2C.Stage.wait_release` 复用的 `LogSampler` 必须是同一个对象，
    否则两条 `LogTail` 各读一份，实时判据看到的不是全量行。
    """

    def __init__(self, sims, pictor_log):
        self.sim = {k: S2C.LogSampler(v) for k, v in sims.items()}
        self.pictor = TextTail(pictor_log)
        self.rows = {k: [] for k in sims}
        self.coop = []
        self.pictor_lines = []
        self.cand = 0            # Q8b：被正则命中的候选行
        self.parsed = 0          # Q8b：成功解析的候选行
        self.pictor_cand = 0
        self.pictor_parsed = 0

    def snapshot_preexisting(self):
        return {k: s.snapshot_preexisting() for k, s in self.sim.items()}

    def start(self, wait_s=60.0):
        """等每台实例的**新**日志文件出现再定游标（否则会读到上一轮的旧 `pleiades.log.*`）。"""
        t0 = time.time()
        pending = set(self.sim)
        while pending and time.time() - t0 < wait_s:
            for name in list(pending):
                s = self.sim[name]
                try:
                    files = sorted(s.log_dir.glob("pleiades.log.*"),
                                   key=lambda p: p.stat().st_mtime)
                except OSError:
                    files = []
                fresh = [p for p in files if str(p) not in s.preexisting]
                if fresh:
                    s.path = fresh[-1]
                    s.tail.path = s.path
                    s.tail.pos = 0
                    s.t_start = time.time()
                    pending.discard(name)
            if pending:
                time.sleep(0.2)
        for name in pending:                      # 超时兜底：退回 LogSampler.start()
            self.sim[name].start()
        return pending

    def paths(self):
        return {k: s.path for k, s in self.sim.items()}

    def pump(self):
        for name, s in self.sim.items():
            for ln in s.poll():
                is_cand = bool(CAR_RE.match(ln.rstrip("\n"))) or bool(START_GOTO.search(ln)) \
                    or "[Sim]" in ln or "[Nav]" in ln
                rec = parse_any(ln)
                if is_cand:
                    self.cand += 1
                    if rec is not None:
                        self.parsed += 1
                if rec is not None:
                    self.rows[name].append(rec)
        for ln, t_wall in self.pictor.poll_lines():
            self.pictor_lines.append(ln)
            is_coop = "[Coop]" in ln                 # D15 撤销：族的样本 = 含 `[Coop]` 的行
            if is_coop:
                self.pictor_cand += 1
            m = COOP_REQ.search(ln)
            if m:
                if is_coop:
                    self.pictor_parsed += 1
                self.coop.append({"kind": "req", "t_wall": round(t_wall, 3),
                                  "vid": m.group("vid"), "x": float(m.group("x")),
                                  "y": float(m.group("y")), "n": int(m.group("n")), "raw": ln})
                continue
            m = COOP_NOCAR.search(ln)
            if m:
                if is_coop:
                    self.pictor_parsed += 1
                self.coop.append({"kind": "nocar", "t_wall": round(t_wall, 3),
                                  "vid": m.group("vid"), "x": float(m.group("x")),
                                  "y": float(m.group("y")), "n": 0, "raw": ln})
                continue
            m = COOP_ASG.search(ln)
            if m:
                if is_coop:
                    self.pictor_parsed += 1
                self.coop.append({"kind": "asg", "t_wall": round(t_wall, 3),
                                  "car": m.group("car"), "idx": int(m.group("idx")),
                                  "x": float(m.group("x")), "y": float(m.group("y")),
                                  "d": float(m.group("d")), "raw": ln})

    def until(self, cond, timeout, note=""):
        """每 100 ms：pump() + cond() + deadline。返回 cond 的结果（超时 None）。"""
        t0 = time.time()
        while True:
            self.pump()
            hit = cond()
            if hit:
                return hit
            if time.time() - t0 >= timeout:
                if note:
                    Log("  [pump] %s 超时（%.0f s）" % (note, timeout))
                return None
            time.sleep(PUMP_DT)


class PumpSctl:
    """`S2C.Stage` 的 `sctl` 包装：每次 `seg/row` 前先泵一遍（Pictor 行不留到后面才打戳）。"""

    def __init__(self, sctl, pump):
        self.sctl = sctl
        self.pump = pump

    def seg(self, seconds, phase, stop=None, deadline=None):
        self.pump.pump()
        return self.sctl.seg(seconds, phase, stop=stop, deadline=deadline)

    def row(self, phase):
        self.pump.pump()
        return self.sctl.row(phase)


# ================================================================ §9 用例


class Stage(S2C.Stage):
    """S4-A/B/C/D 时间轴（单进程连续观测，事后按锚点切窗口）。"""

    def __init__(self, ctx, pump, sctl):
        super().__init__(ctx, pump.sim[PEER_UAV], PumpSctl(sctl, pump), pump.sim[PEER_UAV].tail)
        self.pump = pump
        self.sctl_raw = sctl

    # ── 观测（边泵边采 UAV status）
    def observe(self, until_wall, phase="obs", stop=None):
        while time.time() < until_wall:
            self.pump.pump()
            try:
                self.sctl_raw.row(phase)
            except Exception:                      # 采样失败不打断用例（status 只用于留证）
                pass
            if stop is not None and stop():
                return True
            time.sleep(SAMPLE_DT)
        self.pump.pump()
        return False

    # ── S4-A
    def stage_a(self):
        ctx = self.ctx
        Log("\n[S4-A] 无行人巡逻 %.0f s（发 containment 帧）" % A_OBSERVE_S)
        idx0 = len(self.sampler.recs)
        info = self.send_containment()
        ctx["ctl_A"] = info
        ctx["anchors"]["t_send_wall"] = info["sent_at"]
        ctx["anchors"]["t_send_A_wall"] = info["sent_at"]

        r_start = self.sampler.wait_for(
            lambda recs: next((r for r in recs
                               if r.get("kind") == "start" and "Containment {" in r["raw"]), None),
            60.0, idx0)
        ctx["p1"]["start_line"] = r_start["raw"] if r_start else None
        r_build = self.sampler.wait_for(
            lambda recs: next((r for r in recs if r.get("kind") == "build" and r.get("n") == 1), None),
            20.0, idx0)
        ctx["p1"]["build_line"] = r_build["raw"] if r_build else None
        ctx["p1"]["t_build_wall"] = r_build["t"] if r_build else None
        Log("  [S4-A] 开始任务=%s；建树完成(实例数 1)=%s"
            % ("有" if r_start else "无", "有" if r_build else "无"))

        t_end = min(time.time() + A_OBSERVE_S, ctx["hard_deadline"])
        self.observe(t_end, "A")
        ctx["anchors"]["t_A_end_wall"] = time.time()
        Log("  [S4-A] 结束；已收 UGV 位姿行 %s"
            % {p: len(self.pump.rows[p]) for p in PEERS_UGV})

    # ── S4-A2 起步摆位：4 车 goto 四角，≥15 s 且全部停稳后再放人（step.md:18）
    def stage_cars(self):
        ctx, out = self.ctx, self.ctx["out"]
        t0 = time.time()
        info = {"corners": {p: list(CAR_STAGE[p]) for p in PEERS_UGV},
                "t_start_wall": t0, "t_done_wall": None, "wait_s": None,
                "ack": {}, "settle": {}, "settled": False, "timed_out": False}
        Log("\n[S4-A2] 起步摆位：4 车 goto 四角，等 ≥%.0f s 停稳（上限 %.0f s）"
            % (STAGE_MIN_S, STAGE_MAX_S))
        conns = {}
        try:
            for peer in PEERS_UGV:
                c = Q.Control(UGV_CTL[peer])
                if not c.connect(retries=int(STAGE_CONNECT_S / 0.3), delay=0.3):
                    info["ack"][peer] = {"ok": False, "error": "control 口连不上"}
                    Log("  摆位 %-16s 控制口 %d 连不上" % (peer, UGV_CTL[peer]))
                    continue
                conns[peer] = c
                want = CAR_STAGE[peer]
                try:
                    r = c.send_cmd({"cmd": "goto", "x": want[0], "y": want[1]})
                except Exception as exc:                     # noqa: BLE001（连接断/回包坏都记证据）
                    r = {"ok": False, "error": repr(exc)}
                info["ack"][peer] = r
                Log("  摆位 %-16s goto (%.1f, %.1f) → ok=%s"
                    % (peer, want[0], want[1], r.get("ok")))
            deadline = min(t0 + STAGE_MAX_S, ctx["hard_deadline"])
            while True:
                for peer, c in conns.items():
                    try:
                        st = c.status()
                    except Exception:                        # noqa: BLE001（断线由 StatusSampler 兜）
                        continue
                    want = CAR_STAGE[peer]
                    d = math.hypot(float(st.get("x", 0.0)) - want[0],
                                   float(st.get("y", 0.0)) - want[1])
                    info["settle"][peer] = {"x": round(float(st.get("x", 0.0)), 3),
                                            "y": round(float(st.get("y", 0.0)), 3),
                                            "yaw": round(float(st.get("yaw", 0.0)), 2),
                                            "dist": round(d, 2),
                                            "nav_status": st.get("nav_status"),
                                            "queue_len": st.get("queue_len")}
                ok = (len(info["settle"]) == len(PEERS_UGV)
                      and all(v["dist"] <= STAGE_TOL_M for v in info["settle"].values()))
                if time.time() - t0 >= STAGE_MIN_S and ok:
                    break
                if time.time() >= deadline:
                    info["timed_out"] = True
                    break
                self.observe(min(time.time() + 1.0, deadline), "S")
        finally:
            for c in conns.values():
                try:
                    c.close()
                except Exception:                            # noqa: BLE001
                    pass
        info["t_done_wall"] = time.time()
        info["wait_s"] = round(info["t_done_wall"] - t0, 1)
        info["settled"] = (not info["timed_out"]) and len(info["settle"]) == len(PEERS_UGV) \
            and all(v["dist"] <= STAGE_TOL_M for v in info["settle"].values())
        ctx["stage"] = info
        ctx["anchors"]["t_stage_start_wall"] = info["t_start_wall"]
        ctx["anchors"]["t_stage_done_wall"] = info["t_done_wall"]
        write_json(out / "ctl_stage.json", info)
        Log("  摆位结束：%.1f s（settled=%s，timed_out=%s）各车距角点 %s m"
            % (info["wait_s"], info["settled"], info["timed_out"],
               {p: v["dist"] for p, v in info["settle"].items()}))
        return info["settled"]

    # ── 行人（4 段路点 + 驻留；D4 停点 (64,71)）
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
        return S2C.read_ped_csv(self.ctx["out"] / "ped_trace.csv")

    # ── S4-B / C / D（连续观测）
    def run_cases(self):
        ctx = self.ctx
        Log("\n[S4-B] 等待放人（相位锁；复用 Step 2 的 `wait_release`，上限 %.0f s）" % RELEASE_WAIT_S)
        trigger, rec = self.wait_release()
        t_release = time.time()
        ctx["release"]["t_release_wall"] = t_release
        ctx["anchors"]["t_release_wall"] = t_release
        if trigger == "timeout":
            Log("  [S4-B] 放人等待超时（L 命中 %d 次）→ 不放人；Q2~Q5 无数据，按 FAIL 记证据"
                % ctx["release"]["literal_hits"])
            self.observe(min(time.time() + 60.0, ctx["hard_deadline"]), "B")
            return False
        Log("  [S4-B] 触发=%s（等待 %.1f s，v_eff=%s y_target=%s）：%s"
            % (trigger, ctx["release"]["wait_s"], fmt(ctx["release"]["v_eff"], 3),
               fmt(ctx["release"]["y_target"], 2), (rec or {}).get("raw", "")[:110]))
        if not self.start_ped():
            self.observe(min(time.time() + 60.0, ctx["hard_deadline"]), "B")
            return False
        ctx["anchors"]["t_ped_spawn_wall"] = time.time()

        deadline = min(t_release + OBSERVE_CAP_S, ctx["hard_deadline"])
        arrive = {}
        t_out = None
        t_out_wall = None
        while time.time() < deadline:
            self.observe(min(time.time() + 1.0, deadline), "B")
            rows = self.ped_rows()
            for k in (0, 1, 2):
                if k not in arrive:
                    hit = next((r for r in rows if r["leg"] == k and r["phase"] == "dwell"), None)
                    if hit is not None:
                        arrive[k] = hit["world_time"]
                        Log("  [ped] leg%d 到点并驻留：world_time=%.2f pos=(%.2f, %.2f)"
                            % (k, hit["world_time"], hit["x"], hit["y"]))
            if t_out is None:
                hit = next((r for r in rows if r["leg"] >= 3 and outside_square((r["x"], r["y"]))), None)
                if hit is not None:
                    t_out = hit["world_time"]
                    t_out_wall = time.time()
                    Log("  [S4-D] 行人出界：world_time=%.2f pos=(%.2f, %.2f)"
                        % (t_out, hit["x"], hit["y"]))
            if t_out is not None and time.time() >= t_out_wall + P4_SETTLE_S:
                break
        for k in (0, 1, 2):
            if k in arrive:
                ctx["anchors"]["t_leg%d_arrive" % k] = arrive[k]
        ctx["anchors"]["t_out"] = t_out
        if 2 in arrive:
            t_c_end = arrive[2] + 10.0
            if t_out is not None:
                t_c_end = min(t_c_end, t_out)
            ctx["anchors"]["t_S4C_end"] = t_c_end
        Log("  [S4-B/C/D] 结束：leg0/1/2 到点=%s；t_out=%s；观测 %s"
            % ({k: fmt(v, 2) for k, v in arrive.items()}, fmt(t_out, 2),
               "达到上限" if t_out is None else "正常"))
        return t_out is not None

    # ── 收尾前的“最后观测”（把 pictor 的剩余行读干净）
    def final_pump(self, seconds=2.0):
        t_end = time.time() + seconds
        while time.time() < t_end:
            self.pump.pump()
            time.sleep(PUMP_DT)


# ================================================================ §10 判据 Q1~Q8


def coord_eq(a, b):
    """D8：「2 位小数相等」用 0.01 m 容差判。"""
    return abs(a[0] - b[0]) <= COORD_EQ_M and abs(a[1] - b[1]) <= COORD_EQ_M


def quad_labels(dx, dy):
    """相对行人的象限标签（E/W × S/N）；`|dx|` 或 `|dy|` < BOUNDARY_TOL_M 时同时计入相邻两象限。

    plan §1 Q4：「`dx`/`dy` 绝对值 < 0.05 m 的样本记边界、不硬判」——3 m 环的 4 个点恰好落在
    世界轴上（θ=0/90/180/270°），相对真实行人位置几乎总有一条腿落在轴附近，按单值硬判会把
    两个点算进同一象限。
    """
    xs = ("E",) if dx > BOUNDARY_TOL_M else (("W",) if dx < -BOUNDARY_TOL_M else ("E", "W"))
    ys = ("S",) if dy > BOUNDARY_TOL_M else (("N",) if dy < -BOUNDARY_TOL_M else ("S", "N"))
    return [a + b for a in xs for b in ys]


def build_dispatch(coop, tw, goto_starts):
    """把 `coop` 按顺序切成派发事件（1 条 req + 紧随的 asg），并做车侧时间戳交叉校验（M1）。"""
    events = []
    cur = None
    for item in coop:
        if item["kind"] in ("req", "nocar"):
            cur = {"t_wall": item["t_wall"], "t_world": tw(item["t_wall"]), "vid": item["vid"],
                   "req": (item["x"], item["y"]), "n": item["n"],
                   "nocar": item["kind"] == "nocar", "asg": [], "raw": item["raw"]}
            events.append(cur)
        elif item["kind"] == "asg" and cur is not None and not cur["nocar"]:
            cur["asg"].append({"car": item["car"], "idx": item["idx"], "x": item["x"],
                               "y": item["y"], "d": item["d"], "raw": item["raw"]})
    used = {p: set() for p in PEERS_UGV}
    for ev in events:
        ev["assign"] = {a["car"]: a for a in ev["asg"]}
        matched = {}
        for car, a in ev["assign"].items():
            for i, r in enumerate(goto_starts.get(car) or []):
                if i in used[car]:
                    continue
                if not (ev["t_world"] - 0.5 <= r["t_world"] <= ev["t_world"] + 10.0):
                    continue
                if coord_eq((r["x"], r["y"]), (a["x"], a["y"])):
                    used[car].add(i)
                    matched[car] = {"t_world": r["t_world"], "raw": r["raw"]}
                    break
        ev["matched"] = matched
        ev["t_src"] = "wall"
        ev["t_car"] = None
        ev["dt_car_s"] = None
        if matched:
            t_car = median([m["t_world"] for m in matched.values()])
            ev["t_car"] = t_car
            ev["dt_car_s"] = t_car - ev["t_world"]
            if abs(ev["dt_car_s"]) > 1.0:            # M1：>1.0 s 时改用车侧时间
                ev["t_world"] = t_car
                ev["t_src"] = "car"
    return events


def evaluate(ctx, rows, coop, ped_rows, clock, fps):
    """Q1~Q8（Task §6.4）。返回 criteria 列表；**只在 stub 里生成一次**（不二次覆盖）。"""
    tw = S2C.make_to_world(clock)
    uav_recs = rows.get(PEER_UAV) or []
    uav = [dict(r, t_world=tw(r["t"])) for r in uav_recs
           if r.get("t") is not None and r.get("kind") != "det"]
    det = [dict(r, t_world=r["fts_ms"] / 1000.0) for r in uav_recs if r.get("kind") == "det"]
    goals = [r for r in uav if r["kind"] == "goal"]
    arrives = [r for r in uav if r["kind"] == "arrive"]
    fails = [r for r in uav if r["kind"] == "fail"]
    cars = {p: [dict(r, t_world=tw(r["t"])) for r in (rows.get(p) or []) if r.get("kind") == "ugv"]
            for p in PEERS_UGV}
    goto_starts = {p: [dict(r, t_world=tw(r["t"])) for r in (rows.get(p) or [])
                       if r.get("kind") == "goto_start"] for p in PEERS_UGV}
    dispatch = build_dispatch(coop, tw, goto_starts)
    t_send = tw(ctx["anchors"]["t_send_wall"]) if ctx["anchors"].get("t_send_wall") is not None else None
    texts = ctx.get("texts") or {}
    uav_text = ctx.get("sim_text") or ""
    pictor_text = ctx.get("pictor_text") or ""
    parsed = ctx.get("parse") or {}

    # D16：协助链路断点证据（只做取证，**不参与任何判据的 ok 计算**）
    diag = {}
    for key, pat in DIAG_PAT.items():
        diag[key] = uav_text.count(pat)
    diag["samples"] = [ln.strip()[:180] for ln in uav_text.splitlines()
                       if DIAG_PAT["pub_err_assist"] in ln or DIAG_PAT["pub_err_pose"] in ln][:4]
    ctx["assist_diag"] = diag

    ctx["counts"] = {"uav_rows": len([r for r in uav if r["kind"] == "uav"]),
                     "det_rows": len(det),
                     "nav_goal_rows": len(goals),
                     "nav_arrive_rows": len(arrives),
                     "accepted_person": sum(len(r.get("targets") or []) for r in det),
                     "accepted_nonperson": sum(len(r.get("others") or []) for r in det),
                     "coop_req": sum(1 for c in coop if c["kind"] == "req"),
                     "coop_nocar": sum(1 for c in coop if c["kind"] == "nocar"),
                     "coop_asg": sum(1 for c in coop if c["kind"] == "asg"),
                     "ugv_rows": {p: len(cars[p]) for p in PEERS_UGV},
                     "goto_start_rows": {p: len(goto_starts[p]) for p in PEERS_UGV}}
    ctx["ped_speed"] = S2C.ped_median_speed(ped_rows)
    crit = []

    # ── Q1 链路贯通
    cj = (ctx.get("ctl_A") or {}).get("json") or {}
    evs = [d for d in dispatch if not d["nocar"]]
    uav_vid = ctx.get("uav_vid")
    q1a = cj.get("ack_ok") is True
    q1b0 = (cj.get("payload_len") == GOLDEN_LEN and cj.get("payload_hex") == GOLDEN_HEX
            and cj.get("type_on_wire") == GOLDEN_TYPE)
    q1b = bool(evs) and uav_vid is not None and all(d["vid"] == uav_vid for d in evs)
    q1c = bool(evs) and all(len(d["asg"]) == 4 for d in evs)
    q1d = bool(evs) and all(len(d["matched"]) == 4 for d in evs)
    unk = sum(sum(1 for ln in t.splitlines() if any(p in ln for p in UNKNOWN_PATS))
              for t in ([uav_text, pictor_text] + [texts.get(p, "") for p in PEERS_UGV]))
    q1e = unk == 0
    unknown_type = uav_text.count("[Cmd] 未知 mission type")
    q1f = unknown_type == 0
    ok1 = all((q1a, q1b0, q1b, q1c, q1d, q1e, q1f))
    crit.append({
        "n": 1, "name": "链路贯通", "ok": ok1,
        "detail": "ack_ok=%s；帧=%s(%s B,type=%s)；协助请求 %d 条（requester==UAV vid=%s：%s）；"
                  "每条 4 条指派=%s；4 车 `开始任务 Goto` 对上指派点=%s；unknown msgid/parse 失败=%d 行；"
                  "UAV 未知 type=%d 行；无在线车=%d 条"
                  % (cj.get("ack_ok"), cj.get("payload_hex"), cj.get("payload_len"),
                     cj.get("type_on_wire"), len(evs), uav_vid, q1b, q1c, q1d, unk,
                     unknown_type, ctx["counts"]["coop_nocar"]),
        "sub": {"a": q1a, "b_golden": q1b0, "b_vid": q1b, "c": q1c, "d": q1d, "e": q1e, "f": q1f,
                "uav_vid": uav_vid, "n_events": len(evs), "unknown_lines": unk}})

    # ── Q2 点位正确
    q2_rows = []
    for d in evs:
        pts = [(a["x"], a["y"]) for a in d["assign"].values()]
        uniq = len(pts) == 4 and len({(round(x, 2), round(y, 2)) for x, y in pts}) == 4
        ring = len(pts) == 4 and all(
            abs(math.hypot(x - d["req"][0], y - d["req"][1]) - RING_R) <= RING_TOL for x, y in pts)
        pp = S2C.ped_pos_at(ped_rows, d["t_world"])
        dd = [math.hypot(x - pp[0], y - pp[1]) for x, y in pts] if pp is not None else []
        person = bool(dd) and all(PERSON_TOL_FLOOR <= v <= RING_R + PERSON_TOL for v in dd)
        q2_rows.append({"t_world": d["t_world"], "req": [d["req"][0], d["req"][1]],
                        "pts": [[p[0], p[1]] for p in pts], "n": len(pts),
                        "uniq": uniq, "ring": ring, "person": person,
                        "ped": [pp[0], pp[1]] if pp is not None else None,
                        "dmin": min(dd) if dd else None, "dmax": max(dd) if dd else None,
                        "time_src": d["t_src"],
                        "assign": {c: [a["idx"], a["x"], a["y"]] for c, a in d["assign"].items()},
                        "ok": bool(uniq and ring and person)})
    ok2 = bool(q2_rows) and all(r["ok"] for r in q2_rows)
    crit.append({"n": 2, "name": "点位正确", "ok": ok2,
                 "detail": ("无派发事件（`[Coop] 协助请求` 0 条）⇒ Q2 无数据" if not q2_rows else
                            "派发事件 %d 个：四点两两不同=%s；距请求坐标 3.0±%.2f m=%s；距人 %.0f±%.0f m=%s；"
                            "距人 min~max %s（逐事件明细见下）"
                           % (len(q2_rows), all(r["uniq"] for r in q2_rows),
                              RING_TOL, all(r["ring"] for r in q2_rows), RING_R, PERSON_TOL,
                              all(r["person"] for r in q2_rows),
                              ("%.2f~%.2f" % (min(r["dmin"] for r in q2_rows if r["dmin"] is not None),
                                              max(r["dmax"] for r in q2_rows if r["dmax"] is not None)))
                              if any(r["dmin"] is not None for r in q2_rows) else "-")),
                 "sub": {"events": q2_rows}})

    # ── Q3 到达（40 s、每车距其指派点 ≤1.0 m、车间距 ≥1.0 m）
    t_req1 = dispatch[0]["t_world"] if dispatch else None

    def assign_at(car, t):
        best = None
        for d in dispatch:
            if car in (d.get("assign") or {}) and d["t_world"] <= t:
                best = d
        return best

    q3 = {"t_req1": t_req1, "win": [t_req1, t_req1 + ARRIVE_S] if t_req1 is not None else None,
          "cars": {}, "gap_min": None, "gap_t": None}
    if t_req1 is not None:
        for car in PEERS_UGV:
            first, point = None, None
            for r in cars[car]:
                if r["t_world"] < t_req1 or r["t_world"] > t_req1 + ARRIVE_S:
                    continue
                d = assign_at(car, r["t_world"])
                if d is None:
                    continue
                a = d["assign"][car]
                if math.hypot(r["x"] - a["x"], r["y"] - a["y"]) <= ARRIVE_TOL_M:
                    first, point = r["t_world"] - t_req1, (a["x"], a["y"])
                    break
            q3["cars"][car] = {"t_arrive_rel": first, "point": list(point) if point else None,
                               "ok": first is not None}
        series = {car: [(r["t_world"], r["x"], r["y"]) for r in cars[car]] for car in PEERS_UGV}
        t = t_req1
        while t <= t_req1 + ARRIVE_S:
            pos = {car: interp(series[car], t) for car in PEERS_UGV}
            if all(v is not None for v in pos.values()):
                for i, a in enumerate(PEERS_UGV):
                    for b in PEERS_UGV[i + 1:]:
                        d = dist(pos[a], pos[b])
                        if q3["gap_min"] is None or d < q3["gap_min"]:
                            q3["gap_min"], q3["gap_t"] = d, t - t_req1
            t += TRACE_DT
    ok3 = (t_req1 is not None and all(v["ok"] for v in q3["cars"].values())
           and q3["gap_min"] is not None and q3["gap_min"] >= CAR_GAP_M)
    crit.append({"n": 3, "name": "到达", "ok": ok3,
                 "detail": ("无派发事件（`[Coop] 协助请求` 0 条）⇒ Q3 无数据" if t_req1 is None else
                            "窗 [t_req1, +%.0f s]：每车首次距其指派点 ≤%.1f m = %s（%s）；"
                            "车间最小间距 %s m (rel %s s) ≥ %.1f m"
                           % (ARRIVE_S, ARRIVE_TOL_M,
                              {c: (fmt(v["t_arrive_rel"], 1) if v["ok"] else "未达")
                               for c, v in q3["cars"].items()},
                              all(v["ok"] for v in q3["cars"].values()),
                              fmt(q3["gap_min"], 2), fmt(q3["gap_t"], 1), CAR_GAP_M)),
                 "sub": q3})

    # ── Q4 四象限覆盖（每个“已到达”的 dwell）
    dwells = []
    for k in (0, 1, 2):
        rows_k = [r for r in ped_rows if r["leg"] == k]
        dw = [r for r in rows_k if r["phase"] == "dwell"]
        one = {"leg": k, "reach": bool(dw), "reason": None, "settle_t": None, "ped": None,
               "pos": {}, "quads": {}, "ok": False}
        if dw:
            t1 = dw[-1]["world_time"]
            t0 = max(dw[0]["world_time"], t1 - 3.0)
            one["settle_t"] = (t0 + t1) / 2.0
            pp = S2C.ped_pos_at(ped_rows, one["settle_t"])
            one["ped"] = [pp[0], pp[1]] if pp is not None else None
            if pp is not None:
                quads, labels = {}, {}
                for car in PEERS_UGV:
                    pos = interp([(r["t_world"], r["x"], r["y"]) for r in cars[car]], one["settle_t"])
                    if pos is None:
                        continue
                    dx, dy = pos[0] - pp[0], pos[1] - pp[1]
                    lab = quad_labels(dx, dy)
                    labels[car] = lab
                    for key in lab:
                        quads.setdefault(key, []).append(car)
                    one["pos"][car] = [round(pos[0], 2), round(pos[1], 2),
                                       round(dx, 2), round(dy, 2), round(dist(pos, pp), 2)]
                one["quads"] = quads
                one["labels"] = labels
                one["ok"] = all(q in quads for q in ("ES", "EN", "WS", "WN"))
        else:
            # R3 预登记规则：walk 阶段连续 5 s 位移 <0.05 m 且某车距人 <0.8 m → 记“未达（ped 被车挡）”
            walk = [r for r in rows_k if r["phase"] == "walk"]
            stall = False
            if walk:
                for i, a in enumerate(walk):
                    win = [b for b in walk[i:] if b["world_time"] - a["world_time"] <= R3_STALL_S]
                    if len(win) >= 2 and win[-1]["world_time"] - a["world_time"] >= R3_STALL_S - 0.6:
                        span = max(dist((a["x"], a["y"]), (b["x"], b["y"])) for b in win)
                        if span < R3_STALL_M:
                            near = min((dist((a["x"], a["y"]),
                                             interp([(r["t_world"], r["x"], r["y"]) for r in cars[c]],
                                                    a["world_time"]) or (999.0, 999.0))
                                        for c in PEERS_UGV), default=999.0)
                            stall = near < R3_CAR_NEAR_M
                            break
            one["reason"] = "未达（ped 被车挡）" if stall else "未达（ped 未到该点）"
        dwells.append(one)
    reached = [d for d in dwells if d["reach"]]
    ok4 = bool(reached) and all(d["ok"] for d in reached)
    crit.append({"n": 4, "name": "覆盖", "ok": ok4,
                 "detail": "已到达 dwell %d / 3（%s）：四象限全覆=%s（未达按 R3 记并剔除）"
                           % (len(reached), {d["leg"]: d["ok"] for d in reached},
                              all(d["ok"] for d in reached)),
                 "sub": {"dwells": dwells}})

    # ── Q5 分配稳定
    t_end5 = ctx["anchors"].get("t_S4C_end")
    q5 = {"t_req1": t_req1, "t_end": t_end5, "seq": {}, "changes": {}, "dup_events": []}
    if t_req1 is not None and t_end5 is not None:
        for car in PEERS_UGV:
            seq = [d["assign"][car]["idx"] for d in dispatch
                   if d["t_world"] <= t_end5 and car in (d.get("assign") or {})]
            q5["seq"][car] = seq
            q5["changes"][car] = sum(1 for a, b in zip(seq, seq[1:]) if a != b)
        for d in dispatch:
            if d["t_world"] > t_end5:
                break
            idxs = [a["idx"] for a in d["assign"].values()]
            if len(idxs) != len(set(idxs)):
                q5["dup_events"].append({"t_world": d["t_world"], "idx": idxs})
    ok5 = (t_req1 is not None and t_end5 is not None and bool(q5["changes"])
           and all(v <= IDX_CHANGE_MAX for v in q5["changes"].values()) and not q5["dup_events"])
    crit.append({"n": 5, "name": "分配稳定", "ok": ok5,
                 "detail": "窗 [t_req1, t_S4C_end=%s]：每车点序号变化 %s（≤%d）；同事件内两车同点 %d 次"
                           % (fmt(t_end5, 1), q5["changes"], IDX_CHANGE_MAX, len(q5["dup_events"])),
                 "sub": q5})

    # ── Q6 车运动
    q6 = {"cars": {}}
    for car in PEERS_UGV:
        text = texts.get(car, "")
        vmax = 0.0
        for a, b in zip(cars[car], cars[car][1:]):
            dt = b["t_world"] - a["t_world"]
            if not (0.15 <= dt <= 0.6):
                continue
            vmax = max(vmax, dist((a["x"], a["y"]), (b["x"], b["y"])) / dt)
        blocked = sum(1 for ln in text.splitlines() if "拍无路" in ln)
        estop_t = [t for t in (S2C.line_epoch(ln) for ln in text.splitlines() if ESTOP_PAT in ln)
                   if t is not None]
        segs, last = 0, None
        for t in estop_t:
            if last is None or t - last >= ESTOP_GAP_S:
                segs += 1
            last = t
        xs = [r["x"] for r in cars[car]]
        ys = [r["y"] for r in cars[car]]
        bbox = (min(xs) if xs else None, max(xs) if xs else None,
                min(ys) if ys else None, max(ys) if ys else None)
        inb = bool(xs) and all(0.0 < v < BOUND_M for v in xs) and all(0.0 < v < BOUND_M for v in ys)
        q6["cars"][car] = {"vmax": vmax, "blocked": blocked, "estop_lines": len(estop_t),
                           "estop_segs": segs, "bbox": bbox, "in_bounds": inb,
                           "ok": vmax <= CAR_V_MAX and blocked == 0 and segs <= ESTOP_MAX and inb}
    ok6 = bool(q6["cars"]) and all(v["ok"] for v in q6["cars"].values())
    crit.append({"n": 6, "name": "车运动", "ok": ok6,
                 "detail": "逐车 max v=%s m/s（≤%.2f）；Blocked 行=%s；急停段=%s（≤%d）；越界=%s"
                           % ({c: fmt(v["vmax"], 2) for c, v in q6["cars"].items()}, CAR_V_MAX,
                              {c: v["blocked"] for c, v in q6["cars"].items()},
                              {c: v["estop_segs"] for c, v in q6["cars"].items()}, ESTOP_MAX,
                              [c for c, v in q6["cars"].items() if not v["in_bounds"]]),
                 "sub": q6})

    # ── Q7 UAV 回归（P1~P4）
    q7 = {}
    p1 = ctx.get("p1") or {}
    q7["p1a"] = cj.get("ack_ok") is True
    q7["p1b"] = (cj.get("payload_len") == GOLDEN_LEN and cj.get("payload_hex") == GOLDEN_HEX
                 and cj.get("type_on_wire") == GOLDEN_TYPE)
    sline = p1.get("start_line") or ""
    q7["p1c"] = bool(sline) and "Containment {" in sline and "x: 64" in sline and "y: 64" in sline \
        and "half_side: 8.0" in sline
    bline = p1.get("build_line")
    bt = p1.get("t_build_wall")
    tsendw = ctx["anchors"].get("t_send_wall")
    dt_build = (bt - tsendw) if (bt is not None and tsendw is not None) else None
    q7["p1d"] = bool(bline) and ("实例数 1" in bline) and (dt_build is not None and dt_build <= 10.0)
    q7["p1e"] = unknown_type == 0
    q7["p1f"] = uav_text.count("Containment：未启用 detector") == 0
    q7["dt_build_s"] = dt_build
    p1ok = all(q7["p1" + k] for k in ("a", "b", "c", "d", "e", "f"))

    if t_send is not None:
        w0, w1 = t_send, t_send + S2C.A_OBSERVE_S
        uw = [r for r in uav if r["kind"] == "uav" and w0 <= r["t_world"] <= w1]
        corner_min = []
        for c in CORNERS:
            d = min((dist((r["x"], r["y"]), c) for r in uw), default=None)
            corner_min.append({"corner": list(c), "min_d": d,
                               "t": next((r["t_world"] - t_send for r in uw
                                          if dist((r["x"], r["y"]), c) == d), None)})
        hits = [x for x in corner_min if x["min_d"] is not None and x["min_d"] <= Q7["CORNER_TOL"]]
        q7["p2a"] = len(hits) >= Q7["P2_MIN"]
        gw = [r for r in goals if w0 <= r["t_world"] <= w1]
        goal_corners = {}
        for r in gw:
            c = S2C.hit_corner((r["x"], r["y"]))
            if c is not None:
                goal_corners.setdefault(c, []).append(round(r["t_world"] - t_send, 1))
        q7["p2b"] = len(goal_corners) >= Q7["P2_MIN"]
        fail_w = [r for r in fails if w0 <= r["t_world"] <= w1]
        timeout_cnt = sum(1 for ln in uav_text.splitlines() if "Timeout" in ln)
        q7["p2c"] = not fail_w and timeout_cnt == 0
        q7["p2_corners"] = corner_min
        q7["p2_goal_corners"] = {str(k): v for k, v in goal_corners.items()}
        q7["p2_fails"] = len(fail_w)
        q7["p2_timeout"] = timeout_cnt
    else:
        q7["p2a"] = q7["p2b"] = q7["p2c"] = False
        q7["p2_corners"] = []
    p2ok = q7["p2a"] and q7["p2b"] and q7["p2c"]

    t_ped_spawn = ped_rows[0]["world_time"] if ped_rows else None
    ctx["anchors"]["t_ped_spawn"] = t_ped_spawn
    inside_pred = lambda t: not outside_square((t["x"], t["y"]))          # noqa: E731
    hit0 = S2C.pick_p3_t0(det, t_ped_spawn, inside_pred)
    if hit0 is None:
        q7["p3a"] = q7["p3b"] = q7["p3c"] = False
        q7["t0"] = None
        q7["p3_detail"] = ("行人 spawn（%s）之后从未识别到 class=person 且 判定=采纳 的界内目标"
                           % fmt(t_ped_spawn))
    else:
        t0 = hit0["t_world"]
        tgt = hit0["inside_target"]
        ctx["anchors"]["t_first_in_square_detect"] = t0
        # `S2C.follow_switch_evidence` 只吃 `[Sim] UAV` 行（要有 yaw/v/gx/gy）
        p3a, sub3a = S2C.follow_switch_evidence([r for r in uav if r["kind"] == "uav"],
                                                goals, ped_rows, t0, Q7["SWITCH_S"])
        win10 = [r for r in uav if r["kind"] == "uav" and t0 <= r["t_world"] <= t0 + 10.0]
        disp = dist((win10[0]["x"], win10[0]["y"]), (win10[-1]["x"], win10[-1]["y"])) \
            if len(win10) >= 2 else None
        p3b = disp is not None and disp >= Q7_DISP_MIN_M
        d_first, d_last = [], []
        for r in uav:
            if r["kind"] != "uav":
                continue
            pp = S2C.ped_pos_at(ped_rows, r["t_world"])
            if pp is None:
                continue
            d = dist((r["x"], r["y"]), pp)
            if t0 <= r["t_world"] <= t0 + 5.0:
                d_first.append(d)
            elif t0 + 5.0 < r["t_world"] <= t0 + 10.0:
                d_last.append(d)
        m_first = sum(d_first) / len(d_first) if d_first else None
        m_last = sum(d_last) / len(d_last) if d_last else None
        p3c = m_first is not None and m_last is not None and m_last <= m_first + Q7["P3_SLACK"]
        t0_rel = (t0 - t_send) if t_send is not None else None
        q7.update({"p3a": p3a, "p3b": p3b, "p3c": p3c, "t0": t0,
                   "t0_cls": tgt.get("cls"), "t0_conf": tgt.get("conf"),
                   "t0_target": [tgt["x"], tgt["y"]],
                   "t0_rel": t0_rel,
                   "toward_rows": sub3a["toward_rows"], "step_rows": sub3a["step_rows"],
                   "disp10_m": disp, "mean_d_first": m_first, "mean_d_last": m_last,
                   "p3_detail": "t0=%.1f s（world）/ rel %s s；cls=%s conf=%.2f 世界=(%.2f,%.2f)；"
                                "5 s 内 ①朝人 v>0=%d 行 / ②next_cell 朝人一步=%d 行；"
                                "10 s 位移 %s m（≥%.1f）；首/末 5 s 均距 %s→%s m（≤+%.0f）"
                                % (t0, fmt(t0_rel, 1), tgt.get("cls"), tgt.get("conf"),
                                   tgt["x"], tgt["y"], sub3a["toward_rows"], sub3a["step_rows"],
                                   fmt(disp), Q7_DISP_MIN_M, fmt(m_first), fmt(m_last),
                                   Q7["P3_SLACK"])})
    p3ok = q7.get("p3a") and q7.get("p3b") and q7.get("p3c")

    t_out = ctx["anchors"].get("t_out")
    if t_out is None:
        q7["p4a"] = q7["p4b"] = False
        q7["p4_detail"] = "t_out 不存在（人没有出界）"
    else:
        rows4 = [r for r in uav if r["kind"] == "uav" and t_out <= r["t_world"] <= t_out + Q7["P4_S"]]
        gw4 = [r for r in goals if t_out <= r["t_world"] <= t_out + Q7["P4_S"]]
        h4 = [(r, S2C.hit_corner((r["x"], r["y"]))) for r in gw4]
        h4 = [(r, c) for r, c in h4 if c is not None]
        q7["p4a"] = bool(h4)
        q7["p4b"] = False
        q7["p4_d"] = None
        if h4 and len(rows4) >= 2:
            target = h4[0][1]
            d0 = dist((rows4[0]["x"], rows4[0]["y"]), target)
            d1 = dist((rows4[-1]["x"], rows4[-1]["y"]), target)
            q7["p4_d"] = [d0, d1]
            q7["p4b"] = d1 <= d0 - Q7["P4_MIN"]
        q7["p4_detail"] = ("t_out=%.1f s；窗 %.0f s：+%.1f s 新目标命中角点 %s；朝该角点 %s→%s m"
                           % (t_out, Q7["P4_S"],
                              (h4[0][0]["t_world"] - t_out) if h4 else float("nan"),
                              str(h4[0][1]) if h4 else "-",
                              fmt(q7["p4_d"][0]) if q7["p4_d"] else "-",
                              fmt(q7["p4_d"][1]) if q7["p4_d"] else "-"))
    p4ok = q7["p4a"] and q7["p4b"]
    ok7 = p1ok and p2ok and p3ok and p4ok
    crit.append({"n": 7, "name": "UAV 回归", "ok": ok7,
                 "detail": "P1 建树=%s（+%s s）/ P2 巡逻=%s（角点 %d 个、新目标命中 %d 个、失败 %d、Timeout %d）/"
                           "P3 跟随=%s（%s）/ P4 回巡逻=%s（%s）"
                           % (p1ok, fmt(dt_build, 2), p2ok,
                              len([x for x in q7.get("p2_corners") or []
                                   if x["min_d"] is not None and x["min_d"] <= Q7["CORNER_TOL"]]),
                              len(q7.get("p2_goal_corners") or {}), q7.get("p2_fails", 0),
                              q7.get("p2_timeout", 0), p3ok, q7.get("p3_detail"),
                              p4ok, q7.get("p4_detail")),
                 "sub": q7})

    # ── Q8 无异常 + 收尾 + 边界
    anomalies, samples = S2C.count_anomalies(texts)
    fam, badf = {"sim": 0, "nav": 0, "coop": 0}, {"sim": 0, "nav": 0, "coop": 0}
    other = {"sim": 0, "nav": 0}
    for text in texts.values():
        for ln in text.splitlines():
            if CAR_RE.match(ln.rstrip("\n")) or START_GOTO.search(ln):
                fam["sim"] += 1
                if parse_any(ln) is None:
                    badf["sim"] += 1
            elif "[Sim]" in ln:
                other["sim"] += 1
            if S2C.GOAL_RE.search(ln) or S2C.ARR_RE.search(ln):
                fam["nav"] += 1
                if parse_any(ln) is None:
                    badf["nav"] += 1
            elif "[Nav]" in ln and not BLOCKED_RE.search(ln):
                other["nav"] += 1
    fam["coop"] = parsed.get("pictor_cand", 0)
    badf["coop"] = max(0, parsed.get("pictor_cand", 0) - parsed.get("pictor_parsed", 0))
    # D13：只对「判据用到的行」要求 100% 解析；`[Sim] 任务栈` / `[Nav] 连续…无路` 等行进
    #      独立计数（`other` / Blocked / 急停），不参与解析率
    # D15 已撤销（step.md:19 定稿口径）：三族各 ≥1 行且解析率 100%
    parse_ok = (fam["sim"] >= 1 and fam["nav"] >= 1 and fam["coop"] >= 1
                and not any(badf.values()))
    q8a = all(v == 0 for v in anomalies.values())
    q8b = parse_ok
    q8c = (not ctx.get("residual")) and (not ctx.get("busy_ports")) and not ctx.get("xlocks")
    q8d = bool(ctx.get("frozen_equal"))
    q8e = bool(ctx.get("pictor_snapshot_equal"))
    ok8 = q8a and q8b and q8c and q8d and q8e
    crit.append({"n": 8, "name": "无异常", "ok": ok8,
                 "detail": "ERROR/panic/失败/Blocked/SCRIPT ERROR/Parse Error = %s；解析率 sim=%d/nav=%d/"
                           "coop=%d（≥1 行是定稿口径，D15 已撤销；未解析 %s；其它 [Sim]/[Nav] 行 %s 不计入）；"
                           "残留 %d 活；端口 %s；"
                           "X lock=%d；冻结路径=%s；Pictor 仓=%s"
                           % ("/".join(str(anomalies[k]) for k in
                                       ("error", "panic", "task_fail", "blocked",
                                        "script_error", "parse_error")),
                              fam["sim"], fam["nav"], fam["coop"], badf, other,
                              len(ctx.get("residual") or []),
                              "全空" if not ctx.get("busy_ports") else ctx.get("busy_ports"),
                              len(ctx.get("xlocks") or []),
                              "一致" if q8d else "不一致", "一致" if q8e else "不一致"),
                 "sub": {"a": q8a, "b": q8b, "c": q8c, "d": q8d, "e": q8e,
                         "anomalies": anomalies, "samples": samples, "fam": fam,
                         "bad_fam": badf, "other": other, "parse": parsed}})

    # D16/R10：事件密度（只进 verdict 诊断段与 cases.json，**不参与判据**）
    ev_t = [d["t_world"] for d in evs]
    gaps = [round(b - a, 2) for a, b in zip(ev_t, ev_t[1:])]
    ctx["assist_rate"] = {"n_events": len(evs), "gaps": gaps,
                          "gap_median": median(gaps) if gaps else None,
                          "unmatched_events": [i for i, d in enumerate(evs)
                                               if len(d["matched"]) != 4][:10],
                          "asg_counts": [len(d["asg"]) for d in evs][:10]}

    ctx["criteria"] = crit
    ctx["uav_rows"] = uav
    ctx["det_rows"] = det
    ctx["goal_rows"] = goals
    ctx["arrive_rows"] = arrives
    ctx["dispatch"] = dispatch
    ctx["dwells"] = dwells
    ctx["car_rows"] = cars
    ctx["ped_rows"] = ped_rows
    ctx["goto_start_rows"] = goto_starts
    ctx["q2_rows"] = q2_rows
    return crit


# ================================================================ §11 产物

CAR_LABELS = ("c1", "c2", "c3", "c4")
SIM_LOG_NAME = {PEER_UAV: "sim_uav.log"}
for _p in PEERS_UGV:
    SIM_LOG_NAME[_p] = "sim_%s.log" % _p.replace("simq_", "")
MAIN_ARTIFACTS = (["verdict.md", "sim_uav.log"] + ["sim_%s.log" % p.replace("simq_", "") for p in PEERS_UGV]
                  + ["pictor.log", "ped.log", "world.log", "trace.csv", "cases.json"])
TRACE_FIELDS = (["t_world", "uav_x", "uav_y", "ped_x", "ped_y"]
                + [x for lab in CAR_LABELS for x in ("%s_x" % lab, "%s_y" % lab,
                                                     "%s_tx" % lab, "%s_ty" % lab)]
                + ["assist_n", "phase"])


def phase_of(t, anchors, tw):
    t_send = tw(anchors["t_send_wall"]) if anchors.get("t_send_wall") is not None else None
    t_a_end = tw(anchors["t_A_end_wall"]) if anchors.get("t_A_end_wall") is not None else None
    t_stage = tw(anchors["t_stage_done_wall"]) \
        if anchors.get("t_stage_done_wall") is not None else None
    t_rel = tw(anchors["t_release_wall"]) if anchors.get("t_release_wall") is not None else None
    t_leg2 = anchors.get("t_leg2_arrive")
    t_out = anchors.get("t_out")
    if t_send is None or t < t_send:
        return "pre"
    if t_rel is None or t < t_rel:
        # S4-A2 起步摆位窗口（plan §2.9.1）：t_A_end 之后、4 车到角点之前
        if t_a_end is not None and t >= t_a_end and (t_stage is None or t < t_stage):
            return "S"
        return "A"
    if t_leg2 is None or t < t_leg2:
        return "B"
    if t_out is None or t < t_out:
        return "C"
    return "D"


def write_trace(out, ctx):
    """`trace.csv`（plan §4.8 / Task §6.5）：100 ms 网格 + 线性插值 + 每车当期目标点。"""
    dispatch = ctx.get("dispatch") or []
    clock = ctx.get("clock") or {"k": 1.0, "a": 0.0}
    tw = S2C.make_to_world(clock)
    anchors = ctx.get("anchors") or {}
    uav = [(r["t_world"], r["x"], r["y"]) for r in (ctx.get("uav_rows") or [])
           if r.get("kind") == "uav"]
    ped = [(r["world_time"], r["x"], r["y"]) for r in (ctx.get("ped_rows") or [])]
    cars = {p: [(r["t_world"], r["x"], r["y"]) for r in (ctx.get("car_rows") or {}).get(p, [])]
            for p in PEERS_UGV}
    with (out / "trace.csv").open("w", newline="", encoding="utf-8") as f:
        wr = csv.writer(f)
        wr.writerow(TRACE_FIELDS)
        series = uav + ped + [x for v in cars.values() for x in v]
        if not series:
            return 0
        t0 = min(s[0] for s in series)
        t1 = max(s[0] for s in series)
        if anchors.get("t_send_wall") is not None:
            t0 = min(t0, tw(anchors["t_send_wall"]))
        n = 0
        t = t0
        while t <= t1 + 1e-9:
            row = [round(t, 3)]
            u = interp(uav, t)
            row += [round(u[0], 3) if u else "", round(u[1], 3) if u else ""]
            p = interp(ped, t)
            row += [round(p[0], 3) if p else "", round(p[1], 3) if p else ""]
            for car in PEERS_UGV:
                pos = interp(cars[car], t)
                row += [round(pos[0], 3) if pos else "", round(pos[1], 3) if pos else ""]
                tgt = None
                for d in dispatch:
                    if car in (d.get("assign") or {}) and d["t_world"] <= t:
                        tgt = d["assign"][car]
                row += [round(tgt["x"], 3) if tgt else "", round(tgt["y"], 3) if tgt else ""]
            row.append(sum(1 for d in dispatch if d["t_world"] <= t))
            row.append(phase_of(t, anchors, tw))
            wr.writerow(row)
            n += 1
            t += TRACE_DT
    return n


def ensure_artifacts(out, ctx):
    """先给 11 件主产物写占位（早退时产物不能缺；Step 2 的教训）。"""
    made = []
    why = ctx.get("fatal") or "本轮到该阶段前终止（见 driver.log / verdict 的致命闸门段）"
    for name in MAIN_ARTIFACTS:
        p = out / name
        if p.exists():
            continue
        if name.endswith(".csv"):
            p.write_text(",".join(TRACE_FIELDS) + "\n", encoding="utf-8")
        elif name.endswith(".json"):
            p.write_text("{}\n", encoding="utf-8")
        else:
            p.write_text("[%s] 未启动：%s\n" % (name.split(".")[0], why), encoding="utf-8")
        made.append(name)
    return made


def refresh_placeholders(out, names):
    """只保留「内容仍是占位」的产物名。"""
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


def parse_fps(text):
    vals = [float(m.group("f")) for m in FPS_RE.finditer(text)]
    return {"values": vals, "n": len(vals), "median": median(vals),
            "min": min(vals) if vals else None, "max": max(vals) if vals else None}


def durations(ctx, tw):
    """用例时长（秒）：优先用实测 `dur_*`，否则用锚点换算（`--reanalyze` 的同一批产物也能给出）。"""
    a = ctx.get("anchors") or {}

    def wall(key):
        v = a.get(key)
        return None if v is None else tw(v)

    out = {"A": ctx.get("dur_A"), "B": ctx.get("dur_B"), "C": ctx.get("dur_C")}
    t_send, t_a_end = wall("t_send_wall"), wall("t_A_end_wall")
    t_rel, t_spawn, t_end = wall("t_release_wall"), wall("t_ped_spawn_wall"), wall("t_run_end")
    if out["A"] is None and t_send is not None and t_a_end is not None:
        out["A"] = round(t_a_end - t_send, 1)
    if out["B"] is None and t_rel is not None and t_spawn is not None:
        out["B"] = round(t_spawn - t_rel, 1)
    if out["C"] is None and t_spawn is not None and t_end is not None:
        out["C"] = round(t_end - t_spawn, 1)
    return out


def write_verdict(out, ctx, residual, busy, zombie, xlocks):
    crit = ctx["criteria"]
    by = {c["n"]: c for c in crit}
    anchors = ctx.get("anchors") or {}
    clock = ctx.get("clock") or {"k": 1.0, "a": 0.0, "n": 0, "max_resid": 0.0,
                                 "source": "n/a", "cross_check_s": None}
    tw = S2C.make_to_world(clock)
    t_send_w = tw(anchors["t_send_wall"]) if anchors.get("t_send_wall") is not None else None
    cj = (ctx.get("ctl_A") or {}).get("json") or {}
    L = []
    L.append("# task_41 Step 4 verdict — 1 UAV + 4 UGV + Pictor 协控（**只验不改**）")
    L.append("- 口径：UAV `vel_fwd=0.8` m/s（yaw_rate 30 °/s）/ detector `yolov8s.safetensors`（CPU，conf 0.35）"
             " / 行人命令 0.5 m/s / UGV `max_speed=2.0` + `forward_speed=100`（2 m/s）+ `turn_speed=35` / "
             "环半径 3 m（§二.10）/ 判据窗 Q3=%.0f s、Q5 至 t_S4C_end、Q7 P2=%.0f s、P3 位移 ≥%.1f m（人类裁决）"
             % (ARRIVE_S, S2C.A_OBSERVE_S, Q7_DISP_MIN_M))
    dur = durations(ctx, tw)
    L.append("- 运行：%s；rc=%s；总时长 %s s；S4-A %s s / **S4-A2 摆位 %s s** / 放人等待 %s s / "
             "连续观测 %s s"
             % (ctx.get("utc"), ctx.get("rc"), fmt(ctx.get("elapsed_s"), 1), fmt(dur["A"], 1),
                fmt(ctx.get("dur_stage"), 1), fmt(dur["B"], 1), fmt(dur["C"], 1)))
    _sp = ctx.get("ped_speed") or (None, 0)
    L.append("- 行人口径：命令 %.2f m/s（§二.13）；实测中位 %s m/s（n=%s）；停点 (%.0f,%.0f)→(%.0f,%.0f)→"
             "(%.0f,%.0f)→(%.0f,%.0f)（D4：首点文档值 (64,72)）"
             % (PED_SPEED, fmt(_sp[0], 3), _sp[1], PED_STOPS[0][0], PED_STOPS[0][1],
                PED_STOPS[1][0], PED_STOPS[1][1], PED_STOPS[2][0], PED_STOPS[2][1],
                PED_STOPS[3][0], PED_STOPS[3][1]))
    L.append("- 环境：world_fast（diff 只 override.cfg，`msaa_3d=0`）+ `xvfb-run -a`；"
             "1×UAV ctl 9122 / net 9125；4×UGV ctl 9131..9134 / net 9135..9138（`diff_drive_ugv`）；"
             "Pictor 镜像 `RobotSim_TestEnv/pictor_ws`（ws 9100，kernel 9123，订阅含 assist_request）")
    L.append("- 任务帧：`%s`（%s B，type=%s，cx=64 cy=64 half=8），Pictor ack ok=%s"
             % (cj.get("payload_hex", "-"), cj.get("payload_len", "-"),
                cj.get("type_on_wire", "-"), cj.get("ack_ok", "-")))
    L.append("- 时钟：t_world = a + k·t_log（k=%.6f，n=%d，max_resid=%.4f s，源=%s；ped 交叉校验差 %s s）"
             % (clock["k"], clock["n"], clock["max_resid"], clock["source"],
                fmt(clock.get("cross_check_s"), 3)))
    L.append("")
    if ctx.get("fatal"):
        L.append("## 致命闸门（FAIL(env) 时人类先看这段）")
        L.append("- 致命：%s" % ctx["fatal"])
        L.append("")
    L.append("## Q1~Q8（Task §6.4）")
    L.append("")
    L.append("| # | 判据 | 实测 | 阈值 | 结果 |")
    L.append("|---|---|---|---|---|")
    thr = {1: "ack+14 B+4 指派+4 车 Goto", 2: "四点异 / 3.0±0.05 m / 距人 3.0±1.0 m",
           3: "40 s 内 ≤1.0 m / 车间距 ≥1.0 m", 4: "四象限各 ≥1 台",
           5: "序号变化 ≤2 / 无同点", 6: "≤2.05 m/s / 0 Blocked / 急停 ≤1 / 不越界",
           7: "P1~P4（P3 ≥2.5 m）", 8: "0 异常 / 0 残留 / 边界不变"}
    for n in range(1, 9):
        c = by.get(n)
        if c is None:
            L.append("| Q%d | - | 未计算 | %s | FAIL(env) |" % (n, thr.get(n, "-")))
            continue
        tag = "PASS" if c["ok"] else ("FAIL(env)" if c.get("env") else "FAIL")
        L.append("| Q%d | %s | %s | %s | %s |"
                 % (n, c["name"], str(c["detail"]).replace("|", "/"), thr.get(n, "-"), tag))
    L.append("")
    L.append("## 闸门（运行期）")
    L.append("")
    L.append("| 闸门 | 结果 | 证据 |")
    L.append("|---|---|---|")
    for g in ctx.get("gates") or []:
        L.append("| %s | %s | %s |" % (g["name"], "OK" if g["ok"] else "FAIL",
                                       str(g["detail"]).replace("|", "/")))
    L.append("")
    L.append("## Q2 逐事件明细（请求坐标 / 4 个点 / 距人 / time_src）")
    L.append("")
    L.append("| # | t_world (rel t_send) | 请求坐标 | 4 个点（点#:坐标） | 距人 min~max (m) | time_src | 结果 |")
    L.append("|---|---|---|---|---|---|---|")
    for i, r in enumerate(ctx.get("q2_rows") or []):
        pts = " ".join("#%d:(%.2f,%.2f)" % (j, p[0], p[1]) for j, p in enumerate(r["pts"]))
        L.append("| %d | %.2f (%s) | (%.2f,%.2f) | %s | %s~%s | %s | %s |"
                 % (i + 1, r["t_world"],
                    fmt(r["t_world"] - t_send_w) if t_send_w is not None else "-",
                    r["req"][0], r["req"][1], pts, fmt(r["dmin"]), fmt(r["dmax"]),
                    r["time_src"], "PASS" if r["ok"] else "FAIL"))
    if not ctx.get("q2_rows"):
        L.append("| - | - | - | - | - | - | 无派发事件 |")
    L.append("")
    L.append("## Q4 逐 dwell 象限")
    L.append("")
    L.append("| leg | 到点 | settle (world) | 行人位置 | 各车象限（dx,dy,距） | 覆盖 | 结果 |")
    L.append("|---|---|---|---|---|---|---|")
    for d in ctx.get("dwells") or []:
        pos = "; ".join("%s:%s(%s,%s,%.2f)" % (c, "/".join((d.get("labels") or {}).get(c) or ["-"]),
                                               fmt(v[2]), fmt(v[3]), v[4])
                        for c, v in (d.get("pos") or {}).items()) or "-"
        L.append("| %d | %s | %s | %s | %s | %s | %s |"
                 % (d["leg"], "是" if d["reach"] else "否", fmt(d["settle_t"]),
                    ("(%.2f,%.2f)" % (d["ped"][0], d["ped"][1])) if d.get("ped") else "-",
                    pos, ",".join(sorted(d.get("quads") or {})) or "-",
                    ("PASS" if d["ok"] else "FAIL") if d["reach"] else (d.get("reason") or "-")))
    L.append("")
    # ── 协助链路断点证据（D16：UAV 实例 debug 日志；只做取证，不参与判据）
    diag = ctx.get("assist_diag") or {}
    rate = ctx.get("assist_rate") or {}
    L.append("## 协助链路断点证据（D16：UAV 实例 `[Log] level=debug`）")
    L.append("")
    L.append("| 证据串 | 次数 | 说明 |")
    L.append("|---|---|---|")
    L.append("| `%s` | %s | UAV 侧发布成功 ⇒ 断点在 kernel→Godot |"
             % (DIAG_PAT["pub_ok_assist"], diag.get("pub_ok_assist")))
    L.append("| `%s` | %s | UAV 侧发布失败 ⇒ 断点在发布侧（错误类型见下行） |"
             % (DIAG_PAT["pub_err_assist"], diag.get("pub_err_assist")))
    L.append("| `%s` | %s | UAV 学到「kernel 订阅了 assist」⇒ 订阅表正常 |"
             % (DIAG_PAT["sub_assist"], diag.get("sub_assist")))
    L.append("| `%s` | %s | 对照：位姿发布（同一条命令通道） |"
             % (DIAG_PAT["pub_ok_pose"], diag.get("pub_ok_pose")))
    L.append("| `%s` | %s | 对照：位姿发布失败 |"
             % (DIAG_PAT["pub_err_pose"], diag.get("pub_err_pose")))
    L.append("")
    for ln in (diag.get("samples") or [])[:4]:
        L.append("- `%s`" % ln)
    if not (diag.get("pub_ok_assist") or diag.get("pub_err_assist")):
        L.append("- **没有 assist 发布痕迹**：`UAV_LOG_LEVEL=%s` 未生效，或树没走到 `InsideSquare` 成功分支"
                 "（对照 Q7/P3）" % UAV_LOG_LEVEL)
    # ── 事件密度（R10 的诊断：只记录，不合并、不去重、不放宽）
    L.append("- 协助事件：%d 条（间隔中位 %s s）；4 车开始任务未全对上的事件下标：%s"
             % (rate.get("n_events", 0), fmt(rate.get("gap_median")),
                rate.get("unmatched_events")))
    L.append("- 前 10 个事件的指派条数：%s" % (rate.get("asg_counts"),))
    L.append("")
    L.append("## 锚点（world = 绝对世界时间；rel = 相对 t_send）")
    L.append("")
    L.append("| 锚点 | world (s) | rel t_send (s) |")
    L.append("|---|---|---|")
    for label, key, kind in (("t_send", "t_send_wall", "wall"),
                             ("t_A_end", "t_A_end_wall", "wall"),
                             ("t_stage_start", "t_stage_start_wall", "wall"),
                             ("t_stage_done", "t_stage_done_wall", "wall"),
                             ("t_release", "t_release_wall", "wall"),
                             ("t_ped_spawn", "t_ped_spawn", "world"),
                             ("t_leg0_arrive", "t_leg0_arrive", "world"),
                             ("t_leg1_arrive", "t_leg1_arrive", "world"),
                             ("t_leg2_arrive", "t_leg2_arrive", "world"),
                             ("t_S4C_end", "t_S4C_end", "world"),
                             ("t_first_detect", "t_first_in_square_detect", "world"),
                             ("t_out", "t_out", "world"),
                             ("t_run_end", "t_run_end", "wall")):
        val = anchors.get(key)
        if val is None:
            L.append("| %s | - | - |" % label)
            continue
        if kind == "wall":
            val = tw(val)
        L.append("| %s | %.3f | %s |" % (label, val,
                                         "-" if t_send_w is None else "%.3f" % (val - t_send_w)))
    L.append("")
    fps = ctx.get("fps") or {}
    pose = ctx.get("pose_age") or {}
    det = ctx.get("det_hz")
    L.append("## 环境实测")
    L.append("- 世界 fps：中位 %s / min %s / max %s（n=%s；`--print-fps`，D6 无时间戳）"
             % (fmt(fps.get("median"), 1), fmt(fps.get("min"), 1), fmt(fps.get("max"), 1), fps.get("n")))
    L.append("- UAV 位姿龄：中位 %s ms / p90 %s ms（阈值 ≤%d ms）"
             % (fmt(pose.get("median"), 0), fmt(pose.get("p90"), 0), POSE_AGE_MAX_MS))
    L.append("- detector：%s Hz（阈值 ≥%.0f）；采纳目标 person=%s / 非 person=%s"
             % (fmt(det, 2), DET_MIN_HZ, (ctx.get("counts") or {}).get("accepted_person", "-"),
                (ctx.get("counts") or {}).get("accepted_nonperson", "-")))
    L.append("- 计数：%s" % json.dumps(ctx.get("counts") or {}, ensure_ascii=False)[:600])
    L.append("")
    L.append("## 偏差登记（D1~D17）")
    L.append("- D1 只 import 复用三个既有工具脚本（不 import `S2C.main`/`S2C.evaluate`）")
    L.append("- D2 Pictor 用项目镜像 `RobotSim_TestEnv/pictor_ws` 作 `--path`（godot 强制 CWD=--path；"
             "`Pictor/.config` 只读校验 sha=%s）"
             % (ctx.get("pictor_cfg_sha") or "-")[:16])
    L.append("- D3 镜像同步额外排除 `.git`（`diff -r` 同样排除）；关键文件 sha 逐文件核对")
    L.append("- D4 行人首停点 (64,71) vs 文档 (64,72)（边界线问题；Task 原文「南边界内」）")
    L.append("- D5 急停行按代码取 `前方障碍`（`emergency_stop.rs`），plan 的 `[Sim] 前方障碍` 在代码里不存在")
    L.append("- D6 `Project FPS` 行无时间戳 ⇒ 世界 fps 取全程样本中位")
    L.append("- D7 `[Coop]` 行无时间戳 ⇒ M1 墙钟 + 4 车 `开始任务` 交叉校验（>1.0 s 改用车侧时间）")
    L.append("- D8 Q1d 坐标匹配容差 0.01 m（等价于 2 位小数相等）")
    L.append("- D9 Q2c 下界 = 3.0 − 1.0 = 2.0 m")
    L.append("- D10 异常计数覆盖 8 份文本（含 `world.log`）")
    L.append("- D11 `[Sim] UGV` 为 5 Hz ⇒ Q3b 用 100 ms 插值、Q6a 只用 Δt∈[0.15,0.6] s 的相邻行")
    L.append("- D12 行人 leg 从 0 起，仅 leg0/1/2 有 dwell；S4-D 的 leg3 不入 Q4")
    L.append("- D13 Q8b 的解析率按「判据用到的行」算：`[Nav]` 的 Blocked/急停行进独立计数器"
             "（`拍无路`/`前方障碍`），不进 `parse_line` 的记录流")
    L.append("- D14 Q4 的边界样本（|dx| 或 |dy| < %.2f m）同时计入相邻两象限（plan §1「不硬判」）；"
             "3 m 环的 4 点落在世界轴上，单值硬判会把两个点并进同一象限" % BOUNDARY_TOL_M)
    L.append("- D15 **已撤销**：Q8b 恢复定稿口径 —— `[Sim]`/`[Nav]`/`[Coop]` 三族各 ≥1 行且解析率 100%"
             "（上一轮「零样本不硬判」是事后放宽，违反 step.md:19）")
    L.append("- D16 UAV 实例 `[Log] level=\"%s\"`（**只改实例配置**，不改源码/判据）：为看见 "
             "`command_handler.rs:59,62` 的 `gossipsub 发布成功/失败`（info 级不可见）；"
             "证据只进「协助链路断点证据」段，不参与任何判据" % UAV_LOG_LEVEL)
    L.append("- D17 4 车先摆位到四角（step.md:18）⇒ Q4 的「四象限」在车没动时也可能成立；"
             "故 Q4 单独不能证明协同链路，链路结论以 Q1/Q2/Q3/Q5 为准")
    L.append("")
    L.append("## 关键日志（≤10 行）")
    key = ctx.get("key_lines") or []
    if not key:
        key = [ln for ln in (ctx.get("pictor_text") or "").splitlines() if ln.strip()][-5:] or \
              ["（本轮未产生可引用的关键日志：见上节致命闸门与闸门表）"]
    L.append("```text")
    for ln in key[:10]:
        L.append(ln.rstrip())
    L.append("```")
    L.append("")
    L.append("## 收尾")
    L.append("- 残留进程：%d（活）/ %d（<defunct> 忽略）；端口 %s：%s；`/tmp/.X*-lock`：%d"
             % (len(residual), zombie, list(PORTS), "全空" if not busy else "占用 %s" % busy,
                len(xlocks)))
    if residual:
        L.extend("- 残留：%s" % x for x in residual[:5])
    L.append("- 冻结路径 == 基线：%s；Pictor 独立仓 == 进场快照：%s"
             % ("是" if ctx.get("frozen_equal") else "否",
                "是" if ctx.get("pictor_snapshot_equal") else "否"))
    L.append("")
    ok_all = all(c["ok"] for c in crit) and len(crit) == 8
    L.append("**结论：%s**" % ("PASS（Q1~Q8 全过）" if ok_all else
                             "FAIL（见上表；只验不改：未调参、未改判据、未重跑）"))
    (out / "verdict.md").write_text("\n".join(L) + "\n", encoding="utf-8")
    return ok_all


def write_cases_json(out, ctx, checks, pictor_snapshot):
    cj = (ctx.get("ctl_A") or {}).get("json") or {}
    anchors = dict(ctx.get("anchors") or {})
    clock = ctx.get("clock") or {}
    tw = S2C.make_to_world(clock)
    t_send_w = tw(anchors["t_send_wall"]) if anchors.get("t_send_wall") is not None else None
    rel = {}
    for k, v in anchors.items():
        if v is None or not isinstance(v, (int, float)):
            rel[k] = None
            continue
        w = tw(v) if k.endswith("_wall") else v
        rel[k] = None if t_send_w is None else round(w - t_send_w, 3)
    data = {
        "run_tag": out.name,
        "started_utc": ctx.get("started_utc"),
        "finished_utc": utc_iso(),
        "exit_code": ctx.get("rc"),
        "argv": sys.argv,
        "ports": {"world": WORLD_PORT, "pictor_ws": WS_PORT, "pictor_net": PIC_NET_PORT,
                  "uav_ctl": 9122, "uav_net": 9125,
                  "ugv_ctl": UGV_CTL, "ugv_net": UGV_NET},
        "peers": {"uav": PEER_UAV, "ugvs": list(PEERS_UGV), "ped": PEER_PED},
        "configs": {"uav": "simq_uav_containment/.config/config.toml（沿用 Step 2）",
                    "ugv": {p: "configs/ugv_%s.toml" % p.split("_")[-1] for p in PEERS_UGV},
                    "pictor": "RobotSim_TestEnv/pictor_ws/.config/config.toml（镜像）",
                    "pictor_readonly": {"path": "Pictor/.config/config.toml",
                                        "sha256": ctx.get("pictor_cfg_sha")},
                    "checks": checks},
        "gates": ctx.get("gates") or [],
        "p1": ctx.get("p1"),
        "uav_vid": ctx.get("uav_vid"),
        "fatal": ctx.get("fatal"),
        "placeholders": ctx.get("placeholders") or [],
        "mission": {"type": GOLDEN_TYPE, "x": CX, "y": CY, "half_side": HALF,
                    "payload_hex": cj.get("payload_hex"), "payload_len": cj.get("payload_len"),
                    "ack_ok": cj.get("ack_ok"), "type_on_wire": cj.get("type_on_wire")},
        "square": {"cx": CX, "cy": CY, "half": HALF, "corners": [list(c) for c in CORNERS]},
        "person": {"spawn": list(PED_SPAWN), "stops": [list(s) for s in PED_STOPS],
                   "doc_stops": [list(s) for s in PED_DOC_STOPS], "speed": PED_SPEED,
                   "measured_speed_median": (ctx.get("ped_speed") or (None,))[0],
                   "legs": PED_LEGS, "peer_id": PEER_PED, "arrive_tol_m": PED_ARRIVE_TOL},
        "release": ctx.get("release"),
        "anchors": anchors,
        "anchors_rel": rel,
        "clock": clock,
        "fps": ctx.get("fps"),
        "pose_age_ms": ctx.get("pose_age"),
        "detector_hz": ctx.get("det_hz"),
        "counts": ctx.get("counts"),
        "dispatch": [{"t_wall": d["t_wall"], "t_world": d["t_world"], "t_car": d.get("t_car"),
                      "dt_car_s": d.get("dt_car_s"), "time_src": d["t_src"],
                      "vid": d["vid"], "req": list(d["req"]), "n": d["n"],
                      "nocar": d["nocar"],
                      "asg": d["asg"],
                      "matched": {c: m["t_world"] for c, m in (d.get("matched") or {}).items()}}
                     for d in (ctx.get("dispatch") or [])],
        "coop_raw": ctx.get("coop") or [],
        "dwells": ctx.get("dwells") or [],
        "criteria": ctx.get("criteria") or [],
        "stage": ctx.get("stage"),
        "uav_log_level": UAV_LOG_LEVEL,
        "assist_diag": ctx.get("assist_diag"),
        "assist_rate": ctx.get("assist_rate"),
        "script_sha": ctx.get("script_sha"),
        "windows": {"A_s": A_OBSERVE_S, "release_wait_s": RELEASE_WAIT_S,
                    "observe_cap_s": OBSERVE_CAP_S, "p4_settle_s": P4_SETTLE_S,
                    "arrive_s": ARRIVE_S, "idx_change_max": IDX_CHANGE_MAX,
                    "car_v_max": CAR_V_MAX, "estop_max": ESTOP_MAX,
                    "q7_disp_min_m": Q7_DISP_MIN_M, "max_s": ctx.get("max_s"),
                    "stage_min_s": STAGE_MIN_S, "stage_max_s": STAGE_MAX_S,
                    "ped_dwell0_s": 120.0},
        "deviations": ["D1 只 import 复用三个既有工具脚本",
                       "D2 Pictor 项目镜像作 --path（CWD=--path）",
                       "D3 镜像同步排除 .git",
                       "D4 行人首停点 (64,71) vs 文档 (64,72)",
                       "D5 急停行文本取 `前方障碍`（按代码）",
                       "D6 世界 fps 取全程中位（无时间戳）",
                       "D7 [Coop] 事件时刻 = 墙钟 + 车侧交叉校验",
                       "D8 Q1d 坐标容差 0.01 m",
                       "D9 Q2c 下界 2.0 m",
                       "D10 异常计数含 world.log",
                       "D11 [Sim] UGV 5 Hz",
                       "D12 行人 leg 0 起、S4-D 无 dwell",
                       "D13 Q8b 解析率按判据用到的行算",
                       "D14 Q4 边界样本（|dx|/|dy| < 0.05 m）计入相邻两象限",
                       "D15 已撤销：Q8b 恢复三族各 ≥1 行 + 解析率 100%",
                       "D16 UAV 实例 [Log] level=debug（只改实例配置，仅取证不入判据）",
                       "D17 4 车先摆位四角 ⇒ Q4 单独不能证明协同链路"],
        "pictor": {"cwd": str(PICTOR_WS), "config": str(PICTOR_WS_CFG),
                   "kernel_log": ctx.get("pictor_kernel_log"),
                   "kernel_listen_port": PIC_NET_PORT,
                   "pictor_repo_snapshot": pictor_snapshot,
                   "pictor_repo_now": ctx.get("pictor_now")},
        "git_boundary": {"frozen_equal": ctx.get("frozen_equal"),
                         "frozen_now": ctx.get("frozen_now"),
                         "frozen_baseline": ctx.get("frozen_base")},
        "residual_procs": ctx.get("residual") or [],
        "ports_busy": ctx.get("busy_ports") or [],
        "x_locks": ctx.get("xlocks") or [],
        "duration": {"A": durations(ctx, tw).get("A"), "B": durations(ctx, tw).get("B"),
                     "C": durations(ctx, tw).get("C"), "total": ctx.get("elapsed_s")},
    }
    write_json(out / "cases.json", data)


# ================================================================ §12 入口


class EnvFail(Exception):
    """环境/闸门失败（FAIL(env)）：只负责把原因带到 `finalize`，不吞已测数据。"""


def pick_out(args):
    if args.out:
        return pathlib.Path(args.out).resolve()
    return (RESULTS / ("task41_s4_" + utc_stamp())).resolve()


def new_ctx(out, max_s):
    t = time.time()
    return {
        "out": out, "utc": utc_iso(), "started_utc": utc_iso(), "t_process": t,
        "hard_deadline": t + max_s, "max_s": max_s, "finalized": False,
        "procs": {}, "residual": [], "busy_ports": [], "xlocks": [],
        "gates": [], "fatal": None, "anchors": {},
        "release": {"trigger_used": None, "literal_hits": 0, "literal_wait_s": None,
                    "wait_s": None, "v_eff": None, "y_target": None, "trigger_row": None,
                    "skip_passes": 0, "t_release_wall": None},
        "p1": {}, "clock": {"k": 1.0, "a": 0.0, "n": 0, "max_resid": 0.0,
                            "source": "n/a", "cross_check_s": None},
        "counts": {}, "criteria": [], "key_lines": [], "ctl_A": None, "ctl_vehicles": None,
        "uav_vid": None, "sim_text": "", "pictor_text": "", "texts": {}, "parse": {},
        "coop": [], "frozen_equal": False, "pictor_snapshot_equal": False, "rc": None,
        "fps": {}, "pose_age": {}, "det_hz": None, "ped_speed": None,
        "dur_A": None, "dur_B": None, "dur_C": None, "elapsed_s": None, "placeholders": [],
        "dispatch": [], "dwells": [], "q2_rows": [], "car_rows": {}, "ped_rows": [],
        "pictor_kernel_log": None, "pictor_cfg_sha": None,
        "frozen_now": None, "frozen_base": None, "pictor_now": None,
    }


def ctl_vehicles(out=None):
    """`pictor_ws_ctl.py vehicles --json`（拿 UAV 的 vid + 4 台 car 的类型）。"""
    argv = [sys.executable, str(CTL), "vehicles", "--port", str(WS_PORT), "--json"]
    rc, so, se = T40.run_capture(argv, timeout=90)
    data = {}
    for line in reversed(so.splitlines()):
        s = line.strip()
        if s.startswith("{"):
            try:
                data = json.loads(s)
                break
            except json.JSONDecodeError:
                continue
    info = {"argv": argv, "rc": rc, "json": data, "stderr": se[-2000:]}
    if out is not None:
        write_json(out / "ctl_vehicles.json", info)
    return info


def count_parse(texts):
    cand = parsed = 0
    for t in texts.values():
        for ln in t.splitlines():
            is_cand = bool(CAR_RE.match(ln.rstrip("\n"))) or bool(START_GOTO.search(ln)) \
                or "[Sim]" in ln or "[Nav]" in ln
            if is_cand:
                cand += 1
                if parse_any(ln) is not None:
                    parsed += 1
    return cand, parsed


def count_coop(text):
    """`pictor.log` → (族样本数, 可解析数)。与 `Pump.pump` 同一口径（含 `[Coop]` 即算样本）。"""
    cand = parsed = 0
    for ln in text.splitlines():
        if "[Coop]" not in ln:
            continue
        cand += 1
        if COOP_REQ.search(ln) or COOP_NOCAR.search(ln) or COOP_ASG.search(ln):
            parsed += 1
    return cand, parsed


def pick_key_lines(ctx, recs):
    """≤10 行关键日志，优先级（plan §2.8.3）：断点证据在前。

    ① 首条 `[Coop] 协助请求`｜② 首条 assist 的 `gossipsub 发布成功/失败`（D16）｜③ 首 1~2 条
    `[Coop] 指派`｜④ 4 车各 1 条 `[Robot] 开始任务 … Goto`｜⑤ 建树完成 / 开始任务｜兜底：首个异常样本。
    """
    key = []
    ptext = ctx.get("pictor_text") or ""
    uav_text = ctx.get("sim_text") or ""
    for ln in ptext.splitlines():                     # ①
        if "[Coop] 协助请求" in ln:
            key.append(ln)
            break
    for ln in uav_text.splitlines():                  # ②
        if DIAG_PAT["pub_ok_assist"] in ln or DIAG_PAT["pub_err_assist"] in ln:
            key.append(ln)
            break
    n_asg = 0
    for ln in ptext.splitlines():                     # ③
        if "[Coop] 指派" in ln:
            key.append(ln)
            n_asg += 1
            if n_asg >= 2:
                break
    for peer in PEERS_UGV:                            # ④
        for r in (ctx.get("goto_start_rows") or {}).get(peer, [])[:1]:
            key.append(r["raw"])
    for r in recs:                                    # ⑤
        if r["kind"] in ("start", "build", "fail"):
            key.append(r["raw"])
    for r in recs:                                    # 兜底：首个 person 采纳的检测行
        if r["kind"] == "det" and r.get("targets") and len(key) < 10:
            key.append(r["raw"])
            break
    if len(key) < 10:
        anomalies = ((ctx.get("criteria") or [{}])[-1].get("sub") or {}).get("samples") or []
        if anomalies:
            key.append(anomalies[0])
    return key[:10]


def write_fallback_verdict(out, ctx, tb_text):
    """`write_verdict` / `write_cases_json` 抛异常时的兜底 verdict（plan §2.8.4）。

    邮件附件的 `verdict.md` 因此永远不是占位文件，也不需要「见 driver.log」这种死指针：
    Q1~Q8 一律 `FAIL(env)` + traceback 前 20 行 + `pick_key_lines` 的结果都写在这里。
    """
    L = []
    L.append("# task_41 Step 4 verdict（**兜底渲染**：正常渲染抛异常）")
    L.append("- 渲染时间：%s（UTC）" % utc_iso())
    L.append("- 运行：%s；rc=%s；总时长 %s s"
             % (ctx.get("utc"), ctx.get("rc"), fmt(ctx.get("elapsed_s"), 1)))
    if ctx.get("fatal"):
        L.append("- 致命：%s" % ctx["fatal"])
    L.append("")
    L.append("## Q1~Q8（渲染失败 ⇒ 一律 FAIL(env)）")
    L.append("")
    L.append("| # | 判据 | 实测 | 结果 |")
    L.append("|---|---|---|---|")
    for c in (ctx.get("criteria") or []):
        L.append("| Q%d | %s | %s | FAIL(env) |"
                 % (c.get("n"), c.get("name"), str(c.get("detail")).replace("|", "/")))
    L.append("")
    L.append("## traceback（前 20 行）")
    L.append("")
    L.append("```text")
    for ln in (tb_text or "").splitlines()[:20]:
        L.append(ln.rstrip())
    L.append("```")
    L.append("")
    L.append("## 关键日志（≤10 行）")
    L.append("")
    L.append("```text")
    key = [ln for ln in (ctx.get("key_lines") or []) if ln is not None]
    if not key:
        key = ["（无可用关键日志：渲染阶段已失败）"]
    for ln in key[:10]:
        L.append(ln.rstrip())
    L.append("```")
    L.append("")
    (out / "verdict.md").write_text("\n".join(L) + "\n", encoding="utf-8")
    return True


def finalize(ctx, checks, pictor_snapshot, procs, pump, sctl, exit_code, fatal, only=None):
    if ctx.get("finalized"):
        return ctx.get("rc", exit_code)
    ctx["finalized"] = True
    out = ctx["out"]
    if fatal:
        ctx["fatal"] = fatal
    Log("\n── 收尾（ped → pictor → 5×sim → world）──")
    if pump is not None:
        try:
            pump.pump()
        except Exception:
            pass
    if not ctx.get("keep"):
        for name in ["ped", "pictor"] + ["sim_" + p for p in ALL_PEERS] + ["world"]:
            p = (procs or {}).get(name) or (ctx.get("procs") or {}).get(name)
            if p is not None:
                try:
                    p.stop()
                except Exception:
                    traceback.print_exc()
        residual, busy, zombie = wait_clean(20.0)
    else:
        residual, busy, zombie = [], [], 0
    xlocks = sorted(str(p) for p in pathlib.Path("/tmp").glob(".X*-lock"))
    ctx["residual"], ctx["busy_ports"], ctx["xlocks"] = residual, busy, xlocks
    Log("  残留：%d 活进程（%d 个 <defunct>）；端口：%s；/tmp/.X*-lock：%s"
        % (len(residual), zombie, "全空" if not busy else busy, xlocks or "无"))

    # ── 5 份 sim 日志 + pictor / ped / world
    texts = {}
    for peer in ALL_PEERS:
        s = (pump.sim.get(peer) if pump is not None else None)
        dst = out / SIM_LOG_NAME[peer]
        if s is not None and s.path is not None and pathlib.Path(s.path).exists():
            try:
                shutil.copy2(s.path, dst)
            except OSError:
                pass
        texts[peer] = (s.full_text() if s is not None else "") or read_text(dst)
    ctx["sim_text"] = texts.get(PEER_UAV, "")
    ctx["pictor_text"] = read_text(out / "pictor.log")
    texts["pictor"] = ctx["pictor_text"]
    texts["ped"] = read_text(out / "ped.log")
    texts["world"] = read_text(out / "world.log")
    ctx["texts"] = texts

    # ── 时钟 / fps / 位姿龄 / detector / 解析率
    ped_rows = S2C.read_ped_csv(out / "ped_trace.csv")
    recs = parse_text(ctx["sim_text"])
    try:
        ctx["clock"] = S2C.setup_clock(recs, ped_rows)
    except Exception:
        traceback.print_exc()
    ctx["fps"] = parse_fps(texts["world"])
    ages = [r["age"] for r in recs if r.get("kind") == "uav"]
    ctx["pose_age"] = {"median": median(ages), "p90": percentile(ages, 0.9), "n": len(ages)}
    det = [r for r in recs if r.get("kind") == "det"]
    ctx["det_hz"] = ((len(det) - 1) / ((det[-1]["fts_ms"] - det[0]["fts_ms"]) / 1000.0)
                     if len(det) >= 2 and det[-1]["fts_ms"] > det[0]["fts_ms"] else None)
    ctx["parse"] = ({"cand": pump.cand, "parsed": pump.parsed,
                     "pictor_cand": pump.pictor_cand, "pictor_parsed": pump.pictor_parsed}
                    if pump is not None else {"cand": 0, "parsed": 0,
                                              "pictor_cand": 0, "pictor_parsed": 0})
    ctx["coop"] = pump.coop if pump is not None else []

    # ── 边界
    ctx["frozen_now"] = git_frozen_now()
    ctx["frozen_base"] = git_frozen_baseline()
    ctx["frozen_equal"] = (ctx["frozen_now"] is not None and ctx["frozen_base"] is not None
                           and sorted(ctx["frozen_now"]) == sorted(ctx["frozen_base"]))
    ctx["pictor_now"] = git_pictor_now()
    ctx["pictor_snapshot_equal"] = (ctx["pictor_now"] is not None
                                    and sorted(ctx["pictor_now"]) == sorted(pictor_snapshot))
    ctx["pictor_cfg_sha"] = Q.sha256_file(PICTOR_CFG) if PICTOR_CFG.is_file() else ""

    def gate(name, ok, detail):
        ctx["gates"].append({"name": name, "ok": bool(ok), "detail": detail,
                             "t_wall": round(time.time(), 3)})
        Log("  闸门 %-24s %s  %s" % (name, "OK" if ok else "FAIL", detail))
        return ok

    fps_med = (ctx["fps"] or {}).get("median")
    gate("world_fps", fps_med is not None and fps_med >= WORLD_FPS_MIN,
         "Project FPS 中位 %s（n=%s，阈值 ≥%.0f）"
         % (fmt(fps_med, 1), (ctx["fps"] or {}).get("n"), WORLD_FPS_MIN))
    gate("pose_age", ctx["pose_age"]["median"] is not None
         and ctx["pose_age"]["median"] <= POSE_AGE_MAX_MS,
         "UAV 位姿龄中位 %s ms（阈值 ≤%d）" % (fmt(ctx["pose_age"]["median"], 0), POSE_AGE_MAX_MS))

    # ── 判据
    rows = {p: parse_text(texts.get(p, "")) for p in ALL_PEERS}
    if recs and ctx["anchors"].get("t_send_wall"):
        try:
            evaluate(ctx, rows, ctx["coop"], ped_rows, ctx["clock"], ctx["fps"])
        except Exception:
            traceback.print_exc()
            ctx["fatal"] = ctx["fatal"] or "判据计算异常（见 driver.log 的 traceback）"
    have = {c["n"] for c in (ctx["criteria"] or [])}
    for n, nm in ((1, "链路贯通"), (2, "点位正确"), (3, "到达"), (4, "覆盖"),
                  (5, "分配稳定"), (6, "车运动"), (7, "UAV 回归"), (8, "无异常")):
        if n not in have:
            ctx["criteria"].append({"n": n, "name": nm, "ok": False, "env": True, "sub": {},
                                    "detail": "无 sim 日志 / 无发送锚点 / 判据异常（FAIL(env)），无法判定"})
    ctx["criteria"].sort(key=lambda c: c["n"])

    # ── 产物
    ctx["placeholders"] = ensure_artifacts(out, ctx)
    if ctx.get("pictor_kernel_log") and pathlib.Path(ctx["pictor_kernel_log"]).exists():
        try:
            shutil.copy2(ctx["pictor_kernel_log"], out / "pictor_kernel.log")
        except OSError:
            pass
    try:
        ntr = write_trace(out, ctx)
    except Exception:
        traceback.print_exc()
        ntr = 0
    env_fail = bool(ctx.get("fatal")) or exit_code == 2 \
        or not all(g["ok"] for g in (ctx.get("gates") or []))
    for c in ctx["criteria"]:
        if env_fail and not c["ok"]:
            c["env"] = True
    ctx["key_lines"] = pick_key_lines(ctx, recs)
    ok_all = (all(c["ok"] for c in ctx["criteria"]) and len(ctx["criteria"]) == 8
              and not residual and not busy and not xlocks
              and ctx["frozen_equal"] and ctx["pictor_snapshot_equal"])
    rc = 2 if env_fail else (0 if ok_all else 1)
    ctx["rc"] = rc
    ctx["elapsed_s"] = round(time.time() - ctx["t_process"], 1)
    try:
        write_verdict(out, ctx, residual, busy, zombie, xlocks)
        write_cases_json(out, ctx, checks, pictor_snapshot)
    except Exception:
        tb = traceback.format_exc()
        traceback.print_exc()
        rc = 2
        ctx["rc"] = 2
        try:                                    # plan §2.8.4：兜底 verdict（不留占位/死指针）
            write_fallback_verdict(out, ctx, tb)
        except Exception:
            traceback.print_exc()
    ctx["placeholders"] = refresh_placeholders(out, ctx.get("placeholders") or [])
    for c in ctx["criteria"]:
        if only is not None and ("Q%d" % c["n"]) not in only:
            continue
        Log("  [Q%d] %-10s %s  %s" % (c["n"], c["name"],
                                      "PASS" if c["ok"] else ("FAIL(env)" if c.get("env") else "FAIL"),
                                      c["detail"]))
    if ctx.get("fatal"):
        Log("  [致命] %s" % ctx["fatal"])
    Log("\n  产物：verdict.md / sim_uav.log / sim_ugv_c1..c4.log / pictor.log / ped.log / world.log / "
        "trace.csv（%d 行）/ cases.json（+ped_trace.csv %d 行）" % (ntr, len(ped_rows)))
    Log("\nEXIT=%d" % rc)
    return rc


def run_real(args):
    out = pick_out(args)
    out.mkdir(parents=True, exist_ok=True)
    fh = open(out / "driver.log", "w", buffering=1, encoding="utf-8")
    LOG_FH[0] = fh
    ctx = new_ctx(out, args.max_s)
    ctx["script_sha"] = Q.sha256_file(pathlib.Path(__file__))
    ctx["t_process"] = time.time()
    ctx["hard_deadline"] = ctx["t_process"] + args.max_s
    pictor_snapshot = git_pictor_now() or []
    os.environ["LIBGL_ALWAYS_SOFTWARE"] = "1"
    os.environ.pop("RUST_LOG", None)      # D16：保证 `[Log] level` 生效（bootstrap.rs:107-108）
    procs, pump, sctl = {}, None, None
    exit_code = 2
    fatal = None
    checks = []
    only = None
    if args.only:
        only = {x.strip().upper() for x in args.only.split(",") if x.strip()}
    Log("=" * 92)
    Log("task_41 Step 4 — 1 UAV + 4 UGV + Pictor 协控（只验不改，1 轮）  产物 → %s" % out)
    Log("=" * 92)
    try:
        Log("\n── 前置检查（18 条）──")
        checks = preflight(out)
        bad = [c for c in checks if not c["ok"]]
        if bad:
            for c in bad:
                Log("[PREFLIGHT FAIL] #%d %s：%s" % (c["n"], c["name"], c["detail"]))
            raise EnvFail("静态前置未过（%d 条 FAIL：%s）→ FAIL(env)"
                          % (len(bad), [c["n"] for c in bad]))

        def gate(name, ok, detail):
            ctx["gates"].append({"name": name, "ok": bool(ok), "detail": detail,
                                 "t_wall": round(time.time(), 3)})
            Log("  闸门 %-24s %s  %s" % (name, "OK" if ok else "FAIL", detail))
            return ok

        # ── 世界
        Log("\n── 起世界（world_fast + xvfb-run -a + --print-fps）──")
        procs["world"] = start_proc("world", T40.world_argv() + ["--print-fps"], ROOT,
                                    out / "world.log")
        dt = wait_text(out / "world.log", "[ws] listening on port %d" % WORLD_PORT, WORLD_LISTEN_S)
        gate("world_listen", dt is not None, "%.1f s" % dt if dt is not None else "timeout")
        if dt is None:
            raise EnvFail("世界 %.0f s 内没有监听 :%d（FAIL(env)）" % (WORLD_LISTEN_S, WORLD_PORT))
        Log("  世界监听 :%d（%.1f s）" % (WORLD_PORT, dt))

        # ── Pictor（镜像）
        Log("\n── 起 Pictor（--path %s）──" % PICTOR_WS)
        pre_kernel = ({p.name for p in PICTOR_WS_LOG.glob("pleiades.log.*")}
                      if PICTOR_WS_LOG.is_dir() else set())
        procs["pictor"] = start_proc(
            "pictor", ["timeout", str(PICTOR_TIMEOUT_S), "godot", "--headless",
                       "--path", str(PICTOR_WS), "--", "--ws-port=%d" % WS_PORT],
            PICTOR_WS, out / "pictor.log")
        dtp = wait_text(out / "pictor.log", "[WsCmd] listening on 127.0.0.1:%d" % WS_PORT,
                        PICTOR_LISTEN_S)
        gate("pictor_ws_9100", dtp is not None, "%.1f s" % dtp if dtp is not None else "timeout")
        if dtp is None:
            raise EnvFail("Pictor WS %.0f s 内未监听 :%d（FAIL(env)）" % (PICTOR_LISTEN_S, WS_PORT))
        klog, kports, t_k = None, [], time.time()
        while time.time() - t_k < PICTOR_KERNEL_S:
            fresh = sorted([p for p in PICTOR_WS_LOG.glob("pleiades.log.*")
                            if p.name not in pre_kernel], key=lambda p: p.stat().st_mtime)
            if fresh:
                klog = fresh[-1]
                kports = sorted({int(m.group("port"))
                                 for m in S2C.KERNEL_LISTEN_RE.finditer(read_text(klog))})
                if kports:
                    break
            time.sleep(0.2)
        ctx["pictor_kernel_log"] = str(klog) if klog is not None else None
        gate("kernel_libp2p_9123", kports == [PIC_NET_PORT],
             "监听地址 …/tcp/%d/（%s）" % (PIC_NET_PORT, klog.name if klog is not None else "<无>")
             if kports == [PIC_NET_PORT] else "实测端口=%s（日志 %s）" % (kports or "无", klog))
        if kports != [PIC_NET_PORT]:
            raise EnvFail("Pictor kernel 未绑定 :%d（实测 %s）→ FAIL(env)" % (PIC_NET_PORT, kports or "无"))
        try:
            with socket.create_connection(("127.0.0.1", PIC_NET_PORT), 0.5):
                pass
            gate("kernel_tcp_connect", True, "127.0.0.1:%d 可连" % PIC_NET_PORT)
        except OSError as exc:
            gate("kernel_tcp_connect", False, repr(exc))
            raise EnvFail("Pictor kernel :%d 已打印监听但 TCP 不可连（%r）→ FAIL(env）"
                          % (PIC_NET_PORT, exc))
        t_sub = time.time()
        while time.time() - t_sub < 30.0:
            if "GossipSub 已订阅 6 个 topic" in read_text(klog):
                break
            time.sleep(0.2)
        sub_ok = "GossipSub 已订阅 6 个 topic" in read_text(klog)
        gate("kernel_subscribed", sub_ok,
             "`GossipSub 已订阅 6 个 topic`（5 默认 + assist_request）")
        if not sub_ok:
            raise EnvFail("kernel 未订阅到 6 个 topic（assist_request 未生效）→ FAIL(env)")

        # ── 5 个机栈实例
        Log("\n── 起 5 个机栈实例（UAV + 4×UGV，间隔 1.5 s）──")
        pump = Pump({p: inst_dir(p) / "Log" for p in ALL_PEERS}, out / "pictor.log")
        pump.snapshot_preexisting()
        before = read_text(out / "world.log").count("vehicle spawned")
        for peer in ALL_PEERS:
            procs["sim_" + peer] = start_proc(
                "sim_" + peer, [str(inst_dir(peer) / "pleiades-sim"), "--headless"],
                inst_dir(peer), out / ("stdout_%s.log" % peer))
            time.sleep(1.5)
        pending_logs = pump.start()
        if pending_logs:
            Log("  [WARN] 这些实例 %.0f s 内没写出新日志，退回最新一份：%s"
                % (60.0, sorted(pending_logs)))
        for peer in ALL_PEERS:
            Log("  sim 日志 %-20s %s" % (peer, pathlib.Path(pump.sim[peer].path).name
                                         if pump.sim[peer].path else "<无>"))
        t0 = time.time()
        while time.time() - t0 < SPAWN_S:
            if read_text(out / "world.log").count("vehicle spawned") >= before + 5:
                break
            time.sleep(0.3)
        got = read_text(out / "world.log").count("vehicle spawned") - before
        gate("world_vehicle_spawned", got >= 5,
             "world.log +%d 条 `vehicle spawned`（%.1f s）" % (got, time.time() - t0))
        if got < 5:
            raise EnvFail("世界 %.0f s 内没有 5 条 `vehicle spawned`（实测 +%d）→ FAIL(env）"
                          % (SPAWN_S, got))

        ctls, asm, t0 = {}, {}, time.time()
        while time.time() - t0 < ASSEMBLED_S:
            for peer in ALL_PEERS:
                if asm.get(peer):
                    continue
                port = 9122 if peer == PEER_UAV else UGV_CTL[peer]
                c = ctls.get(peer)
                if c is None:
                    c = Q.Control(port)
                    if not c.connect(retries=3, delay=0.3):
                        continue
                    ctls[peer] = c
                try:
                    if c.status().get("assembled"):
                        asm[peer] = True
                except Exception:
                    try:
                        c.close()
                    except Exception:
                        pass
                    ctls.pop(peer, None)
            if len(asm) == len(ALL_PEERS):
                break
            time.sleep(0.5)
        gate("sim_assembled_x5", len(asm) == len(ALL_PEERS),
             "%d/5 完成 assembly（%.1f s）" % (len(asm), time.time() - t0))
        if len(asm) < len(ALL_PEERS):
            raise EnvFail("仅 %d/5 完成 assembly（%s）→ FAIL(env）"
                          % (len(asm), [p for p in ALL_PEERS if p not in asm]))
        sctl = S2C.StatusSampler(ctls[PEER_UAV])
        for peer in PEERS_UGV:
            try:
                ctls[peer].close()
            except Exception:
                pass
        Log("  5/5 已装配（%.1f s）" % (time.time() - t0))

        def sim_has(needle, timeout):
            t0 = time.time()
            while time.time() - t0 < timeout:
                if needle in pump.sim[PEER_UAV].full_text():
                    return round(time.time() - t0, 2)
                time.sleep(0.3)
            return None

        dt_det = sim_has("[detector] 已启动", DETECTOR_S)
        gate("detector_started", dt_det is not None,
             "%.1f s" % dt_det if dt_det is not None else "timeout")
        dt_cam = sim_has("slot=camera_nose 首帧", CAMERA_S)
        gate("camera_first_frame", dt_cam is not None,
             "%.1f s" % dt_cam if dt_cam is not None else "timeout")
        if dt_det is None or dt_cam is None:
            raise EnvFail("就绪闸门未过（detector=%s 相机=%s）→ FAIL(env）" % (dt_det, dt_cam))

        reg = {}
        for peer in ALL_PEERS:
            rc, so, se = T40.run_capture(
                [sys.executable, str(CTL), "wait", "--target", peer, "--port", str(WS_PORT),
                 "--json", "--timeout", str(REGISTER_S)], timeout=REGISTER_S + 30)
            reg[peer] = rc
            Log("  ctl wait %-20s rc=%s %s" % (peer, rc, (so.strip().splitlines() or [""])[-1][:110]))
        gate("pictor_registered_x5", all(v == 0 for v in reg.values()), "rc=%s" % reg)
        if any(v != 0 for v in reg.values()):
            raise EnvFail("有实例未在 Pictor 注册（%s）→ FAIL(env）" % reg)

        vinfo = ctl_vehicles(out)
        ctx["ctl_vehicles"] = vinfo
        vlist = (vinfo.get("json") or {}).get("list") or []
        car_rows = [r for r in vlist if str(r.get("node_type")) == "car"]
        uav_row = next((r for r in vlist if str(r.get("name")) == PEER_UAV), None)
        ctx["uav_vid"] = str(uav_row.get("vid")) if uav_row else None
        gate("pictor_cars_typed", len(car_rows) == 4 and all(r.get("vid") for r in car_rows),
             "vehicles=%d / car=%d / uav_vid=%s"
             % (len(vlist), len(car_rows), ctx["uav_vid"]))
        if len(car_rows) != 4:
            raise EnvFail("`ctl vehicles` 里 car 类型不是 4 台（实测 %d）→ FAIL(env）" % len(car_rows))

        # ── 用例
        stage = Stage(ctx, pump, sctl)
        t_a = time.time()
        stage.stage_a()
        ctx["dur_A"] = round(time.time() - t_a, 1)
        t_s = time.time()
        staged = stage.stage_cars()
        ctx["dur_stage"] = round(time.time() - t_s, 1)
        st_info = ctx.get("stage") or {}
        gate("cars_staged_x4", staged,
             "4 车 goto 四角（52,52)/(76,52)/(76,76)/(52,76)：%s s；距角点 %s m"
             % (fmt(st_info.get("wait_s"), 1),
                {p: v.get("dist") for p, v in (st_info.get("settle") or {}).items()}))
        if not staged:
            raise EnvFail("4 车未在 %.0f s 内停到四角（见 ctl_stage.json）→ FAIL(env）" % STAGE_MAX_S)
        t_b = time.time()
        stage.run_cases()
        tot = round(time.time() - t_b, 1)
        ctx["dur_B"] = ctx["release"].get("wait_s") or tot
        ctx["dur_C"] = round(max(0.0, tot - (ctx["dur_B"] or 0.0)), 1)
        stage.final_pump(2.0)
        ctx["anchors"]["t_run_end"] = time.time()
        exit_code = 1
    except EnvFail as exc:
        fatal = str(exc)
        exit_code = 2
    except Exception:
        traceback.print_exc()
        fatal = "运行期异常（见 driver.log 的 traceback）"
        exit_code = 2
    finally:
        try:
            rc = finalize(ctx, checks, pictor_snapshot, procs, pump, sctl, exit_code, fatal, only)
        finally:
            try:
                fh.close()
            except Exception:
                pass
            LOG_FH[0] = None
    return rc


def run_dry(out):
    LOG_FH[0] = open(out / "dryrun.log", "w", buffering=1, encoding="utf-8")
    try:
        Log("=" * 92)
        Log("task_41 Step 4 — 静态前置（--dry-run，不起任何长驻进程）  产物 → %s" % out)
        Log("=" * 92)
        checks = preflight(out)
        bad = [c for c in checks if not c["ok"]]
        write_json(out / "checks.json", {"utc": utc_iso(), "argv": sys.argv, "checks": checks,
                                         "ok": not bad, "n": len(checks),
                                         "n_pass": len(checks) - len(bad)})
        Log("\nPREFLIGHT %s (%d/%d)" % ("OK" if not bad else "FAIL",
                                        len(checks) - len(bad), len(checks)))
        for c in bad:
            Log("  [FAIL] #%d %s：%s" % (c["n"], c["name"], c["detail"]))
        return 0 if not bad else 2
    finally:
        LOG_FH[0].close()
        LOG_FH[0] = None


# ---------------------------------------------------------------- §12.1 --selftest（纯离线）

SELFTEST_URL = "https://selftest.invalid/"


def _synth(out):
    """合成 ctx/rows/coop/ped_rows/clock（不碰进程/显示/网络），用于跑通 `evaluate` + 产物。"""
    T = 1000.0
    UGV_START = {PEERS_UGV[0]: (66.5, 64.0), PEERS_UGV[1]: (64.0, 66.5),
                 PEERS_UGV[2]: (61.5, 64.0), PEERS_UGV[3]: (64.0, 61.5)}
    P1 = {PEERS_UGV[0]: (67.0, 71.0), PEERS_UGV[1]: (64.0, 74.0),
          PEERS_UGV[2]: (61.0, 71.0), PEERS_UGV[3]: (64.0, 68.0)}
    P2 = {PEERS_UGV[0]: (67.0, 64.0), PEERS_UGV[1]: (64.0, 67.0),
          PEERS_UGV[2]: (61.0, 64.0), PEERS_UGV[3]: (64.0, 61.0)}
    E1, E2, T_OUT = T + 60.0, T + 95.0, T + 140.0
    rows = {p: [] for p in ALL_PEERS}

    def emit_uav(t, x, y, yaw, v):
        rows[PEER_UAV].append({"kind": "uav", "t": t, "x": x, "y": y, "yaw": yaw,
                               "gx": None, "gy": None, "v": v, "omega": 0.0, "age": 30,
                               "raw": "[Sim] UAV 位置=(%.2f,%.2f) yaw=%.1f° next=None → v=%.1f"
                                      " omega=0 位姿龄=30ms" % (x, y, yaw, v)})

    def emit_car(peer, t, x, y, yaw):
        rows[peer].append({"kind": "ugv", "t": t, "x": x, "y": y, "yaw": yaw,
                           "gx": None, "gy": None, "speed": 100, "turn": 0, "age": 20,
                           "raw": "2026-10-30T06:00:00.000000Z  INFO [Sim] UGV 位置=(%.2f,%.2f) "
                                  "yaw=%.1f° next=None → speed=100 turn=0 位姿龄=20ms" % (x, y, yaw)})

    def seg(emit, t0, t1, p0, p1, dt=0.5, v=0.8):
        n = max(1, int(round((t1 - t0) / dt)))
        moving = (abs(p1[0] - p0[0]) > 1e-9 or abs(p1[1] - p0[1]) > 1e-9)
        yaw = math.degrees(math.atan2(p1[1] - p0[1], p1[0] - p0[0])) if moving else 90.0
        for i in range(n + 1):
            k = i / n
            emit(t0 + (t1 - t0) * k, p0[0] + (p1[0] - p0[0]) * k, p0[1] + (p1[1] - p0[1]) * k,
                 yaw, v if moving else 0.0)

    # UAV：巡逻 → 跟随 → 回巡逻
    seg(emit_uav, T, T + 20.0, (56.0, 56.0), (72.0, 56.0), 0.5, 0.8)
    seg(emit_uav, T + 20.0, T + 40.0, (72.0, 56.0), (72.0, 72.0), 0.5, 0.8)
    seg(emit_uav, T + 40.0, T + 50.0, (72.0, 72.0), (72.0, 72.0), 0.5, 0.0)
    seg(emit_uav, T + 50.0, T + 68.0, (72.0, 72.0), (64.5, 71.0), 0.5, 0.8)
    seg(emit_uav, T + 68.0, T + 140.0, (64.5, 71.0), (64.5, 71.0), 1.0, 0.0)
    seg(emit_uav, T + 140.0, T + 160.0, (64.0, 66.0), (70.0, 71.0), 0.5, 0.8)
    rows[PEER_UAV].append({"kind": "goal", "t": T + 0.5, "x": 56.0, "y": 56.0,
                           "raw": "[Nav] 新目标 (56.00, 56.00)"})
    rows[PEER_UAV].append({"kind": "goal", "t": T + 21.0, "x": 72.0, "y": 56.0,
                           "raw": "[Nav] 新目标 (72.00, 56.00)"})
    rows[PEER_UAV].append({"kind": "goal", "t": T + 141.0, "x": 72.0, "y": 72.0,
                           "raw": "[Nav] 新目标 (72.00, 72.00)"})
    uav_text = "\n".join(r["raw"] for r in rows[PEER_UAV])
    uav_text = ("2026-10-30T06:00:01.000000Z  INFO [Robot] 开始任务 TaskId(1)："
                "Containment { x: 64.0, y: 64.0, half_side: 8.0 }\n"
                "2026-10-30T06:00:01.500000Z  INFO [Sim/uav] 任务 TaskId(1) 建树完成（实例数 1）\n"
                # D16：合成 1 条「assist 发布成功」（debug 级；selftest 只验计数路径）
                "2026-10-30T06:00:58.000000Z DEBUG pleiades_base::network::command_handler: "
                "gossipsub 发布成功: topic=pleiades/robot/assist_request id=MessageId(\"1a2b3c\")\n"
                + uav_text)

    # 行人：leg0 dwell (64,71) / leg1 dwell (64,64) / leg2 dwell (66,64) / leg3 出界
    ped = []

    def ped_emit(t, x, y, leg, phase):
        ped.append({"t_wall": t, "world_time": t, "x": x, "y": y, "z": 0.0, "yaw_deg": 90.0,
                    "v": 0.5 if phase == "walk" else 0.0, "leg": leg, "phase": phase})

    def pseg(t0, t1, p0, p1, leg, phase, dt=0.2):
        n = max(1, int(round((t1 - t0) / dt)))
        for i in range(n + 1):
            k = i / n
            ped_emit(t0 + (t1 - t0) * k, p0[0] + (p1[0] - p0[0]) * k,
                     p0[1] + (p1[1] - p0[1]) * k, leg, phase)

    pseg(T + 40.0, T + 58.0, (64.0, 80.0), (64.0, 71.0), 0, "walk")
    pseg(T + 58.0, T + 78.0, (64.0, 71.0), (64.0, 71.0), 0, "dwell")
    pseg(T + 78.0, T + 92.0, (64.0, 71.0), (64.0, 64.0), 1, "walk")
    pseg(T + 92.0, T + 112.0, (64.0, 64.0), (64.0, 64.0), 1, "dwell")
    pseg(T + 112.0, T + 116.0, (64.0, 64.0), (66.0, 64.0), 2, "walk")
    pseg(T + 116.0, T + 126.0, (66.0, 64.0), (66.0, 64.0), 2, "dwell")
    pseg(T + 126.0, T + 154.0, (66.0, 64.0), (64.0, 50.0), 3, "walk")

    # 4 台车：出生 → 事件 1 四点 → 事件 2 四点
    for peer in PEERS_UGV:
        seg(lambda t, x, y, yaw, v, _p=peer: emit_car(_p, t, x, y, yaw),
            T + 30.0, E1, UGV_START[peer], UGV_START[peer], 0.2, 0.0)
        seg(lambda t, x, y, yaw, v, _p=peer: emit_car(_p, t, x, y, yaw),
            E1, E1 + 15.0, UGV_START[peer], P1[peer], 0.2, 0.4)
        seg(lambda t, x, y, yaw, v, _p=peer: emit_car(_p, t, x, y, yaw),
            E1 + 15.0, E2, P1[peer], P1[peer], 0.5, 0.0)
        seg(lambda t, x, y, yaw, v, _p=peer: emit_car(_p, t, x, y, yaw),
            E2, E2 + 15.0, P1[peer], P2[peer], 0.2, 0.3)
        seg(lambda t, x, y, yaw, v, _p=peer: emit_car(_p, t, x, y, yaw),
            E2 + 15.0, T + 165.0, P2[peer], P2[peer], 0.5, 0.0)

    def start_row(t, xy):
        return {"kind": "goto_start", "t": t, "x": xy[0], "y": xy[1],
                "raw": "2026-10-30T06:00:00.000000Z  INFO [Robot] 开始任务 TaskId(2)："
                       "Goto { x: %.1f, y: %.1f }" % (xy[0], xy[1])}

    for peer in PEERS_UGV:
        rows[peer].append(start_row(E1 + 0.2, P1[peer]))
        rows[peer].append(start_row(E2 + 0.2, P2[peer]))

    # det（person 界内；t0 = 第一条 y ≤ 72 的）
    dets = []
    for i in range(60):
        t = T + 42.0 + 2.0 * i
        pp = next((p for p in ped if p["world_time"] >= t), ped[-1])
        dets.append({"kind": "det", "t": t, "t_world": t, "fts_ms": int(t * 1000.0),
                     "seq": i, "infer_ms": 600, "ndet": 1, "px": 0.0, "py": 0.0,
                     "pz": 0.0, "pyaw": 0.0, "others": [],
                     "targets": [{"cls": "person", "conf": 0.62, "x": pp["x"], "y": pp["y"],
                                  "range_m": 6.0, "bearing": 0.0, "rel": 0.0}],
                     "raw": "[detector] frame_ts=%d" % int(t * 1000.0)})

    # coop（2 个事件，各 4 条指派）
    coop = []
    pic_lines = []
    POINT = {PEERS_UGV[0]: 0, PEERS_UGV[1]: 1, PEERS_UGV[2]: 2, PEERS_UGV[3]: 3}
    for ev_t, req, pts in ((E1, (64.0, 71.0), P1), (E2, (64.0, 64.0), P2)):
        coop.append({"kind": "req", "t_wall": ev_t, "vid": "deadbeef", "x": req[0],
                     "y": req[1], "n": 4,
                     "raw": "[Coop] 协助请求 requester=deadbeef 目标=(%.2f,%.2f) 车数=4 "
                            "半径=3.0m 分配=…" % req})
        pic_lines.append(coop[-1]["raw"])
        for peer in PEERS_UGV:
            xy = pts[peer]
            coop.append({"kind": "asg", "t_wall": ev_t + 0.01, "car": peer,
                         "idx": POINT[peer], "x": xy[0], "y": xy[1], "d": 5.0,
                         "raw": "[Coop] 指派 车=%s 点#%d 目标=(%.2f,%.2f) 当前距=5.00 m"
                                % (peer, POINT[peer], xy[0], xy[1])})
            pic_lines.append(coop[-1]["raw"])

    car_texts = {p: "\n".join(r["raw"] for r in rows[p]) for p in PEERS_UGV}
    texts = dict(car_texts)
    texts[PEER_UAV] = uav_text
    texts["pictor"] = "\n".join(pic_lines)
    texts["ped"] = "2026-10-30T06:00:40.000000Z  INFO [ped] ready peer='s4_ped'\n"
    texts["world"] = "2026-10-30T06:00:00.000000Z  INFO [ws] vehicle spawned\n"

    ctx = new_ctx(out, 600)
    ctx["t_process"] = time.time()
    ctx["anchors"] = {"t_send_wall": T, "t_A_end_wall": T + 30.0, "t_release_wall": T + 35.0,
                      "t_ped_spawn": T + 40.0, "t_leg0_arrive": T + 58.0,
                      "t_leg1_arrive": T + 92.0, "t_leg2_arrive": T + 116.0,
                      "t_S4C_end": T + 126.0, "t_out": T_OUT,
                      "t_first_in_square_detect": T + 54.0, "t_run_end": T + 170.0}
    ctx["p1"] = {"start_line": ("2026-10-30T06:00:01.000000Z  INFO [Robot] 开始任务 TaskId(1)："
                                "Containment { x: 64.0, y: 64.0, half_side: 8.0 }"),
                 "build_line": "2026-10-30T06:00:01.500000Z  INFO [Sim/uav] 任务 TaskId(1) "
                               "建树完成（实例数 1）",
                 "t_build_wall": T + 1.5}
    ctx["ctl_A"] = {"json": {"ack_ok": True, "payload_hex": GOLDEN_HEX, "payload_len": GOLDEN_LEN,
                             "type_on_wire": GOLDEN_TYPE}}
    ctx["uav_vid"] = "deadbeef"
    ctx["texts"] = texts
    ctx["sim_text"] = uav_text
    ctx["pictor_text"] = texts["pictor"]
    ctx["parse"] = {"cand": 0, "parsed": 0, "pictor_cand": len(coop), "pictor_parsed": len(coop)}
    ctx["coop"] = coop
    ctx["clock"] = {"k": 1.0, "a": 0.0, "n": 60, "max_resid": 0.0,
                    "source": "selftest", "cross_check_s": 0.0}
    ctx["fps"] = {"values": [10.0, 11.0, 10.5], "n": 3, "median": 10.5, "min": 10.0, "max": 11.0}
    ctx["pose_age"] = {"median": 40.0, "p90": 80.0, "n": 300}
    ctx["det_hz"] = 0.5
    ctx["frozen_equal"] = True
    ctx["pictor_snapshot_equal"] = True
    ctx["gates"] = [{"name": "selftest", "ok": True, "detail": "合成数据（不跑进程）", "t_wall": T}]
    # S4-A2 摆位（plan §2.10）+ D16 证据（合成 1 条发布成功）
    ctx["anchors"]["t_stage_start_wall"] = T + 30.0
    ctx["anchors"]["t_stage_done_wall"] = T + 33.0
    ctx["stage"] = {"wait_s": 15.0, "settled": True, "timed_out": False,
                    "corners": {p: list(CAR_STAGE[p]) for p in PEERS_UGV},
                    "settle": {p: {"x": CAR_STAGE[p][0], "y": CAR_STAGE[p][1], "yaw": 0.0,
                                   "dist": 0.2, "nav_status": "arrived", "queue_len": 0}
                               for p in PEERS_UGV},
                    "ack": {p: {"ok": True} for p in PEERS_UGV}}
    ctx["assist_diag"] = {"pub_ok_assist": 1, "pub_err_assist": 0, "sub_assist": 1,
                          "pub_ok_pose": 9, "pub_err_pose": 0, "samples": []}
    ctx["script_sha"] = "selftest"
    # 合成 rows：det 也进 UAV 记录流；`开始任务` / `建树完成` 的原始行也在（pick_key_lines 的 ⑤）
    all_rows = {p: list(rows[p]) for p in ALL_PEERS}
    all_rows[PEER_UAV] = ([{"kind": "start", "t": T + 1.0, "raw": ctx["p1"]["start_line"]},
                           {"kind": "build", "t": T + 1.5, "n": 1, "raw": ctx["p1"]["build_line"]}]
                          + all_rows[PEER_UAV] + dets)
    checks = [{"n": 1, "name": "selftest", "ok": True, "detail": "合成"}]
    return ctx, all_rows, coop, ped, ctx["clock"], checks, ["selftest"]


def run_selftest():
    out = ROOT / "tmp" / "task41" / "selftest"
    out.mkdir(parents=True, exist_ok=True)
    LOG_FH[0] = open(out / "selftest.log", "w", buffering=1, encoding="utf-8")
    try:
        Log("=" * 92)
        Log("task_41 Step 4 — --selftest（纯离线：合成数据跑通 evaluate + 三个产物）  产物 → %s" % out)
        Log("=" * 92)
        ctx, rows, coop, ped_rows, clock, checks, snapshot = _synth(out)
        ctx["criteria"] = evaluate(ctx, rows, coop, ped_rows, clock, ctx["fps"])
        ensure_artifacts(out, ctx)
        ctx["key_lines"] = pick_key_lines(ctx, rows.get(PEER_UAV) or [])
        ntr = write_trace(out, ctx)
        write_verdict(out, ctx, [], [], 0, [])
        write_cases_json(out, ctx, checks, snapshot)
        assert len(ctx["criteria"]) == 8, "criteria 不是 8 条：%d" % len(ctx["criteria"])
        vtext = read_text(out / "verdict.md")
        for n in range(1, 9):
            assert ("| Q%d |" % n) in vtext, "verdict 缺少 `| Q%d |` 行" % n
        cj = json.loads(read_text(out / "cases.json"))
        assert len(cj.get("criteria") or []) == 8, "cases.json.criteria 不是 8 条"
        assert ntr >= 2, "trace.csv 行数 %d < 2" % ntr
        for k in ("stage", "assist_diag", "assist_rate", "script_sha", "uav_log_level"):
            assert k in cj, "cases.json 缺少 %s" % k
        # 反例：协助链路断（coop 0 行）时 Q8 必须 FAIL（D15 已撤销，定稿口径）
        neg = out / "_neg"
        neg.mkdir(parents=True, exist_ok=True)
        ctx0, rows0, _coop0, ped0, clock0, _checks0, _snap0 = _synth(neg)
        ctx0["coop"] = []
        ctx0["parse"] = dict(ctx0["parse"], pictor_cand=0, pictor_parsed=0)
        ctx0["pictor_text"] = ""
        ctx0["texts"] = dict(ctx0["texts"], pictor="")
        crit0 = evaluate(ctx0, rows0, [], ped0, clock0, ctx0["fps"])
        q8_0 = next(c for c in crit0 if c["n"] == 8)
        assert not q8_0["ok"], "coop 0 行时 Q8 仍 PASS ⇒ 定稿口径没生效"
        Log("  [selftest] 反例 OK：coop 0 行 ⇒ Q8 FAIL")
        for c in ctx["criteria"]:
            Log("  [Q%d] %-10s %s  %s" % (c["n"], c["name"],
                                          "PASS" if c["ok"] else "FAIL", c["detail"]))
        Log("SELFTEST OK（trace.csv %d 行；criteria 8 条；verdict/cases.json 齐）" % ntr)
        return 0
    except Exception:
        traceback.print_exc()
        Log("SELFTEST FAIL")
        return 2
    finally:
        if LOG_FH[0] is not None:
            LOG_FH[0].close()
            LOG_FH[0] = None


# ---------------------------------------------------------------- §12.2 --reanalyze（同批产物离线重渲染）


def run_reanalyze(dirpath):
    out = pathlib.Path(dirpath).resolve()
    if not out.is_dir():
        Log("REANALYZE FAIL：目录不存在 %s" % out)
        return 2
    LOG_FH[0] = open(out / "driver.log", "a", buffering=1, encoding="utf-8")
    try:
        Log("=" * 92)
        Log("task_41 Step 4 — --reanalyze（只读同一批产物，不重启任何进程）  %s" % out)
        Log("=" * 92)
        old = {}
        try:
            old = json.loads(read_text(out / "cases.json"))
        except Exception:
            old = {}
        ctx = new_ctx(out, 0)
        ctx["t_process"] = time.time()
        started = old.get("started_utc") or ""
        m = re.match(r"^.*_(\d{8}T\d{6}Z)$", str(old.get("run_tag") or out.name))
        if m:
            started = "%s-%s-%sT%s:%s:%sZ" % (m.group(1)[0:4], m.group(1)[4:6], m.group(1)[6:8],
                                              m.group(1)[9:11], m.group(1)[11:13], m.group(1)[13:15])
        ctx["started_utc"] = started or utc_iso()
        ctx["utc"] = "%s（reanalyze）" % ctx["started_utc"]
        ctx["anchors"] = {k: v for k, v in (old.get("anchors") or {}).items() if v is not None}
        ctx["p1"] = old.get("p1") or {}
        ctx["uav_vid"] = old.get("uav_vid")
        ctx["stage"] = old.get("stage")            # 同一批产物：摆位证据原样保留（plan §2.9）
        ctx["script_sha"] = old.get("script_sha")  # 原跑次的进场 sha（证明出结果后没改脚本）
        if old.get("release"):
            ctx["release"] = old["release"]
        ctx["ctl_A"] = {"json": old.get("mission") or {}}
        texts = {}
        for peer in ALL_PEERS:
            texts[peer] = read_text(out / SIM_LOG_NAME[peer])
        ctx["sim_text"] = texts.get(PEER_UAV, "")
        ctx["pictor_text"] = read_text(out / "pictor.log")
        texts["pictor"] = ctx["pictor_text"]
        texts["ped"] = read_text(out / "ped.log")
        texts["world"] = read_text(out / "world.log")
        ctx["texts"] = texts
        ctx["coop"] = old.get("coop_raw") or []
        cand, parsed = count_parse(texts)
        coop_cand, coop_parsed = count_coop(ctx["pictor_text"])
        ctx["parse"] = {"cand": cand, "parsed": parsed,
                        "pictor_cand": coop_cand, "pictor_parsed": coop_parsed}
        ped_rows = S2C.read_ped_csv(out / "ped_trace.csv")
        recs = parse_text(ctx["sim_text"])
        ctx["clock"] = old.get("clock") or S2C.setup_clock(recs, ped_rows)
        if not (ctx["clock"] or {}).get("k"):
            ctx["clock"] = {"k": 1.0, "a": 0.0, "n": 0, "max_resid": 0.0,
                            "source": "relative", "cross_check_s": None}
        ctx["fps"] = parse_fps(texts["world"])
        ages = [r["age"] for r in recs if r.get("kind") == "uav"]
        ctx["pose_age"] = {"median": median(ages), "p90": percentile(ages, 0.9), "n": len(ages)}
        det = [r for r in recs if r.get("kind") == "det"]
        ctx["det_hz"] = ((len(det) - 1) / ((det[-1]["fts_ms"] - det[0]["fts_ms"]) / 1000.0)
                         if len(det) >= 2 and det[-1]["fts_ms"] > det[0]["fts_ms"] else None)
        residual, busy, zombie = wait_clean(1.0)
        xlocks = sorted(str(p) for p in pathlib.Path("/tmp").glob(".X*-lock"))
        ctx["residual"], ctx["busy_ports"], ctx["xlocks"] = residual, busy, xlocks
        ctx["frozen_now"], ctx["frozen_base"] = git_frozen_now(), git_frozen_baseline()
        ctx["frozen_equal"] = (ctx["frozen_now"] is not None and ctx["frozen_base"] is not None
                               and sorted(ctx["frozen_now"]) == sorted(ctx["frozen_base"]))
        ctx["pictor_now"] = git_pictor_now()
        snap = (old.get("pictor") or {}).get("pictor_repo_snapshot") or []
        ctx["pictor_snapshot_equal"] = (ctx["pictor_now"] is not None
                                        and sorted(ctx["pictor_now"]) == sorted(snap))
        ctx["pictor_cfg_sha"] = Q.sha256_file(PICTOR_CFG) if PICTOR_CFG.is_file() else ""
        ctx["gates"] = old.get("gates") or []
        rows = {p: parse_text(texts.get(p, "")) for p in ALL_PEERS}
        if recs and ctx["anchors"].get("t_send_wall"):
            evaluate(ctx, rows, ctx["coop"], ped_rows, ctx["clock"], ctx["fps"])
        have = {c["n"] for c in (ctx["criteria"] or [])}
        for n, nm in ((1, "链路贯通"), (2, "点位正确"), (3, "到达"), (4, "覆盖"),
                      (5, "分配稳定"), (6, "车运动"), (7, "UAV 回归"), (8, "无异常")):
            if n not in have:
                ctx["criteria"].append({"n": n, "name": nm, "ok": False, "env": True, "sub": {},
                                        "detail": "产物不足，无法判定"})
        ctx["criteria"].sort(key=lambda c: c["n"])
        ctx["placeholders"] = ensure_artifacts(out, ctx)
        write_trace(out, ctx)
        dur_old = old.get("duration") or {}
        ctx["dur_A"], ctx["dur_B"], ctx["dur_C"] = dur_old.get("A"), dur_old.get("B"), dur_old.get("C")
        ctx["dur_stage"] = (old.get("stage") or {}).get("wait_s")
        ctx["elapsed_s"] = dur_old.get("total")
        ok_all = all(c["ok"] for c in ctx["criteria"]) and len(ctx["criteria"]) == 8
        rc = 0 if ok_all else 1
        ctx["rc"] = rc
        ctx["key_lines"] = pick_key_lines(ctx, recs)
        write_verdict(out, ctx, residual, busy, zombie, xlocks)
        write_cases_json(out, ctx, (old.get("configs") or {}).get("checks") or [], snap)
        Log("REANALYZE OK（criteria 8 条；verdict.md 已重写；EXIT=%d）" % rc)
        return rc
    except Exception:
        traceback.print_exc()
        Log("REANALYZE FAIL")
        return 2
    finally:
        if LOG_FH[0] is not None:
            LOG_FH[0].close()
            LOG_FH[0] = None


def main():
    ap = argparse.ArgumentParser(
        description="task_41 Step 4 — 1 UAV + 4 UGV + Pictor 协控端到端验证（只验不改）")
    ap.add_argument("--out", default=None, help="产物目录（默认 RobotSim_TestEnv/results/task41_s4_<UTC>）")
    ap.add_argument("--dry-run", action="store_true", help="只跑静态前置（产物落 tmp/task41/dryrun/）")
    ap.add_argument("--selftest", action="store_true", help="纯离线自测（合成数据，产物落 tmp/task41/selftest/）")
    ap.add_argument("--reanalyze", default=None, metavar="DIR", help="同一批产物离线重渲染 verdict.md")
    ap.add_argument("--only", default=None, help="只裁剪 stdout 打印的判据，如 Q1,Q7")
    ap.add_argument("--max-s", type=int, default=1200, help="全脚本时长上限（默认 1200 s）")
    ap.add_argument("--keep", action="store_true", help="结束时**不**停进程（仅调试，默认停）")
    args = ap.parse_args()
    S2C.Log = Log              # 复用函数（`wait_release` 等）写进 driver.log
    if args.selftest:
        return run_selftest()
    if args.reanalyze:
        return run_reanalyze(args.reanalyze)
    if args.dry_run:
        out = ROOT / "tmp" / "task41" / "dryrun"
        out.mkdir(parents=True, exist_ok=True)
        return run_dry(out)
    return run_real(args)


if __name__ == "__main__":
    sys.exit(main())
