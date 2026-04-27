# Orchestrator 后续实现计划

**日期**：2026-04-23（最近更新：2026-04-27）
**基线**：Phase 2B 完成 + 指令集/编译器/handler_network 架构扩展

---

## 1. 当前已完成清单

| 模块 | 文件 | 状态 |
|------|------|------|
| Core 主循环 | `core.rs` | ✅ run() + select! + lifecycle 通道 |
| Job 公共契约 | `job.rs` | ✅ JobId/JobKind(Run,Coordinator,Relay)/JobState/LifecycleEvent |
| Command 定义 | `command.rs` | ✅ UserCommand/NetworkCommand |
| Slot 系统 | `slot.rs` | ✅ SlotId/SlotValue(含IoHandle)/SlotFile/ConstValue |
| 指令集 | `instruction.rs` | ✅ 12 条指令（数据2 + 推理5 + 网络3 + 控制2） |
| TaskEngine | `executor/task_engine.rs` | ✅ step() 路由全部 12 条指令 + ExecutionMode + 5 个测试 |
| JobExecutor | `executor/mod.rs` | ✅ run() + 补偿循环 + 3 个测试 |
| handler_data | `executor/handler_data.rs` | ✅ Const/Move + 4 个测试 |
| handler_control | `executor/handler_control.rs` | ✅ JumpIf/Abort + 5 个测试 |
| handler_inference | `executor/handler_inference.rs` | ✅ CreateSession/ShutdownSession 真实实现 + RunProgram/AnalyzeModel/SplitModel 占位 + 8 个测试 |
| handler_network | `executor/handler_network.rs` | ⚠️ SendFile/ReceiveFile/OpenTensorStream 占位 |
| Compiler | `compiler.rs` | ⚠️ 骨架 + TaskProgramBuilder，compile_run/compile_coordinator/compile_relay 为 todo!() |
| Core spawn 测试 | `core.rs#core_tests` | ✅ 4 个测试（spawn/abort/compile_error/multi_job） |
| Capabilities 集成 | `mod.rs` | ✅ ml_engine: Box\<dyn ML_Engine_Capability\>（替代旧 InferenceCapability） |

**外部依赖模块**：
| 模块 | 状态 |
|------|------|
| StorageManager | ✅ 已完成 |
| LLM_IO（Broker + Capability） | ✅ 已完成并集成 |
| ML_Engine（ML_Engine_Capability + ML_Engine_Service） | ✅ 已完成并集成 |

---

## 2. 实施路线图

### Phase 2A：IO 通道打通（必须先行）

**目标**：让 IoHandle 能通过指令系统流入 ML Thread，完成数据面打通。

#### 步骤 1：SlotValue 扩展 ✅ 已完成
- **文件**：`slot.rs`
- **改动**：
  - `SlotValue` 枚举新增 `IoHandle(crate::llm_io::IoHandle)` 变体
  - `SlotFile` 新增 `take_io_handle(slot: SlotId) -> Result<IoHandle, String>` 方法
  - 采纳方案 A：拆分出 `ConstValue` 枚举（可 Clone 子集），`SlotValue` 不实现 Clone
- **注意**：`IoHandle` 不实现 Clone（含 mpsc::Receiver），只能 take，不能 get

#### 步骤 2：CreateSession 指令扩展 ✅ 已完成
- **文件**：`instruction.rs`
- **改动**：`CreateSession` 新增 `io: SlotId` 参数
  ```rust
  CreateSession {
      model: SlotId,
      device: SlotId,
      io: SlotId,       // ← 新增
      result: SlotId,
  }
  ```
- **联动**：
  - `task_engine.rs` step() 的 match 分支已传递 `io` 参数
  - `handler_inference.rs` 的 `handle_create_session` 签名已同步更新

#### 步骤 3：移除 InferenceCapability，集成 ML_Engine_Capability ✅ 已完成（设计变更）
- **原计划**：给 `InferenceCapability::create_session` 添加 `io: IoHandle` 参数
- **实际执行**：ML Engine 已有完整的 `ML_Engine_Capability` trait（含 `Create_Session`、`Shutdown_Session`、`Run_Program`、`Analyze_Model`、`Split_Model`），无需维护冗余的 `InferenceCapability` 占位 trait
- **改动**：
  - `executor/mod.rs`：移除 `InferenceCapability` trait 定义
  - `Orchestrator/mod.rs`：`Capabilities.inference: Box<dyn InferenceCapability>` → `Capabilities.ml_engine: Box<dyn ML_Engine_Capability>`
  - 所有测试 Stub：`StubInference` → `StubMLEngine`（impl `ML_Engine_Capability`，所有方法 `unimplemented!("stub")`）
  - 涉及文件：`executor/mod.rs`、`task_engine.rs`、`handler_data.rs`、`handler_control.rs`、`core.rs`、`tests/common/mod.rs`

#### 步骤 4：JobExecutor 初始化时注入 IoHandle 到 SlotFile ✅ 已完成
- **文件**：`executor/mod.rs`、`compiler.rs`
- **改动**：
  - `compiler.rs` 新增 `pub const SLOT_IO: SlotId = SlotId(0)` 约定槽位常量
  - `JobExecutor::new()` 中调用 `task_engine.slots.set(SLOT_IO, SlotValue::IoHandle(io))`
  - `JobExecutor` 结构体移除 `io` 字段（IoHandle 已在 SlotFile 中，不再需要额外持有）
- **设计**：固定槽位 ID 约定，Compiler 编译时引用 `SLOT_IO` 常量

#### 步骤 5：更新所有测试 Stub ✅ 已完成（与步骤 3 合并）
- 步骤 3 的设计变更已同步完成所有 Stub 更新
- 所有测试文件已使用 `StubMLEngine`（impl `ML_Engine_Capability`）

---

### Phase 2B：Handler 填充

**目标**：让 AcquireDevice / CreateSession / ShutdownSession 具备真实调用 Capability 的逻辑。

#### ~~步骤 6：handler_compute.rs — AcquireDevice~~ ✅ 已移除
- **决策**：采用方案 A，移除整套 `AcquireDevice` / `ComputeCapability` / `DeviceLease` 体系
- **原因**：ML Engine 的 `ML_Session_Config.device` 是简单的 `String`（"cpu" / "cuda"），不需要 lease/acquire 语义
- **改动**：删除 `handler_compute.rs`；移除 `ComputeCapability` trait、`DeviceLease` struct、`AcquireDevice` 指令
- **替代方案**：Compiler 生成 `Const { value: String(device), dst: SLOT_DEVICE }`，handler_inference 从槽位读取 device 字符串

#### 步骤 7：handler_inference.rs — CreateSession ✅ 已完成
- **目标实现**：
  1. 从 `model` 槽位 `get_string` 获取模型路径（作为 `model_file_id`）
  2. 从 `device` 槽位 `get_string` 获取设备字符串（缺失则默认 "cpu"）
  3. 从 `io` 槽位 `take_io_handle` 获取 IoHandle
  4. 构造 `ML_Session_Config`（session_id 由 job_id 生成，model_file_id、layer_start=0/layer_end=MAX、device 从槽位或默认值获取）
  5. 调用 `self.capabilities.ml_engine.Create_Session(config, io_handle).await`
  6. 成功 → 将 `session_id` 存入 `result` 槽位（SlotValue::String），返回 Continue
  7. 失败 → 返回 Abort
- **注意**：ML_Engine_Capability 返回 `Model_Info`，session 通过 `session_id: String` 标识（非旧的 `SessionHandle` 结构）
- **改动**：TaskEngine 新增 `job_id: JobId` 字段用于生成 session_id；handler 方法改为 async
- **测试**：5 个测试（正常创建、模型路径缺失、IO 句柄缺失、ML Engine 错误、设备默认值）

#### 步骤 8：handler_inference.rs — ShutdownSession ✅ 已完成
- **目标实现**：
  1. 从 `session` 槽位 `get_string` 获取 session_id
  2. 调用 `self.capabilities.ml_engine.Shutdown_Session(&session_id).await`
  3. 返回 Continue（即使失败也尽力清理，不 Abort）
- **测试**：3 个测试（正常关闭、空槽位处理、ML Engine 错误仍返回 Continue）

---

### Phase 2C：Compiler 完整实现

**目标**：三个编译器方法生成可执行的 TaskProgram。

#### 步骤 9：compile_run 实现（单机推理）
- **生成的指令序列**（Run 作业）：
  ```
  正向序列：
    Const { value: String(model_path),  dst: SLOT_MODEL }
    Const { value: String(device_pref), dst: SLOT_DEVICE }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEVICE, io: SLOT_IO, result: SLOT_SESSION }
    RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```
- **约定槽位**：SLOT_IO = 0（由 JobExecutor 初始化时写入）

#### 步骤 10：compile_coordinator 实现（分布式协调者）
- **生成的指令序列**（Coordinator 作业）：
  ```
  正向序列：
    Const { value: String(model_path),  dst: SLOT_MODEL }
    AnalyzeModel { model: SLOT_MODEL, result: SLOT_MODEL_INFO }
    SplitModel { source: SLOT_MODEL, start: SLOT_START, end: SLOT_END, output: SLOT_SHARD }
    Const { value: String(peer_id),     dst: SLOT_PEER }
    SendFile { peer: SLOT_PEER, file: SLOT_SHARD }
    OpenTensorStream { peer: SLOT_PEER, result: SLOT_TENSOR_IO }
    Const { value: String(device_pref), dst: SLOT_DEVICE }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEVICE, io: SLOT_IO, result: SLOT_SESSION }
    RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```

#### 步骤 10b：compile_relay 实现（分布式中继 Worker）
- **生成的指令序列**（Relay 作业）：
  ```
  正向序列：
    ReceiveFile { result: SLOT_MODEL }
    Const { value: String(peer_id),     dst: SLOT_PEER }
    OpenTensorStream { peer: SLOT_PEER, result: SLOT_TENSOR_IO }
    Const { value: String(device_pref), dst: SLOT_DEVICE }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEVICE, io: SLOT_IO, result: SLOT_SESSION }
    RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```

#### 步骤 11：Compiler 单元测试
- 验证三个编译方法生成的 TaskProgram 指令数量、类型、槽位连接正确性
- 验证补偿序列包含正确的清理指令
- 验证参数校验（空路径、空 peer_id、空 peers 列表）

---

### Phase 2D：Core 端到端集成测试

**目标**：通过 UserCommand 触发完整 compile → spawn → execute → Done 流程。

#### 步骤 12：Core::run() 端到端测试
- **测试 1**（Run 作业端到端）：
  1. 创建 Core（使用 Stub Capabilities）
  2. 通过 `user_cmd_tx` 发送 `UserCommand::Run { model_path }`
  3. Core::run() 内部 compile_run → spawn_job → executor 执行 → Done
  4. 发送 `UserCommand::Quit`
  5. Core::run() 退出
  6. 验证 registry 为空

- **测试 2**（Cancel 作业）：
  1. 使用阻塞型 StubMLEngine（在 Create_Session 中等待 Notify）
  2. 发送 Run 命令，Job 阻塞在 CreateSession
  3. 发送 Cancel 命令
  4. 验证收到 Cancelled 结果

- **测试 3**（Quit 优雅退出）：
  1. spawn 多个 Job
  2. 发送 Quit → 触发 shutdown → 所有 Job cancel
  3. 验证 Core::run() 正常退出

---

## 3. 依赖图

```
步骤 1 (SlotValue +IoHandle) ✅
    ↓
步骤 2 (CreateSession +io) ✅  ←  步骤 3 (ML_Engine_Capability 集成) ✅
    ↓
步骤 4 (JobExecutor 注入 IoHandle 到 SlotFile) ✅
    ↓
步骤 5 (测试 Stub 更新) ✅ (已与步骤3合并完成)
    ↓
┌───────────────────┐
│  Phase 2A 完成 ✅  │
└───────┬───────────┘
        ↓
步骤 6 (handler_compute)
步骤 7 (handler_inference: CreateSession)  ← 可并行
步骤 8 (handler_inference: ShutdownSession)
        ↓
┌───────────────────┐
│  Phase 2B 完成     │
└───────┬───────────┘
        ↓
步骤 9  (compile_run)
步骤 10 (compile_worker_relay)  ← 可并行
步骤 11 (Compiler 测试)
        ↓
┌───────────────────┐
│  Phase 2C 完成     │
└───────┬───────────┘
        ↓
步骤 12 (Core 端到端集成测试)
        ↓
┌───────────────────┐
│  Phase 2 完成      │
└───────────────────┘
```

---

## 4. 执行建议

1. **每步一个 task.md 条目**：保持"一步一测"节奏
2. **步骤 4 可独立执行**：只需在 `JobExecutor::new()` 中将 `io` 写入 `task_engine.slots`
3. **步骤 6–8 的 async 化**：handler_compute 和 handler_inference 改为 async 后，task_engine.rs 的 match 分支也需要加 `.await`，这是一次性批量改动
4. **Compiler 使用常量槽位约定**：定义常量如 `const SLOT_IO: SlotId = SlotId(0)`，避免魔法数字
5. **handler_inference 需适配 ML_Engine_Capability API**：`Create_Session` 接受 `ML_Session_Config` + `IoHandle`，返回 `Model_Info`；Session 通过 `session_id: String` 标识，不再使用旧的 `SessionHandle` 占位结构

---

## 5. 风险与注意事项

| 风险 | 缓解措施 |
|------|---------|
| IoHandle 不可 Clone，SlotFile.set 需要所有权 | 使用 take 语义，每个 IoHandle 只能被一条指令消费一次 |
| handler 改为 async 后 TaskEngine::step 签名不变（已是 async） | 只需在 match 分支加 .await，不影响上层 |
| Compiler 生成的 SlotId 必须与 JobExecutor 初始化的 SlotId 对齐 | 使用 compiler.rs 中定义的全局常量 |
| Cancel 测试时序问题 | Phase 2D 使用 Notify 阻塞的 Stub 保证确定性 |
| ~~SlotValue::IoHandle 无法 Clone~~ | ✅ 已解决：拆分 `ConstValue`（可 Clone）和 `SlotValue`（不可 Clone） |
| ~~InferenceCapability 与 ML_Engine_Capability 冗余~~ | ✅ 已解决：移除 InferenceCapability，直接使用 ML_Engine_Capability |
| Session 标识从 `SessionHandle` 改为 `session_id: String` | handler_inference 和 SlotFile 需使用 `SlotValue::String` 存储 session_id |

### 5.1 SlotValue Clone 问题 ✅ 已解决

采纳**方案 A**：拆分 `ConstValue`（可 Clone 子集，用于 `TaskInstruction::Const`）和 `SlotValue`（不可 Clone，含 IoHandle）。
- `SlotValue` 不实现 `Clone`
- `TaskInstruction::Const` 的 `value` 类型为 `ConstValue`（可 Clone）
- `TaskProgram` 仍可 Clone（因 `ConstValue` 可 Clone）
- IoHandle 仅通过 `SlotFile.take_io_handle()` 消费

### 5.2 ML_Engine_Capability 集成注意事项（步骤 3 新增）

`ML_Engine_Capability` 的 API 与旧 `InferenceCapability` 有显著差异：
- `Create_Session(config: ML_Session_Config, io_handle: IoHandle) -> Result<Model_Info, ML_Engine_Error>`
  - 需构造 `ML_Session_Config`（session_id, model_file_id, layer_start, layer_end, device, tensor_io）
  - 返回 `Model_Info` 而非 `SessionHandle`
  - Session 通过 `session_id: String` 标识
- `Shutdown_Session(session_id: &str)` — 传 session_id 而非 SessionHandle
- `Run_Program(session_id, program, params, cancel_flag)` — 新增方法，Phase 3 会用到
- `Analyze_Model` / `Split_Model` — 独立方法，分布式推理场景使用
