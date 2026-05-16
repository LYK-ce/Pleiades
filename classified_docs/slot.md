# Slot 模块设计文档

**定位**：Orchestrator 层的**显式状态容器**。所有跨步骤的资源、配置与中间结果，必须通过 SlotFile 传递，禁止隐式全局变量。

---

## 1. 模块归属

`pleiades::orchestrator::slot`

- `SlotId` 与 `SlotValue` 是 Orchestrator 层的**公共词汇**，Compiler 生成指令、TaskEngine 解释执行、Capability 消费句柄，均依赖此模块。
- `SlotFile` 由 `TaskEngine` 私有持有，每个 `JobExecutor` 一份，Job 结束后随 Drop 自动清理。

---

## 2. 核心结构

| 类型 | 说明 |
|------|------|
| `SlotId(u32)` | 槽位编号，类似寄存器索引。由 `TaskProgramBuilder` 在编译时分配。 |
| `SlotValue` | 枚举，封装所有可存入槽位的类型。 |
| `SlotFile` | `HashMap&lt;SlotId, SlotValue&gt;` 的包装，提供类型安全的访问方法。 |

---

## 3. SlotValue 变体（Phase 2）

采用**显式枚举**，编译期类型明确，调试友好。

| 变体 | 用途 |
|------|------|
| `Nil` | 空槽位，初始状态或 `take` 后的残留 |
| `Bool(bool)` | 控制流条件（`JumpIf`） |
| `U64(u64)` | 数值参数（如 `max_tokens`） |
| `String(String)` | 设备名、错误描述 |
| `PathBuf(PathBuf)` | 模型路径、文件位置 |
| `DeviceLease(...)` | 计算设备租约（RAII，Phase 2 先用 Stub） |
| `SessionHandle(...)` | ML Session 句柄（Phase 2 先用 Stub） |
| `Error(String)` | 步骤执行失败的错误传递 |

**原则**：类型数量可控，不引入 `dyn Any`。未来新增句柄类型时，直接扩展枚举变体。

---

## 4. SlotFile 方法

### 4.1 通用操作

| 方法 | 语义 |
|------|------|
| `set(slot, value)` | 写入或覆盖槽位。旧值自然 `Drop`，RAII 资源自动释放。 |
| `get(slot) -&gt; Option&lt;&SlotValue&gt;` | 只读引用，不转移所有权。 |
| `take(slot) -&gt; Option&lt;SlotValue&gt;` | 取出所有权，原槽位置 `Nil`。 |
| `remove(slot) -&gt; Option&lt;SlotValue&gt;` | 删除槽位，返回旧值。 |

### 4.2 类型安全便利方法

针对 Phase 2 高频场景，提供专用访问接口，避免调用方手动 `match`：

| 方法 | 场景 |
|------|------|
| `get_string(slot) -&gt; Result&lt;&String, String&gt;` | 读取设备名、路径字符串 |
| `get_path(slot) -&gt; Result&lt;&PathBuf, String&gt;` | 读取模型路径 |
| `get_bool(slot) -&gt; Result&lt;bool, String&gt;` | 读取 `JumpIf` 条件 |
| `take_device(slot) -&gt; Result&lt;DeviceLease, String&gt;` | 移交设备租约给 `CreateSession` |
| `take_session(slot) -&gt; Result&lt;SessionHandle, String&gt;` | 移交 Session 给 `ShutdownSession` |

类型不匹配时返回 `Err(String)`，由 `TaskEngine` 捕获并进入 `Abort`。

---

## 5. 设计约束

| 红线 | 说明 |
|------|------|
| **显式化** | 所有跨步骤状态必须在 SlotFile 中声明，禁止隐式全局变量 |
| **所有权明确** | `set` 覆盖时旧值 `Drop`；`take` 后原槽位变 `Nil` |
| **类型校验** | 运行时检查，不匹配即 `Abort`，不静默转换 |
| **Job 隔离** | 每个 JobExecutor 独立 `SlotFile`，Job 结束后全量 Drop |
| **不感知业务** | `SlotFile` 只存数据，不解释语义（语义由 `TaskInstruction` 定义） |