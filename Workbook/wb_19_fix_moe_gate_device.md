# wb_19_fix_moe_gate_device

## 任务
task_19_fix_moe_gate_device: 修 G1 MoE gate 反量化硬编码 CPU。

## 背景(2026-08-08)
- G1 来源: Task18 双卡 30B pipeline 测试, worker ML forward device mismatch。
- 子agent 全量排查 ML 设备问题: 仅 qwen3_moe.rs:117 一处"提前反量化到错误设备必崩"。
- 用户决策: P2/P3(device_preference 断链、session cpu)暂不管; Lua 硬编码不动。

## 实施
- 开始: 2026-08-08
- 改动: qwen3_moe.rs 3 处
  1. Build_From_Extracted 签名 +device: &Device (model_dtype 后)
  2. :117 dequantize(&Device::Cpu) → dequantize(device)
  3. Load_Stages :240 调用传 device
- 不涉及: gguf_model_legacy.rs(死代码无 mod 声明)、qwen3.rs(dense 全 QMatMul 无需 device)

## 验证 (2026-08-08)
- `./build.sh check` ✅ Finished, 无 error(22 预存 warning, 无 moe 相关)
- `cargo test --lib` ✅ 112 passed / 1 failed(唯一失败 test_sandbox_os_blocked 为预存 os 沙箱问题, 与本修复无关)
- `./build.sh`(release) ✅ Finished in 17.15s, 无 error

## 双卡 2 节点实测 (2026-08-08) ✅ 全部通过

部署: `./build.sh deploy` ✅ → 节点1(CUDA 0,1)/节点2(CUDA 0,1)

| 验证项 | 结果 |
|--------|------|
| mDNS 双向发现 | ✅ |
| 30B 模型加载(50 层 MoE,含 G1 的 MoE 层) | ✅ **无 device mismatch**(G1 修复生效) |
| 流水线就绪(worker: GPU0[0-24] + GPU1[25-49]) | ✅ worker 就绪等待推理 |
| API 启动 (:8080) | ✅ |
| 首轮: "你好...以meow~结尾" → "Hello meow~" | ✅ meow~ 结尾 |
| 二轮: "今天天气真好..." → "...meow~" | ✅ 上下文记忆(think 引用指令) |
| 三轮: "背诵静夜思" → 四句每句 meow~ | ✅ 长文本 + meow~ |
| /clear → "最喜欢的动物?" → 无 meow~ | ✅ 上下文正确清除 |
| quit 退出 | ✅ 节点1/2 均干净退出 |

### 过程备注
- chat.py 交互式输入(无 -s system prompt),max-tokens 默认 256 会截断 Qwen3 think 输出 → 用 --max-tokens 1024
- 中途遇一次模型退化(重复"一些"),根因: 被 Ctrl-C 中断的请求污染 Session KV cache 状态 → 重启节点后恢复正常(非 G1 问题)
- worker 日志: ⚡ Prefill: GPU0=0.14s GPU1=0.16s,每 token ~10ms(GPU0)/~28ms(GPU1)

## 3 节点单卡实测 (2026-08-08) ✅ 全部通过

部署: `pipeline_coord_single` → 节点1(A,桥接) + node-b(层[0-24]) + C(层[25-49])

| 验证项 | 结果 |
|--------|------|
| mDNS 三节点互联 | ✅ |
| MoE 模型跨 2 worker 切分加载(各 25 层) | ✅ 无 device mismatch(15.24s / 14.74s) |
| 流水线就绪(fwd→node-b, bwd←C) | ✅ |
| 首轮: meow~ 指令 → "你好！meow~" | ✅ |
| 二轮: 散步 → "好的，散步是个好主意！meow~" | ✅ 上下文记忆 |
| 三轮: 静夜思 → 诗句 + meow~ | ✅ 长文本 |
| /clear → 动物问题 → 无 meow~ | ✅ 上下文清除 |
| worker 性能 | ⚡ Prefill FWD=0.14-0.16s; 20 tokens FWD=9ms(node-b)/23ms(C) |
| quit 退出 | ✅ 3 节点干净退出 |

## 结论 (2026-08-08)
- ✅ G1 修复在双卡 2 节点 + 单卡 3 节点两种模式下均实测通过
- ✅ MoE gate 反量化到模型所在设备, 分布式 MoE 推理完全正常
- 结束: 2026-08-08
