# task_22_1_rwlock_refactor — 状态锁重构（tokio → std）

> 状态：**已取消**（决策变更：保持 `tokio` 锁 + 异步 caps，见下方「决策变更」）
> Created Date ： 2026-08-20
> Modified Date ： 2026-08-20
> 依赖：`task_22_universal_robot.md` 的前置改造
> 分支：`Universal-Robot`

---

## ⚠️ 决策变更（2026-08-20）：本任务已取消，不换同步锁

经过讨论，**不再换 `std::sync::RwLock`，保持 `tokio::sync::RwLock`，caps 用异步函数（`create_async_function`）**。本任务列出的 #1~#7 锁改造**全部不需要做**（它们对 tokio 锁都不是问题）。

### 为什么不要同步锁、而要异步锁

1. **收益太小**：换 std 锁的唯一收益是「caps 省几十纳秒的 async 开销 + 同步函数简单一点」。这在 50ms 决策周期里占比约 0.0001%，物理上完全无感。

2. **代价太大**：换 std 锁要改现有 Rust 代码里几十处 `.read().await` / `.write().await`，还要拆 #1/#6 的「锁内 await」（std 的 `RwLockReadGuard` 是 `!Send`，跨 await 直接编译失败）、处理 #2 的「SLAM 双写」难题——实打实的复杂度和回归风险。

3. **锁不是只给 Lua 用的**：这些锁被整个 Rust 运行时共用（STM32 RX 回调、state_notifier、slam_task、cluster_consumer、executor、main_loop）。为了 Lua 那几十纳秒，去动 Rust 几十处 tokio 代码，是本末倒置。

4. **tokio 锁才是对口的选择**：`tokio::sync::RwLock` 本来就是给「async 上下文 + 可能长持锁」设计的，现有 Rust 代码用它天然匹配；无竞争时 `read().await` 走乐观路径，开销就是一次 Future poll（几十纳秒）。

### 结论

caps 的读方法（`get_position` / `get_cell` / `get_agents` / `get_path`）用 `create_async_function` + `.read().await`，与现有 `network` / `ml` / `storage` caps 风格一致；发动作（`move_forward` / `turn` / `stop`）用 `create_function`（发 channel 是同步 `try_send`）。

---

> 以下为取消前记录的原方案内容（问题清单 #1~#7 仅作历史存档，不再实施）。

## 一、目标

把 Robot 侧的状态锁从 `tokio::sync::RwLock` 换成 `std::sync::RwLock`，作为「caps 全同步」的前置条件。

**动机**：robot caps（Lua 接口）的读状态、发动作都是纳秒级瞬时操作，用 `std::sync::RwLock` 更高效（无 async 开销），且能让 caps 全部用同步函数（`create_function`），省去 `create_async_function` 的复杂度。

## 二、现状问题（子 agent 调研结论）

直接换 std 锁**不安全**，存在以下问题（按严重程度）：

### 🔴 #1 锁内 await（必须改，否则换 std 锁直接出事）

**位置**：`Src/Robot/core/robot.rs:266-296`（`state_notifier`）

- `let s = state.read().await;`（266）和 `let es = execute_state.read().await;`（268）两个读 guard，**一直持有到 293 行 `nh.Gossipsub_Publish(...).await`**。
- 即 `robot_state` + `execute_state` 的读锁**跨过了一次网络发布**。
- `Gossipsub_Publish` 内部是 `cmd_tx.send(...).await`，通道满（backpressure）时挂起 → 锁被无限期持有。
- 换 std 锁后：`std::sync::RwLockReadGuard` 是 `Send` 的，编译器**不会**拦下"跨 await 持锁"，会静默阻塞整个 tokio worker。

### 🟠 #2 写锁内跑完整 SLAM 建图

**位置**：`Src/Robot/core/robot.rs:358-361`（`slam_task`）

- `grid.write().await` 之后直接 `slam::update(&mut *g, ...)`：Bresenham 射线遍历 + HashSet 去重 + 逐格 `update/decay`。
- 全项目最重的临界区，持锁时间取决于点云数量（几十~几百微秒）。

### 🟠 #3 写锁内跑 D* Lite 规划

**位置**：`Src/Robot/core/robot.rs:488-489`（`main_loop` auto_tick）

- `&mut *mission_queue.write().await` 和 `&mut *execute_state.write().await` 作为 `executor.step()` 实参，两个写 guard **存活到整条 step() 结束**。
- `step()` 内会 `pop_next()` + D* `next_step()/compute_shortest_path()`（重 CPU）。
- 即 `mission_queue` + `execute_state` 写锁被 D* 规划全程持有。

### 🟡 #4 锁内 clone 大对象

**位置**：`Src/Robot/core/robot.rs:477-478`（`main_loop` auto_tick 读快照）

- `grid.read().await; (*guard).clone()`：clone **两个 64KB Chunk = 128KB**。
- `lidar_state.read().await.clone()`：clone 整个 `LaserScan`（几百点≈几 KB，最坏 4K 点≈48KB）。

### 🟢 #5 低风险（可接受，不改）

- `robot_state` clone（~88B）、`stm32`/`lidar` 的 `try_write`：纳秒~微秒，安全。
- `op_mode`（Copy enum，临时 guard）、`cluster_table` 的 upsert/snapshot（小规模）：安全。
- 锁顺序：无死锁级顺序反转，换 std 锁后仍安全。

### 🔴 #6 slam_task 的 robot_state 读锁跨 await（二轮检查新发现，与 #1 同类）

**位置**：`Src/Robot/core/robot.rs:333-340`（`slam_task`）

- `rs`（robot_state 读锁）在 333 取得，340 又 `lidar_state.read().await`，`rs` 一直活到块结束 → 读锁跨 await。
- 换 std 锁后，`std` 的 `RwLockReadGuard` 是 `!Send`，`tokio::spawn` 的 future 跨 await 直接编译失败。
- 修法同 #1：把 robot_state 的读取收进 `{}` 块，取完字段就 drop guard，再拿 lidar_state 锁。

### 🟡 #7 其他补充（二轮检查）

- `consumer.rs:113-121`：grid 写锁内循环 `apply_delta`（单格 O(1)、条目少，轻量，但应列出并注释，避免未来塞大循环）。
- `cluster_info.rs` 全表方法（upsert/get/snapshot/len/is_empty/remove_stale）async → 同步改写，调用点（`robot.rs:355/480`、`consumer.rs:86`、`maintenance.rs:29`）去 `.await`，测试同步改写。
- `stm32/mod.rs:138` 的 `get_state()`：当前无调用者，改同步或删除。

### D* 锁专项

- 通用化后 `get_path` 跨线程访问 D*，需 `std::sync::Mutex<Option<DStarLite>>`。
- `get_path` 内部「move_to + set_dynamic_obstacles + next_step」除 D* 锁外还需读：`robot_state`（move_to 算当前格）、`cluster_table`（动态障碍）、`grid`（cost 查格子）。
- `next_step → compute_shortest_path` 是相对重操作（持 D* 锁做增量重算），但设 goal 低频、Lua 单调用者，可接受——**需在实现处显式标注**。
- 锁序：`D* → {robot_state, cluster_table, grid}`，当前无反向顺序，无死锁。

## 三、改造方案（初步，待讨论）

### #1 拆锁（state_notifier）

把读锁的存活范围收进 `{}` 块，drop guard 后再发布：

```rust
// 锁内只组帧
let (pose, frame) = {
    let s = state.read().unwrap();
    let es = execute_state.read().unwrap();
    // 构造 Pose + PoseData + encode frame
    (pose, frame)
};  // ← guard 在此 drop

// 锁外发布
if let (Some(nh), Some(peer_id)) = (&node_handle, &local_peer_id) {
    let _ = nh.Gossipsub_Publish_Try(TOPIC_ROBOT_POSE, frame);  // 同步版，或用 async 版（已 drop 锁）
}
```

### #2 缩小 SLAM 写锁粒度

方案 A：锁外算 delta，短促写锁应用（推荐，但需拆 `slam::update` 为"算 + 应用"两段）
方案 B：先读锁 clone work copy，锁外算，再写锁合并
方案 C：接受写锁持有 = SLAM 时长，但确认多 worker + 读者可容忍

### #3 拆"算"与"提交"

```rust
// 锁外/短读锁内：算 sub_target + 要 pop 的 mission
let (action, sub_target) = { /* 读状态 + D* next_step，不持写锁 */ };

// 短促写锁：只提交
{
    let mut mq = mission_queue.write().unwrap();
    // pop_next() 提交
    let mut es = execute_state.write().unwrap();
    es.sub_target = sub_target;
}
```

### #4 锁外 clone

```rust
// 锁内只取最小引用，锁外 clone
let g = grid.read().unwrap().clone();  // 若不能接受，改 Arc copy-on-write
```

### 锁类型整体替换

| 锁 | 现状 | 改后 |
|---|---|---|
| robot_state / grid | `Arc<tokio::sync::RwLock<T>>` | `Arc<std::sync::RwLock<T>>` |
| cluster_table 内部 | `tokio::sync::RwLock<HashMap>` | `std::sync::RwLock<HashMap>` |
| D*（DStarLite） | 内嵌 executor，无独立锁 | `std::sync::Mutex<Option<DStarLite>>`（全 `&mut self`，用 Mutex 不用 RwLock） |
| mission_queue / execute_state / lidar_state / op_mode | `Arc<tokio::sync::RwLock<T>>` | **保持 tokio 不变**（caps 不读，无需换） |

`cluster_table` 的方法（`upsert/get/snapshot/len/is_empty/remove_stale`）需从 `async fn` 改同步函数（去 `.await`），调用点（`robot.rs`/`consumer.rs`/`maintenance.rs`）同步去 `.await`。

## 四、涉及文件

| 文件 | 改动 |
|---|---|
| `Src/Robot/core/robot.rs` | #1 拆锁、#2 SLAM 锁粒度、#3 拆算/提交、#4 锁外 clone；各读写点 `.await` → `.unwrap()` |
| `Src/Robot/core/state.rs` | （无锁类型，仅被引用） |
| `Src/Robot/core/cluster/cluster_info.rs` | 内部锁换 std；方法去 async |
| `Src/Robot/core/cluster/consumer.rs` | 调用点去 `.await` |
| `Src/Robot/core/cluster/maintenance.rs` | 调用点去 `.await` |
| `Src/Robot/control/device/stm32/mod.rs` | 读写点去 `.await` |
| `Src/Robot/control/device/lidar/mod.rs` | 读写点去 `.await` |
| `Src/Robot/core/executor.rs` | 若拆"算/提交"，签名调整 |

## 五、实施步骤（待讨论细化）

1. 修 #1（state_notifier 拆锁）—— 最优先，改完即可安全换 std 锁
2. 修 #3（auto_tick 拆算/提交）
3. 修 #2（SLAM 锁粒度）
4. 修 #4（锁外 clone）
5. 整体换锁类型 + 去 async
6. `cargo check` + 行为不变验证

## 六、验证方式

- 每步 `cargo check` 通过
- 换锁后，车行为与改造前一致（走格子、转向、直行、急停、集群广播）

## 七、风险

1. **锁内 await 是硬约束**：换 std 锁后，任何"跨 await 持锁"都会静默阻塞 worker（编译器不拦），必须靠人工审查保证。
2. **std 锁锁内禁止 await**：约定"锁内只允许纳秒~微秒级瞬时操作"，这是后续所有读写的红线。
3. #2/#3/#4 若不同步改，换 std 锁会把"纳秒级快照"变成"几十~几百微秒阻塞"，高并发读时抖动。
