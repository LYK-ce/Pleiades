# 指令设计

## 寄存器使用约定

### 文本寄存器
| 常量             | 寄存器       | 用途                                       | 生命周期             |
| -------------- | --------- | ---------------------------------------- | ---------------- |
| `TEXT_PROMPT`  | **TEXT1** | 输入 Prompt（原始文本）                          | 单次请求只读           |
| `TEXT_CHUNK`   | **TEXT2** | **当前 step 生成的文本片段**（如一个子词 "ing"）         | 每步更新，流式输出用       |
| `TEXT_HISTORY` | **TEXT3** | 保留（多轮对话历史等未来用途）                         | -                |
| `TEXT_SYSTEM`  | **TEXT4** | 系统指令/模板（如 "You are a helpful assistant"） | 单次请求只读           |

### Token ID 寄存器
| 常量                  | 寄存器          | 用途                                            | 说明                |
| ------------------- | ------------ | --------------------------------------------- | ----------------- |
| `TOKENS_FULL`       | **TOKENID1** | **完整 Token 序列**（Prompt tokens + 所有已生成 tokens） | Sample 每步自动追加     |
| `TOKEN_NEW`         | **TOKENID2** | **当前 step 新生成的单个 Token ID**（采样结果）             | 长度固定为 1，用于自回归输入   |
| `TOKENS_PROMPT`     | **TOKENID3** | 原始 Prompt 的 Token IDs（缓存，避免重复 Tokenize）       | Prefill 阶段写入，后续只读 |
| `TOKENS_CANDIDATES` | **TOKENID4** | 候选 Token 列表（投机采样/对比搜索用）                       | 高级功能预留            |

### Tensor 寄存器
| 常量                 | 寄存器         | 用途                                         | Shape 示例                      |
| ------------------ | ----------- | ------------------------------------------ | ----------------------------- |
| `TENSOR_INPUT`     | **TENSOR1** | 当前 step 的输入张量（Receive 写入 / 推理输入）         | `[1, 1, hidden_dim]`（单 token） |
| `TENSOR_OUTPUT`    | **TENSOR2** | 模型输出的张量，可能是 logits 也可能是 hidden state       | `[1, vocab_size]`             |
| `TENSOR_POS`       | **TENSOR3** | 位置编码（或 Position IDs 张量）                    | `[1, 1]` 或 `[1, current_len]` |
| `TENSOR_ATTN_MASK` | **TENSOR4** | Attention Mask（或 KV Cache 引用标记）            | 因果掩码或 sliding window 掩码      |

### 标志寄存器
| 常量                 | 寄存器       | 用途                               | 触发条件                                             |
| ------------------ | --------- | -------------------------------- | ------------------------------------------------ |
| `FLAG_BREAK`       | **FLAG1** | **统一终止标志**                       | 任何需要退出 Loop 的条件均设此 FLAG                          |
| `FLAG_CACHE_READY` | **FLAG2** | KV Cache 已填充                     | 保留，当前未使用                                         |
| `FLAG_RESERVED`    | **FLAG3** | 保留                               | -                                                |
| `FLAG_ERROR`       | **FLAG4** | **错误标志**（指令执行出错时设置）              | 任何指令执行失败时设为 `true`，同时设 `FLAG_BREAK`=true 触发退出循环 |

### 元数据寄存器
| 常量               | 寄存器       | 用途                                       | 更新时机                               |
| ---------------- | --------- | ---------------------------------------- | ---------------------------------- |
| `META_OFFSET`    | **META1** | **当前序列长度**（即 `TOKENS_FULL.len()`，用于位置编码） | Encode 设初始值；Sample 每步 `+1`         |
| `META_REMAINING` | **META2** | **剩余可生成 Token 数**（从 `max_tokens` 递减）     | Sample 每步 `-1`，到 0 时设 `FLAG_BREAK` |
| `META_TIMESTAMP` | **META3** | 当前 step 开始时间戳（性能统计）                      | 每步重置                               |
| `META_STEP_IDX`  | **META4** | 已执行生成步数（从 0 开始）                          | Sample 每步 `+1`                     |
| `META5-META8`    | -         | **保留**                                   | 用于特定算法（如投机采样等）                     |

---

## 指令集

### 推理输入参数类型

`Inference` 指令的输入来源由参数指定，支持两种类型：

```
Inference_Input:
  Tokens(Token_Reg)    — 从 Token ID 寄存器读取，内部先做 embedding 再 forward
  Tensor(Tensor_Reg)   — 从 Tensor 寄存器读取，直接用张量做 forward（分布式 Worker 场景）
```

### 指令表

| 指令                          | 语义（基于寄存器约定）                                                                                             | 备注                          |
| --------------------------- | --------------------------------------------------------------------------------------------------------- | --------------------------- |
| **Input**                   | 阻塞读取 `input_data_rx` → **TEXT1**                                                                          | 等待 Control 层 Send\_Input    |
| **Encode**                  | **TEXT1** → **TOKENID3**，清空 **TOKENID1**，设 **META1** = len(TOKENID3)                                      | 准备接收生成序列，初始化 offset        |
| **Set(reg, value)**         | 将字面量 value 写入指定寄存器                                                                                       | 用于初始化运行时状态（如 META2=max\_tokens） |
| **Inference(input)**        | 根据 input 类型执行前向推理，输出写入 **TENSOR2**                                                                        | input 可为 Token\_Reg 或 Tensor\_Reg |
| **Sample(tensor\_reg)**     | 从指定 Tensor 寄存器采样 → **TOKENID2**，追加到 **TOKENID1**；维护 META1/META2/META4；EOS 或 remaining≤0 时设 **FLAG1**=true | 采样 + 序列维护 + 终止检查            |
| **Decode**                  | **TOKENID2** → **TEXT2**（当前文本片段）                                                                           | 增量解码，用于流式输出                 |
| **Output**                  | **TEXT2** → 发送到 `output_data_tx`                                                                           | 流式输出给 Control 层             |
| **Send**                    | **TENSOR2** → 序列化 → `tensor_io.Send()`                                                                    | 发送模型输出张量到下游节点               |
| **Receive**                 | `tensor_io.Receive()` → **TENSOR1**；收到 EOF 哨兵帧时设 **FLAG1**=true                                           | 从上游节点接收张量                   |
| **Loop { body }**           | 循环执行 body 指令序列，每次迭代后检查 FLAG1                                                                              | FLAG1=true 时退出循环            |
| **BreakIf**                 | 检查 **FLAG1**，若为 true 则设 `should_break` 并返回                                                                | 统一出口，配合 Loop 使用             |
| **SendEOF**                 | 通过 `tensor_io` 发送 EOF 哨兵帧                                                                                | 通知下游节点推理结束（仅分布式场景）         |
| **EndOutput**               | 通过 `output_data_tx` 发送结束信号给 Control 层                                                                     | 所有场景均需调用，通知 Control 流式输出结束 |

### 指令副作用详解

#### Encode 副作用
1. **TEXT1** → tokenize → **TOKENID3**（Prompt token IDs）
2. 清空 **TOKENID1**（准备接收生成序列）
3. **META1**(offset) = len(**TOKENID3**)（初始化位置偏移量）

#### Sample 副作用
`Sample` 是每步生成的核心指令，负责采样、序列维护和终止检查：
1. 从指定 Tensor 寄存器读取 logits，采样得到 next\_token → **TOKENID2**
2. 将 next\_token 追加到 **TOKENID1**（维护完整序列）
3. **META1**(offset) += 1
4. **META2**(remaining) -= 1
5. **META4**(step\_idx) += 1
6. 若 next\_token == eos\_token\_id → **FLAG1** = true
7. 若 **META2** ≤ 0 → **FLAG1** = true

#### Receive 副作用
1. 从网络接收张量帧 → **TENSOR1**
2. 若收到 EOF 哨兵帧（`offset=u64::MAX, length=0`）→ **FLAG1** = true

#### 错误处理
任何指令执行出错时：
1. **FLAG4**(ERROR) = true
2. **FLAG1**(BREAK) = true（触发循环退出）

### 寄存器生命周期

- 寄存器在 `Create_Session` 时通过 `Register_File.Reset_All()` 初始化清空
- 每次 `Run_Program` 开始时由 Engine 自动调用 `Reset_All()` 清空上次残留状态
- Program 不需要手动重置 FLAG/META

---

## 三种场景的程序编排

### 单机推理
```
Input                              TEXT1 ← prompt
Encode                             TEXT1 → TOKENID3, 清空 TOKENID1, META1 = prompt_len
Set(META2, max_tokens)             初始化 remaining
Inference(TOKENID3)                Prefill: TOKENID3 → TENSOR2 (logits)
Sample(TENSOR2)                    采样 → TOKENID2, 追加 TOKENID1, 维护 META, 检查终止
Decode                             TOKENID2 → TEXT2
Output                             TEXT2 → Control
Loop [
    BreakIf                        FLAG1=true → 退出
    Inference(TOKENID2)            Decode step: TOKENID2 → TENSOR2 (logits)
    Sample(TENSOR2)                采样 → TOKENID2, 追加 TOKENID1, 维护 META, 检查终止
    Decode                         TOKENID2 → TEXT2
    Output                         TEXT2 → Control
]
EndOutput                          通知 Control 流式输出结束
```

### Coordinator（分布式协调者）
```
Input                              TEXT1 ← prompt
Encode                             TEXT1 → TOKENID3, 清空 TOKENID1, META1 = prompt_len
Set(META2, max_tokens)             初始化 remaining
Inference(TOKENID3)                Prefill: TOKENID3 → TENSOR2 (本机 layers 的 hidden state)
Send                               TENSOR2 → 下游节点
Receive                            上游节点 → TENSOR1 (最终 logits), EOF→FLAG1
Sample(TENSOR1)                    采样 → TOKENID2, 追加 TOKENID1, 维护 META, 检查终止
Decode                             TOKENID2 → TEXT2
Output                             TEXT2 → Control
Loop [
    BreakIf                        FLAG1=true → 退出
    Inference(TOKENID2)            Decode step: TOKENID2 → TENSOR2 (hidden state)
    Send                           TENSOR2 → 下游节点
    Receive                        上游节点 → TENSOR1 (logits), EOF→FLAG1
    Sample(TENSOR1)                采样 → TOKENID2, 追加 TOKENID1, 维护 META, 检查终止
    Decode                         TOKENID2 → TEXT2
    Output                         TEXT2 → Control
]
SendEOF                            通知下游节点推理结束
EndOutput                          通知 Control 流式输出结束
```

### Worker（分布式工作节点）
```
Loop [
    Receive                        上游节点 → TENSOR1, EOF→FLAG1
    BreakIf                        FLAG1=true → 退出
    Inference(TENSOR1)             TENSOR1 → TENSOR2 (本机 layers 的 hidden state 或 logits)
    Send                           TENSOR2 → 下游节点
]
```
