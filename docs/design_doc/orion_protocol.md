# Orion 统一通信协议（Orion Communication Protocol）

> 创建日期：2026-08-07
> 修改日期：2026-11-05
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
| 身份识别 | 网络层身份（libp2p peer_id）即车辆系统身份；**帧内 sysid = 完整 peer_id**（2026-08-10 升级，见 §4） |
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

Pictor（Godot 地面站）迁移至 libp2p 之前，**WebSocket 链路保留**（连接机制 + `hello` 握手消息，`hello` 仅发送一次，成本可忽略）。

- **WS 消息 payload 已迁移为 ORION 帧**（2026-08-07 实施）：pose / map_delta / map_full / 命令全部为二进制 ORION 帧，旧 JSON 协议（`cmd/action` 三层命令、`type:pose` 遥测）**已移除**
- `hello` 为唯一保留的 JSON 消息（连接握手），当前字段：`type` / `vehicle_id`(peer_name) / `address`
- ✅ **已实施（2026-08-12，Task 14）**：`hello` 增加 `peer_id` 字段——本车完整 peer_id（`PeerId::to_bytes()` 原始字节，hex 编码下发）
  - 动机：群发任务（`ORION_TASK_SET` members 列表）需要终端持有每辆参与车的 peer_id 字节，且必须与车端排序所用字节表示完全一致；此前终端只能从下行帧头 `sysid` 间接解析，无显式身份宣告通道
  - 现状：终端连接即得车身份（hex 解码回原始字节），`members` 原样回填车端字节，杜绝字节表示不一致导致的序号错位
  - `hello` 完整字段：`type` / `vehicle_id`(peer_name) / `address` / `peer_id`（76 hex 字符，Ed25519 38B）
- **`Tool/robot_control.html` 已废弃**（旧 JSON 协议，不再维护；主力地面站为 Pictor）
- Pictor 完成 libp2p 接入后，WS 链路与 `hello` 一并退役

---

## 2. 帧格式（MAVLink 风格 + 按需扩展）

帧结构参考 MAVLink v2，按需扩展如下：

```
┌─────────┬───────┬─────┬───────────┬──────────┬─────────┬────────┬──────────────┬──────────┐
│  magic  │  len  │ seq │ sysid_len │  sysid   │ compid  │ msgid  │   payload    │ checksum │
│  0x4F   │ u32   │ u8  │ u8        │ N 字节   │ u8      │ u16    │ 0~4G 字节    │ u16      │
│  (1B)   │       │     │           │          │         │        │              │          │
└─────────┴───────┴─────┴───────────┴──────────┴─────────┴────────┴──────────────┴──────────┘
```

| 字段 | 长度 | 说明 |
|---|---|---|
| `magic` | 1B | 固定 `0x4F`（'O' = Orion）。自定义标识——len 已非标准，保留 MAVLink `0xFD` 会令标准解析器误认 |
| `len` | 4B | payload 字节数（uint32，上限约 4GB）。**放宽自 MAVLink v2 的 1 字节（255B 上限）**——原始限制是串口时代紧凑设计，无法承载全量栅格地图（65536 cell），予以废弃 |
| `seq` | 1B | **保留字段，第一版恒填 0**——libp2p/TCP 可靠传输，无需丢包检测；未来跑串口/无线等不可靠链路时再启用计数 |
| `sysid_len` | 1B | sysid 字节数（u8，上限 255）；**0 = 无身份**（控制终端上行命令，配合 compid=200） |
| `sysid` | N | 发送方**完整 libp2p PeerId 二进制**（multihash，Ed25519 下 38B；2026-08-10 起由 1B 末字节升级为完整身份，见 §4） |
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
| seq / compid | 各 1B | 各 1B（保留） |
| sysid | 1B（≤255 系统） | **变长（sysid_len + 完整 peer_id）**——MAVLink 的 1B 是"域内编址"，本协议用于"身份"，需完整 peer_id |
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
| 2 | `ORION_MAP_FULL` | 车 → 集群/终端；**终端 → 车（新车初始化）** | 连接建立时 / 按需 |
| 3 | `ORION_MAP_DELTA` | 车 → 集群/终端 | 地图有变化时（≤5Hz） |
| 4 | `ORION_MANUAL_CONTROL` | 终端/集群 → 车 | 事件驱动 |
| 5 | `ORION_TASK_SET` | 终端/集群 → 车 | 事件驱动 |
| 6 | `ORION_RTCM` | 基站/终端 → 车（gossipsub 广播） | 1Hz（基站 RTCM 实时流，Task 24 新增） |
| 8 | `ORION_MODE` | 终端/集群 → 车 | 事件驱动（模式切换，task_44 新增） |

> 字段命名遵循 MAVLink 风格（`snake_case`、C 类型）。类型字节序：大端（与 MAVLink 一致）。

### 3.1 ORION_POSE（msgid 1）— 位姿

一次打包坐标 + 速度 + 朝向 + 时间戳（保持单车时代的一次性语义），并附带寻路意图子目标与 RTK 可信标志。

| 字段 | 类型 | 单位 | 说明 |
|---|---|---|---|
| `time_boot_ms` | uint32 | ms | 开机起算时间戳 |
| `x` | float | m | 全局世界坐标 X（x 东） |
| `y` | float | m | 全局世界坐标 Y（y 南） |
| `z` | float | m | 垂直高度（车恒 0，机写飞控 EKF；RTK 不更新 z，Task 22） |
| `vx` | float | m/s | X 方向速度 |
| `vy` | float | m/s | Y 方向速度 |
| `yaw` | float | rad | 朝向角，范围 [-π, π]，**顺时针为正** |
| `valid` | uint8 | — | 意图有效标志（Task 13_1）：0 = 无当前任务/子目标 |
| `sub_gx` | int32 | 格 | 意图：D* 寻路下一格网格坐标 X；`valid=0` 时忽略 |
| `sub_gy` | int32 | 格 | 意图：D* 寻路下一格网格坐标 Y；`valid=0` 时忽略 |
| `rtk_fixed` | uint8 | — | RTK 固定解标志（Task 24）：FIXED=1 / 失锁=0；未启用 RTK 恒 0 |

**payload 布局**（大端，共 38 字节，Task 24 起）：

| 偏移 | 字段 | 类型 |
|---|---|---|
| 0 | `time_boot_ms` | u32 |
| 4 | `x` | f32 |
| 8 | `y` | f32 |
| 12 | `z` | f32 |
| 16 | `vx` | f32 |
| 20 | `vy` | f32 |
| 24 | `yaw` | f32 |
| 28 | `valid` | u8 |
| 29 | `sub_gx` | i32 |
| 33 | `sub_gy` | i32 |
| 37 | `rtk_fixed` | u8 |

> 长度演进：24B（初版：时间戳 + x/y/vx/vy/yaw）→ 33B（Task 13_1 追加 `valid` + `sub_gx/sub_gy`）→ 37B（Task 22 追加 `z`）→ **38B（Task 24 末尾追加 `rtk_fixed`）**。`decode_pose` 严格校验 38 字节（`payload.len() != 38` 直接拒绝），旧帧无兼容过渡期——整车/机/终端/Pictor 须同版本合入。

来源映射：`x/y` 直接映射 `RobotState.{x, y}`（全局世界坐标）、`z` 映射 `RobotState.z`（车恒 0）、`{vx, vy}`、`attitude.yaw`，**无坐标变换**；`time_boot_ms` 由开机基准时间换算（`now_boot_ms()`，替代原 `Pose.ts` unix 秒，字段类型 f64 → u32）；`valid/sub_gx/sub_gy` 映射 `ExecuteState.sub_target`（Executor 写，`state_notifier` 读，Task 13_1）；`rtk_fixed` 映射 `RobotState.rtk_fixed`（LG290P 实时写，Task 24）。

> 意图语义（2026-08-10 决策）：subtarget = 本车 D* 寻路**下一格**（第一版 1 格，k 格 + 时间窗留扩展字段）；接收方存入 `ClusterInfo.sub_target`，用于窄路口意图判断/未来寻路障碍注入。`valid=0`（无任务/空闲）时接收方必须忽略 sub 坐标。

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
| `data` | int8[width×height] | 每 cell 一字节：**log-odds 原始值（i8，−8~+8）**（Task 13_2：由三态改为原始 log-odds；接收方按阈值 ±6 派生三态） |

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

状态编码（2026-08-11 变更，Task 13_2）：data 从三态（0/100/255）改为 **log-odds 原始值 i8（clamp ±8）**——接收方（Pictor）需按阈值派生三态：`>+6 Occupied，<−6 Free，其余 Unknown`。⚠️ 复用 msgid=2，与车端同批升级，无兼容过渡期。

发送时机：控制终端/其他车连接建立后发送一次；后续按需重发（协议层不做主动周期推送）。

**接收语义（2026-08-12 实施，Task 13_2 方案二）**：车端 WS 入站识别 msgid=2 **就地处理**（不经过命令通道）——`decode_map_full` 宽松校验（仅结构性长度 20+width×height），origin/width/height/resolution 读出不校验（当前全系统恒 origin=(0,0)、256×256、0.5m，多 chunk/偏移场景留后续）；校验通过后 `set_log_odds` 整表替换 merged（**own 保留**——own = 本车观测贡献，对账上报数据源）。终端下发时机：**新车接入 WS 时**（初始化全量，弥补新车收不到历史 gossip Δ 的缺口）。⚠️ **时序约定（Pictor 必须遵守）**：终端须等**收到新车 own 上报之后**再下发 FULL——否则下发的全局图不含新车贡献，新车 merged 将被覆盖、丢失自身初始观测（先 own 后 FULL，缺一不可）。

**方向语义（2026-08-12 收敛）**：车→终端 = 接入时上报 own 整表（现有）；终端→车 = 新车接入时下发全局全量（新增）；**车↔车无 FULL**（车启动=入网、own 从 0 开始、Δ 流即完整历史）。

### 3.3 ORION_MAP_DELTA（msgid 3）— 地图增量

仅传变化的格子。4 字节 len 下，一帧可承载 13107 项（65535 字节），足以覆盖单次 SLAM 更新（数百~数千项），**无需分帧**。

| 字段 | 类型 | 说明 |
|---|---|---|
| `time_boot_ms` | uint32 | ms |
| `count` | uint16 | 变化格子数；**协议上限 65535 条目/帧** |
| `entries` | 结构数组 | 每项 9 字节（见下） |

`entries[i]`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `gx` | int32 | 全局网格坐标 X（绝对坐标，无需 chunk） |
| `gy` | int32 | 全局网格坐标 Y（绝对坐标，无需 chunk） |
| `delta` | int8 | **log-odds 数值差分**（+3/−1 等，clamp 后；Task 13_2：由三态改为 Δ） |

**payload 布局**（大端）：

| 偏移 | 字段 | 类型 |
|---|---|---|
| 0 | `time_boot_ms` | u32 |
| 4 | `count` | u16 |
| 6 | `entries[count]` | 每项 9 字节（见下） |

`entries[i]` 布局（9 字节）：`gx` i32 [0..4) + `gy` i32 [4..8) + `delta` i8 [8..9)

**总大小 = 6 + 9×count 字节**

来源映射（2026-08-11，Task 13_2）：`slam::update` 返回的 `Vec<Delta{gx,gy,delta}>`，slam_task 跨帧聚合净变化（**不做 ±8 clamp，发真实差分**；接收方应用时 clamp ±8 完成精确重放）。
频率：每 5 帧（1s）发送一次（slam_task 模 5 节流，WS 与 gossip 链路一致）。

### 3.4 ORION_MANUAL_CONTROL（msgid 4）— 手动两轴连续量

task_44 起，手动命令由「一维动作枚举」改为「**两轴连续量**（v, ω）+ 事件」，**按 payload 长度分流**：
`8 字节` = 运动，`3 字节` = 事件，其它长度整帧拒。口径 §1.2 #5：**不做版本兼容**，四端（base / 真车 / 真机 /
sim）与 Pictor **必须同批升级**，无过渡期。

统一量纲（§1.2 #2）：`(v m/s, ω rad/s)`，**`ω > 0` = 左转（逆时针）**——注意与 §5.2 的 `yaw`
（顺时针为正）符号相反：`d(yaw)/dt = −ω`。

**表 A：运动（8 字节，大端）**

| 偏移 | 字段 | 类型 | 说明 |
|---|---|---|---|
| 0 | `v` | float | 线速度，**m/s**（正 = 前进，负 = 后退） |
| 4 | `omega` | float | 角速度，**rad/s**，`> 0` = 左转（逆时针），`< 0` = 右转 |

> 手动就是 task_44 §1.4 那张输出表：`W` → `(0.45, 0)`、`W+A` → `(0.45, +0.5)`、`A` → `(0, +0.5)`、全松 → `(0, 0)`。
> 金标字节串（单测硬编码，三端共用同一串）：

```
v=0.5,  omega=0      -> 3f00000000000000
v=0,    omega=0      -> 0000000000000000
v=0,    omega=-0.5   -> 00000000bf000000
```

> 每串 8 字节 = 前 4 B（`v`）| 后 4 B（`omega`）。

> ⚠️ f32 大端是**位模式直传**：`0.0` → `00000000`、`-0.0` → `80000000`。全零运动必须用 `+0.0`，
> 不要写 `-0.0`（`-0.0` 的字节是 `80000000`，与"全零"不等）。

**表 B：事件（3 字节，大端）**

| 偏移 | 字段 | 类型 | 说明 |
|---|---|---|---|
| 0 | `action` | uint8 | 事件枚举（见下） |
| 1 | `param` | int16 | 动作参数（`beep` = 时长 ms）；其余事件填 0 |

`action` 枚举（task_44 重排，旧的 `0~4` 运动项与 `8/9` 模式项已删除）：

| 值 | 动作 | param 含义 | 端 |
|---|---|---|---|
| 1 | `beep` | 时长 ms | 车 |
| 2 | `start_lidar` | — | 车 |
| 3 | `stop_lidar` | — | 车 |
| 4 | `takeoff` | — | 机 |
| 5 | `land` | — | 机 |

**长度分流表**：

| payload 长度 | 语义 | 处理 |
|---|---|---|
| `== 8` | 运动（表 A） | `ManualPayload::Motion { v, omega }` |
| `== 3` | 事件（表 B） | `ManualPayload::Event { action, param }` |
| 其它 | 非法帧 | `decode_manual_control` 返回 `None` → **整帧拒**（日志 warn，队列/执行器不动） |

`action` ∉ `{1,2,3,4,5}` → **整帧拒**（未知 action 不做兼容映射）。

**映射**：

| 载荷 | 内部命令 |
|---|---|
| 运动 8 B | `Command::Manual(ManualCmd::Motion { v, omega })` |
| 事件 `beep` | `ManualCmd::Beep(param as u16)`（i16 → u16 位模式直传，不钳负值） |
| 事件 `start_lidar` / `stop_lidar` | `ManualCmd::StartLidarScan` / `StopLidarScan` |
| 事件 `takeoff` / `land` | `ManualCmd::Takeoff` / `Land` |

**停车语义**：协议里**没有 `stop` 动作**；全零 8 B 运动帧 `(0, 0)` 即停。`ManualCmd::Stop` 只作
**内部命令**保留（sim 控制口的 `stop`、切模式时的显式停车），设备端把它解释为 `(0, 0)`。

**⚠️ 旧值警告（无兼容过渡期，混跑即事故）**：

| 旧值（旧 msgid 4 的 action） | 新语义 | 后果 |
|---|---|---|
| `0`~`4`（`forward` / `backward` / `spin_left` / `spin_right` / `stop`） | 新表中不存在 | 整帧拒（车收不到） |
| `8` / `9`（`switch_to_manual` / `switch_to_auto`） | 已迁到 **`msgid 8`**（§3.6）；出现在 msgid 4 里按未知拒 | 整帧拒（模式切不了） |
| `5`（旧 `beep`） | 新表 `5` = **`land`** | ⚠️ **机端会误降落** ⇒ 必须同批升级，不得混跑 |
| 旧 3 B 帧（`action` + `param`） | 3 B 仍是合法长度 → 按**事件**解 | 旧 `6`/`7`（旧 `start/stop_lidar`）在新表里不存在 → 整帧拒 |

频率：事件驱动（Pictor 按键集合变化才发一帧，Step 3 落地）；全松键发 `(0, 0)`。

### 3.5 ORION_TASK_SET（msgid 5）— 任务队列（Task 36：单分支，无群发段）

**整体替换语义**（非追加）：收到即丢弃当前任务队列（含正在执行的任务，立即中断），装载新队列从头执行。

**payload 布局**（大端，Task 36 §十二 Step 3 起）：

| 偏移 | 字段 | 类型 | 说明 |
|---|---|---|---|
| 0 | `mission_count` | uint8 | 任务数；**协议上限 255 任务/帧** |
| 1 | `missions[mission_count]` | 结构数组 | 每项**变长** 9 / 13 / 17 字节（见下） |

`missions[i]` 布局（变长，公共头 9 字节）：`type` u8 [0..1) + `x` f32 [1..5) + `y` f32 [5..9)；
`type = 3` 再追加 `half_side` f32 [9..13)；`type = 5` 再追加 `fx` f32 [9..13) + `fy` f32 [13..17)

| 字段 | 类型 | 说明 |
|---|---|---|
| `type` | uint8 | 任务类型：0 = `Goto`、2 = `Follow`、3 = `Containment`、4 = `Face`、5 = `Guard`（其余值 → **整帧拒**） |
| `x` | float | `Goto` / `Follow` / `Guard`：目标点 / 值守点 X；`Containment`：正方形中心 X；`Face`：**要朝向的世界点** X（全局世界坐标，米） |
| `y` | float | 同上（Y） |
| `half_side` | float | 仅 `type = 3`：半边长（米）；必须有限且 `> 0`，否则整帧拒 |
| `fx` / `fy` | float | 仅 `type = 5`：注视点（朝向 = `atan2(fy − y, fx − x)`）；**不做**有限性校验（有限性门控在 Pictor 入口） |

**记录长度（含 `type` 字节）**：`0 / 2 / 4 → 9 B`、`3 → 13 B`、`5 → 17 B`；
**总大小 = 1 + Σlen 字节**（`1 + 9×mission_count` 只对「全是 9 B 记录」的帧成立）。

> `Follow`（Task 37 §五.2）：`type = 2`，`x/y` 预留（现在恒 0），后续可用于「跟随偏移」等语义。
> `Face`（task_48 §2.2）：`type = 4`，`x/y` = 到点后要**朝向**的世界点（原地转、不移动；60 拍没对准也 `Success`）。
> `Guard`（task_48 §2.8）：`type = 5`，`x/y` = 值守点、`fx/fy` = 注视点（持续任务，永不 `Success`）。
> ⚠️ **`1` 号是 Task 36 已删的旧 `Circle`，不复用**——地面站/旧工具链里可能仍有该值，
> 复用会让旧 Circle 帧被误认成 `Follow`；旧 type 长度未知 ⇒ **整帧拒**（队列不动）。

**单分支语义**（Task 36 定稿；取代 Task 14 的 `member_count` 三分支）：

| 条件 | 语义 | 处理 |
|---|---|---|
| `mission_count == 0` | **取消全部任务**（唯一取消入口） | 空队列替换，车停车待命 |
| `mission_count > 0`，`type ∈ {0, 2, 3, 4, 5}` | **逐条入队** | `Goto { x, y }` / `Follow { x, y }` / `Containment { x, y, half_side }` / `Face { x, y }` / `Guard { x, y, fx, fy }` 依序替换本地队列 |
| `mission_count > 0`，`type ∉ {0, 2, 3, 4, 5}`（含退役的 `1`） | 未知 type | 长度未知 → `decode_task_set` 返回 `None`（**整帧拒**，队列不动） |
| 长度 ≠ `1 + Σlen`（多 1 B / 与条数不符） | 非法帧 | `decode_task_set` 返回 `None` |
| `type = 3` 的 `half_side` 非有限或 `<= 0` | 非法帧 | **整帧拒** |

行为约束：
- **替换**：新队列到达即替换旧队列，不合并、不追加
- **立即中断**：正在执行的任务立即终止（`executor.reset()` + `stm32.stop()`），从新队列第一个任务开始
- **协同上移指挥层**（Task 36 §十一 #27）：地面站逐车算点、**逐车单独发帧**，`missions[0]` 即该车自己的目标点；
  车端不再持有名单、不做分配（`planning/assignment.rs` 三份已删）
- `Face` 是**一次性**任务（进 8° 死区即 `Success`；60 拍没对准也 `Success`，**不得** `Failure`）；`Guard` 是**持续**任务
  （永不 `Success`，只有新的 `TASK_SET` 或 `mission_count = 0` 能结束它）
- 「到点后朝人」= **同一帧两条** `[Goto(点), Face(人)]`（替换语义，缺一条就少一段行为；口径 #4）
- ⚠️ **同批升级无过渡期**：车端与 Pictor 必须同批上线；旧帧（含 `member_count`/`members` 段）因长度校验必然被拒

### 3.6 ORION_MODE（msgid 8）— 模式切换

> 本节按 task_44 编排插在 RTCM 之前（节序因此略偏离 msgid 升序）；msgid 6 RTCM 见 §3.7。

模式切换拆成**独立消息**（task_44 §1.2 #4）：payload **恒 1 字节**，事件驱动（Pictor 面板勾选）。

| 偏移 | 字段 | 类型 | 说明 |
|---|---|---|---|
| 0 | `mode` | uint8 | `0` = 手动（manual） / `1` = 自动（auto） |

金标字节串（单测硬编码）：

```
mode=manual  -> 00
mode=auto    -> 01
```

**解码规则**（`decode_mode`，不猜）：

| 条件 | 处理 |
|---|---|
| `len != 1` | 拒（整帧） |
| `mode ∉ {0, 1}` | 拒（整帧） |

**映射**：`mode 0` → `Command::Mode(ModeCmd::SwitchToManual)`；`mode 1` → `Command::Mode(ModeCmd::SwitchToAuto)`。

> ⚠️ **关键约束：必须映射到 `Command::Mode`，不得映射成 `Command::Manual`** —— 它要**绕过**
> `main_loop` 的手动模式门闩；否则「从手动切回自动」这条命令会被手动门闩自己拦住。

**取代关系**：原 `msgid 4` 的 `action` 旧值 `8` / `9`（旧的 `switch_to_manual` / `switch_to_auto`，
已删除）不再存在，模式切换**只走本消息**；`msgid 4` 里出现这两个值按未知 action 整帧拒。

方向：终端/集群 → 车（`compid = 200`）。

### 3.7 ORION_RTCM（msgid 6）— RTK 改正数据（Task 24 新增）

基站（`pleiades-terminal` / UM960）向所有车广播的 **RTCM 3.x 原始帧**，供车侧流动站（LG290P）解算厘米级定位。**双层帧**：外层 ORION 帧（传输 + 身份），内层 RTCM（业务数据），payload 无自定义字段结构。

| 字段 | 类型 | 说明 |
|---|---|---|
| `payload` | bytes[] | **RTCM 3.x 原始帧**（基站侧已切帧，一条消息 = 一帧完整 RTCM） |

**payload 布局**：即 RTCM 原始字节，长度可变（无固定偏移表；帧长由外层 `len` 字段给出）。

- **方向**：基站/终端 → 车（**gossipsub 广播**，topic `pleiades/robot/rtcm` = `TOPIC_RTK_RTCM`）；车侧由 LG290P 驱动独立订阅 robot_bus 认领（`match msgid == 6`，D7 方式 A，`cluster_consumer` 不改）
- **频率**：1Hz 实时流（随基站串口 RTCM 输出；`Gossipsub_Publish_Try` 满则丢弃，下一帧补上）
- **帧身份**：`sysid` = 基站（terminal）本机 peer_id 字节（`Get_Local_Peer_Id().to_bytes()`）、`compid` = 200（`COMPID_GROUND_STATION`）
- **快照重放**：复用现有统一机制（发布即更新快照），新车接入可重放最近一帧（无害）
- 实现：基站侧 `encode_frame(MSGID_RTCM, &sysid, COMPID_GROUND_STATION, &rtcm)`（`pleiades-terminal/src/device/um960/mod.rs`）；车侧 `decode_frame` + `MSGID_RTCM` 比对后取 `frame.payload` 写串口（`pleiades-ugv/src/device/lg290p/mod.rs`）
- ⚠️ **无独立 `encode_rtcm` 辅助函数**：当前直接调用通用 `encode_frame` 组帧（rtk_design §5.1 曾计划新增 `encode_rtcm`，实际未落地，以代码为准）

---

## 4. sysid / compid 约定

### 4.1 身份层次

Orion 中**网络层身份（libp2p peer_id）即车辆系统身份**——车即节点，无第二层身份。帧内 `sysid` 承载**完整 peer_id**（2026-08-10 升级），接收方直接以 sysid 识别发送方；`sysid` **不参与路由**（libp2p 负责投递），仅作身份标识。

### 4.2 sysid = 完整 peer_id（2026-08-10 升级，Task 13 阶段一）

```
sysid = libp2p PeerId 的 multihash 字节数组（Ed25519 下 38B）
帧内格式：sysid_len(1B) + sysid(N 字节)
```

- **身份语义**：sysid 即车辆身份（车 = libp2p 节点），接收方无需额外映射即可识别发送方；无碰撞（此前 1B 末字节方案 1/256 碰撞率，仅适合"编址"不适合"身份"，已废弃）
- **地面站上行**：sysid_len = 0（空身份）+ compid = 200，识别为终端命令；Pictor 迁移 libp2p 后天然获得 peer_id，自动升级
- **本车过滤**：gossipsub 默认自环回流本机消息，接收端用 sysid 与本地 peer_id 比对过滤本车数据
- 实现：`PeerId::to_bytes()`（Vec<u8>）直接入帧；`encode_frame(msgid, &peer_id, compid, payload)`

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
- ⚠️ **角速度 ω 的正方向与此相反**：`ω > 0` = 左转（逆时针）= **yaw 减小**；即 `d(yaw)/dt = −ω`
  （task_44 §1.2 #2 / §1.4 的 ± 号来源——驱动反解与执行器符号都按这条，写反会左右颠倒）。

### 5.3 网格坐标

- 全局网格坐标：`gx = floor(世界米 / 0.5)`，从网格原点 (0, 0) 起算的绝对坐标
- `ORION_MAP_FULL` 的 `origin_gx/origin_gy`：chunk 原点全局网格坐标（data 为 chunk 内顺序数组，必须有原点定位）
- `ORION_MAP_DELTA` 的 `gx/gy`：全局网格坐标（绝对坐标自定位，**无需 chunk 标识**）

### 5.4 单位汇总

| 量 | 单位 |
|---|---|
| 位置 | 米（m） |
| 线速度 | 米/秒（m/s） |
| 角速度 | 弧度/秒（rad/s），**ω > 0 = 左转（逆时针）** |
| 角度 | 弧度（rad），**顺时针为正** |
| 时间 | 毫秒（ms），开机起算 |

---

## 6. 待补充（后续讨论）

以下章节待后续讨论后补充：

- 与现有代码的映射（各 JSON/二进制点位替换方案）
- 第一版实施范围与步骤
- Pictor（Godot）端接入方案
- libp2p 层的消息投递方式（单播/广播）在协议中的约定
