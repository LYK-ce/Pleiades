//Presented by KeJi
//Date ： 2026-05-09

//! Orchestrator 虚拟机执行引擎。
//!
//! step() 逐条 fetch → decode → dispatch 指令。
//! 公共指令委托给 Vm_Base::Vm，领域指令由各自的 handler 实现。

use std::sync::Arc;
use crate::vm_base::{StepResult, Vm};
use super::instruction::OrchestratorInstruction;
use super::slots::OrchestratorSlots;
use crate::orchestrator::Capabilities;
use crate::orchestrator::job::JobId;

pub struct Orchestrator_VM {
    pub vm: Vm,
    pub slots: OrchestratorSlots,
    pub capabilities: Arc<Capabilities>,
    pub job_id: JobId,
    program: Vec<OrchestratorInstruction>,
}

impl Orchestrator_VM {
    pub fn new(job_id: JobId, capabilities: Arc<Capabilities>) -> Self {
        Self {
            vm: Vm::new(),
            slots: OrchestratorSlots::new(),
            capabilities,
            job_id,
            program: Vec::new(),
        }
    }

    /// 加载指令程序，重置 IP。
    pub fn load(&mut self, program: Vec<OrchestratorInstruction>) {
        self.vm.ip = 0;
        self.program = program;
    }

    pub async fn step(&mut self) -> StepResult {
        let inst = &self.program[self.vm.ip];
        self.vm.ip += 1;
        match inst {
            // 公共指令 → 委托给 Vm
            OrchestratorInstruction::Const { value, dst } => self.vm.handle_const(value.clone(), *dst),
            OrchestratorInstruction::Move { src, dst }    => self.vm.handle_move(*src, *dst),
            OrchestratorInstruction::Add { dst, delta }   => self.vm.handle_add(*dst, *delta),
            OrchestratorInstruction::Jump { target }      => self.vm.handle_jump(*target),
            OrchestratorInstruction::JumpIf { condition, target } => self.vm.handle_jump_if(*condition, *target),
            // 领域指令 → handler 文件
            OrchestratorInstruction::CreateSession { model, device, start, end, io, tensor_io, result } => {
                self.handle_create_session(*model, *device, *start, *end, *io, *tensor_io, *result).await
            }
            OrchestratorInstruction::ShutdownSession { session } => {
                self.handle_shutdown_session(*session).await
            }
            OrchestratorInstruction::RunProgram { session, result } => {
                self.handle_run_program(*session, *result).await
            }
            OrchestratorInstruction::AnalyzeModel { model, result } => {
                self.handle_analyze_model(*model, *result).await
            }
            OrchestratorInstruction::SplitModel { source, start, end, output } => {
                self.handle_split_model(*source, *start, *end, *output).await
            }
            OrchestratorInstruction::SendFile { peer, file } => {
                self.handle_send_file(*peer, *file).await
            }
            OrchestratorInstruction::ReceiveFile { stream, file_name, file_size, checksum, result } => {
                self.handle_receive_file(*stream, *file_name, *file_size, *checksum, *result).await
            }
            OrchestratorInstruction::PlanPipeline { model_info, inference_id, result } => {
                self.handle_plan_pipeline(*model_info, *inference_id, *result).await
            }
            OrchestratorInstruction::EstablishStreams { plan, result } => {
                self.handle_establish_streams(*plan, *result).await
            }
            OrchestratorInstruction::JoinWorkers { plan, result } => {
                self.handle_join_workers(*plan, *result).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm_base::{ConstValue, SlotId};
    use crate::event_bus::EventBus;
    use crate::storage::StorageManager;
    use crate::llm_io::LLM_IO_Broker;
    use crate::tensor_io::Tensor_Port_Switch;
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager, StubScheduler};
    use crate::ml_engine::capability::ML_Engine_Capability;
    use crate::ml_engine::ml_thread_engine_instruction::Model_Info;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicBool;

    struct StubMLEngine;
    #[async_trait]
    impl ML_Engine_Capability for StubMLEngine {
        async fn Create_Session(&self, _config: crate::ml_engine::capability::ML_Session_Config, _io: crate::llm_io::IoHandle) -> Result<Model_Info, crate::ml_engine::capability::ML_Engine_Error> { unimplemented!("stub") }
        async fn Shutdown_Session(&self, _session_id: &str) -> Result<(), crate::ml_engine::capability::ML_Engine_Error> { unimplemented!("stub") }
        async fn Run_Program(&self, _session_id: &str, _program: Vec<crate::ml_engine::ml_thread_engine_instruction::Instruction>, _params: crate::ml_engine::ml_thread_engine_instruction::Pipeline_Params, _cancel: Arc<AtomicBool>) -> Result<crate::ml_engine::ml_thread_engine_instruction::Pipeline_Result, crate::ml_engine::capability::ML_Engine_Error> { unimplemented!("stub") }
        async fn Analyze_Model(&self, _model_file_id: &str) -> Result<Model_Info, crate::ml_engine::capability::ML_Engine_Error> { unimplemented!("stub") }
        async fn Split_Model(&self, _source_file_id: &str, _start: usize, _end: usize, _output_file_id: &str) -> Result<(), crate::ml_engine::capability::ML_Engine_Error> { unimplemented!("stub") }
    }

    async fn stub_caps() -> (Arc<Capabilities>, tempfile::TempDir) {
        let temp_dir = tempfile::TempDir::new().unwrap();
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

    #[tokio::test]
    async fn empty_program_done() {
        let (caps, _tmp) = stub_caps().await;
        let mut ovm = Orchestrator_VM::new(JobId(999), caps);
        ovm.load(vec![]);
        assert_eq!(ovm.vm.ip, 0);
    }

    #[tokio::test]
    async fn const_then_step() {
        let (caps, _tmp) = stub_caps().await;
        let mut ovm = Orchestrator_VM::new(JobId(999), caps);
        ovm.load(vec![
            OrchestratorInstruction::Const { value: ConstValue::U64(42), dst: SlotId(0) },
        ]);
        let r = ovm.step().await;
        assert_eq!(r, StepResult::Continue);
        assert_eq!(ovm.vm.ip, 1);
        assert_eq!(ovm.vm.slots.get_u64(SlotId(0)).unwrap(), 42);
    }

    #[tokio::test]
    async fn const_move_chain() {
        let (caps, _tmp) = stub_caps().await;
        let mut ovm = Orchestrator_VM::new(JobId(999), caps);
        ovm.load(vec![
            OrchestratorInstruction::Const { value: ConstValue::String("hi".into()), dst: SlotId(0) },
            OrchestratorInstruction::Move { src: SlotId(0), dst: SlotId(1) },
        ]);
        ovm.step().await;
        ovm.step().await;
        assert_eq!(ovm.vm.ip, 2);
        assert_eq!(ovm.vm.slots.get_string(SlotId(1)).unwrap(), "hi");
    }

    #[tokio::test]
    async fn jump_to_target() {
        let (caps, _tmp) = stub_caps().await;
        let mut ovm = Orchestrator_VM::new(JobId(999), caps);
        ovm.load(vec![
            OrchestratorInstruction::Const { value: ConstValue::U64(1), dst: SlotId(0) },
            OrchestratorInstruction::Jump { target: 3 },
            OrchestratorInstruction::Const { value: ConstValue::U64(2), dst: SlotId(0) },
            OrchestratorInstruction::Const { value: ConstValue::U64(3), dst: SlotId(0) },
        ]);
        ovm.step().await;
        ovm.step().await;
        ovm.step().await;
        assert_eq!(ovm.vm.slots.get_u64(SlotId(0)).unwrap(), 3);
    }

    #[tokio::test]
    async fn jump_if_conditional() {
        let (caps, _tmp) = stub_caps().await;
        let mut ovm = Orchestrator_VM::new(JobId(999), caps);
        ovm.load(vec![
            OrchestratorInstruction::Const { value: ConstValue::Bool(true), dst: SlotId(10) },
            OrchestratorInstruction::JumpIf { condition: SlotId(10), target: 3 },
            OrchestratorInstruction::Const { value: ConstValue::U64(1), dst: SlotId(0) },
            OrchestratorInstruction::Const { value: ConstValue::U64(2), dst: SlotId(0) },
        ]);
        for _ in 0..3 {
            ovm.step().await;
        }
        assert_eq!(ovm.vm.slots.get_u64(SlotId(0)).unwrap(), 2);
    }

    #[tokio::test]
    async fn add_increment() {
        let (caps, _tmp) = stub_caps().await;
        let mut ovm = Orchestrator_VM::new(JobId(999), caps);
        ovm.load(vec![
            OrchestratorInstruction::Const { value: ConstValue::F64(0.0), dst: SlotId(0) },
            OrchestratorInstruction::Add { dst: SlotId(0), delta: 5.0 },
        ]);
        ovm.step().await;
        ovm.step().await;
        assert_eq!(ovm.vm.slots.get_f64(SlotId(0)).unwrap(), 5.0);
    }
}
