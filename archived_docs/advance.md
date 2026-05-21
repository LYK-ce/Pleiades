# Orchestrator 架构讨论与未来演进方案

**日期**：2026-04-28  
**参与者**：KeJi + Agent  
**基线**：Phase 2B 完成，orchestrator_implementation.md 节点

---

## 1. 当前通信架构确认

### 1.1 核心结论：只有 Core↔Core 通信，没有 Job↔Job 通信

当前系统采用 **星型拓扑（Star Topology）**，Core 是唯一的通信枢纽，Job 是完全隔离的执行单元。

| 通信维度 | 路径 | 是否存在 |
|---------|------|---------|
| Core ↔ Core（跨节点） | `NetworkCommand` / `Network_Inbound_Event` / libp2p | ✅ |
| Core → Job（本地） | `spawn_job()` + `CancellationToken` | ✅ |
| Job → Core（本地） | `lifecycle_tx` (`LifecycleEvent::Done`) | ✅ |
| **Job ↔ Job（本地或跨节点）** | 无 | ❌ |

### 1.2 Job 间数据传递的实际路径

尽管 Job 之间没有直接通道，但 Job 可以通过共享的 `Arc<Capabilities>` 间接与外部交互：

- **控制面（Request-Response）**：Job 通过 `capabilities.network.send_data().await` 发起请求，远端 Core 的 Control 层回复。Job 同步获取结果。
- **数据面（Tensor Stream）**：
  - 出站：Job 通过 `capabilities.network.open_tensor_stream()` 主动建立
  - 入站：Core 收到 `TensorStreamArrived` → `Tensor_IO_Broker.Store_Inbound()` → Job 通过 `Take_Inbound().await` 阻塞获取

### 1.3 SendFile 场景的 Request-Response 机制

发起 SendFile 的 Job 通过以下调用链**同步**获取对方的 accept/reject 结果：

```
Job (handle_send_file)
  → capabilities.network.send_data().await  (Network_Capability trait)
    → NodeHandle::Send_Data().await         (创建 oneshot channel)
      → cmd_tx.send(SendData { response_tx: Some(tx) })  (发给 Network_Service)
        → Network_Service → libp2p Request-Response → 远端节点
        → 远端 Control 层回复 → libp2p 返回 Response
      → response_tx.send(Ok(response))      (Network_Service 回填 oneshot)
    → rx.await (30s 超时)                   (NodeHandle 等待回填)
  → Ok(response)                            (Job 拿到远端响应)
```

**关键**：远端回复 ACCEPT/REJECT 的是 Core/Control 层，不是 Job。接收端的 Job 在元数据协商之后才会被 spawn。

---

## 2. 是否需要 Core→Job 通信通道

### 2.1 当前的结论：暂不需要

当前所有场景都不需要 Core→Job 通用通道：

- 控制面：Job 主动 `.await` 拉取（拉模型）
- 数据面：`Tensor_IO_Broker` + `Notify` 专用推送（比通用通道更精确）
- 入站请求：由 Core/Control 层处理，不需要路由到 Job

### 2.2 未来可能需要的触发条件

- **运行时动态重配置**：Peer 上/下线，需要通知运行中的 Job 切换下游节点
- **Job 间协作**：同一节点上多个 Job 共享中间结果
- **请求路由到 Job**：远端请求需要由特定 Job 做业务决策

### 2.3 后续加入 Core→Job 通道的改造评估

**难度：低**。只需修改 3 个文件（`executor/mod.rs`、`core.rs`、`job.rs`），约 10 行代码。所有 handler、TaskEngine、指令集、Compiler 完全不受影响。

推荐的消息接收方式是 **步间非阻塞检查**（`try_recv()`），不要放进 `select!` 第三分支（会中断正在执行的 step）：

```rust
// executor/mod.rs — run() 循环
loop {
    while let Ok(msg) = self.core_rx.try_recv() {
        self.handle_core_message(msg);
    }
    tokio::select! {
        biased;
        _ = self.cancel.cancelled() => { ... }
        result = self.task_engine.step() => { ... }
    }
}
```

---

## 3. 动态重配置的两种演进方案

### 3.1 方案 1：热替换程序（Program Hot-Swap）— 推荐

**原理**：Core 编译新的 `TaskProgram`，通过 Core→Job 通道注入。Job 替换程序但**保留 SlotFile 状态**。

**与当前架构的契合度极高**：`TaskEngine` 天然分离 `program`（程序）和 `slots`（状态），新增 `reload()` 方法仅需 3 行代码。

**执行流程**：
```
Core 检测到 Peer 离线
  → Core 编译恢复程序 compile_recovery(new_peer, slot_conventions)
  → Core 发送 CoreToJobMessage::ReloadProgram(new_program, injections)
  → Job 等当前 step 完成
  → Job 将 injections 注入 SlotFile（如新的 IoHandle）
  → Job 调用 task_engine.reload(new_program)
  → TaskEngine: 替换 program、重置 ip、保留 slots
  → Job 继续线性执行新程序（ShutdownSession → OpenTensorStream → CreateSession → RunProgram）
```

**改动清单**：

| 组件 | 改动 | 难度 |
|------|------|------|
| `task_engine.rs` | 新增 `reload()` 方法 | ⭐ |
| `executor/mod.rs` | 加 `core_rx` + `try_recv()` + `handle_core_message()` | ⭐ |
| `core.rs` | `spawn_job` 创建通道，`JobHandle` 保存 `core_tx` | ⭐ |
| `compiler.rs` | 新增 `compile_recovery()` | ⭐⭐⭐ |
| handler 及其他 | 无改动 | ✅ |

**优势**：
- Job 身份（job_id）不变，前端 IoHandle 不断，Broker 条目不需迁移
- SlotFile 原地保留，旧 Slot 值（session_id、model 路径等）可直接引用
- ML Session 生命周期清晰（同一 Job 内先 Shutdown 再 Create）

**需解决的问题**：
- IoHandle 已被 CreateSession take 消费，新程序需要新 IoHandle → 由 `CoreToJobMessage` 携带注入
- RunProgram 是长时间阻塞指令，消息只能在 step 间处理 → 可接受（下游断开会导致 IO 错误自然中断）

### 3.2 方案 2：新 Job 继承遗产（Legacy Inheritance）

**原理**：Cancel 旧 Job，旧 Job 退出时打包 SlotFile，Core 将遗产注入新 Job。

**执行流程**：
```
Core 检测到 Peer 离线
  → Core 向旧 Job 发送特殊 CancellationToken（遗赠模式）
  → 旧 Job 跳过补偿（不 ShutdownSession），打包 SlotFile
  → 旧 Job 上报 LifecycleEvent::Done { legacy: Some(SlotFile) }
  → Core 编译延续程序 compile_continuation(new_peer, legacy)
  → Core spawn 新 Job，SlotFile 预加载遗产
  → 新 Job 执行恢复指令
```

**改动清单**：

| 组件 | 改动 | 难度 |
|------|------|------|
| `job.rs` | `LifecycleEvent::Done` 新增 `legacy` 字段 | ⭐ |
| `executor/mod.rs` | 区分正常取消 vs 遗赠取消，run() 返回 `Option<SlotFile>` | ⭐⭐ |
| `core.rs` | 处理 legacy、改变 spawn 模型（JoinHandle await） | ⭐⭐ |
| `slot.rs` | SlotFile 支持 merge 或预加载 | ⭐ |
| `compiler.rs` | 新增 `compile_continuation()` | ⭐⭐⭐ |

**劣势**（相比方案 1）：
- job_id 变化，前端需重新连接
- SlotValue 不可 Clone，需要 move 所有权，改变 `run()` 签名
- Core 的 spawn 模型从 fire-and-forget 变为需要 await JoinHandle
- 跨 Job 的 ML Session 继承存在状态不确定性

### 3.3 方案对比总结

| 维度 | 方案 1（热替换） | 方案 2（继承） |
|------|----------------|--------------|
| Job 身份 | 不变 ✅ | 变化 ❌ |
| SlotFile 转移 | 不需要 ✅ | 需要 move 所有权 ❌ |
| Core spawn 模型 | 不变 ✅ | 需改为 JoinHandle ❌ |
| ML Session 连续性 | 自然连续 ✅ | 跨 Job 继承复杂 ❌ |
| IoHandle 问题 | CoreToJobMessage 携带 | 同样存在 |
| Compiler 复杂度 | 高 | 高 |
| 总改动量 | 较小 ✅ | 较大 ❌ |

**结论：方案 1（热替换程序）更优**。

---

## 4. 核心设计原则

### 4.1 Job 是无情的命令解释器

```
Core = 大脑（决策、编排、路由）
Job  = 四肢（无条件执行指令序列）
```

无论采用哪种演进方案，Job 自身**永远不做判断**。它不知道"为什么"程序变了，也不关心。所有复杂的策略逻辑集中在 Core + Compiler，不散落在 handler 里。

类比：Job 就是 CPU，Core 就是操作系统，Compiler 就是编译器。CPU 不决定执行什么程序——操作系统决定。

### 4.2 分支基础设施已就绪

当前指令系统已内置分支能力，暂时未被 Compiler 使用：

- `JumpIf`：读取槽位布尔值，为真则跳转到 `labels[label]`
- `TaskProgram.labels`：`HashMap<String, usize>` 标签→指令索引映射
- `TaskEngine.ip`：程序计数器，`JumpIf` 可直接覆盖

未来 Compiler 生成带检查点的程序时可直接复用，无需修改 TaskEngine。

### 4.3 渐进式演进策略

```
当前 → 完成 compile_coordinator/relay 的线性版本
     → 方案 C 兜底（Peer 崩溃 → Cancel + 重启 Job）
     
短期 → 加入 Core→Job 通道 + task_engine.reload()（约 10 行代码）
     → Compiler 支持 compile_recovery()

中期 → 如需 RunProgram 中断响应，引入 StepResult::LongRunning
```

**建议：现在不改，保持线性执行模型**。等 `compile_coordinator` 的线性版本跑通后，再根据实际需求决定演进路径。改造难度极低，不存在架构锁定风险。
