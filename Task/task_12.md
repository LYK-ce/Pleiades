# Task 12: DeepSeek V3.2 模型架构支持

> Presented by KeJi
> Date: 2026-05-30

## 描述

在 ML Engine 中新增 DeepSeek V3.2 架构支持，使其能够加载 GGUF 格式的 DeepSeek V3.2 模型并进行推理。DeepSeek V3.2 与当前已支持的 Qwen3 Dense 架构有本质差异：MLA（Multi-head Latent Attention）压缩 KV Cache、DeepSeekMoE 细粒度专家路由。

## 子任务总览

| # | 任务 | 涉及文件数 | 状态 |
|---|------|-----------|------|
| 12.0 | 创建 `deepseek` 分支 | 0 | ✅ |
| 12.1 | 调研 MLA 注意力 + DeepSeekMoE 参考实现 | 0 | ✅ |
| 12.2 | 新建 `deepseek_v3.rs` — 模型权重结构体 | 1 | ✅ |
| 12.3 | 集成到 `GGUF_Load_Model` + `MlSession` | 3 | ✅ |
| 12.5 | 支持 `cuda:0` / `cuda:1` 等多 GPU 设备选择 | 7 | ✅ |
| 12.6 | `pipe_5`/`pipe_6` — 2节点 × 2 GPU 流水线并行脚本 | 2 | ✅ |
| 12.7 | 集成 Qwen MoE 支持（基于 candle `quantized_qwen3_moe.rs`） | TBD | ⬜ |

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

---

### 12.5 支持 `cuda:0` / `cuda:1` 等多 GPU 设备选择

#### 架构原则

设备解析是 **ML Engine 的职责**，不属于 Lua VM 层。当前 `parse_device_str()` 放在 `lua_tensor.rs` 中是放错了地方。

#### 现状

所有 CUDA 设备都硬编码为 `Device::new_cuda(0)`：

| 文件 | 问题 |
|------|------|
| `Src/ML_Engine/context.rs` L108 | `MlSession::new()` 自己写了一段重复的设备解析，`"cuda" => new_cuda(0)` |
| `Src/ML_Engine/lua_tensor.rs` L141 | `parse_device_str()` 定义在此处（位置不对），`"cuda" => new_cuda(0)` |
| `Src/TUI/mod.rs` L697 | `set-device` 只接受 `cpu` / `cuda`，拒绝 `cuda:1` |
| `Src/Orchestrator/core/branch_user.rs` L46 | 帮助文本 `"set-device cpu|cuda"` |
| `Src/Orchestrator/mod.rs` L167 | `query_free_memory_mb()` 精确匹配 `== "cuda"` |

#### 实施计划

**1. 新建 `Src/ML_Engine/device.rs`** — ML Engine 唯一的设备解析入口：

```rust
//Presented by KeJi
//Date : 2026-05-30

//! 设备解析 — 将字符串转换为 candle Device
//!
//! ML Engine 对外暴露的唯一切入点，Lua VM 层不应自行解析设备。

use candle_core::Device;

/// 解析设备字符串，支持：
/// - "cpu"              → Device::Cpu
/// - "cuda" / "cuda:0"  → Device::new_cuda(0)
/// - "cuda:N"           → Device::new_cuda(N)
pub fn parse_device_str(s: &str) -> Result<Device, String> {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "cpu" => Ok(Device::Cpu),
        _ if s == "cuda" || s == "cuda:0" => {
            Device::new_cuda(0).map_err(|e| format!("cuda:0 unavailable: {e}"))
        }
        _ if s.starts_with("cuda:") => {
            let idx: usize = s[5..].parse()
                .map_err(|_| format!("invalid cuda device index: '{s}'"))?;
            Device::new_cuda(idx)
                .map_err(|e| format!("cuda:{idx} unavailable: {e}"))
        }
        _ => Err(format!("unknown device: '{s}'. Use 'cpu', 'cuda', or 'cuda:N'")),
    }
}
```

**2. `Src/ML_Engine/lua_tensor.rs`** — 删除原有的 `parse_device_str()`，改为 `use super::device::parse_device_str;`

**3. `Src/ML_Engine/context.rs`** — `MlSession::new()` 删除重复的 match，改为调用 `device::parse_device_str()`

**4. `Src/ML_Engine/mod.rs`** — 添加 `pub mod device;` 和 `pub use device::parse_device_str;`

**修改清单：**

| # | 文件 | 操作 |
|---|------|------|
| 1 | `Src/ML_Engine/device.rs` | **新建** — ML Engine 唯一设备解析入口 |
| 2 | `Src/ML_Engine/lua_tensor.rs` | 删除 `parse_device_str()`，改为 `use super::device` |
| 3 | `Src/ML_Engine/context.rs` | `MlSession::new()` 删除重复解析，复用 `device::parse_device_str` |
| 4 | `Src/ML_Engine/mod.rs` | `pub mod device;` + re-export |
| 5 | `Src/TUI/mod.rs` | 扩展验证：允许 `cuda:N` 格式 |
| 6 | `Src/Orchestrator/core/branch_user.rs` | 更新帮助文本 |
| 7 | `Src/Orchestrator/mod.rs` | `query_free_memory_mb()` 改用 `starts_with("cuda")` |

> 注：TUI/Orchestrator 层的改动为配套修改，核心逻辑全部收敛在 ML Engine 的 `device.rs` 中。

**向后兼容：**
- `"cuda"` 仍解析为 `cuda:0`，现有 Lua 脚本、TUI 命令、流水线协议无需修改

---

### 12.6 `pipe_5` / `pipe_6` — 2节点 × 2 GPU 流水线并行

#### 硬件环境

4 × A100-SXM4-80GB，每张 80GB 显存：

| 节点 | GPU | 本地设备名 | 用途 |
|------|-----|-----------|------|
| Node 1 | GPU 0,1 | `cuda:0`, `cuda:1` | 前半模型 (pipe_5) |
| Node 2 | GPU 2,3 | `cuda:0`, `cuda:1` | 后半模型 (pipe_6) |

通过 `CUDA_VISIBLE_DEVICES=0,1` / `CUDA_VISIBLE_DEVICES=2,3` 隔离。

#### 架构

```
Node 1 (pipe_5)                              Node 2 (pipe_6)
┌─────────────────────────┐      网络       ┌─────────────────────────┐
│  Session                │                 │                          │
│    │                    │                 │                          │
│    ▼                    │                 │                          │
│  GPU:0 (层 0 ~ mid_0)   │  hidden_state   │  GPU:0 (mid_1 ~ mid_2)  │
│    │ to_device("cuda:1") │ ──────────────▶ │    │ to_device("cuda:1") │
│    ▼                    │                 │    ▼                    │
│  GPU:1 (mid_0+1 ~ end)  │                 │  GPU:1 (mid_2+1 ~ end) │
│    │                    │                 │    │                    │
│    │ send_tensor ───────│──hidden────────▶│─── accept_tensor        │
│    │                    │                 │    │                    │
│    │ accept_tensor ◀────│──logits─────────│─── send_tensor          │
│    ▼                    │                 │    ▼                    │
│  Session ◀──────────────│                 │  (返回 logits)          │
└─────────────────────────┘                 └─────────────────────────┘
```

#### 与 pipe_1/pipe_2 的关键差异

| | pipe_1/2 | pipe_5/6 |
|---|---|---|
| GPU 数/节点 | 1 | 2 |
| 内部传输 | 无 | `tensor:to_device("cuda:1")` 跨 GPU 搬运 |
| 设备指定 | `ml.new("cuda")` | `ml.new("cuda:0")` / `ml.new("cuda:1")` |
| 层分割方式 | 无内部分割 | 分片内再对半分为 GPU:0 和 GPU:1 |

#### pipe_5 逻辑

```lua
-- 1. 读取模型，分析 split 元数据
-- 2. 将 [split_start, split_end] 对半分为 GPU:0 和 GPU:1 范围
-- 3. GPU:0 = ml.new("cuda:0"), 加载前半层
-- 4. GPU:1 = ml.new("cuda:1"), 加载后半层
-- 5. 连接 Session (local_tensor)
-- 6. rexec 启动 pipe_6
-- 7. 循环:
--    Session → recv_tensor("cuda:0")
--    → GPU:0:forward → to_device("cuda:1")
--    → GPU:1:forward
--    → network.send_tensor → pipe_6
--    ← network.recv_tensor ← pipe_6
--    → local_tensor.send_tensor → Session
```

#### pipe_6 逻辑

```lua
-- 1. 读取模型，分析 split 元数据
-- 2. 将 [split_start, split_end] 对半分为 GPU:0 和 GPU:1 范围
-- 3. GPU:0 = ml.new("cuda:0"), 加载前半层
-- 4. GPU:1 = ml.new("cuda:1"), 加载后半层
-- 5. 发现 pipe_5 节点
-- 6. 循环:
--    ← network.recv_tensor ← pipe_5
--    → GPU:0:forward → to_device("cuda:1")
--    → GPU:1:forward
--    → network.send_tensor → pipe_5
```

#### 涉及文件

| # | 文件 | 说明 |
|---|------|------|
| 1 | `programs/user/pipe_5.lua` | Node 1 脚本，2 GPU 前半段 |
| 2 | `programs/user/pipe_6.lua` | Node 2 脚本，2 GPU 后半段 |

---

### 12.7 集成 Qwen MoE 支持

#### 背景

candle-transformers 0.10.2 已内置 Qwen MoE 的三个参考实现：

| 文件 | 路径 | 适用场景 |
|------|------|---------|
| `qwen2_moe.rs` | `candle-transformers/src/models/` | Qwen2 MoE，shared expert + sparse experts |
| `qwen3_moe.rs` | `candle-transformers/src/models/` | Qwen3 MoE，CUDA 时自动走 `FusedMoe` kernel |
| **`quantized_qwen3_moe.rs`** | `candle-transformers/src/models/` | Qwen3 MoE **GGUF 量化**，直接对接 GGUF 权重 |

本项目 `Cargo.toml` 已启用 `default = ["cuda"]`，`FusedMoe` CUDA kernel 直接可用。

#### 架构分析

`quantized_qwen3_moe.rs` 的核心设计：

```rust
// MoE 层与 Dense 层混合 — 每 N 层出现一个 MoE
enum MoeOrMlp {
    FusedMoe(FusedMoeGGUF),  // MoE: gate_inp + shared_expert + experts
    Mlp(Mlp),                // Dense: gate/up/down SwiGLU
}

// FusedMoeGGUF 从 GGUF 加载的权重
struct FusedMoeGGUF {
    gate_inp: QMatMul,              // Router: [hidden → num_experts]
    shared_expert_gate: QMatMul,    // shared expert gate
    shared_expert_up: QMatMul,      // shared expert up
    shared_expert_down: QMatMul,    // shared expert down
    experts_gate: Vec<QMatMul>,     // [n_experts]
    experts_up: Vec<QMatMul>,
    experts_down: Vec<QMatMul>,
    num_experts_per_tok: usize,     // 每 token 激活 expert 数
    norm_topk_prob: bool,           // 是否归一化 top-k
}
```

**与我们现有实现的对比：**

| | 我们 (qwen3 Dense) | candle Qwen3 MoE |
|---|---|---|
| Attention | ✅ `Attention_Weights` | ✅ 复用同一个 `Attention_Weights` |
| FFN | `Mlp_Weights` (gate/up/down) | `MoeOrMlp` (dense 或 MoE) |
| MoE Router | — | `gate_inp` (router) |
| Shared Expert | — | `shared_expert` (始终激活) |
| Sparse Experts | — | `Vec<QMatMul>` × 3 (gate/up/down) |
| CUDA 加速 | — | `FusedMoe` kernel (已启用) |
| GGUF 加载 | 按 `blk.N.ffn_*` 前缀 | 按 `blk.N.ffn_gate_inp` + `blk.N.ffn_gate_exps` 等 |

#### 实施策略

**方案：在 `GGUF_Models/` 下新建 `qwen3_moe.rs`，参照 `quantized_qwen3_moe.rs` 的模式。**

- **Attention 不动** — 直接服用现有的 `Attention_Weights`（Qwen3 MoE 的 attention 和 Dense 完全相同）
- **FFN 改为枚举** — `MoeOrMlp { Mlp(Mlp_Weights), MoE(New_MoE_Weights) }`
- **GGUF tensor 命名** — 专家权重命名格式：
  ```
  blk.N.ffn_gate_inp.weight         ← Router
  blk.N.ffn_gate.weight             ← shared expert gate
  blk.N.ffn_up.weight               ← shared expert up
  blk.N.ffn_down.weight             ← shared expert down
  blk.N.ffn_gate_exps.weight        ← 所有专家 gate (合并)
  blk.N.ffn_up_exps.weight          ← 所有专家 up
  blk.N.ffn_down_exps.weight        ← 所有专家 down
  ```
- **架构分发** — `GGUF_Load_Model` 中新增 `"qwen3_moe"` 分支，metadata key 前缀使用 `qwen3` (Qwen3 MoE 和 Dense 共享同一套架构参数)

#### 涉及文件（预估）

| # | 文件 | 操作 |
|---|------|------|
| 1 | `Src/ML_Engine/GGUF_Models/qwen3_moe.rs` | **新建** — Qwen MoE 权重 + Forward |
| 2 | `Src/ML_Engine/GGUF_Models/mod.rs` | 添加 `pub mod qwen3_moe` |
| 3 | `Src/ML_Engine/gguf_model.rs` | `AnyModel` 新增 `Qwen3Moe` 变体 |
| 4 | `Src/ML_Engine/mod.rs` | re-export |

---

- 基分支: `reforge`
- 工作分支: `deepseek` (在 `reforge` 基础上创建)
- DeepSeek V3.2 为 671B 参数 MoE 模型（`deepseek_v3` 架构），**不存在 Lite 版本**（此前文档中的 "V3.2-Lite" 为 AI 幻觉）
- 测试方案待定，优先完成代码实现和编译验证
- 第一阶段只实现 prefill + 单 token decode，暂不实现 MTP speculative decoding
- 暂不实现 DeepSeek Sparse Attention（超长上下文优化），先用标准 causal mask
- MLA KV Cache 采用 "吸收" 策略：prefill 时计算完整 K/V 并缓存压缩 latent，decode 时只算增量
- MoE 的 expert 负载均衡 loss 在推理阶段不需要（仅训练用）
- candidate 参考文件: `DevJadhav/deepseek-from-scratch/rust-src/src/model/mla.rs` + `moe.rs`
