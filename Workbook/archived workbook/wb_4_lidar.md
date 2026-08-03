# Workbook — Task 4: LiDAR

> 对应任务：`Task/task_4_lidar.md`
> 创建日期：2026-07-20

---

## 背景调研（2026-07-20）

### 现有轮子

`/vepfs-mlp2/c20250205/240804016/Workspace/YDLidar-SDK/rust/` 含两个 crate：

| Crate | 用途 | 依赖 | 可用性 |
|-------|------|------|:---:|
| `tmini-protocol` | 纯协议解析（状态机+点云解码） | **零外部依赖** | ✅ 直接 Vendor |
| `tmini-driver` | C FFI 驱动（cdylib .so） | `serial2` (sync) | ❌ async 不兼容 |

### 协议要点

- 串口: 230400 bps, 8N1
- 命令帧: `0xA5` + 命令码（CMD_SCAN=0x60, CMD_STOP=0x65, CMD_FORCE_STOP=0x00）
- 数据帧: `0xAA 0x55` + CT + LSN + FSA(2B) + LSA(2B) + CS(2B) + samples
- 强度位: NODE_QUAL8（1B qual + 2B dist = 3B/点）
- 采样率: 4K, 单包最大 80 点 → 包最大 ~250B

### 与 Orion 的集成方式

- 复用 `spawn_port()` — TX+RX tokio task
- RX 回调用 tmini 状态机（`feed_byte`）替代 STM32 自定义状态机
- 状态写回用 `try_write()`（与 Task 2 修复后的模式一致）
- 命令通过 `mpsc::Sender<Vec<u8>>` 发送原始字节

---

## 实现计划（2026-07-20）

### 新增文件

| 文件 | 来源 | 说明 |
|------|------|------|
| `control/device/lidar/mod.rs` | 新建 | `LidarDevice` 驱动 + re-export |
| `control/device/lidar/constants.rs` | Vendor | tmini 协议常量 |
| `control/device/lidar/types.rs` | Vendor | NodeInfo / LaserPoint / LaserScan |
| `control/device/lidar/parser.rs` | Vendor | feed_byte 状态机 + parse_points + do_process_simple |
| `control/device/lidar/checksum.rs` | Vendor | XOR 校验和 |

### 修改文件

| 文件 | 改动 |
|------|------|
| `state.rs` | + `lidar_scan: Option<LaserScan>` |
| `core/command.rs` | + `StartLidarScan` / `StopLidarScan` |
| `core/robot.rs` | `launch()` 支持可选 LiDAR; `dispatch()` 新增 LiDAR 分支 |
| `control/device/mod.rs` | + `pub mod lidar;` |
| `main.rs` | 传入 LiDAR 串口参数 |

### LidarDevice API

- `spawn(port, baudrate, state) -> Result<Self>`
- `start_scan() -> Result<()>`
- `stop_scan() -> Result<()>`
- `shutdown()`

