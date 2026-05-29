# Pleiades 系统架构文档

> **用途**: 指导 Agent 编写代码时正确使用已有组件，避免重复造轮子或绕过系统基础设施。
>
> **最后更新**: 2026-05-28

---

## 目录

1. [项目结构](#1-项目结构)
2. [启动流程](#2-启动流程)
3. [核心模块](#3-核心模块)
   - [Config](#31-config)
   - [EventBus](#32-eventbus)
   - [Storage](#33-storage)
   - [ML_Engine](#34-ml_engine)
   - [Network](#35-network)
   - [Orchestrator](#36-orchestrator)
   - [VM (Lua 引擎)](#37-vm-lua-引擎)
   - [TUI](#38-tui)
4. [关键工作流](#4-关键工作流)
5. [⚠️ Agent 常见错误](#5️-agent-常见错误)

---

## 1. 项目结构

```
/workspace/
├── .config/                    ← 运行时配置 (keypair.bin + config.toml)
├── Architecture/               ← 架构设计文档
├── Src/                        ← 全部 Rust 源代码
│   ├── main.rs                 ← 6 阶段 bootstrap
│   ├── Config/                 ← 配置读写
│   ├── EventBus/               ← 广播事件总线
│   ├── ML_Engine/              ← ML 推理引擎 (GGUF/PGGUF)
│   ├── Network/                ← P2P 网络 (libp2p)
│   ├── Orchestrator/           ← 任务编排核心
│   ├── PeerManagement/         ← 节点发现与管理
│   ├── Session_Manager/        ← 推理会话槽位
│   ├── Storage/                ← 统一存储管理
│   ├── TUI/                    ← 终端 UI (ratatui)
│   └── VM/                     ← Lua 脚本引擎 + 所有绑定
├── programs/                   ← Lua 脚本
│   ├── builtin/                ← 内置 (优先级低于 user)
│   │   └── legacy/             ← 过期脚本 (API 不兼容)
│   └── user/                   ← 用户脚本
└── tests/                      ← 集成测试
```

---

## 2. 启动流程

| 阶段 | 操作 |
|------|------|
| Phase 1 | `Ensure_Config()` + `Ensure_Identity()` → Ed25519 keypair |
| Phase 2 | 初始化 tracing 日志 → `Log/pleiades.log` |
| Phase 3 | 创建 EventBus, PeerManager, StorageManager |
| Phase 4 | 构建 libp2p Swarm (TCP + Noise + Yamux + mDNS + Kademlia + Stream) |
| Phase 5 | 组装 `Capabilities` → 创建 `Core` |
| Phase 5.5 | 后台 `storage.flush()` + 同步 models 到 PeerManager |
| Phase 6 | spawn Network 事件循环 + TUI + `core.run()` 主循环 |

---

## 3. 核心模块

### 3.1 Config

**位置**: `Src/Config/`

```rust
pub fn Ensure_Config() -> (Pleiades_Config, PathBuf);   // 自动创建默认配置
pub fn Read_Config(path: &Path) -> Pleiades_Config;      // 解析 TOML
pub fn Ensure_Identity(dir: &Path) -> Keypair;           // Ed25519 密钥
```

配置文件 `.config/config.toml` 结构：
```toml
[Log]       level = "info"
[Network]   LAN = true, WAN = false
[Runtime]   device = "cpu", model_path = "", max_token = 128, temperature = 0.8
[Storage]   workspace_dir = "Pleiades_Workspace"
[Session]   max_slots = 4
[Identity]  peer_name = "new_peer"

> ⚠️ `config.toml` 中存在 `[Scheduler]` 段（strategy = "uniform"），但 `Pleiades_Config` 中尚未实现对应的 `Scheduler_Config` 结构体，该段在反序列化时被静默忽略。
```

---

### 3.2 EventBus

**位置**: `Src/EventBus/`

基于 `tokio::sync::broadcast` 的全局事件总线。4 种通用事件类型，payload 为 JSON 字符串：

| 变体 | 语义 | 典型 `type` 值 |
|------|------|----------------|
| `Notify { level, message }` | 一次性通知 | 日志、错误 |
| `State { payload }` | 持久状态变更 | `peer_discovered`, `job_phase_changed`, `inference_completed` |
| `Stream { payload }` | 高频流式推送 | `token` (逐 token), `file_progress` |
| `Output { payload }` | 命令输出 | `cmd_result`, `help` |

---

### 3.3 Storage ⚠️

**位置**: `Src/Storage/`

**这是文件访问的唯一入口！严禁绕过 Storage 直接使用 `std::fs` 或 `tokio::fs`。**

```rust
#[async_trait]
pub trait StorageCapability {
    async fn acquire_read(&self, file_id: &str)  -> Result<(PathBuf, ReadGuard), StorageError>;
    async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError>;
    async fn remove(&self, file_id: &str)         -> Result<(), StorageError>;
    async fn exists(&self, file_id: &str)         -> Result<bool, StorageError>;
    async fn list(&self)                          -> Result<Vec<FileEntry>, StorageError>;
    async fn checksum(&self, file_id: &str, algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError>;
    async fn flush(&self)                         -> Result<(usize, usize), StorageError>;
}
```

**使用模式**:
1. `acquire_read("model.pgguf")` → 返回 `(PathBuf, ReadGuard)`
2. 用返回的 `PathBuf` 读取文件
3. Drop `ReadGuard` / `WriteGuard` → **自动释放锁**

**安全约束**:
- `file_id` 不能为空、不能含 `/`、`\`、`..`，不能以 `.` 开头
- `ReadGuard` 共享读锁（多并发）；`WriteGuard` 排他写锁
- `acquire_read` 支持惰性发现：磁盘文件不在索引 → 自动注册

**FileEntry 结构**:
```rust
pub struct FileEntry {
    pub file_name: String,
    pub model_id: Option<u32>,           // xxhash32(pgguf 内容)
    pub size: u64,
    pub num_layers: Option<u32>,
    pub layer_bitmap: Option<[u8; 32]>,  // 256-bit, bit N=1 表示持有第 N 层
    pub architecture: Option<String>,
}
```

---

### 3.4 ML_Engine

**位置**: `Src/ML_Engine/`

#### 3.4.1 GGUF → PGGUF 转换 ⚠️

**PGGUF 不是 split model 的产物！** PGGUF = 原始 GGUF + `pleiades.model_id` + `pleiades.layer_bitmap` 元数据。**完整保留了所有模型权重、tokenizer、架构信息。**

流程 (`GGUF_Analyze_And_Convert`):
1. 读取 `.gguf` 文件
2. 检查是否已有 `pleiades.model_id` → 已是 PGGUF，直接返回
3. 计算 `xxhash32` → `model_id`
4. 构建 256-bit `layer_bitmap`
5. 写入新 `.pgguf` 文件（追加元数据）
6. **删除原始 `.gguf`**，返回 `.pgguf` 路径

#### 3.4.2 层编号规则 ⚠️

| 层索引 | 内容 |
|--------|------|
| **0** | 输入层 (embedding, `token_embd.weight`) |
| **1 ..= N** | Transformer block (`blk.0` ~ `blk.(N-1)`) |
| **N+1** | 输出层 (`output_norm.weight` + `output.weight`) |

#### 3.4.3 核心 API (Rust)

```rust
// 模型分析 (GGUF/PGGUF 通用)
pub fn GGUF_Analyze(path: &Path) -> Result<Model_Arch_Info>;
// 分析 + 自动转换 GGUF→PGGUF
pub fn GGUF_Analyze_And_Convert(path: &Path, workspace: &Path) -> Result<PathBuf>;

// 模型切分 (输出 {stem}_split_{start}_{end}.pgguf)
pub fn GGUF_Split_Model(path: &Path, start: usize, end: usize, workspace: &Path) -> Result<PathBuf>;

// 按层加载权重
pub fn GGUF_Load_Layer(path: &Path, layer_idx: usize, device: &Device) -> Result<HashMap<String, QTensor>>;

// 完整加载组装模型
pub fn GGUF_Load_Model(path: &Path, start: usize, end: usize, device: &Device) -> Result<GGUF_Model>;
```

#### 3.4.4 Model_Arch_Info

```rust
pub struct Model_Arch_Info {
    pub architecture: String,       // "qwen3"
    pub num_layers: usize,          // transformer block 数
    pub embedding_length: usize,
    pub head_count, head_count_kv, head_dim: usize,
    pub feed_forward_length, context_length: usize,
    pub rms_norm_eps, rope_freq_base: f64,
    pub vocab_size: usize,
    pub eos_token_id: u32,
    pub layers: Vec<Layer_Info>,

    // 仅 PGGUF 有
    pub model_id: Option<u32>,
    pub layer_bitmap: Option<[u8; 32]>,
    pub is_split: bool,
    pub split_start, split_end: usize,
}
```

#### 3.4.5 MlSession (推理会话)

```rust
impl MlSession {
    pub fn New(device: Device) -> MlSession;
    pub fn Load_Model(&mut self, path: &Path, start: usize, end: usize) -> Result<()>;
    pub fn Unload(&mut self);
    pub fn Has_Model(&self) -> bool;
    pub fn Encode(&self, text: &str) -> Result<Vec<u32>>;          // Tokenizer
    pub fn Decode(&self, token_id: u32) -> Result<String>;
    pub fn Tensorize(&self, token_ids: &[u32]) -> Result<Tensor>;  // 无需模型已加载
    pub fn Forward(&self, tensor: &Tensor, offset: Option<usize>) -> Result<Tensor>;
    pub fn Sample(&self, logits: &Tensor, temperature: f64) -> Result<u32>;
    pub fn Get_Eos(&self) -> u32;
}
```

---

### 3.5 Network

**位置**: `Src/Network/`

libp2p 协议栈：TCP + Noise 加密 + Yamux 多路复用 + mDNS 发现 + Kademlia DHT + Request-Response + Stream + Ping。

```rust
#[async_trait]
pub trait Network_Capability {
    async fn Send_Data(&self, peer: PeerId, data_type: &str, payload: &[u8]) -> Result<Vec<u8>>;
    async fn Dial(&self, addr: &str) -> Result<()>;
    async fn Disconnect(&self, peer: &PeerId) -> Result<()>;
    async fn Test_Bandwidth(&self, peer: &PeerId) -> Result<()>;
    async fn Send_File(&self, peer: &PeerId, path: &Path) -> Result<()>;
    async fn Open_Tensor_Stream(&self, peer: &PeerId, inference_id: u64) -> Result<NetworkStream>;
    async fn Accept_Tensor_Stream(&self, inference_id: u64, timeout: u64) -> Result<NetworkStream>;
    async fn Send_Response(&self, request_id: u64, data_type: &str, payload: &[u8]) -> Result<()>;
    async fn Put_Record(&self, key: &str, value: &[u8]) -> Result<()>;
    async fn Get_Record(&self, key: &str) -> Result<Vec<u8>>;
    async fn Open_File_Stream(&self, peer: &PeerId, file_id: &str) -> Result<FileStream>;
    async fn Send_File_Data(&self, stream: &mut FileStream, data: &[u8]) -> Result<()>;
    async fn Receive_File_Data(&self, stream: &mut FileStream, path: &Path, size: u64) -> Result<()>;
    async fn Send_Tensor(&self, stream: &mut NetworkStream, tensor: &Tensor, offset: u64) -> Result<()>;
    async fn Recv_Tensor(&self, stream: &mut NetworkStream, device: &Device) -> Result<(Tensor, u64)>;
    async fn Send_Eof(&self, stream: &mut NetworkStream) -> Result<()>;
}
```

**Tensor 流协议**: `[offset: u64][len: u64][data: bytes]...`，EOF = `[u64::MAX][0]`

---

### 3.6 Orchestrator

**位置**: `Src/Orchestrator/`

`Core` 结构体的主循环通过 `tokio::select!` 处理 5 个分支：

| 分支 | 功能 |
|------|------|
| B1 | `route_user()` — 用户命令 → 查找/执行 Lua 脚本 |
| B2 | `route_inbound()` — 解析 NetworkProtocol (ESTABLISH_TENSOR_STREAM, JOIN_PIPELINE 等) |
| B3 | `route_stream()` — FileStreamArrived, TensorStreamArrived |
| B4 | `route_lifecycle()` — JobExecutor 完成回调 |
| B5 | `route_eventbus()` — ⚠️ 未实现，EventBus 由 TUI 直接消费（见 3.8 节） |

**Capabilities 容器**:
```rust
pub struct Capabilities {
    pub network: Arc<dyn Network_Capability>,
    pub storage: Arc<dyn StorageCapability>,
    pub peer_manager: Arc<PeerManager>,
    pub event_bus: Arc<EventBus>,
    pub local_stream_hub: Arc<LocalStreamHub>,
}
```

---

### 3.7 VM (Lua 引擎)

**位置**: `Src/VM/`

#### 3.7.1 沙箱环境

只加载 `string`, `table`, `math`。**禁用** `os`, `io`, `require`, `dofile`, `loadfile`。

**脚本发现**: `ProgramRegistry` 在启动时扫描 `programs/builtin/` 和 `programs/user/`，user 目录优先级高于 builtin。每个脚本按 `COMMAND` 全局变量注册命令名，通过 TUI `exec <command>` 调用。

#### 3.7.2 Lua 脚本规范

每个脚本必须声明两个全局变量：
```lua
COMMAND = "mycommand"
DESCRIPTION = "description"
function execute(params)
    -- ...
end
```

#### 3.7.3 Lua 可用 API 完整参考

**`caps` 表 (基础能力)**:
```lua
caps.print(msg)              -- 写入日志 + TUI
caps.echo(msg) → String       -- 测试回显
caps.add(a, b) → f64          -- 测试加法
caps.ping() → String          -- 异步 pong
```

**`ml` 表 (ML Engine)**:
```lua
ml.new(device) → MlSession                           -- 创建推理会话 ("cpu"|"cuda")
ml.tensor_from_bytes(bytes, device) → LuaTensor       -- 字节反序列化
ml.analyze_model(path) → Table                        -- 异步，返回 15 个字段
ml.split_model(path, start, end, output_dir) → ()     -- 异步切分
```

**MlSession 方法** (UserData):
```lua
sess:load_model(path, start, end)   -- 仅支持 .pgguf
sess:unload()
sess:has_model() → bool
sess:encode(text) → table<u32>      -- Tokenizer 编码
sess:decode(token_id) → String     -- 单 token 解码
sess:tensorize(token_ids) → LuaTensor
sess:forward(tensor, offset?) → LuaTensor
sess:sample(logits, temperature) → u32
sess:get_eos() → u32
sess:get_offset() → usize        -- 当前 forward offset
sess:set_seed(seed)              -- 设置随机种子
```

**LuaTensor 方法** (UserData):
```lua
tensor:dims() → table<usize>      -- 维度信息
tensor:to_bytes() → String       -- 序列化为字节
tensor:to_device(device) → LuaTensor  -- 迁移到指定设备 ("cpu"|"cuda")
```

**`caps.storage_*` 表 (Storage)**:
```lua
caps.storage_list() → Table                             -- 异步
caps.storage_exists(file_id) → bool                      -- 异步
caps.storage_acquire_read(file_id) → StorageReadHandle   -- 异步, handle:path(), :release()
caps.storage_acquire_write(file_id) → StorageWriteHandle -- 异步
caps.storage_remove(file_id) → ()                        -- 异步
caps.storage_checksum(file_id, algo?) → String           -- 异步, algo: "blake3"|"sha256"|"xxhash64"
caps.storage_flush() → {added, removed}                  -- 异步
```

**`caps.network.*` 表 (Network)**:
```lua
caps.network.send_data(peer, data_type, payload) → {payload}
caps.network.get_local_peer_id() → String
caps.network.dial(addr)
caps.network.disconnect(peer)
caps.network.send_file(peer, file_path)
caps.network.open_tensor_stream(peer, inference_id) → NetworkStream
caps.network.accept_tensor_stream(inference_id, timeout) → NetworkStream
caps.network.send_tensor(stream, tensor, offset)
caps.network.recv_tensor(stream, device) → (LuaTensor, offset)
caps.network.send_eof(stream)
caps.network.send_response(request_id, data_type, payload)  -- 响应请求
caps.network.put_record(key, value)                          -- DHT 存储
caps.network.get_record(key) → Value                         -- DHT 查询
caps.network.open_file_stream(peer, file_id) → FileStream    -- 文件流
```

**`caps.storage_*` 补充方法**:
```lua
handle:release()                 -- 手动释放锁（否则 drop 时自动释放）
```

**`local_tensor.*` 表 (本地流)**:
```lua
local_tensor.open_stream(id) → LocalTensorStream
local_tensor.accept_stream(id, timeout_ms) → LocalTensorStream
local_tensor.send_tensor(stream, tensor, offset)
local_tensor.recv_tensor(stream, device) → (LuaTensor, offset)
local_tensor.send_eof(stream)
```

---

### 3.8 TUI

**位置**: `Src/TUI/`

ratatui + crossterm。双输入框布局：

```
┌──────────────────────┬──────────────┐
│ Log (70%)            │ Network (30%)│
├──────────────────────┴──────────────┤
│ Job (3 行)                          │
├─────────────────────────────────────┤
│ Command Output (8 行)               │
├─────────────────────────────────────┤
│ Prompt> (3 行)                      │
├─────────────────────────────────────┤
│ pleiades> (3 行, 命令输入)          │
└─────────────────────────────────────┘
```

**内置命令**: `run`, `pipeline`, `cancel`, `dp`, `set-device`, `ls`, `flush`, `reload`, `set-name`, `distribute`, `send`, `profile`, `exec`, `clear`, `quit`, `help`

---

### 3.9 Session_Manager

**位置**: `Src/Session_Manager/`

管理推理会话的生命周期和槽位分配。每个会话绑定到一个推理模型和推理设备。

```rust
#[async_trait]
pub trait Session_Capability {
    async fn Create_Session(&self, model_id: u32, device: &str, start: usize, end: usize) -> Result<(SessionId, IoFrontend)>;
    async fn Destroy_Session(&self, id: SessionId) -> Result<()>;
    async fn Get_Session(&self, id: SessionId) -> Result<SessionInfo>;
    async fn List_Sessions(&self) -> Result<Vec<SessionInfo>>;
    async fn Has_Capacity(&self) -> Result<bool>;
}
```

**核心结构**:
- `SessionManager` — 全局管理器，通过 `max_slots` 限制并发会话数
- `Session` — 内部结构，持有 MlSession + 设备信息 + 模型引用
- `Slot` / `SlotState` — 槽位状态机 (Idle / Occupied / Loading)
- `SessionInfo` — 对外暴露的只读会话信息

> ⚠️ 当前 `SessionManager` 已实现但尚未集成到 Core 主循环中。

### 4.1 模型加载与分析

```
1. 用户将 model.gguf 放入 Pleiades_Workspace/
2. flush → StorageManager 扫描 → 发现 model.gguf
3. flush 内部调用 GGUF_Analyze_And_Convert:
   a. 读取 GGUF → 计算 model_id (xxhash32)
   b. 构建 layer_bitmap
   c. 写入 model.pgguf (追加元数据)
   d. 删除原始 model.gguf
4. Storage 索引更新 → FileEntry
5. PeerManager 同步 supported_models
```

### 4.2 单机推理 (Lua `run` 脚本)

```lua
sess = ml.new("cpu")
sess:load_model(path, 0, 999999)     -- 加载完整模型
tokens = sess:encode(prompt)         -- Tokenizer 编码
t = sess:tensorize(tokens)           -- → Tensor[1, seq_len]
hidden = sess:forward(t, 0)          -- prefill
for i = 1, max_tokens do
    tok = sess:sample(hidden, 0.8)
    io.output(sess:decode(tok))
    if tok == eos then break end
    next_t = sess:tensorize({tok})
    hidden = sess:forward(next_t)    -- offset 自动递增
end
sess:unload()
```

### 4.3 本地流水线推理

两个 Lua 脚本分别运行，通过 `local_tensor` 双工配对：

```
Thread A: exec local_coord         Thread B: exec local_work
  │                                  │
  │ 加载前半模型 (0..mid)            │ 加载后半模型 (mid+1..N+1)
  │ open_stream("fwd")               │ accept_stream("fwd", timeout)
  │ accept_stream("bwd", timeout)    │ open_stream("bwd")
  │       ◄── 双工配对 ──►           │
  │                                  │
  │ encode + forward → send hidden   │ recv hidden → forward → send logits
  │ recv logits → sample → decode    │
```

### 4.4 文件传输

```
发送端:
  caps.storage_acquire_read(file_id) → handle:path()
  caps.network.send_file(peer, handle:path())

接收端:
  FileStreamArrived → storage.acquire_write(file_name) → (dest_path, guard)
  → network.receive_file_data(stream, dest_path, file_size)
  → guard drop → 自动注册到 Storage 索引
```

---

## 5. ⚠️ Agent 常见错误

以下是 Agent 在 Pleiades 项目中最常犯的错误，**请严格遵守**：

### 错误 1: 绕过 Storage 直接读文件

```
❌ local f = io.open("model.gguf", "r")        -- 绕过 Storage!
❌ std::fs::read("Pleiades_Workspace/model.gguf")  -- 绕过 Storage!
✅ caps.storage_acquire_read("model.pgguf") → handle:path()
```

### 错误 2: 认为 PGGUF 丢失了元信息

```
❌ "PGGUF 是 split model 产生的，原始 GGUF 的元信息已经丢失"
✅ PGGUF = 原始 GGUF + pleiades.model_id + pleiades.layer_bitmap
   所有权重、tokenizer、架构信息完整保留。
   元信息由 GGUF_Analyze_And_Convert 自动追加。
```

### 错误 3: 层编号从 1 开始

```
❌ for layer = 1, num_layers do ... end    -- 丢失了 embedding 层!
✅ 0=embedding, 1..N=transformer, N+1=output
```

### 错误 4: 在 Lua 绑定闭包中写业务逻辑

```rust
// ❌ 禁止
methods.add_method("analyze_model", |_, path: String| {
    let info = ...;  // 15 行业务逻辑
    Ok(t)
});

// ✅ 正确
impl MlSession {
    pub fn analyze_model(path: &str) -> Result<ModelArchInfo> { /* 独立函数 */ }
}
methods.add_method("analyze_model", |_, _, path: String| {
    MlSession::analyze_model(&path).map_err(|e| mlua::Error::runtime(e))
});
```

### 错误 5: 跳过 analyze_model 直接推断架构

```
❌ 根据文件名猜测架构 → "qwen3"
✅ 使用 ml.analyze_model(path) → table.architecture
```

### 错误 6: 自己实现 tokenizer

```
❌ 使用外部 tokenizer 库或手写 encode/decode
✅ 使用 sess:encode(text) / sess:decode(token_id)
   系统已集成 shimmytok tokenizer
```
