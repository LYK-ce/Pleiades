# Pleiades 项目模块现状总结

Presented by KeJi
Date ： 2026-05-17

## 1. 编译状态

| 条件 | 状态 |
|------|------|
| `cargo check --no-default-features` | ✅ 0 errors |
| `cargo test --no-default-features --lib` | ✅ 122 tests, 120 passed |
| `cargo test --no-default-features --test t01/02/03/04` | ✅ 20/20 passed |

## 2. 模块总览

| 模块 | 功能 | 单元测试 | 集成测试 | 状态 |
|------|------|---------|---------|------|
| **Storage** | 文件存储与模型管理 | 34 | 5 | ✅ |
| **Session_Manager** | 推理会话与 IO 通道管理 | 17 | 5 | ✅ |
| **PeerManagement** | P2P 节点信息管理 | 0 | 6 | ✅ |
| **EventBus** | 全局事件广播 | 4 | 4 | ✅ |
| **Orchestrator** | 命令路由与组件编排 | 10 | 0 | ⚠️ |
| **Network** | P2P 网络通信 | 4 | 8(ignore) | ⚠️ |
| **ML_Engine** | ML 推理引擎 | 2 | 0 | ❌ |
| **Lua** | Lua 脚本沙箱 | 3 | 0 | ⚠️ |
| **Config** | 配置解析与身份管理 | 0 | 0 | ❌ |
| **TUI** | 终端图形界面 | 0 | 0 | ❌ |
| **Vm_Base** | VM 执行骨架 | 47 | 0 | ⚠️ 计划移除 |
| **Scheduler** | 分布式拓扑调度 | — | — | 🗑️ 已移除 |

---

## 3. 各模块详情

### 3.1 Storage — 文件存储与模型管理 ✅

**功能**：管理 Pleiades 的文件存储，提供锁与 I/O 解耦的扁平命名空间。GGUF/PGGUF 模型文件统一管理，惰性发现磁盘文件。

**对外接口** (`StorageCapability`)：

| 方法 | 说明 |
|------|------|
| `acquire_read(file_id)` | 获取共享读锁 + 路径，惰性发现 |
| `acquire_write(file_id)` | 获取独占写锁 + 路径 |
| `remove(file_id)` | 删除文件，有活跃锁则返回 InUse |
| `exists(file_id)` | 检查是否存在，自动清理僵尸条目 |
| `list()` | 返回所有文件的 `FileEntry` 列表 |
| `checksum(file_id, algo)` | 实时计算文件校验码 (Blake3/Sha256/XxHash64) |
| `flush()` | 扫描磁盘同步索引，刷新 size，清理僵尸 |

**数据结构**：`FileEntry` (file_name, model_id, size, num_layers, layer_bitmap, architecture)、`ReadGuard`/`WriteGuard` 自动释放锁

**状态**：39 测试，最稳定模块。已知 `size` 非实时（仅 flush 时刷新）。

---

### 3.2 Session_Manager — 推理会话与 IO 通道管理 ✅

**功能**：管理推理会话生命周期和文本 IO 通道。每个 Session 持有通向 ML Thread 的固定通道对（通过 `IoHandle`），`connect()` 按需创建槽位级前端通道（通过 `IoFrontend`）。

**对外接口** (`Session_Capability`)：

| 方法 | 说明 |
|------|------|
| `create_session(model_id)` | 创建会话，返回 (session_id, IoHandle) |
| `destroy_session(session_id)` | 销毁会话，释放所有通道和槽位 |
| `connect(session_id)` | 申请槽位，返回 (slot_id, IoFrontend) |
| `list_sessions()` | 列出所有活跃 Session 信息 |
| `release_slot(session_id, slot_id)` | 释放槽位 |

**数据结构**：`Session` (含 ml_input_tx/ml_output_rx + frontend_pairs)、`IoHandle` (input_rx + output_tx, ML 侧)、`IoFrontend` (input_tx + output_rx, 前端侧)、`Slot` (Vacant/Occupied)

**状态**：17 单元 + 5 集成测试全过。通道对端已不再 drop。待实现跨层 batching 桥接和 `Take_Frontend`。

---

### 3.3 PeerManagement — P2P 节点信息管理 ✅

**功能**：管理网络中已知节点的资源信息（性能画像、持有模型列表），提供并发安全的读写操作。本地节点自动注册。

**对外接口** (`Peer_Management_Capability`)：

| 方法 | 说明 |
|------|------|
| `Get_Peers()` | 获取远程节点列表（排除本地） |
| `Get_Peer(peer_id)` | 获取单个节点信息 |
| `Contains_Peer(peer_id)` | 检查节点是否存在 |
| `Count()` | 获取节点总数 |
| `Upsert_Peer(peer_info)` | 添加或覆盖节点信息 |
| `Remove_Peer(peer_id)` | 移除节点（保护本地节点） |
| `Update_Profile(peer_id, profile)` | 更新性能画像（带宽/延迟/内存） |
| `Update_Supported_Models(peer_id, models)` | 更新持有模型列表 |
| `Update_Heartbeat(peer_id, latency)` | 更新心跳延迟 |
| `Update_Status(peer_id, status)` | 更新连接状态 |
| `Cleanup_Timeout_Peers(timeout)` | 清理超时节点 |
| `Clear()` | 清空所有节点（保留本地） |

**工厂函数**：`create_peer_management(local_peer_id) → (Arc<PeerManager>, Box<dyn Peer_Management_Capability>)`

**数据结构**：`PeerInfo` (peer_id, addresses, local, profile, supported_models)、`PeerProfile` (latency_ms, bandwidth_mbps, memory_mb)、`PeerStatus` (Connected/Disconnected)、`PeerCapability` (has_gpu, cpu_cores, gpu_name)、`SupportedModel` (id, file_name, layer_bitmap)

**状态**：6 集成测试全过，API 已稳定。

---

### 3.4 EventBus — 全局事件广播 ✅

**功能**：基于 `tokio::sync::broadcast` 的多生产者多消费者事件总线，独立于所有组件。TUI 通过订阅接收网络事件、推理进度、日志等。

**对外接口**：

| 方法 | 说明 |
|------|------|
| `New(capacity)` | 创建总线实例 |
| `Publish(event)` | 向所有订阅者广播事件 |
| `Subscribe()` | 创建新的订阅者接收端 |

**事件类型** (`Bus_Event`)：Peer_Discovered/Left、Connection_Established/Closed、Job_Created/State_Changed/Completed、Inference_Started/Token/Completed、File_Progress、Log、Error、Device_Changed

**状态**：8 测试全过，稳定无改动需求。

---

### 3.5 Orchestrator — 命令路由与组件编排 ⚠️

**功能**：策略路由与组件编排层。接收 TUI/CLI 的 `UserCommand` 和 Network 的入站请求，组合调用各 Capability 方法完成业务编排。不负责推理、传输、存储——只做路由。

**对外接口** (Core)：

| 方法 | 说明 |
|------|------|
| `new(capabilities, user_cmd_rx, inbound_rx, net_inbound_rx)` | 创建 Core 实例 |
| `run()` | 4 分支 select! 事件循环 |

**Capabilities 容器**：`network: Box<dyn Network_Capability>`、`storage: Arc<dyn StorageCapability>`、`peer_manager: Box<dyn Peer_Management_Capability>`、`session: Box<dyn Session_Capability>`、`event_bus: Arc<EventBus>`

**UserCommand 变体** (10 个)：Execute、Run、Cancel、Quit、DisplayPeer、SetDevice、DistributeModel、List、Send、Profile

**NetworkProtocol 变体** (3 个)：Establish_Tensor_Stream、Join_Pipeline、Profile_Request

**状态**：Core 骨架就绪，DisplayPeer/SetDevice/List/Cancel/Quit 已实现，其余 `todo!()`。core/ 5 个子模块零测试。

---

### 3.6 Network — P2P 网络通信 ⚠️

**功能**：P2P 网络层，负责 mDNS 发现、Kademlia DHT、请求-响应传输、文件流、张量流、带宽测试流。内部 Network_Service 事件循环处理 Swarm 事件。

**对外接口** (`Network_Capability`，15 个方法)：

| 类别 | 方法 |
|------|------|
| 请求-响应 | `send_data`, `send_response` |
| 连接管理 | `dial`, `disconnect` |
| 文件流 | `open_file_stream`, `send_file_data`, `receive_file_data` |
| 张量流 | `open_tensor_stream`, `accept_tensor_stream` |
| DHT | `put_record`, `get_record` |
| 节点信息 | `get_local_peer_id` |
| 带宽测试 | `test_bandwidth` |

**子协议**：`Request_Response/` (编解码)、`File_Stream/`、`Tensor_Stream/` (含 rendezvous 匹配)、`Bandwidth_Stream/`

**入站事件**：`FileStreamArrived { peer, stream }`、`TensorStreamArrived { peer, stream }`

**状态**：4 个浅层测试，8 集成测试全部 `#[ignore]`。子模块路径已修复。

---

### 3.7 ML_Engine — ML 推理引擎 ❌

**功能**：GGUF 模型解析、加载、推理、编解码、采样。支持 Qwen3 模型。通过 `MlSession` 提供 `mlua::UserData` 接口供 Lua 脚本调用。

**主要导出**：

| 类别 | 类型/函数 |
|------|----------|
| 张量序列化 | `GGUF_Tensor_Packet`, `GGUF_Tensor_Serialize/Deserialize` |
| 模型管理 | `GGUF_Analyze`, `GGUF_Load_Layer`, `GGUF_Split_Model` |
| 模型抽象 | `GGUF_Model`, `GGUF_Load_Model`, `GGUF_Model_Inference` |
| 推理上下文 | `MlSession` (encode/decode/tensorize/forward/sample/unload) |
| 独立函数 | `analyze_model(path)`, `split_model(src, start, end, out_dir)` |

**MlSession Lua 方法**：encode、decode、tensorize、forward、sample、get_eos、get_offset、unload

**状态**：仅 2 个错误路径测试。`tensorize`/`forward` 改为 stub。GGUF 加载/推理/Qwen3 全无测试。

---

### 3.8 Lua — Lua 脚本沙箱 ⚠️

**功能**：提供安全的 Lua 执行环境，禁用 os/io/require 等危险 API。为 Orchestrator 的策略脚本提供运行时。

**对外接口**：

| 方法 | 说明 |
|------|------|
| `LuaContext::new()` | 创建沙箱化 Lua 实例 |
| `ProgramRegistry` | 程序注册表（按 COMMAND 名查找 Lua 脚本） |

**状态**：3 个测试。Lua capability binding 待实现。

---

### 3.9 Config — 配置解析与身份管理 ❌

**功能**：读取/写入 `config.toml`，管理节点密钥对持久化。

**对外函数**：

| 函数 | 说明 |
|------|------|
| `Ensure_Config()` | 确保配置文件存在，返回 (Pleiades_Config, path) |
| `Read_Config(path)` | 解析配置文件 |
| `Update_Config(path, section, key, value)` | 修改配置项（保留格式/注释） |
| `Ensure_Identity(config_dir)` | 确保密钥对存在，返回 Keypair |

**配置段**：`[Log]`、`[Network]`、`[Runtime]`、`[Storage]`、`[Session]`

**状态**：零测试。`Scheduler_Config` 已移除。

---

### 3.10 TUI — 终端图形界面 ❌

**功能**：基于 ratatui + crossterm 的终端 UI。双输入框设计（命令 + Prompt），通过 EventBus 订阅事件更新面板，通过 mpsc 发送 UserCommand 给 Core。

**面板**：Log、Network、Job、Command Output、Prompt 输入、命令输入

**入口**：`TUI_Loop(event_rx, user_cmd_tx, io_broker)`

**状态**：零测试。`llm_io` 引用已迁移到 `session` 模块。

---

### 3.11 Vm_Base — VM 执行骨架 ⚠️

**功能**：提供 SlotFile（键值存储）和 VM 指令（Const/Move/Add/Sub/Jump/JumpIf/Timer）。原用于 Orchestrator_VM 指令分发，现已解耦。

**状态**：47 测试全过。**计划移除**。

---

## 4. 本次修复记录（2026-05-17）

| 类别 | 内容 |
|------|------|
| Scheduler | 删除模块目录，清理所有引用 |
| 路径修复 | `Cargo.toml`/`lib.rs`/`Network` 添加 `#[path]` |
| PeerManagement | 添加 `PeerStatus`/`PeerCapability`/`PeerEvent`，trait 新增方法 |
| Network | 方法名对齐、子模块路径修复 |
| Orchestrator | core 导入路径修正 |
| TUI | `llm_io`→`session`，`Pipeline`→`Execute` |
| ML_Engine | `tensorize`/`forward` 类型标注修复 |
| Session | 结构体添加通道对端字段，不再 drop |
| main.rs | 重写适配当前 API |
| 集成测试 | t01/t02/t04 按新 API 重写 |
| 文档 | `orchestrator_design.md`、`storage_design.md` 更新 |

## 5. 待办事项

- [ ] 移除 Vm_Base 模块
- [ ] SessionManager 跨层消息路由（batching）
- [ ] `Take_Frontend` 完整实现
- [ ] Orchestrator route_inbound/route_stream 实现
- [ ] Config/ML_Engine/TUI 补充测试
- [ ] Network 子协议单元测试
- [ ] t07 network 集成测试激活
- [ ] t08 e2e 重写
- [ ] Storage `flush` 中 ML Analyze 集成
