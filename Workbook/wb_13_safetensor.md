# wb_13_safetensor — Task 13 工作记录
#
# Presented by KeJi
# Date: 2026-05-31 (updated)

## 分支
- `hf2gguf` ← `reforge` (commit 20cb306, pushed)

## 阶段 1: Qwen3 完成 ✅
- 12 个 Python 文件, Qwen3 映射表完整
- Qwen3-0.6B: safetensors → PGGUF → L1+L2+L3 测试全部通过
- 关键修复: FLOAT64→FLOAT32, merges 格式, model_type 大小写, added_tokens 补齐

## 阶段 2: DeepSeek V4 规划中
- 目标: deepseek-ai/DeepSeek-V4-Flash (284B, FP8+FP4)
- 策略: FP8/FP4 → BF16 dequant → 单文件 PGGUF (~320GB)
- 环境: 4×A100 80GB + 935GB CPU RAM
- 分配: 每 GPU ~15 层 × 5.2GB BF16 ≈ 78GB

### 待实现
1. FP8/FP4 dequant (dequant.py)
2. DeepSeek V4 tensor 映射 (deepseek_v4.py)
3. 分片按层转换
4. Pipeline Lua 脚本 (pipe_dsv4.lua)
5. 集成测试

### Next
- 在目标机器上运行转换 (需要 Python torch + safetensors)
- 先测单层 dequant 验证正确性
