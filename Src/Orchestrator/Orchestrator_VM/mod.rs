//Presented by KeJi
//Date ： 2026-05-09

pub mod instruction;
pub mod slots;
pub mod engine;
pub mod inference_handler;
pub mod network_handler;
pub mod scheduler_handler;

pub use instruction::OrchestratorInstruction;
pub use slots::{OrchestratorSlots, OrchestratorSlotValue, SessionHandle};
pub use engine::Orchestrator_VM;

/// 将旧 orchestrator::slot::SlotId 转换为 vm_base::SlotId。
/// 后续统一为 vm_base::SlotId 后移除。
pub(crate) fn to_vm_slot(id: crate::orchestrator::slot::SlotId) -> crate::vm_base::SlotId {
    crate::vm_base::SlotId(id.0)
}
