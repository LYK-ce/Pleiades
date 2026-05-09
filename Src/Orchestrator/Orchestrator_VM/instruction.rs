//Presented by KeJi
//Date ： 2026-05-09

//! Orchestrator 指令集。
//!
//! 公共指令（Const/Move/Add/Jump/JumpIf）平铺在枚举中，委托给 Vm_Base::Vm 处理。
//! 领域指令由 Orchestrator_VM 自己的 handler 实现。

use crate::vm_base::{ConstValue, SlotId};

#[derive(Debug, Clone)]
pub enum OrchestratorInstruction {
    // ─── 公共指令（委托给 Vm_Base::Vm）──────────────────────
    Const {
        value: ConstValue,
        dst: SlotId,
    },
    Move {
        src: SlotId,
        dst: SlotId,
    },
    Add {
        dst: SlotId,
        delta: f64,
    },
    Jump {
        target: usize,
    },
    JumpIf {
        condition: SlotId,
        target: usize,
    },

    // ─── 推理生命周期 ──────────────────────────────────────
    CreateSession {
        model: SlotId,
        device: SlotId,
        start: SlotId,
        end: SlotId,
        io: SlotId,
        tensor_io: Option<SlotId>,
        result: SlotId,
    },
    ShutdownSession {
        session: SlotId,
    },
    RunProgram {
        session: SlotId,
        result: SlotId,
    },
    AnalyzeModel {
        model: SlotId,
        result: SlotId,
    },
    SplitModel {
        source: SlotId,
        start: SlotId,
        end: SlotId,
        output: SlotId,
    },

    // ─── 网络操作 ──────────────────────────────────────────
    SendFile {
        peer: SlotId,
        file: SlotId,
    },
    ReceiveFile {
        stream: SlotId,
        file_name: SlotId,
        file_size: SlotId,
        checksum: SlotId,
        result: SlotId,
    },

    // ─── Pipeline 规划 ─────────────────────────────────────
    PlanPipeline {
        model_info: SlotId,
        inference_id: SlotId,
        result: SlotId,
    },

    // ─── Pipeline 编排 ─────────────────────────────────────
    EstablishStreams {
        plan: SlotId,
        result: SlotId,
    },
    JoinWorkers {
        plan: SlotId,
        result: SlotId,
    },
}
