# Instruction 模块设计文档

**定位**：Orchestrator 层的**指令契约**。定义 Compiler 生成、TaskEngine 解释执行的所有任务指令类型。纯数据定义，无运行时逻辑。

---

## 1. 模块归属

`pleiades::orchestrator::instruction`

- 由 **Compiler** 生成 `TaskProgram` 时填充指令序列。
- 由 **TaskEngine** 加载后逐条解释执行。
- 作为公共模块，供 Compiler 和 Executor 平等依赖。

---

## 2. 设计原则

- **业务即数据**：作业流程不是硬编码函数，而是编译后的指令序列。
- **纯数据定义**：`instruction.rs` 只包含枚举和结构体定义，不实现任何执行逻辑。
- **最小够用**：Phase 2 仅定义 9 条基础指令，覆盖数据搬运、资源申请、会话管理、存储隔离与基础控制流。

---

## 3. Phase 2 指令集

### 3.1 数据搬运指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `Const { value, dst }` | `value: SlotValue`, `dst: SlotId` | 将常量值写入目标槽位 |
| `Move { src, dst }` | `src: SlotId`, `dst: SlotId` | 从源槽位取出值，写入目标槽位 |

### 3.2 存储资源指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `EnsureWorkspace { job_id }` | `job_id: JobId` | 为指定作业创建隔离工作目录。幂等，已存在时不报错 |
| `CleanupWorkspace { job_id }` | `job_id: JobId` | 清理指定作业的隔离工作目录及临时文件 |

### 3.3 计算资源指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `AcquireDevice { preferred, result }` | `preferred: SlotId`, `result: SlotId` | 从 `preferred` 槽位读取设备偏好字符串，向 ComputeManager 申请租约，结果写入 `result` 槽位。Phase 2 占位返回空 Stub |

### 3.4 推理会话指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `CreateSession { model, device, result }` | `model: SlotId`, `device: SlotId`, `result: SlotId` | 从 `model` 槽位读取模型路径，从 `device` 槽位 **take** 设备租约，创建 ML Session，句柄写入 `result` 槽位 |
| `ShutdownSession { session }` | `session: SlotId` | 从 `session` 槽位 **take** Session 句柄，调用 Inference Capability 关闭 |

### 3.5 控制流指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `JumpIf { condition, label }` | `condition: SlotId`, `label: String` | 从 `condition` 槽位读取布尔值。为真时，指令指针跳转到 `TaskProgram.labels` 中对应标签的索引 |
| `Abort { reason }` | `reason: String` | 立即终止正向执行，触发补偿链 |

---

## 4. 程序容器

```rust
TaskProgram {
    instructions: Vec&lt;TaskInstruction&gt;,   // 正向执行序列
    compensation: Vec&lt;TaskInstruction&gt;,    // 补偿/清理序列（Cancel 或 Abort 时执行）
    labels: HashMap&lt;String, usize&gt;,        // 标签名 → 指令索引映射
}