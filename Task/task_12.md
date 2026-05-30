# Task 12: DeepSeek V3.2 模型架构支持

> Presented by KeJi
> Date: 2026-05-30

## 描述

在 ML Engine 中新增 DeepSeek V3.2 架构支持，使其能够加载 GGUF 格式的 DeepSeek V3.2 模型并进行推理。DeepSeek V3.2 与当前已支持的 Qwen3 Dense 架构有本质差异：MLA（Multi-head Latent Attention）压缩 KV Cache、DeepSeekMoE 细粒度专家路由。

## 子任务总览

| # | 任务 | 涉及文件数 | 状态 |
|---|------|-----------|------|
| 12.1 | 调研 MLA 注意力 + DeepSeekMoE 参考实现 | 0 | ⬜ |
| 12.2 | 新建 `deepseek_v3.rs` — 模型权重结构体 | 1 | ⬜ |
| 12.3 | 集成到 `GGUF_Load_Model` + `MlSession` | 3 | ⬜ |
| 12.4 | Lua 侧验证 + 编写测试脚本 | 1 | ⬜ |

---

## 详细实施计划

### 12.1 调研 MLA + DeepSeekMoE 参考实现

#### 参考源 1: `DevJadhav/deepseek-from-scratch` (Rust/candle)

- **仓库**: https://github.com/DevJadhav/deepseek-from-scratch
- **关键文件**: `rust-src/src/model/`
  - `mla.rs` — Multi-head Latent Attention 实现（压缩 KV Cache → 减少显存）
  - `moe.rs` — DeepSeekMoE（细粒度专家 + 共享专家 + top-k 路由）
  - `attention.rs` — MQA/GQA/MLA 多种注意力统一接口
  - `sparse_attention.rs` — DeepSeek Sparse Attention（混合局部+膨胀全局注意力）
  - `mtp.rs` — Multi-Token Prediction（并行预测多个 future token）

#### 参考源 2: `mistral.rs` (EricLBuehler)

- **仓库**: https://github.com/EricLBuehler/mistral.rs
- 已支持 DeepSeek V2 GGUF 加载，GGUF tensor 命名解析、MLA 解码逻辑可直接参考
- 与 Pleiades 的 candle + GGUF 模式更接近

#### 需要理解的核心差异

| 特性 | Qwen3 Dense (现有) | DeepSeek V3.2 |
|------|-------------------|---------------|
| **注意力** | 标准 MHA (Q/K/V/O + QK Norm) | MLA: Q/K 压缩到低秩 latent space，减少 KV Cache |
| **FFN** | SwiGLU (gate/up/down) | MoE: N 个细粒度 expert + 共享 expert + top-k 路由 |
| **KV Cache** | `ConcatKvCache` (candle 标准) | 需要 MLA 专用 cache（压缩后的 latent KV） |
| **Head 结构** | 标准 num_heads / num_kv_heads | MLA 有额外的 `q_lora_rank` / `kv_lora_rank` |
| **RoPE** | 标准 RoPE (q, k) | MLA 只对部分维度做 RoPE (decoupled RoPE) |
| **Aux Loss** | 无 | 专家负载均衡 loss + 设备级均衡 loss |

#### GGUF Tensor 命名差异

```
Qwen3 Dense:                  DeepSeek V3.2:
blk.N.attn_q.weight           blk.N.attn.q_a.weight         ← Q 压缩投影
blk.N.attn_k.weight           blk.N.attn.kv_a.weight        ← KV 联合压缩
blk.N.attn_v.weight           blk.N.attn.kv_b.weight        ← KV 解压投影
blk.N.attn_output.weight      blk.N.attn.o.weight           ← 输出投影
blk.N.attn_q_norm.weight      (无 QK Norm，MLA 不需要)
blk.N.attn_k_norm.weight
blk.N.ffn_gate.weight         blk.N.ffn_gate.weight          ← 共享 expert
blk.N.ffn_up.weight           blk.N.ffn_up.weight
blk.N.ffn_down.weight         blk.N.ffn_down.weight
(无)                          blk.N.ffn_gate.{e}.weight      ← 细粒度 expert gate
(无)                          blk.N.ffn_up.{e}.weight
(无)                          blk.N.ffn_down.{e}.weight
(无)                          blk.N.ffn_gate_s.weight        ← 共享 expert
(无)                          blk.N.ffn_up_s.weight
(无)                          blk.N.ffn_down_s.weight
(无)                          blk.N.attn.router.weight       ← MoE 路由
token_embd.weight             token_embd.weight
output_norm.weight            output_norm.weight
output.weight                 output.weight
```

---

### 12.2 新建 `deepseek_v3.rs` — 模型权重结构体

**文件**: `Src/ML_Engine/GGUF_Models/deepseek_v3.rs`

#### 12.2a MLA 注意力权重

```rust
/// MLA 压缩投影权重（在 latent space 中做 attention）
pub struct MLA_Weights {
    // Q 路径: x → q_a (压缩) → q_norm → q_b (解压)
    pub q_a: QMatMul,          // [hidden, q_lora_rank]  Q 压缩
    pub q_norm: RmsNorm,       // LayerNorm for Q
    pub q_b: QMatMul,          // [q_lora_rank, n_heads * head_dim]  Q 解压

    // KV 路径: x → kv_a (联合压缩) → kv_norm → kv_b (解压为 K+V)
    pub kv_a: QMatMul,         // [hidden, kv_lora_rank + qk_rope_dim]  KV 联合压缩
    pub kv_norm: RmsNorm,      // LayerNorm for KV
    pub kv_b: QMatMul,         // [kv_lora_rank, n_heads * (head_dim + k_rope_dim + v_dim)]

    // 输出投影
    pub o_proj: QMatMul,       // [n_heads * v_dim, hidden]

    // 配置
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub q_lora_rank: usize,    // Q 压缩秩
    pub kv_lora_rank: usize,   // KV 压缩秩
    pub qk_rope_dim: usize,    // 解耦 RoPE 维度（只对这部分做旋转编码）
    pub head_dim: usize,       // 非 RoPE 的 head 维度
    pub v_head_dim: usize,     // V 的 head 维度

    // RoPE
    pub rotary: Arc<Rotary_Embedding>,

    // KV Cache (存储压缩后的 latent KV)
    pub kv_cache: MLA_KV_Cache,
}
```

#### 12.2b MLA KV Cache

与 candle 标准 `ConcatKvCache` 不同，MLA 缓存的是压缩后的 `kv_latent` 和解耦的 `k_pe`：

```rust
pub struct MLA_KV_Cache {
    // 压缩后的 KV latent（核心节省显存的部分）
    kv_latent: Option<Tensor>,    // [batch, seq, kv_lora_rank]
    // 解耦的 RoPE key（只需要这小部分做位置编码）
    k_pe: Option<Tensor>,         // [batch, 1, seq, qk_rope_dim]
    capacity: usize,
}
```

#### 12.2c DeepSeekMoE

```rust
pub struct DeepSeekMoE_Weights {
    // 共享 expert（始终激活）
    pub shared_gate: QMatMul,
    pub shared_up: QMatMul,
    pub shared_down: QMatMul,

    // 细粒度 experts
    pub expert_gates: Vec<QMatMul>,     // [n_routed_experts]
    pub expert_ups: Vec<QMatMul>,
    pub expert_downs: Vec<QMatMul>,

    // 路由权重
    pub router: QMatMul,               // [hidden, n_routed_experts]

    // 配置
    pub n_routed_experts: usize,       // 细粒度 expert 数量
    pub n_shared_experts: usize,       // 共享 expert 数量（通常 1）
    pub top_k: usize,                  // 每 token 激活的 expert 数
    pub norm_topk_prob: bool,          // 是否归一化 top-k 概率
}
```

#### 12.2d Transformer 层

```rust
pub struct DeepSeek_Layer {
    pub mla: MLA_Weights,
    pub moe: DeepSeekMoE_Weights,
    pub ln1: RmsNorm,     // pre-attention norm
    pub ln2: RmsNorm,     // pre-MoE norm
}

pub struct DeepSeek_Model {
    pub embed: Embedding,
    pub layers: Vec<DeepSeek_Layer>,
    pub norm: RmsNorm,
    pub lm_head: QMatMul,
}
```

### 12.3 集成到已有框架

#### 12.3a `Src/ML_Engine/gguf_model.rs` — `GGUF_Load_Model` 扩展

当前 `GGUF_Load_Model` 按 `architecture` 分发。需要新增 `"deepseek_v3"` 分支：

```rust
match arch_info.architecture.as_str() {
    "qwen3" | "qwen2" => {
        // 现有 Qwen3 Dense 加载逻辑
    }
    "deepseek_v3" => {
        // 新: DeepSeek V3.2 加载逻辑
        // 1. 提取 MLA 配置 (q_lora_rank, kv_lora_rank, qk_rope_dim)
        // 2. 提取 MoE 配置 (n_routed_experts, top_k)
        // 3. 逐层加载 MLA + MoE 权重
        // 4. 组装 DeepSeek_Model
    }
    _ => bail!("unsupported architecture: {}", arch_info.architecture),
}
```

#### 12.3b `Src/ML_Engine/gguf_model.rs` — `GGUF_Load_Layer` 扩展

或新增 `GGUF_Load_DeepSeek_Layer` 函数按 `blk.N.attn.*` + `blk.N.ffn_gate.{e}.*` 前缀加载。

#### 12.3c `Src/ML_Engine/context.rs` — `MlSession` 扩展

`MlSession` 的 `load_model` / `forward` 需要支持 DeepSeek 的 forward 路径：
- 当前 `forward` 调用 `GGUF_Model_Inference` → `Model_Weights::Forward`
- 需新增 dispatch: 根据模型类型调用 `DeepSeek_Model::Forward`

### 12.4 验证

#### 12.4a 测试脚本 `programs/user/ds_test.lua`

```lua
COMMAND = "ds_test"
function execute(params)
    local model = params.model or "DeepSeek-V3.2-Lite.pgguf"
    local handle = caps.storage_acquire_read(model)
    local path = handle:path()
    local info = ml.analyze_model(path)
    caps.print("架构: " .. (info.architecture or "unknown"))

    local sess = ml.new("cuda")
    sess:load_model(path, 0, info.num_layers + 1)
    handle:release()

    -- 简单测试 forward
    local tokens = sess:encode("Hello")
    local t = sess:tensorize(tokens)
    local logits = sess:forward(t, 0)
    caps.print("forward 完成, logits: [" .. table.concat(logits:dims(), ",") .. "]")
end
```

#### 12.4b 验证步骤

1. 下载 DeepSeek-V3.2-Lite GGUF（~15GB，比完整版小很多）
2. `flush` → `exec ds_test model=DeepSeek-V3.2-Lite.pgguf`
3. 确认架构识别正确、forward 无报错、logits 维度正确

---

## 备注

- 基分支: `reforge`
- 优先使用 **DeepSeek-V3.2-Lite** 测试（参数量小，GGUF 约 15GB）
- 第一阶段只实现 prefill + 单 token decode，暂不实现 MTP speculative decoding
- 暂不实现 DeepSeek Sparse Attention（超长上下文优化），先用标准 causal mask
- MLA KV Cache 采用 "吸收" 策略：prefill 时计算完整 K/V 并缓存压缩 latent，decode 时只算增量
- MoE 的 expert 负载均衡 loss 在推理阶段不需要（仅训练用）
- candidate 参考文件: `DevJadhav/deepseek-from-scratch/rust-src/src/model/mla.rs` + `moe.rs`
