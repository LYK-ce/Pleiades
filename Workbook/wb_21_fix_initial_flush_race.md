# wb_21_fix_initial_flush_race.md

## 任务
修复"初始 flush 一次性事件早于 EventBus 订阅导致 TUI 本机不显示"的时序竞态（先订阅再 flush）。

## 背景（2026-08-09）
- 症状：orion-robot（Pleiades-Orion 分支）启动后 TUI Network 面板无 🏠 本机条目
- 根因：`spawn_initial_flush` 在 `Subscribe()` 之前触发；EventBus 是 broadcast 无历史重放（event_bus.rs Publish 注释"无订阅者静默丢弃"），本机 peer_info_updated 一次性事件必然丢失
- 远端节点不受影响（mDNS/gossipsub 持续事件流）
- **ML_review 分支评估**：结构相同但实际不触发（Subscribe 同步紧跟 spawn，flush 异步首次 poll 即让出，窗口 µs 级）——预防性加固

## 实施（2026-08-08→09，ML_review 分支，2 文件）

### 1. Src/Orchestrator/core.rs
`spawn_initial_flush(caps: Arc<Capabilities>)` 关联函数 → `spawn_initial_flush(&self)` 实例方法（函数体不变，内部 `self.capabilities.clone()`）

### 2. Src/main.rs
- 删除 Phase 5 末尾旧调用（原 L109-110）
- CLI 分支：`spawn_stdout_subscriber`（内部 Subscribe）之后插入 `core.spawn_initial_flush()`
- TUI 分支：`event_bus.Subscribe()` 之后插入 `core.spawn_initial_flush()`

### 安全依据（核实）
- 调用点唯一：全库仅 main.rs（grep 确认）
- 借用顺序：`&core.spawn_initial_flush()` → `core.run().await`（mut self move），先借后移合法
- do_flush 仍被 branch_user.rs 手动 flush 使用，不受影响

### 编译验证
`./build.sh check` ✅ exit=0，0 error，22 个预存 warning（main.rs/core.rs 无新增）

## 顺带分析（未动手）
"flush 后置 → gossipsub 首轮广播不再空投"**不成立**：network Start() 已 spawn 但 gossipsub 3 topic 订阅在其内部，首轮 publish 仍可能 NoPeersSubscribedToTopic。真正兜底 = Task 20 快照机制（已实测 B/C 后启动拿到 A 快照）。

## 状态
- ✅ 实施完成 + 编译通过
- 待办：推送（等用户确认）；robot 分支（Pleiades-Orion）按同方案实施
