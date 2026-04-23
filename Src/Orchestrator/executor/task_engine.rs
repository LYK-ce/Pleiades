// Presented by KeJi
// Date ： 2026-04-23

use std::sync::Arc;
use super::{Capabilities, TaskProgram};
use crate::orchestrator::instruction::TaskInstruction;
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
    pub fn new(capabilities: Arc<Capabilities>) -> Self {
        TaskEngine {
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
            TaskInstruction::Const { value, dst } => self.handle_const(value, dst),
            TaskInstruction::Move { src, dst } => self.handle_move(src, dst),
            TaskInstruction::AcquireDevice { preferred, result } => self.handle_acquire_device(preferred, result),
            TaskInstruction::CreateSession { model, device, io, result } => self.handle_create_session(model, device, io, result),
            TaskInstruction::ShutdownSession { session } => self.handle_shutdown_session(session),
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
    use crate::orchestrator::{ComputeCapability, InferenceCapability, NetworkCapability, UiCapability};
    use crate::orchestrator::slot::ConstValue;
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use crate::orchestrator::slot::{DeviceLease, SessionHandle};
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

    // 测试用例
    #[tokio::test]
    async fn unloaded_engine_aborts() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(caps);
        assert!(matches!(engine.step().await, StepResult::Abort(_)));
    }

    #[tokio::test]
    async fn empty_program_done() {
        let (caps, _temp_dir) = stub_caps().await;
        let mut engine = TaskEngine::new(caps);
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
        let mut engine = TaskEngine::new(caps);
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
        let mut engine = TaskEngine::new(caps);
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
        let mut engine = TaskEngine::new(caps);
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