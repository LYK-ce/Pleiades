# handler_data.rs 完善与测试说明

## 一、完善内容

### Const 指令
将 `value` 的克隆写入 `dst` 槽位，覆盖原有值（旧值自然 Drop）。

### Move 指令
从 `src` 槽位 `take` 值（原槽位置 `Nil`），写入 `dst` 槽位。若 `src` 为空，返回 `Abort` 并携带错误信息。

## 二、测试编写

### 测试位置
写在 `handler_data.rs` 底部的 `#[cfg(test)] mod tests` 中。`handle_const` 与 `handle_move` 为 `pub(super)`，同模块内可直接调用。

### 测试用例

| 编号 | 场景 | 验证点 |
|------|------|--------|
| TC-01 | Const 写入后读取 | `slots.get(dst)` 返回原值 |
| TC-02 | Const 覆盖旧值 | 旧值被替换，新值正确 |
| TC-03 | Move 成功转移 | 源槽位变为 `Nil`，目标槽位有值 |
| TC-04 | Move 从空槽位 | 返回 `Abort`，目标槽位不受影响 |

### 测试方法
直接构造 `TaskEngine::new()`，调用 `engine.handle_const(...)` 或 `engine.handle_move(...)`，然后检查 `engine.slots` 的状态。无需任何 Capability Stub，因为这两个 Handler 不依赖外部能力。

## 三、运行命令

```bash
cargo test handler_data -- --nocapture
```

## 四、通过标准

- [ ] TC-01 ~ TC-02 通过：Const 读写与覆盖正确
- [ ] TC-03 ~ TC-04 通过：Move 转移与空槽错误处理正确


# handler_control.rs 完善与测试说明

## 一、完善内容

### JumpIf 指令
从 `condition` 槽位读取布尔值：
- **为真**：通过 `self.program.labels.get(label)` 查找目标索引，覆盖 `self.ip = target`。这是唯一允许修改 `ip` 的 handler。
- **为假**：不做任何操作，`ip` 保持默认的 `+1`（自然顺序执行下一条）。
- **槽位为空或类型不匹配**：返回 `Abort` 并携带错误信息。
- **标签不存在**：返回 `Abort` 并携带错误信息。

### Abort 指令
直接返回 `StepResult::Abort(reason.to_string())`，由 `step()` 的调用者触发补偿链。

## 二、测试编写

### 测试位置
写在 `handler_control.rs` 底部的 `#[cfg(test)] mod tests` 中。`handle_jump_if` 与 `handle_abort` 为 `pub(super)`，同模块内可直接调用。

### 测试用例

| 编号 | 场景 | 验证点 |
|------|------|--------|
| TC-01 | JumpIf 真分支跳转 | `ip` 被覆盖到标签目标位置，跳过中间指令 |
| TC-02 | JumpIf 假分支顺序执行 | `ip` 不被覆盖，自然执行下一条指令 |
| TC-03 | JumpIf 标签不存在 | 返回 `Abort`，`ip` 不被修改 |
| TC-04 | JumpIf 槽位类型不匹配 | 返回 `Abort`（如槽位是 `U64` 而非 `Bool`） |
| TC-05 | Abort 返回错误 | 返回 `Abort`，携带原始 `reason` 字符串 |

### 测试方法
直接构造 `TaskEngine::new()`，手动 `load()` 一个包含 `JumpIf` 或 `Abort` 的 `TaskProgram`，然后调用 `engine.step(&stub_caps())` 观察 `StepResult`。对于 JumpIf，需要在 `TaskProgram.labels` 中预设标签映射。

**注意**：`step()` 内部会先执行 `self.ip += 1`，然后才进入 `match` 分发到 `handle_jump_if`。因此测试时需要理解：`handle_jump_if` 看到的 `ip` 已经是 **+1 之后**的值。如果 `handle_jump_if` 决定跳转，它会**覆盖**这个 `ip`。

## 三、运行命令

```bash
cargo test handler_control -- --nocapture
```

## 四、通过标准

- [ ] TC-01 通过：真分支跳转正确，`ip` 被覆盖
- [ ] TC-02 通过：假分支不跳转，顺序执行
- [ ] TC-03 通过：缺失标签返回 `Abort`
- [ ] TC-04 通过：类型错误返回 `Abort`
- [ ] TC-05 通过：`Abort` 携带正确 reason


# handler_storage.rs 完善与测试说明

## 一、完善内容

### EnsureWorkspace 指令
调用 `capabilities.storage.ensure_workspace(job_id)`：
- 成功：返回 `StepResult::Continue`
- 失败：返回 `StepResult::Abort`，携带 StorageCapability 的错误信息

### CleanupWorkspace 指令
调用 `capabilities.storage.cleanup_workspace(job_id)`：
- 成功：返回 `StepResult::Continue`
- 失败：返回 `StepResult::Abort`，携带 StorageCapability 的错误信息

## 二、测试编写

### 测试位置
写在 `handler_storage.rs` 底部的 `#[cfg(test)] mod tests` 中。`handle_ensure_workspace` 与 `handle_cleanup_workspace` 为 `pub(super)`，同模块内可直接调用。

### Stub 策略
需要**带计数器的 Stub**，验证 Handler 确实调用了 Capability，且传参正确：

| Stub 字段 | 用途 |
|-----------|------|
| `ensure_called: AtomicUsize` | 记录 `ensure_workspace` 被调用次数 |
| `cleanup_called: AtomicUsize` | 记录 `cleanup_workspace` 被调用次数 |
| `last_job_id: Mutex<Option<JobId>>` | 记录最后一次传入的 `job_id` |
| `fail_ensure: bool` | 控制是否模拟失败 |
| `fail_cleanup: bool` | 控制是否模拟失败 |

### 测试用例

| 编号 | 场景 | 验证点 |
|------|------|--------|
| TC-01 | EnsureWorkspace 成功 | `Continue`，Stub 计数器 +1，`job_id` 正确 |
| TC-02 | EnsureWorkspace 失败 | `Abort`，错误信息包含原始错误，Stub 计数器 +1 |
| TC-03 | CleanupWorkspace 成功 | `Continue`，Stub 计数器 +1，`job_id` 正确 |
| TC-04 | CleanupWorkspace 失败 | `Abort`，错误信息包含原始错误，Stub 计数器 +1 |

### 测试方法
直接构造 `TaskEngine::new()`，调用 `engine.handle_ensure_workspace(job_id, &stub_caps)` 或 `engine.handle_cleanup_workspace(job_id, &stub_caps)`，然后检查 `StepResult` 和 Stub 计数器状态。

## 三、运行命令

```bash
cargo test handler_storage -- --nocapture
```

## 四、通过标准

- [ ] TC-01 通过：成功路径调用 Capability 正确
- [ ] TC-02 通过：错误路径正确传播为 `Abort`
- [ ] TC-03 通过：Cleanup 成功路径正确
- [ ] TC-04 通过：Cleanup 错误路径正确传播

## 五、后续工作

Stub 测试通过后，再实现真实的 `StorageManager`：
- 基于 `tokio::fs` 创建/删除目录
- 路径格式：`{base_dir}/job_{job_id}/`
- 幂等性：`create_dir_all` 已存在时不报错
- 引用计数（Phase 3）：多个 Job 共享缓存时的生命周期管理
