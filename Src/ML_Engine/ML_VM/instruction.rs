//Presented by KeJi
//Date ： 2026-05-11

//! ML_VM 指令集（扁平，基于槽位）
//!
//! 公共指令平铺进枚举，不通过 Base(BaseInstruction) 嵌套。
//! Loop/BreakIf 已由 Jump/JumpIf 替代，Set/CopyMeta 已由 Const/Move 替代。

use crate::vm_base::{ConstValue, SlotId};

// ─── InferenceInputType ───────────────────────────────────────

/// Inference 指令的输入来源类型。
///
/// 决定了 handler 如何读取输入槽位以及 META1 的递增策略：
/// - Tokens: 从 Token IDs 槽位读取 → embedding → forward, META1 += 1
/// - Tensor: 从 Tensor 槽位读取 → 直接 forward, META1 += seq_len
#[derive(Debug, Clone)]
pub enum InferenceInputType {
    Tokens,
    Tensor,
}

// ─── MlInstruction ────────────────────────────────────────────

/// ML 执行引擎指令（基于槽位架构，扁平）
///
/// 公共指令 Const/Move/Add/Jump/JumpIf 委托给 Vm_Base::Vm.handler，
/// 领域指令由 ML_VM.step() 分发到对应 handler。
#[derive(Debug, Clone)]
pub enum MlInstruction {
    // ─── 公共指令（委托给 Vm）──────────────────────────────
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
    Sub {
        src: SlotId,
        dst: SlotId,
    },
    Timer {
        slot: SlotId,
    },

    // ─── 数据输入 ──────────────────────────────────────────
    Input,

    // ─── 编解码 ────────────────────────────────────────────
    Encode,
    Decode,

    // ─── 推理 ──────────────────────────────────────────────
    Prefill {
        input: SlotId,
    },
    Inference {
        input_type: InferenceInputType,
        input: SlotId,
    },

    // ─── 采样 ──────────────────────────────────────────────
    Sample {
        tensor_slot: SlotId,
    },

    // ─── Control I/O ───────────────────────────────────────
    Output,
    EndOutput,

    // ─── 网络 I/O ──────────────────────────────────────────
    Send,
    Receive,
    SendEOF,

    // ─── Profile ───────────────────────────────────────────
    FillTensor {
        dst: SlotId,
        shape_slots: Vec<SlotId>,
        value: f32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_enum_variants_exist() {
        let inst = MlInstruction::Input;
        assert!(matches!(inst, MlInstruction::Input));

        let inst = MlInstruction::Encode;
        assert!(matches!(inst, MlInstruction::Encode));

        let inst = MlInstruction::Prefill {
            input: SlotId(1012),
        };
        assert!(matches!(inst, MlInstruction::Prefill { .. }));

        let inst = MlInstruction::Inference {
            input_type: InferenceInputType::Tokens,
            input: SlotId(1011),
        };
        match inst {
            MlInstruction::Inference { input_type, input } => {
                assert!(matches!(input_type, InferenceInputType::Tokens));
                assert_eq!(input, SlotId(1011));
            }
            _ => panic!("expected Inference"),
        }
    }
}
