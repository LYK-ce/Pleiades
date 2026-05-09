# Pleiades 项目阶段性总结报告

**日期**：2026-04-30  
**版本**：v0.1.0  
**作者**：KeJi

---

## 一、项目概述

**Pleiades** 是一个基于 Rust 构建的**边缘设备分布式推理运行时框架**。其核心目标是将大型语言模型（LLM）的推理任务拆分到多个 P2P 连接的边缘设备上，实现分布式流水线推理（Pipeline Inference）。

项目采用模块化、事件驱动架构，以 `tokio` 异步运行时为基础，使用 `libp2p` 实现去中心化的节点通信，使用 `candle` 框架进行 GGUF 模型的本地推理，支持 CUDA GPU 加速。

### 核心能力

| 能力 | 描述 |
|------|------|
| 单机推理 | 加载 GGUF 模型在本地节点执行完整推理 |
| 模型切分 | 按层范围将模型文件切分为分片 |
| 文件分发 | 通过 P2P 网络将模型文件/分片传送至远端节点 |
| 分布式推理 | 多节点流水线协同推理（Coordinator + Worker 模式） |
| 张量流传输 | 节点间通过持久化 Stream 实时传递中间张量 |
| TUI 界面 | 终端图形界面，实时展示日志、节点状态、作业进度 |

---

## 二、系统架构全景

```text
┌─────────────────────────────────────────────────────────────────────┐
│                            main.rs (入口)                            │
│  配置读取 → 组件初始化 → Network/TUI/Core 启动                        │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                    ┌──────────────┼──────────────┐
                    ▼              ▼              ▼
              ┌──────────┐  ┌──────────┐  ┌──────────────┐
              │   TUI    │  │ Network  │  │ Orchestrator │
              │(用户交互) │  │(P2P通信) │  │  (编排核心)   │
              └────┬─────┘  └────┬─────┘  └──────┬───────┘
                   │             │               │
                   │  UserCommand│  InboundReq   │ Capabilities
                   └────────────►│◄──────────────┘
                                 │
         ┌───────────────────────┼───────────────────────┐
         ▼              ▼        ▼         ▼             ▼
   ┌──────────┐  ┌──────────┐ ┌────────┐ ┌──────────┐ ┌──────────┐
   │ Storage  │  │ML_Engine │ │EventBus│ │ LLM_IO   │ │Tensor_IO │
   │(存储管理)│  │(推理引擎)│ │(事件总线│ │(文本通道)│ │(张量流)  │
   └──────────┘  └──────────┘ └────────┘ └──────────┘ └──────────┘
         ▲              ▲                       ▲             ▲
         │              │                       │             │
         └──────────────┴───────────────────────┴─────────────┘
                        PeerManagement (节点管理)
```

---

## 三、模块详解

### 3.1 Config (`Src/Config/`)

**职责**：配置文件解析与节点身份管理。

| 文件 | 作用 |
|------|------|
| `config.rs` | 解析 `config.toml`，提供 `Ensure_Config`/`Read_Config`/`Update_Config` 函数 |
| `identity.rs` | 管理节点 Ed25519 密钥对的持久化（`Ensure_Identity`） |
| `config.toml` | 配置文件模板（网络、日志、存储、运行时参数） |

**对外接口**：`Pleiades_Config`、`Ensure_Config()`、`Ensure_Identity()`

---

### 3.2 Network (`Src/Network/`)

**职责**：基于 libp2p 的 P2P 网络通信层，支持三种传输模式。

| 文件 | 作用 |
|------|------|
| `network_service.rs` | 核心事件循环：Swarm 事件处理、mDNS 发现、Kademlia DHT |
| `node_handle.rs` | NodeHandle：通过 mpsc 命令通道操作 Swarm（解耦网络线程） |
| `capability.rs` | `Network_Capability` trait 定义 + `Network_Service_Capability` 实现 |
| `data_protocol.rs` | Request-Response 协议编解码（PleiadesCodec） |
| `stream_protocol.rs` | 文件流传输协议（分块、Header、ACK） |
| `tensor_stream_protocol.rs` | 张量流协议（Frame 格式、Handshake、EOF） |
| `inbound_manager.rs` | 入站请求路由管理 |
| `outbound_manager.rs` | 出站响应路由管理 |

**传输模式**：
1. **Request-Response**：命令/控制消息（`DataType::Command`/`Data`/`File`）
2. **File Stream**：大文件流式传输（`/pleiades/file-stream/1.0.0`）
3. **Tensor Stream**：持久化张量流推理（`/pleiades/tensor-stream/1.0.0`）

**对外接口**：`Network_Capability` trait（send_data、open_file_stream、open_tensor_stream 等）

---

### 3.3 ML_Engine (`Src/ML_Engine/`)

**职责**：GGUF 模型的解析、加载、推理会话管理。

| 文件 | 作用 |
|------|------|
| `capability.rs` | `ML_Engine_Capability` trait（对 Orchestrator 的统一接口） |
| `service.rs` | `ML_Engine_Service` 实现（Session 注册表 + Storage 锁） |
| `ml_thread_engine.rs` | Session Thread 核心：独立 OS 线程执行推理循环 |
| `ml_thread_engine_instruction.rs` | 指令集定义（Instruction 枚举：Encode/Decode/Forward/Pipeline 等） |
| `ml_thread_register.rs` | 寄存器系统（Token/Tensor/Flag/Meta 等） |
| `gguf_model_manager.rs` | GGUF 文件分析 + 按层切分（`GGUF_Analyze`、`GGUF_Split_Model`） |
| `gguf_model.rs` | 模型加载与推理 API（`GGUF_Load_Model`、`GGUF_Model_Inference`） |
| `gguf_tensor.rs` | 张量序列化/反序列化（跨节点张量传输） |
| `GGUF_Models/qwen3.rs` | Qwen3 模型具体实现（权重加载、前向传播） |

**Session 生命周期**：
```text
Create_Session → Session Thread spawn (独立 OS 线程)
    → Run_Program (提交指令序列)
    → Shutdown_Session (终止线程 + 释放 Storage 锁)
```

**对外接口**：`ML_Engine_Capability` trait（Create_Session、Run_Program、Shutdown_Session、Analyze_Model、Split_Model）

---

### 3.4 Orchestrator (`Src/Orchestrator/`)

**职责**：系统编排核心，负责命令路由、作业生命周期管理、指令编译与执行。

| 文件 | 作用 |
|------|------|
| `core.rs` | Core 主循环（select! 四分支：用户命令/网络入站/流入站/生命周期） |
| `command.rs` | `UserCommand` 枚举 + `NetworkProtocol` 协议定义 |
| `compiler.rs` | 无状态编译器：将高级命令编译为 `TaskProgram` 指令序列 |
| `instruction.rs` | `TaskInstruction` 枚举（所有可执行指令的纯契约定义） |
| `job.rs` | `JobId`/`JobKind`/`JobState`/`LifecycleEvent` 公共契约 |
| `slot.rs` | 槽位系统（`SlotId`/`SlotValue`/`SlotFile`：指令间数据传递） |
| `inference_id.rs` | Pipeline 全局推理 ID 生成器 |
| `mod.rs` | `Capabilities` 结构体（聚合所有子系统引用） |

**Executor 子模块** (`Src/Orchestrator/executor/`)：

| 文件 | 作用 |
|------|------|
| `mod.rs` | `JobExecutor`：spawn 独立 tokio task 执行 TaskProgram |
| `task_engine.rs` | `TaskEngine`：指令解释器（IP 驱动、Forward/Compensation 双模式） |
| `handler_data.rs` | 数据操作 handler（Const、Move） |
| `handler_control.rs` | 控制流 handler（JumpIf、Abort） |
| `handler_inference.rs` | 推理生命周期 handler（CreateSession、RunProgram、ShutdownSession、AnalyzeModel、SplitModel） |
| `handler_network.rs` | 网络操作 handler（SendFile、ReceiveFile） |

**Core 主循环四分支**：
| 分支 | 通道来源 | 处理逻辑 |
|------|---------|---------|
| B1 `user_cmd_rx` | TUI/CLI | 路由用户命令 → Compiler → spawn Job |
| B2 `inbound_rx` | Network Request-Response | 解析命令协议 → 调度处理 |
| B3 `network_inbound_rx` | Network Stream | 文件流/张量流到达事件 |
| B4 `lifecycle_rx` | JobExecutor | Job 完成/失败 → 清理资源 |

---

### 3.5 Storage (`Src/Storage/`)

**职责**：本地文件存储管理，提供配额控制、读写锁、完整性校验。

| 文件 | 作用 |
|------|------|
| `capability.rs` | `StorageCapability` trait + `StorageError` + `QuotaInfo` |
| `manager.rs` | `StorageManager` 实现（文件注册表、配额、Flush 同步） |
| `guard.rs` | `ReadGuard`/`WriteGuard`（RAII 读写锁守卫） |
| `reservation.rs` | `Reservation`（空间预留，写入前原子扣减配额） |

**对外接口**：`StorageCapability` trait（acquire_read、acquire_write、remove、list、flush、check_quota）

---

### 3.6 LLM_IO (`Src/LLM_IO/`)

**职责**：大语言模型文本交互通道层，连接前端（TUI）与 ML Thread 的双向文本流。

| 文件 | 作用 |
|------|------|
| `capability.rs` | `LLM_IO_Capability` trait + `IoFrontend`/`IoHandle` |
| `broker.rs` | `LLM_IO_Broker`（通道分配/回收/查询） |

**使用流程**：
```text
1. Orchestrator: broker.Allocate(job_id) → 创建双向通道
2. ML Side: broker.Take_ML_Side(job_id) → IoHandle 注入 Session
3. Frontend: broker.Take_Frontend(job_id) → IoFrontend 给 TUI
4. 结束: broker.Deallocate(job_id) → 清理
```

---

### 3.7 Tensor_IO (`Src/Tensor_IO/`)

**职责**：分布式推理张量流的共享锁管理与运行时热切换。

| 文件 | 作用 |
|------|------|
| `tensor_port_switch.rs` | `Tensor_Port_Switch`：以 inference_id 为 key 管理张量流注册/端点创建/热切换/清理 |

**设计特点**：
- Core 管理张量流（Mutex 保护的 libp2p::Stream）
- ML Thread 零拷贝访问（通过 Arc 共享 Mutex 内的 stream）
- 支持广播发送（一次写入多个出站目标）
- 故障上报不中断推理（异步 channel 通知 Core）

---

### 3.8 EventBus (`Src/EventBus/`)

**职责**：全局事件广播系统（多生产者多消费者），基于 `tokio::sync::broadcast`。

| 文件 | 作用 |
|------|------|
| `event.rs` | `Bus_Event` 枚举（网络事件/作业事件/推理事件/日志） |
| `event_bus.rs` | `EventBus` 结构体（Publish/Subscribe） |

**事件类型**：Peer_Discovered、Connection_Established、Job_Created、Job_Completed、Inference_Token、Log 等。

---

### 3.9 PeerManagement (`Src/PeerManagement/`)

**职责**：节点信息管理（状态跟踪、能力发现、并发安全访问）。

| 文件 | 作用 |
|------|------|
| `peer_info.rs` | `PeerInfo`/`PeerStatus`/`PeerCapability` 数据结构 |
| `peer_manager.rs` | `PeerManager`（RwLock 保护的节点注册表） |
| `peer_handle.rs` | `PeerHandle`（impl `Peer_Management_Capability`） |
| `capability.rs` | `Peer_Management_Capability` trait 定义 |

---

### 3.10 TUI (`Src/TUI/`)

**职责**：终端图形界面，基于 `ratatui` + `crossterm`。

| 文件 | 作用 |
|------|------|
| `mod.rs` | `TUI_Loop` 主循环（事件处理 + 渲染） |
| `app.rs` | 应用状态管理（App 结构体） |
| `log_panel.rs` | 日志显示区 |
| `network_panel.rs` | 网络/节点状态区 |
| `job_panel.rs` | 作业进度区 |
| `command_panel.rs` | 命令输入/输出区 |

**布局**：
```text
┌─────────────────────────┬────────────────┐
│ Log (70%)               │ Network (30%)  │
├─────────────────────────┴────────────────┤
│ Job (3行)                                 │
├──────────────────────────────────────────┤
│ Command Output (8行)                      │
├──────────────────────────────────────────┤
│ Prompt> (推理对话输入)                     │
├──────────────────────────────────────────┤
│ pleiades> (系统命令输入)                   │
└──────────────────────────────────────────┘
```

---

## 四、模块间调用关系

### 4.1 依赖关系图

```text
                          ┌──────────────────┐
                          │   Orchestrator   │
                          │     (Core)       │
                          └────────┬─────────┘
                                   │ 通过 Capabilities 聚合引用
         ┌─────────┬───────┬───────┼────────┬──────────┬──────────┐
         ▼         ▼       ▼       ▼        ▼          ▼          ▼
    ┌─────────┐┌───────┐┌──────┐┌───────┐┌────────┐┌────────┐┌────────┐
    │Storage  ││ML     ││Net-  ││Peer   ││Event-  ││LLM_IO  ││Tensor  │
    │Manager  ││Engine ││work  ││Mgmt   ││Bus     ││Broker  ││_IO     │
    └─────────┘└───┬───┘└──────┘└───────┘└────────┘└────────┘└────────┘
                   │
                   ▼
              ┌─────────┐
              │Storage  │  (ML_Engine 通过 Arc 共享 Storage)
              │Manager  │
              └─────────┘
```

### 4.2 关键通信路径

| 路径 | 方向 | 机制 |
|------|------|------|
| TUI → Core | UserCommand | `mpsc::channel<UserCommand>` |
| Core → TUI | 事件通知 | EventBus broadcast |
| Network → Core (命令) | InboundRequest | `mpsc::channel<InboundRequest>` |
| Network → Core (流) | Network_Inbound_Event | `mpsc::channel<Network_Inbound_Event>` |
| JobExecutor → Core | LifecycleEvent | `mpsc::channel<LifecycleEvent>` |
| Core → JobExecutor | 取消信号 | `CancellationToken` |
| ML Thread ↔ TUI | 文本流 | LLM_IO Broker（双向 mpsc） |
| ML Thread ↔ Network | 张量流 | Tensor_Port_Switch（Mutex + libp2p::Stream） |
| ML_Engine → Storage | 文件访问 | `Arc<StorageManager>` 直接引用 |

### 4.3 作业执行流程

```text
用户输入 "run model.gguf"
    │
    ▼
TUI → UserCommand::Run → Core (B1 分支)
    │
    ▼
Core: Compiler.compile_run() → TaskProgram
    │
    ▼
Core: JobExecutor::new() + tokio::spawn
    │
    ▼
TaskEngine: step() 逐条执行指令
    │
    ├─ Const(model_path) → slot[1]
    ├─ Const(device) → slot[2]
    ├─ CreateSession → ML_Engine.Create_Session() → slot[3]
    ├─ RunProgram → ML_Engine.Run_Program() (阻塞直到推理结束)
    └─ ShutdownSession → ML_Engine.Shutdown_Session()
    │
    ▼
LifecycleEvent::Done → Core (B4 分支) → 清理 registry
```

---

## 五、测试体系

项目包含集成测试和端到端测试：

| 测试文件 | 覆盖内容 |
|---------|---------|
| `t01_storage_integration.rs` | Storage 模块生命周期（写入/读取/删除/配额） |
| `t02_llm_io_integration.rs` | LLM_IO 通道分配/释放/双向通信 |
| `t03_event_bus_integration.rs` | EventBus 发布/订阅/多消费者 |
| `t04_peer_management_integration.rs` | PeerManager 注册/状态更新/并发 |
| `t05_ml_engine_integration.rs` | ML_Engine Session 生命周期 |
| `t07_network_integration.rs` | Network 协议编解码/流传输 |
| `t08_e2e_inference.rs` | 端到端推理流程（Core → ML_Engine → 输出） |

此外，Orchestrator 内部各 handler 均有模块内单元测试。

---

## 六、技术栈

| 类别 | 技术/库 | 用途 |
|------|---------|------|
| 语言 | Rust (2021 edition) | 系统编程 |
| 异步 | tokio + futures | 异步运行时 |
| P2P | libp2p (0.56) | 去中心化网络 |
| ML | candle (0.10.2) | GGUF 模型推理 |
| GPU | CUDA (可选) | GPU 加速推理 |
| 序列化 | bincode + serde | 网络协议 |
| TUI | ratatui + crossterm | 终端界面 |
| 日志 | tracing + tracing-appender | 结构化日志 |
| 哈希 | blake3 + sha2 + xxhash | 文件完整性校验 |
| 分词 | shimmytok | Tokenizer |

---

## 七、当前进展总结

截至 2026-04-30，项目已完成 **56 项任务**，核心进展如下：

### 已完成

1. **Orchestrator 编排核心**：Core 主循环四分支全部就绪，支持用户命令路由、网络入站处理、流事件、生命周期管理。
2. **指令系统**：TaskInstruction 定义完整（数据操作/推理/网络/控制流），Compiler 可编译 Run/Distribute/Send/Receive 四种作业类型。
3. **执行引擎**：TaskEngine 可解释执行指令序列，支持正向执行 + 补偿链。
4. **ML 推理**：支持 Qwen3 模型本地推理，Session 独立线程。
5. **文件传输**：P2P 文件流传输完整实现（Header 协商 + 分块传输 + 校验）。
6. **张量流**：Tensor_Port_Switch 实现热切换、广播、故障上报。
7. **存储管理**：配额控制、RAII 锁守卫、文件注册表。
8. **TUI**：双输入框布局（命令 + 推理对话），实时事件展示。
9. **main.rs 入口**：完整的启动引导流程（6 阶段初始化 + 运行时）。

### 进行中 / 待修复

- 任务 56：修复 `Deregister_Pipeline(job_id_val)` 问题，清除旧版 Pipeline 残留代码。

---

## 八、接下来的工作

根据当前 task.md 和代码中的 TODO 标记，后续工作方向如下：

### 短期（即将执行）

1. **Pipeline 残留清理**（任务 56）：修复 `Deregister_Pipeline` 的参数问题，移除旧版 Pipeline 相关代码。
2. **新版 Pipeline 实施方案**：在 Compiler 中用新设计替换 TODO 占位符，实现 Coordinator/Relay 作业的编译逻辑。
3. **分布式推理端到端联调**：Coordinator 建立 Pipeline → Worker 加载模型 → 张量流传输 → 逐层推理完整流程。

### 中期

4. **设备管理完善**：重新设计 DeviceLease 体系（当前已移除旧方案，待设计新方案）。
5. **错误恢复与重试**：当前 Job 失败直接终止，后续需要添加自动重试和故障转移逻辑。
6. **DHT 模型注册**：利用 Kademlia DHT 记录各节点持有的模型信息，实现模型发现。
7. **性能优化**：ML Thread 中的张量零拷贝路径优化，减少锁竞争。

### 长期

8. **多模型支持**：扩展 GGUF_Models 支持更多模型架构。
9. **动态负载均衡**：基于各节点计算能力自动分配推理层范围。
10. **Web API 前端**：除 TUI 外提供 HTTP/WebSocket API 接口。
11. **安全性**：节点身份验证、传输加密、访问控制。

---

## 九、目录结构总览

```text
Pleiades/
├── .config/                    # 配置目录（config.toml + keypair.bin）
├── docs/                       # 设计文档
├── Src/
│   ├── main.rs                 # 入口点
│   ├── lib.rs                  # 库根（模块声明 + 类型导出）
│   ├── Config/                 # 配置解析 + 身份管理
│   ├── Network/                # P2P 网络层
│   ├── ML_Engine/              # ML 推理引擎
│   ├── Orchestrator/           # 编排核心
│   │   ├── core.rs             # Core 主循环
│   │   ├── compiler.rs         # 指令编译器
│   │   ├── command.rs          # 命令定义
│   │   ├── instruction.rs      # 指令契约
│   │   ├── job.rs              # 作业契约
│   │   ├── slot.rs             # 槽位系统
│   │   └── executor/           # 执行引擎
│   │       ├── mod.rs          # JobExecutor
│   │       ├── task_engine.rs  # TaskEngine 解释器
│   │       └── handler_*.rs    # 各类指令处理器
│   ├── Storage/                # 存储管理
│   ├── LLM_IO/                 # 文本交互通道
│   ├── Tensor_IO/              # 张量流管理
│   ├── EventBus/               # 事件总线
│   ├── PeerManagement/         # 节点管理
│   └── TUI/                    # 终端界面
├── tests/                      # 集成测试
├── Cargo.toml                  # 项目配置
└── task.md                     # 任务跟踪
```

---

*本报告基于 2026-04-30 代码快照编写，反映项目当前阶段的架构设计与实现状态。*
