# Presented by KeJi
# Date ： 2026-04-22

# Orchestrator 测试命令说明

## 运行所有 Orchestrator 测试

```bash
cargo test orchestrator -- --nocapture
```

---

## 分模块测试命令

### 1. TaskEngine 基础测试

```bash
cargo test task_engine::tests -- --nocapture
```

**测试内容：**
| 测试函数 | 说明 |
|---------|------|
| `unloaded_engine_aborts` | 未加载程序时 step() 返回 Abort |
| `empty_program_done` | 空程序立即返回 Done |
| `single_instruction_then_done` | 单条指令执行后返回 Done |
| `compensation_mode_switch` | 验证正向执行 → Abort → 补偿模式切换 |
| `multi_step_natural_finish` | 多步骤正常执行完成 |

---

### 2. Handler Data 测试（Const/Move 指令）

```bash
cargo test handler_data::tests -- --nocapture
```

**测试内容：**
| 测试函数 | 说明 |
|---------|------|
| `test_const_write_and_read` | TC-01: Const 写入后读取，验证槽位值正确 |
| `test_const_overwrite` | TC-02: Const 覆盖旧值，验证新值替换旧值 |
| `test_move_success` | TC-03: Move 成功转移，源槽位变 Nil，目标槽位有值 |
| `test_move_from_empty_slot` | TC-04: Move 从空槽位返回 Abort，目标槽位不受影响 |

---

### 3. Handler Control 测试（JumpIf/Abort 指令）

```bash
cargo test handler_control::tests -- --nocapture
```

**测试内容：**
| 测试函数 | 说明 |
|---------|------|
| `tc01_jump_if_true_branch` | TC-01: JumpIf 真分支跳转，ip 被覆盖到标签目标位置 |
| `tc02_jump_if_false_branch` | TC-02: JumpIf 假分支顺序执行，ip 不被覆盖 |
| `tc03_jump_if_label_not_found` | TC-03: JumpIf 标签不存在返回 Abort |
| `tc04_jump_if_type_mismatch` | TC-04: JumpIf 槽位类型不匹配返回 Abort |
| `tc05_abort_returns_reason` | TC-05: Abort 返回原始 reason 字符串 |

---

### 4. Handler Storage 测试（EnsureWorkspace/CleanupWorkspace 指令）

```bash
cargo test handler_storage::tests -- --nocapture
```

**测试内容：**
| 测试函数 | 说明 |
|---------|------|
| `tc01_ensure_workspace_success` | TC-01: EnsureWorkspace 成功，返回 Continue，Stub 计数器+1 |
| `tc02_ensure_workspace_failure` | TC-02: EnsureWorkspace 失败，返回 Abort 携带错误信息 |
| `tc03_cleanup_workspace_success` | TC-03: CleanupWorkspace 成功，返回 Continue，Stub 计数器+1 |
| `tc04_cleanup_workspace_failure` | TC-04: CleanupWorkspace 失败，返回 Abort 携带错误信息 |

**Stub 特性：**
- `CountingStorage`: 带计数器的 Storage Stub
- `ensure_called/cleanup_called`: 调用次数计数
- `last_job_id`: 记录最后传入的 job_id
- `fail_ensure/fail_cleanup`: 控制模拟失败

---

### 5. JobExecutor 端到端测试

```bash
cargo test executor_tests -- --nocapture
```

**测试内容：**
| 测试函数 | 说明 |
|---------|------|
| `tc01_forward_execution_success` | TC-01: 正向执行自然结束，收到 Success 结果 |
| `tc02_abort_triggers_compensation` | TC-02: Abort 触发补偿链，收到 Failed 结果 |
| `tc03_cancel_signal_interrupts` | TC-03: Cancel 信号中断执行，收到 Cancelled 结果 |

---

## 快速验证命令

```bash
# 验证编译通过
cargo check -p pleiades

# 运行所有测试（简洁输出）
cargo test orchestrator

# 运行所有测试（详细输出）
cargo test orchestrator -- --nocapture

# 运行特定测试（支持模糊匹配）
cargo test handler_data -- --nocapture
cargo test handler_control -- --nocapture
cargo test handler_storage -- --nocapture
cargo test executor_tests -- --nocapture
```
