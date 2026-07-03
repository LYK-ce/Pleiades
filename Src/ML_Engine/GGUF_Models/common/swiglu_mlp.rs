//Presented by KeJi
//Created Date ： 2026-06-17
//Modified Date ： 2026-06-17

//! 标准 SiLU-gated MLP 权重
//!
//! gate_proj / up_proj / down_proj 三层结构，被 Qwen3、Llama、DeepSeek V3 共用。

use candle_core::quantized::QTensor;
use candle_core::{Result, Tensor};
use candle_nn::{Activation, Module};
use candle_transformers::models::with_tracing::QMatMul;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Mlp_Weights {
    pub gate_proj: QMatMul,
    pub up_proj: QMatMul,
    pub down_proj: QMatMul,
    pub act_fn: Activation,
    pub(crate) span: tracing::Span,
}

impl Mlp_Weights {
    /// 从已提取的 QTensors 构建 Dense FFN
    pub fn Build_From_Extracted(tensors: &mut HashMap<String, QTensor>, prefix: &str) -> Result<Self> {
        let gate_proj = {
            let qt = tensors.remove(&format!("{prefix}.ffn_gate.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_gate.weight")))?;
            QMatMul::from_weights(Arc::new(qt))
                .map_err(|e| candle_core::Error::Msg(format!("ffn_gate: {e}")))?
        };
        let up_proj = {
            let qt = tensors.remove(&format!("{prefix}.ffn_up.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_up.weight")))?;
            QMatMul::from_weights(Arc::new(qt))
                .map_err(|e| candle_core::Error::Msg(format!("ffn_up: {e}")))?
        };
        let down_proj = {
            let qt = tensors.remove(&format!("{prefix}.ffn_down.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_down.weight")))?;
            QMatMul::from_weights(Arc::new(qt))
                .map_err(|e| candle_core::Error::Msg(format!("ffn_down: {e}")))?
        };
        Ok(Self {
            gate_proj, up_proj, down_proj,
            act_fn: Activation::Silu,
            span: tracing::span!(tracing::Level::TRACE, "mlp"),
        })
    }
}

impl Module for Mlp_Weights {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let gate = self.gate_proj.forward(x)?.apply(&self.act_fn)?;
        let up = self.up_proj.forward(x)?;
        let gated = (gate * up)?;
        self.down_proj.forward(&gated)
    }
}
