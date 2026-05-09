//Presented by KeJi
//Date ： 2026-05-09

//! Orchestrator 领域槽位系统。
//!
//! 与 Vm_Base::SlotFile 同构——一个 HashMap<SlotId, T>，每个 SlotId 全局唯一。
//! 基础类型走 Vm.slots，领域类型走 OrchestratorSlots。

use crate::llm_io::IoHandle;
use crate::ml_engine::ml_thread_engine_instruction::Model_Info;
use crate::scheduler::Pipeline_Plan;
use crate::tensor_io::Tensor_IO_Endpoint;
use crate::vm_base::SlotId;
use std::collections::HashMap;
use std::sync::Mutex;

/// Phase 2 ML Session 句柄的占位符。
/// 后续替换为实际的 Session 句柄类型。
#[derive(Debug, Clone)]
pub struct SessionHandle;

#[derive(Debug)]
pub enum OrchestratorSlotValue {
    IoHandle(IoHandle),
    SessionHandle(SessionHandle),
    Stream(Mutex<Option<libp2p::Stream>>),
    TensorIO(Tensor_IO_Endpoint),
    PipelinePlan(Pipeline_Plan),
    ModelInfo(Model_Info),
}

#[derive(Debug, Default)]
pub struct OrchestratorSlots {
    slots: HashMap<SlotId, OrchestratorSlotValue>,
}

impl OrchestratorSlots {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }

    pub fn set(&mut self, slot: SlotId, value: OrchestratorSlotValue) {
        self.slots.insert(slot, value);
    }

    pub fn take(&mut self, slot: SlotId) -> Option<OrchestratorSlotValue> {
        self.slots.remove(&slot)
    }

    /// 设置 Stream 槽位。
    pub fn set_stream(&mut self, slot: SlotId, stream: libp2p::Stream) {
        self.set(
            slot,
            OrchestratorSlotValue::Stream(Mutex::new(Some(stream))),
        );
    }

    // ─── 类型化 take ──────────────────────────────────────

    pub fn take_io_handle(&mut self, slot: SlotId) -> Option<IoHandle> {
        match self.take(slot) {
            Some(OrchestratorSlotValue::IoHandle(h)) => Some(h),
            _ => None,
        }
    }

    pub fn take_session(&mut self, slot: SlotId) -> Option<SessionHandle> {
        match self.take(slot) {
            Some(OrchestratorSlotValue::SessionHandle(h)) => Some(h),
            _ => None,
        }
    }

    pub fn take_stream(&mut self, slot: SlotId) -> Option<libp2p::Stream> {
        match self.take(slot) {
            Some(OrchestratorSlotValue::Stream(m)) => m.into_inner().unwrap(),
            _ => None,
        }
    }

    pub fn take_tensor_io(&mut self, slot: SlotId) -> Option<Tensor_IO_Endpoint> {
        match self.take(slot) {
            Some(OrchestratorSlotValue::TensorIO(t)) => Some(t),
            _ => None,
        }
    }

    pub fn take_pipeline_plan(&mut self, slot: SlotId) -> Option<Pipeline_Plan> {
        match self.take(slot) {
            Some(OrchestratorSlotValue::PipelinePlan(p)) => Some(p),
            _ => None,
        }
    }

    pub fn take_model_info(&mut self, slot: SlotId) -> Option<Model_Info> {
        match self.take(slot) {
            Some(OrchestratorSlotValue::ModelInfo(m)) => Some(m),
            _ => None,
        }
    }

    // ─── 只读 get ─────────────────────────────────────────

    pub fn get_pipeline_plan(&self, slot: SlotId) -> Option<&Pipeline_Plan> {
        match self.slots.get(&slot) {
            Some(OrchestratorSlotValue::PipelinePlan(p)) => Some(p),
            _ => None,
        }
    }

    pub fn get_model_info(&self, slot: SlotId) -> Option<&Model_Info> {
        match self.slots.get(&slot) {
            Some(OrchestratorSlotValue::ModelInfo(m)) => Some(m),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm_base::SlotId;

    #[test]
    fn set_and_take_roundtrip() {
        let mut slots = OrchestratorSlots::new();
        let handle = SessionHandle;
        slots.set(SlotId(0), OrchestratorSlotValue::SessionHandle(handle));
        assert!(slots.take_session(SlotId(0)).is_some());
        assert!(slots.take_session(SlotId(0)).is_none());
    }

    #[test]
    fn take_wrong_type_returns_none() {
        let mut slots = OrchestratorSlots::new();
        slots.set(
            SlotId(0),
            OrchestratorSlotValue::SessionHandle(SessionHandle),
        );
        assert!(slots.take_io_handle(SlotId(0)).is_none());
    }

    #[test]
    fn take_nonexistent_returns_none() {
        let mut slots = OrchestratorSlots::new();
        assert!(slots.take_session(SlotId(99)).is_none());
    }

    #[test]
    fn multiple_slots_independent() {
        let mut slots = OrchestratorSlots::new();
        slots.set(
            SlotId(0),
            OrchestratorSlotValue::SessionHandle(SessionHandle),
        );
        slots.set(
            SlotId(1),
            OrchestratorSlotValue::SessionHandle(SessionHandle),
        );
        assert!(slots.take_session(SlotId(0)).is_some());
        assert!(slots.get_model_info(SlotId(0)).is_none());
    }
}
