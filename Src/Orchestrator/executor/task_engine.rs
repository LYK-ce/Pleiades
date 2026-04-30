// Presented by KeJi
// Date ： 2026-04-23

use std::sync::Arc;
use super::{Capabilities, TaskProgram};
use crate::orchestrator::instruction::TaskInstruction;
use crate::orchestrator::job::JobId;
use crate::orchestrator::slot::{SlotId, SlotFile};

/// 执行模式：正向执行或补偿链执行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionMode {
    Forward,
    Compensation,
}

/// 步骤执行结果枚举
pub enum StepResult {
    Continue,
    Ready,
    Done,
    Abort(String),
}

/// 私有 TaskEngine 结构体
pub struct TaskEngine {
    /// Job 标识，用于生成 session_id 等
    pub(super) job_id: JobId,
    /// 指令指针，pub(super) 以允许 handler_control 跳转修改
    pub(super) ip: usize,
    /// 槽位文件，pub(super) 以允许 handler 文件访问
    pub(super) slots: SlotFile,
    /// 加载的程序，pub(super) 以允许 handler_control 访问标签映射
    pub(super) program: Option<TaskProgram>,
    /// 能力集合，pub(super) 以允许 handler 访问
    pub(super) capabilities: Arc<Capabilities>,
    mode: ExecutionMode,
}

impl TaskEngine {
    pub fn new(job_id: JobId, capabilities: Arc<Capabilities>) -> Self {
        TaskEngine {
            job_id,
            ip: 0,
            slots: SlotFile::new(),
            program: None,
            capabilities,
            mode: ExecutionMode::Forward,
        }
    }

    /// 获取 slots 的只读引用，用于测试验证
    #[cfg(test)]
    pub(super) fn slots(&self) -> &SlotFile {
        &self.slots
    }

    pub fn load(&mut self, program: &TaskProgram) {
        self.program = Some(program.clone());
        self.ip = 0;
    }

    pub async fn step(&mut self) -> StepResult {
        // 如果未加载程序，直接 Abort
        let instr = {
            let program = match self.program.as_ref() {
                Some(p) => p,
                None => return StepResult::Abort("program not loaded".to_string()),
            };
            // 根据当前模式选择指令序列
            let sequence = match self.mode {
                ExecutionMode::Forward => &program.instructions,
                ExecutionMode::Compensation => &program.compensation,
            };
            // 指令指针超出范围，任务完成
            if self.ip >= sequence.len() {
                return StepResult::Done;
            }
            // 克隆指令，释放对 self 的不可变借用
            sequence[self.ip].clone()
        };
        // 默认 ip 自增（指向下一条指令）
        self.ip += 1;
        // 处理指令，JumpIf 条件为真时会在 handler 内部覆盖 ip
        match instr {
            // 数据操作
            TaskInstruction::Const { value, dst } => self.handle_const(value, dst),
            TaskInstruction::Move { src, dst } => self.handle_move(src, dst),
            // 推理生命周期
            TaskInstruction::CreateSession { model, device, start, end, io, tensor_io, result } => self.handle_create_session(model, device, start, end, io, tensor_io, result).await,
            TaskInstruction::ShutdownSession { session } => self.handle_shutdown_session(session).await,
            TaskInstruction::RunProgram { session, result } => self.handle_run_program(session, result).await,
            TaskInstruction::AnalyzeModel { model, result } => self.handle_analyze_model(model, result).await,
            TaskInstruction::SplitModel { source, start, end, output } => self.handle_split_model(source, start, end, output).await,
            // 网络操作
            TaskInstruction::SendFile { peer, file } => self.handle_send_file(peer, file).await,
            TaskInstruction::ReceiveFile { stream, file_name, file_size, checksum, result } => self.handle_receive_file(stream, file_name, file_size, checksum, result).await,
            TaskInstruction::RequestPipeline { peer, model, device, start, end, result } => self.handle_request_pipeline(peer, model, device, start, end, result).await,
            TaskInstruction::OpenTensorStream { peer, target_job } => self.handle_open_tensor_stream(peer, target_job).await,
            TaskInstruction::TakeInboundStream { result } => self.handle_take_inbound_stream(result).await,
            TaskInstruction::TakeOutboundStream { result } => self.handle_take_outbound_stream(result).await,
            TaskInstruction::BuildTensorIo { inbound, outbound, result } => self.handle_build_tensor_io(inbound, outbound, result).await,
            // 控制流
            TaskInstruction::JumpIf { condition, label } => self.handle_jump_if(condition, &label),
            TaskInstruction::Abort { reason } => self.handle_abort(&reason),
        }
    }

    pub async fn enter_compensation(&mut self) {
        // 切换到补偿模式，重置指令指针
        self.mode = ExecutionMode::Compensation;
        self.ip = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager};
    use crate::event_bus::EventBus;
    use crate::orchestrator::slot::ConstValue;
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use crate::orchestrator::tensor_io_broker::Tensor_IO_Broker;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Pipeline_Params, Pipeline_Result, Model_Info};
    use async_trait::async_trait;
    use std::collections::HashMap;
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

    // 测试用例
    #[tokio::test]
    async fn unloaded_engine_aborts() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        assert!(matches!(engine.step().await, StepResult::Abort(_)));
    }

    #[tokio::test]
    async fn empty_program_done() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        engine.load(&TaskProgram {
            instructions: vec![],
            compensation: vec![],
            labels: HashMap::new(),
        });
        assert!(matches!(engine.step().await, StepResult::Done));
    }

    #[tokio::test]
    async fn single_instruction_then_done() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        engine.load(&TaskProgram {
            instructions: vec![TaskInstruction::Abort { reason: "x".into() }],
            compensation: vec![],
            labels: HashMap::new(),
        });
        assert!(matches!(engine.step().await, StepResult::Abort(ref s) if s == "x"));
        assert!(matches!(engine.step().await, StepResult::Done));
    }

    #[tokio::test]
    async fn compensation_mode_switch() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        engine.load(&TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
                TaskInstruction::Abort { reason: "fail".into() },
            ],
            compensation: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(1) },
            ],
            labels: HashMap::new(),
        });

        assert!(matches!(engine.step().await, StepResult::Continue)); // Const
        assert!(matches!(engine.step().await, StepResult::Abort(_)));  // Abort

        engine.enter_compensation().await;

        assert!(matches!(engine.step().await, StepResult::Continue)); // 补偿 Const
        assert!(matches!(engine.step().await, StepResult::Done));     // 补偿结束
    }

    #[tokio::test]
    async fn multi_step_natural_finish() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(JobId(999), caps);
        engine.load(&TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(1) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(2) },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        });

        assert!(matches!(engine.step().await, StepResult::Continue));
        assert!(matches!(engine.step().await, StepResult::Continue));
        assert!(matches!(engine.step().await, StepResult::Continue));
        assert!(matches!(engine.step().await, StepResult::Done));
    }
}