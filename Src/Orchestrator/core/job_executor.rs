// Presented by KeJi
// Date ： 2026-05-11

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use crate::orchestrator::job::{JobId, JobKind, JobState, JobResult, LifecycleEvent};
use crate::llm_io::IoHandle;
use crate::vm_base::SlotId;

use crate::orchestrator::orchestrator_vm::{Orchestrator_VM, OrchestratorInstruction, OrchestratorSlotValue};
use crate::orchestrator::program_selector::SLOT_IO;
use crate::vm_base::StepResult;

use crate::orchestrator::Capabilities;

/// JobExecutor 结构体 — 封装 Orchestrator_VM，提供 tokio 任务生命周期管理
pub struct JobExecutor {
    job_id: JobId,
    kind: JobKind,
    program: Vec<OrchestratorInstruction>,
    cancel: CancellationToken,
    capabilities: Arc<Capabilities>,
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    engine: Orchestrator_VM,
    state: JobState,
}

impl JobExecutor {
    pub fn new(
        job_id: JobId,
        kind: JobKind,
        program: Vec<OrchestratorInstruction>,
        cancel: CancellationToken,
        capabilities: Arc<Capabilities>,
        io: Option<IoHandle>,
        lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    ) -> Self {
        let mut engine = Orchestrator_VM::new(job_id, Arc::clone(&capabilities));
        if let Some(io) = io {
            engine.slots.set(
                SLOT_IO,
                OrchestratorSlotValue::IoHandle(io),
            );
        }
        JobExecutor {
            job_id,
            kind,
            program,
            cancel,
            capabilities,
            lifecycle_tx,
            engine,
            state: JobState::Preparing,
        }
    }

    pub fn inject_slot(&mut self, slot_id: SlotId, value: OrchestratorSlotValue) {
        self.engine.slots.set(slot_id, value);
    }

    pub async fn run(mut self) {
        self.engine.load(self.program.clone());

        let mut exit_reason = ExitReason::Success;

        loop {
            tokio::select! {
                biased;

                _ = self.cancel.cancelled() => {
                    exit_reason = ExitReason::Cancelled;
                    break;
                }

                result = self.engine.step() => {
                    match result {
                        StepResult::Continue => continue,
                        StepResult::Ready => { self.state = JobState::Ready; }
                        StepResult::Done => break,
                        StepResult::Abort(e) => {
                            self.report_error(&e).await;
                            exit_reason = ExitReason::Failed(e);
                            break;
                        }
                    }
                }
            }
        }

        let result = match exit_reason {
            ExitReason::Success => JobResult::Success,
            ExitReason::Cancelled => JobResult::Cancelled,
            ExitReason::Failed(e) => JobResult::Failed(e),
        };

        let _ = self.lifecycle_tx.send(LifecycleEvent::Done {
            job_id: self.job_id,
            result,
        }).await;
        self.cleanup().await;
    }

    async fn report_error(&self, _error: &str) {}
    async fn cleanup(&self) {}
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
    use crate::orchestrator::orchestrator_vm::OrchestratorInstruction;
    use crate::vm_base::{ConstValue, SlotId};
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager, StubScheduler};
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use crate::tensor_io::Tensor_Port_Switch;
    use crate::event_bus::EventBus;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::pipeline::{Pipeline_Params, Pipeline_Result, Model_Info};
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    struct StubMLEngine;
    #[async_trait]
    impl ML_Engine_Capability for StubMLEngine {
        async fn Create_Session(&self, _config: ML_Session_Config, _io_handle: IoHandle) -> Result<Model_Info, ML_Engine_Error> { unimplemented!("stub") }
        async fn Shutdown_Session(&self, _session_id: &str) -> Result<(), ML_Engine_Error> { unimplemented!("stub") }
        async fn Run_Program_VM(&self, _session_id: &str, _program: Vec<crate::ml_engine::ml_vm::MlInstruction>, _params: Pipeline_Params, _cancel_flag: Arc<AtomicBool>) -> Result<Pipeline_Result, ML_Engine_Error> { unimplemented!("stub") }
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
            scheduler: Box::new(StubScheduler),
            event_bus: Arc::new(EventBus::New(16)),
            io_broker: Arc::new(LLM_IO_Broker::New()),
            tensor_switch: Arc::new(Tensor_Port_Switch::New()),
        });
        (caps, temp_dir)
    }

    async fn stub_io_handle(caps: &Arc<Capabilities>, job_id: JobId) -> IoHandle {
        use crate::llm_io::LLM_IO_Capability;
        caps.io_broker.Allocate(job_id).await.unwrap();
        caps.io_broker.Take_ML_Side(job_id).await.unwrap()
    }

    #[tokio::test]
    async fn tc01_forward_execution_success() {
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::channel::<LifecycleEvent>(16);
        let cancel = CancellationToken::new();
        let (caps, _temp_dir) = stub_capabilities().await;

        let instructions = vec![
                OrchestratorInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
            ];

        let job_id = JobId(1);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(job_id, JobKind::Run, instructions, cancel, caps, Some(io), lifecycle_tx);
        tokio::spawn(executor.run());

        let event = lifecycle_rx.recv().await.expect("应收到 LifecycleEvent");
        match event {
            LifecycleEvent::Done { job_id, result } => {
                assert_eq!(job_id, JobId(1));
                assert!(matches!(result, JobResult::Success));
            }
        }
    }

    #[tokio::test]
    async fn tc02_abort_triggers_failed() {
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::channel::<LifecycleEvent>(16);
        let cancel = CancellationToken::new();
        let (caps, _temp_dir) = stub_capabilities().await;

        let instructions = vec![
                OrchestratorInstruction::Abort { reason: "test failure".to_string() },
            ];

        let job_id = JobId(2);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(job_id, JobKind::Run, instructions, cancel, caps, Some(io), lifecycle_tx);
        tokio::spawn(executor.run());

        let event = lifecycle_rx.recv().await.expect("应收到 LifecycleEvent");
        match event {
            LifecycleEvent::Done { job_id, result } => {
                assert_eq!(job_id, JobId(2));
                assert!(matches!(result, JobResult::Failed(ref s) if s == "test failure"));
            }
        }
    }

    #[tokio::test]
    async fn tc03_cancel_signal_interrupts() {
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::channel::<LifecycleEvent>(16);
        let cancel = CancellationToken::new();
        let (caps, _temp_dir) = stub_capabilities().await;

        let instructions = vec![
                OrchestratorInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
                OrchestratorInstruction::Const { value: ConstValue::Nil, dst: SlotId(1) },
                OrchestratorInstruction::Const { value: ConstValue::Nil, dst: SlotId(2) },
            ];

        let job_id = JobId(3);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(job_id, JobKind::Run, instructions, cancel.clone(), caps, Some(io), lifecycle_tx);
        cancel.cancel();
        let handle = tokio::spawn(executor.run());
        let _ = handle.await;

        let event = lifecycle_rx.recv().await.expect("应收到 LifecycleEvent");
        match event {
            LifecycleEvent::Done { job_id, result } => {
                assert_eq!(job_id, JobId(3));
                assert!(matches!(result, JobResult::Cancelled));
            }
        }
    }
}
