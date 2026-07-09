#!/usr/bin/env python3
"""
YDLIDAR Tmini 最小测试脚本

跳过设备信息/健康查询，直接打开串口开始扫描。
依赖: pip install pyserial matplotlib numpy
用法: python3 test_lidar.py [/dev/rplidar]
"""

import sys, struct, time, threading
import serial
import numpy as np
import matplotlib.pyplot as plt
from matplotlib.animation import FuncAnimation

PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/rplidar"
BAUD = 230400

latest_scan = []  # 最新一圈的 (x, y)
lock = threading.Lock()


def main():
    print(f"[INFO] 打开串口: {PORT}, {BAUD}")
    ser = serial.Serial(PORT, BAUD, timeout=0.5)

    # 直接开始扫描
    print("[INFO] 发送 A5 60 (开始扫描)")
    ser.write(bytes([0xA5, 0x60]))
    time.sleep(0.1)

    # 读应答头
    buf = b""
    while True:
        b = ser.read(1)
        if not b:
            continue
        buf += b
        # 找 A5 5A ?????? 81
        if len(buf) >= 7 and buf[-7] == 0xA5 and buf[-6] == 0x5A:
            size_and_sub = struct.unpack('<I', buf[-4:-1] + bytes([0]))[0]
            resp_type = buf[-1]
            print(f"[INFO] 应答头: type=0x{resp_type:02X}, size={size_and_sub & 0x3FFFFFFF}")
            if resp_type == 0x81:
                break
            buf = b""

    print("[INFO] 开始接收扫描数据...")
    print("[INFO] 按 Ctrl+C 退出")

    # 扫描线程
    def scan():
        global latest_scan
        circ_pts = []
        while True:
            # 找 55 AA
            while True:
                b = ser.read(1)
                if b and b[0] == 0x55:
                    b2 = ser.read(1)
                    if b2 and b2[0] == 0xAA:
                        break
            # 读包头
            hdr = ser.read(8)
            if len(hdr) < 8:
                continue
            ct, cnt = hdr[0], hdr[1]
            if cnt == 0 or cnt > 200:
                continue
            first = struct.unpack('<H', hdr[2:4])[0]
            last = struct.unpack('<H', hdr[4:6])[0]
            # 读距离
            nodes = ser.read(cnt * 2)
            if len(nodes) < cnt * 2:
                continue
            # 解析点
            interval = (last - first) / max(cnt - 1, 1)
            for i in range(cnt):
                a_q64 = first + interval * i
                a_deg = a_q64 / 64.0
                raw = struct.unpack('<H', nodes[i*2:i*2+2])[0]
                d_mm = (raw & 0xFFFC) / 4.0
                if d_mm > 0:
                    a_rad = np.radians(a_deg)
                    circ_pts.append((d_mm / 1000.0 * np.cos(a_rad),
                                     d_mm / 1000.0 * np.sin(a_rad)))
            if ct & 0x01 and circ_pts:
                with lock:
                    latest_scan = circ_pts
                circ_pts = []

    t = threading.Thread(target=scan, daemon=True)
    t.start()

    # matplotlib
    fig, ax = plt.subplots(figsize=(8, 8))
    ax.set_xlim(-6, 6); ax.set_ylim(-6, 6)
    ax.set_xlabel("X (m)"); ax.set_ylabel("Y (m)")
    ax.set_title("YDLIDAR Tmini - Scan")
    ax.grid(True); ax.set_aspect('equal')
    ax.plot([0], [0], 'ro', markersize=10)
    (sc,) = ax.plot([], [], 'g.', markersize=1, alpha=0.6)

    def update(_):
        with lock:
            pts = latest_scan[:] if latest_scan else []
        if pts:
            sc.set_data([p[0] for p in pts], [p[1] for p in pts])
        return sc,

    ani = FuncAnimation(fig, update, interval=100, blit=True, cache_frame_data=False)
    plt.show()
    ser.close()


if __name__ == "__main__":
    main()
