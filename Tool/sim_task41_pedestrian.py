#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Presented by KeJi
# Created Date ： 2026-10-30
# Modified Date ： 2026-10-30
#
# task_41 Step 2 行人「路点 + 驻留」驱动（plan.md §2.2 / 偏差 D9）。
#
# 为什么另起一个脚本：Task §4.2 要求「行人 `pedestrian_client.py`、`python3 -u`」，而 S2-B 需要
#   「走到 (64,71) → **停 20 s** → 再走向 (64,64)」。`patrol.py` 没有驻留能力（只有 `--linger`
#   停发探针），`pedestrian_client.py` 只能定速直线。本脚本**只加一层「路点 @ 驻留」驱动**，
#   内部**直接 import** `pedestrian_client.PedestrianClient`：同一个客户端、同一套协议、
#   同样的 `python3 -u` 启动方式，**不复制任何协议代码**。
#
# 停点口径（D4）：文档 §4.3 写 (64,72)，但那一点**恰在正方形边界线上**（|y−64| = 8.00），而
#   `--arrive-tol 0.30` 会让它停在 y∈(72.00,72.30]（**界外**）；停点取边界内 1 m 的 (64,71)
#   （Task 原文即「南边界**内**」）。详见测试脚本的 D4。
#
# 三条硬边界（与 `pedestrian_client.py` 一致）：
#   ① 只发 `motion_cmd{v,dir}`、只读 `pose`：不读地图、不连 P2P、不认识任何 Pleiades 概念。
#   ② `dir` 用**世界系弧度**（0 = +X 东，逆时针为正）；世界侧 `rotation.y = -dir`，这里不再取反。
#   ③ 世界 300 ms 收不到 `motion_cmd` 就自动停车（只停不断连）⇒ 必须持续发（30 Hz，见 `--send-hz`）。
#
# 每帧的决策（照 `patrol.py` 的结构，刻意做到最简单，**不含任何避障**）：
#   误差 > --turn-threshold-deg → v=0 原地转（避免「边走边大角度纠正」画大弧）
#   否则                        → v=--speed 边走边微调朝向
#   到点（|Δ| ≤ --arrive-tol）   → 驻留 dwell 秒（v=0）→ 下一个路点
#
# 用法：
#   python3 -u Tool/sim_task41_pedestrian.py --spawn 64,80 \
#       --legs "64,71@20;64,64@3;64,50@0" --speed 0.5 --peer-id s2_ped \
#       --csv <out>/ped_trace.csv --arrive-tol 0.30 --quiet
#
# 产物：`--csv` 指定的 CSV（`t_wall,world_time,x,y,z,yaw_deg,v,leg,phase`），每来一条新 pose 写一行。
# 退出码沿用 `pedestrian_client`：0 / 2 连接失败 / 3 ready 超时 / 4 协议异常 / 5 spawn 被拒 /
#   6 验证不过（本脚本不用）/ 130 Ctrl-C 或 SIGTERM。

import argparse
import csv
import math
import os
import signal
import sys
import time

# 同目录没有客户端；它住在世界仓的 tool/pedestrian/ 下（与 `patrol.py` 同源）
_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.dirname(_HERE)
_PED_DIR = os.path.join(_ROOT, "orion-simulation-world", "tool", "pedestrian")
if _PED_DIR not in sys.path:
    sys.path.insert(0, _PED_DIR)

from pedestrian_client import (  # noqa: E402  （本地模块，必须在 sys.path 之后）
    DEFAULT_SEND_HZ, DEFAULT_URL, EXIT_INTERRUPTED, EXIT_OK, EXIT_PROTOCOL,
    ClientError, PedestrianClient, ProtocolError, SpawnFallback, Wrap_Pi,
    parse_spawn_spec,
)

# ---------------------------------------------------------------- 常量
DEFAULT_PEER_ID = "s2_ped"
DEFAULT_SPEED = 0.5                     # m/s（§二.13：行人 0.5 m/s，UAV 0.8 m/s 追得上；世界 clamp 0~1.26）
DEFAULT_ARRIVE_TOL = 0.30               # 到点判定（m）
DEFAULT_TURN_THRESHOLD_DEG = 20.0       # 大于这个误差先原地转
DEFAULT_DURATION = 300.0                # 走完所有路点后保持连接的上限（秒；本轮用例变长）
DEFAULT_LEGS = "64,71@20;64,64@3;64,50@0"

## SIGINT / SIGTERM：处理函数里只置标志，收尾仍在主循环的 finally 里做（不抛异常外溢）。
_INTERRUPTED = False


def _On_Signal(signum, _frame):
    global _INTERRUPTED
    _INTERRUPTED = True
    print("[ped] signal=%d → 收尾" % int(signum), flush=True)


# ---------------------------------------------------------------- 路点解析
def parse_legs(text):
    """`"64,71@20;64,64@3;64,50@0"` → `[(x, y, dwell_s), ...]`（dwell 缺省 0）。

    分号（或换行）分隔路点，`@` 后面是驻留秒数。任何一段不像 `x,y[@dwell]` 都直接抛 ValueError
    （宁可在启动时炸，也不要在 S2-B 中途静默少走一段）。
    """
    legs = []
    for chunk in str(text).replace("\n", ";").split(";"):
        chunk = chunk.strip()
        if not chunk:
            continue
        body, at, dwell = chunk.partition("@")
        parts = [p.strip() for p in body.split(",") if p.strip()]
        if len(parts) != 2:
            raise ValueError("路点必须写成 `x,y[@dwell]`，收到 %r" % (chunk,))
        try:
            x, y = float(parts[0]), float(parts[1])
        except ValueError as exc:
            raise ValueError("路点里有不是数字的段：%r" % (chunk,)) from exc
        dwell_s = 0.0
        if at:
            try:
                dwell_s = max(0.0, float(dwell))
            except ValueError as exc:
                raise ValueError("驻留秒数不是数字：%r" % (chunk,)) from exc
        legs.append((x, y, dwell_s))
    if not legs:
        raise ValueError("--legs 至少要给一个路点")
    return legs


# ---------------------------------------------------------------- CLI
def _build_parser():
    p = argparse.ArgumentParser(
        description="task_41 Step 2 行人「路点 + 驻留」驱动"
                    "（内部 import pedestrian_client.PedestrianClient，只加驱动层）")
    p.add_argument("--url", default=DEFAULT_URL, help="世界 WS（默认 %s）" % DEFAULT_URL)
    p.add_argument("--peer-id", default=DEFAULT_PEER_ID, help="世界内 peer_id（默认 %s）" % DEFAULT_PEER_ID)
    p.add_argument("--spawn", default="64,80", help="出生点 `x,y[,z[,yaw°]]`（默认 64,80）")
    p.add_argument("--legs", default=DEFAULT_LEGS,
                   help="路点表 `x,y@dwell;x,y@dwell;…`（默认 %r）" % DEFAULT_LEGS)
    p.add_argument("--speed", type=float, default=DEFAULT_SPEED, help="线速度 m/s（默认 %.1f）" % DEFAULT_SPEED)
    p.add_argument("--csv", default=None, help="位姿 CSV 输出路径（父目录会自建）")
    p.add_argument("--send-hz", type=float, default=DEFAULT_SEND_HZ, help="发送频率（默认 %.0f）" % DEFAULT_SEND_HZ)
    p.add_argument("--arrive-tol", type=float, default=DEFAULT_ARRIVE_TOL,
                   help="到点容差 m（默认 %.2f）" % DEFAULT_ARRIVE_TOL)
    p.add_argument("--turn-threshold-deg", type=float, default=DEFAULT_TURN_THRESHOLD_DEG,
                   help="超过该误差先原地转（默认 %.0f°）" % DEFAULT_TURN_THRESHOLD_DEG)
    p.add_argument("--duration", type=float, default=DEFAULT_DURATION,
                   help="总时长上限 s（走完路点后保持连接；默认 %.0f）" % DEFAULT_DURATION)
    p.add_argument("--quiet", action="store_true", help="不打印每秒状态行")
    return p


def main(argv=None):
    args = _build_parser().parse_args(argv)
    try:
        legs = parse_legs(args.legs)
        spawn = parse_spawn_spec(args.spawn)
    except ValueError as exc:
        print("[error] %s" % exc, flush=True)
        return EXIT_PROTOCOL
    if args.speed < 0.0:
        print("[error] --speed 不能为负：%r" % (args.speed,), flush=True)
        return EXIT_PROTOCOL

    signal.signal(signal.SIGINT, _On_Signal)
    signal.signal(signal.SIGTERM, _On_Signal)

    client = PedestrianClient(url=args.url, peer_id=args.peer_id, spawn=spawn,
                              send_hz=args.send_hz, verbose=False)
    try:
        client.connect()
    except SpawnFallback as exc:
        print("[error] 世界把 spawn 重定位了（本脚本**不允许**回退）：%s" % exc, flush=True)
        return 5
    except ClientError as exc:
        print("[error] %s" % exc, flush=True)
        return getattr(exc, "exit_code", EXIT_PROTOCOL)
    except (OSError, ProtocolError) as exc:
        print("[error] 连接失败：%s" % exc, flush=True)
        return EXIT_PROTOCOL

    print("[ped] ready peer='%s' spawn=(%.2f, %.2f) legs=%d speed=%.2f engine=%s"
          % (args.peer_id, float(client.spawn.get("x", 0.0)), float(client.spawn.get("y", 0.0)),
             len(legs), args.speed, client.engine_name), flush=True)

    csv_file = csv_writer = None
    if args.csv:
        try:
            parent = os.path.dirname(os.path.abspath(args.csv))
            if parent:
                os.makedirs(parent, exist_ok=True)
            csv_file = open(args.csv, "w", encoding="utf-8", newline="")
            csv_writer = csv.writer(csv_file)
            csv_writer.writerow(["t_wall", "world_time", "x", "y", "z", "yaw_deg", "v", "leg", "phase"])
        except OSError as exc:
            print("[error] 打不开 --csv %r：%s" % (args.csv, exc), flush=True)
            client.close()
            return EXIT_PROTOCOL

    turn_threshold = math.radians(max(float(args.turn_threshold_deg), 0.0))
    leg_index = 0
    phase = "walk"                    # walk | dwell | done
    dwell_until = 0.0
    phase_since = 0.0
    seen_poses = 0
    cmd_v = 0.0
    last_x = last_y = None
    last_target_dir = float(client.spawn.get("yaw", 0.0))
    t0 = time.monotonic()
    next_log = t0 + 1.0
    exit_code = EXIT_OK

    def log_leg_arrive(idx, elapsed):
        print("[ped] leg=%d arrive t=%.2fs pos=(%.2f, %.2f)"
              % (idx, elapsed, last_x if last_x is not None else 0.0,
                 last_y if last_y is not None else 0.0), flush=True)

    try:
        while not _INTERRUPTED:
            elapsed = time.monotonic() - t0
            if elapsed >= args.duration:
                break
            try:
                client.pump(0.0)
            except ProtocolError as exc:
                print("[error] %s" % exc, flush=True)
                exit_code = EXIT_PROTOCOL
                break

            pose = client.pose()
            if pose is None:
                # 刚 ready、pose 还没到（≤100 ms）：先用出生点当起点
                x = float(client.spawn.get("x", 0.0))
                y = float(client.spawn.get("y", 0.0))
                yaw = float(client.spawn.get("yaw", 0.0))
            else:
                x = float(pose.get("x", 0.0))
                y = float(pose.get("y", 0.0))
                yaw = float(pose.get("yaw", 0.0))

            # ---- 状态推进：走 → 到点 → 驻留 → 下一个
            if phase == "walk":
                tx, ty, dwell_s = legs[leg_index]
                dx, dy = tx - x, ty - y
                if math.hypot(dx, dy) <= max(float(args.arrive_tol), 0.0):
                    log_leg_arrive(leg_index, elapsed)
                    if dwell_s > 0.0:
                        phase = "dwell"
                        dwell_until = time.monotonic() + dwell_s
                        phase_since = time.monotonic()
                        print("[ped] leg=%d dwell start t=%.2fs dwell=%.1fs"
                              % (leg_index, elapsed, dwell_s), flush=True)
                    elif leg_index + 1 < len(legs):
                        leg_index += 1
                        print("[ped] leg=%d start t=%.2fs" % (leg_index, elapsed), flush=True)
                    else:
                        phase = "done"
                        print("[ped] done t=%.2fs pos=(%.2f, %.2f)" % (elapsed, x, y), flush=True)
                else:
                    last_target_dir = math.atan2(dy, dx)
                    err = Wrap_Pi(last_target_dir - yaw)
                    if abs(err) > turn_threshold:
                        cmd_v = 0.0                     # 原地转：先把朝向掰过来再走
                        client.drive(cmd_v, last_target_dir)
                    else:
                        cmd_v = args.speed
                        client.drive(cmd_v, last_target_dir)
            elif phase == "dwell":
                cmd_v = 0.0
                client.drive(cmd_v, None)               # 站住，节拍照旧（世界 300 ms 超时）
                if time.monotonic() >= dwell_until:
                    print("[ped] leg=%d dwell end t=%.2fs (%.1fs)"
                          % (leg_index, time.monotonic() - t0,
                             time.monotonic() - phase_since), flush=True)
                    if leg_index + 1 < len(legs):
                        leg_index += 1
                        phase = "walk"
                        print("[ped] leg=%d start t=%.2fs"
                              % (leg_index, time.monotonic() - t0), flush=True)
                    else:
                        phase = "done"
                        print("[ped] done t=%.2fs pos=(%.2f, %.2f)"
                              % (time.monotonic() - t0, x, y), flush=True)
            else:                                            # done：站住等 duration
                cmd_v = 0.0
                client.drive(cmd_v, None)

            # ---- 采样（新 pose 才写磁盘；放在决策后，CSV 里带本帧的 v）
            if pose is not None and client.poses_received != seen_poses:
                seen_poses = client.poses_received
                last_x, last_y = x, y
                if csv_writer is not None:
                    csv_writer.writerow([
                        "%.3f" % time.time(),
                        "%.4f" % float(pose.get("world_time", 0.0)),
                        "%.4f" % x, "%.4f" % y, "%.4f" % float(pose.get("z", 0.0)),
                        "%.2f" % math.degrees(yaw),
                        "%.3f" % cmd_v,
                        leg_index,
                        phase,
                    ])
                    csv_file.flush()

            now = time.monotonic()
            if not args.quiet and now >= next_log:
                next_log += 1.0
                print("[ped] t=%.1fs leg=%d/%d phase=%s pos=(%.2f, %.2f) yaw=%.1f° v=%.2f poses=%d"
                      % (now - t0, leg_index + 1, len(legs), phase, x, y, math.degrees(yaw),
                         cmd_v, client.poses_received), flush=True)

            client.tick()
            client.sleep_to_next()
    except KeyboardInterrupt:
        exit_code = EXIT_INTERRUPTED
        print("[warn] Ctrl-C：先发 v=0 再退出", flush=True)
    except ProtocolError as exc:
        print("[error] %s" % exc, flush=True)
        exit_code = EXIT_PROTOCOL
    finally:
        if _INTERRUPTED and exit_code == EXIT_OK:
            exit_code = EXIT_INTERRUPTED
        try:
            client.drive(0.0)
        except Exception:                      # 收尾路径不许再抛
            pass
        client.close(quiet=True)
        if csv_file is not None:
            try:
                csv_file.flush()
                csv_file.close()
            except OSError:
                pass
        print("[ped] exit code=%d" % exit_code, flush=True)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
