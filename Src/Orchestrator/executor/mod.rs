// Presented by KeJi
// Date ： 2026-04-23

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use super::job::{JobId, JobKind, JobState, LifecycleEvent};
use super::slot::{SlotId, SlotValue};
use super::compiler::SLOT_IO;
use crate::llm_io::IoHandle;

mod task_engine;
mod handler_data;
mod handler_inference;
mod handler_network;
mod handler_control;
use task_engine::{TaskEngine, StepResult};

/// 从 instruction 模块导入 TaskProgram
pub use super::instruction::TaskProgram;

use super::Capabilities;

/// JobExecutor 结构体
pub struct JobExecutor {
    job_id: JobId,
    kind: JobKind,
    program: TaskProgram,
    cancel: CancellationToken,
    capabilities: Arc<Capabilities>,
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    task_engine: TaskEngine,
    state: JobState,
}

impl JobExecutor {
    /// 创建新的 JobExecutor 实例
    ///
    /// `io` 参数为 `Option<IoHandle>`：
    /// - `Some(io)`: 推理类 Job（Run/Coordinator/Relay），IoHandle 注入到 `SLOT_IO`
    /// - `None`: 非推理 Job（Send/Distribute/Receive），无需 ML I/O 通道
    pub fn new(
        job_id: JobId,
        kind: JobKind,
        program: TaskProgram,
        cancel: CancellationToken,
        capabilities: Arc<Capabilities>,
        io: Option<IoHandle>,
        lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    ) -> Self {
        let mut task_engine = TaskEngine::new(job_id, Arc::clone(&capabilities));
        // 仅当提供 IoHandle 时注入到约定槽位（推理类 Job 需要）
        if let Some(io) = io {
            task_engine.slots.set(SLOT_IO, SlotValue::IoHandle(io));
        }
        JobExecutor {
            job_id,
            kind,
            program,
            cancel,
            capabilities,
            lifecycle_tx,
            task_engine,
            state: JobState::Preparing,
        }
    }

    /// 在 spawn 前向 SlotFile 注入额外的运行时资源
    ///
    /// 用于 Core 在创建 Executor 后、调用 `run()` 前注入不可通过 Const 指令表达的值
    /// （如 `libp2p::Stream`），因为 Stream 不属于 `ConstValue` 子集。
    ///
    /// # 用法
    /// ```ignore
    /// let mut executor = JobExecutor::new(...);
    /// executor.inject_slot(SLOT_RECEIVE_STREAM, SlotValue::Stream(Mutex::new(Some(stream))));
    /// tokio::spawn(executor.run());
    /// ```
    pub fn inject_slot(&mut self, slot_id: SlotId, value: SlotValue) {
        self.task_engine.slots.set(slot_id, value);
    }

    /// 主执行循环
    pub async fn run(mut self) {
        self.task_engine.load(&self.program);
        
        // 记录退出原因
        let mut exit_reason = ExitReason::Success;
        
        loop {
            tokio::select! {
                biased;

                _ = self.cancel.cancelled() => {
                    exit_reason = ExitReason::Cancelled;
                    self.task_engine.enter_compensation().await;
                    // 执行补偿序列
                    self.run_compensation().await;
                    break;
                }
                
                result = self.task_engine.step() => {
                    match result {
                        StepResult::Continue => continue,
                        StepResult::Ready => { self.state = JobState::Ready; }
                        StepResult::Done => break,
                        StepResult::Abort(e) => {
                            self.report_error(&e).await;
                            exit_reason = ExitReason::Failed(e);
                            self.task_engine.enter_compensation().await;
                            // 执行补偿序列
                            self.run_compensation().await;
                            break;
                        }
                    }
                }
            }
        }
        
        // 根据退出原因发送对应的 LifecycleEvent
        let result = match exit_reason {
            ExitReason::Success => super::job::JobResult::Success,
            ExitReason::Cancelled => super::job::JobResult::Cancelled,
            ExitReason::Failed(e) => super::job::JobResult::Failed(e),
        };
        
        let _ = self.lifecycle_tx.send(LifecycleEvent::Done {
            job_id: self.job_id,
            result,
        }).await;
        self.cleanup().await;
    }

    /// 执行补偿序列直到完成
    async fn run_compensation(&mut self) {
        loop {
            match self.task_engine.step().await {
                StepResult::Done => break,
                StepResult::Continue | StepResult::Ready => continue,
                StepResult::Abort(_) => break, // 补偿序列中的 Abort 直接结束
            }
        }
    }

    /// 错误报告（占位符，测试用 Stub）
    async fn report_error(&self, _error: &str) {
        // 占位符：通过 UI Capability 报告错误
        // 目前为空实现，支持测试
    }

    /// 清理资源（占位符，测试用 Stub）
    async fn cleanup(&self) {
        // 占位符：执行清理操作
        // 目前为空实现，支持测试
    }
}

/// 执行退出原因枚举
enum ExitReason {
    Success,
    Cancelled,
    Failed(String),
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use crate::orchestrator::job::{JobId, JobKind, JobResult};
    use crate::orchestrator::instruction::{TaskInstruction, TaskProgram};
    use crate::orchestrator::slot::{SlotId, ConstValue};
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager};
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use crate::orchestrator::tensor_io_broker::Tensor_IO_Broker;
    use crate::event_bus::EventBus;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Pipeline_Params, Pipeline_Result, Model_Info};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    // ---- Stub 实现 ----

    struct StubMLEngine;
    #[async_trait]
    impl ML_Engine_Capability for StubMLEngine {
        async fn Create_Session(&self, _config: ML_Session_Config, _io_handle: IoHandle) -> Result<Model_Info, ML_Engine_Error> { unimplemented!("stub") }
        async fn Shutdown_Session(&self, _session_id: &str) -> Result<(), ML_Engine_Error> { unimplemented!("stub") }
        async fn Run_Program(&self, _session_id: &str, _program: Vec<Instruction>, _params: Pipeline_Params, _cancel_flag: Arc<AtomicBool>) -> Result<Pipeline_Result, ML_Engine_Error> { unimplemented!("stub") }
        async fn Analyze_Model(&self, _model_file_id: &str) -> Result<Model_Info, ML_Engine_Error> { unimplemented!("stub") }
        async fn Split_Model(&self, _source_file_id: &str, _start: usize, _end: usize, _output_file_id: &str) -> Result<(), ML_Engine_Error> { unimplemented!("stub") }
    }

    async fn stub_capabilities() -> (Arc<Capabilities>, TempDir) {
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

    /// 创建 stub IoHandle 用于测试
    async fn stub_io_handle(caps: &Arc<Capabilities>, job_id: JobId) -> IoHandle {
        use crate::llm_io::LLM_IO_Capability;
        caps.io_broker.Allocate(job_id).await.unwrap();
        caps.io_broker.Take_ML_Side(job_id).await.unwrap()
    }

    // ---- TC-01: 正向执行自然结束 ----
    #[tokio::test]
    async fn tc01_forward_execution_success() {
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::channel::<LifecycleEvent>(16);
        let cancel = CancellationToken::new();
        let (caps, _temp_dir) = stub_capabilities().await;

        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(1) },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        };

        let job_id = JobId(1);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(
            job_id,
            JobKind::Run,
            program,
            cancel,
            caps,
            Some(io),
            lifecycle_tx,
        );

        tokio::spawn(executor.run());

        // 验证收到 Success 结果
        let event = lifecycle_rx.recv().await.expect("应收到 LifecycleEvent");
        match event {
            LifecycleEvent::Done { job_id, result } => {
                assert_eq!(job_id, JobId(1));
                assert!(matches!(result, JobResult::Success));
            }
        }
    }

    // ---- TC-02: Abort 触发补偿链 ----
    #[tokio::test]
    async fn tc02_abort_triggers_compensation() {
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::channel::<LifecycleEvent>(16);
        let cancel = CancellationToken::new();
        let (caps, _temp_dir) = stub_capabilities().await;

        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
                TaskInstruction::Abort { reason: "test failure".to_string() },
            ],
            compensation: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(10) },
            ],
            labels: HashMap::new(),
        };

        let job_id = JobId(2);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(
            job_id,
            JobKind::Run,
            program,
            cancel,
            caps,
            Some(io),
            lifecycle_tx,
        );

        tokio::spawn(executor.run());

        // 验证收到 Failed 结果
        let event = lifecycle_rx.recv().await.expect("应收到 LifecycleEvent");
        match event {
            LifecycleEvent::Done { job_id, result } => {
                assert_eq!(job_id, JobId(2));
                assert!(matches!(result, JobResult::Failed(ref s) if s == "test failure"));
            }
        }
    }

    // ---- TC-03: Cancel 信号中断 ----
    #[tokio::test]
    async fn tc03_cancel_signal_interrupts() {
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::channel::<LifecycleEvent>(16);
        let cancel = CancellationToken::new();
        let (caps, _temp_dir) = stub_capabilities().await;

        // 使用 ≥5 条空壳指令
        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(1) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(2) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(3) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(4) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(5) },
            ],
            compensation: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(10) },
            ],
            labels: HashMap::new(),
        };

        let job_id = JobId(3);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(
            job_id,
            JobKind::Run,
            program,
            cancel.clone(),
            caps,
            Some(io),
            lifecycle_tx,
        );

        // 在启动执行器之前发送 cancel 信号
        // select! 的 biased 模式会在第一次轮询时优先捕获 cancel
        cancel.cancel();

        // 启动执行器
        let handle = tokio::spawn(executor.run());

        // 等待执行器完成
        let _ = handle.await;

        // 验证收到 Cancelled 结果
        let event = lifecycle_rx.recv().await.expect("应收到 LifecycleEvent");
        match event {
            LifecycleEvent::Done { job_id, result } => {
                assert_eq!(job_id, JobId(3));
                assert!(matches!(result, JobResult::Cancelled));
            }
        }
    }
}