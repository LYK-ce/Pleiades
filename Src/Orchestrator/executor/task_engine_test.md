
# TaskEngine 骨架冒烟测试思路

## 前提

Handler 已实现**空壳版本**（如 `handle_jump_if` 返回 `Continue`、`handle_abort` 返回 `Abort(reason)` 等），`TaskEngine` 的 `match` 分发不会 panic。  
本阶段目标：**只验证 TaskEngine 自身的调度骨架能否运转**，不验证任何指令的业务语义。

## 测试文件位置

`src/orchestrator/executor/task_engine.rs` 底部 `#[cfg(test)]`

## Stub Capabilities

空壳 Handler 不调用 Capability，Stub 只需满足 trait 签名即可：

```rust
use async_trait::async_trait;

struct StubStorage;
#[async_trait]
impl StorageCapability for StubStorage {
    async fn ensure_workspace(&self, _job_id: JobId) -> Result<(), String> { Ok(()) }
    async fn cleanup_workspace(&self, _job_id: JobId) -> Result<(), String> { Ok(()) }
}

struct StubCompute;
#[async_trait]
impl ComputeCapability for StubCompute {
    async fn acquire_device(&self, _pref: Option<String>) -> Result<DeviceLease, String> {
        Ok(DeviceLease)
    }
}

struct StubInference;
#[async_trait]
impl InferenceCapability for StubInference {
    async fn create_session(&self, _model: &str, _dev: DeviceLease) -> Result<SessionHandle, String> {
        Ok(SessionHandle)
    }
    async fn shutdown_session(&self, _sess: SessionHandle) -> Result<(), String> { Ok(()) }
}

fn stub_caps() -> Capabilities {
    Capabilities {
        storage: Box::new(StubStorage),
        compute: Box::new(StubCompute),
        inference: Box::new(StubInference),
    }
}
```

## 测试用例（5 个）

### TC-01: 未加载程序 → Abort
验证 `step()` 在未 `load()` 时安全返回 `Abort`，不 panic。

```rust
#[tokio::test]
async fn unloaded_engine_aborts() {
    let mut engine = TaskEngine::new();
    let caps = stub_caps();
    assert!(matches!(engine.step(&caps).await, StepResult::Abort(_)));
}
```

### TC-02: 空程序 → Done
验证 `load()` 后空指令序列直接 `Done`。

```rust
#[tokio::test]
async fn empty_program_done() {
    let mut engine = TaskEngine::new();
    engine.load(TaskProgram {
        instructions: vec![],
        compensation: vec![],
        labels: HashMap::new(),
    });
    assert!(matches!(engine.step(&stub_caps()).await, StepResult::Done));
}
```

### TC-03: 单条指令 → 执行后再次 step → Done
验证 `ip` 递增机制：执行完最后一条后，再次 `step` 超出范围返回 `Done`。

```rust
#[tokio::test]
async fn single_instruction_then_done() {
    let mut engine = TaskEngine::new();
    engine.load(TaskProgram {
        instructions: vec![TaskInstruction::Abort { reason: "x".into() }],
        compensation: vec![],
        labels: HashMap::new(),
    });
    assert!(matches!(engine.step(&stub_caps()).await, StepResult::Abort(ref s) if s == "x"));
    assert!(matches!(engine.step(&stub_caps()).await, StepResult::Done));
}
```

### TC-04: 正向执行 → 进入补偿 → 补偿结束
验证 `enter_compensation()` 切换 `mode` 并重置 `ip = 0`，补偿序列独立执行。

```rust
#[tokio::test]
async fn compensation_mode_switch() {
    let mut engine = TaskEngine::new();
    let caps = stub_caps();
    engine.load(TaskProgram {
        instructions: vec![
            TaskInstruction::Const { value: SlotValue::Nil, dst: SlotId(0) },
            TaskInstruction::Abort { reason: "fail".into() },
        ],
        compensation: vec![
            TaskInstruction::Const { value: SlotValue::Nil, dst: SlotId(1) },
        ],
        labels: HashMap::new(),
    });

    assert!(matches!(engine.step(&caps).await, StepResult::Continue)); // Const
    assert!(matches!(engine.step(&caps).await, StepResult::Abort(_)));  // Abort

    engine.enter_compensation().await;

    assert!(matches!(engine.step(&caps).await, StepResult::Continue)); // 补偿 Const
    assert!(matches!(engine.step(&caps).await, StepResult::Done));     // 补偿结束
}
```

### TC-05: 多步正向自然走完 → Done
验证 `ip` 从 0 逐步递增到序列末尾，连续 `Continue` 后最终 `Done`。

```rust
#[tokio::test]
async fn multi_step_natural_finish() {
    let mut engine = TaskEngine::new();
    let caps = stub_caps();
    engine.load(TaskProgram {
        instructions: vec![
            TaskInstruction::Const { value: SlotValue::Nil, dst: SlotId(0) },
            TaskInstruction::Const { value: SlotValue::Nil, dst: SlotId(1) },
            TaskInstruction::Const { value: SlotValue::Nil, dst: SlotId(2) },
        ],
        compensation: vec![],
        labels: HashMap::new(),
    });

    assert!(matches!(engine.step(&caps).await, StepResult::Continue));
    assert!(matches!(engine.step(&caps).await, StepResult::Continue));
    assert!(matches!(engine.step(&caps).await, StepResult::Continue));
    assert!(matches!(engine.step(&caps).await, StepResult::Done));
}
```

## 运行命令

```bash
cargo test task_engine::tests -- --nocapture
```

## 通过标准

- [ ] `cargo test` 编译通过，不 panic
- [ ] TC-01 ~ TC-05 全部通过
- [ ] **不验证任何指令的业务逻辑**（如 Move 是否真拷贝、JumpIf 是否真跳转、AcquireDevice 是否真拿到 GPU）

## 下一步

骨架冒烟通过后，逐步给 Handler 填充真实逻辑，并为其编写语义级单元测试。
```