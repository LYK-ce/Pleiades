# wb_17_codereview (2026-06-14)

## 完成项
- [x] 17.1-17.8 Code Review 子任务全部完成
- [x] Pipeline 层分配 Bug 修复 (coordinator 不参与分配)
- [x] Storage 架构调整 (持有 PeerManager + EventBus)
- [x] Flush 统一到 Core 管理
- [x] 双卡 2 节点 30B 测试通过
- [x] 单卡 3 节点 30B 测试通过

## 关键决策
- 组件分层: 低层(EventBus, Config) → 中层(PeerManager, Network, Storage) → 高层(Core, TUI)
- Storage 与 Network 同级，均可调用 PeerManager + EventBus
- Flush 入口统一为 Core，消除 main.rs 中的业务逻辑

## 文件变更
- Src/Config/config.rs: +OnceLock kvcache_dir, +Pleiades_Config methods
- Src/Config/config.toml: +kvcache_dir
- Src/main.rs: 大幅精简 (去 Phase 5.5, 去硬编码, CLI 提取)
- Src/CLI/mod.rs: 新建, ~80 行
- Src/Storage/storage_manager.rs: +peer_manager + event_bus + sync_models
- Src/Orchestrator/core.rs: +do_flush + spawn_initial_flush
- Src/Orchestrator/core/branch_user.rs: Flush handler 简化
- Src/EventBus/event_bus.rs: +Debug
- programs/user/pipeline_coord.lua: 动态层分配(coordinator 不参与)
- programs/user/pipeline_coord_single.lua: 同上
- Architecture/Pleiades_Architecture.md: 更新 Storage/Core/flush 文档
