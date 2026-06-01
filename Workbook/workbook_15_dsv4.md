# Workbook 15: DeepSeek V4 Flash 集成

- 开始时间: 2026-06-01
- 分支: hf2gguf
- 状态: 代码完成，cargo check --no-default-features 通过

## 依赖
- MScanter/deepseek-v4-candle: cloned to .zgent/deepseek-v4-candle-reference
- 现有: deepseek_v3.rs (KV cache 参考), gguf_model.rs, gguf_model_manager.rs

## 实施记录

### 15.1 Python (完成)
- 新增 Tool/hf2gguf/wrap_native.py (278 行)
- 修改 Tool/hf2gguf/converter.py: +16 行 (convert_hf_to_pgguf_wrap_native)
- 修改 Tool/hf2gguf/__init__.py: 导出新函数
- 修改 Tool/convert_hf_to_gguf.py: +12 行 (--wrap-native flag)

### 15.2 Rust: PGGUF 读取分支 (完成)
- gguf_model_manager.rs: Model_Arch_Info +weight_format +num_shards; GGUF_Analyze_And_Convert 跳过 safetensors
- gguf_model.rs: AnyModel::DeepSeekV4 变体; GGUF_Load_Model deepseek_v4 分支

### 15.3+15.4 Rust: deepseek_v4 模块 (完成, 9 files)
- mod.rs: DeepSeekV4Model + from_pgguf + forward + KV cache API
- config.rs: Config + from_gguf_metadata
- quant.rs: FP4/FP8/UE8M0 dequant
- loader.rs: SafeTensors (from bytes) + MultiSafeTensors (multi-shard)
- rope.rs: YaRN RoPE
- attention.rs: MLA + sink-softmax + Head (with KV cache)
- moe.rs: MoE + Gate::route_hashed (hash routing)
- mhc.rs: Hyper-Connections (Sinkhorn)
- sparse.rs: Compressor + Indexer
- block.rs: Block (mHC-wrapped attn+MoE, KV cache)
- model.rs: Transformer::from_config

### 编译
- cargo check --no-default-features: PASS (0 errors, 0 new warnings)

