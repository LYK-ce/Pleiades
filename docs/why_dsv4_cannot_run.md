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
| 转换后 BF16 大小 | Q4_K_M ≈ 135GB | BF16 ≈ **580GB** (FP4→BF16 膨胀 4×) |

**结论**: DeepSeek V4 采用 FP8/FP4 存储是先进的压缩技术——safetensors 只有 160GB。但 **candle 等开源推理后端不支持 FP8/FP4 直接运算**，必须反量化到 BF16 才能做矩阵乘法。FP4→BF16 膨胀 4 倍，导致从 160GB 暴增到 ~580GB——4 张 A100 根本放不下。

### 2. 推理计算

| | Q4_K_M (Qwen3) | FP8/FP4 (DeepSeek V4 Flash) |
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
- DeepSeek V4 Flash: `wq_a/wq_b` 压缩投影 → `wkv` 联合 KV → `wo_a/wo_b` 输出 → `compressor` → `indexer` → MoE

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
DeepSeek V4 Flash 发布时采用 FP8/FP4 存储 (2025 年最新精度)。
这确实是黑科技——160GB 能存 284B 参数。

但问题在于：这些格式依赖 NVIDIA H100/H200/B200 的专用 tensor core
做硬件加速，而开源推理框架 (candle, llama.cpp, vLLM 社区版) 的
矩阵乘法 kernel 根本不知道 FP8/FP4 是什么。

反量化到 BF16 之后:
  - Qwen3 235B:  135GB (Q4_K_M ≈ 4-bit, kernel 原生支持)
  - DeepSeek V4 Flash: 580GB (FP4×4 + FP8×2, 膨胀 3.6×)

这就好比别人送你一辆用特殊燃料的跑车——车是好车，但你手里只有 95 号汽油。\n```

---

## 有没有办法绕过？

**技术上可以，但需要大幅修改系统架构。**

核心思路是 **swap layer 策略**：

```
CPU RAM (935GB) ← 存完整的 BF16 权重 (~580GB)
GPU VRAM (80GB) ← 只加载当前计算层 + 下一层预取

推理时:
  1. GPU 加载 Layer 0 → 计算 → 输出 hidden
  2. GPU 释放 Layer 0，预加载 Layer 2
  3. GPU 加载 Layer 1 → 计算
  4. 如此往复...

每 GPU 峰值: 2 层 BF16 (~10GB) + KV cache ≈ 15GB → 完全没有压力
```

为什么目前做不到：

| 需要的改动 | 说明 |
|-----------|------|
| **新 Storage 层** | 需实现 `LayerSwapManager`，管理 CPU↔GPU 数据传输 |
| **异步预取** | 当前层计算时，后台 stream 预取下一层 |
| **流水线调度** | 协调 compute stream 和 copy stream 不阻塞 |
| **内存管理** | GPU 显存分配器需支持动态换入换出 |
| **Orchestrator 改造** | Core 主循环需感知 layer swap 事件 |

这不是"加一个功能"，而是**重构 ML Engine 的内存模型**。candle 目前的设计是将模型完整加载到设备上，不支持流式加载。需要从 `GGUF_Load_Model` 开始改造为 `GGUF_Stream_Layer` 模式。

> 类比：现在 Pleiades 的 GPU 是一间 80m² 的房间，我们的家具（模型权重）是 580m² 的。swap layer 就是把家具存在走廊（CPU RAM），需要时搬进房间用完再搬出去。走廊够大（935GB），但需要"搬家工人"（GPU copy engine）和"调度员"（异步预取）——这两者目前都没有。

**不是不能做，但这是几个月的大工程，不是短期可以完成的。**
