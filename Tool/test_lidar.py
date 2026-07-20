#!/usr/bin/env python3
"""
YDLIDAR Tmini 测试 — 严格参考 C++ YDlidarDriver::waitPackage 逻辑
用法: python3 test_lidar.py [/dev/rplidar]
"""

import sys, struct, time, threading
import serial
import numpy as np
import matplotlib.pyplot as plt
from matplotlib.animation import FuncAnimation

PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/rplidar"
BAUD = 230400

# 协议常量 (ydlidar_protocol.h)
PH1 = 0xAA
PH2 = 0x55
LIDAR_CMD_SYNC_BYTE = 0xA5
LIDAR_CMD_SCAN       = 0x60
LIDAR_CMD_FORCE_STOP = 0x00
LIDAR_CMD_STOP       = 0x65
LIDAR_ANS_SYNC_BYTE1 = 0xA5
LIDAR_ANS_SYNC_BYTE2 = 0x5A

latest_scan = []
lock = threading.Lock()


def send_cmd(ser, cmd):
    ser.write(bytes([LIDAR_CMD_SYNC_BYTE, cmd]))
    print(f"[CMD] A5 {cmd:02X}")


def main():
    ser = serial.Serial(PORT, BAUD, timeout=0.5)
    print(f"[INFO] 打开 {PORT}")

    ser.setDTR(True)
    time.sleep(0.3)

    send_cmd(ser, LIDAR_CMD_FORCE_STOP)
    time.sleep(0.05)
    send_cmd(ser, LIDAR_CMD_STOP)
    time.sleep(0.1)
    ser.reset_input_buffer()

    send_cmd(ser, LIDAR_CMD_SCAN)

    # 跳过应答头，直接找数据
    print("[INFO] 等待扫描数据...")
    raw_start = time.time()
    raw_buf = bytearray()
    # 先在终端看看来了什么字节 (from raw debug approach)
    while time.time() - raw_start < 2:
        b = ser.read(1)
        if b:
            raw_buf.append(b[0])
    if raw_buf:
        print(f"[RAW] 前2秒收到 {len(raw_buf)} 字节:")
        for i in range(0, min(100, len(raw_buf)), 20):
            line = raw_buf[i:i+20]
            hx = ' '.join(f'{b:02X}' for b in line)
            asc = ''.join(chr(b) if 32<=b<127 else '.' for b in line)
            print(f"  {i:04X}: {hx:<58s} {asc}")

    # ──── 数据接收线程 ────
    def scan_loop():
        global latest_scan
        buf = bytearray()
        circ_pts = []
        prev_last = -1
        pkt_cnt = 0

        while True:
            # 收一批字节
            chunk = ser.read(512)
            if not chunk:
                continue
            buf.extend(chunk)
            if len(buf) > 8192:
                buf = buf[-4096:]  # 防内存溢出

            # 在缓冲区里找 AA 55
            pos = 0
            while pos < len(buf) - 1:
                if buf[pos] != PH1 or buf[pos+1] != PH2:
                    pos += 1
                    continue

                # 需要 10 字节包头 + 至少 2 字节数据
                if pos + 12 > len(buf):
                    break

                ct    = buf[pos+2]
                cnt   = buf[pos+3]
                first = struct.unpack('<H', bytes(buf[pos+4:pos+6]))[0]
                last  = struct.unpack('<H', bytes(buf[pos+6:pos+8]))[0]
                # cs    = struct.unpack('<H', bytes(buf[pos+8:pos+10]))[0]

                if cnt == 0 or cnt > 80:
                    pos += 1
                    continue

                data_start = pos + 10
                data_end   = data_start + cnt * 2
                if data_end > len(buf):
                    break

                pkt_cnt += 1
                node_data = buf[data_start:data_end]
                interval = (last - first) / max(cnt - 1, 1)

                for i in range(cnt):
                    angle_q64 = first + interval * i
                    raw = struct.unpack('<H', node_data[i*2:i*2+2])[0]
                    qual = raw & 0x0003
                    dist = (raw & 0xFFFC) / 4000.0
                    if qual == 0 and 0 < dist < 6.0:
                        a = np.radians(angle_q64 / 64.0)
                        circ_pts.append((dist * np.cos(a), dist * np.sin(a)))

                # 圈检测：角度回绕
                if prev_last >= 0 and first < prev_last and circ_pts:
                    with lock:
                        latest_scan = circ_pts
                    circ_pts = []
                prev_last = last

                pos = data_end
                if pos > 2048:
                    buf = buf[pos:]
                    pos = 0
                    break

    t = threading.Thread(target=scan_loop, daemon=True)
    t.start()

    # ──── matplotlib ────
    fig, ax = plt.subplots(figsize=(8, 8))
    ax.set_xlim(-6, 6); ax.set_ylim(-6, 6)
    ax.set_xlabel("X (m)"); ax.set_ylabel("Y (m)")
    ax.set_title("YDLIDAR Tmini")
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

    send_cmd(ser, LIDAR_CMD_STOP)
    ser.setDTR(False)
    ser.close()


if __name__ == "__main__":
    main()
