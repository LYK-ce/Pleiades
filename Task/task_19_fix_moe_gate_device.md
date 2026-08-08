# task_19_fix_moe_gate_device

## 概述

修复 G1 🔴:MoE router gate 权重硬编码反量化到 CPU(device mismatch)。

- **位置**: `Src/ML_Engine/GGUF_Models/qwen3/qwen3_moe.rs:117`
- **错误**: 双卡 2 节点 30B pipeline 推理时 worker 报 `Forward failed: device mismatch in matmul, lhs: Cuda{gpu_id:0}, rhs: Cpu`
- **根因**: `gate_qt.dequantize(&Device::Cpu)` 硬编码 CPU,而 `FusedMoeGGUF.forward` 中 `self.gate.forward(&xs)` 的 xs 来自 attention 输出(Cuda) × gate 权重(CPU) → mismatch
- **参考**: candle 官方 `FusedMoeGGUF::new` 用 `dequantize(vb.device())`;冻结代码 `deepseek_v3.rs:438/483` 用 `dequantize(&gg.device)`/`dequantize(device)`;embedding `gguf_model.rs:209` 用 `dequantize(device)`

## 子 agent 排查结论(2026-08-08)

- 与 G1 严格同类的问题(提前反量化到错误设备 → CUDA 必崩):**仅此一处**
- 其他提前反量化点设备均正确(QMatMul/QTensor 延迟反量化、RmsNorm 跟随 QTensor、embedding 正确传 device)
- 设备传递链: `MlContext::load_model → GGUF_Load_Model → Load_Stages` 全程正确,唯一断链点是 `Load_Stages → Build_From_Extracted`(签名无 device 参数)
- P2(session.rs 硬编码 cpu)/ P3(device_preference 只写不读):**用户决策暂不处理**
- Lua 脚本硬编码 cuda:0/cuda:1(pipe_worker 等):**本次不动**,另开任务

## 修复方案(3 处,单文件)

1. `Qwen3MoE_Layer::Build_From_Extracted` 签名加 `device: &Device`(model_dtype 之后)
2. `qwen3_moe.rs:117`:`gate_qt.dequantize(&Device::Cpu)` → `gate_qt.dequantize(device)`
3. `Load_Stages` 调用处(qwen3_moe.rs:240)传 `device`(参数已有)

## 验证 ✅ (2026-08-08)

- [x] `./build.sh check` 编译通过
- [x] release 编译通过
- [x] lib 单测(112 passed / 1 failed 预存 os 沙箱问题,与本修复无关)
- [x] 双卡 2 节点 30B pipeline 实测: 全部通过(详见 wb_19)
  - 模型加载无 device mismatch(核心验证)
  - 多轮对话 meow~ / 上下文记忆 / 静夜思长文本 / /clear 清除 全部 ✅

## 状态

- 开始: 2026-08-08
- 结束: 2026-08-08 ✅
