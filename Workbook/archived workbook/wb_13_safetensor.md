# wb_13_safetensor — Task 13 工作记录
#
# Presented by KeJi
# Date: 2026-06-01 (updated)

## 分支
- `hf2gguf` ← `reforge` (merged task14swaplayer, commit 52dd712)

## 阶段 1: Qwen3 完成 ✅
- 12 个 Python 文件, Qwen3 映射表完整
- Qwen3-0.6B: safetensors → PGGUF → L1+L2+L3 测试全部通过
- 关键修复: FLOAT64→FLOAT32, merges 格式, model_type 大小写, added_tokens 补齐, chat_template 嵌入

## 阶段 2: DeepSeek V4 Flash → 移至 Task 15
- 方案: GGUF 容器 + MScanter 架构 → `Task/task_15_dsv4_integration.md`

## 合并
- 已合并 task14swaplayer (DeepSeek V3.2 改进 + offloading + pipeline Lua)
