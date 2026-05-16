# Request_Response 子模块提取方案

Presented by KeJi
Date ： 2026-05-16

## 1. 背景

当前 Network 模块内，`data_protocol.rs`、`inbound_manager.rs`、`outbound_manager.rs` 三文件共同实现 Request-Response 传输机制，与已提取的 `File_Stream/` 和 `Tensor_Stream/` 是同构的独立传输协议子系统。

提取后 Network 三种传输机制完全对称：

| 子系统 | 协议定义 | 入站处理 | 出站处理 / 交接 |
|--------|---------|---------|----------------|
| Request_Response | codec.rs | inbound.rs | outbound.rs |
| File_Stream | protocol.rs | (Orchestrator侧) | (Orchestrator侧) |
| Tensor_Stream | protocol.rs | rendezvous.rs | (Capability侧) |

## 2. 修改范围

### 2.1 涉及修改的文件清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `Src/Network/data_protocol.rs` | 移动+改名 | → `Src/Network/Request_Response/codec.rs` |
| `Src/Network/inbound_manager.rs` | 移动+改名 | → `Src/Network/Request_Response/inbound.rs` |
| `Src/Network/outbound_manager.rs` | 移动+改名 | → `Src/Network/Request_Response/outbound.rs` |
| `Src/Network/Request_Response/mod.rs` | **新建** | 子模块入口 + re-export |
| `Src/Network/mod.rs` | 修改 | 删除三个 `pub mod`，新增一个 `pub mod request_response`，更新 re-export 路径 |
| `Src/Network/network_service.rs` | 修改 | 更新 3 处 `use super::data_protocol/inbound_manager/outbound_manager` 导入 |
| `Src/Network/capability.rs` | 修改 | 更新 1 处 `use super::data_protocol` 导入 |
| `Src/Network/node_handle.rs` | 修改 | 更新 1 处 `use super::data_protocol` 导入 |
| `Src/lib.rs` | 修改 | 更新 `pub use network::data_protocol` → `pub use network::request_response` |

### 2.2 各文件具体变更

#### (1) data_protocol.rs → Request_Response/codec.rs

内部 `use super::...` 无需改动（在 `Request_Response/` 下 `super` 即为 `Network/`，当前 `super::*` 引用均为 Network 内类型，无跨级依赖）。

文件头注释的模块描述需从 `//! 网络协议定义模块` 更新为 `//! Request-Response 编解码模块`。

#### (2) inbound_manager.rs → Request_Response/inbound.rs

内部引用：
- `use super::data_protocol::Network_Data` → `use super::codec::Network_Data`
- `use super::node_handle::InboundRequest` → `use super::super::node_handle::InboundRequest`

因为文件从 `Network/inbound_manager.rs`（`super` = Network）移动到 `Network/Request_Response/inbound.rs`（`super` = Request_Response，`super::super` = Network）。

#### (3) outbound_manager.rs → Request_Response/outbound.rs

内部引用：
- `use super::data_protocol::Network_Data` → `use super::codec::Network_Data`

#### (4) Request_Response/mod.rs（新建）

```rust
pub mod codec;
pub mod inbound;
pub mod outbound;

pub use codec::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use inbound::Inbound_Manager;
pub use outbound::Outbound_Manager;
```

#### (5) Network/mod.rs

删除：
```rust
pub mod data_protocol;
pub mod inbound_manager;
pub mod outbound_manager;
pub use data_protocol::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
```

新增：
```rust
pub mod request_response;
```

模块注释中 `inbound_manager` / `outbound_manager` 的描述更新为 `request_response`。

更新现有 re-export 行（第 44 行 `pub use capability::...`、第 47 行 `pub use data_protocol::...`），将 `data_protocol` 改为 `request_response::codec` 或统一通过 `request_response` 重新导出。

#### (6) network_service.rs

3 处 import 变更：
```rust
// 旧
use super::data_protocol::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
use super::inbound_manager::Inbound_Manager;
use super::outbound_manager::Outbound_Manager;

// 新
use super::request_response::codec::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
use super::request_response::inbound::Inbound_Manager;
use super::request_response::outbound::Outbound_Manager;
```

`test_single_bandwidth` 方法内的两处 `super::data_protocol::` 引用（第 857-858 行）同步更新为 `super::request_response::codec::`。

#### (7) capability.rs

```rust
// 旧
use super::data_protocol::{DataType, Network_Data};

// 新
use super::request_response::codec::{DataType, Network_Data};
```

#### (8) node_handle.rs

```rust
// 旧
use super::data_protocol::{DataType, Network_Data};

// 新
use super::request_response::codec::{DataType, Network_Data};
```

#### (9) lib.rs

```rust
// 旧
pub use network::data_protocol;

// 新
pub use network::request_response;
```

`lib.rs` 第 61-63 行的 Network 模块类型导出中：
```rust
// 旧
pub use network::{DataType, Network_Data};

// 新（保持不变，因为 Network/mod.rs 会 re-export 这些类型）
pub use network::{DataType, Network_Data};
```
如果 `Network/mod.rs` 仍 re-export `DataType` / `Network_Data`（通过 `pub use request_response::codec::{...}` 或等效路径），`lib.rs` 此处无需修改。

## 3. 提取后结构

```
Src/Network/
├── mod.rs                        # 模块入口
├── network_service.rs            # 核心事件循环
├── capability.rs                 # Network_Capability trait + 实现
├── node_handle.rs                # NodeHandle API + NodeCommand
├── Request_Response/             # ★ 新增子目录
│   ├── mod.rs                    # 子模块入口 + re-export
│   ├── codec.rs                  # TLV编解码 / DataType / Network_Data / PleiadesCodec
│   ├── inbound.rs                # 入站请求路由管理 (Inbound_Manager)
│   └── outbound.rs               # 出站响应路由管理 (Outbound_Manager)
├── File_Stream/                  # 文件流传输
│   ├── mod.rs
│   └── protocol.rs
└── Tensor_Stream/                # 张量流传输
    ├── mod.rs
    ├── protocol.rs
    └── rendezvous.rs
```

## 4. 实施步骤

### Step 1 — 创建目录

```bash
mkdir Src/Network/Request_Response
```

### Step 2 — 移动文件

```bash
git mv Src/Network/data_protocol.rs Src/Network/Request_Response/codec.rs
git mv Src/Network/inbound_manager.rs Src/Network/Request_Response/inbound.rs
git mv Src/Network/outbound_manager.rs Src/Network/Request_Response/outbound.rs
```

### Step 3 — 修改文件内部引用

修改 `Request_Response/codec.rs`：
- 文件头模块注释从 `//! 网络协议定义模块` 改为 `//! Request-Response 编解码模块`

修改 `Request_Response/inbound.rs`：
- `use super::data_protocol::Network_Data` → `use super::codec::Network_Data`
- `use super::node_handle::InboundRequest` → `use super::super::node_handle::InboundRequest`

修改 `Request_Response/outbound.rs`：
- `use super::data_protocol::Network_Data` → `use super::codec::Network_Data`

### Step 4 — 创建 Request_Response/mod.rs

创建 `Src/Network/Request_Response/mod.rs`，内容：
```rust
//Presented by KeJi
//Date ： 2026-05-16

//! Request-Response 传输子系统
//!
//! 提供基于 TLV 帧格式的请求-响应协议，包含：
//! - codec: TLV 编解码器 (PleiadesCodec)、数据类型枚举 (DataType)、网络数据帧 (Network_Data)
//! - inbound: 入站请求路由管理 (Inbound_Manager)
//! - outbound: 出站响应路由管理 (Outbound_Manager)

pub mod codec;
pub mod inbound;
pub mod outbound;

pub use codec::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use inbound::Inbound_Manager;
pub use outbound::Outbound_Manager;
```

### Step 5 — 修改 Network/mod.rs

- 删除 `pub mod data_protocol;`、`pub mod inbound_manager;`、`pub mod outbound_manager;` 三行
- 新增 `pub mod request_response;`
- 更新模块文档注释：`inbound_manager` / `outbound_manager` → `request_response`
- 删除第 47 行 `pub use data_protocol::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};`
- 如果需要在 Network 层保持扁平 re-export，新增：
  ```rust
  pub use request_response::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
  ```

### Step 6 — 修改 network_service.rs

将以下 3 个 use 语句：
```rust
use super::data_protocol::{...};
use super::inbound_manager::Inbound_Manager;
use super::outbound_manager::Outbound_Manager;
```
替换为：
```rust
use super::request_response::codec::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
use super::request_response::inbound::Inbound_Manager;
use super::request_response::outbound::Outbound_Manager;
```

同时更新 `test_single_bandwidth` 方法中的两处 `super::data_protocol::` 引用。

### Step 7 — 修改 capability.rs

```rust
// 旧
use super::data_protocol::{DataType, Network_Data};
// 新
use super::request_response::codec::{DataType, Network_Data};
```

### Step 8 — 修改 node_handle.rs

```rust
// 旧
use super::data_protocol::{DataType, Network_Data};
// 新
use super::request_response::codec::{DataType, Network_Data};
```

### Step 9 — 修改 lib.rs

```rust
// 旧
pub use network::data_protocol;
// 新
pub use network::request_response;
```

### Step 10 — 编译验证

```bash
cargo check
```

修复所有编译错误直至通过。

### Step 11 — 创建设计文档

在 `docs/` 目录下创建 `request_response_design.md`，格式参考 `docs/storage_design.md`，涵盖：

1. 模块概述
2. 数据结构（DataType、Network_Data、PleiadesCodec、InboundRequest、NodeCommand(SendData/SendResponse)、Inbound_Manager、Outbound_Manager）
3. Trait 定义（如适用）
4. 核心实现（TLV帧格式、编解码流程、入站路由流程、出站响应路由流程）
5. 模块导出
6. 与 Network_Service 的协作
7. 已知风险
8. TODO

## 5. 不修改的文件

以下文件无需任何改动：

| 文件 | 原因 |
|------|------|
| `Network/File_Stream/*` | 无任何 `data_protocol`/`inbound_manager`/`outbound_manager` 引用 |
| `Network/Tensor_Stream/protocol.rs` | 仅注释提及 `data_protocol.rs`，无代码引用 |
| `Network/Tensor_Stream/rendezvous.rs` | 独立，无引用 |
| `tests/*` | 测试通过 `pleiades::*` 公共 API 引用，不受内部路径变更影响 |
| 其他所有模块 | 均不直接依赖 `data_protocol`/`inbound_manager`/`outbound_manager` |
