Presented by KeJi
Date: 2026-05-31

# 为什么 Qwen3 235B 能跑，DeepSeek V4 Flash 不能跑

## 一句话总结

Qwen3 235B MoE 有社区维护的量化版 GGUF 文件 + Pleiades 已有完整 Rust 加载器；DeepSeek V4 Flash 两者都没有。

---

## 详细对比

### 1. 模型格式

| | Qwen3 235B MoE | DeepSeek V4 Flash |
|---|---|---|
| HuggingFace 发布格式 | BF16 safetensors | **FP8 + FP4** mixed safetensors |
| 社区 GGUF 文件 | ✅ Q4_K_M / Q8_0 等多种量化 | ❌ 不存在 |
| GGUF 格式是否支持该精度 | ✅ Q4_K_M 是标准量化类型 | ❌ GGUF 规范没有 FP8/FP4 类型码 |
| 转换后 BF16 大小 | Q4_K_M ≈ 60GB | BF16 ≈ **580GB** (FP4→BF16 膨胀 4×) |

**结论**: 必须在 Python 侧做 FP8/FP4 → BF16 dequant 才能写入 PGGUF，文件从 160GB 膨胀到 ~320GB。

### 2. 推理计算

| | Q4_K_M (Qwen3) | FP8/FP4 (DeepSeek V4) |
|---|---|---|
| candle 是否支持直接计算 | ✅ `QMatMul` 有 Q4_K_M kernel | ❌ 没有 FP8/FP4 matmul kernel |
| 是否需要运行时 dequant | 不需要（量化 kernel 直接算） | **必须** dequant 到 BF16 再算 |
| VRAM 占用 | Q4_K_M ≈ 压缩 4× | BF16 = 无压缩 |

**结论**: Q4_K_M 的 kernel 来自 llamacpp 社区 3 年打磨；FP8/FP4 的 GPU kernel 几乎不存在于开源推理框架中（H100 专属硬件特性）。

### 3. Rust 加载器

| | Qwen3 235B MoE | DeepSeek V4 Flash |
|---|---|---|
| Pleiades 加载器 | ✅ `Src/ML_Engine/GGUF_Models/qwen3_moe.rs` | ❌ 不存在 |
| 架构支持 | ✅ 标准 Transformer + MoE | ❌ MLA (Multi-head Latent Attention) + Compressor + Indexer + MTP |

**结论**: DeepSeek V4 的 MLA 结构与标准 Transformer 完全不同：
- Qwen3: Q/K/V 投影 → attention → FFN
- DeepSeek V4: `wq_a/wq_b` 压缩投影 → `wkv` 联合 KV → `wo_a/wo_b` 输出 → `compressor` → `indexer` → MoE

这不是"参数多一点"的差异，是架构层的差异。

### 4. 操作复杂度

| 步骤 | Qwen3 235B | DeepSeek V4 |
|---|---|---|
| 下载模型 | `huggingface-cli download xxx.gguf` | 46 个分片，370GB |
| 格式处理 | 无需处理（已经是 GGUF） | Python 转换: unpack FP4 + dequant FP8 + 69K tensor 映射 |
| 加载器 | 直接 `GGUF_Load_Model` | 需新写 800+ 行 Rust 代码 |
| 内存策略 | 4×A100 Q4_K_M ≈ 60GB/卡 | BF16 需 320GB, 不转换则 matmul 不支持 |

---

## 本质原因

```
DeepSeek V4 Flash 是 2025 年发布的最新模型，使用 FP8(2023) + FP4(2025) 精度。
这两个精度依赖 NVIDIA H100/H200/B200 的硬件 tensor core，开源生态尚未跟进。
```

Qwen3 用的是 2023 年的 Q4_K_M 量化——这个已经在 llama.cpp 生态里打磨了 3 年，candle 完全集成。

**等社区有人把 DeepSeek V4 转成 GGUF Q4_K_M 的那一天，它就也能跑了。**
