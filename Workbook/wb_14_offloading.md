# Workbook 14: Offloading

- 开始时间：2026-06-01
- 最后更新：2026-06-01 05:25 UTC
- 分支：task14swaplayer
- 状态：DeepSeek V3.2 首次推理跑通，待 golden 验证

## 实施记录

### Offloading 核心
- MlContext 新增 `offloaded_kv: Option<Vec<(Tensor, Tensor)>>`
- 4 个 API: offload_to_cpu/to_cuda/save/load
- KVCX 序列化格式：magic+KVCX+header+per-layer KV
- Lua 绑定：sess methods + ml.offload_load
- Bug: derive_layer_range 对非 split PGGUF 返回全模型范围 → 修复为 load_model 时保存实际 start/end
- Bug: offload_to_cuda 用 take() 消费 model_path → 修复为 clone + re-save

### pipe_7/pipe_8
- 参照 pipe_5/pipe_6 拓扑，每 GPU 内 chunks_per_gpu 段 offload
- offset==0 → load_model, offset>0 → offload_to_cuda
- os.clock 计时
- Lua os 库解禁

### DeepSeek V3.2 适配
- 架构名: deepseek2 (Unsloth GGUF)
- 维度: q_head=192, qk_nope=128, qk_rope=64, v_head=128
- Unsloth metadata key: key_length_mla, value_length_mla, rope.dimension_count
- tensor 命名: 全部下划线分隔 (attn_q_a, attn_kv_a_mqa, attn_k_b 等)
- k_b: 3D [128,512,128] → dequantize+reshape [16384,512] → Linear
- v_b: 3D [128,128,512] → dequantize+reshape [16384,512] → Linear
- MLA KV Cache: 改为缓存展开 K/V [b,heads,seq,dim]（对齐 llama.cpp）
- MoE: FusedMoeGGUF, shared expert → ffn_*_shexp, routed → ffn_*_exps
- Dense layer: 自动检测 ffn_gate_inp 存在性，降级为 Mlp_Weights
- contiguous: narrow + transpose 后加 .contiguous()

### 验证
- DeepSeek V3.2 671B 在 4×A100 80G 成功加载并推理
- 每 token ~280s (load 99.9%, forward <0.2s)
- 生成 2 token 后 EOS，可能输出不正确

## 待处理
- [ ] MLA 输出 golden 验证
- [ ] RoPE YaRN scaling
- [ ] PGGUF reload I/O 优化
- [ ] 清理 debug 日志
