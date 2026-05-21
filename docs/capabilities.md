# Pleiades Lua 能力函数参考

本文档面向 Lua 脚本开发者，列出系统中所有可用的能力函数。

---

## 日志

### `caps.print(msg)`

输出日志到 TUI 日志面板和 tracing 日志文件。

| 参数 | 类型 | 说明 |
|------|------|------|
| `msg` | `string` | 日志内容 |

```lua
caps.print("正在发送文件...")
```

---

## Storage — 文件存储

### `caps.storage_list()`

列出所有已注册文件的元数据。

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 文件列表 | `table[]` | 每项包含 `file_name` (string) 和 `size` (number) |

```lua
local files = caps.storage_list()
for _, f in ipairs(files) do
    caps.print(f.file_name .. " (" .. f.size .. " bytes)")
end
```

### `caps.storage_exists(file_id)`

检查文件是否存在。

| 参数 | 类型 | 说明 |
|------|------|------|
| `file_id` | `string` | Storage 文件名 |

| 返回值 | 类型 |
|--------|------|
| 是否存在 | `boolean` |

### `caps.storage_acquire_read(file_id)`

获取文件读锁和磁盘路径。返回 `StorageReadHandle`，变量离开作用域后自动释放读锁。

| 参数 | 类型 | 说明 |
|------|------|------|
| `file_id` | `string` | Storage 文件名 |

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 句柄 | `StorageReadHandle` | 持有读锁，可调用 `:path()` 获取完整磁盘路径 |

```lua
local handle = caps.storage_acquire_read("model.chunk")
local path = handle:path()   -- "/workspace/model.chunk"
-- handle 离开作用域后读锁自动释放
```

### `caps.storage_acquire_write(file_id)`

获取文件写锁和磁盘路径。返回 `StorageWriteHandle`。

| 参数 | 类型 | 说明 |
|------|------|------|
| `file_id` | `string` | Storage 文件名 |

| 返回值 | 类型 |
|--------|------|
| 句柄 | `StorageWriteHandle` |

```lua
local handle = caps.storage_acquire_write("incoming.chunk")
local path = handle:path()
-- 在 path 处写入文件...
-- handle 离开作用域后写锁自动释放
```

### `caps.storage_remove(file_id)`

删除文件（幂等：文件不存在不报错）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `file_id` | `string` | Storage 文件名 |

### `caps.storage_checksum(file_id [, algo])`

计算文件校验码。

| 参数 | 类型 | 说明 |
|------|------|------|
| `file_id` | `string` | Storage 文件名 |
| `algo` | `string?` | `"blake3"` / `"sha256"` / `"xxhash64"`，默认 `"xxhash64"` |

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 校验码 | `string` | 格式 `"algo:hex"` |

### `caps.storage_flush()`

重新扫描工作目录，发现新文件并清理索引僵尸条目。

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 统计 | `{added, removed}` | `added`: 新发现文件数, `removed`: 清理僵尸数 |

---

## Network — 网络通信

### `caps.network.send_file(peer, file_path)`

向指定节点发送文件（一键完成：开流 → 写头 → 写数据）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `peer` | `string` | 目标节点 PeerId（Base58 编码） |
| `file_path` | `string` | 文件完整磁盘路径（通常从 `storage_acquire_read:path()` 获得） |

```lua
local handle = caps.storage_acquire_read("data.bin")
caps.network.send_file(peer_id, handle:path())
```

### `caps.network.send_data(peer, data_type, payload)`

发送数据并等待对方响应。

| 参数 | 类型 | 说明 |
|------|------|------|
| `peer` | `string` | 目标节点 PeerId |
| `data_type` | `string` | `"Command"` / `"Data"` / `"File"` / `"Info"` |
| `payload` | `string` | 载荷内容 |

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 响应 | `{payload}` | 包含 `payload` 字段 |

### `caps.network.get_local_peer_id()`

获取本地节点 PeerId。

| 返回值 | 类型 |
|--------|------|
| PeerId | `string` |

### `caps.network.dial(addr)`

主动连接到指定地址。

| 参数 | 类型 | 说明 |
|------|------|------|
| `addr` | `string` | Multiaddr 地址 |

### `caps.network.disconnect(peer)`

断开与指定节点的连接。

| 参数 | 类型 | 说明 |
|------|------|------|
| `peer` | `string` | 目标节点 PeerId |

### `caps.network.test_bandwidth(peer)`

测试与指定节点之间的带宽。

| 参数 | 类型 | 说明 |
|------|------|------|
| `peer` | `string` | 目标节点 PeerId |

| 返回值 | 类型 |
|--------|------|
| 带宽 (Mbps) | `number` |

### `caps.network.open_tensor_stream(peer, inference_id)`

向指定节点打开张量流连接（含握手）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `peer` | `string` | 目标节点 PeerId |
| `inference_id` | `number` | 本次推理的唯一标识 |

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 流对象 | `NetworkStream` | 可用于 `send_tensor` / `recv_tensor` / `send_eof` |

### `caps.network.accept_tensor_stream(inference_id, timeout)`

等待并接受对端发起的张量流（通过 rendezvous 匹配）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `inference_id` | `number` | 与发送方相同的推理标识 |
| `timeout` | `number` | 超时秒数 |

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 流对象 | `NetworkStream` | 可用于 `send_tensor` / `recv_tensor` / `send_eof` |

### `caps.network.send_tensor(stream, tensor, offset)`

通过张量流发送一个 Tensor（序列化后传输）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `stream` | `NetworkStream` | `open_tensor_stream` 或 `accept_tensor_stream` 返回的流 |
| `tensor` | `LuaTensor` | 要发送的张量（来自 `sess:tensorize()` 或 `sess:forward()`） |
| `offset` | `number` | 张量在序列中的偏移量 |

### `caps.network.recv_tensor(stream, device)`

从张量流接收一个 Tensor（反序列化重建）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `stream` | `NetworkStream` | 张量流对象 |
| `device` | `string` | 目标设备：`"cpu"` / `"cuda"` |

| 返回值 | 类型 | 说明 |
|--------|------|------|
| `tensor` | `LuaTensor` | 接收到的张量 |
| `offset` | `number` | 张量偏移量；若收到 EOF，抛出错误 |

### `caps.network.send_eof(stream)`

发送 EOF 帧，表示本端不再发送更多张量。

| 参数 | 类型 | 说明 |
|------|------|------|
| `stream` | `NetworkStream` | 张量流对象 |

```lua
-- 张量流完整示例
local stream = caps.network.open_tensor_stream(peer_id, 42)
local t = sess:tensorize({1, 2, 3})
caps.network.send_tensor(stream, t, 0)
caps.network.send_eof(stream)
```

---

## ML Engine — 推理引擎

### `ml.new(device)`

创建推理会话。

| 参数 | 类型 | 说明 |
|------|------|------|
| `device` | `string` | 设备名：`"cpu"` / `"cuda"` / `"metal"` |

| 返回值 | 类型 |
|--------|------|
| 会话对象 | `MlSession` |

### `ml.tensor_from_bytes(bytes, device)`

从字节数组反序列化为 Tensor（配合 `tensor:to_bytes()` 使用）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `bytes` | `string` | `LuaTensor:to_bytes()` 的输出 |
| `device` | `string` | 目标设备：`"cpu"` / `"cuda"` |

| 返回值 | 类型 |
|--------|------|
| 张量对象 | `LuaTensor` |

### `ml.analyze_model(path)`

解析 GGUF/PGGUF 文件，返回模型架构元信息。

| 参数 | 类型 | 说明 |
|------|------|------|
| `path` | `string` | 模型文件路径 |

| 返回值 | 类型 | 说明 |
|--------|------|------|
| 架构信息 | `table` | `{architecture, num_layers, embedding_length, head_count, head_count_kv, head_dim, feed_forward_length, context_length, rms_norm_eps, rope_freq_base, vocab_size, eos_token_id, is_split, split_start, split_end}` |

### `ml.split_model(path, start, end, output_dir)`

切分模型指定层范围，输出到目标目录。

| 参数 | 类型 | 说明 |
|------|------|------|
| `path` | `string` | 源模型文件路径 |
| `start` | `number` | 起始层索引（含） |
| `end` | `number` | 结束层索引（含） |
| `output_dir` | `string` | 输出目录 |

---

### LuaTensor 方法

`MlSession:tensorize()` 和 `MlSession:forward()` 返回 `LuaTensor` 对象：

| 方法 | 返回 | 说明 |
|------|------|------|
| `tensor:dims()` | `table` `{dim1, dim2, ...}` | 获取张量各维度大小（1-indexed） |
| `tensor:to_bytes()` | `string` | 序列化为字节数组（含 shape header） |
| `tensor:to_device(device)` | `LuaTensor` | 将张量移动到指定设备（`"cpu"` / `"cuda"`），直接设备间拷贝 |

```lua
-- CPU/GPU 混合推理示例
local hidden = cpu:forward(t, offset)          -- hidden 在 CPU 上
local hidden_gpu = hidden:to_device("cuda")     -- 搬到 GPU
local logits = gpu:forward(hidden_gpu, offset)  -- GPU 推理
local logits_cpu = logits:to_device("cpu")      -- 搬回 CPU
local tok = cpu:sample(logits_cpu, temperature)
```

---

### MlSession 实例方法

| 方法 | 返回 | 说明 |
|------|------|------|
| **生命周期** | | |
| `sess:load_model(path, start, end)` | `nil` | 加载模型层范围（先卸载已有） |
| `sess:unload()` | `nil` | 卸载模型，回到空壳 |
| `sess:has_model()` | `boolean` | 是否已加载模型 |
| **编解码** | | |
| `sess:encode(text)` | `{token_id, ...}` | 文本 → token 序列 |
| `sess:decode(token_id)` | `string` | token → 文本 |
| **推理** | | |
| `sess:tensorize(token_ids)` | `LuaTensor` | token 序列 → Tensor[1, seq_len] |
| `sess:forward(tensor [, offset])` | `LuaTensor` | 前向推理，返回 logits |
| **采样** | | |
| `sess:sample(logits, temperature)` | `number` | 从 logits 采样下一个 token |
| **状态查询** | | |
| `sess:get_eos()` | `number` | 获取 EOS token ID |
| `sess:get_offset()` | `number` | 获取当前序列偏移量 |
| **随机种子** | | |
| `sess:set_seed(seed)` | `nil` | 设置采样 PRNG 种子（可复现结果） |

```lua
local sess = ml.new("cpu")
sess:load_model("model.chunk", 0, 16)
local tokens = sess:encode("Hello")
local t = sess:tensorize(tokens)
local logits = sess:forward(t, 0)
local next_token = sess:sample(logits, 0.8)
caps.print(sess:decode(next_token))
sess:unload()
```

---

## LocalTensorStream — 本地线程间张量流

通过 `local_tensor` 表访问，用于同进程内两个 Lua 线程间的张量传输。

### 配对机制

基于 `LocalStreamHub` 的 `open/accept` 模式：一端调用 `open_stream(id)` 创建流对并拿到左半，另一端调用 `accept_stream(id, timeout_ms)` 阻塞等待右半出现。

### API

| 函数 | 返回 | 说明 |
|------|------|------|
| `local_tensor.open_stream(id)` | `LocalTensorStream` | 创建流对，返回左半（发起端） |
| `local_tensor.accept_stream(id, timeout_ms)` | `LocalTensorStream` | 阻塞等待右半，超时抛出错误（接收端） |
| `local_tensor.send_tensor(stream, tensor, offset)` | `nil` | 发送 `LuaTensor`（自动序列化传输） |
| `local_tensor.recv_tensor(stream, device)` | `LuaTensor, number` | 接收张量，返回 (tensor, offset) |
| `local_tensor.send_eof(stream)` | `nil` | 发送 EOF 哨兵，对端 `recv_tensor` 抛出错误 |

### 使用示例

```lua
-- 线程 A (协调端)
local fwd = local_tensor.open_stream("fwd")
local bwd = local_tensor.accept_stream("bwd", 60000)

local_tensor.send_tensor(fwd, hidden, 0)

local logits, _ = local_tensor.recv_tensor(bwd, "cpu")
local tok = sess:sample(logits, 0.8)
```

```lua
-- 线程 B (工作端)
local fwd = local_tensor.accept_stream("fwd", 120000)
local bwd = local_tensor.open_stream("bwd")

local hidden, recv_offset = local_tensor.recv_tensor(fwd, "cpu")
local logits = sess:forward(hidden, recv_offset)

local_tensor.send_tensor(bwd, logits, sess:get_offset())
```

> **接口已与 `caps.network` 对齐**：`send_tensor` 直接接受 `LuaTensor`（内部自动 `tensor_to_bytes`），`recv_tensor` 直接返回 `LuaTensor`（内部自动 `bytes_to_tensor`），无需手动序列化。

---

## Demo — 示例函数

| 函数 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `caps.echo(msg)` | `string` | `string` | 直接返回参数 |
| `caps.add(a, b)` | `number, number` | `number` | 两数求和 |
| `caps.ping()` | — | `"pong"` | 返回固定字符串 |
| `caps.table_sum(t)` | `table` | `number` | 读取 `t.a + t.b` 返回 |
