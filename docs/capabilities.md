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

**MlSession 实例方法**：

| 方法 | 返回 | 说明 |
|------|------|------|
| `sess:has_model()` | `boolean` | 是否已加载模型 |
| `sess:get_eos()` | `number` | 获取 EOS token ID |
| `sess:get_offset()` | `number` | 获取当前偏移量 |
| `sess:tensorize(token_ids)` | `{dim0, dim1}` | 将 token 序列转为张量维度的 table |
| `sess:load_model(path, start, end)` | `nil` | 加载模型层范围 |
| `sess:unload()` | `nil` | 卸载模型 |

```lua
local sess = ml.new("cuda")
sess:load_model("model.chunk", 0, 16)
local dims = sess:tensorize({1, 2, 3})
caps.print(string.format("shape: %d x %d", dims[1], dims[2]))
```

---

## Demo — 示例函数

| 函数 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `caps.echo(msg)` | `string` | `string` | 直接返回参数 |
| `caps.add(a, b)` | `number, number` | `number` | 两数求和 |
| `caps.ping()` | — | `"pong"` | 返回固定字符串 |
| `caps.table_sum(t)` | `table` | `number` | 读取 `t.a + t.b` 返回 |
