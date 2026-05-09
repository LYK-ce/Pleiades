
# Core Instruction Handler 设计文档

**定位**：`TaskEngine` 的**指令解释实现层**。将 `step()` 的 `match` 分发逻辑与各类指令的具体处理分离，避免 `task_engine.rs` 膨胀。

---

## 1. 目录结构（平铺）

所有文件直接放在 `executor/` 目录下，**不嵌套子目录**：

```
src/orchestrator/executor/
├── mod.rs                  // JobExecutor
├── task_engine.rs          // TaskEngine 结构体 + step() match 骨架
├── handler_data.rs         // Const, Move
├── handler_storage.rs      // EnsureWorkspace, CleanupWorkspace
├── handler_compute.rs      // AcquireDevice
├── handler_inference.rs    // CreateSession, ShutdownSession
└── handler_control.rs      // JumpIf, Abort
```

**平铺理由**：
- 子目录会增加 `use` 路径层级（`handlers::compute` vs `handler_compute`），平铺更直接。
- Phase 2 每类指令数量可控（2-3 条），不需要再按目录聚类。
- 编译单元清晰：改 `handler_inference.rs` 不会触发 `handler_storage.rs` 重编译。

---

## 2. 与 TaskEngine 的关系

**分散 `impl TaskEngine`**。

`task_engine.rs` 只保留结构体定义、`step()` 的 `match` 分发骨架、以及 `ExecutionMode` 等内部状态。

各 `handler_*.rs` 文件通过 `impl TaskEngine` 块，为 TaskEngine 添加**私有方法**：

```rust
// handler_data.rs
impl TaskEngine {
    pub(super) fn handle_const(&mut self, value: &SlotValue, dst: SlotId) -> StepResult {
        self.slots.set(dst, value.clone());
        StepResult::Continue
    }
}
```

**可见性**：所有 `handle_*` 方法为 `pub(super)`，仅 `executor` 模块内部（即 `task_engine.rs`）可调用，外部不可见。

---

## 3. 各文件职责

| 文件 | 指令 | 职责 |
|------|------|------|
| `handler_data.rs` | `Const`, `Move` | SlotFile 基础读写 |
| `handler_storage.rs` | `EnsureWorkspace`, `CleanupWorkspace` | StorageManager 交互 |
| `handler_compute.rs` | `AcquireDevice` | ComputeManager 交互 |
| `handler_inference.rs` | `CreateSession`, `ShutdownSession` | Inference Capability 交互 |
| `handler_control.rs` | `JumpIf`, `Abort` | 控制流与错误终止 |

---

## 4. 关键约束

- **不持有独立状态**：各 handler 文件只包含 `impl TaskEngine` 的方法，**不定义新结构体**。状态统一由 `TaskEngine` 的 `slots`、`ip`、`program` 承载。
- **不直接 spawn task**：handler 内部的长时操作（如 Capability 调用）必须是**可 await 的纯 Future**，禁止 fire-and-forget。
- **错误即 Abort**：handler 内部任何 Slot 类型不匹配或 Capability 返回 `Err`，直接返回 `StepResult::Abort`，由 `task_engine.rs` 的 `step()`  caller 触发补偿链。
- **不感知 Cancel**：handler 只负责单条指令的执行，Cancel 信号由 `JobExecutor` 的 `select!` 捕获，不传入 handler。

---

## 5. 边界红线

- **禁止在 handler 中修改 `ip`**（`JumpIf` 除外）。`step()` 骨架负责 `ip += 1`，handler 只读取当前指令参数。
- **禁止在 handler 中访问 `ExecutionMode`**。handler 不知道当前执行的是正向序列还是补偿序列。
- **禁止跨 handler 文件互相引用**。每个文件只操作 `TaskEngine` 的公共字段，文件之间零依赖。
```