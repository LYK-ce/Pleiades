#!/usr/bin/env python3
"""
YDLIDAR Tmini 测试脚本 — 严格参考 C++ YDlidarDriver 实现

依赖: pip install pyserial matplotlib numpy
用法: python3 test_lidar.py [/dev/rplidar]
"""

import sys, struct, time, threading
import serial
import numpy as np
import matplotlib.pyplot as plt
from matplotlib.animation import FuncAnimation

# ── 配置 ────────────────────────────────────
PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/rplidar"
BAUD = 230400

# ── 协议常量 (ydlidar_protocol.h) ───────────
PH  = 0x55AA  # 包头值
PH1 = 0xAA    # AA 55 = 0x55AA 小端
PH2 = 0x55

CT_Normal    = 0
CT_RingStart = 1

LIDAR_CMD_SYNC_BYTE    = 0xA5
LIDAR_CMD_SCAN         = 0x60
LIDAR_CMD_FORCE_STOP   = 0x00
LIDAR_CMD_STOP         = 0x65

LIDAR_ANS_SYNC_BYTE1   = 0xA5
LIDAR_ANS_SYNC_BYTE2   = 0x5A
LIDAR_ANS_TYPE_MEASUREMENT = 0x81

NODE_SYNC   = 1
NODE_UNSYNC = 2
TRI_PACKHEADSIZE = 10
TRI_PACKMAXNODES = 80

# ── 全局 ─────────────────────────────────────
latest_scan = []
lock = threading.Lock()


# ============================================================
# 命令行交互 (from YDlidarDriver::sendCommand)
# ============================================================
def send_cmd(ser, cmd):
    """发送 2 字节命令"""
    ser.write(bytes([LIDAR_CMD_SYNC_BYTE, cmd]))
    print(f"[CMD] A5 {cmd:02X}")


# ============================================================
# 应答头 (from YDlidarDriver::waitResponseHeader)
# ============================================================
def wait_response_header(ser, timeout_ms=1000):
    """等待 A5 5A 应答头，返回 type 或 None"""
    start = time.time()
    buf = bytearray()
    while (time.time() - start) * 1000 < timeout_ms:
        b = ser.read(1)
        if not b:
            continue
        buf.append(b[0])
        if len(buf) < 7:
            continue
        # 从后往前检查
        if buf[-7] == LIDAR_ANS_SYNC_BYTE1 and buf[-6] == LIDAR_ANS_SYNC_BYTE2:
            size_and_sub = struct.unpack('<I', bytes(buf[-4:]))[0]
            resp_type = buf[-1]
            size = size_and_sub & 0x3FFFFFFF
            print(f"[RESP] type=0x{resp_type:02X} size={size}")
            return resp_type
    return None


# ============================================================
# 解析一个包 (from YDlidarDriver::waitPackage)
# ============================================================
class ScanParser:
    """解析扫描数据包，逐点输出 node_info"""
    def __init__(self):
        self.package_index = 0
        self.recv_pos = 0
        self.package_remain = 0
        self.pkg = bytearray(TRI_PACKHEADSIZE + TRI_PACKMAXNODES * 2)

    def feed_byte(self, b):
        """喂一个字节，返回 (angle_q64, dist_mm, is_sync) 或 None"""
        # 找包头 AA 55
        if self.recv_pos == 0:
            if b == PH1:
                self.pkg[0] = b
                self.recv_pos = 1
            return None
        if self.recv_pos == 1:
            if b == PH2:
                self.pkg[1] = b
                self.recv_pos = 2
                self.package_remain = TRI_PACKHEADSIZE - 2
            else:
                self.pkg[0] = b
                self.recv_pos = 1 if b == PH1 else 0
            return None

        # 收包头
        if self.recv_pos >= 2 and self.package_remain > 0:
            self.pkg[self.recv_pos] = b
            self.recv_pos += 1
            self.package_remain -= 1
            if self.package_remain > 0:
                return None

        # 包头收完，解析
        if self.recv_pos >= TRI_PACKHEADSIZE - 1 and self.package_remain == 0:
            # 解析 count
            ct = self.pkg[2]
            count = self.pkg[3]
            if count == 0 or count > TRI_PACKMAXNODES:
                self.recv_pos = 0
                return None

            first_angle = struct.unpack('<H', bytes(self.pkg[4:6]))[0]
            last_angle  = struct.unpack('<H', bytes(self.pkg[6:8]))[0]
            # cs = struct.unpack('<H', bytes(self.pkg[8:10]))[0]

            self.node_count = count
            self.node_first_angle = first_angle
            self.node_last_angle  = last_angle
            self.node_ct = ct
            self.node_index = 0
            self.nodes_pos = TRI_PACKHEADSIZE

            # 需要收 count * 2 字节距离数据
            self.package_remain = count * 2
            self.node_buf = bytearray(self.package_remain)
            self.node_buf_pos = 0
            self.recv_pos += 1

        # 收距离数据
        if self.recv_pos > TRI_PACKHEADSIZE and self.package_remain > 0:
            self.node_buf[self.node_buf_pos] = b
            self.node_buf_pos += 1
            self.package_remain -= 1
            self.recv_pos += 1
            if self.package_remain > 0:
                return None

            # 所有数据收完，开始逐点输出
            count = self.node_count
            first = self.node_first_angle
            last  = self.node_last_angle
            is_sync = (self.node_ct & CT_RingStart) != 0

            # 计算角度间隔
            interval = 0.0
            if count > 1:
                interval = (last - first) / (count - 1)

            results = []
            for i in range(count):
                angle_q64 = first + interval * i
                raw = struct.unpack('<H', bytes(self.node_buf[i*2:i*2+2]))[0]
                results.append((angle_q64, raw, is_sync and i == 0))

            self.recv_pos = 0  # 准备收下一个包
            return results

        self.recv_pos += 1
        return None


# ============================================================
# 主函数
# ============================================================
def main():
    print(f"[INFO] 打开串口: {PORT}, {BAUD}")
    ser = serial.Serial(PORT, BAUD, timeout=0.5)

    # DTR 拉高 — 电机转 (from startMotor)
    print("[INFO] DTR=True (电机启动)")
    ser.setDTR(True)
    time.sleep(0.3)

    # 强制停止 (from stopScan)
    print("[INFO] 发送强制停止")
    send_cmd(ser, LIDAR_CMD_FORCE_STOP)
    time.sleep(0.05)
    send_cmd(ser, LIDAR_CMD_STOP)
    time.sleep(0.1)

    # 清缓冲 (from flushSerial)
    ser.reset_input_buffer()

    # 开始扫描 (from startScan)
    print("[INFO] 发送 A5 60 (开始扫描)")
    send_cmd(ser, LIDAR_CMD_SCAN)

    # 等应答头（最多 3 秒，超时就跳过直接收数据）
    typ = wait_response_header(ser, 3000)
    if typ == LIDAR_ANS_TYPE_MEASUREMENT:
        print("[INFO] 收到扫描应答")
    elif typ is not None:
        print(f"[WARN] 应答头 type=0x{typ:02X}，尝试继续...")
    else:
        print("[WARN] 超时未收到应答头，直接开始接收...")

    print("[INFO] 开始接收扫描数据...")

    # ── 扫描线程 (from cacheScanData) ──
    def scan_loop():
        global latest_scan
        parser = ScanParser()
        circ_pts = []
        byte_count = 0
        pkt_count = 0
        circle_count = 0
        while True:
            b = ser.read(1)
            if not b:
                continue
            byte_count += 1
            if byte_count % 10000 == 0:
                print(f"[INFO] 已收 {byte_count} 字节, {pkt_count} 包, {circle_count} 圈")
            result = parser.feed_byte(b[0])
            if result is None:
                continue

            pkt_count += 1
            for angle_q64, raw, is_sync in result:
                if raw == 0:
                    continue
                qual = raw & 0x0003
                dist = raw & 0xFFFC
                if qual != 0:
                    continue
                dist_m = dist / 4000.0
                if dist_m > 6.0:
                    continue
                angle_rad = np.radians(angle_q64 / 64.0)
                circ_pts.append((dist_m * np.cos(angle_rad),
                                 dist_m * np.sin(angle_rad)))

            if is_sync and circ_pts:
                circle_count += 1
                with lock:
                    latest_scan = circ_pts
                circ_pts = []

    t = threading.Thread(target=scan_loop, daemon=True)
    t.start()

    # ── matplotlib 可视化 ──
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

    # 清理
    print("[INFO] 停止")
    send_cmd(ser, LIDAR_CMD_STOP)
    ser.setDTR(False)
    ser.close()


if __name__ == "__main__":
    main()
