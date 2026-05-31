# Qwen3MoE tensor name mapping: HuggingFace safetensors → GGUF
#
# Presented by KeJi
# Date: 2026-05-31

# Inherits all Qwen3 standard mappings plus MoE-specific tensors
from .qwen3 import QWEN3_TENSOR_MAP as _BASE

QWEN3_MOE_TENSOR_MAP: dict[str, str] = {
    **_BASE,
    # === MoE Routing ===
    "model.layers.{n}.mlp.gate.wg.weight": "blk.{n}.ffn_gate_inp.weight",

    # === MoE Expert Weights (shared across experts) ===
    "model.layers.{n}.mlp.experts.gate_proj.weight": "blk.{n}.ffn_gate_exps.weight",
    "model.layers.{n}.mlp.experts.up_proj.weight":   "blk.{n}.ffn_up_exps.weight",
    "model.layers.{n}.mlp.experts.down_proj.weight": "blk.{n}.ffn_down_exps.weight",
}

# Additional GGUF metadata for MoE
QWEN3_MOE_EXTRA_CONFIG: dict[str, str] = {
    "num_experts":           "expert_count",
    "num_experts_per_tok":   "expert_used_count",
    "intermediate_size":     "expert_feed_forward_length",
}

# HF architectures that map to Qwen3MoE
QWEN3_MOE_HF_ARCHITECTURES: set[str] = {
    "Qwen3MoeForCausalLM",
    "Qwen3MoEForCausalLM",
}
