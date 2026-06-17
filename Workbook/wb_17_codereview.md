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
- 🔧 已完成: 删 PeerHandle、删 StubPeerManager、删 for Arc impl、Network/Capabilities Box<dyn>→Arc<dyn>、工厂简化

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

### 17.4 Storage ✅ (2026-06-15)
- FileState + FileEntry 合并为统一 FileEntry，lock 字段 pub(crate)
- 新建 file_entry.rs（对齐 PeerManagement/peer_info.rs 模式）
- 删除: FileState 结构体、Build_File_Entry 函数、flush_and_sync 函数（死代码）
- 简化: list() = values().cloned().collect()
- 头注释全部标准化: //Date → //Created/Modified Date
- mod.rs 补全 Level 1 标注 + 依赖说明
- 单元测试: 36/36 ✅
- 双卡 2 节点 Qwen3-30B: Pipeline ✅, API ✅, 多轮对话 ✅
- 单卡 3 节点 Qwen3-30B: Pipeline ✅ (B[0-24] C[25-49]), API ✅, 多轮对话 ✅

### 文件变更
- Src/Storage/file_entry.rs: 新建, 33 行
- Src/Storage/capability.rs: -FileEntry 定义, +pub use re-export, 头注释修正
- Src/Storage/storage_manager.rs: -FileState -Build_File_Entry -flush_and_sync, +file_entry import, state→entry 重命名
- Src/Storage/mod.rs: +mod file_entry, +Level 1 文档, 头注释修正
- Src/Storage/guard.rs: 头注释修正

### 📝 待写入设计文档的要点
- Lazy_Discover 只做最小注册（仅 size，模型字段全 None），不调 analyze_model
  原因：analyze_model 会触发 GGUF→PGGUF 转换（读18GB+写hash+删旧文件），acquire_read 不应有这种重操作
- 模型元信息由 flush() 统一填充（analyze_model），职责分离
- Ensure_Entry 也不填元信息（size=0），写入方自己创建文件后由 flush 刷新
- Resolve_Path = Validate_File_Id + Full_Path 合并
- ChecksumAlgorithm 从 capability.rs 移入 file_entry.rs
- 函数统一 Pascal snake case (Acquire_Read, Hash_File 等)
- WriteGuard on_drop 回调自动刷新 size
- 设计文档已重写 (docs/design_doc/storage_design.md)

### 第三轮: 追加 ML_Engine (2026-06-16)
- 本轮追加 ML_Engine (Level 0) 到审查计划
- 更新后的模块列表:
  Level 0: EventBus ✅ → PeerManagement ✅ → Config ✅ → ML_Engine
  Level 1: Storage ✅
  Level 2: Network → VM
  Level 3: Orchestrator → TUI/CLI

- 设计文档已重写 (docs/design_doc/storage_design.md)

### 17.5 ML_Engine 分层分析 (2026-06-16)
- 模块规模: ~25 文件, ~7,800 行, Level 0
- 三层架构:
  底层: device.rs + gguf_tensor.rs + lua_tensor.rs + GGUF_Models/* + gguf_model_manager.rs
  中层: gguf_model.rs (组装中枢) + capability.rs (异步封装) + context.rs (会话管理)
  上层: mod.rs re-export ~30 符号 (MlSession, analyze_model, GGUF_Model 等)
- qwen3.rs 是隐藏"标准库" (Rotary_Embedding/Mlp_Weights/Attention_Weights 被 llama/deepseek_v3 复用)
- 无显式 Model trait, 通过 AnyModel 枚举 match dispatch
- DeepSeekV4 完全独立 (safetensors 分片, 不经过 gguf_model_manager)
- 初步发现: 6 个问题 (无 trait/命名不统一/context.rs 过大等)
- 审查计划: 7 轮, 自底向上
