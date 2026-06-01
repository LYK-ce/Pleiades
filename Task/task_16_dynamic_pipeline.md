Presented by KeJi
Date: 2026-06-01

# Task 16: 动态流水线编排

> 状态：方案已完成，待执行 Phase 1

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

### 16.1 Rust：暴露 Peer 模型信息给 Lua

**新增 Lua API**：`caps.network.list_model_peers(model_id_or_name)`

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

**实现**：在 `register_network_caps` 中新增一个 async Lua 函数，调用 `PeerManager::get_all_peers()`，过滤出 `supported_models` 中包含目标 model_id 的 peer，序列化 `SupportedModel` + `PeerProfile` → Lua table。

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

### 16.4 清理：归档旧 pipe 脚本

`pipe_1.lua` ~ `pipe_8.lua` + `pipeline1.lua`/`pipeline2.lua` 共 10 个文件移入 `programs/archived/`。

`local_coord.lua` + `local_work.lua` 保留（本地双进程测试用）。

---

## 文件变更总览

```
分支：hf2gguf

新增：
  programs/user/pipeline_coord.lua   — Coordinator (120 行)
  programs/user/pipe_worker.lua      — 通用 Worker (100 行)
  Task/task_16_dynamic_pipeline.md   — 本文档

修改：
  Src/VM/capability_binding.rs       — list_model_peers 绑定 (~40 行)

归档：
  programs/user/pipe_1.lua ~ pipe_8.lua  → programs/archived/
  programs/user/pipeline1.lua            → programs/archived/
  programs/user/pipeline2.lua            → programs/archived/
```

---

## 人类评审

<!-- 在此区域写下评审意见 -->
