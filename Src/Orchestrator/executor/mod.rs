// Presented by KeJi
// Date ： 2026-05-09

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use super::job::{JobId, JobKind, JobState, LifecycleEvent};
use super::slot::SlotValue;
use super::compiler::SLOT_IO;
use crate::llm_io::IoHandle;

mod task_engine;
mod handler_data;
mod handler_inference;
mod handler_network;
mod handler_scheduler;
mod handler_control;

use super::orchestrator_vm::{Orchestrator_VM, OrchestratorInstruction};
use crate::vm_base::StepResult;

/// 从 instruction 模块导入 TaskProgram（后续迁移到 OrchestratorInstruction）
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
    engine: Orchestrator_VM,
    state: JobState,
}

/// 将旧 TaskInstruction → 新 OrchestratorInstruction，同时转换 SlotId。
fn convert_instruction(
    inst: &super::instruction::TaskInstruction,
    labels: &std::collections::HashMap<String, usize>,
) -> OrchestratorInstruction {
    use super::instruction::TaskInstruction;
    use super::orchestrator_vm::to_vm_slot;
    use crate::vm_base::{ConstValue, SlotId};

    match inst {
        TaskInstruction::Const { value, dst } => {
            let cv = match value {
                super::slot::ConstValue::Nil => ConstValue::Nil,
                super::slot::ConstValue::Bool(b) => ConstValue::Bool(*b),
                super::slot::ConstValue::U64(v) => ConstValue::U64(*v),
                super::slot::ConstValue::String(s) => ConstValue::String(s.clone()),
                super::slot::ConstValue::PathBuf(p) => ConstValue::PathBuf(p.clone()),
                super::slot::ConstValue::Error(e) => ConstValue::Error(e.clone()),
            };
            OrchestratorInstruction::Const { value: cv, dst: to_vm_slot(*dst) }
        }
        TaskInstruction::Move { src, dst } => OrchestratorInstruction::Move {
            src: to_vm_slot(*src),
            dst: to_vm_slot(*dst),
        },
        TaskInstruction::CreateSession { model, device, start, end, io, tensor_io, result } => {
            OrchestratorInstruction::CreateSession {
                model: to_vm_slot(*model),
                device: to_vm_slot(*device),
                start: to_vm_slot(*start),
                end: to_vm_slot(*end),
                io: to_vm_slot(*io),
                tensor_io: tensor_io.map(|s| to_vm_slot(s)),
                result: to_vm_slot(*result),
            }
        }
        TaskInstruction::ShutdownSession { session } => OrchestratorInstruction::ShutdownSession {
            session: to_vm_slot(*session),
        },
        TaskInstruction::RunProgram { session, result } => OrchestratorInstruction::RunProgram {
            session: to_vm_slot(*session),
            result: to_vm_slot(*result),
        },
        TaskInstruction::AnalyzeModel { model, result } => OrchestratorInstruction::AnalyzeModel {
            model: to_vm_slot(*model),
            result: to_vm_slot(*result),
        },
        TaskInstruction::SplitModel { source, start, end, output } => OrchestratorInstruction::SplitModel {
            source: to_vm_slot(*source),
            start: to_vm_slot(*start),
            end: to_vm_slot(*end),
            output: to_vm_slot(*output),
        },
        TaskInstruction::SendFile { peer, file } => OrchestratorInstruction::SendFile {
            peer: to_vm_slot(*peer),
            file: to_vm_slot(*file),
        },
        TaskInstruction::ReceiveFile { stream, file_name, file_size, checksum, result } => OrchestratorInstruction::ReceiveFile {
            stream: to_vm_slot(*stream),
            file_name: to_vm_slot(*file_name),
            file_size: to_vm_slot(*file_size),
            checksum: to_vm_slot(*checksum),
            result: to_vm_slot(*result),
        },
        TaskInstruction::PlanPipeline { model_info, inference_id, result } => OrchestratorInstruction::PlanPipeline {
            model_info: to_vm_slot(*model_info),
            inference_id: to_vm_slot(*inference_id),
            result: to_vm_slot(*result),
        },
        TaskInstruction::EstablishStreams { plan, result } => OrchestratorInstruction::EstablishStreams {
            plan: to_vm_slot(*plan),
            result: to_vm_slot(*result),
        },
        TaskInstruction::JoinWorkers { plan, result } => OrchestratorInstruction::JoinWorkers {
            plan: to_vm_slot(*plan),
            result: to_vm_slot(*result),
        },
        TaskInstruction::JumpIf { condition, label } => {
            let target = *labels.get(label).expect(&format!("JumpIf: label '{}' not found", label));
            OrchestratorInstruction::JumpIf {
                condition: to_vm_slot(*condition),
                target,
            }
        }
        TaskInstruction::Abort { reason } => OrchestratorInstruction::Abort {
            reason: reason.clone(),
        },
        TaskInstruction::TeardownPipeline { plan: _ } => {
            // MVP: 无补偿链，跳过 TeardownPipeline
            OrchestratorInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) }
        }
    }
}

impl JobExecutor {
    pub fn new(
        job_id: JobId,
        kind: JobKind,
        program: TaskProgram,
        cancel: CancellationToken,
        capabilities: Arc<Capabilities>,
        io: Option<IoHandle>,
        lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    ) -> Self {
        let mut engine = Orchestrator_VM::new(job_id, Arc::clone(&capabilities));
        if let Some(io) = io {
            engine.slots.set(
                super::orchestrator_vm::to_vm_slot(SLOT_IO),
                super::orchestrator_vm::OrchestratorSlotValue::IoHandle(io),
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

    pub fn inject_slot(&mut self, slot_id: super::slot::SlotId, value: SlotValue) {
        use super::slot::SlotValue::{IoHandle, Stream, TensorIo, ModelInfo, PipelinePlan};
        use super::orchestrator_vm::{to_vm_slot, OrchestratorSlotValue};
        let vm_slot = to_vm_slot(slot_id);
        match value {
            IoHandle(h) => { self.engine.slots.set(vm_slot, OrchestratorSlotValue::IoHandle(h)); }
            Stream(s) => {
                let inner = s.lock().unwrap().take();
                self.engine.slots.set(vm_slot, OrchestratorSlotValue::Stream(std::sync::Mutex::new(inner)));
            }
            TensorIo(t) => {
                let inner = t.lock().unwrap().take();
                if let Some(ep) = inner {
                    self.engine.slots.set(vm_slot, OrchestratorSlotValue::TensorIO(ep));
                }
            }
            ModelInfo(m) => { self.engine.slots.set(vm_slot, OrchestratorSlotValue::ModelInfo(m)); }
            PipelinePlan(p) => { self.engine.slots.set(vm_slot, OrchestratorSlotValue::PipelinePlan(p)); }
            _ => { self.engine.vm.slots.set(vm_slot, convert_basic_slot(value)); }
        }
    }

    pub async fn run(mut self) {
        let instructions: Vec<OrchestratorInstruction> = self.program.instructions.iter()
            .map(|inst| convert_instruction(inst, &self.program.labels))
            .collect();
        self.engine.load(instructions);

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

    async fn report_error(&self, _error: &str) {}
    async fn cleanup(&self) {}
}

/// 执行退出原因枚举
enum ExitReason {
    Success,
    Cancelled,
    Failed(String),
}

fn convert_basic_slot(old: SlotValue) -> crate::vm_base::SlotValue {
    match old {
        SlotValue::Nil => crate::vm_base::SlotValue::Nil,
        SlotValue::Bool(b) => crate::vm_base::SlotValue::Bool(b),
        SlotValue::U64(v) => crate::vm_base::SlotValue::U64(v),
        SlotValue::String(s) => crate::vm_base::SlotValue::String(s),
        SlotValue::PathBuf(p) => crate::vm_base::SlotValue::PathBuf(p),
        SlotValue::Error(e) => crate::vm_base::SlotValue::Error(e),
        _ => crate::vm_base::SlotValue::Nil,
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use crate::orchestrator::job::{JobId, JobKind, JobResult};
    use crate::orchestrator::instruction::{TaskInstruction, TaskProgram};
    use crate::orchestrator::slot::{SlotId, ConstValue};
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager, StubScheduler};
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use crate::tensor_io::Tensor_Port_Switch;
    use crate::event_bus::EventBus;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Pipeline_Params, Pipeline_Result, Model_Info};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

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

        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        };

        let job_id = JobId(1);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(job_id, JobKind::Run, program, cancel, caps, Some(io), lifecycle_tx);
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

        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Abort { reason: "test failure".to_string() },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        };

        let job_id = JobId(2);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(job_id, JobKind::Run, program, cancel, caps, Some(io), lifecycle_tx);
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

        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(0) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(1) },
                TaskInstruction::Const { value: ConstValue::Nil, dst: SlotId(2) },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        };

        let job_id = JobId(3);
        let io = stub_io_handle(&caps, job_id).await;

        let executor = JobExecutor::new(job_id, JobKind::Run, program, cancel.clone(), caps, Some(io), lifecycle_tx);
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
