// Presented by KeJi
// Date ： 2026-04-22

use crate::orchestrator::slot::SlotId;
use super::task_engine::StepResult;

impl super::TaskEngine {
    /// 处理 JumpIf 指令：从 condition 槽位读取布尔值，为真时跳转到 label 对应的指令索引
    ///
    /// - 为真：通过 self.program.labels.get(label) 查找目标索引，覆盖 self.ip = target
    /// - 为假：不做任何操作，ip 保持默认的 +1（由 step() 已预先自增）
    /// - 槽位为空或类型不匹配：返回 Abort 并携带错误信息
    /// - 标签不存在：返回 Abort 并携带错误信息
    pub(super) fn handle_jump_if(&mut self, condition: SlotId, label: &str) -> StepResult {
        // 从 condition 槽位读取布尔值
        let cond_value = match self.slots.get_bool(condition) {
            Ok(v) => v,
            Err(e) => return StepResult::Abort(format!("JumpIf: condition slot error: {}", e)),
        };

        // 条件为假时，不修改 ip，保持 step() 中的默认 +1
        if !cond_value {
            return StepResult::Continue;
        }

        // 条件为真时，查找标签并跳转
        let program = match self.program.as_ref() {
            Some(p) => p,
            None => return StepResult::Abort("JumpIf: program not loaded".to_string()),
        };

        match program.labels.get(label) {
            Some(&target) => {
                // 覆盖 ip 到目标位置
                self.ip = target;
                StepResult::Continue
            }
            None => StepResult::Abort(format!("JumpIf: label '{}' not found", label)),
        }
    }

    /// 处理 Abort 指令：立即终止正向执行，触发补偿链
    pub(super) fn handle_abort(&mut self, reason: &str) -> StepResult {
        StepResult::Abort(reason.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::task_engine::{TaskEngine, StepResult};
    use super::super::TaskProgram;
    use crate::orchestrator::instruction::TaskInstruction;
    use crate::orchestrator::job::JobId;
    use crate::orchestrator::slot::{SlotId, SlotValue, ConstValue};
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager};
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use crate::orchestrator::tensor_io_broker::Tensor_IO_Broker;
    use crate::event_bus::EventBus;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Pipeline_Params, Pipeline_Result, Model_Info};
    use super::super::Capabilities;
    use async_trait::async_trait;
    use std::collections::HashMap;
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
            event_bus: Arc::new(EventBus::New(16)),
            io_broker: Arc::new(LLM_IO_Broker::New()),
            tensor_io_broker: Tensor_IO_Broker::New(),
        });
        (caps, temp_dir)
    }

    /// TC-01: JumpIf 真分支跳转
    /// 验证：ip 被覆盖到标签目标位置，跳过中间指令
    #[tokio::test]
    async fn tc01_jump_if_true_branch() {
        let mut labels = HashMap::new();
        labels.insert("target".to_string(), 3); // 跳转到索引 3

        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        engine.load(&TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Bool(true), dst: SlotId(0) },  // 0
                TaskInstruction::JumpIf { condition: SlotId(0), label: "target".into() }, // 1
                TaskInstruction::Abort { reason: "should be skipped".into() },            // 2
                TaskInstruction::Const { value: ConstValue::Bool(false), dst: SlotId(1) }, // 3 (target)
            ],
            compensation: vec![],
            labels,
        });

        // Step 0: Const 写入 true 到 slot 0
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // Step 1: JumpIf 条件为真，跳转到索引 3
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // 验证 ip 已经跳转到 3，下一步应执行索引 3 的指令
        // Step 2: 执行 target 处的 Const
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // Step 3: 程序结束
        assert!(matches!(engine.step().await, StepResult::Done));
        
        // 验证 slot 1 被写入（说明跳转成功，执行了索引 3 的指令）
        assert!(matches!(engine.slots().get(SlotId(1)), Some(SlotValue::Bool(false))));
    }

    /// TC-02: JumpIf 假分支顺序执行
    /// 验证：ip 不被覆盖，自然执行下一条指令
    #[tokio::test]
    async fn tc02_jump_if_false_branch() {
        let mut labels = HashMap::new();
        labels.insert("skip".to_string(), 3);

        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        engine.load(&TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Bool(false), dst: SlotId(0) }, // 0
                TaskInstruction::JumpIf { condition: SlotId(0), label: "skip".into() },   // 1
                TaskInstruction::Const { value: ConstValue::U64(42), dst: SlotId(1) },     // 2 (不跳过)
                TaskInstruction::Const { value: ConstValue::U64(99), dst: SlotId(2) },     // 3 (skip target)
            ],
            compensation: vec![],
            labels,
        });

        // Step 0: Const 写入 false 到 slot 0
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // Step 1: JumpIf 条件为假，不跳转
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // Step 2: 执行索引 2 的 Const（未被跳过）
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // 验证 slot 1 被写入（说明没有跳转）
        assert!(matches!(engine.slots().get(SlotId(1)), Some(SlotValue::U64(42))));
        
        // Step 3: 执行索引 3 的 Const
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // Step 4: 程序结束
        assert!(matches!(engine.step().await, StepResult::Done));
    }

    /// TC-03: JumpIf 标签不存在
    /// 验证：返回 Abort，ip 不被修改
    #[tokio::test]
    async fn tc03_jump_if_label_not_found() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);

        engine.load(&TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Bool(true), dst: SlotId(0) },
                TaskInstruction::JumpIf { condition: SlotId(0), label: "nonexistent".into() },
            ],
            compensation: vec![],
            labels: HashMap::new(), // 空标签映射
        });

        // Step 0: Const
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // Step 1: JumpIf 标签不存在，返回 Abort
        let result = engine.step().await;
        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("label") && s.contains("not found")));
    }

    /// TC-04: JumpIf 槽位类型不匹配
    /// 验证：返回 Abort（如槽位是 U64 而非 Bool）
    #[tokio::test]
    async fn tc04_jump_if_type_mismatch() {
        let mut labels = HashMap::new();
        labels.insert("target".to_string(), 2);

        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        engine.load(&TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::U64(123), dst: SlotId(0) }, // 写入 U64 而非 Bool
                TaskInstruction::JumpIf { condition: SlotId(0), label: "target".into() },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(1) },
            ],
            compensation: vec![],
            labels,
        });

        // Step 0: Const 写入 U64
        assert!(matches!(engine.step().await, StepResult::Continue));
        
        // Step 1: JumpIf 类型不匹配，返回 Abort
        let result = engine.step().await;
        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("not a Bool")));
    }

    /// TC-05: Abort 返回错误
    /// 验证：返回 Abort，携带原始 reason 字符串
    #[tokio::test]
    async fn tc05_abort_returns_reason() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);

        engine.load(&TaskProgram {
            instructions: vec![
                TaskInstruction::Abort { reason: "test error message".into() },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        });

        let result = engine.step().await;
        assert!(matches!(result, StepResult::Abort(ref s) if s == "test error message"));
    }
}