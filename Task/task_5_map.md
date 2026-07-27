# Task 5: Map

> 状态：实现中（log-odds 三态占据栅格 v2）
> 创建日期：2026-07-21
> 最后更新：2026-07-27

## 目标

'ENDOFFILE' Chunk 占据栅格建图，通过 Robot 的 broadcast channel 推送到 Pictor 可视化。

## 当前状态

- `map_delta`: 200ms 增量发送 ✅
- `map_full`: WebSocket 连接时发送 ✅
- LiDAR 帧组装（零位包触发全帧） ✅
- log-odds 概率栅格 v1（二态，偏激进） ✅ 已推送
- **log-odds 概率栅格 v2（三态，对标 Cartographer）** ❌ 待实现

---

## 🔴 二值栅格闪烁问题

### 现象

import json Occupied/Free 反复横跳，即使 ROS2 rviz2 点云本身是稳定的。

### 根因

import json `set(gx, gy, 0/1)` 直接覆盖，无法容忍传感器噪声。Tmini 距离抖动 ±1~3cm 在 0.5m 栅格上足够让 Bresenham 终点跳相邻格。

---

## log-odds 概率占据栅格 v2（三态）

### 对标

Google Cartographer、ROS GMapping 的标准做法。

### 参数

| 参数 | 值 | 说明 |
|------|:---:|------|
| `OCCUPIED_INCREMENT` | +3 | 每次命中 +3 |
| `FREE_DECREMENT` | -2 | 每次穿过 -2 |
| `OCCUPIED_CLAMP` | +30 | Occupied 夹断上限 |
| `FREE_CLAMP` | -20 | Free 夹断下限 |
| `OCCUPIED_THRESHOLD` | >10 | 越过此值 → Occupied |
| `FREE_THRESHOLD` | <-10 | 越过此值 → Free |
| 中间 [-10, +10] | Unknown | 证据不足 |

### 为什么上下限不对称（+30 vs -20）

- Occupied 上限更高：墙壁 4 次观测（+3×4=12>10）即确认，多出的余量让它更抗噪声
- Free 下限较浅：空地 6 次观测（-2×6=-12<-10）即确认，-20 的深度让它较容易被新障碍物推翻

### 三态判定

```rust
fn log_to_state(log: i8) -> u8 {
    if log > OCCUPIED_THRESHOLD {                // > +10
        CellState::Occupied as u8
    } else if log < FREE_THRESHOLD {             // < -10
        CellState::Free as u8
    } else {
        CellState::Unknown as u8                 // [-10, +10]
    }
}
```

 0 落在 [-10, +10] → Unknown。

### 更新逻辑

```rust
if occupied {
    cell.saturating_add(OCCUPIED_INCREMENT).min(OCCUPIED_CLAMP)
} else {
    cell.saturating_sub(FREE_DECREMENT).max(FREE_CLAMP)
}
```

### 数值示例

| 场景 | 演化 | 结果 |
|------|------|:---:|
| 空地（初始 0） | 0→-2→-4→-6→-8→-10→**-12** | Free（6圈≈0.6s） |
| 墙壁 | 0→+3→+6→+9→**+12** | Occupied（4圈≈0.4s） |
| 墙壁 1 帧漏打 | +30→+28 | Occupied（未变） |
| 人突然站空地前（从-20） | -20→...→**+11** | Occupied（11圈≈1.1s） |

---

## 实现步骤

### 涉及文件

| 文件 | 改动 |
|------|------|
| `slam/grid.rs` | 常量替换；`update()` 加 Free 夹断；`log_to_state()` 三态；`state_bytes()` 初始化 Unknown；测试更新 |
| `slam/lidar_mapper.rs` | 无需改（`update()`/`state()` API 不变） |

### Step 1-3: grid.rs 逻辑

import json`+3 / -2 / +30 / -20 / >10 / <-10`
import json `.max(FREE_CLAMP)` 下限
import json `log_to_state()`

### Step 4: state_bytes 初始化

 `CellState::Unknown as u8`

### Step 5: 测试

| 测试 | 变更 |
|------|------|
| `test_chunk_initial_state` | 期望 Unknown |
| `test_probabilistic_update` | 围值适配 |
| `test_saturation` | 夹断 +30/-20 |
| 新增 `test_threshold_boundary` | 边界 ±10/±11 |

---

## 地图规格

| 项目 | 值 |
|------|------|
| Chunk | 256×256 cells |
| 分辨率 | 0.5m/cell |
| cell 类型 | `i8` [-20, +30] |
| 初始 | 0（Unknown） |

## 已知限制

- 不维护协方差
- 仅单 Chunk
- Pictor 客户端是否支持三态 Unknown（待确认）

## 人类评审


<!-- 在此区域写下评审意见 -->
