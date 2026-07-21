# Task 4: LiDAR

> 状态：已完成
> 创建日期：2026-07-20
> 最后更新：2026-07-21

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

## 文件结构（当前）

```
Src/Robot/control/device/lidar/
├── mod.rs          ← LidarDevice 驱动（spawn + 命令 + mock）
├── constants.rs    ← Vendor: tmini 协议常量
├── types.rs        ← Vendor: NodeInfo / LaserPoint / LaserScan / ScanPacket
├── parser.rs       ← Vendor: feed_byte 状态机 + parse_points + do_process_simple + 测试
└── checksum.rs     ← Vendor: XOR 校验和 + 测试
```

## 状态设计

LiDAR 拥有独立的 `LidarState`（`state.rs`），不与 `RobotState` 共享锁：

```rust
pub struct LidarState {
    pub scan: Option<LaserScan>,
}
```

`LidarDevice::spawn()` 接收 `Arc<RwLock<LidarState>>`，RX 回调通过 `try_write()` 写入 `guard.scan`。

## LidarDevice API

```rust
impl LidarDevice {
    pub fn spawn(port, baudrate, state: Arc<RwLock<LidarState>>) -> Result<Self>;
    pub async fn start_scan(&self) -> Result<(), String>;   // PHA5 + CMD_SCAN
    pub async fn stop_scan(&self) -> Result<(), String>;    // FORCE_STOP → STOP
    pub fn shutdown(&self);
}
```

## 测试覆盖

| 测试 | 文件 | 覆盖 |
|------|------|------|
| 校验和往返 | `checksum.rs` | calc_check_sum + verify |
| 状态机解析 | `parser.rs` | feed_byte → 完整包 → parse_points |

## 已知问题（来自 Code Review）

| # | 问题 | 状态 |
|---|------|:---:|
| 1 | `feed_byte` 未调用 `verify_check_sum` | 待修复 |
| 2 | `check_block_status` 状态残留 | 待修复 |
| 3 | `stop_scan` 忽略 FORCE_STOP 失败 | 待修复 |

## 人类评审

<!-- 在此区域写下评审意见 -->

