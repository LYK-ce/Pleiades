# Orchestrator 后续实现计划

**日期**：2026-04-23（最近更新：2026-04-28 架构重构 — 文件分发与流水线推理拆分）
**基线**：Phase 2B 完成 + 指令集/编译器/handler_network 架构扩展

---

## 1. 当前已完成清单

| 模块 | 文件 | 状态 |
|------|------|------|
| Core 主循环 | `core.rs` | ✅ run() + select! + lifecycle 通道 |
| Job 公共契约 | `job.rs` | ✅ JobId/JobKind(Run,Coordinator,Relay)/JobState/LifecycleEvent |
| Command 定义 | `command.rs` | ✅ UserCommand(含 oneshot reply)/NetworkCommand |
| Slot 系统 | `slot.rs` | ✅ SlotId/SlotValue(含IoHandle,Stream,ModelInfo)/SlotFile/ConstValue |
| 指令集 | `instruction.rs` | ✅ 12 条指令（数据2 + 推理5 + 网络3 + 控制2） |
| TaskEngine | `executor/task_engine.rs` | ✅ step() 路由全部 12 条指令 + ExecutionMode + 5 个测试 |
| JobExecutor | `executor/mod.rs` | ✅ run() + 补偿循环 + 3 个测试 |
| handler_data | `executor/handler_data.rs` | ✅ Const/Move + 4 个测试 |
| handler_control | `executor/handler_control.rs` | ✅ JumpIf/Abort + 5 个测试 |
| handler_inference | `executor/handler_inference.rs` | ✅ CreateSession/ShutdownSession/RunProgram/AnalyzeModel/SplitModel 全部实现 + 18 个测试 |
| handler_network | `executor/handler_network.rs` | ✅ SendFile(三阶段协议)/ReceiveFile(stream接收+checksum校验) 已实现, OpenTensorStream P2占位 |
| Compiler | `compiler.rs` | ✅ compile_run 已实现 + build_run_ml_program；compile_coordinator/compile_relay 为 todo!() |
| Core spawn 测试 | `core.rs#core_tests` | ✅ 4 个测试（spawn/abort/compile_error/multi_job） |
| Capabilities 集成 | `mod.rs` | ✅ ml_engine: Box\<dyn ML_Engine_Capability\>（替代旧 InferenceCapability） |

**外部依赖模块**：
| 模块 | 状态 |
|------|------|
| StorageManager | ✅ 已完成 |
| LLM_IO（Broker + Capability） | ✅ 已完成并集成（Allocate + Take_ML_Side + Take_Frontend 托管分发模式） |
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

### Phase 2C：指令扩展 + Compiler 完整实现

**目标**：扩展指令集支持分布式通信，三个编译器方法生成可执行的 TaskProgram。

#### 架构重构说明（2026-04-28）

**核心变更**：文件分发与流水线推理拆分为独立 Job。

原设计中 `compile_coordinator` 包含 AnalyzeModel → SplitModel → SendFile → OpenTensorStream → CreateSession → RunProgram 的完整流程。
新设计将其拆分为：
- **文件分发**：独立的用户命令（如「A 把模型分片发给 B 和 C」），对应独立 Job
- **流水线推理**：独立的用户命令（如「B 发起分布式推理，使用 C 作为 Worker」），对应 Coordinator Job

分发文件的节点和发起推理的节点可以不同。Coordinator 的身份由「谁发起推理」决定，不由「谁分发文件」决定。

**通用协议原则**：任何远端操作只要 spawn 了 Job，响应中必须携带 job_id。

#### 步骤 9：compile_run 实现（单机推理）✅ 已完成
- **生成的指令序列**（Run 作业）：
  ```
  正向序列：
    Const { value: String(model_path),  dst: SLOT_MODEL }
    Const { value: String(device_pref), dst: SLOT_DEVICE }
    Const { value: U64(0),              dst: SLOT_LAYER_START }
    Const { value: U64(u64::MAX),       dst: SLOT_LAYER_END }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEVICE, start: SLOT_LAYER_START, end: SLOT_LAYER_END, io: SLOT_IO, tensor_io: None, result: SLOT_SESSION }
    RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```
- **约定槽位**：SLOT_IO = 0（由 JobExecutor 初始化时写入）

#### 步骤 9b：指令集扩展（分布式通信前置准备）

##### 9b-1：新增 `RequestPipeline` 指令

- **文件**：`instruction.rs`
- **定义**：
  ```rust
  /// 请求远端 Peer 加入 Pipeline，获取其 Relay Job ID
  ///
  /// 1. 从各槽位读取 PeerId、model_file_id、device、layer_start、layer_end
  /// 2. 构造 REQUEST_PIPELINE payload（携带 coordinator_job_id + 上述参数）
  /// 3. 通过 send_data(peer, DataType::Command, payload) 发送
  /// 4. 等待远端 Core 回复 relay_job_id
  /// 5. 将 relay_job_id（U64）存入 `result` 槽位
  RequestPipeline {
      peer: SlotId,
      model: SlotId,
      device: SlotId,
      start: SlotId,
      end: SlotId,
      result: SlotId,
  }
  ```
- **联动**：`task_engine.rs` 的 match 新增分支

##### 9b-2：修改 `OpenTensorStream` 签名

- **文件**：`instruction.rs`
- **改动**：新增 `target_job: SlotId` 参数
  ```rust
  // 旧签名
  OpenTensorStream { peer: SlotId }
  // 新签名
  OpenTensorStream { peer: SlotId, target_job: SlotId }
  ```
- **联动**：
  - `task_engine.rs` 的 match 分支传递新参数
  - `handler_network.rs` 的 `handle_open_tensor_stream` 签名同步更新

##### 9b-3：实现 `handle_request_pipeline`

- **文件**：`handler_network.rs`
- **逻辑**：
  1. 从 `peer` 槽位读取 PeerId
  2. 构造 payload：`"REQUEST_PIPELINE|{coordinator_job_id}|{model_file_id}|{device}|{layer_start}|{layer_end}"`
     - `coordinator_job_id` 来自 `self.job_id`（TaskEngine 已有）
     - 其余参数从约定槽位读取（SLOT_MODEL, SLOT_DEVICE, SLOT_LAYER_START, SLOT_LAYER_END）
  3. 调用 `send_data(peer_id, DataType::Command, payload).await`
  4. 解析响应：`"OK|{relay_job_id}"` → 存入 result 槽位 (SlotValue::U64)
  5. 失败 → Abort

##### 9b-4：修复 `handle_open_tensor_stream` 读取 target_job_id

- **文件**：`handler_network.rs`
- **改动**：移除 `let target_job_id: u64 = 0;` 硬编码，改为从 `target_job` 槽位读取
  ```rust
  // 旧代码
  let target_job_id: u64 = 0;
  // 新代码
  let target_job_id = match self.slots.get_u64(target_job) {
      Ok(v) => v,
      Err(e) => return StepResult::Abort(format!("...")),
  };
  ```

##### 9b-5：`NetworkCommand::PipelineFlow` 加 reply 通道 + 扩展参数

- **文件**：`command.rs`
- **改动**：
  ```rust
  // 旧定义
  pub enum NetworkCommand {
      PipelineFlow { peer_id: String }
  }
  // 新定义
  pub enum NetworkCommand {
      PipelineFlow {
          coordinator_peer_id: String,
          coordinator_job_id: u64,
          model_file_id: String,
          device: String,
          layer_start: usize,
          layer_end: usize,
          reply: oneshot::Sender<Result<JobId, String>>,
      }
  }
  ```
- **注意**：`NetworkCommand` 移除 `#[derive(Clone)]`（`oneshot::Sender` 不可 Clone）

##### 9b-6：Core 集成补丁

- **文件**：`core.rs`
- **改动**：
  1. `route_network` 的 `PipelineFlow` 分支：编译成功 + spawn 后通过 `reply.send(Ok(job_id))` 回传 relay_job_id
  2. `spawn_job` 中，当 `kind != Run` 时调用 `tensor_io_broker.Prepare(job_id)`
  3. `handle_lifecycle_event` Done 分支中调用 `tensor_io_broker.Deallocate(job_id)`
  4. `compile_relay` 调用签名更新（传入 coordinator_job_id、model、layers 等参数）

#### 步骤 10：compile_coordinator 实现（分布式协调者 — 纯流水线推理） ✅ 已完成

- **前提**：模型文件已在各节点本地（文件分发是独立命令）
- **新签名**：
  ```rust
  pub fn compile_coordinator(
      &self,
      job_id: JobId,
      model_path: String,
      peers: Vec<String>,
      device_preference: Option<String>,
      layer_start: usize,
      layer_end: usize,
  ) -> Result<TaskProgram, CompilerError>
  ```
- **新增约定槽位**：
  ```rust
  pub const SLOT_PEER: SlotId = SlotId(7);
  pub const SLOT_TARGET_JOB: SlotId = SlotId(8);
  pub const SLOT_INBOUND: SlotId = SlotId(9);
  pub const SLOT_OUTBOUND: SlotId = SlotId(10);
  pub const SLOT_TENSOR_IO: SlotId = SlotId(11);
  ```
- **生成的指令序列**（Coordinator 作业，单 Worker 示例）：
  ```
  正向序列：
    Const { value: String(model_path),  dst: SLOT_MODEL }       // ← 必须在 RequestPipeline 之前
    Const { value: String(device_pref), dst: SLOT_DEVICE }       //    因为 handler 需要从这些槽位
    Const { value: U64(layer_start),    dst: SLOT_LAYER_START }  //    读取参数编入 payload
    Const { value: U64(layer_end),      dst: SLOT_LAYER_END }
    Const { value: String(peer_id),     dst: SLOT_PEER }
    RequestPipeline { peer: SLOT_PEER, model: SLOT_MODEL, device: SLOT_DEVICE, start: SLOT_LAYER_START, end: SLOT_LAYER_END, result: SLOT_TARGET_JOB }
    OpenTensorStream { peer: SLOT_PEER, target_job: SLOT_TARGET_JOB }
    TakeInboundStream { result: SLOT_INBOUND }
    TakeOutboundStream { result: SLOT_OUTBOUND }
    BuildTensorIo { inbound: SLOT_INBOUND, outbound: SLOT_OUTBOUND, result: SLOT_TENSOR_IO }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEVICE, start: SLOT_LAYER_START, end: SLOT_LAYER_END, io: SLOT_IO, tensor_io: Some(SLOT_TENSOR_IO), result: SLOT_SESSION }
    RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```

- **双向 handshake 说明**：
  - Coordinator → Relay：`OpenTensorStream` 写入 `target_job_id = relay_job_id`（从 `RequestPipeline` 获得）
  - Relay → Coordinator：Relay 的 `OpenTensorStream` 写入 `target_job_id = coordinator_job_id`（从 `RequestPipeline` payload 获得）
  - 两端 Core 收到 `TensorStreamArrived` 后读取 handshake，路由到正确 Job 的 Broker

#### 步骤 10b：compile_relay 实现（分布式 Worker — 纯流水线推理） ✅ 已完成

- **前提**：模型分片已在本地（文件分发是独立命令）
- **新签名**：
  ```rust
  pub fn compile_relay(
      &self,
      job_id: JobId,
      coordinator_peer_id: String,
      coordinator_job_id: u64,
      model_file_id: String,
      device_preference: Option<String>,
      layer_start: usize,
      layer_end: usize,
  ) -> Result<TaskProgram, CompilerError>
  ```
- **新增约定槽位**：
  ```rust
  pub const SLOT_COORDINATOR_JOB: SlotId = SlotId(12);
  ```
- **生成的指令序列**（Relay 作业）：
  ```
  正向序列：
    Const { value: String(coordinator_peer_id), dst: SLOT_PEER }
    Const { value: U64(coordinator_job_id),     dst: SLOT_COORDINATOR_JOB }
    OpenTensorStream { peer: SLOT_PEER, target_job: SLOT_COORDINATOR_JOB }
    TakeInboundStream { result: SLOT_INBOUND }
    TakeOutboundStream { result: SLOT_OUTBOUND }
    BuildTensorIo { inbound: SLOT_INBOUND, outbound: SLOT_OUTBOUND, result: SLOT_TENSOR_IO }
    Const { value: String(model_file_id),  dst: SLOT_MODEL }
    Const { value: String(device_pref),    dst: SLOT_DEVICE }
    Const { value: U64(layer_start),       dst: SLOT_LAYER_START }
    Const { value: U64(layer_end),         dst: SLOT_LAYER_END }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEVICE, start: SLOT_LAYER_START, end: SLOT_LAYER_END, io: SLOT_IO, tensor_io: Some(SLOT_TENSOR_IO), result: SLOT_SESSION }
    RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```

#### 步骤 10c：build_coordinator_ml_program / build_relay_ml_program ✅ 已完成

- **Coordinator ML 指令序列**：
  ```
  Input → Encode → Set(META2, max_tokens) → Set(FLAG1, false)
  → Prefill(TOKENID3) → CopyMeta(META5, META1)
  → Send → Receive → Sample(TENSOR1) → Decode → Output
  → Loop [ BreakIf, Inference(TOKENID2), Send, Receive, Sample(TENSOR1), Decode, Output ]
  → SendEOF → EndOutput
  ```
- **Relay ML 指令序列**：
  ```
  Loop [ Receive, BreakIf, Inference(TENSOR1), Send ]
  ```
- **注意**：Coordinator 采样使用 `TENSOR1`（从 Receive 获取的下游计算结果），而非单机推理的 `TENSOR2`（本地前向输出）

#### 步骤 11：Compiler 单元测试 ✅ 已完成
- 验证三个编译方法生成的 TaskProgram 指令数量、类型、槽位连接正确性
- 验证补偿序列包含正确的清理指令
- 验证参数校验（空路径、空 peer_id、空 peers 列表）
- 验证 coordinator 程序中 `RequestPipeline` 在 `OpenTensorStream` 之前
- 验证 relay 程序中 `SLOT_COORDINATOR_JOB` 正确连接到 `OpenTensorStream.target_job`

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
步骤 6 (AcquireDevice 已移除) ✅
步骤 7 (handler_inference: CreateSession) ✅
步骤 8 (handler_inference: ShutdownSession) ✅
handler_network: SendFile/ReceiveFile/OpenTensorStream 等 ✅
handler_inference: RunProgram/AnalyzeModel/SplitModel ✅
        ↓
┌───────────────────┐
│  Phase 2B 完成 ✅  │
└───────┬───────────┘
        ↓
步骤 9   (compile_run) ✅
步骤 9b  (指令扩展: RequestPipeline + OpenTensorStream 签名变更) ✅
             ↓
步骤 10  (compile_coordinator — 纯流水线) ✅  ┐
步骤 10b (compile_relay — 纯流水线) ✅        │← 已完成
步骤 10c (build_coordinator/relay_ml_program) ✅ ┘
             ↓
步骤 11  (Compiler 单元测试) ✅
        ↓
┌───────────────────┐
│  Phase 2C 完成 ✅  │
└───────┬───────────┘
        ↓
步骤 12 (Core 端到端集成测试)
        ↓
┌───────────────────┐
│  Phase 2D 完成     │
└───────┬───────────┘
        ↓
步骤 13 (DistributeRun 命令 + route_user 分支)
步骤 14 (DistributeModel 命令 + compile_distribute)
步骤 15 (FileStreamArrived 入站处理)
步骤 16 (DisplayPeer / SetDevice 实现)
        ↓
┌───────────────────┐
│  Phase 3 完成      │
└───────────────────┘
```

---

## 4. 执行建议

1. **步骤 9b 是前置条件**：指令扩展（RequestPipeline + OpenTensorStream 签名）必须先完成，compile_coordinator/relay 才能编译
2. **步骤 9b 涉及 6 个文件联动**：instruction.rs → task_engine.rs → handler_network.rs → command.rs → core.rs → compiler.rs，建议一次性完成
3. **步骤 10/10b/10c 可并行**：三个编译方法互不依赖
4. **Compiler 使用常量槽位约定**：新增 SLOT_PEER/SLOT_TARGET_JOB/SLOT_INBOUND/SLOT_OUTBOUND/SLOT_TENSOR_IO/SLOT_COORDINATOR_JOB 常量
5. **compile_relay 签名变更较大**：从 `(job_id, peer_id, optional_params)` 扩展为包含 coordinator_job_id/model/device/layers 的完整参数

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

### 5.3 UserCommand Reply 机制（2026-04-27 新增）

**变更**：所有 `UserCommand` 变体携带 `oneshot::Sender` 回复通道，Core 处理完命令后回传结果。

**动机**：原设计为 fire-and-forget，前端发送 `UserCommand::Run` 后无法得知分配的 `job_id`，
也无法调用 `broker.Take_Frontend(job_id)` 获取前端端点。

**回复类型**：

| 变体 | reply 类型 | 成功回复 | 失败回复 |
|------|-----------|---------|---------|
| `Run` | `Result<JobId, String>` | 分配的 job_id | 编译/IO 分配失败原因 |
| `Cancel` | `Result<(), String>` | 已发送取消信号 | Job 不存在 |
| `Quit` | `()` | 已进入关闭流程 | — |
| `DisplayPeer` | `Result<Vec<String>, String>` | 节点列表 | 查询失败 |
| `SetDevice` | `Result<(), String>` | 设置成功 | 设置失败 |

**前端使用模式**：
```rust
let (reply_tx, reply_rx) = oneshot::channel();
user_cmd_tx.send(UserCommand::Run { model_path, reply: reply_tx }).await?;
let job_id = reply_rx.await??;  // 拿到 job_id
let frontend = broker.Take_Frontend(job_id).await?;
frontend.input_tx.send("你好").await?;
```

**注意**：`UserCommand` 不再实现 `Clone`（`oneshot::Sender` 不可 Clone）。
实际使用中 `UserCommand` 通过 mpsc 通道传输（move 语义），无需 Clone。

### 5.4 NetworkCommand Reply 机制（2026-04-28 新增）

**变更**：`NetworkCommand::PipelineFlow` 携带 `oneshot::Sender<Result<JobId, String>>` 回复通道。

**动机**：Coordinator Job 发出 `RequestPipeline` 后需要获得远端 Relay Job 的 `job_id`，
用于 `OpenTensorStream` 的 handshake 帧。远端 Control 层处理入站请求时需要等待 Core 编译 + spawn 完成后才能回复，
因此 `NetworkCommand::PipelineFlow` 也需要 reply 通道（与 `UserCommand` 相同模式）。

**流程**：
```
Remote Control 收到 REQUEST_PIPELINE 入站请求
  → 解析 payload（coordinator_job_id, model, device, layers）
  → 创建 NetworkCommand::PipelineFlow { ..., reply: oneshot::Sender }
  → 发送到 Core 的 network_cmd_rx
  → 等待 reply_rx
  → Core 处理：compile_relay → Allocate IO → spawn_job → reply.send(Ok(job_id))
  → Remote Control 收到 job_id → send_response(request_id, "OK|{relay_job_id}")
```

**注意**：`NetworkCommand` 不再实现 `Clone`（与 `UserCommand` 相同原因）。

### 5.5 文件分发与流水线推理分离原则（2026-04-28 新增）

**核心原则**：文件分发和流水线推理是两个独立的用户操作，由不同的 Job 执行。

| 操作 | 发起者 | 对应 Job | 说明 |
|------|--------|---------|------|
| 分发文件 | 任意节点（如 A） | AnalyzeModel → SplitModel → SendFile | 将模型分片发送到各节点 |
| 流水线推理 | 协调者节点（如 B） | RequestPipeline → OpenTensorStream → CreateSession → RunProgram | 建立 tensor stream 并执行推理 |

**设计优势**：
1. **职责单一** — 每个 Job 只做一件事
2. **灵活组合** — 分发者和推理者可以是不同节点
3. **失败隔离** — 文件分发失败不影响推理 Job
4. **可复用** — 同一份文件分发后可多次启动推理

**通用协议约定**：任何远端操作只要 spawn 了 Job，响应中必须携带 `job_id`。
这使得发起方可以追踪远端 Job 状态，并用于后续通信（如 tensor stream handshake）。

### 5.6 双向 Tensor Stream Handshake（2026-04-28 新增）

**问题**：Coordinator 和 Relay 各需要向对方发送一条 tensor stream，
每条 stream 的 handshake 帧需要写入 **目标 Job 的 ID**，以便对方 Core 路由到正确的 Broker 条目。

**解决方案**：通过 `RequestPipeline` 交换双方 job_id。

| 方向 | 发起方 | handshake 中的 target_job_id | 来源 |
|------|--------|---------------------------|------|
| Coordinator → Relay | Coordinator | relay_job_id | `RequestPipeline` 响应 |
| Relay → Coordinator | Relay | coordinator_job_id | `RequestPipeline` payload |

**RequestPipeline 协议格式**：
```
请求 payload: "REQUEST_PIPELINE|{coordinator_job_id}|{model_file_id}|{device}|{layer_start}|{layer_end}"
成功响应:      "OK|{relay_job_id}"
失败响应:      "REJECT|{reason}"
```

Coordinator 的 `handle_request_pipeline` 将自己的 `job_id` 编入请求 payload。
远端 Core 在 `compile_relay` 时将 `coordinator_job_id` 作为编译参数，注入到 Relay 的 `OpenTensorStream` 指令中。

---

## 9. SlotValue 的 `!Sync` 资源包装策略

### 9.1 问题背景

`libp2p::Stream` 内部包含 `Pin<Box<dyn AsyncReadWrite + Send>>`，trait object 只有 `Send` bound，没有 `Sync`。因此：

```
libp2p::Stream: Send + !Sync
→ SlotValue（含 Stream 变体）: Send + !Sync
→ SlotFile (HashMap<SlotId, SlotValue>): Send + !Sync
→ TaskEngine: Send + !Sync
→ JobExecutor: Send + !Sync
→ &JobExecutor: !Send （因为 &T: Send 要求 T: Sync）
→ JobExecutor::run() 的 async Future: !Send
→ tokio::spawn(executor.run()) 编译失败 ❌
```

同样的问题也适用于 `Tensor_IO_Handle`（内含两个 `libp2p::Stream`），
未来添加 `SlotValue::TensorIo(Tensor_IO_Handle)` 时会遇到相同约束。

### 9.2 解决方案：`Mutex<Option<T>>` 包装

对所有 `Send + !Sync` 的资源类型，在 `SlotValue` 中使用 `std::sync::Mutex<Option<T>>` 包装：

```rust
pub enum SlotValue {
    // ... Send + Sync 的变体保持不变 ...
    Stream(Mutex<Option<libp2p::Stream>>),         // 当前已实现
    TensorIo(Mutex<Option<Tensor_IO_Handle>>),     // 未来添加
}
```

**原理**：`Mutex<T>: Sync` 当 `T: Send`。包装后 `SlotValue: Sync`，整条传播链恢复正常。

**访问方式**：这些资源全部是 **take 一次就消费** 的语义。`take_stream()` 实现：

```rust
pub fn take_stream(&mut self, slot: SlotId) -> Result<libp2p::Stream, String> {
    match self.take(slot) {  // 从 HashMap 移除，拿到 SlotValue 所有权
        Some(SlotValue::Stream(mutex)) => {
            mutex.into_inner()  // 消费 Mutex 本身（无需 lock），返回 Option
                .map_err(|e| format!("mutex poisoned: {}", e))?
                .ok_or_else(|| "stream already taken".to_string())
        }
        // ...
    }
}
```

**关键**：`into_inner()` 消费 Mutex 所有权，直接取出内部值，**不需要加锁**，零争用零开销。

### 9.3 设计约定

| 条件 | 处理方式 |
|------|---------|
| 类型 `T: Send + Sync` | 直接存入 `SlotValue::Xxx(T)` |
| 类型 `T: Send + !Sync` | 包装为 `SlotValue::Xxx(Mutex<Option<T>>)` |
| 访问语义 | 统一通过 `take_xxx()` 消费，`into_inner()` 解包 |
| 存入语义 | `SlotValue::Stream(Mutex::new(Some(stream)))` |

---

## 10. 当前缺口总结与 Phase 3 实施计划

**日期**：2026-04-28
**基线**：Phase 2C 完成（Compiler 全量 + 122 测试通过）

### 10.1 当前缺口一览

| # | 类别 | 缺口描述 | 涉及文件 | 优先级 |
|---|------|---------|---------|--------|
| G1 | 命令缺失 | 无 `DistributeRun` 命令触发 `compile_coordinator` | command.rs, core.rs | P1 |
| G2 | 命令缺失 | 无 `DistributeModel` 命令触发文件分发流程 | command.rs, core.rs, compiler.rs | P2 |
| G3 | Core 分支 | `FileStreamArrived` 入站处理为 TODO 占位符 | core.rs | P2 |
| G4 | Core 分支 | `DisplayPeer` 回复为 TODO 占位符 | core.rs | P3 |
| G5 | Core 分支 | `SetDevice` 回复为 TODO 占位符 | core.rs, mod.rs | P3 |
| G6 | Capabilities | `UiCapability.display()` 为 `todo!()` | mod.rs | P3 |
| G7 | Capabilities | `set_compute_preference()` 为 `todo!()` | mod.rs | P3 |

### 10.2 Phase 3 实施路线图

#### 步骤 13：DistributeRun 命令（分布式推理入口）

- **文件**：`command.rs`、`core.rs`
- **改动**：
  1. `UserCommand` 新增变体：
     ```rust
     DistributeRun {
         model_path: String,
         peers: Vec<String>,
         device_preference: Option<String>,
         layer_start: usize,
         layer_end: usize,
         reply: oneshot::Sender<Result<JobId, String>>,
     }
     ```
  2. `route_user()` 新增分支：
     ```
     compile_coordinator(job_id, model_path, peers, device, layer_start, layer_end)
     → io_broker.Allocate(job_id)
     → io_broker.Take_ML_Side(job_id)
     → tensor_io_broker.Prepare(job_id)
     → spawn_job(job_id, JobKind::Coordinator, program, io)
     → reply.send(Ok(job_id))
     ```
- **注意**：与 `PipelineFlow`（Relay）类似，Coordinator 也需要 `tensor_io_broker.Prepare()`

#### 步骤 14：DistributeModel 命令 + compile_distribute（文件分发）

- **文件**：`command.rs`、`core.rs`、`compiler.rs`
- **改动**：
  1. `UserCommand` 新增变体：
     ```rust
     DistributeModel {
         model_path: String,
         peers: Vec<(String, usize, usize)>,  // (peer_id, layer_start, layer_end)
         reply: oneshot::Sender<Result<JobId, String>>,
     }
     ```
  2. `compiler.rs` 新增 `compile_distribute()` 方法：
     ```
     生成的指令序列：
       AnalyzeModel { model: SLOT_MODEL, result: SLOT_MODEL_INFO }
       // 对每个 peer 生成 SplitModel + SendFile
       SplitModel { source: SLOT_MODEL, start, end, output: SLOT_SHARD_i }
       SendFile { peer: SLOT_PEER_i, file: SLOT_SHARD_i }
     补偿序列：（空或清理临时分片）
     ```
  3. `JobKind` 可能需要新增 `Distribute` 变体
  4. `route_user()` 新增分支：compile_distribute → spawn_job

- **依赖**：handler_network 的 SendFile 已实现，handler_inference 的 AnalyzeModel/SplitModel 已实现
- **设计决策**：每个 peer 的分片范围由用户指定（而非自动均分），因为用户了解各节点的计算能力

#### 步骤 15：FileStreamArrived 入站处理

- **文件**：`core.rs`
- **改动**：
  1. `FileStreamArrived` 分支实现：
     ```
     收到 FileStreamArrived { peer, stream }
     → 从 pending_file_receives 中查找匹配的元数据（file_name, file_size, checksum）
     → 编译 ReceiveFile job（compile_receive_file）
     → 将 stream 和元数据注入 SlotFile
     → spawn_job
     ```
  2. 需要在 Core 中维护 `pending_file_receives: HashMap<key, FileMetadata>` 状态
  3. 文件元数据协商（阶段 1）的入站处理也需要对接：
     - Network 层 `DataType::File` 入站 → 解析元数据 → 存入 pending → 回复 accept
     - Network 层 `StreamProtocol::File` 入站 → 触发 `FileStreamArrived`

- **前置条件**：需要明确 Network 层如何将阶段 1（元数据协商）和阶段 2（文件流）分别通知到 Core
- **备选方案**：将文件接收的完整三阶段全部放在 handler_network 内处理（当前 ReceiveFile handler 已有完整逻辑），Core 只负责 spawn ReceiveFile Job 并注入初始 stream

#### 步骤 16：DisplayPeer / SetDevice 实现

- **文件**：`core.rs`、`mod.rs`
- **改动**：
  1. `DisplayPeer`：通过 PeerManager Capability 查询节点列表 → reply 回传
     - 前提：Capabilities 中需添加 PeerManager 引用
  2. `SetDevice`：存储设备偏好到 Core 内部状态
     - 新增 `Core.device_preference: String` 字段
     - `route_user(Run/DistributeRun)` 时使用该偏好作为默认值
  3. 移除 `UiCapability`（显示由 TUI 层直接通过 reply 通道获取数据实现）
  4. 移除 `set_compute_preference`（由 `SetDevice` 命令的 Core 内部状态替代）

### 10.3 Phase 3 依赖图

```
步骤 12 (Phase 2D: Core 端到端测试)
    ↓
步骤 13 (DistributeRun 命令)  ← 依赖 compile_coordinator ✅
    ↓
步骤 14 (DistributeModel 命令) ← 依赖 AnalyzeModel/SplitModel/SendFile handler ✅
    ↓
步骤 15 (FileStreamArrived)   ← 依赖 ReceiveFile handler ✅
    ↓
步骤 16 (DisplayPeer/SetDevice) ← 无强依赖
    ↓
┌───────────────────┐
│  Phase 3 完成      │
│  Orchestrator 可用  │
└───────────────────┘
```

### 10.4 执行建议

1. **步骤 13 最高优先级** — 完成后分布式推理可触发（配合已有的 PipelineFlow 和 compile_relay）
2. **步骤 14 和 15 可并行** — 文件分发和文件接收互不依赖（但端到端测试需要两者都完成）
3. **步骤 16 优先级最低** — DisplayPeer/SetDevice 不影响推理核心流程
4. **步骤 15 可能需要与 Network 层协调** — FileStreamArrived 的触发时机和元数据传递方式需要明确
