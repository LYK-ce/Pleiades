# 独立本地流 (Local Tensor Stream) 设计文档

> Presented by KeJi
> Date: 2026-05-20

## 目标

为同进程内的两个线程（Prompt 线程 + ML 推理线程）提供轻量张量流通信，
与网络 Tensor Stream 共享同一套帧格式，但完全绕过 libp2p。

## 架构

```
Prompt线程 (std::thread)                     ML推理线程 (std::thread)
  │                                              │
  │  lua: local.open_stream("infer-1")           │  lua: local.accept_stream("infer-1")
  │         ↓                                     │         ↓
  │  LocalStreamHub::open(1) → DuplexLeft        │  LocalStreamHub::accept(1) → DuplexRight
  │         ↓                                     │         ↓
  │  LocalTensorStream { stream: DuplexLeft }     │  LocalTensorStream { stream: DuplexRight }
  │         ↓                                     │         ↓
  │  local.send_tensor(s, tensor, 0)             │  local.recv_tensor(s, "cpu")
  │  local.recv_tensor(s, "cpu")                │  local.send_tensor(s, tensor, 1)
  │  local.send_eof(s)                           │  (收到 EOF)
  │                                              │
  └──────── tokio::io::duplex() ─────────────────┘
           64KB buffer, 零网络开销
```

与 Tensor Stream 对比：

| | Network Tensor Stream | Local Tensor Stream |
|---|---|---|
| 传输介质 | `libp2p::Stream` | `tokio::io::DuplexStream` |
| 配对机制 | RendezvousMap (swarm 事件驱动) | LocalStreamHub (内存 HashMap) |
| 帧格式 | `[8B offset][8B len][data]` | 完全相同 |
| 跨进程 | ✅ | ❌ |
| 零拷贝可能 | ❌ (网络拷贝) | ✅ (内存共享，未来) |

## 文件结构

```
Src/Orchestrator/local_tensor_stream/
  ├── mod.rs          # 模块入口，LocalStreamHub
  ├── frames.rs       # 本地帧读写函数（泛型 S: AsyncRead+AsyncWrite+Unpin）
  └── lua_binding.rs  # Lua userdata: LocalTensorStream, 注册 local 表
```

调用链：

```
Lua: local.open_stream(id)  →  lua_binding::register_local_stream_caps()
                                 →  LocalStreamHub::open(id)
                                    →  tokio::io::duplex()
                                       →  LocalTensorStream { DuplexLeft }
```

## 核心组件

### 1. LocalStreamHub（`mod.rs`）

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::io::{duplex, DuplexStream};

pub struct LocalStreamHub {
    /// 等待 accept 的流右半部分，按 stream_id 索引
    pending: Mutex<HashMap<String, DuplexStream>>,
}

impl LocalStreamHub {
    /// 发起方调用：创建一对 duplex，左半返回，右半存入 pending
    pub fn open(&self, id: &str) -> io::Result<DuplexStream> {
        let (local, remote) = duplex(64 * 1024);
        self.pending.lock().unwrap().insert(id.to_string(), remote);
        Ok(local)
    }

    /// 接收方调用：从 pending 取出右半（阻塞等待，可加超时）
    pub fn accept(&self, id: &str, timeout_ms: u64) -> io::Result<DuplexStream> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if let Some(remote) = self.pending.lock().unwrap().remove(id) {
                return Ok(remote);
            }
            if std::time::Instant::now() > deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "accept timeout"));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
```

> `accept` 在 OS 线程中调用，不能用 `.await`，用 spin-sleep。Hub 实例放在 `Arc<Capabilities>` 中共享。

### 2. 本地帧读写（`frames.rs`）

完全复用 Tensor Stream 的帧格式，但用泛型签名：

```rust
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 本地版 Send_Tensor_Frame
pub async fn local_send_frame<S: AsyncWriteExt + Unpin>(
    stream: &mut S, offset: u64, data: &[u8],
) -> io::Result<()> {
    stream.write_all(&offset.to_le_bytes()).await?;
    stream.write_all(&(data.len() as u64).to_le_bytes()).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

/// 本地版 Receive_Tensor_Frame
pub async fn local_recv_frame<S: AsyncReadExt + Unpin>(
    stream: &mut S, buffer: &mut Tensor_Buffer,
) -> io::Result<u64> {
    // 完全相同的帧解析逻辑
    let mut header = [0u8; 16];
    stream.read_exact(&mut header).await?;
    let offset = u64::from_le_bytes(header[0..8].try_into().unwrap());
    let length = u64::from_le_bytes(header[8..16].try_into().unwrap());
    if offset == TENSOR_EOF_OFFSET && length == 0 {
        return Ok(TENSOR_EOF_OFFSET);
    }
    let slice = buffer.As_Mut_Slice(length as usize);
    stream.read_exact(slice).await?;
    Ok(offset)
}

/// 本地版 Send_EOF
pub async fn local_send_eof<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
) -> io::Result<()> {
    stream.write_all(&TENSOR_EOF_OFFSET.to_le_bytes()).await?;
    stream.write_all(&0u64.to_le_bytes()).await?;
    stream.flush().await?;
    Ok(())
}
```

复用的常量：`TENSOR_EOF_OFFSET`，`MAX_TENSOR_SIZE`（从 `protocol.rs` 导入）。

### 3. Lua userdata（`lua_binding.rs`）

```rust
use mlua::{UserData, UserDataMethods};
use tokio::io::DuplexStream;
use std::sync::Mutex;

/// Lua 可见的本地流句柄
pub struct LocalTensorStream {
    pub stream: Mutex<DuplexStream>,
}

impl UserData for LocalTensorStream {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // 可加不需要外部能力的纯 stream 操作
    }
}
```

### 4. Lua 能力注册（在 `capability_binding.rs` 中新增）

```rust
use crate::orchestrator::local_tensor_stream::{
    LocalStreamHub, LocalTensorStream, local_send_frame, local_recv_frame, local_send_eof,
};

pub fn register_local_stream_caps(
    lua: &Lua,
    hub: Arc<LocalStreamHub>,
) -> mlua::Result<()> {
    let local = lua.create_table()?;

    // ─── open_stream ──────────────────────────────
    let h = hub.clone();
    local.set("open_stream", lua.create_function(move |_, id: String| {
        let stream = h.open(&id)
            .map_err(|e| mlua::Error::runtime(format!("open_stream: {}", e)))?;
        Ok(LocalTensorStream { stream: Mutex::new(stream) })
    })?)?;

    // ─── accept_stream ────────────────────────────
    let h = hub.clone();
    local.set("accept_stream", lua.create_function(move |_, (id, timeout): (String, u64)| {
        let stream = h.accept(&id, timeout)
            .map_err(|e| mlua::Error::runtime(format!("accept_stream: {}", e)))?;
        Ok(LocalTensorStream { stream: Mutex::new(stream) })
    })?)?;

    // ─── send_tensor ──────────────────────────────
    // (在 tokio::runtime 内调用 async 帧函数)
    // ─── recv_tensor ──────────────────────────────
    // ─── send_eof ─────────────────────────────────

    lua.globals().set("local", local)?;
    Ok(())
}
```

> 注意：`send_tensor`/`recv_tensor` 需要 tokio runtime 执行 async 帧函数。
> spawn 的线程内部已有 tokio runtime（`block_on`），直接在闭包内 `rt.block_on(...)` 即可。

## Lua 使用示例

```lua
-- === Prompt 线程 ===
function execute(params)
    local s = local.open_stream("session-1")

    -- 发送 prompt
    local t = ml.tensor_from_bytes(prompt_bytes, "cpu")
    local.send_tensor(s, t, 0)

    -- 接收生成结果
    while true do
        local tensor, offset = local.recv_tensor(s, "cpu")
        if offset == 0xFFFFFFFFFFFFFFFF then break end
        -- 解码 token，输出给用户
    end

    local.send_eof(s)
end

-- === ML 推理线程 ===
function execute(params)
    local s = local.accept_stream("session-1", 30000)

    -- 接收 prompt
    local tensor, offset = local.recv_tensor(s, "cpu")

    -- 推理循环
    for i = 1, 120 do
        local hidden = sess:forward(tensor, i)
        local.send_tensor(s, hidden, i)
    end

    local.send_eof(s)
end
```

## 实施计划

| 步骤 | 内容 | 预估行数 |
|------|------|---------|
| 1 | 创建 `Src/Orchestrator/local_tensor_stream/` 目录结构 | — |
| 2 | `mod.rs` — LocalStreamHub（open/accept 配对） | ~50 |
| 3 | `frames.rs` — 本地帧读写泛型函数 | ~80 |
| 4 | `lua_binding.rs` — LocalTensorStream userdata | ~50 |
| 5 | 在 `capability_binding.rs` 新增 `register_local_stream_caps()` | ~80 |
| 6 | 在 `Core::new()` 中创建 `Arc<LocalStreamHub>`，传入 Lua 注册 | ~10 |
| 7 | `cargo check` + 单测 | — |

## 不涉及

- `protocol.rs` 不动
- `NetworkStream` 不动
- `network.*` Lua API 不动
- libp2p 层
