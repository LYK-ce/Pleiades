# Task 14: Offloading (Layer Swapping)

> 状态：设计中
> 创建日期：2026-06-01
> 分支：task14swaplayer

## 背景

GPU 显存有限，当模型过大或多 session 并发时，需要将暂时不用的模型上下文从 GPU offload 到 CPU 内存或磁盘，腾出显存给活跃 session 使用。

## 目标

以 `MlContext`（即整个 `GGUF_Model`）为单位，提供：
1. **GPU 卸下 / 恢复**：`ml.offload_to_cpu(sess)` / `ml.offload_to_cuda(sess)`（CPU 不参与运算，仅暂存 KV Cache 状态）
2. **磁盘持久化**：`ml.offload_save(sess, file_id)` / `sess = ml.offload_load(file_id, device)`

### 核心原则：权重 reload + KV Cache 序列化

- **权重不序列化**：始终从 PGGUF 文件通过 `GGUF_Load_Model` reload，PGGUF 文件本身就是权重的持久化形态
- **KV Cache 序列化**：这是唯一需要保存的运行时状态（推理上下文），每层 2 个 Tensor
- **其他状态**：rng_state、offset、eos_token_id、chat_template 直接存储

## 方案

### KV Cache 统一提取/恢复接口

在 `AnyModel` 上定义统一接口，三种架构对外一致：

| 架构 | KV Cache 类型 | Tensor 1 | Tensor 2 |
|------|--------------|----------|----------|
| Qwen3 | `ConcatKvCache` | `k` (完整 K) | `v` (完整 V) |
| Qwen3 MoE | `ConcatKvCache` | `k` | `v` |
| DeepSeek V3 | `MLA_KV_Cache` | `kv_latent` (压缩) | `k_pe` (RoPE) |

```rust
impl AnyModel {
    /// 提取所有层的 KV Cache，返回 Vec<(tensor_k, tensor_v)>
    fn extract_kv_cache(&self) -> Vec<(Tensor, Tensor)>;
    /// 恢复所有层的 KV Cache（每层 append 一对 k/v）
    fn restore_kv_cache(&mut self, kvs: Vec<(Tensor, Tensor)>) -> Result<()>;
}
```

- Qwen3/Qwen3 MoE：通过 `ConcatKvCache::k()` / `v()` 获取引用后 clone
- DeepSeek V3：通过 `MLA_KV_Cache` 的 `kv_latent` / `k_pe` 字段 clone

### API 设计

```lua
-- 设备间迁移
ml.offload_to_cpu(sess)        -- GPU → CPU
ml.offload_to_cuda(sess)       -- CPU → GPU

-- 磁盘持久化
ml.offload_save(sess, file_id)           -- 保存到 .kvcache/{file_id}
sess = ml.offload_load(file_id, device)  -- 从 .kvcache/{file_id} 恢复
```

### 磁盘存储

- 目录：`/workspace/.kvcache/`（启动时创建，不走 Storage 模块）
- 文件名：用户指定的 `file_id`
- 格式：自定义二进制，header(metadata) + 逐层 KV tensor(raw bytes)

```
┌──────────────────────────────────────┐
│ header:                              │
│   magic: [u8; 4] = b"KVCX"          │
│   version: u32                       │
│   model_path_len: u32                │
│   model_path: [u8]                   │
│   start_layer: u32                   │
│   end_layer: u32                     │
│   arch_len: u32                      │
│   arch: [u8]                         │
│   rng_state: u64                     │
│   offset: u64                        │
│   eos_token_id: u32                  │
│   dtype: u8                          │
│   chat_template_len: u32             │
│   chat_template: [u8]                │
│   num_layers: u32                    │
├──────────────────────────────────────┤
│ for each layer:                      │
│   k_ndim: u32                        │
│   k_shape[ndim]: [u64]               │
│   k_dtype: u8                        │
│   k_data_len: u64                    │
│   k_data: [u8]                       │
│   v_ndim: u32                        │
│   v_shape[ndim]: [u64]               │
│   v_dtype: u8                        │
│   v_data_len: u64                    │
│   v_data: [u8]                       │
└──────────────────────────────────────┘
```

### 实现流程

**MlContext 新增挂起态**：
```rust
struct MlContext {
    model: Option<GGUF_Model>,         // None = 已 offload
    tokenizer: Option<Tokenizer>,
    offset: usize,
    rng_state: u64,
    eos_token_id: u32,
    chat_template: Option<String>,
    device: Device,

    // 新增：KV Cache 暂存（已在 CPU RAM，等 to_cuda 恢复）
    offloaded_kv: Option<Vec<(Tensor, Tensor)>>,
}
```

**offload_to_cpu**：
1. `extract_kv_cache()` 提取所有层 KV → KV tensor 自然在 CPU RAM
2. `GGUF_Unload_Model(old_model)` 释放 GPU 显存
3. 将 KV 存入 `offloaded_kv`，model 置为 None
4. 保持 tokenizer、rng_state、offset 等状态不变

> 全程零 I/O、零权重重建。CPU 不参与运算，仅暂存状态。

**offload_to_cuda**：
1. 调用 `GGUF_Load_Model(start, end, path, device)` 在 GPU 上重建权重
2. 从 `offloaded_kv` 取出 KV，`restore_kv_cache()` 恢复到新模型
3. `offloaded_kv` 置为 None
4. 恢复 rng_state、offset、eos_token_id、chat_template

**offload_save**：
1. 若 model 已加载：`extract_kv_cache()` 提取 KV（若已 offload 直接用 `offloaded_kv`）
2. 对每层 k/v Tensor 调用 `to_vec1::<f32>()` 获取 raw bytes
3. 写入 header + 逐层数据到 `.kvcache/{file_id}`
4. ⚠️ **释放模型**：若 model 仍在，`unload()` 释放设备显存；KV 保留在 CPU RAM（`offloaded_kv`），方便后续 `to_cuda` 恢复

**offload_load**：
1. 读取 `.kvcache/{file_id}`，解析 header
2. 调用 `GGUF_Load_Model` 在指定 device 上加载权重
3. `Tensor::from_vec()` 从 raw bytes 重建 KV tensor
4. `restore_kv_cache()` 恢复到新模型
5. 恢复 runtime 状态，返回新 `MlSession`

### 依赖

- candle-core: `Tensor::to_vec1()`, `Tensor::from_vec()`, `Tensor::to_device()`
- candle-nn: `ConcatKvCache::k()`, `ConcatKvCache::v()`, `ConcatKvCache::append()`
- 现有：`GGUF_Load_Model`, `GGUF_Unload_Model`, `MlSession`, `MlContext`

## 涉及文件与变更摘要

| # | 文件 | 变更 |
|---|------|------|
| 1 | `Src/main.rs` | 启动时创建 `.kvcache/` 目录 |
| 2 | `Src/ML_Engine/gguf_model.rs` | `AnyModel` 新增 `extract_kv_cache()` / `restore_kv_cache()`，各架构分别实现 |
| 3 | `Src/ML_Engine/context.rs` | `MlContext` 新增 `offloaded_kv` 字段；`MlSession` 新增 `offload_to_cpu/to_cuda/save/load` 方法 + 序列化逻辑 + Lua UserData 注册 |
| 4 | `Src/VM/capability_binding.rs` | `ml` 表新增 `ml.offload_load(file_id, device)` 函数绑定 |

**不需要新建文件**：序列化逻辑放在 `context.rs` 内，与 MlSession 紧密耦合。

### 变更细节

#### 1. `Src/main.rs` — 目录创建
```rust
// Phase 1 末尾，workspace_dir 创建之后
std::fs::create_dir_all(".kvcache")?;
```

#### 2. `Src/ML_Engine/gguf_model.rs` — AnyModel KV 接口
```rust
impl AnyModel {
    pub fn extract_kv_cache(&self) -> Vec<(Tensor, Tensor)> {
        match self {
            AnyModel::Qwen3(m) => m.layers.iter()
                .map(|l| (l.self_attn.kv_cache.k().unwrap().clone(),
                           l.self_attn.kv_cache.v().unwrap().clone()))
                .collect(),
            AnyModel::Qwen3Moe(m) => m.layers.iter()
                .map(|l| (l.self_attn.kv_cache.k().unwrap().clone(),
                           l.self_attn.kv_cache.v().unwrap().clone()))
                .collect(),
            AnyModel::DeepSeek(m) => m.layers.iter()
                .map(|l| (l.mla.kv_cache.kv_latent.clone().unwrap(),
                           l.mla.kv_cache.k_pe.clone().unwrap()))
                .collect(),
        }
    }
    // restore_kv_cache: 对每层 append(k, v)
}
```

#### 3. `Src/ML_Engine/context.rs` — 核心逻辑
- `MlContext.offloaded_kv: Option<Vec<(Tensor, Tensor)>>`（均在 CPU）
- `MlSession::offload_to_cpu()` — extract → to_device(Cpu) → unload → 存入 offloaded_kv
- `MlSession::offload_to_cuda()` — load_model(GPU) → KV to_device(GPU) → restore → clear
- `MlSession::offload_save(file_id)` — 取 KV（model 或 offloaded_kv） → to_vec1 → 写 .kvcache/
- `MlSession::offload_load(file_id, device)` — 读 .kvcache/ → load_model → from_vec → restore
- UserData 注册：`offload_to_cpu` / `offload_to_cuda` / `offload_save`（`offload_load` 在 ml 表）

#### 4. `Src/VM/capability_binding.rs` — Lua 绑定
```rust
ml.set("offload_load", lua.create_function(|_, (file_id, device): (String, String)| {
    MlSession::offload_load(&file_id, &device)
        .map_err(|e| mlua::Error::runtime(e))
})?)?;
```
