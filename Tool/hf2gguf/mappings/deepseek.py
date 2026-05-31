# DeepSeek tensor name mapping: HuggingFace safetensors → GGUF
#
# Presented by KeJi
# Date: 2026-05-31
#
# DeepSeek V3 uses MLA (Multi-head Latent Attention) with a different
# weight structure than standard transformers. This is a STUB —
# the actual mapping needs to be verified against real DeepSeek safetensors.
#
# Reference: Src/ML_Engine/GGUF_Models/deepseek_v3.rs

DEEPSEEK_TENSOR_MAP: dict[str, str] = {
    # === Embedding ===
    "model.embed_tokens.weight": "token_embd.weight",

    # === MLA Attention (per layer) ===
    # DeepSeek uses compressed KV: q_a, kv_a, q_b, kv_b, etc.
    # These need to be verified against actual safetensors tensor names.
    "model.layers.{n}.self_attn.q_a_proj.weight":   "blk.{n}.attn_q_a.weight",
    "model.layers.{n}.self_attn.q_b_proj.weight":   "blk.{n}.attn_q_b.weight",
    "model.layers.{n}.self_attn.kv_a_proj_with_mqa.weight": "blk.{n}.attn_kv_a_mqa.weight",
    "model.layers.{n}.self_attn.kv_b_proj.weight":  "blk.{n}.attn_kv_b.weight",
    "model.layers.{n}.self_attn.o_proj.weight":     "blk.{n}.attn_output.weight",

    # === Layer Norms ===
    "model.layers.{n}.input_layernorm.weight":          "blk.{n}.attn_norm.weight",
    "model.layers.{n}.post_attention_layernorm.weight": "blk.{n}.ffn_norm.weight",

    # === MoE (DeepSeekMoE) ===
    "model.layers.{n}.mlp.gate.wg.weight":              "blk.{n}.ffn_gate_inp.weight",
    "model.layers.{n}.mlp.experts.gate_proj.weight":    "blk.{n}.ffn_gate_exps.weight",
    "model.layers.{n}.mlp.experts.up_proj.weight":      "blk.{n}.ffn_up_exps.weight",
    "model.layers.{n}.mlp.experts.down_proj.weight":    "blk.{n}.ffn_down_exps.weight",

    # === Shared Expert ===
    "model.layers.{n}.mlp.shared_experts.gate_proj.weight": "blk.{n}.ffn_gate_shexp.weight",
    "model.layers.{n}.mlp.shared_experts.up_proj.weight":   "blk.{n}.ffn_up_shexp.weight",
    "model.layers.{n}.mlp.shared_experts.down_proj.weight": "blk.{n}.ffn_down_shexp.weight",

    # === Output ===
    "model.norm.weight": "output_norm.weight",
    "lm_head.weight":    "output.weight",
}

# HF architectures that map to DeepSeek
DEEPSEEK_HF_ARCHITECTURES: set[str] = {
    "DeepseekV3ForCausalLM",
    "DeepSeekV3ForCausalLM",
    "DeepseekForCausalLM",
    "DeepSeekForCausalLM",
}
