#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# 矩形 / 折线巡逻示例（task_35 §二 Step 4 / COMPONENTS.md §8.1）：
#   路点 -> 方位角 -> 恒定 v，用 `PedestrianClient` 以 20~50 Hz 持续发 motion_cmd{v, dir}。
#
# 每帧的决策（刻意做到最简单，**不含任何避障**）：
#   误差 > --turn-threshold-deg -> v=0 原地转（避免"边走边大角度纠正"画出大弧）
#   否则                        -> v=--speed 边走边微调朝向
#   到点（< --arrive-tol）       -> 下一个路点；--loop 取模，否则停在最后一个点（v=0）
#   人不动了怎么办：改航点前先看 World.gd 的 OBSTACLES（默认矩形离最近障碍 12.17 m）。
#
# `--linger` 是验收「停发 300 ms 后世界自动 v->0」的专用开关：驱动结束后**不再发送**
#   （也不发 v=0、不关连接），只读 pose，统计最后一条位移的时刻与之后的漂移。
#
# 用法：
#   python3 patrol.py --duration 10 --speed 1.0 --peer-id patrol-01 \
#       --csv /path/to/pose.csv
#   python3 patrol.py --duration 3 --linger 1.5 --quiet        # 300 ms 超时停车验证
#
# 退出码与 pedestrian_client 一致：0 / 2 连接 / 3 ready / 4 协议 / 5 spawn / 130 Ctrl-C / 6 验证不过。

import argparse
import csv
import math
import os
import signal
import sys
import time

# 同目录直接 import（脚本方式运行时 sys.path[0] 就是本目录；被 -m 调起来时兜一下）
_HERE = __file__.rsplit("/", 1)[0]
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from pedestrian_client import (  # noqa: E402  （本地模块，必须在 sys.path 之后）
    DEFAULT_SEND_HZ, DEFAULT_URL, EXIT_INTERRUPTED, EXIT_OK, EXIT_PROTOCOL, EXIT_SPAWN,
    EXIT_VERIFY, POSE_STALE_S, READY_TIMEOUT_S, ClientError, PedestrianClient,
    ProtocolError, SpawnFallback, Wrap_Pi, parse_spawn_spec,
)

# ---------------------------------------------------------------- 常量
DEFAULT_PEER_ID = "patrol-01"
## 默认矩形（世界坐标，米）。选点依据（plan §1 / §9 R13）：
##   四角 (64,56) (76,56) (76,66) (64,66) —— 对 World.OBSTACLES 的 15 个 box 最小净空 12.17 m，
##   离静态人 person_02 4.00 m；出生点 (64,56) 就是世界的行人默认点，无需 spawn 也能对齐。
DEFAULT_WAYPOINTS = "64,56;76,56;76,66;64,66"
DEFAULT_SPEED = 1.0             # m/s（世界 clamp 0~1.26，资产步速也是 1.26）
DEFAULT_DURATION = 20.0
DEFAULT_ARRIVE_TOL = 0.30       # 到点判定（m）
DEFAULT_TURN_THRESHOLD_DEG = 20.0
## 停发后「还在动」的观测窗口：世界超时是 300 ms，pose 是 10 Hz -> 判据放宽到 450 ms。
LINGER_MOVE_WINDOW_S = 0.45
LINGER_DRIFT_LIMIT_M = 0.01
## 位移判据（m）：pose 是 float，1e-4 足够区分「真在走」与「数值噪声」。
MOVE_EPS_M = 1.0e-4

## SIGINT / SIGTERM：处理函数里只置标志，收尾仍在主循环的 finally 里做（不抛异常外溢）。
_INTERRUPTED = False


def _On_Signal(signum, frame) -> None:
    global _INTERRUPTED
    _INTERRUPTED = True


def parse_waypoints(text: str) -> list:
    """`"64,56;76,56;76,66"` -> [(64.0, 56.0), (76.0, 56.0), (76.0, 66.0)]。"""
    points = []
    for chunk in str(text).split(";"):
        chunk = chunk.strip()
        if not chunk:
            continue
        parts = [p.strip() for p in chunk.split(",") if p.strip()]
        if len(parts) < 2:
            raise ValueError("路点必须给 x,y：%r" % chunk)
        try:
            points.append((float(parts[0]), float(parts[1])))
        except ValueError as exc:
            raise ValueError("路点里有不是数字的段：%r" % chunk) from exc
    if not points:
        raise ValueError("--waypoints 一个点都没有：%r" % text)
    return points


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="行人折线巡逻示例：路点 -> 方位角 -> 恒定 v（20~50 Hz 持续发送）")
    parser.add_argument("--speed", type=float, default=DEFAULT_SPEED,
                        help="线速度 m/s（世界 clamp 0~1.26）")
    parser.add_argument("--loop", action="store_true",
                        help="走到最后一个路点后回到第一个循环（仍受 --duration 约束）")
    parser.add_argument("--duration", type=float, default=DEFAULT_DURATION,
                        help="从 ready 起算的总时长（秒）")
    parser.add_argument("--peer-id", default=DEFAULT_PEER_ID,
                        help="WS 上的 peer_id（一个 id 一个进程；重连会摘掉旧 avatar）")
    parser.add_argument("--spawn", default=None,
                        help='出生点 "x,y[,z[,yaw_度]]"；缺省不带 spawn = 世界默认点 (64,56)')
    parser.add_argument("--waypoints", default=DEFAULT_WAYPOINTS,
                        help='路点表 "x,y;x,y;..."（世界坐标，米）')
    parser.add_argument("--send-hz", type=float, default=DEFAULT_SEND_HZ,
                        help="motion_cmd 发送频率（硬限 20~50 Hz；世界 300 ms 超时）")
    parser.add_argument("--arrive-tol", type=float, default=DEFAULT_ARRIVE_TOL,
                        help="到点判定（米）")
    parser.add_argument("--turn-threshold-deg", type=float, default=DEFAULT_TURN_THRESHOLD_DEG,
                        help="朝向误差超过该角度就原地转（v=0）")
    parser.add_argument("--csv", default=None,
                        help="pose 采样落盘路径（UTF-8，带表头；目录不存在会自动创建）")
    parser.add_argument("--linger", type=float, default=0.0,
                        help="驱动结束后停发但保持连接的秒数（验证 300 ms 超时停车）")
    parser.add_argument("--url", default=DEFAULT_URL)
    parser.add_argument("--ready-timeout", type=float, default=READY_TIMEOUT_S)
    parser.add_argument("--engine", default="auto", choices=("auto", "websockets", "stdlib"))
    parser.add_argument("--allow-spawn-fallback", action="store_true",
                        help="spawn 被世界回退时降级为 WARNING 继续跑（默认 exit 5）")
    parser.add_argument("--quiet", action="store_true", help="不打每秒一行的进度")
    return parser


def main(argv=None) -> int:
    global _INTERRUPTED
    args = build_parser().parse_args(argv)

    try:
        waypoints = parse_waypoints(args.waypoints)
    except ValueError as exc:
        print("[error] %s" % exc)
        return EXIT_PROTOCOL
    spawn = None
    if args.spawn:
        try:
            spawn = parse_spawn_spec(args.spawn)
        except ValueError as exc:
            print("[error] %s" % exc)
            return EXIT_PROTOCOL

    signal.signal(signal.SIGINT, _On_Signal)
    signal.signal(signal.SIGTERM, _On_Signal)

    try:
        client = PedestrianClient(url=args.url, peer_id=args.peer_id, spawn=spawn,
                                  send_hz=args.send_hz, ready_timeout=args.ready_timeout,
                                  engine=args.engine, verbose=False,
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

    csv_file = None
    csv_writer = None
    if args.csv:
        try:
            parent = os.path.dirname(os.path.abspath(args.csv))
            if parent:
                os.makedirs(parent, exist_ok=True)
            csv_file = open(args.csv, "w", encoding="utf-8", newline="")
            csv_writer = csv.writer(csv_file)
            csv_writer.writerow(["t_wall", "world_time", "x", "y", "z", "yaw_deg",
                                 "target_dir_deg", "v", "wp"])
        except OSError as exc:
            print("[error] 打不开 --csv %r：%s" % (args.csv, exc))
            client.close()
            return EXIT_PROTOCOL

    if not args.quiet:
        print("[patrol] start: waypoints=%d speed=%.2f duration=%.1fs loop=%s peer='%s' engine=%s"
              % (len(waypoints), args.speed, args.duration, args.loop,
                 args.peer_id, client.engine_name))

    turn_threshold = math.radians(max(args.turn_threshold_deg, 0.0))
    distance = 0.0
    wp_index = 0
    exits = 0
    seen_poses = 0
    last_x = None
    last_y = None
    last_target_dir = client.spawn.get("yaw", 0.0)
    held = False                       # 无 --loop 且走完所有点：停在最后一个点
    t0 = time.monotonic()
    next_log = t0 + 1.0
    exit_code = EXIT_OK

    try:
        while not _INTERRUPTED:
            now = time.monotonic()
            elapsed = now - t0
            if elapsed >= args.duration:
                break
            try:
                client.pump(0.0)
            except ProtocolError as exc:
                exits += 1
                print("[error] %s" % exc)
                exit_code = EXIT_PROTOCOL
                break

            pose = client.pose()
            if pose is None:
                # 刚 ready、pose 还没到（≤100 ms）：先用出生点当起点，别把第一帧浪费掉
                x = float(client.spawn.get("x", 0.0))
                y = float(client.spawn.get("y", 0.0))
                yaw = float(client.spawn.get("yaw", 0.0))
            else:
                x = float(pose.get("x", 0.0))
                y = float(pose.get("y", 0.0))
                yaw = float(pose.get("yaw", 0.0))

            # ---- 决策：方位角 -> 原地转 or 边走边微调
            if not held:
                target_x, target_y = waypoints[wp_index]
                dx, dy = target_x - x, target_y - y
                if math.hypot(dx, dy) <= args.arrive_tol:
                    if wp_index + 1 < len(waypoints):
                        wp_index += 1
                    elif args.loop:
                        wp_index = 0
                    else:
                        held = True                 # 停住，等 --duration 走完
                if not held:
                    target_x, target_y = waypoints[wp_index]
                    dx, dy = target_x - x, target_y - y
                    last_target_dir = math.atan2(dy, dx)
                else:
                    last_target_dir = yaw

            if held:
                cmd_v = 0.0
                client.drive(cmd_v, None)           # 走完了：站住，但连接与节拍照旧
            else:
                err = Wrap_Pi(last_target_dir - yaw)
                if abs(err) > turn_threshold:
                    cmd_v = 0.0
                    client.drive(cmd_v, last_target_dir)   # 原地转：先把朝向掰过来再走
                else:
                    cmd_v = args.speed
                    client.drive(cmd_v, last_target_dir)

            # ---- 采样（新 pose 才写磁盘 / 累距离；放在决策后，CSV 里带本帧的 v）
            if pose is not None and client.poses_received != seen_poses:
                seen_poses = client.poses_received
                if last_x is not None:
                    distance += math.hypot(x - last_x, y - last_y)
                last_x, last_y = x, y
                if csv_writer is not None:
                    csv_writer.writerow([
                        "%.3f" % time.time(),
                        "%.4f" % float(pose.get("world_time", 0.0)),
                        "%.4f" % x, "%.4f" % y, "%.4f" % float(pose.get("z", 0.0)),
                        "%.2f" % math.degrees(yaw),
                        "%.2f" % math.degrees(last_target_dir),
                        "%.3f" % cmd_v,
                        wp_index,
                    ])
                    csv_file.flush()

            if not args.quiet and now >= next_log:
                next_log += 1.0
                print("[patrol] t=%.1fs wp=%d/%d pos=(%.2f, %.2f) yaw=%.1f° v=%.2f dir=%.1f° poses=%d"
                      % (elapsed, wp_index + 1, len(waypoints), x, y, math.degrees(yaw),
                         cmd_v, math.degrees(last_target_dir), client.poses_received))

            client.tick()
            client.sleep_to_next()
    except KeyboardInterrupt:
        exit_code = EXIT_INTERRUPTED
        print("[warn] Ctrl-C：先发 v=0 再退出")
    except ProtocolError as exc:
        exits += 1
        print("[error] %s" % exc)
        exit_code = EXIT_PROTOCOL
    finally:
        if _INTERRUPTED and exit_code == EXIT_OK:
            exit_code = EXIT_INTERRUPTED
        # ---- --linger：停发但不断连，量「多久才停」「之后漂不漂」
        if args.linger > 0.0 and exit_code in (EXIT_OK,):
            verify_code = run_linger_check(client, args.linger, args.quiet)
            if verify_code != EXIT_OK:
                exit_code = verify_code
        client.close()
        if csv_file is not None:
            csv_file.close()

    pose = client.pose()
    final = "pos=(%.2f, %.2f)" % (float(pose.get("x", 0.0)), float(pose.get("y", 0.0))) \
        if pose is not None else "pos=(未知)"
    elapsed = time.monotonic() - t0
    print("[patrol] done: duration=%.1fs distance=%.2fm speed=%.1f poses=%d cmds=%d exits=%d %s"
          % (elapsed, distance, args.speed, client.poses_received, client.sent_commands,
             exits, final))
    if client.pose_age() is not None and client.pose_age() > POSE_STALE_S and exit_code == EXIT_OK:
        print("[warn] 最后一条 pose 已过期 %.2fs（世界还在回传吗？）" % client.pose_age())
    return exit_code


def run_linger_check(client: PedestrianClient, linger_s: float, quiet: bool) -> int:
    """停发（不发 v=0、不关连接）后只读 pose：last_move_at 应 ≤ 450 ms、之后漂移 < 1 cm。"""
    t_stop = time.monotonic()
    samples = []
    pose = client.pose()
    if pose is not None:
        samples.append((0.0, float(pose.get("x", 0.0)), float(pose.get("y", 0.0))))
    seen = client.poses_received
    while time.monotonic() - t_stop < linger_s:
        try:
            client.pump(0.02)
        except ProtocolError as exc:
            print("[warn] linger 期间连接断开：%s" % exc)
            break
        if client.poses_received != seen:
            seen = client.poses_received
            pose = client.pose()
            if pose is not None:
                samples.append((time.monotonic() - t_stop,
                                float(pose.get("x", 0.0)), float(pose.get("y", 0.0))))
    if not quiet:
        print("[verify] 停发 %.2fs，收到 %d 条 pose（10 Hz）" % (linger_s, max(len(samples) - 1, 0)))

    last_move_at = 0.0
    for i in range(1, len(samples)):
        t_rel, x, y = samples[i]
        px, py = samples[i - 1][1], samples[i - 1][2]
        if math.hypot(x - px, y - py) >= MOVE_EPS_M:
            last_move_at = max(last_move_at, t_rel)

    drift = 0.0
    base = None
    for t_rel, x, y in samples:
        if t_rel >= LINGER_MOVE_WINDOW_S:
            base = (x, y)
            break
    if base is not None and samples:
        drift = math.hypot(samples[-1][1] - base[0], samples[-1][2] - base[1])

    ok = last_move_at <= LINGER_MOVE_WINDOW_S and drift < LINGER_DRIFT_LIMIT_M
    print("[verify] linger=%.1fs last_move_at=%.2fs drift=%.3fm -> %s"
          % (linger_s, last_move_at, drift, "PASS" if ok else "FAIL"))
    return EXIT_OK if ok else EXIT_VERIFY


if __name__ == "__main__":
    raise SystemExit(main())
