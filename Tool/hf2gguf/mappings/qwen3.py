# Qwen3 tensor name mapping: HuggingFace safetensors → GGUF
#
# Presented by KeJi
# Date: 2026-05-31
#
# Based on Pleiades GGUF_Load_Model expected tensor names (see Src/ML_Engine/gguf_model.rs).

QWEN3_TENSOR_MAP: dict[str, str] = {
    # === Embedding ===
    "model.embed_tokens.weight": "token_embd.weight",

    # === Attention (per layer, {n} = layer index) ===
    "model.layers.{n}.self_attn.q_proj.weight":   "blk.{n}.attn_q.weight",
    "model.layers.{n}.self_attn.k_proj.weight":   "blk.{n}.attn_k.weight",
    "model.layers.{n}.self_attn.v_proj.weight":   "blk.{n}.attn_v.weight",
    "model.layers.{n}.self_attn.o_proj.weight":   "blk.{n}.attn_output.weight",

    # === Q/K Norm (Qwen3 specific) ===
    "model.layers.{n}.self_attn.q_norm.weight":   "blk.{n}.attn_q_norm.weight",
    "model.layers.{n}.self_attn.k_norm.weight":   "blk.{n}.attn_k_norm.weight",

    # === Layer Norms ===
    "model.layers.{n}.input_layernorm.weight":          "blk.{n}.attn_norm.weight",
    "model.layers.{n}.post_attention_layernorm.weight": "blk.{n}.ffn_norm.weight",

    # === MLP (Feed-Forward) ===
    "model.layers.{n}.mlp.gate_proj.weight": "blk.{n}.ffn_gate.weight",
    "model.layers.{n}.mlp.up_proj.weight":   "blk.{n}.ffn_up.weight",
    "model.layers.{n}.mlp.down_proj.weight": "blk.{n}.ffn_down.weight",

    # === Output ===
    "model.norm.weight": "output_norm.weight",
    "lm_head.weight":    "output.weight",
}

# HF config.json → GGUF metadata key mapping
QWEN3_CONFIG_TO_GGUF: dict[str, str] = {
    "num_hidden_layers":        "block_count",
    "hidden_size":              "embedding_length",
    "num_attention_heads":      "attention.head_count",
    "num_key_value_heads":      "attention.head_count_kv",
    "intermediate_size":        "feed_forward_length",
    "max_position_embeddings":  "context_length",
    "rms_norm_eps":             "attention.layer_norm_rms_epsilon",
    "rope_theta":               "rope.freq_base",
    "vocab_size":               "vocab_size",
}

# head_dim: if present in config.json, use it; otherwise derive from hidden_size / num_attention_heads
QWEN3_GGUF_KEY_HEAD_DIM = "attention.key_length"

# HF architectures that map to Qwen3 GGUF prefix
QWEN3_HF_ARCHITECTURES: set[str] = {
    "Qwen3ForCausalLM",
    "Qwen3Model",
}
