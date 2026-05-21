# Core::run() 事件分支与请求处理全览

**日期**：2026-04-29
**基线**：Phase 2C 完成，Control 层计划移除，所有入站逻辑迁移至 Orchestrator Core，Network DataType 预筛选已实现，B2(NetworkCommand) 合并到 B2(InboundRequest)

---

## 1. Core::run() select! 分支总览

Core 事件循环共 4 个 select! 分支，每个对应一个外部事件源：

| # | 通道 | 类型 | 来源 | 守卫条件 | 处理方法 | 状态 |
|---|------|------|------|---------|---------|------|
| B1 | `user_cmd_rx` | `mpsc::Receiver<UserCommand>` | TUI / CLI | `!shutting_down` | `route_user()` | ✅ |
| B2 | `inbound_rx` | `mpsc::Receiver<InboundRequest>` | Network Request-Response（仅 Command + File） | `!shutting_down` | `handle_inbound_request()` | ❌ 需新建 |
| B3 | `network_inbound_rx` | `mpsc::Receiver<Network_Inbound_Event>` | Network Stream（文件流 + 张量流） | `!shutting_down` | `handle_network_inbound()` | ⚠️ 部分 |
| B4 | `lifecycle_rx` | `mpsc::Receiver<LifecycleEvent>` | JobExecutor | 无（始终活跃） | `handle_lifecycle_event()` | ✅ |

> **B2 来源**：Network_Service 收到 Request-Response 入站请求后，按 `DataType` **预筛选**（✅ 已实现）：仅将 `Command` 和 `File` 类型转发给 Core，其余类型（`BandwidthTest`、`Data`、`Info`）在 Network 内部直接处理并回复（见第 8 节）。

> **设计变更**：原 B2（`network_cmd_rx` / `NetworkCommand`）已合并到新 B2 中。`REQUEST_PIPELINE` 入站命令由 `handle_inbound_request()` 直接调用 `route_pipeline_flow()`，不再通过独立 channel 绕路。`NetworkCommand` 枚举和 `network_cmd_rx` 通道将被移除。

---

## 2. B1：UserCommand 分支（route_user）

### 2.1 已实现

| 命令 | 流程 | reply 类型 |
|------|------|-----------|
| `Run { model_path, reply }` | compile_run → Allocate IO → Take_ML_Side → spawn_job → reply(Ok(job_id)) | `Result<JobId, String>` |
| `Cancel { job_id, reply }` | 查 registry → cancel_job → reply | `Result<(), String>` |
| `Quit { reply }` | shutdown() → reply | `()` |
| `DisplayPeer { reply }` | peer_manager.List_Peers() → 格式化 → reply | `Result<Vec<String>, String>` |
| `DistributeModel { model_path, peers, reply }` | compile_distribute → Allocate IO → spawn_job → reply | `Result<JobId, String>` |

### 2.2 未实现

| 命令 | 流程 | reply 类型 | 说明 |
|------|------|-----------|------|
| `SetDevice { device, reply }` | 存储 device_preference 到 Core 内部状态 | `Result<(), String>` | 当前 TODO 占位 |
| `DistributeRun { model_path, peers, device_preference, layer_start, layer_end, reply }` | compile_coordinator → Allocate IO → tensor_io_broker.Prepare → spawn_job → reply | `Result<JobId, String>` | 分布式推理入口，UserCommand 变体尚未添加 |

---

## 3. B2：InboundRequest 分支（handle_inbound_request）— 需新建

这些是 **Request-Response 协议** 的入站请求。Network_Service 按 DataType 预筛选后，仅将 `Command` 和 `File` 类型转发到此分支。每个请求携带 `request_id`，处理完后必须通过 `network.send_response(request_id, ...)` 回复。

> **注意**：`DataType::BandwidthTest`、`DataType::Data`、`DataType::Info` 由 Network_Service 内部处理，不到达 Core（见第 8 节）。

### 3.1 InboundRequest 结构

```rust
pub struct InboundRequest {
    pub request_id: u64,
    pub peer: PeerId,
    pub data_type: DataType,
    pub payload: Vec<u8>,
}
```

### 3.2 DataType::File — 文件元数据协商（阶段1接收侧）

**触发时机**：发送方 `handle_send_file()` 阶段1 调用 `send_data(peer, DataType::File, "file_id|size|checksum")`。

**处理流程**：
1. 解析 payload：`file_name|file_size|checksum`
2. 校验格式合法性（字段数量、file_size 可解析为 u64）
3. 存入 `pending_file_receives[peer].push(FileMetadata { file_name, file_size, checksum })`
4. 回复 `send_response(request_id, DataType::Command, b"ACCEPT")`
5. 校验失败 → 回复 `send_response(request_id, DataType::Command, b"REJECT|reason")`

### 3.3 DataType::Command — 命令分发

需解析 payload 文本，按命令前缀分发。当前需处理的命令：

#### 3.3.1 `REQUEST_PIPELINE|{coordinator_job_id}|{model_file_id}|{device}|{layer_start}|{layer_end}`

**触发时机**：远端 Coordinator Job 的 `handle_request_pipeline()` 发送。

**处理流程**：
1. 解析 payload 各字段
2. 直接调用 `route_pipeline_flow(...)` 执行编译和 spawn（原 `route_network(PipelineFlow)` 逻辑）
3. 收到 `Ok(relay_job_id)` → 回复 `send_response(request_id, DataType::Command, format!("OK|{}", relay_job_id))`
4. 收到 `Err(reason)` → 回复 `send_response(request_id, DataType::Command, format!("REJECT|{}", reason))`

**`route_pipeline_flow()` 内部逻辑**（从原 `route_network(PipelineFlow)` 迁移，✅ 已实现）：
- `compile_relay()` → 编译 Relay Job 指令序列
- `io_broker.Allocate()` + `Take_ML_Side()` → IO 通道分配
- `tensor_io_broker.Prepare()` → 预注册张量流路由
- `spawn_job()` → spawn Relay Job
- 返回 `Result<JobId, String>`

#### 3.3.2 `VERIFY_FILE|{file_name}`

**触发时机**：发送方 `handle_send_file()` 阶段3 发送。

**处理流程**：
1. 解析 file_name
2. 查 `StorageManager`：`storage.exists(&file_name)` 或 `storage.checksum(&file_name, ...)`
3. 文件存在且完整 → 回复 `send_response(request_id, DataType::Command, b"confirmed")`
4. 文件不存在或损坏 → 回复 `send_response(request_id, DataType::Command, b"failed|reason")`

> **注意**：ReceiveFile Job 的 handler 已在内部做了 checksum 校验（handler_network.rs:167-183）。VERIFY_FILE 是发送方的二次确认，可简化为只检查文件是否存在。

---

## 4. B3：Network_Inbound_Event 分支（handle_network_inbound）

这些是 **Stream 协议** 的入站事件，由 Network_Service 通过 `orchestrator_event_tx` 转发。

### 4.1 已实现

| 事件 | 流程 |
|------|------|
| `TensorStreamArrived { peer, stream }` | Read_Tensor_Stream_Handshake → 提取 target_job_id → tensor_io_broker.Store_Inbound(job_id, stream) |

### 4.2 未实现

| 事件 | 流程 | 依赖 |
|------|------|------|
| `FileStreamArrived { peer, stream }` | 从 `pending_file_receives` 查找匹配元数据 → compile_receive_file → 注入 stream+元数据到 SlotFile → spawn_job | 依赖 B2 中 `DataType::File` 入站提前存入 pending |

#### FileStreamArrived 详细设计

**前提**：B2 中 `DataType::File` 入站已将元数据存入 `pending_file_receives`。

**处理流程**：
1. 从 `pending_file_receives` 中按 peer 查找匹配的 `FileMetadata`（取出，一次性消费）
2. 未找到 → warn 并丢弃 stream
3. 找到 → 生成 job_id
4. 编译 ReceiveFile 作业（可内联编译或新增 `compile_receive_file`）：
   ```
   指令序列仅 1 条：
     ReceiveFile { stream: SLOT_STREAM, file_name: SLOT_FILE_NAME, file_size: SLOT_FILE_SIZE, checksum: SLOT_CHECKSUM, result: SLOT_RESULT }
   ```
5. 创建 SlotFile，注入初始值：
   - SLOT_STREAM ← SlotValue::Stream(Mutex::new(Some(stream)))
   - SLOT_FILE_NAME ← SlotValue::String(file_name)
   - SLOT_FILE_SIZE ← SlotValue::U64(file_size)
   - SLOT_CHECKSUM ← SlotValue::String(checksum)
6. Allocate IO → spawn_job（ReceiveFile Job 不使用 ML 推理，IO 仅为接口一致）

**Core 新增字段**：
```rust
pending_file_receives: HashMap<PeerId, Vec<FileMetadata>>,
```
其中 `FileMetadata`:
```rust
struct FileMetadata {
    file_name: String,
    file_size: u64,
    checksum: String,
    request_id: u64,  // 用于 VERIFY_FILE 阶段查找
}
```

---

## 5. B4：LifecycleEvent 分支（handle_lifecycle_event）

### 5.1 已实现

| 事件 | 流程 |
|------|------|
| `Done { job_id, result }` | 发布 Bus_Event::Job_Completed → tensor_io_broker.Deallocate(job_id) → registry.remove(job_id) |

> 注意：当前 Deallocate 使用 `tokio::spawn` 异步执行（因 handle_lifecycle_event 是同步方法）。

---

## 6. Core 新增/变更状态字段

```rust
pub struct Core {
    // --- 内核状态 ---
    registry: HashMap<JobId, JobHandle>,
    shutting_down: bool,

    // --- 路由工具 ---
    compiler: Arc<Compiler>,
    capabilities: Arc<Capabilities>,

    // --- B1: 用户命令 ---
    user_cmd_rx: mpsc::Receiver<UserCommand>,

    // --- B2: Request-Response 入站（新增） ---
    inbound_rx: mpsc::Receiver<InboundRequest>,

    // --- B3: Stream 入站 ---
    network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,

    // --- B4: 生命周期 ---
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    lifecycle_rx: mpsc::Receiver<LifecycleEvent>,

    // --- B2/B3 共享状态 ---
    pending_file_receives: HashMap<PeerId, Vec<FileMetadata>>,

    // --- SetDevice ---
    device_preference: String,
}

struct FileMetadata {
    file_name: String,
    file_size: u64,
    checksum: String,
}
```

> **移除的字段**：`network_cmd_rx: mpsc::Receiver<NetworkCommand>` — 原 B2 通道，已合并到新 B2。

---

## 7. Core::run() 最终 select! 结构

```rust
pub async fn run(mut self) {
    loop {
        if self.shutting_down && self.registry.is_empty() {
            break;
        }
        tokio::select! {
            // B1: 用户命令
            Some(cmd) = self.user_cmd_rx.recv(), if !self.shutting_down => {
                self.route_user(cmd).await;
            }
            // B2: Request-Response 入站请求（仅 Command + File）
            Some(req) = self.inbound_rx.recv(), if !self.shutting_down => {
                self.handle_inbound_request(req).await;
            }
            // B3: Network 转发的 Stream 入站事件（文件流 + 张量流）
            Some(event) = self.network_inbound_rx.recv(), if !self.shutting_down => {
                self.handle_network_inbound(event).await;
            }
            // B4: 生命周期事件（始终活跃）
            Some(event) = self.lifecycle_rx.recv() => {
                self.handle_lifecycle_event(event);
            }
        }
    }
}
```

---

## 8. Network_Service 入站请求预筛选 ✅ 已实现

### 8.1 设计原则

Network_Service 收到 Request-Response 入站请求后，按 `DataType` 分流：
- **需要业务决策的**（`Command`、`File`）→ 通过 `inbound_tx` 转发给 Core B2
- **纯网络层的**（`BandwidthTest`、`Data`、`Info`）→ Network 内部直接处理并回复，不上报 Core

### 8.2 Network 内部处理的 DataType

| DataType | 处理方式 | 说明 |
|----------|---------|------|
| `BandwidthTest` | 读取请求中的数据包大小（u64 小端） → 创建等大响应 → 直接回复 | 纯网络测量，50MB 上限防 OOM |
| `Data` | 回复 `DataType::Data` + `b"OK"` | 当前无用途，预留 |
| `Info` | 回复 `DataType::Info` + `b"OK"` | 当前无用途，预留 |

### 8.3 实现详情

**已实现文件**：`Src/Network/network_service.rs`

**改动 1**：`Handle_Request_Response_Event()` 的 `Message::Request` 分支新增 DataType 分流：

```rust
match request.data_type {
    DataType::BandwidthTest => {
        self.Handle_Bandwidth_Test_Inbound(request.payload, channel);
    }
    DataType::Data => {
        let response = Network_Data { data_type: DataType::Data, payload: b"OK".to_vec() };
        self.swarm.behaviour_mut().request_response.send_response(channel, response);
    }
    DataType::Info => {
        let response = Network_Data { data_type: DataType::Info, payload: b"OK".to_vec() };
        self.swarm.behaviour_mut().request_response.send_response(channel, response);
    }
    DataType::Command | DataType::File => {
        self.inbound_manager.Register_Inbound(peer, request, channel).await;
    }
}
```

**改动 2**：新增 `Handle_Bandwidth_Test_Inbound()` 方法（同步，50MB 上限）

**改动 3**：更新 `Inbound_Manager` 注释（"Control 层" → "Orchestrator Core"）

---

## 9. 实施依赖关系

```
B2 handle_inbound_request (DataType::File 元数据存 pending)
    ↓
B3 handle_network_inbound (FileStreamArrived 从 pending 取元数据)
    ↓
compile_receive_file + spawn ReceiveFile Job
```

```
B2 handle_inbound_request (REQUEST_PIPELINE → route_pipeline_flow)
    ↓
route_pipeline_flow (compile_relay → spawn Relay Job)
    ↓（逻辑已实现，需迁移为 B2 内部调用）
```

```
B1 route_user (DistributeRun → compile_coordinator → spawn Coordinator Job)
    ↓
需先添加 UserCommand::DistributeRun 变体
```

---

## 10. 与 Network_Service 的对接变更

Control 层移除后，`inbound_tx` 的接收端从 Control 转移到 Core：

```
// 旧：Network_Service → inbound_tx → Control::inbound_rx（所有 DataType）
// 新：Network_Service → inbound_tx → Core::inbound_rx（仅 Command + File）
```

**变更点**：
- `Core::new()` 新增 `inbound_rx` 参数
- main.rs 中创建 `(inbound_tx, inbound_rx)` 通道：`inbound_tx` 传给 Network_Service，`inbound_rx` 传给 Core
- 移除 Control 层对 `inbound_rx` 的持有
- **移除** `network_cmd_rx` / `network_cmd_tx` 通道和 `NetworkCommand` 枚举
- Network_Service 的入站请求处理逻辑已完成（见第 8 节）
