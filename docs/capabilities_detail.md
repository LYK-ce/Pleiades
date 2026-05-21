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

## 张量流

### 打开/接受流

```
发送方:
  caps.network.open_tensor_stream(peer, inference_id)
    → Network_Capability::open_tensor_stream(peer, inference_id)
      → tensor_stream_control.open_stream(peer, "/pleiades/tensor-stream/1.0.0")
      → Write_Tensor_Stream_Handshake(stream, inference_id)  # 写入 8B LE inference_id
      → 返回 NetworkStream { stream: Mutex<libp2p::Stream>, caps }

接收方:
  caps.network.accept_tensor_stream(inference_id, timeout_secs)
    → Network_Capability::accept_tensor_stream(inference_id, timeout)
      → RendezvousMap::register_accept(inference_id)  # 注册期望
      → 等待 Network_Service 转发入站流 (Network_Inbound_Event::TensorStreamArrived)
      → RendezvousMap 匹配 → 返回 NetworkStream
```

### 发送张量

```
caps.network.send_tensor(stream_ud, tensor_ud, offset)
  → 取出 LuaTensor → tensor_to_bytes() 序列化
  → 取出 NetworkStream → Mutex lock stream
  → Send_Tensor_Frame(&mut stream, offset, &bytes)
    → 写入帧: [帧头] [数据]
      Header: [8B offset LE] [8B data_len LE] [8B checksum LE]
      Data: raw bytes
```

### 接收张量

```
caps.network.recv_tensor(stream_ud, device_str)
  → 取出 NetworkStream → Mutex lock stream
  → 创建 Tensor_Buffer(16 MiB)
  → Receive_Tensor_Frame(&mut stream, &mut buffer)
    → 读取帧头 → 读取数据 → 校验 checksum
    → 返回 offset
  → 若 offset == TENSOR_EOF_OFFSET → 抛出 "received EOF" 错误
  → bytes_to_tensor(buffer.as_slice(), &device) → LuaTensor
  → 返回 (LuaTensor, offset)
```

### 发送 EOF

```
caps.network.send_eof(stream_ud)
  → 取出 NetworkStream → Mutex lock stream
  → Send_EOF(&mut stream)
    → 写入 8B LE TENSOR_EOF_OFFSET 作为帧头
```

### NetworkStream UserData

定义于 `Src/VM/network_stream.rs`，包装 `Mutex<libp2p::Stream>` + `Arc<Capabilities>`：

```rust
pub struct NetworkStream {
    pub stream: Mutex<libp2p::Stream>,
    caps: Arc<Capabilities>,
}
```

- `Mutex` 使异步函数可通过共享引用获得可变访问，无需 `&mut self`
- 实现 `mlua::UserData`，无实例方法 — 所有操作在 `caps.network.*` 全局函数中完成

---

## ML Engine 会话

### MlSession UserData

```
ml.new(device)
  → MlSession::new(device)
    → 创建空壳 MlContext { model=None, offset=0, rng_state=299792458, eos=151645, device }
    → 不加载模型（延迟加载）
    → 注册 UserData 方法
```

**模型加载**:
```
sess:load_model(path, start_layer, end_layer)
  → 若已加载，先 unload()
  → GGUF_Load_Model(start, end, path, &device) → GGUF_Model
  → 更新 eos_token_id = model.inference_config.eos_token
```

**推理完整流程**:
```
-- 文本 → token
local tokens = sess:encode("Hello")
  → MlSession::encode(text) → GGUF_Encode → Vec<u32>

-- token → Tensor[1, seq_len]
local t = sess:tensorize(tokens)
  → MlSession::tensorize(&[u32]) → Tensor::new → unsqueeze(0) → LuaTensor

-- 前向推理
local logits = sess:forward(t, 0)
  → MlSession::forward(&tensor, offset)
    → GGUF_Model_Inference(&mut model, &tensor, offset)
    → device.synchronize()
    → offset += seq_len (若未显式指定 offset)
    → 返回 LuaTensor(logits)   # 形状: [1, seq_len, vocab_size]

-- 采样
local next = sess:sample(logits, 0.8)
  → MlSession::sample(&logits, temperature)
    → extract_last_logits(logits)  # 取最后一个位置
    → 若 temperature <= 0: argmax（贪婪）
    → 若 temperature > 0: softmax → multinomial 采样 (xoshiro RNG)
    → 返回 u32 token_id
```

**空壳状态**: `load_model()` 之前，`tensorize()` 可用（纯数据转换，仅需 device），`encode()`/`decode()`/`forward()`/`sample()` 返回 "no model loaded" 错误。

---

## Tensor 序列化

### LuaTensor UserData

定义于 `Src/ML_Engine/lua_tensor.rs`，包装 `candle_core::Tensor`：

```rust
pub struct LuaTensor(pub Tensor);
```

- 实现 `Deref<Target=Tensor>` — Rust 侧零开销解引用
- `UserData` 方法：`dims() → table`, `to_bytes() → string`

### 序列化格式 (`tensor_to_bytes` / `bytes_to_tensor`)

```
+--------+-------+-----+-------+--------+
| ndim   | d0    | ... | dn    | data   |
| u64 LE | u64LE |     | u64LE | f32 LE |
+--------+-------+-----+-------+--------+

Header: 8 + ndim × 8 字节
Data:   (d0 × d1 × ... × dn) × 4 字节
```

- 先 flatten 为 1D f32 数组，再序列化
- 可在 CPU/CUDA 间透明传输（candle 处理 device transfer）
- `ml.tensor_from_bytes(data, device)` 在指定设备上重建 Tensor

---

## ML 分析/切分（独立函数）

### `ml.analyze_model(path)`

```
ml.analyze_model(path)
  → capability::analyze_model(Path)
    → 打开 GGUF/PGGUF 文件
    → 读取元数据头
    → 返回 Model_Arch_Info {
        architecture, num_layers, embedding_length, head_count,
        head_count_kv, head_dim, feed_forward_length, context_length,
        rms_norm_eps, rope_freq_base, vocab_size, eos_token_id,
        is_split, split_start, split_end
      }
```

不依赖 MlSession，可在加载模型前调用。用于判断模型是否已切分、层数范围等。

### `ml.split_model(path, start, end, output_dir)`

```
ml.split_model(path, start, end, output_dir)
  → capability::split_model(Path, start, end, out_dir)
    → 读取 GGUF 头
    → 提取指定层范围的 tensor
    → 写入 .pgguf 文件到 output_dir
```

输出为 PGGUF (Pleiades GGUF) 格式，可直接被 `MlSession:load_model()` 加载。

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
