// Presented by KeJi
// Date ： 2026-04-23

use crate::orchestrator::slot::{SlotId, ConstValue};
use super::task_engine::StepResult;

impl super::task_engine::TaskEngine {
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
    use super::super::task_engine::TaskEngine;
    use super::super::Capabilities;
    use crate::orchestrator::job::JobId;
    use crate::orchestrator::slot::{SlotId, SlotValue, ConstValue};
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager, StubScheduler};
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use crate::tensor_io::Tensor_Port_Switch;
    use crate::event_bus::EventBus;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Pipeline_Params, Pipeline_Result, Model_Info};
    use super::super::task_engine::StepResult;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    struct StubMLEngine;
    #[async_trait]
    impl ML_Engine_Capability for StubMLEngine {
        async fn Create_Session(&self, _config: ML_Session_Config, _io_handle: crate::llm_io::IoHandle) -> Result<Model_Info, ML_Engine_Error> { unimplemented!("stub") }
        async fn Shutdown_Session(&self, _session_id: &str) -> Result<(), ML_Engine_Error> { unimplemented!("stub") }
        async fn Run_Program(&self, _session_id: &str, _program: Vec<Instruction>, _params: Pipeline_Params, _cancel_flag: Arc<AtomicBool>) -> Result<Pipeline_Result, ML_Engine_Error> { unimplemented!("stub") }
        async fn Analyze_Model(&self, _model_file_id: &str) -> Result<Model_Info, ML_Engine_Error> { unimplemented!("stub") }
        async fn Split_Model(&self, _source_file_id: &str, _start: usize, _end: usize, _output_file_id: &str) -> Result<(), ML_Engine_Error> { unimplemented!("stub") }
    }

    async fn stub_caps() -> (Arc<Capabilities>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(StorageManager::New(temp_dir.path()).await.unwrap());
        let caps = Arc::new(Capabilities {
            storage,
            ml_engine: Box::new(StubMLEngine),
            network: Box::new(StubNetwork),
            peer_manager: Box::new(StubPeerManager),
            scheduler: Box::new(StubScheduler),
            event_bus: Arc::new(EventBus::New(16)),
            io_broker: Arc::new(LLM_IO_Broker::New()),
            tensor_switch: Arc::new(Tensor_Port_Switch::New()),
        });
        (caps, temp_dir)
    }

    /// TC-01: Const 写入后读取
    /// 验证点：`slots.get(dst)` 返回原值
    #[tokio::test]
    async fn test_const_write_and_read() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
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
        let mut engine = TaskEngine::new(JobId(999), caps);
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

        // 旧值被覆盖
        let overwritten = engine.handle_const(ConstValue::Bool(true), dst);

        assert!(matches!(overwritten, StepResult::Continue));
        let stored = engine.slots().get(dst);
        assert!(stored.is_some());
        match stored.unwrap() {
            SlotValue::Bool(b) => assert!(*b),
            _ => panic!("Expected SlotValue::Bool"),
            _ => panic!("Expected SlotValue::Bool"),
        }
    }

    /// TC-04: Move 从空槽位
    /// 验证点：返回 `Abort`，目标槽位不受影响
    #[tokio::test]
    async fn test_move_from_empty_slot() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
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