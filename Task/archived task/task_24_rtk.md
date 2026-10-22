# task_24_rtk — RTK 局域差分定位接入

> Created Date ： 2026-08-31
> Modified Date ： 2026-08-31
> 状态：进行中（规划阶段，逐步明确）
> 关联文档：`docs/design_doc/rtk_design.md`（RTK 设计，架构决策已固化）

---

## 一、目标

把 RTK 局域差分定位接入 Orion：

- 地面站（`pleiades-terminal`）作为 RTK 基站节点，接 UM960；
- 车（`pleiades-ugv`）作为流动站，接 LG290P，获得厘米级定位；
- RTCM 改正数据走 gossipsub 广播，复用现有 robot 数据面；
- 定位结果以 offset 方式更新 `RobotState`。

## 二、架构决策（详见 `rtk_design.md` §3）

| # | 决策 |
|---|------|
| D1 | UM960 驱动放 `pleiades-terminal`（地面站=基站，同一节点） |
| D2 | LG290P 驱动放 `pleiades-ugv` |
| D3 | 三个纯函数各写一份、不共享 |
| D4 | RTCM 包 ORION 帧（新增 `msgid=6`） |
| D5 | 复用 robot_bus，不开 rtk_bus |
| D6 | 快照重放复用现有统一机制（零改动） |
| D7 | 方式 A：LG290P 驱动独立订阅 robot_bus，自己 `match msgid==6` |
| D8 | 基站侧切帧广播（`Rtc3Parser`） |
| D9 | offset 更新 `RobotState`；`RTK_FIXED` 才覆盖、失锁保持 |
| D10 | offset 用「首次 FIXED 车位置」做 ENU base（`enu()` base 参数 = 车首次 FIXED GGA 坐标），**不需要基站坐标下行** |
| D11 | **z 轴不更新**：LG290P 忽略 U（高度）分量，`RobotState.z` 保持设备端现状 |

---

## 三、第一部分：RTK 驱动代码

### 3.1 三个纯函数

- `parse_gga` / `ecef` / `enu` 放**各自 device 代码里**（不是独立 geo 模块）；
- terminal 的 um960 驱动一份、ugv 的 lg290p 驱动一份，内容相同但物理独立、不共享。

### 3.2 目录结构

- terminal：`src/device/um960/`（UM960 驱动 + 三个函数）
- ugv：`src/device/lg290p/`（LG290P 驱动 + 三个函数）
- **不碰现有 uav/ugv 结构**，以后逐步把 stm32/lidar/mavlink 也调整进 `device/`。
- terminal 侧新增 `src/config.rs`（读 `[um960]` 段）+ UM960 装配；terminal 是 cdylib（**无 main.rs**），装配挂 `lib.rs` 后台线程。

### 3.3 config 配置

| 段 | 归属 | enabled 默认 | 字段 |
|----|------|-------------|
| `[um960]` | terminal（基站） | `false` | `enabled` / `port` / `baudrate` / `survey_seconds` |
| `[lg290p]` | ugv（流动站） | `false` | `enabled` / `port` / `baudrate` |

- `enabled=false` → 不 spawn、不启动对应 tokio task；
- `enabled=true` → spawn + 启动 tokio，后台自己跑，**自己根据 GGA quality 决定是否更新 `RobotState`**（FIXED 才更新、失锁保持）。

### 3.4 UM960 基站驱动（terminal 侧）

**整体形态**：复用 `spawn_port`（TX + RX 两个 tokio，**无第三个**）；生命周期状态机 `Init → Surveying → Stable → Broadcasting`；共享状态 `Arc<RwLock<Um960Phase>>`（RX 读 phase 决定处理方式，TX/RX 在切换时写 phase）。

#### 状态 1：Init

- TX 顺序发 3 条命令：`UNLOG COM1` → `MODE BASE TIME <survey_seconds>` → `GPGGA COM1 1`；
- **每次无条件重发**（UM960 未 `saveconfig`，掉电后 baud/RTCM 配置恢复默认，不能假设已有配置）；
- 发完 → Surveying。

#### 状态 2：Surveying

- RX 按行切分 GGA（找 `\r\n`）→ `parse_gga`；
- 原点稳定判定：`quality==7` 且**连续**两帧 `enu()` 距离 < 0.5m（水平 + 垂直）→ 切 Stable；非 7 帧**清空候选**（保证连续性）；
- 时长：`survey_seconds`（config，默认 180）；
- 失败：**静默停留 Surveying**，不重发、不报错；每 30s 健康日志（RX 发，记录 `started_at`/`last_quality`/`last_sats`/`last_log`）。

#### 状态 3：Stable

- TX 顺序发 7 条命令：`UNLOG COM1 GPGGA`（关 GGA）+ `RTCM1006/1033 COM1 10` + `RTCM1074/1084/1094/1124 COM1 1`（开 RTCM）；
- 顺序：先关文本、再开二进制（避免交错）；
- 发完 → Broadcasting。

#### 状态 4：Broadcasting（稳态）

- RX 回调（同步一条龙）：`Rtc3Parser` 切帧（找 `0xD3` + 读长度）→ `encode_rtcm` 包 ORION（`msgid=6`、`sysid`=terminal PeerId、`compid`=200）→ `Gossipsub_Publish_Try` 广播；
- 用 `Gossipsub_Publish_Try`（同步 try_send，满则丢弃）——RTCM 是 1Hz 实时流，丢一帧下一秒补上；**不新增第三个 tokio**；
- 切到 Broadcasting 时 `Rtc3Parser` 从干净状态初始化（丢弃 GGA 残留）；
- `Rtc3Parser` 只按 `0xD3` + 长度切帧，**不校验 CRC**（CRC 交给 LG290P 内部校验）。
- 需要注入：`NodeHandle` + `TOPIC_RTK_RTCM`。

### 3.5 LG290P 流动站驱动（ugv 侧）

**整体结构**：3-task（复用 `spawn_port` + 订阅 task）：

- task① TX（spawn_port）：监听 mpsc → 写串口；
- task② RX（spawn_port）：`on_bytes` 回调读 GGA；
- task③ robot_bus 订阅：`recv` → `decode_frame` → `match msgid==6` → 取 RTCM 帧 → `serial_cmd_tx.send(rtcm)` → TX 写串口。
- LG290P 默认 Rover 模式，驱动**不需配置接收机模式**（只读 GGA + 写 RTCM）。

**状态机**（2 态）：

- `WaitingFirstFix`：等首次 RTK_FIXED（约 10s~几分钟），期间不更新 RobotState（靠里程计/SLAM 推）；
- `Tracking`：已记录基准，进入 offset 更新。

**GGA 处理**（task② `on_bytes`）：按行切分 → `parse_gga` → quality 判断 → FIXED 才 `ecef`/`enu` → offset 更新。

**offset 更新**：

- ENU base 用**首次 FIXED 的车位置**（`enu()` 的 base 参数 = 车首次 FIXED 的 GGA 坐标），**不需要基站坐标下行**；
- 首次 FIXED 记录基准 ENU `(E₀,N₀,U₀)`，对齐世界原点 `origin`（64,64,0，靠「同一位置启动」保证车间对齐）；
- `world_x = x₀ + (E−E₀)`、`world_y = y₀ − (N−N₀)`（y 取反）；
- **z 不更新**（LG290P 忽略 U 分量，`RobotState.z` 保持设备端现状）；
- yaw 不动（RTK 只给位置）；FIXED 才更新，失锁保持。

**rtk_fixed 字段**：

- `RobotState` 加 `rtk_fixed: bool`，LG290P **实时写**：FIXED 写 `true`，失锁/非 FIXED 写 `false`（x/y/z 位置保持不更新，仅 flag 反映实时可信度）；
- `PoseData` 加 `rtk_fixed: bool`（POSE 37→38 字节），终端据此判断位置是否可信；未启用 RTK 的车恒为 false。

**定位源**：

- x/y 积分载体从 STM32 `local_state` 改为共享 `RobotState`（里程计与 RTK 读写同一个 `RobotState.x/y`，RTK 覆盖后里程计自然从新位置继续 `+=` 积分，无需「定位源切换」）；
- **写者改造**：STM32 RX 回调里 `*guard = local_state.clone()` 全量覆盖必须拆掉——传感器字段（vx/vy/battery/attitude…）照写，x/y 改为在共享 `RobotState` 上**读-改-写**（`guard.x += 里程计增量`），且读+改+写在同一把写锁内完成（避免与 RTK 覆盖交错丢增量）；里程计积分从无锁变有锁（10Hz，开销预期可忽略）。

---

## 四、第二部分：base 修改

### 4.1 ORION 协议层

| 落点 | 文件 | 改动 |
|------|------|------|
| msgid | `Robot/core/protocol/mod.rs` | 加 `MSGID_RTCM: u16 = 6`（msgid 常量块末尾） |
| PoseData | `Robot/core/protocol/messages.rs` | 加 `pub rtk_fixed: bool` |
| encode_pose | `Robot/core/protocol/messages.rs` | 37→38 字节，末尾加 `rtk_fixed`（1 字节） |
| decode_pose | `Robot/core/protocol/messages.rs` | `!= 37` → `!= 38`，末尾解 `rtk_fixed` |

RTCM 封装：**不新增函数**，terminal 侧直接调 `encode_frame(MSGID_RTCM, sysid, COMPID_GROUND_STATION, &rtcm_frame)`。

### 4.2 状态层

- `Robot/core/state.rs` 的 `RobotState`：加 `pub rtk_fixed: bool`；
- 注释同步：x/y/z 写入者从「仅 STM32 里程计」改为「STM32 里程计 + LG290P RTK 共同写」。

### 4.3 位姿广播（state_notifier）

- `Robot/core/robot.rs` 的 `state_notifier`：
  - 快照元组加 `s.rtk_fixed`（无额外锁）；
  - `PoseData` 构造加 `rtk_fixed` 字段。

### 4.4 gossipsub 网络层

| 落点 | 文件 | 改动 |
|------|------|------|
| topic 常量 | `Network/Gossipsub/mod.rs` | 加 `TOPIC_RTK_RTCM = "pleiades/robot/rtcm"` |
| 订阅 | `Config/config.rs` + `bootstrap.rs` + `Network/network_service.rs` | 新增 `[Network].subscribe_topics` 配置项；ugv 配 `["pleiades/robot/rtcm"]` 才订阅（**不硬编码全节点订阅**） |
| 分发 | `Network/swarm_events.rs` | `TOPIC_RTK_RTCM` 进透传分支（`StreamRaw`，与 POSE/MAP 同款） |
| 导出 | `Network/mod.rs` | `pub use ... TOPIC_RTK_RTCM` |

> 快照重放**零改动**（发布即更新快照是统一机制）。

---

## 五、已明确（原「待明确」）

- `Rtc3Parser` 切帧：`0xD3` 帧头 + 长度字段（`((byte1&0x03)<<8)|byte2`，完整帧长 = length+6）攒缓冲切帧；**不校验 CRC**；坏长度跳过重同步；缓冲区设上限（如 4096）。
- GGA 行解析器：照搬 `uploads/lg290p_enu.py` 的 `parse_gga`（XOR 校验 + 度分转换 + 空字段容错），`self_test` 样本可移植为单测。
- config 字段：`[um960]` = enabled/port/baudrate/survey_seconds；`[lg290p]` = enabled/port/baudrate。
- `sysid`：`NodeHandle::Get_Local_Peer_Id()` 取 terminal PeerId，注入即可。

---

## 六、详细实施计划（function 级）

> 本文只写**实施计划**，不写实现代码（不改任何 .rs 文件）。
> 路径均相对 workspace root：`/vepfs-mlp2/c20250205/240804016/Workspace/Orion`。
> 签名以当前代码为准（截至 2026-08-31 读到的版本）。

### 6.0 阶段总览与依赖

| 阶段 | 内容 | 涉及 crate | 交付物 |
|------|------|-----------|--------|
| 阶段 0 | base 协议 / 状态 / 网络改动 | `pleiades-base` | `MSGID_RTCM=6`、`PoseData.rtk_fixed`（37→38B）、`RobotState.rtk_fixed`、`TOPIC_RTK_RTCM` 订阅+透传 |
| 阶段 1 | terminal 侧 UM960 基站 | `pleiades-terminal` | `config.rs` + `device/um960/`（geo / rtcm_parser / 状态机 / spawn） |
| 阶段 2 | ugv 侧 LG290P 流动站 | `pleiades-ugv` | `Lg290pConfig` + `device/lg290p/` + STM32 写者改造 + odometry 改造 + 装配 |

依赖关系：

1. **阶段 0 必须先做**：阶段 1 依赖 `MSGID_RTCM` / `COMPID_GROUND_STATION` / `encode_frame` / `TOPIC_RTK_RTCM`（re-export）；阶段 2 依赖 `MSGID_RTCM` / `decode_frame` / `RobotState.rtk_fixed`，且 RTCM 能到达车的前提是 ugv 已通过 `[Network].subscribe_topics` 订阅 `TOPIC_RTK_RTCM`。
2. 阶段 0 内部顺序（编译单元依赖）：
   `protocol/mod.rs`（常量）→ `protocol/messages.rs`（PoseData 38B）→ `protocol/frame.rs`（确认零改动）→ `state.rs`（RobotState.rtk_fixed）→ `robot.rs`（state_notifier 组帧）→ `Network/Gossipsub/mod.rs`（topic 常量）→ `Network/mod.rs`（re-export）→ `Network/network_service.rs`（订阅）→ `Network/swarm_events.rs`（透传分支）。
3. 阶段 1 与阶段 2 **相互独立**，可并行；但两者都要等阶段 0 合入后才能编译/联调。
4. 阶段 2 内部的「STM32 写者改造 + odometry 改造」与「LG290P 驱动」**必须同一合入**：否则存在两个并发写 x/y 的写者（旧的全量覆盖会踩掉 RTK 的 x/y，或 RTK 增量被覆盖丢增量）。

---

### 6.1 阶段 0 — base 改动（`pleiades-base`）

#### 6.1.1 `pleiades-base/src/Robot/core/protocol/mod.rs`

- 新增常量（放在 `MSGID_TASK_SET` 之后）：

```rust
pub const MSGID_RTCM: u16 = 6;
```

- 功能：RTCM 改正数据的 ORION 消息 ID（协议文档 §3 的 6 号消息）。不需要改 `pub use` 列表（常量在本文件直接定义，按路径 `pleiades_base::robot::core::protocol::MSGID_RTCM` 引用）。

#### 6.1.2 `pleiades-base/src/Robot/core/protocol/messages.rs`

- 修改 `pub struct PoseData`（当前 37 字节，Task 22 加 z 后）：在 `sub_gy` 之后**追加**字段：

```rust
/// RTK 固定解标志：true = 位置由 LG290P RTK_FIXED 覆盖，终端据此判断位置可信度
pub rtk_fixed: bool,
```

- 修改 `pub fn encode_pose(pose: &PoseData) -> Vec<u8>`：
  - 功能：37 → 38 字节；保持现有 4+4+4+4+4+4+4+1+4+4 布局不变，**末尾追加 1 字节** `rtk_fixed`。
  - 改动点：`Vec::with_capacity(37)` → `with_capacity(38)`；函数末尾加 `buf.push(pose.rtk_fixed as u8);`；同步更新 doc 注释「37 字节」→「38 字节」。

- 修改 `pub fn decode_pose(payload: &[u8]) -> Option<PoseData>`：
  - 功能：长度校验 `payload.len() != 37` → `payload.len() != 38`；在构造 `PoseData` 时追加 `rtk_fixed: payload[37] != 0`。
  - 注意：**不兼容旧 37 字节帧**（旧帧会被拒绝），需全队同步升级。

- 修改测试（`#[cfg(test)] mod tests`，同一文件）：
  - `test_pose_roundtrip`：`PoseData { ... }` 两个字面量补 `rtk_fixed` 字段；`assert_eq!(encoded.len(), 37)` → `38`；`decode_pose(&encoded[..33])` 截断用例可保留或改为 `[..37]` 验证旧 37B 帧被拒。
  - `test_frame_with_message`：`PoseData` 字面量补 `rtk_fixed` 字段。

#### 6.1.3 `pleiades-base/src/Robot/core/protocol/frame.rs`

- **零改动**（确认项）：RTCM 封装**不新增** `encode_rtcm`，直接复用现有：

```rust
pub fn encode_frame(msgid: u16, sysid: &[u8], compid: u8, payload: &[u8]) -> Vec<u8>
pub fn decode_frame(bytes: &[u8]) -> Option<Frame>
```

- 功能：外层 ORION 帧（magic/len/seq/sysid/compid/msgid/payload/checksum）不感知 payload 内容，RTCM 帧作为 payload 原样装入即可（双层帧）。阶段 1 调用形式：`encode_frame(MSGID_RTCM, &sysid, COMPID_GROUND_STATION, &rtcm_frame)`。

#### 6.1.4 `pleiades-base/src/Robot/core/state.rs`

- 修改 `pub struct RobotState`：在 `z: f32` 之后追加字段：

```rust
/// RTK 固定解标志：LG290P 实时写——FIXED 时 true、失锁/非 FIXED 时 false；未启用 RTK 恒 false
pub rtk_fixed: bool,
```

（`#[derive(Default)]` 自动给 `false`，无需手写。）

- 修改注释（3 处，与字段同步）：
  - 文件头「单一写入者不变式」注释：x/y 写入者从「STM32 里程计」改为「STM32 里程计 + LG290P RTK（共同写 x/y，读-改-写均在同一把写锁内）」；z 保持「设备自己写（车恒 0，机写飞控 EKF），RTK 不更新 z」。
  - `x` 字段注释（当前「由 STM32 RX 回调中的 odometry::accumulate 直接积分维护」）：改为「由 STM32 RX 回调 odometry::accumulate 积分 + LG290P RTK_FIXED 覆盖」。
  - `z` 字段注释：补充「LG290P 忽略 U 分量，z 不更新（D11）」。

#### 6.1.5 `pleiades-base/src/Robot/core/robot.rs`

- 修改私有函数 `async fn state_notifier(state: Arc<RwLock<RobotState>>, pose_tx: broadcast::Sender<Pose>, node_handle: Option<Arc<NodeHandle>>, _peer_name: String, local_peer_id: Option<Vec<u8>>, execute_state: Arc<RwLock<ExecuteState>>, cancel: CancellationToken)`：
  - 快照元组（约 L219-223）从 `(x, y, z, yaw, vx, vy, sub_target)` 改为 `(x, y, z, yaw, vx, vy, rtk_fixed, sub_target)`，`rtk_fixed` 取自 `s.rtk_fixed`（与现有 x/y/z 同一把读锁，不新增锁）。
  - `PoseData` 构造（约 L233-241）追加字段 `rtk_fixed,`。
  - 本地 `pub struct Pose`（pose_tx 广播）**不变**：rtk_fixed 只进网络 `PoseData`，不进本地广播结构。

#### 6.1.6 `pleiades-base/src/Network/Gossipsub/mod.rs`

- 新增常量（放在 `TOPIC_ROBOT_MAP` 之后）：

```rust
pub const TOPIC_RTK_RTCM: &str = "pleiades/robot/rtcm";
```

- 功能：RTCM 改正数据的 gossipsub topic。

#### 6.1.7 `pleiades-base/src/Network/mod.rs`

- 修改 `pub use Gossipsub::{ ... }`（约 L85-89），在 `TOPIC_ROBOT_POSE, TOPIC_ROBOT_MAP` 后追加：

```rust
TOPIC_RTK_RTCM,
```

- 功能：让终端/ugv 通过 `pleiades_base::network::TOPIC_RTK_RTCM` 引用。

#### 6.1.8 gossipsub 订阅机制改造（config 配置订阅，涉及 3 文件）

- **`pleiades-base/src/Config/config.rs`** — `Network_Config` 结构体（约 L101-113）新增字段：

```rust
pub subscribe_topics: Option<Vec<String>>,  // 额外订阅的 gossipsub topic 列表
```

- **`pleiades-base/src/bootstrap.rs`** — `net_cfg` 构造（约 L156-169）新增映射：

```rust
subscribe_topics: n.and_then(|n| n.subscribe_topics.clone()).unwrap_or_default(),
```

- **`pleiades-base/src/Network/network_service.rs`** — `NetworkConfig` 结构体新增 `pub subscribe_topics: Vec<String>`（`Default` 空）；`Start()` 订阅处（约 L405-411）改为「基础 5 个 + config 追加」：

```rust
let mut topics = vec![ /* 基础 5 个 */ ];
for t in &self.config.subscribe_topics {
    topics.push(gossipsub::IdentTopic::new(t));
}
for t in &topics { /* subscribe */ }
```

- 功能：**不硬编码「谁订阅 RTCM」**；由各节点 config.toml 决定额外订阅哪些 topic。ugv 在 `.config/config.toml` 的 `[Network]` 段配 `subscribe_topics = ["pleiades/robot/rtcm"]`，terminal/uav/sim 不配则只订阅基础 5 个。

#### 6.1.9 `pleiades-base/src/Network/swarm_events.rs`

- 修改 `pub(super) async fn Handle_Gossipsub_Event(&mut self, event: gossipsub::Event)` 内的 topic match（约 L364）：

```rust
super::TOPIC_ROBOT_POSE | super::TOPIC_ROBOT_MAP | super::TOPIC_RTK_RTCM => {
    let _ = self.robot_bus.Publish(Bus_Event::StreamRaw { payload: message.data });
}
```

- 功能：RTCM 走与 POSE/MAP 相同的「ORION 二进制透传」分支，不 JSON 解析。只有 config 里订阅了 `TOPIC_RTK_RTCM` 的节点（ugv）才会收到 RTCM 帧。

> 快照重放**零改动**：`command_handler.rs` 的 `NodeCommand::GossipsubPublish` 分支已统一 `snapshot_cache.Update(&topic, payload)`（L67），阶段 1 用 `Gossipsub_Publish_Try` 发布 RTCM 会自动进入快照，新节点订阅后自动重放最近一帧。

---

### 6.2 阶段 1 — terminal 侧（UM960 基站）

#### 6.2.1 `pleiades-terminal/Cargo.toml`（清单遗漏，需补）

- 新增依赖（config 解析需要，当前 terminal 只有 serde_json 没有 serde/toml）：

```toml
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"
toml_edit = "0.22"
```

#### 6.2.2 `pleiades-terminal/src/config.rs`（新增）

参照 `pleiades-ugv/src/config.rs` / `pleiades-uav/src/config.rs` 的写法（`#[serde(flatten)]` + `toml_edit` 幂等补全）。

- 新增 `pub struct TerminalConfig`：

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct TerminalConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// RTK 基站设备配置（UM960）
    pub um960: Option<Um960Config>,
}
```

- 新增 `pub struct Um960Config`：

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Um960Config {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    pub survey_seconds: Option<u64>,
}
```

- 新增 `pub fn Ensure_Terminal_Config() -> Result<TerminalConfig, Box<dyn std::error::Error>>`：
  - 功能：`config_file_path()` 不存在则先 `pleiades_base::config::Ensure_Config()`，再 `ensure_terminal_section` 幂等补全 `[um960]`，最后 `toml::from_str` 读取。

- 新增 `fn ensure_terminal_section(path: &Path) -> Result<(), Box<dyn std::error::Error>>`：读 → `DocumentMut` → `fill_um960` → 有变化才 `fs::write`。

- 新增 `fn fill_um960(doc: &mut DocumentMut) -> bool`：缺 `[um960]` 段则建表，逐键补默认值（幂等）：`enabled=false`、`port="/dev/ttyUSB0"`、`baudrate=460800`、`survey_seconds=180`。

- 新增测试：`test_ensure_terminal_section_fills_defaults` / `test_ensure_terminal_section_keeps_existing`（照搬 ugv 测试模板）。

#### 6.2.3 `pleiades-terminal/src/device/mod.rs`（新增）

```rust
pub mod um960;
```

#### 6.2.4 `pleiades-terminal/src/device/um960/geo.rs`（新增）

照搬 `uploads/lg290p_enu.py` L27-94 的 `parse_gga` / `ecef` / `enu`，Rust 化（f64 坐标）。

- 新增常量：

```rust
pub const WGS84_A: f64 = 6_378_137.0;
pub const WGS84_F: f64 = 1.0 / 298.257_223_563;
pub const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);
pub const DEFAULT_GEOID: f64 = -9.535;
```

- 新增 `pub struct GgaFix`：

```rust
#[derive(Debug, Clone)]
pub struct GgaFix {
    pub utc: String,
    pub lat: f64,       // 十进制度
    pub lon: f64,
    pub quality: u8,    // 0/1/2/4/5/6/7
    pub satellites: u8,
    pub hdop: f64,
    pub alt_msl: f64,   // 海拔（MSL，米）
    pub geoid: f64,     // 大地水准面差距（米）
    pub age: f64,       // 差分龄期（秒）
}
```

- 新增 `pub fn parse_gga(line: &str) -> Option<GgaFix>`：
  - 功能：`$` 开头 + `*` 切分 → XOR 校验 → 校验失败返回 None；仅接受 `GNGGA`/`GPGGA`；度分（`dddmm.mmmm`）转十进制度；S/W 半球取负；空字段容错；任意数值/下标异常返回 None。

- 新增 `pub fn ecef(lat_deg: f64, lon_deg: f64, height: f64) -> (f64, f64, f64)`：
  - 功能：WGS84 经纬高 → ECEF（米）。`height = alt_msl + geoid`（调用方传入已加 geoid 的高度）。

- 新增 `pub fn enu(base_lat: f64, base_lon: f64, base_height: f64, lat: f64, lon: f64, height: f64) -> (f64, f64, f64)`：
  - 功能：以 base 为原点返回 `(east, north, up)`（米）。实现 = 两个 `ecef` 做差 + 旋转矩阵（照搬 Python L78-94）。

- 新增测试：移植 Python `self_test` 样本（`$GNGGA,...,5,32,0.75,41.438,M,-9.535,M,0.2,0000*70` → `lat≈40.0872346207`、`quality==5`；`enu(40,116,10,40,116,10)≈(0,0,0)`；`enu(0,0,0,0.00001,0,0)` 的 north∈[1.10,1.12]）。

#### 6.2.5 `pleiades-terminal/src/device/um960/rtcm_parser.rs`（新增）

- 新增常量 `pub const RTCM_BUFFER_MAX: usize = 4096;`（缓冲上限，超限丢最旧字节重同步）。

- 新增 `pub struct Rtc3Parser { buf: Vec<u8> }`。

- 新增 `impl Rtc3Parser`：
  - `pub fn new() -> Self` — 空缓冲。
  - `pub fn reset(&mut self)` — 清空缓冲（切 Broadcasting 时丢弃 GGA 残留）。
  - `pub fn feed(&mut self, chunk: &[u8]) -> Vec<Vec<u8>>`：
    - 功能：append chunk → 循环找 `0xD3`（丢弃帧头前的杂散字节）→ 读 `byte1/byte2` 算长度 `((byte1 & 0x03) << 8) | byte2`，完整帧长 = `length + 6` → 攒满即切出完整 RTCM 帧；**不校验 CRC**（CRC 由 LG290P 内部校验）；坏长度/超 `RTCM_BUFFER_MAX` 则跳过一个字节重同步；返回本次切出的所有完整帧。

#### 6.2.6 `pleiades-terminal/src/device/um960/mod.rs`（新增）

模块声明：`pub mod geo; pub mod rtcm_parser;`。

- 新增 `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Um960Phase { Init, Surveying, Stable, Broadcasting }`。

- 新增共享状态：`Arc<std::sync::RwLock<Um960Phase>>`（**std RwLock**：RX 回调是同步闭包且跑在 tokio task 里，`blocking_read` 会 panic，`try_read` 会丢读；std RwLock 短临界区可安全 `.read().unwrap()`）。

- 新增 `struct SurveyState`（仅 RX 回调内部可变状态，不需锁）：

```rust
struct SurveyState {
    started_at: Instant,
    candidates: Vec<GgaFix>,      // 连续 quality==7 的候选，非 7 清空
    last_quality: Option<u8>,
    last_sats: Option<u8>,
    last_log: Instant,            // 30s 健康日志节拍
}
```

- 新增 `pub struct Um960Device`：

```rust
pub struct Um960Device {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    phase: Arc<std::sync::RwLock<Um960Phase>>,
    cancel: CancellationToken,
}
```

- 新增 `impl Um960Device`：
  - `pub fn spawn(port: &str, baudrate: u32, survey_seconds: u64, node_handle: Arc<NodeHandle>) -> Result<Self, String>`：
    - 功能：
      1. `node_handle.Get_Local_Peer_Id().to_bytes()` 算一次 `sysid`（terminal PeerId 字节）。
      2. `cmd_tx_shared = Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>`（mavlink 同款：RX 回调里发 Stable 命令需要 TX 句柄，spawn_port 返回后才回填）。
      3. 调 `spawn_port(port, baudrate, 4096, on_bytes, cancel.clone())`；RX 回调 `on_bytes` 内部：读 `phase` → 按状态处理（Surveying 按行切 GGA、Broadcasting 喂 `Rtc3Parser`）。
      4. 回填 `cmd_tx_shared`；顺序发 Init 三条命令（`UNLOG COM1` → `MODE BASE TIME <survey_seconds>` → `GPGGA COM1 1`，每条 `\r\n` 结尾、`try_send`）→ `phase.write(Surveying)`。
      5. 返回 `Self`。
  - `pub fn shutdown(&self)` — `self.cancel.cancel()`。

- 新增命令常量（模块内）：

```rust
const CMD_UNLOG_ALL: &str = "UNLOG COM1";
const CMD_GGA_OFF: &str = "UNLOG COM1 GPGGA";
const CMD_GGA_ON: &str = "GPGGA COM1 1";
const RTCM_CMDS: [&str; 6] = [
    "RTCM1006 COM1 10", "RTCM1033 COM1 10",
    "RTCM1074 COM1 1",  "RTCM1084 COM1 1",
    "RTCM1094 COM1 1",  "RTCM1124 COM1 1",
];
fn mode_base_cmd(survey_seconds: u64) -> String  // "MODE BASE TIME {n}"
fn um960_command_bytes(cmd: &str) -> Vec<u8>     // cmd + "\r\n"
```

- 新增 RX 内部函数（同步，跑在 `on_bytes` 里）：
  - `fn survey_complete(prev: &GgaFix, curr: &GgaFix) -> bool` — `enu(prev.lat, prev.lon, prev.alt_msl+prev.geoid, curr.lat, curr.lon, curr.alt_msl+curr.geoid)` 的水平距离 `< 0.5` 且 `|up| < 0.5`。
  - `fn handle_surveying(...)` — 按行 `parse_gga`；`quality==7` 推入 candidates，连续两帧 `survey_complete` 且已过 `survey_seconds` → 切 `Stable`；非 7 清空 candidates（保证连续性）；每 30s `info!` 健康日志（`started_at/last_quality/last_sats`）。
  - `fn enter_stable_and_broadcast(...)` — `phase.write(Stable)` → 顺序 `try_send` 7 条（`UNLOG COM1 GPGGA` + 6 条 RTCM，先关文本再开二进制）→ `parser.reset()` → `phase.write(Broadcasting)`。
  - `fn handle_broadcasting(...)` — `parser.feed(chunk)` 切帧，每帧 `encode_frame(MSGID_RTCM, &sysid, COMPID_GROUND_STATION, &rtcm)` → `node_handle.Gossipsub_Publish_Try(TOPIC_RTK_RTCM, orion_frame)`（同步 try_send，满则丢弃；不新增第三个 tokio）。

> 失败策略：Surveying 失败**静默停留**，不重发、不报错（仅 30s 健康日志）。Init 三条命令**每次无条件重发**（UM960 未 saveconfig，掉电恢复默认，不能假设已有配置）。

#### 6.2.7 `pleiades-terminal/src/lib.rs`（修改）

- 文件顶部新增模块声明：`mod config; mod device;`。

- 修改私有方法 `fn spawn_background(&mut self)`：在 `core_bootstrap()` 成功后、`node_handle.set(...)` 之后、`robot_forward_loop` / `event_loop` spawn 之前（或紧随其后，均在同 runtime 内），插入：

```rust
// RTK 基站（UM960）：enabled 才 spawn，失败只告警不拖垮节点
match config::Ensure_Terminal_Config() {
    Ok(cfg) => {
        if let Some(u) = cfg.um960.as_ref().filter(|u| u.enabled.unwrap_or(false)) {
            let port = u.port.clone().unwrap_or_else(|| "/dev/ttyUSB0".into());
            let baud = u.baudrate.unwrap_or(460800);
            let secs = u.survey_seconds.unwrap_or(180);
            match device::um960::Um960Device::spawn(
                &port, baud, secs, Arc::new(boot.node_handle.clone()),
            ) {
                Ok(d) => { /* 持有句柄到线程结束：存局部变量，随 run_headless 生命周期存活 */ }
                Err(e) => godot_warn!("[Kernel] UM960 启动失败: {e}"),
            }
        }
    }
    Err(e) => godot_warn!("[Kernel] 读取 [um960] 配置失败: {e}"),
}
```

- 注意：`Um960Device` 句柄需保存到 `run_headless()` 返回前都存活的局部变量（其 `shutdown`/`cancel` 随线程结束自然释放；GDExtension 无显式 UM960 停机入口，跟随后台线程生命周期）。

---

### 6.3 阶段 2 — ugv 侧（LG290P 流动站）

#### 6.3.1 `pleiades-ugv/src/config.rs`（修改）

- `pub struct UgvConfig` 追加字段：

```rust
/// RTK 流动站设备配置（LG290P）
pub lg290p: Option<Lg290pConfig>,
```

- 新增 `pub struct Lg290pConfig`：

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Lg290pConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
}
```

- 修改 `fn ensure_ugv_section(path: &Path)`：追加 `changed |= fill_lg290p(&mut doc);`。

- 新增 `fn fill_lg290p(doc: &mut DocumentMut) -> bool`：缺 `[lg290p]` 段则建表，逐键补默认值（幂等）：`enabled=false`、`port="/dev/ttyUSB2"`、`baudrate=460800`。

- 修改测试：`test_ensure_ugv_section_fills_defaults` 追加 lg290p 断言；`test_ensure_ugv_section_keeps_existing` 照旧。

#### 6.3.2 `pleiades-ugv/src/main.rs`（修改）

- 顶部追加模块声明：`mod device;`（当前为 `mod bootstrap; mod config; mod ugv;`）。

#### 6.3.3 `pleiades-ugv/src/device/mod.rs`（新增）

```rust
pub mod lg290p;
```

#### 6.3.4 `pleiades-ugv/src/device/lg290p/geo.rs`（新增）

- 与 terminal `device/um960/geo.rs` **内容相同但物理独立**（D3）：`GgaFix`、`parse_gga`、`ecef`、`enu`、`DEFAULT_GEOID`、测试全部照抄一份。签名一致：

```rust
pub fn parse_gga(line: &str) -> Option<GgaFix>
pub fn ecef(lat_deg: f64, lon_deg: f64, height: f64) -> (f64, f64, f64)
pub fn enu(base_lat: f64, base_lon: f64, base_height: f64, lat: f64, lon: f64, height: f64) -> (f64, f64, f64)
```

#### 6.3.5 `pleiades-ugv/src/device/lg290p/mod.rs`（新增）

模块声明：`pub mod geo;`。

- 新增 `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Lg290pPhase { WaitingFirstFix, Tracking }`（2 态状态机，仅 RX 回调内部使用，无需共享锁）。

- 新增 `pub struct Lg290pDevice`：

```rust
pub struct Lg290pDevice {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    cancel: CancellationToken,
}
```

- 新增 `impl Lg290pDevice`：
  - `pub fn spawn(port: &str, baudrate: u32, robot_bus: Arc<EventBus>, state: Arc<RwLock<RobotState>>, origin: (f32, f32, f32)) -> Result<Self, String>`：
    - 功能：
      1. `cancel = CancellationToken::new()`；RX 回调局部变量：`phase = WaitingFirstFix`、`line_buf: Vec<u8>`、`base: Option<GgaFix>`（首次 FIXED 基准，含 lat/lon/height）。
      2. 调 `spawn_port(port, baudrate, 512, on_bytes, cancel.clone())` 得 `serial_cmd_tx`；RX 回调按行切 GGA → `parse_gga` → `quality==4` 才处理。
      3. `tokio::spawn(rtcm_relay_loop(robot_bus.Subscribe(), serial_cmd_tx.clone(), cancel.clone()))`（第 3 个 task）。
      4. 返回 `Self`。
    - 说明：LG290P 默认 Rover 模式，**不发送接收机模式配置**（只读 GGA + 写 RTCM）。`origin` 用于对齐世界原点（`x₀=origin.0, y₀=origin.1`）。
  - `pub fn shutdown(&self)` — `self.cancel.cancel()`。

- 新增 `async fn rtcm_relay_loop(mut rx: tokio::sync::broadcast::Receiver<Bus_Event>, tx: mpsc::Sender<Vec<u8>>, cancel: CancellationToken)`：
  - 功能：订阅 robot_bus → 仅取 `Bus_Event::StreamRaw { payload }` → `decode_frame(&payload)` → `frame.msgid == MSGID_RTCM` 时 `tx.try_send(frame.payload)`（把 RTCM 帧写串口喂 LG290P）；`Lagged` 记 warn 继续、`Closed` 退出、`cancel` 退出。

- 新增 RX 内部函数（同步，跑在 `on_bytes` 里）：
  - `fn handle_gga_line(line: &str, phase: &mut Lg290pPhase, base: &mut Option<GgaFix>, state: &Arc<RwLock<RobotState>>, origin: (f32, f32, f32))`：
    - 功能：`parse_gga`；分两条路径：
      - **FIXED（`quality == 4`）**：`height = alt_msl + geoid`（geoid 非有限则 `DEFAULT_GEOID`）；
        - `WaitingFirstFix`：`*base = Some(fix.clone())`（基准 = **车首次 FIXED 位置**，D10，不需要基站坐标下行），切 `Tracking`，`guard.rtk_fixed = true`；
        - `Tracking`：`(e, n, _u) = enu(base.lat, base.lon, base.height, fix.lat, fix.lon, height)`；`state.try_write()` 成功后 `guard.x = origin.0 + e as f32; guard.y = origin.1 - n as f32; guard.rtk_fixed = true;`（**y 取反**；`u` 忽略、**z 不更新** D11；yaw 不动）；
      - **失锁/非 FIXED（`quality != 4`）**：`state.try_write()` 成功后 `guard.rtk_fixed = false`（x/y 保持不写，仅 flag 反映失锁）；
      - `try_write` 失败 warn 丢弃。

> rtk_fixed 语义（已定）：实时反映——FIXED 写 true，失锁/非 FIXED 写 false（位置 x/y/z 保持不更新）。

#### 6.3.6 `pleiades-ugv/src/ugv/stm32/mod.rs`（写者改造）

- 修改 `pub fn spawn(port: &str, baudrate: u32, car_type: CarType, state: Arc<RwLock<RobotState>>, origin: (f32, f32, f32), forward_speed: i16, turn_speed: i16) -> Result<Self, String>` 内的 RX 回调：
  - 删除 `local_state.x/y/z = origin.*` 注入（x/y/z 源头已移到共享 `RobotState`，由 `Robot::new` 注入 origin；`local_state` 降级为纯传感器 staging）。
  - 顶部加 `let mut pending_dt: Option<f32> = None;`；在 `func == RPT_SPEED` 分支里不再调 `odometry::accumulate(&mut local_state, dt)`，改为 `pending_dt = Some(dt);`。
  - 把 `if let Ok(mut guard) = state_clone.try_write() { *guard = local_state.clone(); }` 替换为「传感器字段照写 + x/y 读-改-写（同一把写锁内）」：

```rust
if let Ok(mut guard) = state_clone.try_write() {
    // 传感器字段照写（Task 24 写者改造：不再全量覆盖 x/y）
    guard.vx = local_state.vx;
    guard.vy = local_state.vy;
    guard.vz = local_state.vz;
    guard.battery = local_state.battery;
    guard.attitude = local_state.attitude.clone();
    guard.gyro = local_state.gyro.clone();
    guard.accel = local_state.accel.clone();
    guard.mag = local_state.mag.clone();
    guard.z = local_state.z;      // 车恒 0（D11：z 保持设备端现状）
    // x/y：读-改-写在同一写锁内，避免与 LG290P 覆盖交错丢增量
    if let Some(dt) = pending_dt.take() {
        odometry::accumulate(&mut *guard, dt);
    }
    // 不写 guard.rtk_fixed（LG290P 独写）
} else {
    warn!("[STM32] try_write 失败，状态更新丢弃");
}
```

  - 同步改造 `pub fn spawn_mock(...)`（测试用 mock，逻辑一致）。
  - 更新 mock 测试 `test_mock_origin_injected`（语义不变但断言从「首帧覆盖写」改为「首帧后 x/y 由共享态 origin + 零位移保持」）。

#### 6.3.7 `pleiades-ugv/src/ugv/slam/odometry.rs`（修改）

- 修改 `pub fn accumulate(state: &mut RobotState, dt: f32)` 的**调用契约与注释**（签名不变）：
  - 功能：仍为 `x += (vx·cos − vy·sin)·dt`、`y += (vx·sin + vy·cos)·dt`。
  - 契约变化：调用点从「RX 回调的 local_state」改为「STM32 RX 回调**已持有写锁的共享 guard**」（`accumulate(&mut *guard, dt)`）；RTK 覆盖后里程计自然从新位置继续 `+=` 积分，无需「定位源切换」。
  - 更新模块头注释：删除「只允许作用于 local_state，不得直接对共享 RobotState 调用」；改为「必须在 STM32 RX 回调的同一把写锁内调用」。
  - 测试保持语义不变（`test_accumulate_world_coords` 直接对 `RobotState` 调用仍合法）。

#### 6.3.8 `pleiades-ugv/src/bootstrap.rs`（修改）

- 修改 `pub async fn ugv_bootstrap(config: &UgvConfig, node_handle: Arc<NodeHandle>, robot_bus: Arc<EventBus>, robot_cmd_frame_rx: mpsc::Receiver<Vec<u8>>, origin: (f32, f32, f32)) -> Result<Arc<Robot>, String>`：
  - 在 `CarDeviceHandler::new(robot.clone(), config.clone(), node_handle.clone(), origin)` 调用处追加参数 `robot_bus.clone()`（`robot_bus` 已是参数，直接传）。

#### 6.3.9 `pleiades-ugv/src/ugv/robot_handler.rs`（修改）

- 修改 `struct CarInner`：追加字段 `lg290p: Option<Lg290pDevice>,`。

- 修改 `pub struct CarDeviceHandler`：追加字段 `robot_bus: Arc<EventBus>,`。

- 修改 `pub fn new(robot: Arc<Robot>, config: UgvConfig, node_handle: Arc<NodeHandle>, origin: (f32, f32, f32)) -> Self` → 增加参数 `robot_bus: Arc<EventBus>`（签名变为 5 参），构造时存储。

- 修改 `async fn start(&self)`（`DeviceHandler` 实现）：
  - 读取 `r.lg290p`：`enabled = lg290p.and_then(|l| l.enabled).unwrap_or(false)`（默认 false，与 chassis/lidar 的默认 true 不同）。
  - enabled 时：`port` 默认 `/dev/ttyUSB2`、`baudrate` 默认 `460800`；`Lg290pDevice::spawn(&port, baud, self.robot_bus.clone(), self.robot.robot_state.clone(), self.origin)`；spawn 失败时**先清理已启动的 stm32/lidar 再 `return Err(e)`**（沿用 lidar spawn 失败的清理模式）。
  - 装配进 `CarInner { stm32, lidar, lg290p, goal_service, executor }`。

- 修改 `async fn shutdown(&self)`：追加 `if let Some(l) = &d.lg290p { l.shutdown(); }`。

---

### 6.4 与已知清单的差异 / 补充 / 风险

**清单遗漏（需一并落地）**：

1. `pleiades-terminal/Cargo.toml` 缺 `serde`（derive）/ `toml` / `toml_edit`（§6.2.1）。
2. 模块声明：terminal `src/lib.rs` 需加 `mod config; mod device;`；ugv `src/main.rs` 需加 `mod device;`；并新增 `src/device/mod.rs`（两个 crate 各一份）。
3. `pleiades-base/src/Robot/core/protocol/messages.rs` 的**单测**需同步改（`PoseData` 字面量补 `rtk_fixed`、`encoded.len()` 37→38）。
4. `pleiades-ugv/src/ugv/stm32/mod.rs` 的 `spawn_mock` 及 `test_mock_origin_injected` 需同步写者改造。
5. `CarDeviceHandler` 需新增 `robot_bus` 字段与 `new` 参数（当前 `Robot` 结构体**不存 robot_bus**，LG290P 订阅 robot_bus 只能从 `ugv_bootstrap` 传入）。

**与 `docs/design_doc/rtk_design.md` 的不一致（以本任务文档为准）**：

- `rtk_design.md` §5.1 写「新增 `encode_rtcm`」，但 `task_24_rtk.md` §4.1 与清单已改为「**不新增函数，复用 `encode_frame`**」——本计划按复用 `encode_frame` 执行（§6.1.3）。

**风险 / 不确定点**：

1. **首次 FIXED 的位置跳变**（已解决）：靠操作约束「车停在 origin(64,64) 直到 FIXED，期间不动」保证首次 FIXED 时 `E−E₀=0`，x/y 正好落在 origin，无跳变。等待期间车不移动是硬约束（task24「同一位置启动」）。
2. **`rtk_fixed` 失锁语义**（已定）：实时反映——FIXED 写 true，失锁/非 FIXED 写 false（位置 x/y/z 保持不更新）。终端据此判断位置可信度。
3. **POSE 38B 的跨层同步**：terminal 的 Rust 侧只透传原始帧；Godot 侧 `MessageParser.parse_orion_frame`（GDScript/C#，不在本 workspace）若硬编码 37 字节 POSE 偏移，需同步升级，否则 `rtk_fixed` 解析会错位/失败。
4. **全队同步升级**：`decode_pose` 改为严格 38B 后，旧固件车的 37B POSE 会被 `cluster_consumer` 拒绝（warn 日志）。需整车/机/终端同版本合入。
5. **std RwLock vs tokio RwLock**：UM960 的 `phase` 计划用 `std::sync::RwLock`（RX 回调同步、跑在 tokio task 内，tokio RwLock 的 `blocking_read` 会 panic）。与文档措辞 `Arc<RwLock<Um960Phase>>` 一致但需明确选 std。
6. **LG290P 串口波特率**：`uploads/lg290p_enu.py` 用 460800，硬件拓扑表也用 460800，本计划默认 460800；若实机是 115200 需改 `[lg290p]` 配置，不改代码。

---

## 七、Pictor（Godot 地面站）配套修改

> Pictor 是独立的 Godot 工程（**外部仓库，不在本 workspace**），通过 `pleiades-terminal` 的 GDExtension 桥接入 libp2p。task24 的 POSE 协议变化需 Pictor 同步升级，否则位姿解析错位。

### 7.1 POSE 协议变化

`PoseData` 由 37 字节 → **38 字节**，末尾新增 1 字节 `rtk_fixed: bool`（0=false，非 0=true）。

```
字段序（38B）：time_boot_ms(4) + x(4) + y(4) + z(4) + vx(4) + vy(4) + yaw(4)
            + valid(1) + sub_gx(4) + sub_gy(4) + rtk_fixed(1)
```

### 7.2 Pictor 需要改的地方

1. **`MessageParser.parse_orion_frame`（GDScript/C#）**：POSE 帧的 payload 长度校验从 37 → 38；末尾新增解析 `rtk_fixed`（`payload[37] != 0`）。
2. **位姿显示（可选）**：根据 `rtk_fixed` 判断位置是否 RTK 固定解，可显示「RTK FIXED / 非 FIXED」状态提示（`rtk_fixed=false` 表示失锁/未启用 RTK，位置可信度低）。

### 7.3 兼容性

- 旧固件（37B POSE）与新版（38B）**不兼容**：`decode_pose` 严格校验 38 字节，旧帧会被拒（`cluster_consumer` warn 日志）。
- **必须整车/机/终端/Pictor 同版本合入**（任一环节停留在旧协议都会导致位姿解析失败）。
