Presented by KeJi
Date: 2026-06-01

# Task 16: 动态流水线编排

> 状态：16.0~16.12 已完成，16.13 待实现

---

## 背景

当前所有 Lua 流水线脚本（`pipe_1.lua` ~ `pipe_8.lua`）都是**硬编码角色分配**：

- 每个脚本固定自己的 split 点（N/2）
- 固定找 "第一个非自己" 的 peer 作为 partner
- `pipe_7/pipe_8` 每个 GPU 固定 2 chunks
- 共 8 个高度重复的脚本

在真实集群上，节点数不固定、模型分片方式不固定，需要**动态发现 + 自动编排**。

---

## 目标

一个通用 Lua 脚本，自动完成：

1. **发现**：查询集群中哪些节点有目标模型的哪些层
2. **排序**：把节点按层范围排成 0→N 的完整链条
3. **分发**：rexec 到每个节点启动 worker，节点内部自动双 GPU 分片
4. **桥接**：节点间 tensor stream，节点内 GPU-CPU-GPU 桥接
5. **推理**：coordinator 控制循环：encode → 逐跳 forward → sample → decode

---

## 设计前提

- **所有节点默认 2 GPU**（`cuda:0` + `cuda:1`），不存在单卡或 3+ 卡节点
- **模型在每台机器上均分到两张卡**：若某节点持有 `[L_start, L_end]`，则前半段在 `cuda:0`，后半段在 `cuda:1`
- **本机流水线**：每个节点内部创建两个独立的 `ml context`（`sess_gpu0` / `sess_gpu1`），数据传输路径为 `GPU:0 → CPU → GPU:1`
- **跨机流水线**：`caps.network.send_tensor / recv_tensor`，不经过 CPU 中转

---

## 核心拓扑

```
Coordinator (任一节点)
  │
  ├─ Node 0: [L0 ────────── L10]
  │           GPU:0 → CPU → GPU:1
  │                         │ (tensor stream)
  ├─ Node 1: [L11 ───────── L21] ◄──────────────┘
  │           GPU:0 → CPU → GPU:1
  │                         │ (tensor stream)
  └─ Node 2: [L22 ───────── L42] ◄──────────────┘
              GPU:0 → CPU → GPU:1
                                │
                             sample / decode
```

### 传输方式

| 跳类型 | 方式 |
|--------|------|
| GPU:0 → GPU:1（同节点） | `tensor:to_device("cpu")` → `tensor:to_device("cuda:1")` |
| Node N → Node N+1（跨节点） | `caps.network.send_tensor` / `recv_tensor` |
| 最后节点输出 → 用户 | coordinator 负责 sample/decode |

---

## 任务拆解

### 16.0 分支准备

在当前分支 `hf2gguf` 基础上创建 `demo` 分支，Task 16 的所有后续操作（Rust 修改、Lua 脚本、归档等）均在 `demo` 分支上展开，保持 `hf2gguf` 分支不变。

```bash
git checkout -b demo hf2gguf
```

当所有工作完成后，由人类 Reviewer 确认是否合并回 `hf2gguf` 或 `master`。

---

### 16.1 Rust：暴露 Peer 模型信息给 Lua

**新增 Lua API**：`caps.network.list_model_peers(model_id)` — 接受 `u32` 类型的 model_id（xxhash32(tensor metadata)，跨分片唯一）

返回所有拥有目标模型的 peer 及其层范围：

```lua
{
  { peer_id = "12D3...",  name = "gpu-node-0",
    layer_start = 0,   layer_end = 21,
    file_name   = "DeepSeek-V4-Flash.pgguf",
    devices     = { "cuda:0", "cuda:1" },
    profile     = { latency_ms = 2 },
  },
  ...
}
```

**实现**：**全部在 Rust 端完成**，不涉及 Lua 侧逻辑。流程如下：

1. 在 `register_network_caps` 中新增 `list_model_peers` 绑定函数（Rust 原生函数，注册为 Lua callable）
2. 函数内部调用 `PeerManager::get_all_peers()` 获取所有在线 peer
3. 遍历结果，筛选 `supported_models` 中 `id` 匹配 `model_id` 的 peer
4. 对每个匹配 peer，将其 `SupportedModel` 字段（`layer_start`, `layer_end`, `file_name`, `devices`）和 `PeerProfile` 字段（`latency_ms`）合并序列化为 Lua table
5. 返回 Lua table 数组，Lua 侧直接使用，无需任何处理逻辑

Lua 侧调用示例：`local peers = caps.network.list_model_peers("DeepSeek-V4-Flash.pgguf")`，拿到的是已经过滤、格式化好的 table。

**涉及文件**：
- `Src/VM/capability_binding.rs` — 新增 `list_model_peers` 绑定 (~40 行)

---

### 16.2 Lua：通用 Coordinator 脚本

**新建** `programs/user/pipeline_coord.lua`，`COMMAND = "pipeline"`。

接受参数：
```
{
  model = "DeepSeek-V4-Flash.pgguf",
  prompt = "你好",
  temperature = 0.8,
  max_tokens = 60,
}
```

#### 流程

```
pipeline_coord.lua 执行流程：

1. caps.storage_acquire_read(model) → 获取 model_path
   ml.analyze_model(model_path) → 获取 total_layers

2. caps.network.list_model_peers(model) → peers[]
   │
   ├─ 按 layer_start 排序
   ├─ 验证: 每段 end+1 == 下一段 start（无间隙）
   ├─ 验证: 第一段 start==0，最后一段 end==total_layers
   └─ 失败 → 报错退出

3. for each node in chain (n=0,1,2,...):
   ├─ upstream   = (n==0) ? nil     : chain[n-1].peer_id
   ├─ downstream = (n==N) ? nil     : chain[n+1].peer_id
   └─ rexec node:
        "EXEC|pipe_worker|{model, layer_start, layer_end, upstream, downstream, inference_id}"

4. open tensor stream to chain[1] (第一个 worker 节点)
   accept tensor stream from chain[N] (最后一个 worker 节点)

5. sess = ml.new("cpu")          ← coordinator 只需要 encode/sample/decode
   sess:load_tokenizer(model_path)

6. tokens = sess:encode(prompt)
   hidden = sess:tensorize(tokens)  ← embedding 由 coordinator 做
   → send_tensor(chain[1].stream, hidden)

7. 推理循环 (max_tokens):
   ├─ logits = recv_tensor(chain[N].stream)
   ├─ token = sess:sample(logits, temperature)
   ├─ if token == eos: break
   ├─ output = sess:decode(token)  → print
   └─ next_hidden = sess:tensorize({token})
       → send_tensor(chain[1].stream, next_hidden)
```

**涉及文件**：
- `programs/user/pipeline_coord.lua` — 新增 (~120 行)

---

### 16.3 Lua：通用 Worker 脚本

**新建** `programs/user/pipe_worker.lua`，`COMMAND = "pipe_worker"`。

接受参数：
```
{
  model, layer_start, layer_end,
  upstream,    -- 上游 peer_id（nil = 我是第一跳）
  downstream,  -- 下游 peer_id（nil = 我是最后一跳）
  inference_id,
}
```

#### 流程

```
pipe_worker.lua 执行流程（在远程节点上运行）：

1. caps.storage_acquire_read(model) → model_path

2. 自动双 GPU 分片:
   my_layers = layer_end - layer_start + 1
   mid = layer_start + floor(my_layers / 2) - 1

   sess_gpu0 = ml.new("cuda:0")  → load_model(model_path, layer_start, mid)
   sess_gpu1 = ml.new("cuda:1")  → load_model(model_path, mid+1, layer_end)

3. 建立流:
   if upstream:    accept_tensor_stream(upstream, inference_id, timeout=120s)
   if downstream:  open_tensor_stream(downstream, inference_id)

4. 推理循环:
   loop:
     hidden = recv_tensor(upstream_stream, "cuda:0")
     │
     ├─ sess_gpu0:forward(hidden)
     ├─ hidden:to_device("cpu") → hidden:to_device("cuda:1")
     └─ hidden = sess_gpu1:forward(hidden)
     │
     if downstream:
       send_tensor(downstream_stream, hidden)
     else:
       send_tensor(return_stream, hidden)  ← 最后跳 → coordinator
```

**涉及文件**：
- `programs/user/pipe_worker.lua` — 新增 (~100 行)

---

### 16.4 Lua：单卡 Worker 脚本

**新建** `programs/user/pipe_worker_single.lua`，`COMMAND = "pipe_worker_single"`。

与 `pipe_worker.lua` 的区别：**只用 `cuda:0`，不涉及第二张卡**，因此没有本机 GPU→CPU→GPU 桥接。

接受参数：
```
{
  model, layer_start, layer_end,
  upstream, downstream,
  inference_id,
}
```

#### 流程

```
pipe_worker_single.lua 执行流程：

1. caps.storage_acquire_read(model) → model_path

2. 单卡加载:
   sess = ml.new("cuda:0")  → load_model(model_path, layer_start, layer_end)

3. 建立流（与双卡版相同）:
   if upstream:    accept_tensor_stream(upstream, inference_id, timeout=120s)
   if downstream:  open_tensor_stream(downstream, inference_id)

4. 推理循环（无 CPU 中转）:
   loop:
     hidden = recv_tensor(upstream_stream, "cuda:0")
     │
     └─ hidden = sess:forward(hidden)
     │
     if downstream:
       send_tensor(downstream_stream, hidden)
     else:
       send_tensor(return_stream, hidden)
```

**涉及文件**：
- `programs/user/pipe_worker_single.lua` — 新增 (~60 行)

---

### 16.5 Lua：单卡 Coordinator 脚本

**新建** `programs/user/pipeline_coord_single.lua`，`COMMAND = "pipeline_coord_single"`。

与 `pipeline_coord.lua` 唯一区别：第 3 步 rexec 时下发 `pipe_worker_single` 而非 `pipe_worker`。其余流程（发现、排序、验证、推理循环）完全一致。

**涉及文件**：
- `programs/user/pipeline_coord_single.lua` — 新增 (~120 行，基本复制 pipeline_coord.lua 后改一处)

---

### 16.6 清理：归档旧 pipe 脚本

`pipe_1.lua` ~ `pipe_8.lua` + `pipeline1.lua`/`pipeline2.lua` 共 10 个文件移入 `programs/archived/`。

`local_coord.lua` + `local_work.lua` 保留（本地双进程测试用）。

---

### 16.7 CLI 模式

新增 `./Pleiades cli` 交互式命令行模式。

- `Src/TUI/mod.rs`：提取 `pub fn parse_user_command()` 供 CLI/TUI 共用
- `Src/main.rs`：新增 CLI 分支
  - EventBus 订阅者 → stdout（Notify + Output 事件）
  - stdin REPL → parse_user_command → UserCommand → Core

**涉及文件**：
- `Src/TUI/mod.rs` — 提取 parse_user_command + 重构 Handle_Command_Input
- `Src/main.rs` — 新增 cli_mode 分支（~60 行）

---

### 16.8 修复：model_id 发现

`list_model_peers` 过滤条件从 `file_name.contains()` 改为 `m.id` 精确匹配。

- `Src/VM/capability_binding.rs`：`list_model_peers` 参数类型 `String` → `u32`，过滤改为 `m.id == model_id`
- `Src/VM/capability_binding.rs`：`analyze_model` 绑定新增 `model_id` 字段
- `pipeline_coord.lua` / `pipeline_coord_single.lua`：先 `analyze` 取 model_id → `list_model_peers(model_id)`，rexec 传 `p.file_name`
- 返回条目新增 `model_id` 字段

**涉及文件**：
- `Src/VM/capability_binding.rs` — analyze_model + model_id / list_model_peers 过滤改写

---

### 16.9 Session 模式重构

`pipeline_coord.lua` / `pipe_worker.lua` 从独立模式改为 Session 桥接模式。

- **用法变更**：`exec pipeline model=xxx` → `session inference pipeline <sid> <model>`
- **Coordinator**：不再自己做 tokenizer/encode/decode，改为桥接 `Session ↔ local_tensor ↔ network chain`
- **Worker**：接受 `session_id` 参数（用于 tensor stream 配对），增加演示级日志输出
- 同步更新单卡版本 `pipeline_coord_single.lua` / `pipe_worker_single.lua`

**涉及文件**：
- `programs/user/pipeline_coord.lua` — 重写为 Session 桥接模式
- `programs/user/pipe_worker.lua` — 适配 session_id + 演示日志
- `programs/user/pipeline_coord_single.lua` — 同步
- `programs/user/pipe_worker_single.lua` — 同步

---

### 16.10 实验脚本：E1 Scaling

**新建** `programs/user/exp_scale.lua`，`COMMAND = "exp_scale"`。

用法：`exec exp_scale model=xxx.pgguf nodes=N tokens=M`。coordinator 自己做 tokenizer → 发现 N 个节点 → rexec `pipe_worker` → encode + decode 计时 → 输出结构化结果。

输出格式：
```
==== EXP_SCALE RESULT ====
NODES:4  TOKENS:87  TOTAL_S:35.6  ENCODE_S:0.31
PREFILL_S:2.15  DECODE_S:33.2  TOK_S:2.62  TOK_S_E2E:2.44
==== EXP_SCALE END ====
```

**涉及文件**：
- `programs/user/exp_scale.lua` — 新增 (~130 行)

---

### 16.11 实验脚本：E3 单卡模式

**新建** `programs/user/exp_single.lua`，`COMMAND = "exp_single"`。

与 `exp_scale` 唯一区别：rexec `pipe_worker_single` 而非 `pipe_worker`。

**涉及文件**：
- `programs/user/exp_single.lua` — 新增 (~120 行)

---

### 16.12 实验脚本：三节点～五节点流水线

**新建 3 个独立实验脚本**，每个脚本自身包含完整的流水线逻辑（模型加载、forward、网络桥接）。不依赖 pipe_worker，不涉及 offloading。

| 脚本 | 节点 | 每卡载荷 | 描述 |
|------|------|---------|------|
| `exp_qwen_3.lua` | 3 | 45GB | 三机流水线（刚好装下） |
| `exp_qwen_4.lua` | 4 | 33.8GB | 四机流水线 |
| `exp_qwen_5.lua` | 5 | 27GB | 五机流水线 |

每个脚本用法：`exec exp_qwen_N model=xxx.pgguf tokens=M`

输出格式与 `exp_scale` 一致（`==== EXP_QWEN_N RESULT ==== ... ====`）。

**涉及文件**：
- `programs/user/exp_qwen_3.lua` — 新增
- `programs/user/exp_qwen_4.lua` — 新增
- `programs/user/exp_qwen_5.lua` — 新增

---

### 16.13 Llama 3.1 架构支持

Pleiades 使用的 candle 0.10.2 支持 Llama 架构，但 `gguf_model.rs` 的架构分发尚未包含 `"llama"`。

#### 可行性

| 维度 | 结论 |
|------|------|
| candle-core | ✅ 支持 Llama GGUF 加载 |
| candle-transformers | ✅ 提供 Llama 模型实现 |
| GGUF 架构标识 | `"llama"` |
| Tokenizer | shimmytok 0.7.1 支持 tiktoken-based tokenizer |

#### 任务

1. `gguf_model.rs`：架构分发新增 `"llama"` 分支
2. 新建 `Src/ML_Engine/GGUF_Models/llama.rs`：Llama 模型权重结构 + loader
3. `gguf_model.rs`：新增 `LLAMA_CFG_From_Metadata` 从 GGUF 元数据提取超参数
4. `Src/ML_Engine/GGUF_Models/mod.rs`：注册 llama 模块
5. Python 转换工具（可选）：`Tool/hf2gguf/mappings/` 新增 Llama 张量映射
6. 验证：`cargo check` 通过

**涉及文件**：
- `Src/ML_Engine/gguf_model.rs` — 架构分发 + 元数据提取
- `Src/ML_Engine/GGUF_Models/llama.rs` — 新增
- `Src/ML_Engine/GGUF_Models/mod.rs` — 注册模块

---

## 文件变更总览

```
分支：demo（从 hf2gguf 创建）

新增：
  programs/user/pipeline_coord.lua         — Session 模式 Coordinator (219 行)
  programs/user/pipe_worker.lua            — Session 模式 Worker (122 行)
  programs/user/pipeline_coord_single.lua  — 单卡 Coordinator (137 行)
  programs/user/pipe_worker_single.lua     — 单卡 Worker (88 行)
  programs/user/exp_scale.lua              — E1 Scaling 实验 (133 行)
  programs/user/exp_single.lua             — E3 单卡实验 (122 行)
  programs/user/exp_qwen_3.lua             — 3节点流水线实验
  programs/user/exp_qwen_4.lua             — 4节点流水线实验
  programs/user/exp_qwen_5.lua             — 5节点流水线实验
  programs/user/exp_worker.lua             — 实验用轻量 Worker (63 行)
  docs/exp_info.md                         — Fleet 实验环境总结
  docs/exp_plan.md                         — 演示+实验计划
  Task/task_16_dynamic_pipeline.md         — 本文档

修改：
  Src/VM/capability_binding.rs       — list_model_peers (model_id 过滤) + analyze_model (model_id 字段)
  Src/TUI/mod.rs                      — 提取 parse_user_command + 重构 Handle_Command_Input
  Src/main.rs                         — CLI 模式 ./Pleiades cli

待实现 (16.13):
  Src/ML_Engine/gguf_model.rs         — 新增 "llama" 架构分发
  Src/ML_Engine/GGUF_Models/llama.rs  — Llama 模型实现

归档：
  programs/user/pipe_1.lua ~ pipe_8.lua  → programs/archived/
  programs/user/pipeline1.lua            → programs/archived/
  programs/user/pipeline2.lua            → programs/archived/
```

---

## 人类评审

<!-- 在此区域写下评审意见 -->
