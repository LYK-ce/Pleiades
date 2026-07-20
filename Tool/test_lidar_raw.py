#!/usr/bin/env python3
"""YDLIDAR Tmini 原始串口调试 — 严格 C++ 解析，打印 30 个包"""
import sys, struct, serial, time

PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/rplidar"
ser = serial.Serial(PORT, 230400, timeout=1)

# Tmini: DTR 不控制电机
print("[CMD] A5 60")
ser.write(bytes([0xA5, 0x60]))

# 读应答头
buf = b""
while True:
    b = ser.read(1)
    if not b: continue
    buf += b
    if len(buf) >= 7 and buf[-7] == 0xA5 and buf[-6] == 0x5A:
        s = struct.unpack('<I', buf[-4:])[0]
        t = buf[-1]
        print(f"[RESP] A5 5A ... type=0x{t:02X} size={s & 0x3FFFFFFF}")
        if t == 0x81: break
        buf = b""

print("[INFO] 扫描数据流...\n")

# 解析 30 个包，使用 C++ 的 >>1 角度解析
packets = 0
points_total = 0
all_angles = []

while packets < 30:
    # 找 AA 55
    while True:
        b = ser.read(1)
        if b and b[0] == 0xAA:
            b2 = ser.read(1)
            if b2 and b2[0] == 0x55:
                break
    hdr = ser.read(8)
    if len(hdr) < 8: continue
    ct, cnt = hdr[0], hdr[1]
    if cnt == 0 or cnt > 80: continue

    # C++ 解析: firstAngle = (raw >> 1), 需要先检查 LSB
    first_raw = struct.unpack('<H', hdr[2:4])[0]
    last_raw  = struct.unpack('<H', hdr[4:6])[0]
    cs = struct.unpack('<H', hdr[6:8])[0]

    # C++: if (byte & LIDAR_RESP_CHECKBIT) ... else error
    first_ok = (first_raw & 0x01) != 0
    last_ok  = (last_raw & 0x01) != 0
    first = first_raw >> 1   # C++: FirstSampleAngle >>= 1
    last  = last_raw  >> 1

    nodes = ser.read(cnt * 2)
    if len(nodes) < cnt * 2: continue

    packets += 1
    pts = []
    interval = (last - first) / max(cnt - 1, 1)
    for i in range(cnt):
        a_q64 = first + interval * i  # C++: FirstSampleAngle + IntervalSampleAngle * nodeIndex
        a_deg = a_q64 / 64.0
        if a_deg >= 360: a_deg -= 360
        raw = struct.unpack('<H', nodes[i*2:i*2+2])[0]
        q = raw & 0x0003
        d = (raw & 0xFFFC) / 4000.0  # 三角: dist/4000 = 米
        if d > 0:
            pts.append((a_deg, d, q))
            all_angles.append(a_deg)
    points_total += len(pts)

    circle_mark = "[圈始]" if (ct & 0x01) else ""
    print(f"包#{packets:2d}: CT=0x{ct:02X} {circle_mark} cnt={cnt:2d} "
          f"first={first}({first/64:.1f}°) last={last}({last/64:.1f}°) "
          f"ok={first_ok}/{last_ok} valid={len(pts)}")
    if packets <= 3:
        for a, d, q in pts[:5]:
            print(f"  → a={a:.1f}° d={d:.3f}m q={q}")

if all_angles:
    all_angles.sort()
    print(f"\n[DONE] {packets} 包, {points_total} 有效点")
    print(f"角度范围: {all_angles[0]:.1f}° ~ {all_angles[-1]:.1f}°")
    # 找角度跳变
    for i in range(1, len(all_angles)):
        if all_angles[i] - all_angles[i-1] > 180:
            print(f"角度跳变 @ {all_angles[i-1]:.1f}° → {all_angles[i]:.1f}° (零位)")
            break
else:
    print(f"\n[DONE] {packets} 包, 0 有效点 - 检查解析!")

ser.close()
