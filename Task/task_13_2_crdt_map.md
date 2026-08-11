# Task 13_2: CRDT 地图（多车地图一致性）

> 状态：🔵 方案讨论中 + 三步已实施（2026-08-11：对账方案定为**终端聚合（方案 C）**；实施步骤 1~3 已完成，4~5 待实施）
> 创建日期：2026-08-10
> 最后更新：2026-08-11
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

### 后续步骤（待讨论）

4. 入站处理：`consumer.rs` 放行 MAP_DELTA/MAP_FULL，Δ → merged 累加，FULL → merged 替换
5. 终端聚合对账（方案 C）：WS 侧收集各车 own → `clamp(Σ own)` → 下发全量替换 merged
6. 单测：增量重放 / 饱和吸收 / 整表 roundtrip / 协议 roundtrip / 终端聚合全流程 / consumer 入站
7. 文档同步：`orion_protocol.md` §3
8. `cargo check` + `cargo test --lib robot` + build

---

## 待决策

1. ~~增量广播是否保留~~ → **已定：保留高频增量 + 低频对账（方案 C）**（2026-08-11）；对账周期仍待定（候选 5s~30s）
2. **截断上限**：±8（保持现状，间距=2 动态性最好）vs ±20（更保守）
3. **msgid 复用 vs 新增**：MAP_FULL 改 i8 后复用 msgid=2 会影响 WS/Pictor 旧解析，倾向新增 msgid
4. **D\* 对 Unknown 的策略**（与地图一致性正交，见 `multi_robot_map.md` §6.5）：当路（现状）/ 当墙 / 中间值
5. **终端对账细节**：如何感知车队成员、对账触发时机、对账与增量的时序交互（替换瞬间的在途增量）

---

## 人类评审

<!-- 在此区域写下评审意见 -->
