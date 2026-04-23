# JobExecutor 冒烟测试说明

## 测试目标

验证 **JobExecutor 外壳**能完整运转：
- `tokio::spawn` 启动
- `select!` 同时监听 `Cancel` 信号与 `TaskEngine::step()`
- 正向执行、Abort、Cancel 三种退出路径都能正确上报 `LifecycleEvent`
- **不验证任何 Handler 业务逻辑**（仍使用空壳 Stub）

## Stub 策略

Capability 全部用最小 Stub 实现，方法体直接返回 `Ok(())` / `Ok(DeviceLease)` / `Ok(SessionHandle)`。  
因为空壳 Handler 不调用 Capability，Stub 只需满足 trait 签名即可编译通过。

## 测试用例（3 个）

| 编号 | 场景 | 验证点 |
|------|------|--------|
| TC-01 | 正向执行自然结束 | `tokio::spawn` → 正向序列走完 → `LifecycleEvent::Done(Success)` |
| TC-02 | Abort 触发补偿链 | 正向序列中途 `Abort` → `enter_compensation()` → 补偿序列执行 → `LifecycleEvent::Done(Failed)` |
| TC-03 | Cancel 信号中断 | `CancellationToken::cancel()` → `select!` 优先捕获 → 进入补偿 → `LifecycleEvent::Done(Cancelled)` |

## 注意事项

- TC-03 的指令序列需放 **≥5 条空壳指令**，确保 `cancel()` 有机会在 `select!` 中被捕获，而不是瞬间自然走完。
- `JobExecutor::run()` 内部需先解构 `self` 字段（`lifecycle_tx`、`cancel`、`capabilities` 提前取出），再对 `self.program` 做**借用加载**（`&self.program`），避免部分移动（E0382）。

## 运行命令

```bash
cargo test executor_tests -- --nocapture
```

## 通过标准

- [ ] TC-01 通过：端到端 `Success` 上报
- [ ] TC-02 通过：`Abort` → 补偿链 → `Failed` 上报
- [ ] TC-03 通过：`Cancel` 信号 → 补偿链 → `Cancelled` 上报

三项全部通过，即确认 **JobExecutor 集成外壳**就绪，可继续向下填充 Handler 真实逻辑。
