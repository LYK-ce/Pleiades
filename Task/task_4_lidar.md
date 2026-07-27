# Task 4: LiDAR

> 状态：部分完成（帧组装待修）
> 创建日期：2026-07-20
> 最后更新：2026-07-27

## 目标

为 Robot 模块集成 YDLIDAR Tmini 激光雷达驱动，实现 360° 点云实时采集。

## 实现总结

| 步骤 | 内容 | 状态 |
|:---:|------|:---:|
| 1 | Vendor `tmini-protocol` 4 个文件 | ✅ |
| 2 | 创建 `lidar/mod.rs` — LidarDevice | ✅ |
| 3 | 创建 `LidarState`（独立于 RobotState） | ✅ |
| 4 | 添加 `StartLidarScan` / `StopLidarScan` 命令 | ✅ |
| 5 | `Robot::launch()` 支持可选 LiDAR | ✅ |
| 6 | `dispatch()` 处理 LiDAR 命令 | ✅ |
| 7 | 编译 + 测试验证 | ✅ |
| 8 | 修复帧组装逻辑（零位包触发全帧输出） | ❌ 待实现 |

## 文件结构（当前）

```
Src/Robot/control/device/lidar/
├── mod.rs          ← LidarDevice 驱动（spawn + 命令 + mock）
├── constants.rs    ← Vendor: tmini 协议常量
├── types.rs        ← Vendor: NodeInfo / LaserPoint / LaserScan / ScanPacket
├── parser.rs       ← Vendor: feed_byte 状态机 + parse_points + do_process_simple + 测试
└── checksum.rs     ← Vendor: XOR 校验和 + 测试
```

---

## 🔴 帧组装逻辑缺失（导致 Occupied/Free 闪烁的根因）

### 问题

`mod.rs` 的 RX 回调中，每收到一个数据包就立即调用 `parse_points` + `do_process_simple` 输出为独立帧，覆盖 `LidarState.scan`。

Tmini 一圈 4000 个点分成 ~20 个包（每包 ~200 点），所以每次 `feed_byte` 只返回 ~20 个采样点。

### C++ 正确做法

```
收到包 → 缓存到 m_datas
收到零位包（CT byte bit[0] = 1）→ 触发 parsePoints(m_datas) → 完整 400+ 点一圈 → 清空缓存
```

零位包 = 每圈的第一个数据包，CT 字节 bit[0] = 1。参考 `YDlidarDriver.cpp:parseData()`。

### 影响

- SLAM 每次只看到 ~20 点弧段，而非 400+ 点完整圈
- 帧率从 10Hz 变成 ~200Hz（包率）
- 不同弧段交替覆盖 → Bresenham 射线中，弧段 A 标记的 Occupied 格被弧段 B 的射线穿过标记 Free → 反复横跳

---

## 修复方案

### 涉及文件

| 文件 | 改动 |
|------|------|
| `types.rs` | `ScanPacket` 新增 `zero: bool` 字段 |
| `parser.rs` | `feed_byte` 完整包时将 `state.zero` 写入 `ScanPacket` |
| `mod.rs` | RX 回调：累积包到 buffer → 零位包触发全帧组装；mock 同步改 |

### 实施步骤

**Step 1: `types.rs`** — `ScanPacket` 加 `zero` 字段

```rust
pub struct ScanPacket {
    pub raw: Vec<u8>,
    pub stamp: u64,
    pub zero: bool,  // ← 新增：CT bit[0] 零位标记
}
```

**Step 2: `parser.rs`** — `feed_byte` 完整包时写入 zero 标志

在 `Samples` 分支完成包组装的最后，构造 `ScanPacket` 时传递 `state.zero`：

```rust
state.packets.push(ScanPacket {
    raw: state.buf.clone(),
    stamp: state.stamp,
    zero: state.zero,  // ← 从 ParseState 传递
});
```

**Step 3: `mod.rs`** — RX 回调：包累积 + 零位包触发

核心逻辑（对齐 C++ 顺序：先组装、再 push 零位包）：

```rust
let mut packet_buf: Vec<ScanPacket> = Vec::new();
let mut last_zero = Instant::now();

// RX 回调中：
for &b in bytes {
    if feed_byte(&mut sm, b).is_some() {
        let mut got_zero = false;
        for pkt in sm.take_packets() {
            if pkt.raw.len() >= TRI_PACKHEADSIZE && pkt.raw[0] == PH1 && pkt.raw[1] == PH2 {
                if pkt.zero { got_zero = true; }
                packet_buf.push(pkt);
            }
        }

        if got_zero && !packet_buf.is_empty() {
            let nodes = parse_points(&packet_buf, NODE_QUAL8);
            let scan = do_process_simple(&nodes, ...);
            guard.scan = Some(scan);
            packet_buf.clear();
            last_zero = Instant::now();
        }

        // 超时保护：2 秒未收到零位包，清空缓存
        if last_zero.elapsed() > Duration::from_secs(2) {
            packet_buf.clear();
        }
    }
}
```

**Step 4: mock 同步更新**

---

## 已解决的已知问题（子 Agent 对比确认）

| # | 原始标记 | 实际状态 |
|---|------|:---:|
| 1 | `feed_byte` 未调用 `verify_check_sum` | **Orion 已调用**（原始 SDK 缺失） |
| 2 | `check_block_status` 状态残留 | **Orion 已修复**（原始 SDK 仍存在 Bug） |
| 3 | `stop_scan` 忽略 FORCE_STOP 失败 | 仍存在（不影响点云） |

## 测试覆盖

| 测试 | 文件 | 覆盖 |
|------|------|------|
| 校验和往返 | `checksum.rs` | calc_check_sum + verify |
| 状态机解析 | `parser.rs` | feed_byte → 完整包 → parse_points |

## 人类评审

<!-- 在此区域写下评审意见 -->

