# Handler 设计文档

**日期**：2026-04-27  
**基线**：12 条指令已定义，handler_data / handler_control 已完整实现，handler_inference 已完整实现（CreateSession / ShutdownSession / RunProgram / AnalyzeModel / SplitModel），handler_network 全部占位

---

## 1. 总览

Handler 是 TaskEngine 执行指令的实际逻辑层。每条 `TaskInstruction` 对应一个 handler 方法，由 `task_engine.rs` 的 `step()` 方法通过 `match` 分派。

### 架构图

```
TaskEngine::step()
    │
    ├── match TaskInstruction::Const      → handler_data.rs::handle_const()
    ├── match TaskInstruction::Move       → handler_data.rs::handle_move()
    ├── match TaskInstruction::CreateSession    → handler_inference.rs::handle_create_session()
    ├── match TaskInstruction::ShutdownSession  → handler_inference.rs::handle_shutdown_session()
    ├── match TaskInstruction::RunProgram       → handler_inference.rs::handle_run_program()
    ├── match TaskInstruction::AnalyzeModel     → handler_inference.rs::handle_analyze_model()
    ├── match TaskInstruction::SplitModel       → handler_inference.rs::handle_split_model()
    ├── match TaskInstruction::SendFile         → handler_network.rs::handle_send_file()
    ├── match TaskInstruction::ReceiveFile      → handler_network.rs::handle_receive_file()
    ├── match TaskInstruction::OpenTensorStream → handler_network.rs::handle_open_tensor_stream()
    ├── match TaskInstruction::JumpIf    → handler_control.rs::handle_jump_if()
    └── match TaskInstruction::Abort     → handler_control.rs::handle_abort()
```

### Handler 通用约定

- 所有 handler 方法签名：`pub(super) [async] fn handle_xxx(&mut self, ...) -> StepResult`
- 通过 `self.slots` 访问 SlotFile（读写槽位）
- 通过 `self.capabilities` 访问外部能力（Storage / ML_Engine / Network / IO_Broker）
- 返回值语义：
  - `StepResult::Continue` — 执行成功，ip 已由 step() 自动 +1
  - `StepResult::Abort(reason)` — 执行失败，触发补偿链
  - `StepResult::Done` — 程序执行完毕（仅由 step() 内部生成，handler 不直接返回）

---

## 2. handler_data.rs — 数据操作

**状态**：✅ 已完成（含测试）

### 2.1 handle_const(value: ConstValue, dst: SlotId) → StepResult

**作用**：将编译期常量写入槽位。

**输入**：
- `value: ConstValue` — 可 Clone 的值子集（Nil / Bool / U64 / String / PathBuf / Error）
- `dst: SlotId` — 目标槽位

**逻辑**：
1. `value.into()` 转换为 `SlotValue`
2. `self.slots.set(dst, value)` 写入
3. 返回 `Continue`

**错误处理**：无（set 总是成功，覆盖旧值）

### 2.2 handle_move(src: SlotId, dst: SlotId) → StepResult

**作用**：从源槽位取出值（take 语义），写入目标槽位。用于转移不可 Clone 资源（如 IoHandle）。

**输入**：
- `src: SlotId` — 源槽位（take 后变 Nil）
- `dst: SlotId` — 目标槽位

**逻辑**：
1. `self.slots.take(src)` 取出值
2. 若为 `Some(value)` → `self.slots.set(dst, value)` → `Continue`
3. 若为 `None` → `Abort("source slot is empty")`

---

## 3. handler_control.rs — 控制流

**状态**：✅ 已完成（含测试）

### 3.1 handle_jump_if(condition: SlotId, label: &str) → StepResult

**作用**：条件跳转。条件为真时，ip 跳转到 label 对应的指令索引。

**输入**：
- `condition: SlotId` — 存放 Bool 值的槽位
- `label: &str` — 跳转目标标签名

**逻辑**：
1. `self.slots.get_bool(condition)` 读取布尔值
2. 若 `false` → 保持 ip 不变（step() 已 +1）→ `Continue`
3. 若 `true` → 查找 `self.program.labels[label]` → 覆盖 `self.ip = target` → `Continue`

**错误处理**：
- 槽位为空 / 类型不匹配 → `Abort`
- 标签不存在 → `Abort`

### 3.2 handle_abort(reason: &str) → StepResult

**作用**：立即终止正向执行，触发补偿链。

**逻辑**：直接返回 `Abort(reason.to_string())`

---

## 4. handler_inference.rs — 推理生命周期

**状态**：✅ 已完成（含测试）

### 4.1 handle_create_session(model, device, start, end, io, result) → StepResult 【已实现】

**作用**：创建 ML 推理 Session。

**输入槽位**：
- `model: SlotId` → String（Storage 的 file_id）
- `device: SlotId` → String（"cpu" / "cuda"，缺失默认 "cpu"）
- `start: SlotId` → U64（起始层号，缺失默认 0）
- `end: SlotId` → U64（结束层号，缺失默认 usize::MAX）
- `io: SlotId` → IoHandle（take 语义，消费一次）
- `result: SlotId` → 写入 session_id

**逻辑**：
1. `get_string(model)` → model_file_id
2. `get_string(device)` → device_str（默认 "cpu"）
3. `get_u64(start)` → layer_start（默认 0）
4. `get_u64(end)` → layer_end（默认 usize::MAX）
5. `take_io_handle(io)` → io_handle
6. 构造 `ML_Session_Config { session_id: "job-{job_id}", model_file_id, layer_start, layer_end, device, tensor_io: None }`
7. `caps.ml_engine.Create_Session(config, io_handle).await`
8. 成功 → `set(result, String(session_id))` → `Continue`
9. 失败 → `Abort`

**调用的外部能力**：`caps.ml_engine.Create_Session()`

### 4.2 handle_shutdown_session(session) → StepResult 【已实现】

**作用**：关闭 ML Session，释放资源。**尽力清理，不 Abort**。

**输入槽位**：
- `session: SlotId` → String（session_id）

**逻辑**：
1. `get_string(session)` → session_id（读不到直接 Continue）
2. `caps.ml_engine.Shutdown_Session(&session_id).await`（忽略错误）
3. 始终 `Continue`

**设计决策**：作为补偿指令使用，即使 Shutdown 失败也不应再次 Abort。

### 4.3 handle_run_program(session, result) → StepResult 【已实现】

**作用**：向 Session 提交 ML 指令序列执行推理。

**输入槽位**：
- `session: SlotId` → String（session_id）
- `result: SlotId` → 写入 "done" 完成标记

**逻辑**：
1. `get_string(session)` → session_id
2. `Pipeline_Params::default()` → params（max_tokens=120, temperature=0.8）
3. `Compiler::build_run_ml_program(&params)` → ML 指令序列
4. `caps.ml_engine.Run_Program(session_id, program, params, cancel_flag).await`
5. 成功 → `set(result, String("done"))` → `Continue`
6. 失败 → `Abort`

**调用的外部能力**：`caps.ml_engine.Run_Program()`

### 4.4 handle_analyze_model(model, result) → StepResult 【已实现】

**作用**：分析模型文件结构，获取 Model_Info（架构名、层数、input/output head 等）。分布式推理中 Coordinator 用于决策模型切分方案。

**输入槽位**：
- `model: SlotId` → String（file_id）
- `result: SlotId` → 写入 SlotValue::ModelInfo(Model_Info)

**逻辑**：
1. `get_string(model)` → model_file_id
2. `caps.ml_engine.Analyze_Model(&model_file_id).await`
3. 成功 → `set(result, SlotValue::ModelInfo(model_info))` → `Continue`
4. 失败 → `Abort`

**调用的外部能力**：`caps.ml_engine.Analyze_Model()`

**设计决策**：采用方案 B — 新增 `SlotValue::ModelInfo(Model_Info)` 变体，类型安全，无需序列化/反序列化。同时在 `SlotFile` 中添加 `get_model_info(slot)` 便利方法。

### 4.5 handle_split_model(source, start, end, output) → StepResult 【已实现】

**作用**：切分模型文件为指定层范围的子模型。Coordinator 用于将模型分片发送给 Worker。

**输入槽位**：
- `source: SlotId` → String（源模型 file_id）
- `start: SlotId` → U64（起始层号）
- `end: SlotId` → U64（结束层号）
- `output: SlotId` → String（输出文件 file_id）

**逻辑**：
1. `get_string(source)` → source_file_id
2. `get_u64(start)` → layer_start（as usize）
3. `get_u64(end)` → layer_end（as usize）
4. `get_string(output)` → output_file_id
5. `caps.ml_engine.Split_Model(&source_file_id, layer_start, layer_end, &output_file_id).await`
6. 成功 → `Continue`
7. 失败 → `Abort`

**调用的外部能力**：`caps.ml_engine.Split_Model()`

---

## 5. handler_network.rs — 网络操作

**状态**：✅ SendFile/ReceiveFile 已实现，OpenTensorStream P2 占位

### 5.0 文件传输协议概述

文件传输采用**三阶段协议**：元数据协商 → Stream 数据传输 → 校验确认。
每个 Stream 只传输一个文件。多文件传输需要多次完整的三阶段流程（串行）。

```
阶段1：元数据协商（Request-Response 协议，使用 DataType::File）
─────────────────────────────────────────────────────────────────
A (发送方)                                    B (接收方)
    │                                              │
    │  send_data(peer_B, DataType::File,           │
    │    payload={action:"send",                   │
    │      file_name, file_size, checksum})         │
    ├─────────────────────────────────────────────→ │
    │                                              │  Inbound_Manager → Control → Core
    │                                              │  Core 解析元数据，决定接受/拒绝
    │                                              │  接受 → 记录 pending {peer, file_name, file_size, checksum}
    │         send_response(request_id,            │
    │           DataType::File, {accept/reject})   │
    │ ←────────────────────────────────────────────┤
    │                                              │

阶段2：文件数据传输（Stream 协议）
─────────────────────────────────────────────────────────────────
    │  open_file_stream(peer_B) → Stream           │
    ├─────────────────────────────────────────────→ │  Network_Service: FileStreamArrived
    │                                              │  Core 匹配 pending → 取出元数据
    │                                              │  Core spawn ReceiveFile Job
    │                                              │    注入 stream + file_name + file_size + checksum
    │  send_file_data(&mut stream, &path)          │
    │  ════════════════chunk 1, 2, ...═══════════  │  receive_file_data(&mut stream, &dest, size)
    │                                              │  → 写入 Storage
    │  发送完毕                                     │  计算 checksum，匹配 → 保留，不匹配 → 删除
    │                                              │  ReceiveFile Job Done
    │                                              │

阶段3：发送方主动验证（Request-Response 协议，使用 DataType::Command + Verify_File 命令）
─────────────────────────────────────────────────────────────────
    │  send_data(peer_B, DataType::Command,        │
    │    Serialize_Command(Verify_File{file_name})) │
    ├─────────────────────────────────────────────→ │
    │                                              │  Handle_Control_Command 处理 Verify_File
    │                                              │  检查 Storage 中文件是否存在
    │                                              │  存在 → confirmed
    │                                              │  不存在（checksum 不匹配已删除）→ failed
    │         send_response(request_id,            │
    │           DataType::Command,                 │
    │           b"confirmed" / b"failed")          │
    │ ←────────────────────────────────────────────┤
    │  confirmed → Continue                        │
    │  failed → Abort                              │
```

**设计决策**：
- 发送方（handle_send_file）在一个 handler 内完成阶段1 + 阶段2 + 阶段3（主动发送 verify 请求）
- 阶段3 使用 `DataType::Command` + `Control_Command::Verify_File` 命令（而非 `DataType::File`），避免 File 类型语义膨胀，复用现有命令序列化/反序列化机制
- 发送方主动查询（方案B），handler 只需 `send_data(DataType::Command, Serialize_Command(&Verify_File{...})).await` 即可获取结果，无需额外的通道注入机制
- 接收方由 **Core 分步处理**：阶段1 在 Core.route_network() 内处理（元数据协商），阶段2 在 ReceiveFile Job 内完成（接收数据 + 校验），阶段3 verify 由 Core 直接响应
- checksum 使用 `StorageManager::checksum()` 计算（支持 blake3/sha256/xxhash64）
- 文件接收是**独立 Job**，与推理无关。先接收文件存入 Storage，后续推理 Job 引用 Storage 中的 file_id

### 5.1 handle_send_file(peer, file) → StepResult 【已实现】

**作用**：将本地 Storage 中的文件通过三阶段协议发送到远端 Peer。

**输入槽位**：
- `peer: SlotId` → String（PeerId 字符串，需 parse 为 `libp2p::PeerId`）
- `file: SlotId` → String（Storage 的 file_id）

**逻辑**（三阶段均在 handler 内完成）：

```
阶段1 — 元数据协商（含 checksum）：
1.  get_string(peer) → peer_id_str → peer_id_str.parse::<PeerId>()
2.  get_string(file) → file_id
3.  caps.storage.acquire_read(&file_id).await → (path, read_guard)
4.  获取文件大小：tokio::fs::metadata(&path).await → file_size
5.  计算校验和：caps.storage.checksum(&file_id, "blake3").await → checksum
6.  序列化元数据 payload: {file_name: file_id, file_size, checksum}
7.  caps.network.send_data(peer_id, DataType::File, metadata_payload).await → response
8.  解析 response：reject → Abort("对方拒绝接收文件")

阶段2 — 文件数据传输：
9.  caps.network.open_file_stream(peer_id).await → stream
10. caps.network.send_file_data(&mut stream, &path).await
11. drop(read_guard) — 释放读锁

阶段3 — 发送方主动验证（发送 Verify_File 命令，复用 Request-Response）：
12. Serialize_Command(&Control_Command::Verify_File { file_name }) → verify_payload
13. caps.network.send_data(peer_id, DataType::Command, verify_payload).await → response
14. 解析 response payload：
    - b"confirmed" → Continue（传输成功）
    - b"failed" → Abort("接收方校验失败，文件已被删除")
```

**错误处理**：
- PeerId 解析失败 → Abort
- acquire_read 失败（文件不存在）→ Abort
- checksum 计算失败 → Abort
- send_data 超时/失败（阶段1/3，阶段3 使用 DataType::Command）→ Abort
- 对方 reject → Abort
- open_file_stream / send_file_data 失败 → Abort
- 阶段3 verify response 为 "failed" → Abort

**调用的外部能力**：
- `caps.storage.acquire_read()` — 获取文件路径 + 读锁（生命周期覆盖阶段1-2）
- `caps.storage.checksum()` — 计算文件校验和（blake3）
- `caps.network.send_data()` — 阶段1 发送元数据（DataType::File） + 阶段3 发送 Verify_File 命令（DataType::Command）
- `caps.network.open_file_stream()` — 打开到目标 Peer 的文件流（阶段2）
- `caps.network.send_file_data()` — 通过流发送文件原始数据（阶段2）

### 5.1.1 Core 层 — 发送文件 Job 的创建

发送文件由上层主动触发（如 Coordinator 编排分发模型），Core 通过 compile 生成包含 SendFile 指令的 TaskProgram 并 spawn Job。

```
触发：UserCommand 或编排层调度
→ Core.compile_send_file(peer_str, file_id)
→ TaskProgram: [Const(peer) → Const(file) → SendFile(peer, file)]
→ spawn_job()
→ handle_send_file 在 Job 内执行三阶段
```

### 5.2 handle_receive_file(stream, file_name, file_size, checksum, result) → StepResult 【已实现】

**作用**：从入站 Stream 接收文件数据并存入本地 Storage。接收后本地校验 checksum，不匹配则删除。校验结果由发送方通过阶段3 verify 请求主动查询（Core 直接响应）。作为独立 Job 运行，与推理无关。

**输入槽位**（全部由 Core 在 spawn Job 时注入 SlotFile）：
- `stream: SlotId` → SlotValue::Stream（libp2p::Stream，take 语义）— 由 Core 注入
- `file_name: SlotId` → String（文件名，来自阶段1元数据协商）— 由 Core 注入
- `file_size: SlotId` → U64（文件大小，来自阶段1元数据协商）— 由 Core 注入
- `checksum: SlotId` → String（发送方校验和，来自阶段1元数据协商）— 由 Core 注入
- `result: SlotId` → 写入 Storage 注册后的 file_id

**逻辑**（阶段2 接收数据 + 本地 checksum 校验）：
```
1.  take_stream(stream) → libp2p::Stream
2.  get_string(file_name) → file_name_str
3.  get_u64(file_size) → file_size_val
4.  get_string(checksum) → expected_checksum
5.  caps.storage.acquire_write(&file_name_str).await → (dest_path, write_guard)
6.  caps.network.receive_file_data(&mut stream, &dest_path, file_size_val).await
7.  drop(write_guard) — 释放写锁，文件注册到 Storage 索引
8.  caps.storage.checksum(&file_name_str, "blake3").await → actual_checksum
9.  比较 actual_checksum == expected_checksum：
    - 匹配：set(result, String(file_name_str)) → Continue
    - 不匹配：caps.storage.remove(&file_name_str).await → Abort("checksum mismatch")
```

**错误处理**：
- stream / file_name / file_size / checksum 槽位缺失或类型不匹配 → Abort
- acquire_write 失败 → Abort
- receive_file_data 失败（流中断、IO 错误）→ Abort
- checksum 计算失败 → Abort
- checksum 不匹配 → 删除文件 + Abort（发送方通过阶段3 verify 发现）

**调用的外部能力**：
- `caps.storage.acquire_write()` — 获取写锁和目标路径
- `caps.network.receive_file_data()` — 从入站流读取文件数据写入磁盘
- `caps.storage.checksum()` — 计算接收文件的校验和（blake3）
- `caps.storage.remove()` — 删除 checksum 不匹配的损坏文件

**指令定义需扩展**（现有定义只有 `result`，需增加 `stream`、`file_name`、`file_size`、`checksum`）：
```rust
ReceiveFile {
    stream: SlotId,     // SlotValue::Stream — Core 注入
    file_name: SlotId,  // String — 来自元数据协商
    file_size: SlotId,  // U64 — 来自元数据协商
    checksum: SlotId,   // String — 发送方校验和
    result: SlotId,     // 输出：file_id
}
```

### 5.2.1 Core 层 — 接收文件的两步流程

接收文件由 Core 被动响应网络入站事件，分两步完成：

**步骤1：元数据协商（Core.route_network 处理入站 DataType::File 请求）**

```
Network_Service → Inbound_Manager → Control → Core.route_network()
    入站请求: DataType::File, payload = {action: "send", file_name, file_size, checksum}
    │
    Core 解析元数据（根据 action 字段路由）
    │
    action == "send" → 阶段1 元数据协商：
    │  检查条件（空间、权限等，初期可直接 accept）
    │
    ├── 决定 accept:
    │   1. Core.pending_file_transfers.insert(peer, {file_name, file_size, checksum})
    │   2. caps.network.send_response(request_id, DataType::File, {status: "accept"})
    │
    └── 决定 reject:
        1. caps.network.send_response(request_id, DataType::File, {status: "reject"})
        2. 不记录 pending，流程结束
```

**步骤2：文件流到达，spawn ReceiveFile Job（Core.route_network 处理 FileStreamArrived 事件）**

```
Network_Service → FileStreamArrived { peer, stream }
    │
    Core.route_network() 处理:
    │
    1. 从 pending_file_transfers 查找匹配 peer
    │  未找到 → warn 并丢弃 stream（可能协商已超时或被取消）
    │  找到 → 取出 {file_name, file_size, checksum}，移除 pending 条目
    │
    2. compile_receive_file() → TaskProgram
    │  TaskProgram 只含一条指令: ReceiveFile {
    │    stream: SLOT_STREAM, file_name: SLOT_FILE_NAME,
    │    file_size: SLOT_FILE_SIZE, checksum: SLOT_CHECKSUM, result: SLOT_RESULT
    │  }
    │
    3. 创建 JobExecutor，注入 SlotFile:
    │  slots.set(SLOT_STREAM, SlotValue::Stream(stream))
    │  slots.set(SLOT_FILE_NAME, SlotValue::String(file_name))
    │  slots.set(SLOT_FILE_SIZE, SlotValue::U64(file_size))
    │  slots.set(SLOT_CHECKSUM, SlotValue::String(checksum))
    │
    4. spawn_job() — ReceiveFile Job 开始执行
    │  handler 完成阶段2（接收数据 + 本地 checksum 校验）
```

**Core 需新增的状态**：
```rust
/// 等待中的文件传输（阶段1已协商 accept，等待阶段2的 FileStreamArrived）
struct PendingFileTransfer {
    file_name: String,
    file_size: u64,
    checksum: String,  // 发送方提供的校验和（blake3）
}

/// Core 新增字段
pending_file_transfers: HashMap<PeerId, VecDeque<PendingFileTransfer>>,
// 使用 VecDeque 支持同一 peer 的多个串行传输（FIFO 匹配）
```

**步骤3：处理 Verify_File 命令（Control 层处理入站 DataType::Command, Control_Command::Verify_File）**

发送方在阶段3主动发送 `Verify_File` 命令，接收方 Control 层通过现有命令路由机制直接回复（不需要 Job 参与）：

```
Network_Service → Inbound_Manager → Control → Handle_Control_Command()
    入站请求: DataType::Command, payload = "VERIFY_FILE|<file_name>"
    │
    Deserialize_Command → Control_Command::Verify_File { file_name }
    │
    1. caps.storage.exists(&file_name) → file_exists?
    │
    ├── 文件存在（接收成功，checksum 校验通过）：
    │   send_response(request_id, DataType::Command, b"confirmed")
    │
    └── 文件不存在（checksum 不匹配，已被 ReceiveFile Job 删除 / 接收失败）：
        send_response(request_id, DataType::Command, b"failed")
```

**设计优势**：
- Verify_File 作为 `Control_Command` 变体，复用现有的命令序列化/反序列化机制（文本协议，`|` 分隔）
- `DataType::File` 只用于阶段1 的文件元数据协商，语义清晰不膨胀
- Control 层 `Handle_Control_Command` 的 match 自动路由，不需要解析 payload 中的 action 字段
- 发送方的 handler 只需 `send_data(DataType::Command, Serialize_Command(&Verify_File{...})).await` 即可获取最终确认

### 5.3 handle_open_tensor_stream(peer, result) → StepResult 【占位符】

**作用**：与远端 Peer 建立 Tensor Stream 连接，用于分布式推理的中间层张量传输。

**输入槽位**：
- `peer: SlotId` → String（PeerId 字符串）
- `result: SlotId` → 写入 Tensor_IO_Handle

**逻辑**：
1. `get_string(peer)` → peer_id_str → `peer_id_str.parse::<PeerId>()`
2. `caps.network.open_tensor_stream(peer_id).await` → outbound Stream
3. 获取入站 tensor stream（**需要入站流注入，见 §5.3.1**）
4. 构造 `Tensor_IO_Handle::New(inbound_stream, outbound_stream, runtime_handle)`
5. `set(result, SlotValue::TensorIo(tensor_io_handle))` → `Continue`
6. 失败 → `Abort`

**调用的外部能力**：
- `caps.network.open_tensor_stream()` — 打开出站张量流

**需要扩展**：
- `SlotValue` 新增 `TensorIo(Tensor_IO_Handle)` 变体
- `SlotFile` 新增 `take_tensor_io(slot)` 便利方法
- `handler_create_session` 修改：从 tensor_io 槽位读取 `Tensor_IO_Handle`，传入 `ML_Session_Config.tensor_io`

### 5.3.1 入站张量流注入

入站张量流来自 `Network_Inbound_Event::TensorStreamArrived { peer, stream }`。

与文件接收不同，张量流需要在**已有 Job 内使用**（作为 CreateSession 的 tensor_io 参数）。采用与文件接收类似的 Core 注入方案：

- Core 收到 `TensorStreamArrived` 时，将 stream 注入到对应 Job 的约定槽位（`SLOT_INBOUND_TENSOR_STREAM`）
- 或者 Core 在创建 Relay/Coordinator Job 时，预留一个 `mpsc::Receiver<Stream>` 通道
- handler_open_tensor_stream 从 SlotFile 或通道中获取入站流

**具体方案待 Tensor Stream 实现时决定**，当前优先实现 SendFile 和 ReceiveFile。

---

## 6. SlotValue 扩展需求

当前 SlotValue 变体：

| 变体 | 用途 | Clone? |
|------|------|:------:|
| Nil | 空槽位 | ✅ |
| Bool(bool) | 控制流条件 | ✅ |
| U64(u64) | 数值参数 | ✅ |
| String(String) | 字符串参数 | ✅ |
| PathBuf(PathBuf) | 路径参数 | ✅ |
| SessionHandle | ML Session（Stub） | ✅ |
| Error(String) | 错误传递 | ✅ |
| IoHandle(IoHandle) | LLM IO 句柄 | ❌ |
| ModelInfo(Model_Info) | 模型分析结果 | ✅（Model_Info derives Clone） |

**需新增**：

| 变体 | 用途 | Clone? | 对应 take 方法 | 优先级 |
|------|------|:------:|---------------|-------|
| Stream(libp2p::Stream) | 入站文件/张量流 | ❌ | take_stream() | P1 |
| TensorIo(Tensor_IO_Handle) | 张量流句柄 | ❌ | take_tensor_io() | P2 |

---

## 7. 实施优先级

```
✅  handle_analyze_model      — 已完成，含 3 个测试
✅  handle_split_model        — 已完成，含 7 个测试
P1  handle_send_file          — 两阶段协议（元数据协商 + Stream 传输），需 PeerId 解析辅助
P1  handle_receive_file       — Stream 从 SlotFile 读取，Core 层需实现 pending_file_transfers + spawn 逻辑
P1  SlotValue::Stream + ReceiveFile 指令扩展 — handle_receive_file 的前置依赖
P2  handle_open_tensor_stream — 需 SlotValue::TensorIo 扩展 + 入站张量流注入机制
```

---

## 8. 依赖关系图

```
SlotValue::Stream 扩展
    │
    ├──→ ReceiveFile 指令扩展 (stream + file_name + file_size + result)
    │         │
    │         └──→ handle_receive_file
    │
    └──→ Core 层 pending_file_transfers + spawn ReceiveFile Job

handle_send_file
    │
    ├── caps.storage.acquire_read()    — 读取文件路径 + 大小
    ├── caps.network.send_data()       — 阶段1 元数据协商
    ├── caps.network.open_file_stream() — 阶段2 打开文件流
    └── caps.network.send_file_data()  — 阶段2 发送文件数据

Core 层接收文件流程
    │
    ├── route_network(DataType::File)  — 阶段1 解析元数据 + accept/reject
    │       └── pending_file_transfers.insert()
    │
    └── route_network(FileStreamArrived) — 阶段2 匹配 pending + spawn ReceiveFile Job
            └── compile_receive_file() + slot 注入 + spawn_job()

SlotValue::TensorIo 扩展（后续）
    │
    ├──→ handle_open_tensor_stream
    │         │
    │         └──→ handle_create_session 修改（读取 tensor_io 从槽位）
    │
    └──→ compile_coordinator / compile_relay

PeerId 解析辅助（共用）
    │
    ├──→ handle_send_file
    └──→ handle_open_tensor_stream
```
