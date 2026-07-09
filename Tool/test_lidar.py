#!/usr/bin/env python3
"""
YDLIDAR Tmini 串口测试脚本

启动雷达扫描，实时解析点云并在屏幕上可视化显示。
依赖: pip install pyserial matplotlib numpy

用法:
    python3 test_lidar.py [串口路径]
    python3 test_lidar.py /dev/rplidar
"""

import sys
import struct
import time
import serial
import numpy as np
import matplotlib.pyplot as plt
from matplotlib.animation import FuncAnimation

# ============================================================
# 配置
# ============================================================
DEFAULT_PORT = "/dev/rplidar" if len(sys.argv) < 2 else sys.argv[1]
BAUDRATE = 230400
TIMEOUT = 0.5  # 串口读取超时 (秒)

# ============================================================
# 协议常量
# ============================================================
SYNC_BYTE = 0xA5
DATA_HEAD1 = 0x55
DATA_HEAD2 = 0xAA

CMD_SCAN = 0x60       # 开始扫描
CMD_STOP = 0x65       # 停止扫描
CMD_FORCE_SCAN = 0x61 # 强制扫描
CMD_DEVICE_INFO = 0x90
CMD_HEALTH = 0x92

RESP_TYPE_DEVICE_INFO = 0x04
RESP_TYPE_HEALTH = 0x06
RESP_TYPE_SCAN = 0x81

# 点云缓存（一圈的极坐标点）
polar_points = []  # [(angle_rad, dist_m), ...]
latest_scan = []   # 最新一圈的 (x, y) 笛卡尔坐标
scan_lock = False  # 防止动画回调与串口线程竞争


def open_lidar(port):
    """打开串口"""
    print(f"[INFO] 打开串口: {port}, 波特率: {BAUDRATE}")
    ser = serial.Serial(port, BAUDRATE, timeout=TIMEOUT)
    print(f"[INFO] 串口已打开")
    return ser


def send_cmd(ser, cmd):
    """发送 2 字节命令"""
    frame = bytes([SYNC_BYTE, cmd])
    ser.write(frame)
    print(f"[CMD] 发送: {frame.hex(' ')}")


def read_response_header(ser):
    """
    读取应答帧头: A5 5A size(4B LE) type(1B)
    返回 (size, subtype, type) 或 None
    """
    # 找 A5
    while True:
        b = ser.read(1)
        if not b:
            return None
        if b[0] == SYNC_BYTE:
            break

    # 读 5A
    b = ser.read(1)
    if not b or b[0] != 0x5A:
        print(f"[WARN] 应答头第二字节不是 5A: {b.hex() if b else 'timeout'}")
        return None

    # 读 4 字节 size+subtype
    data = ser.read(4)
    if len(data) < 4:
        print(f"[WARN] 应答头数据不足")
        return None

    size_and_sub = struct.unpack('<I', data)[0]
    size = size_and_sub & 0x3FFFFFFF
    subtype = size_and_sub >> 30

    # 读 type
    b = ser.read(1)
    if not b:
        return None
    resp_type = b[0]

    return size, subtype, resp_type


def read_device_info(ser):
    """发送获取设备信息命令并解析"""
    send_cmd(ser, CMD_DEVICE_INFO)
    result = read_response_header(ser)
    if result is None:
        print("[ERROR] 未收到设备信息应答")
        return
    size, subtype, resp_type = result
    if resp_type != RESP_TYPE_DEVICE_INFO:
        print(f"[ERROR] 应答类型错误: 0x{resp_type:02X}, 期望 0x04")
        return

    data = ser.read(size)
    if len(data) < 20:
        print("[ERROR] 设备信息数据不完整")
        return

    model = data[0]
    fw_ver = struct.unpack('<H', data[1:3])[0]
    hw_ver = data[3]
    serial_num = data[4:20].decode('ascii', errors='replace').rstrip('\x00')

    print(f"[INFO] 设备信息: 型号={model}, 固件=v{fw_ver}, 硬件=v{hw_ver}, 序列号={serial_num}")


def read_health(ser):
    """查询健康状态"""
    send_cmd(ser, CMD_HEALTH)
    result = read_response_header(ser)
    if result is None:
        print("[ERROR] 未收到健康状态应答")
        return None
    size, subtype, resp_type = result
    if resp_type != RESP_TYPE_HEALTH:
        print(f"[ERROR] 应答类型错误: 0x{resp_type:02X}")
        return None

    data = ser.read(size)
    if len(data) < 3:
        return None
    status = data[0]
    error_code = struct.unpack('<H', data[1:3])[0]
    status_text = {0: "正常", 1: "警告", 2: "错误"}.get(status, f"未知({status})")
    print(f"[INFO] 健康状态: {status_text}, 错误码={error_code}")
    return status == 0


def parse_scan_packet(data, count):
    """解析一个数据包中的点，返回 [(angle_deg, dist_m), ...]"""
    if len(data) < 10 + count * 2:
        return []

    # 包头: 55 AA (2B), CT(1B), COUNT(1B), firstAngle(2B), lastAngle(2B), CS(2B)
    ct = data[2]
    first_angle_q64 = struct.unpack('<H', data[4:6])[0]
    last_angle_q64 = struct.unpack('<H', data[6:8])[0]

    # 解析每个点
    points = []
    first_deg = first_angle_q64 / 64.0
    last_deg = last_angle_q64 / 64.0

    if count == 1:
        angle_deg = first_deg
        raw = struct.unpack('<H', data[10:12])[0]
        dist_mm = (raw & 0xFFFC) / 4.0
        quality = raw & 0x0003
        if quality == 0:
            points.append((angle_deg, dist_mm / 1000.0))
    else:
        interval = (last_deg - first_deg) / (count - 1)
        for i in range(count):
            angle_deg = first_deg + interval * i
            raw = struct.unpack('<H', data[10 + i * 2: 12 + i * 2])[0]
            dist_mm = (raw & 0xFFFC) / 4.0
            quality = raw & 0x0003
            if quality == 0 and dist_mm > 0:
                points.append((angle_deg, dist_mm / 1000.0))

    return points, (ct & 0x01) == 1  # is_circle_start


def find_packet_header(ser):
    """在串口流中找 55 AA 包头"""
    while True:
        b = ser.read(1)
        if not b:
            return None
        if b[0] == DATA_HEAD1:
            b2 = ser.read(1)
            if b2 and b2[0] == DATA_HEAD2:
                return True
    return None


def scan_loop(ser):
    """主扫描循环：接收数据包，累积一周的点云"""
    global polar_points, latest_scan, scan_lock

    print("[INFO] 开始扫描...")
    send_cmd(ser, CMD_SCAN)

    # 等待扫描应答头
    result = read_response_header(ser)
    if result is None:
        print("[ERROR] 未收到扫描应答")
        return
    size, subtype, resp_type = result
    if resp_type != RESP_TYPE_SCAN:
        print(f"[ERROR] 应答类型不是扫描数据: 0x{resp_type:02X}")
        return
    print(f"[INFO] 收到扫描应答, 开始接收数据...")

    circle_count = 0
    points_this_circle = []
    start_time = time.time()

    try:
        while True:
            # 找下一个包
            if not find_packet_header(ser):
                continue

            # 读包头剩余 8 字节 (CT + COUNT + firstAngle + lastAngle + CS)
            header = ser.read(8)
            if len(header) < 8:
                break

            ct = header[0]
            count = header[1]
            first_angle = struct.unpack('<H', header[2:4])[0]
            last_angle = struct.unpack('<H', header[4:6])[0]

            # 合理性检查
            if count == 0 or count > 200:
                continue

            # 读距离数据
            node_data = ser.read(count * 2)
            if len(node_data) < count * 2:
                break

            # 拼接完整包
            full_packet = b'\x55\xAA' + header + node_data
            points, is_circle_start = parse_scan_packet(full_packet, count)
            points_this_circle.extend(points)

            if is_circle_start and points_this_circle:
                circle_count += 1
                elapsed = time.time() - start_time

                # 更新全局变量（加锁保护）
                scan_lock = True
                latest_scan = []
                for angle_deg, dist_m in points_this_circle:
                    angle_rad = np.radians(angle_deg)
                    x = dist_m * np.cos(angle_rad)
                    y = dist_m * np.sin(angle_rad)
                    latest_scan.append((x, y))
                scan_lock = False

                freq = circle_count / elapsed if elapsed > 0 else 0
                print(f"[SCAN] 第 {circle_count} 圈: {len(points_this_circle)} 点, "
                      f"频率={freq:.1f} Hz, 时间={elapsed:.1f}s")

                points_this_circle = []

    except KeyboardInterrupt:
        pass
    finally:
        print("[INFO] 停止扫描...")
        send_cmd(ser, CMD_STOP)
        time.sleep(0.1)


def main():
    port = DEFAULT_PORT
    print(f"YDLIDAR Tmini 测试脚本")
    print(f"串口: {port}, 波特率: {BAUDRATE}")
    print("=" * 50)

    # 打开串口
    try:
        ser = open_lidar(port)
    except Exception as e:
        print(f"[ERROR] 无法打开串口: {e}")
        return 1

    # 查询设备信息
    read_device_info(ser)

    # 查询健康状态
    if not read_health(ser):
        print("[WARN] 雷达健康状态异常，继续尝试扫描...")

    # 启动扫描（在主线程中运行，同时显示可视化）
    print("[INFO] 启动可视化窗口...")
    print("[INFO] 按 Ctrl+C 退出")

    # 在后台线程中运行扫描
    import threading
    scan_thread = threading.Thread(target=scan_loop, args=(ser,), daemon=True)
    scan_thread.start()

    # 主线程：matplotlib 可视化
    try:
        run_visualization()
    except KeyboardInterrupt:
        pass
    finally:
        ser.close()
        print("[INFO] 串口已关闭")
    return 0


def run_visualization():
    """matplotlib 实时点云可视化"""
    fig, ax = plt.subplots(figsize=(8, 8))
    ax.set_xlim(-6, 6)
    ax.set_ylim(-6, 6)
    ax.set_xlabel("X (m)")
    ax.set_ylabel("Y (m)")
    ax.set_title("YDLIDAR Tmini — 实时点云")
    ax.grid(True)
    ax.set_aspect('equal')

    # 机器人位置（原点）
    (robot_dot,) = ax.plot([0], [0], 'ro', markersize=10, label='Robot')
    (scatter,) = ax.plot([], [], 'g.', markersize=1, alpha=0.6)
    ax.legend()

    def update(frame):
        global latest_scan, scan_lock
        if not scan_lock and latest_scan:
            pts = latest_scan
            if pts:
                xs = [p[0] for p in pts]
                ys = [p[1] for p in pts]
                scatter.set_data(xs, ys)
        return scatter, robot_dot

    ani = FuncAnimation(fig, update, interval=100, blit=True, cache_frame_data=False)
    plt.show()


if __name__ == "__main__":
    sys.exit(main())
