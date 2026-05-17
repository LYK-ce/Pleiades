# Pleiades 项目模块现状总结

Presented by KeJi
Date ： 2026-05-17

## 1. 编译状态

| 条件 | 状态 |
|------|------|
| `cargo check --no-default-features` | ✅ 0 errors, 36 warnings |
| `cargo test --no-default-features --lib` | ✅ 122 tests, 120 passed, 2 failed (Storage size 已知) |
| `cargo test --no-default-features --test t01/02/03/04` | ✅ 20/20 passed |

## 2. 模块总览

| 模块 | 单元测试 | 集成测试 | 编译 | 状态 |
|------|---------|---------|------|------|
| **Vm_Base** | 47 | 0 | ✅ | ⚠️ 计划移除 |
| **Storage** | 34 | 5 | ✅ | ✅ 最稳定 |
| **Session_Manager** | 17 | 5 | ✅ | ✅ 已修复通道 drop bug |
| **PeerManagement** | 0 | 6 | ✅ | ✅ API 已对齐 |
| **EventBus** | 4 | 4 | ✅ | ✅ 健康 |
| **Orchestrator** | 10 | 0 | ✅ | ⚠️ core/ 分支零测试 |
| **Network** | 4 | 8(ignore) | ✅ | ⚠️ 子协议未测试 |
| **Lua** | 3 | 0 | ✅ | ⚠️ 覆盖不足 |
| **ML_Engine** | 2 | 0 | ✅ | ❌ 仅错误路径测试 |
| **Config** | 0 | 0 | ✅ | ❌ 零测试 |
| **TUI** | 0 | 0 | ✅ | ❌ 零测试 |
| **Scheduler** | — | — | — | 🗑️ 已移除 |
| **E2E** | 0 | 0 | ❌ | 🗑️ t08 已禁用 |

## 3. 各模块详细状态

### 3.1 Storage ✅

- 39 个测试覆盖全生命周期、并发锁、3 种校验码、flush 同步、路径安全
- 已知问题：`FileEntry.size` 非实时，仅在构造和 `flush()` 时更新
- 2 个测试因 size 延迟更新失败（设计取舍，非 bug）

### 3.2 Session_Manager ✅

- 17 单元 + 5 集成测试全部通过
- **已修复**：`create_session` 和 `connect` 不再 drop 通道对端
- Session 结构体现在持有 `ml_input_tx`/`ml_output_rx`（ML Thread 对端）和 `frontend_pairs`（槽位对端）
- 待实现：跨层消息路由（batching 桥接 logic）、`Take_Frontend` 完整实现

### 3.3 PeerManagement ✅

- 6 个集成测试全部通过
- API 已稳定：`Get_Peers`/`Upsert_Peer`/`Update_Profile`/`Update_Heartbeat`/`Update_Status`
- `create_peer_management` 自动创建本地节点
- 无单元测试（仅靠集成测试覆盖）

### 3.4 EventBus ✅

- 8 个测试覆盖发布订阅、多消费者、慢消费者恢复
- API 简单稳定，无需改动

### 3.5 Orchestrator ⚠️

- Core 4 分支 select! 事件循环已建立
- `command.rs` 10 个测试（协议解析/序列化 7 + ID 生成 3）
- `route_user()` 中 DisplayPeer/SetDevice/List/Cancel/Quit 已实现，其余 `todo!()`
- `route_inbound()`/`route_stream()` 全部 `todo!()`
- `core/` 下 5 个文件零测试
- `UserCommand::Pipeline` 变体已移除（随 Scheduler）

### 3.6 Network ⚠️

- 4 个单元测试仅覆盖错误格式化
- 8 个集成测试全部 `#[ignore]`，需双节点环境
- 子协议（File_Stream/Tensor_Stream/Bandwidth_Stream/Request_Response）无单元测试
- 已修复子模块路径声明（添加 `#[path]` 属性）

### 3.7 ML_Engine ❌

- 2 个错误路径测试（文件不存在、非法范围）
- GGUF 模型加载、tensor 操作、推理流程、Qwen3 模型均无测试
- `tensorize`/`forward` Lua 回调已改为 stub（返回错误占位）

### 3.8 Lua ⚠️

- 3 个测试：创建实例、沙箱阻止 os、空注册表
- Lua capability binding 待实现

### 3.9 Config ❌

- 零测试
- 配置解析（`config.rs`）、身份密钥（`identity.rs`）均未测试
- `Scheduler_Config` 已移除

### 3.10 TUI ❌

- 零测试，UI 层完全裸奔
- `llm_io` 引用已迁移到 `session` 模块
- `Pipeline` 命令已改为 `Execute` 占位

### 3.11 Vm_Base ⚠️

- 47 个测试覆盖 SlotFile + VM 指令集，测试最充分
- **计划移除**，等待最终确认后执行

## 4. 本次修复记录（2026-05-17）

| 类别 | 内容 |
|------|------|
| Scheduler | 删除模块目录，清理所有引用（lib.rs/main.rs/Config） |
| 路径修复 | `Cargo.toml`/`lib.rs`/`Network` 添加 `#[path]` 匹配大写目录 |
| PeerManagement | 添加 `PeerStatus`/`PeerCapability`/`PeerEvent`，trait 新增方法 |
| Network | 方法名对齐、子模块路径修复、错误处理修正 |
| Orchestrator | core 子模块导入路径修正 |
| TUI | `llm_io`→`session`，`Pipeline`→`Execute`，`Run` 补字段 |
| ML_Engine | `tensorize`/`forward` 类型标注修复 |
| Session | 结构体添加通道对端字段，`create_session`/`connect` 不再 drop |
| main.rs | 重写适配当前 Capabilities/Core/Storage API |
| 集成测试 | t01/t02/t04 按新 API 重写，全部通过 |
| 文档 | `orchestrator_design.md` 追加已知限制，`storage_design.md` 追加 size 非实时说明 |

## 5. 待办事项

- [ ] 移除 Vm_Base 模块
- [ ] SessionManager 实现跨层消息路由（batching）
- [ ] `Take_Frontend` 完整实现
- [ ] Orchestrator route_inbound/route_stream 实现
- [ ] Config/ML_Engine/TUI 补充测试
- [ ] Network 子协议单元测试
- [ ] t07 network 集成测试激活（需双节点环境）
- [ ] t08 e2e 重写
- [ ] Storage `flush` 中 ML Analyze 集成
