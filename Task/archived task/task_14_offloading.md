# Task 14: Offloading (Layer Swapping)

> 状态：✅ 已完成并归档 (2026-06-15)。Offloading 核心 + DeepSeek V3.2 适配已完成。
> 分支：task14swaplayer

## 背景

GPU 显存有限，当模型过大或多 session 并发时，需要将暂时不用的模型上下文从 GPU offload 到 CPU 内存或磁盘，腾出显存给活跃 session 使用。

## 目标

以 `MlContext`（即整个 `GGUF_Model`）为单位，提供：
1. **GPU 卸下 / 恢复**：`ml.offload_to_cpu(sess)` / `ml.offload_to_cuda(sess)`
2. **磁盘持久化**：`ml.offload_save(sess, file_id)` / `sess = ml.offload_load(file_id, device)`

### 进阶目标：分层 offload 流水线

pipe_7/pipe_8：2 节点 × 2 GPU，每 GPU 内 chunk offload，适配超大模型（如 DeepSeek V3.2 671B）

---

## 已完成

### Offloading 核心实现 ✅

- [x] `MlContext.offloaded_kv` 挂起态
- [x] `offload_to_cpu` / `offload_to_cuda` / `offload_save` / `offload_load`
- [x] KVCX 序列化格式（`.kvcache/` 目录）
- [x] Lua 绑定（`ml.offload_load`, `sess:offload_to_cpu/to_cuda/save`）
- [x] `AnyModel::extract_kv_cache/restore_kv_cache`
- [x] reload 层范围修正（load_model 时保存 start/end，不推导）
- [x] offload_to_cuda 后重新保存 model_path

### pipe_7/pipe_8 分层 offload 流水线 ✅

- [x] 2 节点 × 2 GPU，每 GPU `chunks_per_gpu` 段
- [x] offset==0 用 load_model，offset>0 用 offload_to_cuda
- [x] 跨 GPU 搬运（GPU:0 → CPU → GPU:1）
- [x] 双向网络张量流（pipe_7 ↔ pipe_8）
- [x] `os.clock()` 计时（load/forward/offload 分项）
- [x] os 库解禁

### DeepSeek V3.2 适配 ✅

- [x] Unsloth GGUF 命名适配（`attn_q_a`、`attn_kv_a_mqa`、`attn_k_b` 等下划线分隔）
- [x] `k_b`/`v_b` 3D tensor flatten（`[128,512,128]`/`[128,128,512]` → `[16384,512]`）
- [x] MLA KV Cache 改为缓存展开的 K/V（对齐官方 candle + llama.cpp）
- [x] `DeepSeek_Config` 从正确 Unsloth metadata key 读取维度
- [x] MoE 改用 `FusedMoeGGUF`（CUDA kernel），适配 Unsloth `ffn_*_shexp`/`ffn_*_exps` 命名
- [x] Dense/MoE 层自动检测（`ffn_gate_inp.weight` 存在 → MoE，否则 → Dense）
- [x] `analyze.lua` 支持完整 metadata_raw 输出
- [x] `narrow` + `transpose` 后加 `contiguous()`

### 验证结果

- ✅ DeepSeek V3.2 671B 在 4×A100 80G 上成功加载
- ✅ 完整 prefill + decode 流程跑通（无维度错误）
- ✅ 每 token ~280 秒（load 占 99.9%，forward <0.2s）
- ⚠️ 输出可能不正确（生成 2 token 后 EOS，需 golden 验证）

---

## 待解决

- [ ] MLA 输出正确性验证（对比 llama.cpp golden）
- [ ] RoPE YaRN scaling 支持
- [ ] PGGUF reload 性能优化（load 占 99.9% 时间）
- [ ] pipe_7/8 多余 debug 日志清理

---

## 涉及文件

| # | 文件 | 变更 |
|---|------|------|
| 1 | `Src/main.rs` | `.kvcache/` 目录创建 |
| 2 | `Src/ML_Engine/gguf_model.rs` | AnyModel extract/restore KV Cache；DeepSeek 加载路径 |
| 3 | `Src/ML_Engine/context.rs` | offloaded_kv + 4 offload 方法 + KVCX 序列化 + Lua 注册 |
| 4 | `Src/VM/capability_binding.rs` | ml.offload_load 绑定；analyze metadata_raw |
| 5 | `Src/ML_Engine/GGUF_Models/deepseek_v3.rs` | **大量修改**：MLA_KV_Cache、MLA_Weights、DeepSeekMoE_Weights、DeepSeekFFN、config 读取、tensor 命名 |
| 6 | `Src/ML_Engine/GGUF_Models/qwen3.rs` | Mlp_Weights::New_Dense |
| 7 | `Src/VM/engine.rs` | os 库解禁 |
| 8 | `programs/user/pipe_7.lua` | 分层 offload 流水线前半段 |
| 9 | `programs/user/pipe_8.lua` | 分层 offload 流水线后半段 |
| 10 | `programs/user/analyze.lua` | GGUF metadata 完整输出 |
| 11 | `programs/user/offload_pingpong.lua` | 单 GPU 分层 offload 演示 |
