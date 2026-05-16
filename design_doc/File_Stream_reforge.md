# File Stream 重构方案

## 0. 核心定义

> **File Stream = libp2p 文件流协议与工具函数。**
> 将当前平铺在 `stream_protocol.rs` 中的文件流代码提取为 `Src/Network/File_Stream/` 子目录，
> 与已有的 `Tensor_Stream/` 子目录对称，提升模块内聚性和可维护性。

---

## 1. 问题分析

### 1.1 当前结构

```
Src/Network/
├── stream_protocol.rs      ← 文件流协议（平铺，无子目录）
└── Tensor_Stream/          ← 张量流（已有子目录）
    ├── mod.rs
    ├── protocol.rs
    └── rendezvous.rs
```

| 问题 | 详情 |
|------|------|
| **结构不对称** | 张量流有 `Tensor_Stream/` 子目录，文件流是单文件平铺，组织方式不一致 |
| **缺乏扩展空间** | 平铺文件无法容纳未来可能需要的 rendezvous、metadata 等子模块 |
| **命名含义模糊** | `stream_protocol` 名称无法直观区分是"文件流协议"还是"张量流协议" |

### 1.2 设计差异（已确认，保持不变）

| 维度 | File Stream | Tensor Stream |
|------|-------------|---------------|
| **流向** | 单向一次性（写完即关） | 双向持久（推理全生命周期） |
| **建立方式** | Stream → Job（流先到，后创建 Job） | Job → Stream（先有 Job，再建立流） |
| **匹配机制** | 不需要 — 流自带 header（file_name, file_size, checksum） | RendezvousMap（两侧需以同一 inference_id 对撞） |
| **对端认知** | 接收方可完全不知情 | 两侧必须知道彼此（pipeline 环） |
| **本质** | 资源投递 | 会话建立 |

两者语义本质不同，**不强制统一建立顺序**。

---

## 2. 新方案

### 2.1 目标结构

```
Src/Network/
├── File_Stream/            ← 🆕 新建子目录
│   ├── mod.rs              ← 🆕 子模块声明 + 重导出
│   └── protocol.rs         ← 📦 从 stream_protocol.rs 整体迁入
└── Tensor_Stream/          ← 不变
    ├── mod.rs
    ├── protocol.rs
    └── rendezvous.rs
```

### 2.2 模块内部结构

#### protocol.rs（纯函数，无状态）

| 导出项 | 类型 | 说明 |
|--------|------|------|
| `FILE_STREAM_PROTOCOL` | `&str` | 协议标识符 `/pleiades/file-stream/1.0.0` |
| `CHUNK_SIZE` | `usize` | 分块大小 64KB |
| `FILE_HEADER_ACCEPT` | `u8` | ACK 接受标记 `0x01` |
| `FILE_HEADER_REJECT` | `u8` | ACK 拒绝标记 `0x00` |
| `Write_File_Stream_Header(stream, file_name, file_size, checksum)` | async fn | 写入 `[2B name_len BE][name][8B size LE][32B checksum]` |
| `Read_File_Stream_Header(stream)` → `(String, u64, [u8; 32])` | async fn | 读取 header |
| `Write_File_Stream_Ack(stream, accepted)` | async fn | 写入 1 字节 ACK |
| `Read_File_Stream_Ack(stream)` → `bool` | async fn | 读取 1 字节 ACK |
| `Send_File_Data(stream, file_path)` | async fn | 发送文件原始数据（64KB chunks） |
| `Receive_File_Data(stream, dest_path, file_size)` | async fn | 接收文件原始数据（64KB chunks） |

### 2.3 Network_Capability trait（不动）

`Network_Capability` trait 中的以下方法保持原样，仅 import 路径更新：

```rust
async fn open_file_stream(&self, peer: PeerId) -> Result<libp2p::Stream, Network_Error>;
async fn send_file_data(&self, stream: &mut libp2p::Stream, file_path: &Path) -> Result<(), Network_Error>;
async fn receive_file_data(&self, stream: &mut libp2p::Stream, dest_path: &Path, file_size: u64) -> Result<(), Network_Error>;
```

内部实现将委托路径从 `super::stream_protocol::*` 改为 `super::file_stream::protocol::*`。

### 2.4 Network_Service（不动）

`network_service.rs` 中的 `file_accept_control`、`file_open_control`、`incoming_file_streams` 逻辑完全不变，仅 `FILE_STREAM_PROTOCOL` 导入路径更新。

---

## 3. 影响范围（纯路径迁移，零逻辑变更）

### 3.1 删除

| 文件 | 说明 |
|------|------|
| `Src/Network/stream_protocol.rs` | 整文件删除（已迁入 `File_Stream/protocol.rs`） |

### 3.2 新建

| 文件 | 说明 |
|------|------|
| `Src/Network/File_Stream/mod.rs` | 🆕 `pub mod protocol;` + 统一重导出 |
| `Src/Network/File_Stream/protocol.rs` | 📦 从 `stream_protocol.rs` 整体复制，内容不变 |

### 3.3 修改（仅 import 路径）

| 文件 | 操作 | 说明 |
|------|------|------|
| `Src/Network/mod.rs` | ✏️ | `pub mod stream_protocol` → `pub mod file_stream`；`pub use stream_protocol::{...}` → `pub use file_stream::protocol::{...}` |
| `Src/Network/capability.rs` | ✏️ | `use super::stream_protocol::{...}` → `use super::file_stream::protocol::{...}` |
| `Src/Network/network_service.rs` | ✏️ | `use super::stream_protocol::FILE_STREAM_PROTOCOL` → `use super::file_stream::protocol::FILE_STREAM_PROTOCOL` |
| `Src/lib.rs` | ✏️ | `pub use network::stream_protocol` → `pub use network::file_stream::protocol` |
| `Src/Orchestrator/core/branch_stream.rs` | ✏️ | `use crate::network::stream_protocol::{...}` → `use crate::network::file_stream::protocol::{...}` |
| `Src/Orchestrator/Orchestrator_VM/network_handler.rs` | ✏️ | 同上 |

### 3.4 不变

| 模块 | 说明 |
|------|------|
| `Network_Capability` trait | trait 签名不变 |
| `Network_Service_Capability` | 字段/impl 不变 |
| `Network_Service` | 事件循环不变 |
| `DataType::File` | 暂不动（后续与其他冗余类型统一清理） |
| `Network_Inbound_Event::FileStreamArrived` | 不变 |

---

## 4. 实施计划

| # | 文件 | 操作 | 说明 |
|---|------|------|------|
| 1.1 | `Src/Network/File_Stream/mod.rs` | 🆕 新建 | 子模块声明 |
| 1.2 | `Src/Network/File_Stream/protocol.rs` | 📦 迁入 | 从 `stream_protocol.rs` 整体复制 |
| 1.3 | `Src/Network/stream_protocol.rs` | 🗑️ 删除 | |
| 1.4 | `Src/Network/mod.rs` | ✏️ | 更新模块声明和重导出路径 |
| 1.5 | `Src/Network/capability.rs` | ✏️ | 更新 import 路径 |
| 1.6 | `Src/Network/network_service.rs` | ✏️ | 更新 import 路径 |
| 1.7 | `Src/lib.rs` | ✏️ | 更新重导出路径 |
| 1.8 | `Src/Orchestrator/core/branch_stream.rs` | ✏️ | 更新 import 路径 |
| 1.9 | `Src/Orchestrator/Orchestrator_VM/network_handler.rs` | ✏️ | 更新 import 路径 |

**验证**：`cargo check` 通过。

### Phase 2：编写正式设计文档

| # | 文件 | 操作 | 说明 |
|---|------|------|------|
| 2.1 | `design_doc/File_Stream_design.md` | 🆕 新建 | 参考 `storage_design.md` 格式，整理为正式设计文档（模块概述、数据结构、协议格式、Trait 集成、已知风险） |

---

## 5. 讨论项

| # | 问题 | 结论 |
|---|------|------|
| 1 | File Stream 是否需要 `rendezvous.rs`？ | ❌ 不需要。文件流是单向投递，header 自描述，无需配对 |
| 2 | File Stream 与 Tensor Stream 的建立顺序是否应统一？ | ❌ 不应统一。文件流 Stream→Job、张量流 Job→Stream，语义不同 |
| 3 | 是否同时清理 `DataType::File`？ | ❌ 不在本次范围。本次仅做结构提取，后续在类型清理 Codereview 中处理 |
