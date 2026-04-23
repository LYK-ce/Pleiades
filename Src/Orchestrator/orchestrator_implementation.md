# Orchestrator 后续实现计划

**日期**：2026-04-23  
**基线**：Phase 2 中段 — Core spawn_job 已验证，63 个测试全部通过

---

## 1. 当前已完成清单

| 模块 | 文件 | 状态 |
|------|------|------|
| Core 主循环 | `core.rs` | ✅ run() + select! + lifecycle 通道 |
| Job 公共契约 | `job.rs` | ✅ JobId/JobKind/JobState/LifecycleEvent |
| Command 定义 | `command.rs` | ✅ UserCommand/NetworkCommand |
| Slot 系统 | `slot.rs` | ✅ SlotId/SlotValue/SlotFile（缺 IoHandle 变体） |
| 指令集 | `instruction.rs` | ✅ 7 条指令（CreateSession 缺 io 参数） |
| TaskEngine | `executor/task_engine.rs` | ✅ step() + ExecutionMode + 5 个测试 |
| JobExecutor | `executor/mod.rs` | ✅ run() + 补偿循环 + 3 个测试 |
| handler_data | `executor/handler_data.rs` | ✅ Const/Move + 4 个测试 |
| handler_control | `executor/handler_control.rs` | ✅ JumpIf/Abort + 5 个测试 |
| handler_compute | `executor/handler_compute.rs` | ⚠️ 空壳 |
| handler_inference | `executor/handler_inference.rs` | ⚠️ 空壳 |
| Compiler | `compiler.rs` | ⚠️ 骨架 + TaskProgramBuilder，compile_run/compile_worker_relay 为 todo!() |
| Core spawn 测试 | `core.rs#core_tests` | ✅ 4 个测试（spawn/abort/compile_error/multi_job） |

**外部依赖模块**：
| 模块 | 状态 |
|------|------|
| StorageManager | ✅ 已完成 |
| LLM_IO（Broker + Capability） | ✅ 已完成并集成 |

---

## 2. 实施路线图

### Phase 2A：IO 通道打通（必须先行）

**目标**：让 IoHandle 能通过指令系统流入 ML Thread，完成数据面打通。

#### 步骤 1：SlotValue 扩展
- **文件**：`slot.rs`
- **改动**：
  - `SlotValue` 枚举新增 `IoHandle(crate::llm_io::IoHandle)` 变体
  - `SlotFile` 新增 `take_io_handle(slot: SlotId) -> Result<IoHandle, String>` 方法
- **注意**：`IoHandle` 不实现 Clone（含 mpsc::Receiver），只能 take，不能 get

#### 步骤 2：CreateSession 指令扩展
- **文件**：`instruction.rs`
- **改动**：`CreateSession` 新增 `io: SlotId` 参数
  ```rust
  CreateSession {
      model: SlotId,
      device: SlotId,
      io: SlotId,       // ← 新增
      result: SlotId,
  }
  ```
- **联动**：
  - `task_engine.rs` step() 的 match 分支需传递 `io` 参数
  - `handler_inference.rs` 的 `handle_create_session` 签名同步更新

#### 步骤 3：InferenceCapability trait 扩展
- **文件**：`executor/mod.rs`
- **改动**：`create_session` 签名增加 `io: crate::llm_io::IoHandle`
  ```rust
  async fn create_session(
      &self, model: &str, dev: DeviceLease, io: IoHandle
  ) -> Result<SessionHandle, String>;
  ```

#### 步骤 4：JobExecutor 初始化时注入 IoHandle 到 SlotFile
- **文件**：`executor/mod.rs`
- **改动**：`JobExecutor::new()` 中，将传入的 `io: IoHandle` 写入 TaskEngine 的 SlotFile 的约定槽位（如 SlotId(0)）
- **设计**：使用固定槽位 ID 约定，Compiler 编译时引用相同的槽位

#### 步骤 5：更新所有测试 Stub
- `handler_inference.rs` 测试中 StubInference 更新签名
- `executor/mod.rs` executor_tests 中 StubInference 更新签名
- `core.rs` core_tests 中 StubInference 更新签名
- `task_engine.rs` 测试中更新 CreateSession 指令构造

---

### Phase 2B：Handler 填充

**目标**：让 AcquireDevice / CreateSession / ShutdownSession 具备真实调用 Capability 的逻辑。

#### 步骤 6：handler_compute.rs — AcquireDevice
- **当前**：直接返回 `StepResult::Continue`
- **目标实现**：
  1. 从 `preferred` 槽位读取 `Option<String>`（Nil → None）
  2. 调用 `self.capabilities.compute.acquire_device(pref).await`
  3. 成功 → `SlotValue::DeviceLease(lease)` 写入 `result` 槽位，返回 Continue
  4. 失败 → 返回 `StepResult::Abort(error)`
- **改为 async**：handler 方法签名改为 `async fn`，task_engine.rs match 分支加 `.await`
- **测试**：使用 StubCompute（成功/失败两种），验证槽位写入和 Abort 行为

#### 步骤 7：handler_inference.rs — CreateSession
- **目标实现**：
  1. 从 `model` 槽位 `get_string` 获取模型路径
  2. 从 `device` 槽位 `take_device` 获取 DeviceLease
  3. 从 `io` 槽位 `take_io_handle` 获取 IoHandle
  4. 调用 `self.capabilities.inference.create_session(model, dev, io).await`
  5. 成功 → `SlotValue::SessionHandle(handle)` 写入 `result` 槽位
  6. 失败 → 返回 Abort
- **测试**：验证正常创建、模型路径缺失、设备租约缺失、IO 句柄缺失等边界

#### 步骤 8：handler_inference.rs — ShutdownSession
- **目标实现**：
  1. 从 `session` 槽位 `take_session` 获取 SessionHandle
  2. 调用 `self.capabilities.inference.shutdown_session(sess).await`
  3. 返回 Continue（即使失败也尽力清理，不 Abort）
- **测试**：正常关闭、空槽位处理

---

### Phase 2C：Compiler 完整实现

**目标**：compile_run / compile_worker_relay 生成可执行的 TaskProgram。

#### 步骤 9：compile_run 实现
- **生成的指令序列**（Run 作业）：
  ```
  正向序列：
    Const { value: String(model_path),  dst: SLOT_MODEL }
    Const { value: String(device_pref), dst: SLOT_PREF }     // 可选
    AcquireDevice { preferred: SLOT_PREF, result: SLOT_DEV }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEV, io: SLOT_IO, result: SLOT_SESSION }
    // Phase 3: 更多推理相关指令...
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```
- **约定槽位**：SLOT_IO = 0（由 JobExecutor 初始化时写入）

#### 步骤 10：compile_worker_relay 实现
- **生成的指令序列**（WorkerRelay 作业）：
  ```
  正向序列：
    Const { value: String(peer_id),     dst: SLOT_PEER }
    Const { value: String(device_pref), dst: SLOT_PREF }
    AcquireDevice { preferred: SLOT_PREF, result: SLOT_DEV }
    CreateSession { model: SLOT_MODEL, device: SLOT_DEV, io: SLOT_IO, result: SLOT_SESSION }
    // Phase 3: ReceiveFile + PrepareTensorStreams...
  
  补偿序列：
    ShutdownSession { session: SLOT_SESSION }
  ```

#### 步骤 11：Compiler 单元测试
- 验证生成的 TaskProgram 指令数量、类型、槽位连接正确性
- 验证补偿序列包含正确的清理指令
- 验证参数校验（空路径、空 peer_id）

---

### Phase 2D：Core 端到端集成测试

**目标**：通过 UserCommand 触发完整 compile → spawn → execute → Done 流程。

#### 步骤 12：Core::run() 端到端测试
- **测试 1**（Run 作业端到端）：
  1. 创建 Core（使用 Stub Capabilities）
  2. 通过 `user_cmd_tx` 发送 `UserCommand::Run { model_path }`
  3. Core::run() 内部 compile_run → spawn_job → executor 执行 → Done
  4. 发送 `UserCommand::Quit`
  5. Core::run() 退出
  6. 验证 registry 为空

- **测试 2**（Cancel 作业）：
  1. 使用阻塞型 StubInference（在 create_session 中等待 Notify）
  2. 发送 Run 命令，Job 阻塞在 CreateSession
  3. 发送 Cancel 命令
  4. 验证收到 Cancelled 结果

- **测试 3**（Quit 优雅退出）：
  1. spawn 多个 Job
  2. 发送 Quit → 触发 shutdown → 所有 Job cancel
  3. 验证 Core::run() 正常退出

---

## 3. 依赖图

```
步骤 1 (SlotValue +IoHandle)
    ↓
步骤 2 (CreateSession +io)  ←  步骤 3 (InferenceCapability +io)
    ↓
步骤 4 (JobExecutor 注入 IoHandle)
    ↓
步骤 5 (测试 Stub 更新)
    ↓
┌───────────────────┐
│  Phase 2A 完成     │
└───────┬───────────┘
        ↓
步骤 6 (handler_compute)
步骤 7 (handler_inference: CreateSession)  ← 可并行
步骤 8 (handler_inference: ShutdownSession)
        ↓
┌───────────────────┐
│  Phase 2B 完成     │
└───────┬───────────┘
        ↓
步骤 9  (compile_run)
步骤 10 (compile_worker_relay)  ← 可并行
步骤 11 (Compiler 测试)
        ↓
┌───────────────────┐
│  Phase 2C 完成     │
└───────┬───────────┘
        ↓
步骤 12 (Core 端到端集成测试)
        ↓
┌───────────────────┐
│  Phase 2 完成      │
└───────────────────┘
```

---

## 4. 执行建议

1. **每步一个 task.md 条目**：保持"一步一测"节奏
2. **步骤 1–5 可合并为一个大任务**：它们相互紧耦合，拆开会导致大量编译不过的中间态
3. **步骤 6–8 的 async 化**：handler_compute 和 handler_inference 改为 async 后，task_engine.rs 的 match 分支也需要加 `.await`，这是一次性批量改动
4. **Compiler 使用常量槽位约定**：定义常量如 `const SLOT_IO: SlotId = SlotId(0)`，避免魔法数字

---

## 5. 风险与注意事项

| 风险 | 缓解措施 |
|------|---------|
| IoHandle 不可 Clone，SlotFile.set 需要所有权 | 使用 take 语义，每个 IoHandle 只能被一条指令消费一次 |
| handler 改为 async 后 TaskEngine::step 签名不变（已是 async） | 只需在 match 分支加 .await，不影响上层 |
| Compiler 生成的 SlotId 必须与 JobExecutor 初始化的 SlotId 对齐 | 使用 compiler.rs 中定义的全局常量 |
| Cancel 测试时序问题 | Phase 2D 使用 Notify 阻塞的 Stub 保证确定性 |
| SlotValue::IoHandle 无法 Clone | SlotValue 的 #[derive(Clone)] 需要移除或条件处理 |

### 5.1 SlotValue Clone 问题（重要）

当前 `SlotValue` 派生了 `#[derive(Debug, Clone)]`。`IoHandle` 内含 `mpsc::Receiver`，**不可 Clone**。有两种解决方案：

**方案 A**：移除 `SlotValue` 的 `#[derive(Clone)]`，改为手动实现或不实现 Clone。
- 影响：`TaskInstruction::Const { value: SlotValue, .. }` 的 `Clone` 也会受影响
- 联动：`TaskInstruction` 和 `TaskProgram` 的 Clone 需要处理

**方案 B**：IoHandle 不放入 `SlotValue`，改为 TaskEngine 的独立字段。
- 优点：不破坏现有 Clone 链
- 缺点：破坏"所有跨步骤状态必须在 SlotFile 显式化"红线

**推荐方案 A**：因为红线明确要求所有状态在 SlotFile 中。处理方式：
- `SlotValue` 移除 `#[derive(Clone)]`
- `TaskInstruction::Const` 的 value 改为 `SlotValue` 保持所有权转移语义
- `TaskProgram` 的 `instructions` 改为不可 clone（load 时 take 所有权而非 clone）
- 或者将 `Const` 指令的 value 类型限制为可 Clone 的子集（新建 `ConstValue` 枚举，不含 IoHandle）

具体方案在步骤 1 实施时确定。
