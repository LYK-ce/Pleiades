# Task 13_2: CRDT 地图（多车地图一致性）

> 状态：🟢 **方案收敛（2026-08-12：简化版定案——不做周期对账；新车初始化 = 终端下发全量）+ 步骤 4 已实施**，步骤 5 挂起；📦 已归档（2026-08-12，Pictor 端同步已完成）
> 创建日期：2026-08-10
> 最后更新：2026-08-12
> 父任务：`Task/task_13_multirobot_arch.md`
> 问题池：`Task/robot_review_problem.md`
> 设计参考：`docs/design_doc/multi_robot_map.md`（§6 我们的方案）、`docs/design_doc/multi_robot_control.md`（坐标系约定）、`调研报告_分布式地图一致性方案.md`、`调研报告_增量补充_多机地图CRDT同步.md`

## 背景

- Task 13_1 时 MAP_DELTA 入站处理被挂起，指示后续走 **CRDT 地图重构**（wb_13_1 决策 4）
- 现状缺口：gossipsub 快照只重放**最近一帧 map_delta**（增量），新入网车拿不到全图；入站地图合并不存在
- 本质问题：多车**共同构建同一张地图** = 分布式系统的一致性问题（最终一致性，非瞬时一致）

## 目标

多车协作构建同一张地图：每车本地维护 own 贡献表，通过**增量广播（车对车）+ 周期性对账（终端聚合）**，所有副本最终收敛到一致状态。

---

## 方案定案（2026-08-11 讨论收敛）

### 总体结构

```
每车维护：
  own（full map delta）— 本车观测累积贡献，i8，clamp ±8
  full map              — 当前合并地图，平时增量维护

平时（高频）：增量车对车广播
  观测 → own += 观测(clamp ±8)；full map += 本车变化
  格子数值实际变化(Δ≠0) → 广播增量 Δ = s_new − s_old
  收到增量 → full map ← clamp(full map + Δ)

对账（低频）：终端聚合（方案 C）
  每车：own 整表 → 终端
  终端：merged = clamp(Σ 各车 own)
  终端：完整地图 → 所有车（替换各自 full map）
```

### 关键语义

1. **广播的是变化值（Δ），不是观测量（绝对状态）**——接收方做加法，不重放
2. **Δ≠0 才广播**：饱和格（±8）静默，带宽随时间衰减（地图越熟，广播越少）
3. **状态翻转信息必然在变化过程中广播**（0→8 的若干次 +3/+2 不会被吞）
4. **收敛靠对账兜底**：增量丢包造成的偏差，时间上界 = 对账周期
5. **无 per-sender 槽**：聚合在终端完成，车端只有 own + full map
6. **Δ 为 clamp 后实际差分**：如 6→8 时 Δ=+2（clamp 吃掉 1），接收方应用差分 + clamp 仍精确重放发送方路径

### 数值（沿用 grid.rs 现状，不改）

| 参数 | 值 | 说明 |
|---|---|---|
| 网格 | 256×256，0.5m/格 | 覆盖 128m×128m |
| 增量 | 命中 +3 / 掠过 −1 | 不对称 3:1（一次命中抵三次掠过） |
| clamp | [−8, +8] | i8 存储 |
| 阈值 | >+6 Occupied，<−6 Free | 中间 Unknown |
| 判定 | 3 次命中 → Occupied；7 次掠过 → Free | 从 0 出发 |

- 多车观测同一物体 = 独立观测累加（2 车各 1 次 = 1 车 2 次），语义自洽
- clamp ±8 有界 = 防多车重复计数过度自信
- clamp 与阈值间距 = 2：饱和格 2 次反向观测即回 Unknown（动态性较强，静态环境可接受）

### 为什么选方案 C（终端聚合）

| 维度 | 对比结论 |
|---|---|
| 实现复杂度 | 最低：终端一个 Σ 函数，车端无槽、无环拓扑 |
| 对账延迟 | 2 跳（收集 + 分发） |
| 带宽 | 每车 1 发 1 收 64KB/轮 |
| 断链容错 | 无需（每车只连终端） |
| 终端天然存在 | WS 地面站即是聚合点 |
| 附加值 | 终端持有全局视图，可校验一致性 |

> 备选方案（已讨论，未采用）：方案 A 槽式（每车存 N 槽，完全分布式，但内存/复杂度高）、方案 B 链式两圈（无中心但 2N 跳延迟 + 断链容错）。详见 `multi_robot_map.md` §6.4。

---

## 消息协议改动

| 帧 | 现状 | 改后 |
|---|---|---|
| MAP_DELTA（msgid=3） | entry `(gx: i32, gy: i32, state: u8 三态)` | entry `(gx: i32, gy: i32, delta: i8)`（Δ 差分，截断 ±8），9B 不变 |
| MAP_FULL（msgid=2） | data = `state_bytes()` 三态 65536B | data = **i8 log-odds 原始值**（65536B）——own 整表上传/下发的可累加值 |
| 新增 | — | **`decode_map_full`**（当前不存在，只有 encode）；msgid 复用 vs 新增待决策（见待决策 3） |

---

## 代码现状与改动点（2026-08-11 子 agent 梳理）

### 现状一句话

- **发侧完整**：slam_task 200ms 广播 MAP_DELTA（三态，`robot.rs:310-323`）；WS 连接时发一次 MAP_FULL（三态，`server.rs:140-155`）；快照重放地基已就绪（`command_handler.rs:44-57` + `swarm_events.rs:301-311`）
- **收侧为零**：`consumer.rs:71-72` 处 `msgid != MSGID_POSE` 直接 return——MAP_DELTA/MAP_FULL 入站被静默丢弃（注释"留 CRDT 重构"）

### 核心数值缺口（grid.rs）

| 需求 | 现状 | 缺口 |
|---|---|---|
| 周期差分 Δ | `update()` 只返回宏观态变化 `(bool, u8)`，丢弃 log-odds 数值差 | `update()` 需返回 Δ = new_log − old_log |
| 整表 log-odds 导出 | 只有 `state_bytes()`（三态） | 新增 `log_odds_bytes() -> Box<[i8;65536]>` |
| 整表 log-odds 导入 | 无 | 新增 `from_log_odds_bytes()`（终端下发替换） |
| own 贡献表 | 无 | 复用 `Chunk` 结构（65536×i8） |
| merged 地图 | 无 | 新增（own + Σ 远端增量，截断有界） |

### 协议层硬缺口

- **`decode_map_full` 不存在**（`messages.rs` 只有 encode L98-116）——终端上传 own / 车端接收下发全量都需要

### 其他改动点索引

| 文件 | 位置 | 改动 |
|---|---|---|
| `messages.rs:33-37` | MapDeltaEntry | `state: u8 → delta: i8` |
| `robot.rs:278-344` | slam_task | 广播内容 三态 → Δ 差分；节奏 200ms → 低频打包（周期待定） |
| `robot.rs:310-323` | slam_task gossip | 发布内容改 Δ 差分；FULL 全量低频（周期待定） |
| `consumer.rs:60-99` | handle_frame | 放行 MAP_DELTA/MAP_FULL（移除 L71-72 丢弃） |
| `server.rs:140-155` | WS map_full | 数据源 `state_bytes()` → `log_odds_bytes()` |
| `server.rs`（新增） | 终端聚合 | 收集各车 own → `clamp(Σ own)` → 下发全量替换 |
| `WebSocket/mod.rs:16-37` | start 签名 | 可能需要新增通道（own 上传/全量下发） |

---

## 实施步骤

### 已定步骤（2026-08-11 讨论，含子 agent 审查修正）

> ✅ **实施状态（2026-08-11）:三步已实施完成 + review 修复轮**——`cargo test --lib robot` **63/63 通过**（原 55 + 新增 8 项：grid 层 4 项 Δ/own/roundtrip + robot 层 4 项聚合纯函数）。
> 改动文件:`grid.rs`、`slam/mod.rs`、`lidar_mapper.rs`、`messages.rs`、`robot.rs`、`WebSocket/server.rs`、`docs/design_doc/orion_protocol.md`(§3.2/3.3 同步)。
> 待办:① Pictor 终端按 `multi_robot_map.md` §6.7 升级;② 后续步骤(4. 入站处理 / 5. 终端聚合对账)
> review 修复记录:聚合 clamp ±8 已移除(真实差分,接收方再 clamp);聚合抽纯函数 `accumulate_pending`/`drain_pending` + 4 测试;WS full map 数据源用 own 表;orion_protocol §3.3 表述同步

**第一步：存储数据修改（`grid.rs`）**
- `update()` 返回 **Δ = new_log − old_log**（数值差分），**且同时更新 chunk 与 own 表**（own = 本车观测累积贡献，复用 Chunk；Δ 从 own 取差分）
- ⚠️ `update()` 签名变更 `Option<(bool,u8)> → Option<(bool,u8,i8)>` 是**破坏性变更**（非纯新增）——调用点：`lidar_mapper.rs` L57/L82（二元组解构改三元）、`grid.rs` 测试 `test_probabilistic_update`
- 新增 `log_odds_bytes() -> Box<[i8; 65536]>`（整表 i8 导出）
- 新增 `from_log_odds_bytes(data: &[u8]) -> Option<Box<[i8;65536]>>`（长度 ≠ 65536 拒绝、越界返回 None）
- 三态仍读时派生（一份存储，不改现有架构）
- 表述修正：**单车自车行为零影响（状态机/寻路/单机地图不变），但对外协议与终端显示会变**

**第二步：广播内容改造（gossipsub，五文件）**
- ⚠️ Δ 数据是**三层结构传导**，需同步修改，只改 `MapDeltaEntry` 无法编译：
  ```
  grid::Delta{state:u8}(grid.rs:42-47) → robot::MapDelta{state:u8}(robot.rs:51-55)
    → protocol::MapDeltaEntry{state:u8}(messages.rs:33-37)  ← 全部改 delta:i8
  ```
- 涉及文件：`grid.rs`（Delta 结构）、`lidar_mapper.rs`（收集 L57-58/L82-83）、`robot.rs`（MapDelta L51-55 + slam_task L315-334）、`messages.rs`（MapDeltaEntry + encode L84 push / decode L108 `as i8`）、`server.rs`（WS 转发器 L87-103）
- 发送条件：**Δ≠0 才广播**；5 帧内按格聚合**净变化**（**真实差分，不做 ±8 clamp**——窗口净变化上界 ±16 在 i8 内；接收方应用时再 clamp ±8 完成精确重放），净 0 不发
- 广播节奏：slam_task 计数器（loop 前声明，每帧 `wrapping_add(1)`），**模 5 = 0 时发一次**（200ms × 5 = 1s）；`tokio::time::interval` 首次 tick 立即触发已处理（计数器从 0 起、先 +1 再判，首帧不发送）
- 聚合已抽纯函数 `accumulate_pending` / `drain_pending`（可单测）；每 5 帧**无条件 clear**（LiDAR 停转也无滞留）
- **发送层统一节流（2026-08-11 已定）**：计数器加在 slam_task 发送层，`map_tx.send`（WS 链路）与 gossip 发布**都走模 5 = 1s 一次**（两条链路一致）；协议文档 §3.3 频率标注同步更新
- 测试：`lidar_mapper.rs` `test_update_basic` 断言反转（Δ 语义下首帧即有 Δ≠0）

**第三步：WS full map 内容改造（给终端，复用 msgid=2）**
- 数据源：`server.rs:149` `state_bytes()`（三态）→ **`log_odds_bytes()`（own 增量历史 i8）**
- **msgid 决策（2026-08-11 已定）：复用 msgid=2**——msgid=2 的 data 语义从三态 0/100/255 变为 log-odds i8 −8~+8；Pictor 需按阈值 ±6 派生三态
- ⚠️ 类型转换：`log_odds_bytes()` 返回 `Box<[i8;65536]>`，`encode_map_full` 接收 `&[u8]`——逐字节 `as u8`（位模式一致）
- 发送时机暂不变（连接时一次）；周期发送暂不考虑；接收端处理暂不考虑
- 依赖第一步的 `log_odds_bytes()` 且依赖 own 被第一步正确写入（否则发全 0）
- **Pictor 对齐清单见 `multi_robot_map.md` §6.7**（本地 log-odds 缓冲表 + FULL 初始化 + DELTA 累加 + 显示派生）

**跨步联动约束（审查新增）**
- ⚠️ **步骤 2 与 3 必须同批发布**：WS 的 MAP_DELTA（msgid=3）与 MAP_FULL（msgid=2）同一协议族，分开发布会导致终端拿到"三态 full + Δ delta"无法叠加
- ⚠️ Pictor 升级与车端同批（复用 msgid=2 无兼容过渡期）

### 方案二定案：新车初始化 = 终端下发全量（2026-08-12）

**问题**：晚启动/新接入的车收不到入网前其他车的 gossip Δ（gossipsub 不重放历史）→ 缺历史贡献，必须靠 FULL 初始化。

**选型对比**：

| 维度 | 方案一：车对车广播 FULL | **方案二：终端下发全量（选定）** |
|---|---|---|
| 权威性 | ❌ 无权威——各车 merged 各有偏差，新车收 N 份选哪份？ | ✅ 单一权威——终端是全局视图持有者（Σ 各车 own + Δ 流） |
| 实现门槛 | ❌ gossipsub 无点对点请求-响应；Robot 层无"新车加入"事件；需新消息类型 | ✅ 复用 msgid=2；车端零件现成（decode_map_full + set_log_odds） |
| 消息量 | ❌ N 车同时广播 64KB×N | ✅ 一次 64KB |
| 架构一致性 | 偏离方案 C 终端中心 | ✅ 与方案 C 一致 |

**链路**：新车接入 WS → 终端把全局图（Σ 各车 own + Δ 流）→ `MAP_FULL` 下发 → 车端 WS 入站就地处理（`decode_map_full` → `set_log_odds` 替换 merged，own 保留）→ 已实施（`server.rs` `handle_map_full`）。

**方向语义**：车→终端 = 接入时上报 own 整表（现有）；**终端→车 = 新车接入下发全量（新增）**；车↔车无 FULL（车启动=入网、own 从 0 开始、Δ 流即完整历史）。

**降级**：终端不在线时新车无法初始化全量（收 Δ 流缺历史）→ 已知边界 #3。

**Pictor 配合**：新车接入事件 → 编码下发全局全量（encode_map_full）；解析升级同批发布。

### 后续步骤

4. ✅ **入站处理（2026-08-12 实施完成）**：`consumer.rs` 放行 MAP_DELTA → `OccupancyGrid::apply_delta` 累加 merged（只写 chunk，own 不碰）；MAP_FULL 走 WS 入站就地处理（`decode_map_full` → `set_log_odds` 替换 merged）
5. ⏸️ **终端聚合对账（方案 C）挂起**（2026-08-12 人类决定）：不做周期对账；新车初始化改为**方案二（终端下发全局全量）**；对账后续再考虑

#### 已知边界（2026-08-12 收敛，接受并标注）

| # | 场景 | 后果 | 缓解 |
|---|---|---|---|
| 1 | 重连/车重启 | 终端侧 own 重复计数（双倍） | 后续终端按 peer_id 记账 |
| 2 | 丢包 | 偏差永久残留（无对账兜底） | 后续周期对账 |
| 3 | 终端不在线 | 新车无法初始化全量（降级：收 Δ 流缺历史） | 标注 |
| 4 | 晚启动车 | 缺入网前历史 Δ | 方案二：接入终端时下发全量补全 |
| 5 | FULL 替换吞在途 Δ | 替换瞬间的在途增量被抹除（若对方格饱和则跨多轮才愈合） | 首版接受，后续对账周期取小值 |
6. ✅ **单测（2026-08-12）**：增量重放（apply_delta ×4）/ 整表 roundtrip（decode_map_full）/ consumer 入站（MAP_DELTA 应用 + own 不变 + 未知 msgid）；终端聚合全流程留步骤 5
7. ✅ **文档同步（2026-08-12）**：`orion_protocol.md` §3.2（接收语义/方向语义）、本任务状态、wb_13_2
8. ✅ **验证（2026-08-12）**：`cargo check`（27 存量警告不变）+ `cargo test --lib robot` **69/69** + `cargo build --release --bin orion-robot` ✅

### 实施记录（步骤 4，2026-08-12）

**改动文件（7 个代码 + 3 个文档）**：`grid.rs`（apply_delta + 包装 + 4 测试）、`messages.rs`（decode_map_full + 测试）、`protocol/mod.rs`（re-export）、`consumer.rs`（grid 参数 + MAP_DELTA 分支 + 测试改造 6 个）、`robot.rs`（spawn 传 grid）、`WebSocket/protocol.rs`（parse_orion_frame 改收 &Frame）、`WebSocket/server.rs`（入站 msgid==2 就地处理 handle_map_full）；文档：本任务、wb_13_2、orion_protocol.md §3.2（含"先 own 后 FULL"时序约定）。

**验证**：`cargo check` ✅（27 存量警告，无新增）→ `cargo test --lib robot` **69/69** → `cargo build --release --bin orion-robot` ✅

**Commit**：`1e712a8`（Pleiades-Orion，已推送 origin）

**遗留**：① Pictor 端同步（帧解析升级 4 项 + 全局图维护/新车下发 2 项，外部仓库，需人类协调）；② 双车联调（车对车增量互通 + 新车接入初始化验证）；③ 步骤 5（周期对账）挂起

---

## 待决策

1. ✅ ~~增量广播/周期对账~~ → **已定（2026-08-12）**：保留高频增量；**不做周期对账**（挂起，后续再考虑）；新车初始化 = 终端下发全量（方案二）
2. ✅ **截断上限**：±8 保持现状（间距=2 动态性最好）——已实施
3. ✅ **msgid 复用**：复用 msgid=2（2026-08-11 已定并实施），Pictor 需按新语义解析，无兼容过渡期
4. **D\* 对 Unknown 的策略**（与地图一致性正交，见 `multi_robot_map.md` §6.5）：当路（现状）/ 当墙 / 中间值
5. **终端对账细节**：如何感知车队成员、对账触发时机、对账与增量的时序交互（替换瞬间的在途增量）

---

## 人类评审

<!-- 在此区域写下评审意见 -->
