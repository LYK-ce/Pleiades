# Pleiades 推理加速方案

> Date: 2026-05-31
>
> 参考 mistral.rs 和 Crane 的 candle 加速策略，按优先级排列可实施的优化方案。

---

## 一、立即可做 (低风险、高收益)

### 1.1 去掉 `synchronize()` 同步等待

**位置**: `Src/ML_Engine/context.rs` — `MlSession::forward()`

当前代码在 `GGUF_Model_Inference` 之后调用 `self.ctx.device.synchronize()`，强制 CPU 等待 GPU 完成。这是调试用同步点，生产环境删掉后 GPU 可以异步流水线：

```rust
// 删除这行:
self.ctx.device.synchronize()?;
```

**预期收益**: token/s 提升 30-50%（GPU 不再空转等 CPU）。

### 1.2 Release 编译 + LTO

当前 `cargo check` 是 debug 模式。生产构建应使用：

```bash
cargo build --release --features cuda
```

`Cargo.toml` 追加：

```toml
[profile.release]
lto = true
codegen-units = 1
opt-level = 3
```

**预期收益**: 3-5x 整体加速。

### 1.3 Pre-allocated KV Cache

**位置**: `Src/ML_Engine/GGUF_Models/qwen3.rs` — `Attention_Weights::New`

**当前**: `ConcatKvCache::new(2)` — 逐 token 动态扩容，每步可能 realloc。

**改为**: 在 `Load_Model` 时读取 `context_length`，预分配完整容量：

```rust
// 修改 ConcatKvCache 创建逻辑:
let kv_cache = ConcatKvCache::new_with_capacity(2, context_length);
```

需要给 `ConcatKvCache` 或自定义 cache 增加容量参数。

**参考**: Crane 的 "pre-allocated KV cache" 策略。

**预期收益**: 减少 per-token malloc 开销，decode 阶段收益最大。

---

## 二、中期目标 (需一定改动)

### 2.1 FlashAttention 接入

candle 已有 `candle-flash-attn` crate (v0.10.2)。在 prefill 阶段替换标准 causal mask attention：

```toml
candle-flash-attn = { version = "0.10.2", optional = true }
```

**作用**: 分块计算 attention，prefill 阶段的 attention 矩阵从 O(n²) 显存降到 O(n)，长 prompt 场景收益巨大。

**当前潜在风险 #20 (CUDA OOM) 可在很大程度上缓解。**

**参考**: mistral.rs 默认启用 FlashAttention V2。

### 2.2 GQA 4D matmul 优化

**位置**: `qwen3.rs` — `Attention_Weights::Forward`

**当前**:
```rust
let k = repeat_kv(k, self.num_kv_groups)?.contiguous()?;
let v = repeat_kv(v, self.num_kv_groups)?.contiguous()?;
let scores = (q.matmul(&k.transpose(2, 3)?)? * scale)?;
```

**问题**: `repeat_kv` 显式复制 K/V tensor，增加显存和带宽开销。

**改为**: 4D batch matmul，在 matmul 中隐式广播 KV heads，避免物理复制：

```rust
// Q: [B, n_heads, L, D]
// K: [B, n_kv_heads, L, D]
// 使用 broadcast matmul 替代 repeat_kv + matmul
let k = k.unsqueeze(2)?;  // [B, n_kv_heads, 1, L, D]
let scores = q.matmul_broadcast(&k.transpose(3, 4)?)?;
```

**参考**: Crane 的 "GQA 4D matmul"。

### 2.3 Fused RoPE + Cache Append

**当前**: RoPE 旋转 → `kv_cache.append` — 两步操作，两个 kernel launch。

**改为**: 合并为单次 fused kernel，在旋转的同时写入 cache。

**参考**: Crane 的 "fused RoPE with cache pre-growth"。

### 2.4 `cuda-sys` FFI 直接操作

当前通过 candle 的 `Device::new_cuda(0)` 抽象层访问 GPU。可通过 `cuda-sys` 或 candle-kernels 的 custom CUDA kernel 直接做 FFI 调用，绕过 candle 的通用 kernel，针对 Qwen3 的特殊 tensor shape 做定制优化。

**参考**: mistral.rs 的 `mistralrs-paged-attn` crate — 独立的 PagedAttention CUDA kernel。

---

## 三、远期规划

### 3.1 PagedAttention + Continuous Batching

KV cache 分页管理（类似操作系统虚拟内存），多个请求复用物理显存页：

- 不再为每个 slot 分配固定大小 cache
- 共享前缀（system prompt）只存一份
- 动态调度请求

**需要**: 重构 Session Manager 的 slot 模型为 page-based 模型。

**参考**: mistral.rs 的 `PagedAttention` 实现。

### 3.2 ISQ (In-Situ Quantization)

在加载模型时对 F32 权重实时量化，无需预先生成 GGUF 量化文件：

```rust
let model = GGUF_Load_Model(path, device)?;
model.quantize(QuantMethod::Q4_K_M)?;  // 实时量化
```

**参考**: mistral.rs 的 ISQ 机制。

### 3.3 Speculative Decoding

小模型（或 MTP head）草稿 N 个 token → 大模型批量验证 → 全部接受或逐个拒绝。

**适用场景**: 当 DeepSeek V3.2 支持后，利用其 MTP (Multi-Token Prediction) head 做草稿。

---

## 四、对照表

| 优化 | 来源 | 难度 | 收益 | 阶段 |
|------|------|------|------|------|
| 去掉 synchronize | 自主发现 | 低 | ~40% | 一 |
| Release + LTO | 通用 | 低 | 3-5x | 一 |
| Pre-allocated KV Cache | Crane | 中 | decode 加速 | 一 |
| FlashAttention | mistral.rs | 中 | prefill 加速 | 二 |
| GQA 4D matmul | Crane | 中 | 减少显存 | 二 |
| Fused RoPE | Crane | 中 | 减少 kernel launch | 二 |
| CUDA FFI kernel | mistral.rs | 高 | 定制加速 | 二 |
| PagedAttention | mistral.rs | 高 | 并发吞吐 | 三 |
| ISQ 即时量化 | mistral.rs | 高 | 降低加载门槛 | 三 |
| Speculative Decoding | mistral.rs | 高 | 2x 生成速度 | 三 |
