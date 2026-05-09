//Presented by KeJi
//Date ： 2026-05-09

pub mod slot;
pub mod vm;

pub use slot::{ConstValue, SlotFile, SlotId, SlotValue};
pub use vm::{BaseInstruction, StepResult, Vm};
