# 分布式 Pipeline 建立方案

## 概述

本文档描述分布式推理流水线的建立流程。采用**三阶段中心化协调**方案：

1. **Phase 0** — Coordinator 生成全局唯一 `inference_id`，规划拓扑
2. **Phase 1** — `Establish_Tensor_Stream`：建立所有张量流双向连接
3. **Phase 2** — `Join_Pipeline`：Worker 加载模型并启动推理 Job
4. **Phase 3** — Coordinator 加载模型，开始推理执行

## 前置条件

- 模型分片已通过 `DistributeModel` 命令分发到各 Worker 节点（独立步骤）
- Coordinator 知道每个 Worker 的 PeerId、模型分片 file_id、层范围
- Network Request-Response 超时通过 `config.toml` 的 `[Network].request_response_timeout` 配置，默认 **300s**

## 全局唯一 inference_id

`inference_id: u64` 由 Coordinator 生成，保证分布式系统内全局唯一。

生成算法：`PeerId 哈希高 32 位 | 本地计数器低 32 位`

```rust
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::sync::atomic::{AtomicU32, Ordering};

static LOCAL_COUNTER: AtomicU32 = AtomicU32::new(1);

fn Generate_Inference_Id(local_peer_id: &libp2p::PeerId) -> u64 {
    let mut hasher = DefaultHasher::new();
    local_peer_id.hash(&mut hasher);
    let peer_hash = hasher.finish();

    let high = (peer_hash >> 32) as u32;
    let low = LOCAL_COUNTER.fetch_add(1, Ordering::Relaxed);

    ((high as u64) << 32) | (low as u64)
}
```

## 网络协议

### Establish_Tensor_Stream（Request-Response）

请求 Worker 向指定 target 打开一条张量流。

```
请求: ESTABLISH_TENSOR_STREAM|{inference_id}|{target_peer_id}
回复: OK  或  FAIL|{reason}
```

### Join_Pipeline（Request-Response，超时 300s）

通知 Worker 启动推理 Job，Worker 加载完模型后才回复 OK。

```
请求: JOIN_PIPELINE|{inference_id}|{model_file_id}|{device}|{layer_start}|{layer_end}
回复: OK  或  FAIL|{reason}
```

## 三阶段流程

### Phase 0：规划

Coordinator 生成 `inference_id`，确定拓扑：

```
示例拓扑（3 节点）：
  A(Coordinator, layers 0-9) → B(Worker, layers 10-19) → C(Worker, layers 20-29) → A

流向表：
  A → B (Coordinator 自己打开)
  B → C (通过 Establish_Tensor_Stream 命令 B 打开)
  C → A (通过 Establish_Tensor_Stream 命令 C 打开)

节点参数表：
  B: model=shard_10_19, device=cuda, layers=10-19, outbound_target=C
  C: model=shard_20_29, device=cpu,  layers=20-29, outbound_target=A
```

### Phase 1：Establish_Tensor_Stream

**目标**：建立所有张量流双向连接。

**Coordinator 处理逻辑**：
1. 向所有 Worker **并发**发送 `ESTABLISH_TENSOR_STREAM(inference_id, target_peer_id)`
2. Coordinator 自己也打开到第一个 Worker 的出站流 + handshake(inference_id)
3. 将自己的 outbound 注册到 `TensorStreamManager`
4. 等待所有 Worker 回复 OK
5. 任一 Worker 回复 FAIL → 终止 Pipeline 建立，清理已建立的流

**Worker 接收 Establish_Tensor_Stream 的处理逻辑**：
1. 解析 `target_peer_id`
2. `network.open_tensor_stream(target)` → 打开出站流
3. `Write_Tensor_Stream_Handshake(stream, inference_id)` → 写入 handshake
4. `tensor_stream_manager.insert_outbound(inference_id, stream)` → 注册出站
5. 成功 → 回复 `OK`；失败 → 回复 `FAIL|{reason}`

**各节点 TensorStreamArrived 入站流处理**（所有节点通用）：
1. 收到 `TensorStreamArrived` 事件
2. `Read_Tensor_Stream_Handshake` → 得到 `inference_id`
3. `tensor_stream_manager.insert_inbound(inference_id, stream)` → 注册入站

**Phase 1 完成条件**：所有 Worker 回复 OK。  
此时所有出站流已建立；由于 stream 到达有序，入站流也已（或即将）到达各节点。

### Phase 2：Join_Pipeline

**目标**：Worker 取出张量流、加载模型、启动 ML Thread，**等待模型加载完成后回复 OK**。

**Coordinator 处理逻辑**：
1. 向所有 Worker **并发**发送 `JOIN_PIPELINE(inference_id, model_file_id, device, layer_start, layer_end)`
2. 等待所有 Worker 回复 OK（超时 300s）
3. 任一 Worker 回复 FAIL → 向已启动的 Worker 发送取消命令，清理

**Worker 接收 Join_Pipeline 的处理逻辑**：
1. 从 `tensor_stream_manager` 取出 `inference_id` 对应的 inbound + outbound（如尚未到达，短暂等待 3s）
2. 注册到 `Tensor_Port_Switch` → 获得 `Tensor_IO_Endpoint`
3. 编译 Relay TaskProgram
4. 创建 `JobExecutor`，注入 `IoHandle` + `Tensor_IO_Endpoint`
5. 调用 `ML_Engine.Create_Session(config, io_handle)` — 阻塞等待模型加载完成
6. 模型加载成功 → spawn Job（ML Thread 开始执行 Relay 程序 `Loop [Receive, BreakIf, Inference, Send]`） → 回复 `OK`
7. 模型加载失败 → 清理资源 → 回复 `FAIL|{reason}`

**Phase 2 完成条件**：所有 Worker 回复 OK。  
此时所有节点的张量流已连通、模型已加载、ML Thread 正在等待第一帧数据。

### Phase 3：推理执行

**Coordinator 自身启动**：
1. 从 `tensor_stream_manager` 取出自己的 inbound + outbound
2. 注册到 `Tensor_Port_Switch` → 获得 `Tensor_IO_Endpoint`
3. 创建 Coordinator Job（`CreateSession + RunProgram`）
4. 加载模型 → 开始执行 Coordinator ML 程序

**Coordinator ML 程序**：
```
Input → Encode → Set(max_tokens) → Set(FLAG1=false)
→ Prefill(TOKENID3) → CopyMeta(META5→META1)
→ Send → Receive → Sample(TENSOR1) → Decode → Output
→ Loop [ BreakIf, Inference(TOKENID2), Send, Receive, Sample(TENSOR1), Decode, Output ]
→ SendEOF → EndOutput
```

**Worker Relay ML 程序**：
```
Loop [ Receive, BreakIf, Inference(TENSOR1), Send ]
```

**推理结束**：Coordinator 发送 `SendEOF`，Worker 的 `BreakIf` 检测到 EOF 退出循环，各 Job 自然结束。

## 完整时序图

```
User → Coordinator: 发起分布式推理命令

=== Phase 0 ===
Coordinator: 生成 inference_id, 规划拓扑

=== Phase 1: Establish Tensor Stream ===
Coordinator → Worker_B: ESTABLISH_TENSOR_STREAM(id, target=C)  │ 并发
Coordinator → Worker_C: ESTABLISH_TENSOR_STREAM(id, target=A)  │
Coordinator → self:     打开 A→B 出站流 + handshake(id)

Worker_B: 打开 B→C 出站流 + handshake(id) → 注册 outbound → 回复 OK
Worker_C: 打开 C→A 出站流 + handshake(id) → 注册 outbound → 回复 OK

(同时各节点 TensorStreamArrived → 注册 inbound)

Coordinator: 所有 OK 收齐 → Phase 1 完成 ✅

=== Phase 2: Join Pipeline ===
Coordinator → Worker_B: JOIN_PIPELINE(id, model, device, layers)  │ 并发
Coordinator → Worker_C: JOIN_PIPELINE(id, model, device, layers)  │

Worker_B: 取出 streams → 加载模型（~30s）→ spawn Job → 回复 OK
Worker_C: 取出 streams → 加载模型（~20s）→ spawn Job → 回复 OK

Coordinator: 所有 OK 收齐 → Phase 2 完成 ✅

=== Phase 3: 推理执行 ===
Coordinator: 取出 streams → 加载模型 → 开始推理
A → B: 张量数据流
B → C: 张量数据流
C → A: 张量数据流（结果回传）
Coordinator: Sample → Decode → Output → 用户
推理结束: SendEOF → 所有节点退出
```

## Tensor_Port_Switch 扩展

扩展现有的 `Tensor_Port_Switch`，以 `inference_id` 为 key 统一管理张量流全生命周期。
Stream 到达时直接注册，无需暂存/激活两步。

### 改造后的数据结构

```rust
/// 每个 inference pipeline 对应一个条目
struct Pipeline_Entry {
    /// 入站流（从上游节点接收张量）
    inbound: Arc<Mutex<Option<libp2p::Stream>>>,
    /// 出站流（向下游节点发送张量）
    outbound: Arc<Mutex<Option<libp2p::Stream>>>,
    /// 最后发送 offset
    last_send_offset: Arc<AtomicU64>,
    /// 最后接收 offset
    last_recv_offset: Arc<AtomicU64>,
    /// 失败上报通道
    failure_tx: mpsc::UnboundedSender<FailureReport>,
}

struct Tensor_Port_Switch {
    /// 以 inference_id 为 key 管理所有 pipeline 的张量流
    entries: Mutex<HashMap<u64, Pipeline_Entry>>,
}
```

### 接口方法

```rust
impl Tensor_Port_Switch {
    /// 注册入站流（TensorStreamArrived 时调用）
    /// 若 inference_id 对应条目不存在则自动创建
    pub async fn Register_Inbound(&self, inference_id: u64, stream: libp2p::Stream);

    /// 注册出站流（Establish_Tensor_Stream 成功后调用）
    /// 若 inference_id 对应条目不存在则自动创建
    pub async fn Register_Outbound(&self, inference_id: u64, stream: libp2p::Stream);

    /// 创建 Endpoint 供 ML Thread 使用（Join_Pipeline 时调用）
    /// 要求 inbound 和 outbound 均已注册，否则返回错误
    pub async fn Create_Endpoint(
        &self,
        inference_id: u64,
        rt: tokio::runtime::Handle,
    ) -> Result<(Tensor_IO_Endpoint, mpsc::UnboundedReceiver<FailureReport>), Tensor_IO_Error>;

    /// 清理条目（推理结束或建立失败时调用）
    pub async fn Deregister(&self, inference_id: u64);

    /// 热切换：替换入站流（不中断推理）
    pub async fn Swap_Inbound(&self, inference_id: u64, new_stream: libp2p::Stream)
        -> Result<(), Tensor_IO_Error>;

    /// 热切换：替换出站流（不中断推理）
    pub async fn Swap_Outbound(&self, inference_id: u64, new_stream: libp2p::Stream)
        -> Result<(), Tensor_IO_Error>;
}
```

### 生命周期

```
Phase 1:  Register_Inbound / Register_Outbound → stream 直接注册到 entries
Phase 2:  Create_Endpoint(inference_id) → 包装 Arc<Mutex> 引用为 Endpoint 给 ML Thread
运行时:   ML Thread 通过 Endpoint 的 Send/Receive 读写 entries 中的 stream
热切换:   Swap_Inbound / Swap_Outbound → lock → 替换 stream → unlock，ML Thread 无感
结束:     Deregister(inference_id) → 清理
```

### Tensor Stream 握手协议变更

当前 `Write_Tensor_Stream_Handshake` / `Read_Tensor_Stream_Handshake` 传输的是 `target_job_id: u64`，
需改为传输 `inference_id: u64`（wire format 不变，仅语义变更）。

接收方流程：
```
TensorStreamArrived → Read_Tensor_Stream_Handshake → inference_id
                    → tensor_switch.Register_Inbound(inference_id, stream)
```

## 代码变更清单

### 新增文件

| 文件 | 说明 |
|------|------|
| `Src/Orchestrator/inference_id.rs` | `Generate_Inference_Id` 函数 |

### 修改文件

| 文件 | 变更内容 |
|------|---------|
| `Src/Orchestrator/command.rs` | 新增 `NetworkProtocol::Establish_Tensor_Stream` 和 `NetworkProtocol::Join_Pipeline` 变体；新增对应的 `Parse_Network_Command` / `Serialize_Network_Command` 分支；移除旧的 `Request_Pipeline` |
| `Src/Orchestrator/core.rs` | 移除 `pending_distributed_jobs` 和 `PendingDistributedJob`；`handle_network_command` 增加 `Establish_Tensor_Stream` 和 `Join_Pipeline` 分支处理；`handle_network_inbound` 的 `TensorStreamArrived` 改为读取 `inference_id` 后调用 `tensor_switch.Register_Inbound(inference_id, stream)`；新增 Coordinator 侧的 Pipeline 启动逻辑（发送命令 + 等待回复 + Create_Endpoint + 自身启动） |
| `Src/Orchestrator/mod.rs` | 新增 `pub mod inference_id;` |
| `Src/Tensor_IO/tensor_port_switch.rs` | 重构：以 `inference_id` 为 key 替代 `JobId`；新增 `Pipeline_Entry` 结构体（inbound/outbound 用 `Arc<Mutex<Option<Stream>>>`）；新增 `Register_Inbound`、`Register_Outbound`、`Create_Endpoint`、`Swap_Inbound`、`Swap_Outbound` 方法；移除旧的 `Register` 方法 |
| `Src/Network/tensor_stream_protocol.rs` | `Write_Tensor_Stream_Handshake` / `Read_Tensor_Stream_Handshake` 参数语义从 `target_job_id` 改为 `inference_id`（wire format 不变） |
| `Src/Orchestrator/compiler.rs` | 移除 `compile_coordinator` 中的 `RequestPipeline` 指令生成；简化 Coordinator 编译为仅 `CreateSession + RunProgram` |
| `Src/Orchestrator/instruction.rs` | 移除 `TaskInstruction::RequestPipeline` 变体 |
| `Src/Orchestrator/executor/handler_network.rs` | 移除 `handle_request_pipeline` 方法 |
| `Src/Orchestrator/executor/task_engine.rs` | 移除 `TaskInstruction::RequestPipeline` 的 `step()` 分派 |
| `Src/Network/network_service.rs` | Request-Response 超时从 `config.toml` 的 `[Network].request_response_timeout` 字段读取（默认 300s） |
| `Src/Config/config.toml` | `[Network]` 段新增 `request_response_timeout = 300` |
| `Src/Config/config.rs` | `Network_Config` 新增 `request_response_timeout: Option<u64>` 字段 |
| `Src/Orchestrator/job.rs` | 可选：新增 `JobKind::PipelineSetup` 用于追踪 Pipeline 建立过程 |

### 移除内容

| 项目 | 说明 |
|------|------|
| `PendingDistributedJob` 结构体 | 被 `Tensor_Port_Switch` 的 `pending` 暂存区替代 |
| `pending_distributed_jobs` 字段 | 同上 |
| `route_pipeline_flow` 方法 | 逻辑拆分到 `Establish_Tensor_Stream` 和 `Join_Pipeline` 两个 handler 中 |
| `TaskInstruction::RequestPipeline` | Pipeline 建立不再是 TaskInstruction，由 Core 直接网络操作 |
| `handler_network::handle_request_pipeline` | 同上 |

## 设计原则

1. **连接建立与 Job 启动解耦**：Phase 1 只管连接，Phase 2 只管启动
2. **递进门控**：每个 Phase 全部 OK 才进入下一阶段
3. **中心化控制**：Coordinator 是唯一控制平面，Worker 只被动响应
4. **inference_id 统一关联**：所有 stream 和命令通过同一 ID 关联
5. **快速失败**：任一步骤 FAIL 立即终止并清理，不留半初始化状态
6. **Pipeline 建立不走 TaskInstruction**：这是 Core 层的编排逻辑，不需要下沉到指令引擎
7. **单一组件管理张量流全生命周期**：`Tensor_Port_Switch` 统一管理暂存、激活、读写、热切换、清理

## 修改实施计划

### Phase 1：Worker 侧基础设施（已完成 ✅）

当前阶段目标：**仅实现 Worker 侧的 Network Command 处理**，为后续 Coordinator 编排逻辑准备基础。

#### 实施步骤

#### Step 1：新增 `inference_id.rs`

- 新增文件 `Src/Orchestrator/inference_id.rs`
- 实现 `Generate_Inference_Id(local_peer_id: &PeerId) -> u64`
- 在 `Src/Orchestrator/mod.rs` 中添加 `pub mod inference_id;`

#### Step 2：扩展 `NetworkProtocol` 枚举

修改 `Src/Orchestrator/command.rs`：

1. 在 `NetworkProtocol` 枚举中新增两个变体：
   - `Establish_Tensor_Stream { inference_id: u64, target_peer_id: String }`
   - `Join_Pipeline { inference_id: u64, model_file_id: String, device: String, layer_start: usize, layer_end: usize }`
2. 在 `Parse_Network_Command` 中新增对应解析分支
3. 在 `Serialize_Network_Command` 中新增对应序列化分支
4. **保留** 旧的 `Request_Pipeline` 变体（暂不移除，保持向后兼容）

#### Step 3：Core 中新增 `Establish_Tensor_Stream` 处理

修改 `Src/Orchestrator/core.rs` 的 `handle_network_command` 方法：

新增 `NetworkProtocol::Establish_Tensor_Stream` 分支：
1. 解析 `target_peer_id`
2. 调用 `network.open_tensor_stream(target)` → 打开出站流
3. 调用 `Write_Tensor_Stream_Handshake(stream, inference_id)` → 写入 handshake
4. 调用 `tensor_switch.Register_Outbound(inference_id, stream)` → 注册出站
5. 成功 → 回复 `OK`；失败 → 回复 `FAIL|{reason}`

**前置依赖**：Step 4（Tensor_Port_Switch 需要 `Register_Outbound` 方法）

#### Step 4：Tensor_Port_Switch 新增分步注册接口

修改 `Src/Tensor_IO/tensor_port_switch.rs`：

1. 新增 `Pipeline_Entry` 结构体（`inbound: Arc<Mutex<Option<Stream>>>`, `outbound: Arc<Mutex<Option<Stream>>>`）
2. 在 `Tensor_Port_Switch` 中新增 `pipeline_entries: tokio::sync::Mutex<HashMap<u64, Pipeline_Entry>>`
3. 实现以下方法（添加到 `Tensor_IO_Capability` trait 或作为独立方法）：
   - `Register_Inbound(inference_id, stream)` — 若条目不存在则自动创建
   - `Register_Outbound(inference_id, stream)` — 若条目不存在则自动创建
   - `Create_Endpoint(inference_id, rt) -> Result<(Tensor_IO_Endpoint, failure_rx)>` — 要求双向流均已注册
   - `Deregister_Pipeline(inference_id)` — 清理条目
4. **保留** 现有的 `Register` / `Deregister` 方法不变（旧路径仍可工作）

#### Step 5：Core 中新增 `Join_Pipeline` 处理

修改 `Src/Orchestrator/core.rs` 的 `handle_network_command` 方法：

新增 `NetworkProtocol::Join_Pipeline` 分支：
1. 从 `tensor_switch` 取出 `inference_id` 对应的 inbound + outbound（若尚未到达，短暂等待 3s）
2. 调用 `Create_Endpoint(inference_id, rt)` → 获得 `Tensor_IO_Endpoint`
3. 编译 Relay TaskProgram（复用 `compiler.compile_relay`）
4. 分配 IO 通道
5. 创建 `JobExecutor`，注入 `IoHandle` + `Tensor_IO_Endpoint`
6. 调用 `ML_Engine.Create_Session(config, io_handle)` — 阻塞等待模型加载完成
7. 模型加载成功 → spawn Job → 回复 `OK`
8. 模型加载失败 → 清理资源 → 回复 `FAIL|{reason}`

#### Step 6：修改 `TensorStreamArrived` 入站处理

修改 `Src/Orchestrator/core.rs` 的 `handle_network_inbound` 方法中 `TensorStreamArrived` 分支：

1. 读取 handshake → 得到 `inference_id`
2. **优先尝试** `tensor_switch.Register_Inbound(inference_id, stream)`（新路径）
3. **后备** 检查 `pending_distributed_jobs`（旧路径兼容）

这样新旧两种模式在过渡期内可以共存。

#### 验证方式

通过集成测试模拟 Coordinator → Worker 的命令流：
1. 构造 `ESTABLISH_TENSOR_STREAM` payload → 发送给 Worker Core → 验证回复 OK + outbound 已注册
2. 构造 `JOIN_PIPELINE` payload → 发送给 Worker Core → 验证回复 OK + Job 已 spawn
3. 验证 `TensorStreamArrived` 事件能正确触发 `Register_Inbound`

---

### Phase 2：Coordinator 侧编排实现

当前阶段目标：实现 **Coordinator 侧的完整 Pipeline 启动**流程，包括用户命令、compile_relay 补全、三阶段编排逻辑。

#### Step 1：新增 `UserCommand::Pipeline`

修改 `Src/Orchestrator/command.rs`：

```rust
/// 启动分布式流水线推理
///
/// Coordinator 自动完成：分析模型 → 查询可用节点 → 规划拓扑 → 分发模型 → 建立流水线。
/// 用户只需提供模型路径，其余由编排逻辑自动处理。
///
/// 回复：`Ok(JobId)` Pipeline Job 已启动；`Err(String)` 任一阶段失败
Pipeline {
    model_path: String,
    reply: oneshot::Sender<Result<JobId, String>>,
}
```

同步修改：
- `Src/TUI/command_panel.rs`：新增 `pipeline <model_path>` 解析
- `Src/Orchestrator/core.rs`：`route_user` 新增 `UserCommand::Pipeline` 分支（`todo!()` 占位）

#### Step 2：实现 `compile_relay`

修改 `Src/Orchestrator/compiler.rs`：

**签名简化**（移除无用参数）：

```rust
pub fn compile_relay(
    &self,
    _job_id: JobId,
    model_file_id: String,
    device: String,
    layer_start: usize,
    layer_end: usize,
) -> Result<TaskProgram, CompilerError>
```

**生成的 TaskProgram**：

```text
正向序列：
  Const(model_file_id) → SLOT_MODEL
  Const(device) → SLOT_DEVICE
  Const(layer_start) → SLOT_LAYER_START
  Const(layer_end) → SLOT_LAYER_END
  CreateSession { model, device, start, end, io, tensor_io: Some(SLOT_TENSOR_IO), result: SLOT_SESSION }
  RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }

补偿序列：
  ShutdownSession { session: SLOT_SESSION }
```

**RunProgram 提交的 ML 指令序列**（由 `Build_Relay_ML_Program()` 生成）：

```rust
fn Build_Relay_ML_Program() -> Vec<Instruction> {
    vec![
        Instruction::Loop(vec![
            Instruction::Receive,
            Instruction::BreakIf(FLAG1),
            Instruction::Inference(Inference_Input::Tensor(TENSOR1)),
            Instruction::Send,
        ]),
    ]
}
```

Worker Relay 程序语义：接收上游张量 → 检查 EOF → 推理 → 发送到下游，循环直到 EOF。

#### Step 3：Pipeline 编排指令（已完成 ✅，已精简）

将编排逻辑建模为 **TaskInstruction**，由 TaskEngine 执行，保持架构一致性。
Pipeline 编排 = 一个 Job（JobKind::Pipeline），享有 registry 管理、cancel 支持、compensation 自动清理。

**最终保留的指令**（`Src/Orchestrator/instruction.rs`）：

| 指令 | 用途 | handler |
|------|------|---------|
| `EstablishStreams { plan, result }` | Phase 1: Coordinator 自身建流 + 并发命令 Workers 建流 | `handler_network.rs` |
| `JoinWorkers { plan, result }` | Phase 2: 并发通知 Workers 加入流水线 | `handler_network.rs` |
| `TeardownPipeline { plan }` | 补偿: 清理已建立的资源 | `handler_network.rs` |

**已移除的指令**：
- `PlanPipeline` — Scheduler 尚未实现，在 compile_pipeline 中用 todo! 占位
- `DistributeShards` — 模型分发是独立操作（用户先执行 `distribute` 命令）

#### Step 4：实现 `compile_pipeline`

修改 `Src/Orchestrator/compiler.rs`：

**签名**：

```rust
pub fn compile_pipeline(
    &self,
    _job_id: JobId,
    model_path: String,
    device: String,
) -> Result<TaskProgram, CompilerError>
```

**生成的 TaskProgram**：

```text
正向序列：
  [TODO: Scheduler → 生成 plan 写入 SLOT_PLAN]
       plan 包含: inference_id, Vec<(peer_id, start, end)>, coord_layer_start, coord_layer_end
  EstablishStreams { plan: SLOT_PLAN, result: SLOT_STREAMS_RESULT }
       1) Coordinator 自己先向下一跳节点打开出站流 + handshake + Register_Outbound
       2) for 每个 Worker: send_request(ESTABLISH_TENSOR_STREAM)
       3) 等待全部 OK
       4) Create_Endpoint(inference_id) → 写入 SLOT_TENSOR_IO
  JoinWorkers { plan: SLOT_PLAN, result: SLOT_WORKERS_RESULT }
       for 每个 Worker: send_request(JOIN_PIPELINE), 等待全部 OK (超时 300s)
  Const("coordinator") → SLOT_ML_PROGRAM_MODE
  CreateSession { model, device, start, end, io, tensor_io: Some(SLOT_TENSOR_IO), result: SLOT_SESSION }
  RunProgram { session: SLOT_SESSION, result: SLOT_RESULT }

补偿序列：
  TeardownPipeline { plan: SLOT_PLAN }
  ShutdownSession { session: SLOT_SESSION }
```

**Core 中的调用**（替换当前 `todo!()` 占位）：

```rust
UserCommand::Pipeline { model_path, reply } => {
    let job_id = JobId(generate_id());
    let device = if self.device_preference.is_empty() { "cpu".to_string() } else { self.device_preference.clone() };
    let program = match self.compiler.compile_pipeline(job_id, model_path, device) {
        Ok(p) => p,
        Err(e) => {
            let _ = reply.send(Err(format!("编译失败: {:?}", e)));
            return;
        }
    };
    // Pipeline Job 需要 IO 通道（Coordinator 推理需要与前端交互）
    if let Err(e) = self.capabilities.io_broker.Allocate(job_id).await {
        let _ = reply.send(Err(format!("IO 分配失败: {}", e)));
        return;
    }
    match self.capabilities.io_broker.Take_ML_Side(job_id).await {
        Ok(io) => {
            self.spawn_job(job_id, JobKind::Pipeline, program, Some(io));
            let _ = reply.send(Ok(job_id));
        }
        Err(e) => {
            let _ = reply.send(Err(format!("IO Take_ML_Side 失败: {}", e)));
        }
    }
}
```

Pipeline Job 由 TaskEngine 在独立的 `tokio::spawn` task 中执行（JobExecutor::run），
不会阻塞 Core 的 select! 循环。各 handler 内部自由 await 网络操作。

**RunProgram 提交的 ML 指令序列**（由 `Build_Coordinator_ML_Program()` 生成）：

```text
Input → Encode → Set(max_tokens) → Prefill(TOKENID3) → CopyMeta(META5→META1)
→ Send → Receive → Sample(TENSOR1) → Decode → Output
→ Loop [ BreakIf, Inference(TOKENID2), Send, Receive, Sample(TENSOR1), Decode, Output ]
→ SendEOF → EndOutput
```

#### Step 5：逐步实现各 handler

按顺序实现 `handler_network.rs` 中的 3 个 Pipeline handler 方法：

| # | handler | 核心逻辑 |
|---|---------|----------|
| 1 | `handle_establish_streams` | Coordinator 自身建流 + 并发 `send_request(ESTABLISH_TENSOR_STREAM)` + Create_Endpoint |
| 2 | `handle_join_workers` | 并发 `send_request(JOIN_PIPELINE)`，超时 300s |
| 3 | `handle_teardown_pipeline` | `tensor_switch.Deregister_Pipeline` + 通知 Worker 清理 |

#### Step 6：端到端验证

多节点联调测试：Coordinator 节点执行 `pipeline <model_path>` → 自动完成全流程。

### Phase 2 实施顺序

| # | 步骤 | 状态 | 依赖 | 复杂度 |
|---|------|------|------|--------|
| 1 | UserCommand::Pipeline + TUI 解析 + Core todo | ✅ 完成 | 无 | 低 |
| 2 | compile_relay 实现 + Core Join_Pipeline 签名适配 | ✅ 完成 | Step 1 | 中 |
| 3 | Pipeline 编排指令精简（仅 Establish/Join/Teardown） | ✅ 完成 | Step 2 | 低 |
| 4 | compile_pipeline 实现 + Core route_user 接入 | | Step 3 | 中 |
| 5 | 逐步实现 handler_network 中 Pipeline handler | | Step 4 | 高 |
| 6 | 端到端验证（多节点联调） | | Step 5 | 高 |
