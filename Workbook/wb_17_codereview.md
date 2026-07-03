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

### Round 1: device.rs ✅ + qwen3 公共基础设施提取 (2026-06-17)
- device.rs: 头注释标准化 + Parse_Device_Str 命名修正
- 创建 common/ 目录, 提取 GGUF_Reader (后删), Rotary_Embedding, Mlp_Weights
- GGUF_Reader 确认为死代码, 删除; mlp.rs 的 New() → New_From_Extracted()
- qwen3.rs: 删 ~120 行死代码 (New(gg) ×3 + From_GGUF_Reader)
- qwen3_moe.rs: 新增 From_Extracted() (~90行), 消除 gguf_model.rs 中 ~85 行内联构造
- gguf_model.rs: 清理 Mlp_From_Tensors + MoeOrMlp/FusedMoeGGUF/Linear import
- DeepSeek/Llama: 用 #[cfg(feature)] 门控, 代码保留不编译
- 函数命名: clear_kv_cache → Clear_Kv_Cache (消除重复)
- 单元测试 15/15 + 双卡2节点 + 单卡3节点 全部通过

### 项目搬迁到 GPFS (2026-07-01)
- 从 /root/code/Pleiades 搬至 /vepfs-mlp2/c20250205/240804016/Workspace/Pleiades
- 原因: 本地 overlay 磁盘仅 20G (83%), GPFS 有 639T 可用
- 创建 build.sh 脚本, 自动处理 GCC 13 / nvcc 不兼容 (PATH 注入 gcc-12)
- 编译速度: GPFS ~68s vs 本地 ~50s (可接受)
- Orion + llama.cpp 同步搬迁, 本地空间 3.4G→5.7G
- instructions.md 已更新项目路径和 build.sh 使用说明

### 17.5 当前进度 (2026-07-03)
- Round 1 ✅ (device.rs + common/ + qwen3死代码 + qwen3_moe统一)
- Round 2 ⏸ 待开始 (qwen3.rs 剩余: Attention/Layer/Model/Config)
- Round 3-6 待开始

### Round 2a: common/ + 统一 Build_From_Extracted (2026-07-03)
- rope.rs: 审查通过，无改动
- mlp.rs → swiglu_mlp.rs: 文件改名（SwiGLU MLP）
- Mlp_Weights::New_From_Extracted → Build_From_Extracted
- qwen3.rs: Layer_Weights::From_Extracted → Build_From_Extracted
  - 手动构造 MLP → Mlp_Weights::Build_From_Extracted（消除重复）
  - 移除未用 Activation import
- qwen3_moe.rs: Qwen3MoE_Layer::From_Extracted → Build_From_Extracted
  - Mlp_Weights::New_From_Extracted → Build_From_Extracted
- gguf_model.rs: 两处调用点同步改名
- 📝 备忘: Mlp_Weights 结构体名后续需改名（SwiGLU_MLP_Weights），act_fn 字段始终 Silu 需处理
- 📝 备忘: 升级 candle 0.11.0 后可用 #3598 的 enable_cuda_graph_htod_cache 机制，在 MlSession 层实现 warmup→capture→replay（非单个 MLP::forward 内），提升 decode 吞吐 ~10%
- rope.rs: 删无意义的本地变量别名 (dim → head_dim, max_seq_len → max_position_embeddings)
- 📝 备忘: 当前 Model_Weights::Forward 用 Option + if 判断 embed_tokens/norm/lm_head 是否存在。仅两类变体（完整/部分模型），够用。未来支持更多架构（DeepSeek MLA、不同输入输出组合、>5 种 stage）时应引入 `trait Stage { fn forward(...) }` 统一为 `Vec<Box<dyn Stage>>` 消除分支。当前性能无差异（分支可预测 vs vtable）。
### Round 2b: qwen3.rs 审查完成 (2026-07-03)
- Attention_Weights: 新增 Build_From_Extracted，消除两处内联构造重复
- Layer_Weights: Build_From_Extracted 改为调用子组件 Build 方法；删除 Take_Qmatmul
- Model_Weights: From_Dynamic → Build_Model；删除重复的 clear_kv_cache（死代码）
- 清理孤儿注释、未用 import（Activation, ConcatKvCache）
- Qwen3_Config: 审查通过，无改动
### Round 2c: qwen3_moe.rs + mod.rs 审查完成 (2026-07-03)
- qwen3_moe.rs: 头注释标准化（Date → Created/Modified），删除不必要的 #![allow(dead_code)]
- mod.rs: 修正注释措辞（"暂时移除" → "条件编译（功能门控）"），更新日期
- gguf_tensor.rs: 已删除（253行死代码，被 Network/Tensor_Stream/protocol.rs 替代，全项目无调用方）
### gguf_model.rs 重构 (2026-07-03)
- gguf_model.rs → gguf_model_legacy.rs（保留 DeepSeek/Llama 参考）
- 新 gguf_model.rs（494行，-31%）：仅 Qwen3/Qwen3MoE
  - 移除所有 #[cfg(feature)]、DeepSeek/Llama import、is_deepseek 分支
  - 提取 Load_Output_Head、Build_GGUF_Model 共用函数
  - LayerWithAttention trait 统一 extract/restore_kv_cache
- GGUF_Models/mod.rs: 注释掉 deepseek_v3/deepseek_v4/llama 模块声明
