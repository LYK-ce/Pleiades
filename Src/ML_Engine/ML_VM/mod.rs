//Presented by KeJi
//Date ： 2026-05-11

//! ML_VM 模块 — 基于 Vm_Base::Vm 的 ML 执行引擎。

pub mod engine;
pub mod instruction;
pub mod slots;

pub use engine::ML_VM;
pub use instruction::{InferenceInputType, MlInstruction};
pub use slots::{MlSlotValue, MlSlots};
