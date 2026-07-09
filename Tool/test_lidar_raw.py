#!/usr/bin/env python3
"""YDLIDAR Tmini 原始串口调试 —— 只打印，不画图"""
import sys, struct, serial

PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/rplidar"
ser = serial.Serial(PORT, 230400, timeout=1)

# 开始扫描
print("[CMD] A5 60")
ser.write(bytes([0xA5, 0x60]))

# 读应答头
buf = b""
while True:
    b = ser.read(1)
    if not b: continue
    buf += b
    if len(buf) >= 7 and buf[-7] == 0xA5 and buf[-6] == 0x5A:
        s = struct.unpack('<I', buf[-4:] if len(buf)>=4 else buf[-4:-1]+bytes([0]))[0]
        t = buf[-1]
        print(f"[RESP] A5 5A ... type=0x{t:02X} size={s & 0x3FFFFFFF}")
        if t == 0x81:
            break
        buf = b""

print("[INFO] 扫描开始，打印前 200 个字节，然后前 10 个数据包...\n")

# 打印前 200 原始字节
raw = ser.read(200)
print(f"[RAW] {len(raw)} 字节:")
for i in range(0, len(raw), 20):
    chunk = raw[i:i+20]
    hex_str = ' '.join(f'{b:02X}' for b in chunk)
    ascii_str = ''.join(chr(b) if 32 <= b < 127 else '.' for b in chunk)
    print(f"  {i:04X}: {hex_str:<58s} {ascii_str}")

# 然后解析前 10 个包
ser.reset_input_buffer()
ser.write(bytes([0xA5, 0x60]))
time = __import__('time')
time.sleep(0.1)
ser.read(100)  # 跳过应答头

print("\n[INFO] 解析数据包...")
packets = 0
points_total = 0
while packets < 10:
    # 找 55 AA
    while True:
        b = ser.read(1)
        if b and b[0] == 0x55:
            b2 = ser.read(1)
            if b2 and b2[0] == 0xAA:
                break
    hdr = ser.read(8)
    if len(hdr) < 8: continue
    ct, cnt = hdr[0], hdr[1]
    if cnt == 0 or cnt > 200: continue
    first = struct.unpack('<H', hdr[2:4])[0]
    last = struct.unpack('<H', hdr[4:6])[0]
    cs = struct.unpack('<H', hdr[6:8])[0]
    nodes = ser.read(cnt * 2)
    if len(nodes) < cnt * 2: continue

    packets += 1
    pts = []
    interval = (last - first) / max(cnt - 1, 1)
    for i in range(cnt):
        a = (first + interval * i) / 64.0
        raw = struct.unpack('<H', nodes[i*2:i*2+2])[0]
        d = (raw & 0xFFFC) / 4000.0  # 米
        q = raw & 0x0003
        if d > 0:
            pts.append((a, d, q))
    points_total += len(pts)

    print(f"包#{packets}: CT=0x{ct:02X} {'[圈始]' if ct&1 else ''}, "
          f"pts={cnt}, angle={first/64:.1f}-{last/64:.1f}deg, "
          f"CS={cs:04X}, valid={len(pts)}")
    if cnt <= 6:
        for a, d, q in pts:
            print(f"  angle={a:.1f}deg  dist={d:.2f}m  quality={q}")

print(f"\n[DONE] {packets} 包, {points_total} 个有效点")
ser.close()
