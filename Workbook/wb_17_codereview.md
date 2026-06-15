# wb_17_codereview

## 概述

Task 17 对 Pleiades 全系统进行模块级 Code Review，目标：消除死代码、统一规范、精简接口。按模块层级自底向上推进：

```
Level 0: EventBus ✅ → PeerManagement ✅ → Config ✅
Level 1: Storage → ...
Level 2: Network → VM → ...
Level 3: Orchestrator → TUI/CLI
```

每模块审查后更新设计文档（`docs/design_doc/`），并双卡/三卡测试验证。

---

## 第二轮: 模块级 Code Review (2026-06-15)

### 17.1 EventBus ✅
- 审查结论: 模块简单清晰，薄封装职责合理，无需架构改动
- 修复: mod.rs 过期文档示例、Level 0 标注
- 规范更新: instructions.md 头注释 Date → Created/Modified Date
- 设计文档重写: docs/design_doc/event_bus_design.md

### 17.2 PeerManagement ✅ (第二轮清理)
- 删除: SupportedModel 4死方法、PeerProfile.layer_time/bandwidth_mbps/memory_mb、Get_Peers、Update_Peer_Name、PeerHandle.inner()
  5死trait方法(Contains_Peer/Count/Is_Empty/Get_Peer/Cleanup_Timeout_Peers)、PeerInfo.set_name/is_timeout
- 新增: PeerInfo::update_sessions
- 改名: Update_Local_Sessions → Update_Sessions
- 修改: Upsert_Peer 返 bool、upsert_peer 保留 name、PeerInfo 方法 pub(crate)
- trait: 原17→现10方法 (Get_All_Peers/Get_Local_Peer/Get_Peer_By_Name/Upsert_Peer/Set_Local_Name/Remove_Peer/Update_Profile/Update_Supported_Models/Update_Sessions/Clear)
- 📝 备忘: 5死trait方法 + bandwidth_mbps/memory_mb + is_timeout 之后需加回
- 🔧 待修: 删除 PeerHandle，PeerManager 直接 impl trait（方案B统一架构）

### 17.3 Config ✅
- 删除: dead_code allow、Runtime_Config/Session_Config 结构体、quota_gb 字段、[Scheduler] 段、config.toml 文件
- 修改: include_str! → 硬编码 DEFAULT_CONFIG、mod.rs Level 0、头注释格式
- Config: Pleiades_Config 6段→4段 (Log/Network/Storage/Identity)
- 新增: docs/design_doc/config_design.md
- 测试: 环境4首次初始化 ✅、双卡2节点 ✅、三节点单卡 ✅

### 文件变更
- Src/Config/config.rs: 去 dead_code、硬编码默认配置、删死结构体、头注释
- Src/Config/mod.rs: Level 0、去死 re-export、头注释
- Src/Config/config.toml: 删除（内容合并到 config.rs）

### 文件变更
- Src/PeerManagement/ (5文件): 大幅精简，trait 17→15方法，缩进/头注释统一
- Src/Network/swarm_events.rs: Update_Peer_Name→Upsert_Peer
- Src/Network/command_handler.rs: Get_Peers→Get_All_Peers+filter
- Src/Network/mod.rs: 注释更新
- Src/Orchestrator/core/branch_user.rs: Update_Local_Sessions→Update_Sessions
- Src/Orchestrator/mod.rs: StubPeerManager 同步

---

## 第一轮: 消除硬编码 (2026-06-14)

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
