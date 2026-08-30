# Pleiades 集成测试实施设计

**日期**：2026-04-24（更新：2026-04-27 D 组新增，2026-04-29 C 组更新 DataType 分流测试）
**基线**：全模块内联测试通过，集成测试 A 组完成，Orchestrator compile_run + handler_inference + Broker 托管分发 + UserCommand oneshot reply 已完成，Network DataType 预筛选已实现

---

## 1. 目标

为 Pleiades 各模块编写 **跨模块集成测试**，从外部消费者视角验证：
- 模块公共 API 的正确性
- 跨模块协作的完整性
- 并发安全性
- 资源生命周期（分配/释放/清理）

集成测试补充现有内联单元测试的不足，确保在 Orchestrator 后续开发前各基础模块稳固可靠。

---

## 2. 目录结构

```
tests/
├── tests_implementation_design.md          # 本文档
├── test_model/
│   └── Qwen3-0.6B-Q8_0.gguf              # 测试用模型（需手动放置）
├── common/
│   └── mod.rs                              # 共享测试辅助（Stub 工厂、环境构建）
├── t01_storage_integration.rs              # A 组：存储基础设施
├── t02_llm_io_integration.rs              # A 组：IO 通道
├── t03_event_bus_integration.rs            # A 组：事件总线
├── t04_peer_management_integration.rs      # A 组：节点管理
├── t05_ml_engine_integration.rs            # B 组：ML Engine（需真实模型）
├── t06_orchestrator_integration.rs         # B 组：编排器跨模块联合
├── t07_network_integration.rs              # C 组：网络（需真实 TCP）
└── t08_e2e_inference.rs                    # D 组：全链路端到端推理（需真实模型）
```

**命名约定**：`t{编号}_{模块}_integration.rs`，编号决定逻辑实施顺序。

---

## 3. 现有内联测试覆盖分析

| 模块 | 内联测试数 | 覆盖质量 | 集成测试缺口 |
|------|-----------|---------|-------------|
| Storage | 20 | ✅ 很完善 | 跨模块消费者视角 |
| LLM_IO | 11 | ✅ 很完善 | 多 Job 并发竞争 |
| EventBus | 4 | ✅ 基础完备 | 高并发、慢消费者 |
| PeerManagement | 0 | ❌ **无测试** | 完整 CRUD + 并发安全 |
| ML_Engine | 10 | ⚠️ 边界测试为主 | 端到端推理、模型切分、半模型 Session |
| Network | 4 | ⚠️ 仅错误类型 | 双节点真实通信 |
| Orchestrator | 20 | ⚠️ 全部使用 Stub | 跨模块真实 Capability |
| Config | 0 | ❌ **无测试** | 配置读写 + identity |

---

## 4. 共享辅助模块：`common/mod.rs`

提供所有集成测试复用的构建函数和 Stub 实现：

```rust
// 核心辅助函数
pub async fn stub_capabilities() -> (Arc<Capabilities>, TempDir)
pub async fn stub_io_handle(broker: &LLM_IO_Broker, job_id: JobId) -> IoHandle

// Stub 实现
pub struct StubCompute;       // ComputeCapability → Ok(DeviceLease)
pub struct StubInference;     // InferenceCapability → Ok(SessionHandle)
pub struct StubNetwork;       // Network_Capability → unimplemented!("stub")

// 程序构造器
pub fn simple_program() -> TaskProgram      // 两条 Const 指令
pub fn abort_program() -> TaskProgram       // Const + Abort + 补偿 Const
```

---

## 5. 测试用例详细设计

### A 组：基础设施模块

#### t01_storage_integration.rs

**目的**：从外部消费者视角（如 ML_Engine_Service）验证 Storage 生命周期。

| 编号 | 名称 | 描述 |
|------|------|------|
| TC-01 | 完整生命周期 | New → acquire_write → 写入 100KB → drop → acquire_read → 读取验证 → checksum → remove → 验证磁盘删除 |
| TC-02 | 并发多读单写 | 写入文件 → spawn 10 个并发 acquire_read → 全部成功 → 同时 spawn acquire_write 被阻塞 → drop 所有 ReadGuard → 写锁获得 |
| TC-03 | 初始化扫描 | 预先创建 3 个文件 → New(dir) → list() 返回 3 个 → exists() 各自验证 |
| TC-04 | 大文件校验码一致性 | 写入 1MB 文件 → 分别计算 XxHash64/Sha256/Blake3 → 重复读取验证一致 |
| TC-05 | remove 竞争条件 | 持有 ReadGuard → 另一任务 remove → 返回 InUse → drop ReadGuard → 重新 remove → Ok |

---

#### t02_llm_io_integration.rs

**目的**：验证 LLM_IO_Broker 在多 Job 并发场景下的通道隔离和生命周期。

| 编号 | 名称 | 描述 |
|------|------|------|
| TC-01 | 多 Job 并发分配与隔离 | 并发 Allocate 10 个 JobId → 每个 Job frontend 发唯一 prompt → ml_side 收 → 验证无串台 |
| TC-02 | 通道关闭传播 | Allocate → 双向通信 → Deallocate → drop frontend → ml_side.recv() = None |
| TC-03 | 高速吞吐 | Allocate 后连续发 1000 条消息 → 验证全部到达且顺序正确 |
| TC-04 | 并发分配不同 JobId 无竞争 | spawn 20 个任务同时 Allocate 不同 JobId → 全部成功 → Is_Active 全部 true |
| TC-05 | 重复分配同一 JobId 错误处理 | Allocate(job_1) → 再 Allocate(job_1) → AllocationFailed |

---

#### t03_event_bus_integration.rs

**目的**：验证 EventBus 在高并发和慢消费者场景下的行为。

| 编号 | 名称 | 描述 |
|------|------|------|
| TC-01 | 多生产者多消费者 | 5 个生产者各发 100 条 → 3 个消费者各收 500 条 |
| TC-02 | 慢消费者 Lagged 恢复 | EventBus(16) 小缓冲 → 快速发 100 条 → 消费者遇 Lagged → 验证可继续接收 |
| TC-03 | 全部 Bus_Event 变体（14 个） | 逐一发布全部 14 种 Bus_Event 变体 → 消费者 match 验证每个字段 |
| TC-04 | Subscribe 后发布才能接收 | 发布 → Subscribe → 再发布 → 仅收到后者 |

---

#### t04_peer_management_integration.rs

**目的**：为零测试的 PeerManagement 模块建立完整测试覆盖。

| 编号 | 名称 | 描述 |
|------|------|------|
| TC-01 | 批量节点 CRUD | add 20 → list = 20 → get_idle = Connected 数 → update_status Busy → get_busy 验证 → remove 5 → count = 15 |
| TC-02 | 心跳与超时清理 | add 5 → sleep 1.1s → 仅 update_heartbeat 3 个 → cleanup_timeout(1) → 清除 2 → count = 3 |
| TC-03 | 并发安全 | spawn 10 任务同时 add/update/list → 无 panic，最终 count 一致 |
| TC-04 | 能力与带宽更新 | add peer → update_capability → get_peer → 验证 capability 字段 → update_bandwidth → 验证 |
| TC-05 | 空管理器操作 | 空 manager → get_peer = Err → list = empty → remove = Err → is_empty = true |
| TC-06 | PeerHandle Clone 共享 | Clone PeerHandle → 两个 handle 操作同一 manager → 数据一致 |

---

### B 组：ML Engine + Orchestrator 跨模块联合

#### t05_ml_engine_integration.rs

**目的**：验证 ML_Engine_Service 通过 `dyn ML_Engine_Capability` trait 对象的跨模块集成，包括完整推理、模型切分和半模型 Session。

> **前置条件**：需要在 `tests/test_model/` 目录下放置 `Qwen3-0.6B-Q8_0.gguf` 模型文件。
> 若文件不存在，测试将 panic 并输出明确提示。
>
> **模型文件路径**：`tests/test_model/Qwen3-0.6B-Q8_0.gguf`
>
> **层编号规则**（Qwen3-0.6B，N=28 transformer blocks）：
> - Layer 0 = Embedding（token_embd.weight）
> - Layer 1~28 = Transformer Block（blk.0.* ~ blk.27.*）
> - Layer 29 (N+1) = Output（output_norm.weight + output.weight）

| 编号 | 名称 | 需模型 | 描述 |
|------|------|:-----:|------|
| TC-01 | 错误路径 | ❌ | `dyn ML_Engine_Capability` → Shutdown/Run 不存在的 session → Error；Create 不存在的模型 → Error + 无 Storage 锁泄漏 |
| TC-02 | Analyze 完整模型 | ✅ | `Analyze_Model` → `Model_Info` → 验证 `architecture="qwen3"`, `num_layers=28`, `is_split=false` |
| TC-03 | Create + Shutdown 生命周期 | ✅ | `Create_Session(0, 29)` → `Model_Info`（验证 `has_input_head=true, has_output_head=true`） → `Shutdown_Session` → 验证 Storage 读锁释放 |
| TC-04 | 完整单机推理 | ✅ | Create → IoHandle 通信 → `Run_Program`(单机指令序列, `max_tokens=5`) → frontend 收到流式 token → 验证 `Pipeline_Result` 非空 → Shutdown |
| TC-05 | Split 前半模型 | ✅ | `Split_Model(0, 14)` → `Analyze` 产物 → 验证 `is_split=true`, `split_start=0`, `split_end=14`, `num_layers=28`（保留原值） |
| TC-06 | Split 后半模型 | ✅ | `Split_Model(15, 29)` → `Analyze` 产物 → 验证 `split_start=15`, `split_end=29`，若 weight tying 则含 `token_embd.weight` |
| TC-07 | 前半模型 Session | ✅ | 用 TC-05 产物 → `Create_Session(0, 14)` → 验证 `has_input_head=true`, `has_output_head=false` → `Shutdown_Session` |
| TC-08 | 并发 Analyze | ✅ | 5 并发 `Analyze_Model` → 全部成功、结果一致 |

---

#### t06_orchestrator_integration.rs

**目的**：验证 Orchestrator 与真实 Storage + LLM_IO 的跨模块协作。

> **实施约束**：当前 `Compiler.compile_run()` / `compile_worker_relay()` 为 `todo!()` 占位符，
> `Core::spawn_job()` 为 private 方法。因此：
> - TC-02/03/05 直接构造 `JobExecutor` + 手写 `TaskProgram`，**绕过 Core 和 Compiler**
> - TC-04 仅测试 Quit 路径（不触发 Compiler）
> - 待 Compiler 实现后，补充通过 `Core` → `UserCommand::Run` 的端到端路径测试

| 编号 | 名称 | 描述 |
|------|------|------|
| TC-01 | Capabilities 完整组装 | 真实 Storage + 真实 LLM_IO_Broker + Stub Compute/Inference/Network → Arc\<Capabilities\> → 各字段可调用 |
| TC-02 | Job 生命周期端到端 | 组装 Capabilities → Broker.Allocate 获取 IoHandle → 手写简单 TaskProgram → **直接** JobExecutor.run() → LifecycleEvent::Done(Success) |
| TC-03 | Abort + 补偿 + IO | 同上 → abort_program → run() → Done(Failed) → 补偿序列执行 |
| TC-04 | Core Quit 优雅退出 | 组装 Core → 发送 UserCommand::Quit → run() 正常退出（不触发 Compiler） |
| TC-05 | 多 Job 并发（绕过 Core） | **直接** spawn 5 个 JobExecutor → 共享 lifecycle_rx → 全部收到 Done 事件 |

---

### C 组：Network 集成测试

#### t07_network_integration.rs

**目的**：验证双节点真实网络通信和 DataType 入站预筛选。**全部标记 `#[ignore]`**，仅手动执行。

> **测试基础设施**：每个测试启动两个 `Network_Service` 实例（各持独立 Keypair + StubPeerManager），
> 通过 `handle.Dial()` 手动连接（避免 mDNS 发现延迟），监听随机端口（`listen_port: 0`）。

**辅助函数**：
```rust
/// 启动一个轻量级 Network 节点
async fn spawn_node() -> (NodeHandle, PeerId, mpsc::Receiver<InboundRequest>,
                          mpsc::Receiver<Network_Inbound_Event>, Multiaddr, JoinHandle<...>)
```

| 编号 | 名称 | 描述 |
|------|------|------|
| TC-01 | BandwidthTest 分流 — Network 内部回复 | A→B 发送 `DataType::BandwidthTest`（size=1000）→ B 直接回复 1000 字节包 → 验证 B 的 `inbound_rx` 为空（未转发给 Core） |
| TC-02 | BandwidthTest 50MB 上限防护 | A→B 发送 `BandwidthTest`（size=100_000_000）→ B 回复包大小 = 50_000_000（截断到上限）→ 验证 `inbound_rx` 为空 |
| TC-03 | BandwidthTest payload 不足 8 字节 | A→B 发送 `BandwidthTest`（payload=3字节）→ B 回复 8 字节包 → 验证 `inbound_rx` 为空 |
| TC-04 | Data 分流 — Network 内部回复 OK | A→B 发送 `DataType::Data` → B 直接回复 `DataType::Data` + `b"OK"` → 验证 `inbound_rx` 为空 |
| TC-05 | Info 分流 — Network 内部回复 OK | A→B 发送 `DataType::Info` → B 直接回复 `DataType::Info` + `b"OK"` → 验证 `inbound_rx` 为空 |
| TC-06 | Command 转发到 inbound_rx | A→B 发送 `DataType::Command`（payload=`b"TEST_CMD"`）→ 在 B 的 `inbound_rx` 接收到 `InboundRequest`（验证 `data_type=Command`, `payload` 一致, `peer=A`）→ B 调用 `Send_Response` 回复 → A 收到 response |
| TC-07 | File 转发到 inbound_rx | A→B 发送 `DataType::File`（payload=`b"model.gguf|1024|abc123"`）→ 在 B 的 `inbound_rx` 接收到 `InboundRequest`（验证 `data_type=File`）→ B 调用 `Send_Response(Command, b"ACCEPT")` → A 收到 ACCEPT |
| TC-08 | PeerManagement 联动 | 两个节点 Dial 建立连接 → 验证 `StubPeerManager.Add_Peer` 被调用 → `Disconnect` → 验证 `Remove_Peer` 被调用 |

---

### D 组：全链路端到端推理

#### t08_e2e_inference.rs

**目的**：验证 `UserCommand::Run` → `Core::route_user` → `Compiler::compile_run` → `spawn_job` →
`JobExecutor` → `ML_Engine_Service` → `Session_Thread` → 真实推理输出的完整链路。
经过 Core 的 select! 主循环，模拟真实运行环境。

> **前置条件**：需要在 `tests/test_model/` 目录下放置 `Qwen3-0.6B-Q8_0.gguf` 模型文件。
>
> **关键依赖**：
> - `UserCommand::Run` 的 `oneshot` 回复通道（回传 `job_id`）
> - `LLM_IO_Broker` 的 `Take_Frontend(job_id)` 托管分发机制
> - `ML_Engine_Service` 真实推理能力
> - `Compiler::compile_run()` 已实现

**测试环境构建**：
1. 复制模型到 TempDir（同 t05 模式）
2. 创建两个 `StorageManager`：一个给 `ML_Engine_Service`（`Arc<StorageManager>`），一个给 `Capabilities`
3. `Capabilities { storage, ml_engine: Box::new(ML_Engine_Service), network: Box::new(StubNetwork), ui, io_broker }`
4. 创建 `Core`（真实 Compiler + 真实 Capabilities）+ user_cmd 通道
5. `tokio::spawn(core.run())`

**完整交互流程**：
```text
测试 → UserCommand::Run { model_path, reply_tx } → Core
Core: generate_id → Allocate → Take_ML_Side → compile_run → spawn_job → reply Ok(job_id)
测试 ← reply_rx.await → job_id
测试 → broker.Take_Frontend(job_id) → IoFrontend
测试 → frontend.input_tx.send("你好") → drop(input_tx)
ML Session_Thread: Input → Encode → Prefill → Sample → Decode → Output → Loop → EndOutput
测试 ← frontend.output_rx.recv() → token₁, token₂, ... (带超时收集)
测试 → UserCommand::Quit { reply_tx } → Core
Core: shutdown → cancel all → exit
测试: assert tokens 非空 + println!
```

| 编号 | 名称 | 需模型 | 描述 |
|------|------|:-----:|------|
| TC-01 | 单机推理全链路 | ✅ | 发送 Run → 收 job_id → Take_Frontend → 发 "你好" → 收集输出 tokens → `println!` 显示 → Quit → 验证 tokens 非空 |
| TC-02 | 编译错误路径 | ❌ | 发送 Run { model_path: "" } → reply_rx 收 Err（编译失败） → Quit → Core 正常退出 |

---

## 6. 实施优先级

```
P0（最高）  t04_peer_management  — 零测试模块，风险最高
P1          common/mod.rs        — 所有测试的共享基础
P2          t05_ml_engine        — 真实模型验证，覆盖推理+切分+半模型
P2          t08_e2e_inference    — 全链路端到端真实推理（验证系统集成）
P3          t06_orchestrator     — 验证跨模块组装，为后续开发打基础
P3          t01_storage          — 外部视角补充
P3          t02_llm_io           — 并发场景补充
P4          t03_event_bus        — 内联已覆盖核心
P5（最低）  t07_network          — 需真实 TCP，延后
```

---

## 7. 执行命令参考

| 场景 | 命令 |
|------|------|
| 运行全部测试（含内联 + 集成） | `cargo test` |
| 仅运行集成测试 | `cargo test --tests` |
| 仅运行特定集成测试文件 | `cargo test --test t04_peer_management_integration` |
| 仅运行 ML Engine 集成测试 | `cargo test --test t05_ml_engine_integration` |
| 仅运行 D 组端到端推理测试 | `cargo test --test t08_e2e_inference -- --nocapture` |
| 运行被 ignore 的网络测试 | `cargo test --test t07_network_integration -- --ignored` |
| 仅运行现有内联测试 | `cargo test --lib` |
| 运行并显示输出 | `cargo test -- --nocapture` |

---

## 8. 依赖项

当前 `Cargo.toml` 已包含所需依赖：
- `tempfile = "3.10"` — 临时目录创建
- `tokio = { features = ["full"] }` — 异步测试运行时
- `async-trait` — Stub trait 实现

无需新增 `[dev-dependencies]`。

---

## 9. 注意事项

1. **Windows 路径**：Storage 测试使用 `tempfile::TempDir`，自动处理跨平台路径
2. **Network 测试隔离**：全部标记 `#[ignore]`，避免 CI 中因端口占用失败
3. **ML Engine 测试模型**：`tests/test_model/Qwen3-0.6B-Q8_0.gguf` 需手动放置，不纳入版本控制（.gitignore）。测试启动时检测，不存在则 panic 提示
4. **ML Engine 切分文件**：切分产物写入 TempDir（方案 A：先 fs::copy 原始模型到 TempDir，再 Split），避免污染 `test_model/` 目录
5. **ML Engine tokenizer**：切分后的 `.pgguf` 文件不含 tokenizer metadata（被 `GGUF_Split_Model` 清空），`shimmytok::from_gguf_file()` 返回 None。前半模型无法 Encode，但 Create_Session 不受影响
6. **ML Engine block_count**：切分文件保留原始 `block_count=28`，仅通过 `pleiades.split.start/end` 标记实际包含的层范围
7. **Config 模块**：涉及文件系统 `.config/` 目录创建，可在 t01 中附带测试或独立创建
8. **test 并发**：Rust 默认多线程并行执行测试，注意测试间资源隔离（每个测试使用独立 TempDir）
9. **Compiler 状态**：`Compiler.compile_run()` 已实现（单机推理），`compile_coordinator()` 和 `compile_relay()` 仍为 `todo!()`。t06 TC-02/03/05 通过直接构造 `JobExecutor` 绕过。D 组 t08 经过 Core route_user → compile_run 完整路径
10. **Bus_Event 变体数**：当前 `Bus_Event` 枚举包含 **14 个** variant（非 16 个），TC-03 覆盖全部 14 个
11. **PeerManagement 心跳超时**：`is_timeout()` 使用 `elapsed().as_secs() >= timeout_secs`，timeout_secs=0 会清除所有节点（含刚更新的），测试使用 sleep + timeout_secs=1 方案

---

12. **D 组双 StorageManager**：`ML_Engine_Service::New()` 需要 `Arc<StorageManager>`，而 `Capabilities.storage` 是 `StorageManager`（非 Arc）。D 组测试需创建两个 `StorageManager` 指向同一 TempDir，各自独立管理锁索引。这对测试场景无影响（不存在跨 StorageManager 的锁竞争）
13. **D 组 Token 收集**：`Core::handle_lifecycle_event` 当前不调用 `io_broker.Deallocate(job_id)`（Gap 8），Broker 的 `_ml_output_tx` Sender 克隆不会 drop，`frontend.output_rx` 不会自然关闭。D 组测试使用 timeout 收集模式（推理完成后无新 token，timeout 结束收集），不依赖通道关闭信号
14. **UserCommand oneshot reply**：`UserCommand` 不再实现 `Clone`（`oneshot::Sender` 不可 Clone），每次发送需创建 `oneshot::channel()` 包裹 reply_tx

---

## 10. 实施进度

| 文件 | 测试数 | 状态 |
|------|--------|------|
| common/mod.rs | — | ✅ 已完成 |
| t01_storage_integration.rs | 5 | ✅ 已完成 |
| t02_llm_io_integration.rs | 5 | ✅ 已完成 |
| t03_event_bus_integration.rs | 4 | ✅ 已完成 |
| t04_peer_management_integration.rs | 6 | ✅ 已完成 |
| t05_ml_engine_integration.rs | 8 | 📋 待实施（需真实模型） |
| t06_orchestrator_integration.rs | 5 | 📋 待实施 |
| t07_network_integration.rs | 3 | 📋 待实施（需真实 TCP） |
| t08_e2e_inference.rs | 2 | 📋 待实施（需真实模型，经 Core 全链路） |
