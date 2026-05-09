# Pipeline 分布式推理测试问题记录

## 测试日期：2026-05-04

## 测试环境

- 同机双节点（两个目录模拟）
- 模型：Qwen3-0.6B-Q8_0.gguf（28 层）
- 分区：Coordinator layers 0-14，Worker layers 14-28
- 连接：mDNS 本地自动发现

## Pipeline 编排结果

| 阶段 | 状态 | 说明 |
|------|------|------|
| AnalyzeModel | ✅ | 正确识别 28 层 qwen3 |
| PlanPipeline | ✅ | Scheduler 均分：Coord 0-14, Worker 14-28 |
| EstablishStreams | ✅ | 双向张量流建立成功 |
| JoinWorkers | ✅ | Worker 回复 OK |
| CreateSession (Coord) | ✅ | layers 0-14, has_input_head=true, has_output_head=false |
| CreateSession (Worker) | ✅ | layers 14-28, **has_output_head=false** ← Bug |
| RunProgram (Coord) | ✅ | Input→Encode→Prefill→Send 均成功 |
| RunProgram (Worker) | 🔴 | Inference 失败，shape mismatch |

## 问题 1：Worker 模型加载 `has_output_head` 标志错误

### 现象

Worker 加载 layers 14-28（总共 28 层模型的后半段），日志显示：

```
Session [job-2]: 模型加载完成 (arch: qwen3, layers: 28, input_head: false, output_head: false, tokenizer: false)
```

`has_output_head` 应为 `true`，因为 layer_end=28 == num_layers，包含了最后一层，应该携带 lm_head 和 final_norm。

### 影响

- 虽然当前 Relay 程序不需要 Sample（不需要 output head 做采样），但这个标志不正确可能影响模型加载时是否包含 lm_head 权重

### 定位

- 文件：`Src/ML_Engine/gguf_model.rs` 中的 `GGUF_Load_Model` 函数
- 逻辑：layer_end == num_layers 时应设置 has_output_head = true

---

## 问题 2：Worker 部分模型推理 Shape Mismatch

### 现象

Worker 收到 Coordinator 的 hidden states 后执行 Inference 失败：

```
[Inference] 推理失败: Forward pass failed: shape mismatch in broadcast_add, 
  lhs: [1, 16, 23, 23], rhs: [1, 1, 23, 46]
```

### 分析

- `[1, 16, 23, 23]` — 注意力分数矩阵：batch=1, heads=16, seq_len=23, seq_len=23
- `[1, 1, 23, 46]` — 位置编码/注意力掩码：batch=1, 1, seq_len=23, **cache_len=46**
- `23` = prompt 编码后的 token 数
- `46` = KV cache 已有的长度（是 23 的两倍，异常）

### 根因推测

Worker 从第 14 层开始处理，其 KV cache 的初始化与 Coordinator 传来的 metadata（offset=23）不匹配。可能的原因：

1. **KV cache 预分配逻辑**：Worker 的模型在初始化时根据 offset 预分配了 KV cache，但计算时产生了不一致的维度
2. **位置编码偏移**：Worker 的注意力位置编码（RoPE）没有正确使用 Coordinator 传来的起始位置 offset
3. **Prefill 与 Decode 模式混淆**：Coordinator 做 Prefill（一次性处理 23 个 token），Worker 收到的 hidden states 形状与 Worker 期望的输入形状不匹配

### 定位

- 文件：`Src/ML_Engine/GGUF_Models/qwen3.rs` 中的 forward pass 实现
- 关键函数：attention 计算、KV cache 初始化、位置编码相关

---

## 问题 3（次要）：时序相关观察

### Coordinator 第一次尝试

Coordinator 日志中第一次 pipeline（inference_id=...3393）在 Worker 加载模型之前就发送了数据（两节点模型加载时间恰好对齐，因为同一个模型文件 ~15s 加载时间）。

时序对齐说明编排逻辑正确：
- Coordinator 等待 Worker 的 JOIN_PIPELINE OK（Worker model load 15s）
- Coordinator 自身 model load 也是 15s
- 两者并行加载，符合预期

---

## 修复优先级

| # | 问题 | 优先级 | 涉及模块 |
|---|------|--------|---------|
| 1 | Shape mismatch (attention/KV cache) | P0 | ML_Engine / qwen3.rs |
| 2 | has_output_head 标志检测 | P1 | ML_Engine / gguf_model.rs |

## 后续验证

修复上述 ML Engine 问题后，使用相同的双节点测试环境重新验证：
1. Worker Inference 应成功，产生输出 tensor
2. Worker Send 应成功，将结果发回 Coordinator
3. Coordinator Receive 应收到有效 tensor（非 EOF）
4. Coordinator Sample → Decode → Output → TUI 显示生成文本
