# Pleiades Capabilities 内部实现细节

本文档面向开发者，描述各能力函数的内部实现流程。

---

## 文件发送 (`caps.network.send_file`)

### 总体流程

```
Lua: caps.network.send_file(peer, file_path)
 → Rust: Network_Capability::send_file(peer, &path)
   → 1. open_file_stream(peer)           # 与对端建立 libp2p 文件流连接
   → 2. Write_File_Stream_Header(stream, file_name, file_size, "")
   → 3. send_file_data(stream, path)     # 分块读文件并写入流
```

### 步骤详解

**Step 1: `open_file_stream(peer)`**

```
Network_Service_Capability::open_file_stream()
  → file_stream_control.open_stream(peer, "/pleiades/file-stream/1.0.0")
  → 返回 libp2p::Stream
```

底层使用 `libp2p_stream::Control`，不经过 Network_Service 事件循环。

**Step 2: `Write_File_Stream_Header(stream, file_name, file_size, checksum)`**

在流中写入带内 (in-band) header，格式:

```
[2B name_len BE] [name UTF-8] [8B file_size LE] [2B checksum_len BE] [checksum UTF-8]
```

当前 `checksum` 传空字符串。接收方用 `Read_File_Stream_Header` 读取并解析。

**Step 3: `send_file_data(stream, path)`**

```
Send_File_Data(stream, file_path)
  → tokio::fs::File::open(path)
  → 循环: read(CHUNK_SIZE) → stream.write_all(chunk)
  → stream.flush()
```

纯 raw data，无内嵌 header。CHUNK_SIZE = 64KB。

### 注册链

```
execute_lua_script()
  → register_network_caps(&lua, caps)
    → caps.network.send_file → create_async_function → block_on(send_file)
```

---

## 网络请求-响应 (`caps.network.send_data`)

```
Lua: caps.network.send_data(peer, data_type, payload)
  → Rust: Network_Capability::send_data(peer, dt, bytes)
    → NodeHandle.Send_Data(peer, DataType, payload)
      → 命令通道 → Network_Service 事件循环
        → libp2p request_response::send_request
        → 等待对端 send_response
        → 返回 Network_Data { data_type, payload }
```

### DataType 枚举

| Lua 字符串 | Rust DataType |
|------------|---------------|
| `"Command"` | `DataType::Command` |
| `"Data"` | `DataType::Data` |
| `"File"` | `DataType::File` |
| `"Info"` | `DataType::Info` |

### 返回值

```lua
local resp = caps.network.send_data(peer, "Command", "hello")
-- resp.payload = "..."
```

---

## 文件流接收 (Rust 实现)

接收端不走 Lua，在 Core 的 `route_stream` 中直接处理。

```
Network_Service Event Loop:
  → accept 入站 /pleiades/file-stream/1.0.0
  → Network_Inbound_Event::FileStreamArrived { peer, stream }
  → orchestrator_event_tx.send(event)

Core::route_stream():
  → 1. Read_File_Stream_Header(&mut stream)
       → 解析 (file_name, file_size, checksum)
  → 2. Storage::acquire_write(file_name)
       → 获取 WriteGuard + 目标路径
  → 3. Network_Capability::receive_file_data(&mut stream, dest_path, file_size)
       → Receive_File_Data(stream, path, size)
         → 循环: stream.read(chunk) → file.write(chunk)
  → 4. EventBus::Publish(Log) → TUI 通知
```

WriteGuard 离开作用域后自动释放，文件注册到 Storage 索引。

---

## Storage 读写锁

### StorageReadHandle

```
caps.storage_acquire_read(file_id)
  → StorageManager::acquire_read(file_id)
    → 1. RwLock 读锁 + 1
    → 2. 返回 (PathBuf, ReadGuard)
    → 3. 包装为 StorageReadHandle { path, _guard: ReadGuard }
       → handle:path() → path.to_string_lossy()
```

**设计**: Lua 变量释放 → mlua GC 调用 `Drop` → ReadGuard 释放 → RwLock 读锁 - 1。

**使用者**: 发送文件脚本持有读锁期间，文件不会被 `remove()` 或 `acquire_write()` 修改。

### StorageWriteHandle

同 `StorageReadHandle`，使用 `WriteGuard` → RwLock 写锁。

---

## 张量流（待实现）

```
发送方:
  caps.network.open_tensor_stream(peer, inference_id)
    → Network_Service_Capability::open_tensor_stream()
      → tensor_stream_control.open_stream(peer, "/pleiades/tensor-stream/1.0.0")
      → Write_Tensor_Stream_Handshake(stream, inference_id)
      → 返回 libp2p::Stream（包装为 NetworkStream UserData）

接收方:
  caps.network.accept_tensor_stream(inference_id, timeout_secs)
    → Network_Service_Capability::accept_tensor_stream()
      → RendezvousMap::register_accept(inference_id)
      → 等待 Network_Service 转发入站流
      → 返回 libp2p::Stream（包装为 NetworkStream UserData）

NetworkStream UserData:
  stream:read(n_bytes)  → Vec<u8>   (待实现)
  stream:write(data)    → ()         (待实现)
```

Note: `NetworkStream` 结构体已定义 (`Src/Lua/network_stream.rs`)，当前无 UserData 方法。

---

## ML Engine 会话

### MlSession UserData

```
ml.new(device)
  → ML_Engine::MlSession::new(device)
    → 创建 Candida 后端 + 分配 KV Cache 虚拟地址
    → 不加载模型（延迟加载）
    → 注册 UserData 方法
```

**模型加载**:
```
sess:load_model(path, start_layer, end_layer)
  → MlSession::load_model(path, start, end)
    → 实际加载 GGUF 文件指定的层范围到内存
```

**推理**:
```
sess:tensorize({1, 2, 3, 4})
  → 将 token 序列转为张量
  → 返回 {dim0=batch, dim1=seq_len}
```

注意: 空输入 `tensorize({})` 会返回错误。

---

## 日志系统 (`caps.print`)

```
caps.print(msg)
  → tracing::info!(target: "lua", "{}", msg)   # → 写入日志文件
  → EventBus::Publish(Bus_Event::Log { message: msg })  # → TUI 日志面板
```

使用 `create_function`（同步），不阻塞 tokio worker。

---

## 安全设计

- **沙箱隔离**: 每个 `execute()` 调用创建独立 `LuaContext`，禁用 `os`/`io`/`require`/`dofile`/`loadfile`
- **异步不阻塞**: 所有 I/O 使用 `create_async_function`，在 tokio runtime 上自然调度
- **锁自动释放**: `StorageReadHandle`/`StorageWriteHandle` 通过 `Drop` 保证锁释放
- **错误传播**: 能力函数通过 `mlua::Error` 传播错误，Lua 层可捕获处理
