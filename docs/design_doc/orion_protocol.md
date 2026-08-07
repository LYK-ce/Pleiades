# Orion 统一通信协议（Orion Communication Protocol）

> 创建日期：2026-08-07
> 状态：设计中（消息字段以本文件为准，后续讨论补充其余章节）

---

## 1. 背景与目标

### 1.1 背景

早期单车阶段，小车与控制终端（Pictor / Godot）仅通过 WebSocket 通信，消息格式混用：

- **JSON 文本帧**：`hello` / `pose` / `map_delta` / `cmd`（mode / manual / auto 三层命令）
- **二进制帧**：`map_full`（65545 字节全量栅格，Pictor 协议）

单车单链路下，JSON 与二进制混用只是"不一致"，并非错误——收发两端均为自研代码，各自约定即可。

向**多车集群**演进后，问题放大：

- 车 ↔ 车（libp2p `DataType::Robot`）与车 ↔ 控制终端（WebSocket）**两套协议并存**，字段风格不一（`cmd/action` vs `type/x/y`）
- 集群广播 payload 为散装 JSON（`robot_pose` / `robot_map_delta`），且**无 map_full 二进制通道**（未来二进制地图会损坏于 `from_utf8_lossy`）
- 每新增一种消息，需在两条链路上各写一套结构 + 解析

### 1.2 决策过程（2026-08-07 讨论结论）

| 议题 | 结论 |
|---|---|
| 传输层 | 统一为 **libp2p**（车↔车、车↔控制终端均走现有 P2P 网络；WebSocket 后续退役） |
| 协议方向 | **MAVLink 风格帧 + 按需扩展**：帧结构参考 MAVLink v2，但 len 字段放宽为 4 字节（原始 255B payload 上限为串口时代遗产，无法承载全量栅格地图，予以废弃）；消息定义风格参考 MAVLink，内容全部自定义（`ORION_` 前缀） |
| 身份识别 | 网络层身份（libp2p peer_id）即车辆系统身份（Orion 由网络层系统拓展而来）；sysid 仅作形式字段 |
| 心跳 | **不做**（libp2p 存活检测已覆盖） |
| 命令应答 | **不做**（传输层已有"收到"确认） |

### 1.3 目标

1. **一套协议**：同时覆盖车↔车、车↔控制终端两条链路
2. **形式统一**：帧结构与消息定义风格一致（参考 MAVLink），消除 JSON/二进制混用
3. **大数据无分块**：4 字节 len 使全量地图（65536 cell）单条消息一次传完，协议内不存在"装不下"
4. **贴合自研语义**：位姿、增量地图、命令、任务等全部使用自定义消息（`ORION_` 前缀）
5. **消除历史问题**：map_full 二进制获得规范通道（`ORION_MAP_FULL`）

### 1.4 非目标（第一版明确不做）

- 心跳消息（`HEARTBEAT`）
- 命令应答（`COMMAND_ACK`）
- MISSION 协议（`MISSION_COUNT` / `MISSION_ITEM` 等标准任务握手流程）
- 参数协议（`PARAM_*`）
- 与标准 MAVLink 设备的互操作（帧格式已自定义化，不做兼容承诺）

### 1.5 过渡期说明

Pictor（Godot 地面站）迁移至 libp2p 之前，**WebSocket 链路继续使用现有协议**（含 `hello` 连接握手消息，仅发送一次，成本可忽略）。

- 现有 WS 协议（JSON 命令/遥测 + `map_full` 二进制帧）**保持不变，不迁移、不删除**
- 新协议（本文档）面向 libp2p 统一后的目标形态
- Pictor 完成 libp2p 接入后，WS 链路与 `hello` 一并退役

---

## 2. 帧格式（MAVLink 风格 + 按需扩展）

帧结构参考 MAVLink v2，按需扩展如下：

```
┌─────────┬───────┬─────┬────────┬─────────┬────────┬──────────────┬──────────┐
│  magic  │  len  │ seq │ sysid  │ compid  │ msgid  │   payload    │ checksum │
│  0x4F   │ u32   │ u8  │ u8     │ u8      │ u16    │ 0~4G 字节    │ u16      │
│  (1B)   │       │     │        │         │        │              │          │
└─────────┴───────┴─────┴────────┴─────────┴────────┴──────────────┴──────────┘
```

| 字段 | 长度 | 说明 |
|---|---|---|
| `magic` | 1B | 固定 `0x4F`（'O' = Orion）。自定义标识——len 已非标准，保留 MAVLink `0xFD` 会令标准解析器误认 |
| `len` | 4B | payload 字节数（uint32，上限约 4GB）。**放宽自 MAVLink v2 的 1 字节（255B 上限）**——原始限制是串口时代紧凑设计，无法承载全量栅格地图（65536 cell），予以废弃 |
| `seq` | 1B | **保留字段，第一版恒填 0**——libp2p/TCP 可靠传输，无需丢包检测；未来跑串口/无线等不可靠链路时再启用计数 |
| `sysid` | 1B | 系统 ID（派生规则见 §4） |
| `compid` | 1B | 组件 ID（约定见 §4） |
| `msgid` | 2B | 消息 ID（本条消息类型，见 §3；65536 种，留足扩展空间） |
| `payload` | N | 消息内容（按 msgid 定义解析） |
| `checksum` | 2B | **保留字段，第一版恒填 0**——TCP 已保证数据完整性；未来非可靠链路时实现 CRC16 |

### 与 MAVLink v2 帧的差异汇总

| 项目 | MAVLink v2 | 本协议 |
|---|---|---|
| magic | `0xFD` | `0x4F` |
| len | 1B（≤255） | **4B（≤4G）** |
| incompat/compat flags | 2B | 移除（自研网络无需要） |
| seq / sysid / compid | 各 1B | 各 1B（保留） |
| msgid | 3B | 2B |
| 扩展签名 | 可选 13B | 移除 |

### 第一版说明（2026-08-07）

- `seq` / `checksum` 字段位置保留，**第一版恒填 0**，不实现计数与 CRC16 计算（传输层为 libp2p/TCP，已保证可靠性与完整性，与放宽 len 同一逻辑：不为串口时代的问题付代码成本）
- 未来若帧需要跑串口/无线链路（如直连其他设备），再启用 `seq` 计数与 CRC16 校验

---

## 3. 消息定义

### 消息总览（全部自定义）

| msgid | 消息 | 方向 | 频率/时机 |
|---|---|---|---|
| 1 | `ORION_POSE` | 车 → 集群/终端 | 10Hz 广播 |
| 2 | `ORION_MAP_FULL` | 车 → 集群/终端 | 连接建立时 / 按需 |
| 3 | `ORION_MAP_DELTA` | 车 → 集群/终端 | 地图有变化时（≤5Hz） |
| 4 | `ORION_MANUAL_CONTROL` | 终端/集群 → 车 | 事件驱动 |
| 5 | `ORION_TASK_SET` | 终端/集群 → 车 | 事件驱动 |

> 字段命名遵循 MAVLink 风格（`snake_case`、C 类型）。类型字节序：大端（与 MAVLink 一致）。

### 3.1 ORION_POSE（msgid 1）— 位姿

一次打包坐标 + 速度 + 朝向 + 时间戳（保持单车时代的一次性语义）。

| 字段 | 类型 | 单位 | 说明 |
|---|---|---|---|
| `time_boot_ms` | uint32 | ms | 开机起算时间戳 |
| `x` | float | m | 全局世界坐标 X（x 东） |
| `y` | float | m | 全局世界坐标 Y（y 南） |
| `vx` | float | m/s | X 方向速度 |
| `vy` | float | m/s | Y 方向速度 |
| `yaw` | float | rad | 朝向角，范围 [-π, π]，**顺时针为正** |

**payload 布局**（大端，共 24 字节）：

| 偏移 | 字段 | 类型 |
|---|---|---|
| 0 | `time_boot_ms` | u32 |
| 4 | `x` | f32 |
| 8 | `y` | f32 |
| 12 | `vx` | f32 |
| 16 | `vy` | f32 |
| 20 | `yaw` | f32 |

来源映射：`x/y` 直接映射 `RobotState.{x, y}`（全局世界坐标）、`{vx, vy}`、`attitude.yaw`，**无坐标变换**；`time_boot_ms` 由开机基准时间换算（`now_boot_ms()`，替代原 `Pose.ts` unix 秒，字段类型 f64 → u32）。

> 时间戳语义（2026-08-07 决策）：`time_boot_ms` 为**本机单调时间**（开机起算），仅作数据时间标签/显示/调试用；分布式系统无全局统一时钟，**不做跨车时间比较**（数据新鲜度由 libp2p 超时/心跳判断）。
频率：10Hz（沿用 `state_notifier` 节奏）。

### 3.2 ORION_MAP_FULL（msgid 2）— 地图全量

全量栅格地图，单条消息一次传完（利用 4 字节 len，payload ≈ 65.5KB）。替代原 Pictor 二进制 `map_full` 与标准 `OCCUPANCY_GRID`（后者 180 cell 容量上限无法承载 256×256 地图）。

| 字段 | 类型 | 说明 |
|---|---|---|
| `time_boot_ms` | uint32 | ms |
| `origin_gx` | int32 | 栅格原点全局网格坐标 X |
| `origin_gy` | int32 | 栅格原点全局网格坐标 Y |
| `width` | uint16 | 栅格宽（cell 数），当前 256 |
| `height` | uint16 | 栅格高（cell 数），当前 256 |
| `resolution` | float | 分辨率（米/cell），当前 0.5 |
| `data` | int8[width×height] | 每 cell 一字节：**0 = free, 100 = occupied, 255 = unknown**（与内部一致） |

**payload 布局**（大端）：

| 偏移 | 字段 | 类型 |
|---|---|---|
| 0 | `time_boot_ms` | u32 |
| 4 | `origin_gx` | i32 |
| 8 | `origin_gy` | i32 |
| 12 | `width` | u16 |
| 14 | `height` | u16 |
| 16 | `resolution` | f32 |
| 20 | `data[width×height]` | i8[] |

**总大小 = 20 + width×height 字节**（256×256 时 = 65556 字节）

状态编码：内部三态已统一为 **0/100/255**（`CellState` 枚举 `[repr(i8)]`，`Unknown = -1`，u8 线上为 255），full 与 delta 编码一致，**零映射直传**。

发送时机：控制终端/其他车连接建立后发送一次；后续按需重发（协议层不做主动周期推送）。

### 3.3 ORION_MAP_DELTA（msgid 3）— 地图增量

仅传变化的格子。4 字节 len 下，一帧可承载 13107 项（65535 字节），足以覆盖单次 SLAM 更新（数百~数千项），**无需分帧**。

| 字段 | 类型 | 说明 |
|---|---|---|
| `time_boot_ms` | uint32 | ms |
| `count` | uint16 | 变化格子数 |
| `entries` | 结构数组 | 每项 9 字节（见下） |

`entries[i]`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `gx` | int32 | 全局网格坐标 X（绝对坐标，无需 chunk） |
| `gy` | int32 | 全局网格坐标 Y（绝对坐标，无需 chunk） |
| `state` | int8 | 0 = free, 100 = occupied, 255 = unknown（与内部一致，直传） |

**payload 布局**（大端）：

| 偏移 | 字段 | 类型 |
|---|---|---|
| 0 | `time_boot_ms` | u32 |
| 4 | `count` | u16 |
| 6 | `entries[count]` | 每项 9 字节（见下） |

`entries[i]` 布局（9 字节）：`gx` i32 [0..4) + `gy` i32 [4..8) + `state` i8 [8..9)

**总大小 = 6 + 9×count 字节**

来源映射：`slam::update` 返回的 `Vec<Delta{gx,gy,state}>`。
频率：有变化时发送（沿用 `slam_task` 200ms 节奏，≤5Hz）。

### 3.4 ORION_MANUAL_CONTROL（msgid 4）— 手动命令 + 模式切换

自定义枚举语义（MAVLink 标准 `MANUAL_CONTROL` 为摇杆轴量，与小车"动作+速度"命令不匹配，不采用）。

| 字段 | 类型 | 说明 |
|---|---|---|
| `action` | uint8 | 动作枚举（见下） |
| `param` | int16 | 动作参数：速度档位（forward/backward/spin）或时长 ms（beep）；其余动作填 0 |

**payload 布局**（大端，共 3 字节）：

| 偏移 | 字段 | 类型 |
|---|---|---|
| 0 | `action` | u8 |
| 1 | `param` | i16 |

`action` 枚举：

| 值 | 动作 | param 含义 |
|---|---|---|
| 0 | `forward` | 速度 |
| 1 | `backward` | 速度 |
| 2 | `spin_left` | 速度 |
| 3 | `spin_right` | 速度 |
| 4 | `stop` | — |
| 5 | `beep` | 时长 ms |
| 6 | `start_lidar` | — |
| 7 | `stop_lidar` | — |
| 8 | `switch_to_manual` | — |
| 9 | `switch_to_auto` | — |

映射：`ORION_MANUAL_CONTROL` → `Command::Manual(ManualCmd::*)` / `Command::Mode(ModeCmd::*)`。

### 3.5 ORION_TASK_SET（msgid 5）— 任务队列替换

**整体替换语义**（非追加）：收到即丢弃当前任务队列（含正在执行的任务，立即中断），装载新队列从头执行。

| 字段 | 类型 | 说明 |
|---|---|---|
| `count` | uint8 | 任务数；**0 = 取消全部任务（停车待命）** |
| `missions` | 结构数组 | 每项 9 字节（见下） |

`missions[i]`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `type` | uint8 | 任务类型：0 = `Goto`（其余类型预留扩展） |
| `x` | float | 目标点 X（全局世界坐标，米） |
| `y` | float | 目标点 Y（全局世界坐标，米） |

**payload 布局**（大端）：

| 偏移 | 字段 | 类型 |
|---|---|---|
| 0 | `count` | u8 |
| 1 | `missions[count]` | 每项 9 字节（见下） |

`missions[i]` 布局（9 字节）：`type` u8 [0..1) + `x` f32 [1..5) + `y` f32 [5..9)

**总大小 = 1 + 9×count 字节**

行为约束（2026-08-07 决策）：
- **替换**：新队列到达即替换旧队列，不合并、不追加
- **立即中断**：正在执行的任务立即终止（`executor.reset()` + `stm32.stop()`），从新队列第一个任务开始
- **空队列 = 取消**：`count = 0` 即取消全部任务，车停车待命

---

## 4. sysid / compid 约定

### 4.1 身份层次

Orion 中**网络层身份（libp2p peer_id）即车辆系统身份**——车即节点，无第二层身份。MAVLink 帧内 sysid **不参与路由**，仅作为形式字段（帧格式完整性 + 日志可读）。

### 4.2 sysid 派生

```
sysid = peer_id 的 multihash 字节数组最后一字节（0~255）
```

- **确定性**：同一辆车恒得同一 sysid，无需配置
- **碰撞容忍**：不同车可能碰撞（1/256），因不参与路由，无影响
- 实现：解析 libp2p `PeerId` → `as_bytes()` 取末字节

### 4.3 compid 约定

| compid | 组件 |
|---|---|
| 1 | 车（主控） |
| 200 | 控制终端 / 地面站（Pictor） |

---

## 5. 坐标系约定

### 5.1 位置：全局世界坐标

消息内位置数据（`ORION_POSE` 的 x/y、`ORION_TASK_SET` 的目标点）为**全局世界坐标**，单位米：

- **原点**：(0, 0)（全局网格原点）
- **轴方向**：x 东（右）、y 南（下）——Pictor/Godot 2D 约定，与 odometry 积分、SLAM 建图一致
- **初值**：车启动时 x/y = origin（默认 64, 64），由 `odometry::accumulate` 持续积分
- **无坐标变换**：`ORION_POSE` 直接映射 `RobotState.x/y` 原值

### 5.2 朝向 yaw

- 单位：弧度，范围 [-π, π]
- **顺时针为正**（右转 → yaw 增大）——STM32 AHRS 实测定义
- 开机朝向 = 0（非磁北，无绝对航向）
- ⚠️ **踩坑记录**：早期代码误用标准数学"逆时针为正"约定，导致转向/建图镜像错乱；实车测试后修正为顺时针为正（`890e2fc` 对调 spin 方向）。**yaw 不是逆时针为正！**

### 5.3 网格坐标

- 全局网格坐标：`gx = floor(世界米 / 0.5)`，从网格原点 (0, 0) 起算的绝对坐标
- `ORION_MAP_FULL` 的 `origin_gx/origin_gy`：chunk 原点全局网格坐标（data 为 chunk 内顺序数组，必须有原点定位）
- `ORION_MAP_DELTA` 的 `gx/gy`：全局网格坐标（绝对坐标自定位，**无需 chunk 标识**）

### 5.4 单位汇总

| 量 | 单位 |
|---|---|
| 位置 | 米（m） |
| 速度 | 米/秒（m/s） |
| 角度 | 弧度（rad），**顺时针为正** |
| 时间 | 毫秒（ms），开机起算 |

---

## 6. 待补充（后续讨论）

以下章节待后续讨论后补充：

- 与现有代码的映射（各 JSON/二进制点位替换方案）
- 第一版实施范围与步骤
- Pictor（Godot）端接入方案
- libp2p 层的消息投递方式（单播/广播）在协议中的约定
