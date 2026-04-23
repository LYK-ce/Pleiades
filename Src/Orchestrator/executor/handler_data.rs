// Presented by KeJi
// Date ： 2026-04-23

use crate::orchestrator::slot::{SlotId, ConstValue};
use super::task_engine::StepResult;

impl super::TaskEngine {
    /// 处理 Const 指令：将常量值写入目标槽位
    /// 将 `ConstValue` 转换为 `SlotValue` 后写入 `dst` 槽位，覆盖原有值（旧值自然 Drop）
    pub(super) fn handle_const(&mut self, value: ConstValue, dst: SlotId) -> StepResult {
        self.slots.set(dst, value.into());
        StepResult::Continue
    }

    /// 处理 Move 指令：从源槽位取出值，写入目标槽位
    /// 从 `src` 槽位 `take` 值（原槽位置 `Nil`），写入 `dst` 槽位
    /// 若 `src` 为空，返回 `Abort` 并携带错误信息
    pub(super) fn handle_move(&mut self, src: SlotId, dst: SlotId) -> StepResult {
        match self.slots.take(src) {
            Some(value) => {
                self.slots.set(dst, value);
                StepResult::Continue
            }
            None => StepResult::Abort(format!("Move failed: source slot {} is empty", src.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::TaskEngine;
    use super::super::Capabilities;
    use super::super::{ComputeCapability, InferenceCapability};
    use crate::orchestrator::slot::{SlotId, SlotValue, ConstValue, DeviceLease, SessionHandle};
    use crate::orchestrator::{NetworkCapability, UiCapability};
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use super::super::task_engine::StepResult;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tempfile::TempDir;

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

    async fn stub_caps() -> (Arc<Capabilities>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::New(temp_dir.path()).await.unwrap();
        let caps = Arc::new(Capabilities {
            storage,
            compute: Box::new(StubCompute),
            inference: Box::new(StubInference),
            network: NetworkCapability,
            ui: UiCapability,
            io_broker: LLM_IO_Broker::New(),
        });
        (caps, temp_dir)
    }

    /// TC-01: Const 写入后读取
    /// 验证点：`slots.get(dst)` 返回原值
    #[tokio::test]
    async fn test_const_write_and_read() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(caps);
        let dst = SlotId(0);
        let value = ConstValue::String("test_value".to_string());

        let result = engine.handle_const(value, dst);

        assert!(matches!(result, StepResult::Continue));
        let stored = engine.slots().get(dst);
        assert!(stored.is_some());
        match stored.unwrap() {
            SlotValue::String(s) => assert_eq!(s, "test_value"),
            _ => panic!("Expected SlotValue::String"),
        }
    }

    /// TC-02: Const 覆盖旧值
    /// 验证点：旧值被替换，新值正确
    #[tokio::test]
    async fn test_const_overwrite() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(caps);
        let dst = SlotId(0);

        // 写入旧值
        engine.handle_const(ConstValue::U64(100), dst);
        // 覆盖旧值
        let result = engine.handle_const(ConstValue::U64(200), dst);

        assert!(matches!(result, StepResult::Continue));
        let stored = engine.slots().get(dst);
        assert!(stored.is_some());
        match stored.unwrap() {
            SlotValue::U64(v) => assert_eq!(*v, 200),
            _ => panic!("Expected SlotValue::U64"),
        }
    }

    /// TC-03: Move 成功转移
    /// 验证点：源槽位变为 `Nil`，目标槽位有值
    #[tokio::test]
    async fn test_move_success() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(caps);
        let src = SlotId(0);
        let dst = SlotId(1);

        // 先在源槽位写入值
        engine.handle_const(ConstValue::Bool(true), src);
        // 执行 Move
        let result = engine.handle_move(src, dst);

        assert!(matches!(result, StepResult::Continue));
        // 检查源槽位变为 Nil
        let src_value = engine.slots().get(src);
        assert!(src_value.is_some());
        assert!(matches!(src_value.unwrap(), SlotValue::Nil));
        // 检查目标槽位有正确的值
        let dst_value = engine.slots().get(dst);
        assert!(dst_value.is_some());
        match dst_value.unwrap() {
            SlotValue::Bool(b) => assert!(*b),
            _ => panic!("Expected SlotValue::Bool"),
        }
    }

    /// TC-04: Move 从空槽位
    /// 验证点：返回 `Abort`，目标槽位不受影响
    #[tokio::test]
    async fn test_move_from_empty_slot() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(caps);
        let src = SlotId(0); // 空槽位
        let dst = SlotId(1);

        // 先在目标槽位写入值
        engine.handle_const(ConstValue::String("existing".to_string()), dst);
        // 尝试从空槽位 Move
        let result = engine.handle_move(src, dst);

        // 应该返回 Abort
        assert!(matches!(result, StepResult::Abort(_)));
        if let StepResult::Abort(msg) = result {
            assert!(msg.contains("empty"));
        }
        // 目标槽位应该不受影响，仍保持原值
        let dst_value = engine.slots().get(dst);
        assert!(dst_value.is_some());
        match dst_value.unwrap() {
            SlotValue::String(s) => assert_eq!(s, "existing"),
            _ => panic!("Expected SlotValue::String"),
        }
    }
}