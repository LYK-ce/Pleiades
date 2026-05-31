# wb_13_safetensor — Task 13 工作记录
#
# Presented by KeJi
# Date: 2026-05-31

## 分支
- `hf2gguf` ← `reforge` (commit 44ace7a)

## 依赖
- venv: `/workspace/.venv`
- packages: safetensors 0.7.0, gguf 0.19.0, numpy 2.4.6, xxhash 3.7.0

## 文件清单 (12 files)
- Tool/convert_hf_to_gguf.py (CLI)
- Tool/hf2gguf/__init__.py
- Tool/hf2gguf/converter.py (core pipeline)
- Tool/hf2gguf/config_reader.py (config.json → arch detection)
- Tool/hf2gguf/tokenizer_writer.py (tokenizer.json → GGUF)
- Tool/hf2gguf/pgguf_writer.py (pleiades metadata)
- Tool/hf2gguf/utils.py (shard handling)
- Tool/hf2gguf/mappings/__init__.py
- Tool/hf2gguf/mappings/qwen3.py (FULL — tested)
- Tool/hf2gguf/mappings/qwen3_moe.py (STUB)
- Tool/hf2gguf/mappings/deepseek.py (STUB)
- docs/hf2gguf.md (TBD)

## Key decisions
- Full precision (FP16/BF16), no quantization
- GGUFWriter.add_* metadata embeds pleiades.model_id/layer_bitmap directly
- model_id = xxhash32(sorted tensor names + shapes), matches Rust logic
- Sharded safetensors supported via index.json weight_map
- Weight tying: lm_head.weight optional, Rust side handles fallback

## Blockers
- 13.9 integration test needs real safetensors model (e.g. Qwen3-1.8B from HF)
- Can test with mocked config.json + minimal safetensors

## Next
- Download small safetensors model for integration test
- Write docs/hf2gguf.md
