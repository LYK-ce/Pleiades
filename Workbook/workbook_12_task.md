# Task 12 工作记录
> 任务: DeepSeek V3.2 模型架构支持
> 分支: deepseek (基于 reforge, origin/reforge 6281582)
> 开始: 2026-05-30

## 会话初始化 (2026-05-30)
- SSH: 权限 OK (600/644), 认证 OK (LYK-ce)
- Remote: git@github.com:LYK-ce/Pleiades.git
- GIT_SSH_COMMAND: 持久化到 ~/.bashrc
- 基分支: reforge → origin/reforge (6281582)
- 工作分支: deepseek (git checkout -b deepseek)
- 任务文件: Task/task_12.md

## 2026-05-30 文档修正
- 删除 12.4 测试子任务（测试方案待定）
- 修正 "V3.2-Lite" 幻觉：V3.2 为 671B MoE，不存在 Lite 版本

## 依赖
- 上游: reforge 分支基础设施 (VM, Orchestrator, ML_Engine/GGUF_Models/qwen3.rs)
- 参考: DevJadhav/deepseek-from-scratch, EricLBuehler/mistral.rs
- 测试: V3.2 671B MoE，不存在 Lite 版本，测试方案待定

## 子任务
| # | 任务 | 状态 |
|---|------|------|
| 12.0 | 创建 deepseek 分支 | ✅ |
| 12.1 | 调研 MLA + DeepSeekMoE | ✅ |
| 12.2 | 新建 deepseek_v3.rs | ✅ |
| 12.3 | 集成到 GGUF_Load_Model + MlSession | ✅ |
| 12.5 | 支持 cuda:0/cuda:1 多GPU | ✅ |
| 12.6 | pipe_5/pipe_6 2节点×2GPU 脚本 | ✅ |
| 12.7 | 集成 Qwen MoE 支持 | ✅ |
| 12.8 | 修复：架构名/dtype/KV cache/I16 | ✅ |

## 12.8 修复记录 (2026-05-30)

- qwen3moe 架构名识别 → gguf_model.rs
- dtype 从 metadata 读取（BF16/F16/F32）→ gguf_model.rs
- pipe_2/4/6 reset_kv_cache → programs/user/
- I8/I16/U8/U16→I32/U32 转换 → gguf_model_manager.rs
- pipe_5/6 GPU→CPU→GPU 两步搬运 → programs/user/

## 12.7 调研 (2026-05-30)

### 发现
candle-transformers 0.10.2 已有完整的 Qwen MoE 实现：
- qwen2_moe.rs / qwen3_moe.rs / quantized_qwen3_moe.rs
- FusedMoe CUDA kernel 直接可用 (default=["cuda"])

### 方案
Attention 不变（复用现有 Attention_Weights），FFN 改为 MoeOrMlp 枚举。
新增 qwen3_moe.rs，参照 quantized_qwen3_moe.rs 的 GGUF 加载模式。
AnyModel 新增 Qwen3Moe 变体。

## 12.6 调研 (2026-05-30)

### 环境
4×A100 80GB。Node1 用 GPU 0/1 (cuda:0, cuda:1)，Node2 用 GPU 2/3。
通过 CUDA_VISIBLE_DEVICES 隔离。

### 方案
参照 pipe_1/pipe_2 模式，关键差异：
- 每个节点加载 2 个 MlSession：cuda:0 和 cuda:1
- 分片内层再对半分给两张 GPU
- GPU:0 forward → to_device("cuda:1") → GPU:1 forward
- 其余网络通信逻辑与 pipe_1/2 相同

## 12.5 调研 (2026-05-30)

### 现状
所有 CUDA 设备硬编码 `Device::new_cuda(0)`，且 `parse_device_str()` 位置不对（在 lua_tensor.rs 而非 ML Engine）。

### 方案
新建 `Src/ML_Engine/device.rs` 作为 ML Engine 唯一设备解析入口：
- "cpu" → Device::Cpu
- "cuda" / "cuda:0" → Device::new_cuda(0) (向后兼容)
- "cuda:N" → Device::new_cuda(N)

涉及 7 个文件：
- device.rs [新建] — 唯一入口
- lua_tensor.rs — 删除 parse_device_str, use super::device
- context.rs — 删除重复 match, 复用 device::parse_device_str
- mod.rs — pub mod device + re-export
- TUI/mod.rs — 扩展验证
- Orchestrator/core/branch_user.rs — 帮助文本
- Orchestrator/mod.rs — 内存查询

## 12.2-12.3 实施记录 (2026-05-30)

### 新增文件
- `Src/ML_Engine/GGUF_Models/deepseek_v3.rs` (~940 行)
  - MLA_KV_Cache: 压缩 latent KV cache (Debug + Clone)
  - MLA_Weights: Q 低秩 + KV 联合压缩 + 解耦 RoPE forward
  - DeepSeekMoE_Weights: 共享 expert + N 个 routed experts + top-k softmax 路由
  - DeepSeek_Layer: MLA → residual → MoE → residual
  - DeepSeek_Model: 完整模型 (embed + layers + norm + lm_head + Causal_Mask)
  - DeepSeek_Config: GGUF metadata 提取 (14 个参数)

### 修改文件
- `Src/ML_Engine/GGUF_Models/mod.rs`: +pub mod deepseek_v3
- `Src/ML_Engine/gguf_model.rs`:
  - +AnyModel 枚举 (Qwen3 | DeepSeek) with Forward + Clear_Kv_Cache dispatch
  - GGUF_Model.model: Model_Weights → AnyModel
  - GGUF_Load_Model: 架构分发 → deepseek_v3/deepseek2 分支
  - DeepSeek 路径: DeepSeek_Config 读取 metadata, DeepSeek_Layer::From_Extracted 加载
  - RoPE head_dim: DeepSeek 用 qk_rope_dim 而非完整 head_dim
- `Src/ML_Engine/gguf_model_manager.rs`: +pub Get_Metadata_Usize_From_Map
- `Src/ML_Engine/mod.rs`: +AnyModel 导出

### 编译结果
cargo check --no-default-features: 0 errors, 19 pre-existing warnings ✅

## 12.1 调研结果 (2026-05-30)

### 参考源分析

| 参考源 | 语言 | 适用度 | 说明 |
|--------|------|--------|------|
| mistral.rs deepseek2/deepseek3 | Rust/candle | ⭐⭐⭐ | GGUF 加载 + MLA/MoE 最接近 Pleiades 模式 |
| DevJadhav deepseek-from-scratch | Rust/candle | ⭐⭐ | VarBuilder(训练向), MoE 含负载均衡, 不适配 GGUF |

### MLA 权重映射 (GGUF tensor → 代码)

```
Q路径 (3层Low-Rank):
  blk.N.attn.q_a.weight     → q_a:  QMatMul  [hidden → q_lora_rank]
  blk.N.attn.q_a_norm.weight → q_norm: RmsNorm
  blk.N.attn.q_b.weight     → q_b:  QMatMul  [q_lora_rank → n_heads * q_head_dim]

KV路径 (联合压缩):
  blk.N.attn.kv_a_proj_with_mqa.weight → kv_a: QMatMul [hidden → kv_lora_rank + qk_rope_dim]
  blk.N.attn.kv_a_layernorm.weight     → kv_norm: RmsNorm
  blk.N.attn.kv_b.weight              → kv_b: QMatMul [kv_lora_rank → n_heads * (nope_dim + v_dim)]

输出:
  blk.N.attn.o.weight                  → o_proj: QMatMul [n_heads * v_dim → hidden]

MoE:
  blk.N.ffn_gate.weight                → shared expert gate
  blk.N.ffn_up.weight                  → shared expert up
  blk.N.ffn_down.weight                → shared expert down
  blk.N.ffn_gate.{e}.weight            → routed expert gate
  blk.N.ffn_up.{e}.weight              → routed expert up  
  blk.N.ffn_down.{e}.weight            → routed expert down
  blk.N.ffn_gate_s.weight              → shared expert gate (V3 notation)
  blk.N.ffn_up_s.weight                → shared expert up
  blk.N.ffn_down_s.weight              → shared expert down
```

### MLA Forward 流程 (基于 mistral.rs)

```
1. Q: x → q_a → q_norm → q_b → split(q_nope | q_pe)
2. KV: x → kv_a → split(compressed_kv | k_pe)
3. compressed_kv → kv_norm → ckv
4. RoPE: q_pe, k_pe (仅旋转 qk_rope_dim 部分)
5. Prefill: kv_b(ckv) → k_nope, v → concat → standard attention
6. Decode:  吸收优化 (mla_decode_forward): 直接 fused 计算, 省 KV cache 展开
```

### 架构元数据提取

GGUF metadata key 前缀 = `general.architecture` 值 → `"deepseek_v3"`

需读取的 key:
- deepseek_v3.block_count
- deepseek_v3.embedding_length
- deepseek_v3.attention.head_count
- deepseek_v3.attention.head_count_kv
- deepseek_v3.attention.key_length (head_dim for non-RoPE part: qk_nope_dim)
- deepseek_v3.context_length
- deepseek_v3.attention.layer_norm_rms_epsilon
- deepseek_v3.rope.freq_base
- deepseek_v3.feed_forward_length
- deepseek_v3.vocab_size
- deepseek_v3.attention.q_lora_rank
- deepseek_v3.attention.kv_lora_rank
- deepseek_v3.attention.qk_rope_head_dim
- deepseek_v3.attention.v_head_dim

### 12.2 实施策略

参照 qwen3.rs 模式 + mistral.rs MLA 逻辑:
1. MLA_Weights + MLA_KV_Cache → 替代 Attention_Weights + ConcatKvCache
2. DeepSeekMoE_Weights → 替代 Mlp_Weights (含 router + shared + routed experts)
3. DeepSeek_Layer → 替代 Layer_Weights
4. DeepSeek_Model → 替代 Model_Weights
5. DeepSeek_Config → 替代 Qwen3_Config

RoPE: 现有 Rotary_Embedding 可直接复用 (对 q_pe/k_pe 部分做旋转)
