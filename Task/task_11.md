# Task 11: Branch and ML Refine

> Presented by KeJi
> Date: 2026-05-29

## 描述

分支管理与 ML 引擎精细化。

## 子任务总览

| # | 任务 | 涉及文件数 | 状态 |
|---|------|-----------|------|
| 11.1 | Split Model 适配 PGGUF 格式 | 1 | ✅ |
| 11.2 | 恢复 Network Command Branch — 支持远程 exec Lua 脚本 | 4 | ✅ |
| 11.3 | Session Inference 支持用户自定义 ML Thread 脚本 | 3 | ✅ |
| 11.4 | 用户 split.lua 脚本 — PGGUF 均分切分 (含 tokenizer 分配) | 4 | ✅ |
| 11.5 | 分布式流水线推理 pipe_1/pipe_2 — session inference + rexec 自动编排 | 3 | ✅ |
| 11.6 | CPU/GPU 混合流水线 pipe_3/pipe_4 — 单个分片内 CPU+GPU 分层加载 | 2 | ⬜ |

---

## 详细实施计划

### 11.1 Split Model 适配 PGGUF 格式

#### 背景

PGGUF 格式在标准 GGUF metadata 基础上追加了三个 Pleiades 扩展键：

| Key | 类型 | 含义 |
|-----|------|------|
| `pleiades.model_id` | `U32` | 原始 GGUF 文件的 xxhash32，模型唯一标识 |
| `pleiades.layer_bitmap` | `String` (64-char hex) | 256-bit 位图，bit N=1 表示 blk.N 层的 tensor 存在于本文件 |
| `pleiades.split.start` / `pleiades.split.end` | `U32` | 切分模型的层范围标记 (仅 split 文件有) |

`GGUF_Split_Model` 当前实现（`Src/ML_Engine/gguf_model_manager.rs:791–994`）**未正确适配 PGGUF 格式**，导致 split 产物的 metadata 与 PGGUF 语义不一致。

#### 现状分析

当前 `GGUF_Split_Model` 的 metadata 处理：

```
复制原始 metadata → 跳过 tokenizer.* 和 chat_template → 追加 pleiades.split.{start,end}
```

由于源文件是 PGGUF（含 `pleiades.model_id` + `pleiades.layer_bitmap`），这些键会被**原样复制**，但存在以下问题：

| 键 | 当前行为 | 问题 |
|----|---------|------|
| `pleiades.model_id` | 原样复制 | ✅ 正确，split 来源于同一模型 |
| `pleiades.layer_bitmap` | **原样复制** | ❌ bitmap 仍表示完整模型的所有层，未收缩到 split 范围 |
| `pleiades.split.{start,end}` | 新增 | ✅ 正确 |
| `block_count` | 保留原始值 | ✅ 正确，空层自然显示 0 tensors |
| tokenizer.* / chat_template | 无条件删除 | ⚠️ 需确认：split 文件不需要推理，删除合理 |

#### 待修改内容

**核心改动：更新 `pleiades.layer_bitmap`**

在 `GGUF_Split_Model` 中，metadata 复制阶段需要：

1. **识别并移除旧的 `layer_bitmap`**：复制 metadata 时，跳过 `pleiades.layer_bitmap`（不原样复制）

2. **重新计算 split 范围内的 bitmap**：根据已筛选的 `selected_tensor_names`，构建新的 bitmap，只标记实际包含 blk tensor 的层：

   ```rust
   // 伪代码
   let mut new_bitmap = [0u8; 32];
   for tensor_name in &selected_tensor_names {
       if tensor_name.starts_with("blk.") {
           // 提取 blk 索引并置位
           let blk_idx = extract_blk_index(tensor_name);
           new_bitmap[blk_idx / 8] |= 1 << (blk_idx % 8);
       }
   }
   ```

3. **追加新的 `layer_bitmap`**：与 `pleiades.split.{start,end}` 一起写入

4. **复用现有辅助函数**：
   - `Build_Layer_Bitmap()` (line 509) — 可能需要新增一个接受 `&[String]` 的变体
   - `Bitmap_To_Hex()` — 序列化 bitmap 为 64-char hex 字符串

#### 涉及文件

| 文件 | 改动范围 |
|------|---------|
| `Src/ML_Engine/gguf_model_manager.rs` | `GGUF_Split_Model` 函数 (~lines 928–945)，metadata 复制逻辑 |

#### 验证方式

完成后，对一个 PGGUF 文件执行 split 后，检查产物：
1. `pleiades.layer_bitmap` 只标记 split 范围内的 blk 层
2. `pleiades.model_id` 与源文件一致
3. `pleiades.split.{start,end}` 正确
4. `GGUF_Analyze` 读取 split 产物时 `arch_info.layer_bitmap` 正确

---

### 11.2 恢复 Network Command Branch — 支持远程 exec Lua 脚本

#### 背景

当前 Orchestrator 的 B2 入站路由 (`branch_command.rs`) 是一个 **stub**——收到 `DataType::Command` 后仅打日志，不做任何处理。同时，用户侧也缺少将命令发送到远程节点的途径。

需要恢复 Network Command 的完整双向通道，第一步实现 `exec` 命令的远程版本：在本地通过 TUI 输入命令，将 Lua 脚本名称和参数发送到远程节点执行，并获取结果。

#### 现有基础设施 (可直接复用)

| 组件 | 状态 | 说明 |
|------|------|------|
| `network.send_data(peer, DataType::Command, bytes)` | ✅ | 可靠请求-响应，发送任意字节到 peer |
| `network.send_response(request_id, DataType, payload)` | ✅ | 响应入站请求 |
| `InboundRequest { peer, data_type, payload, request_id, ... }` | ✅ | B2 接收的入站请求结构 |
| `Parse_Network_Command(bytes)` / `Serialize_Network_Command(cmd)` | ✅ | 文本协议解析/序列化框架 |
| `spawn_lua_script(path, params, caps, label)` | ✅ | 本地 Lua 脚本执行（fire-and-forget，通过 EventBus 输出） |
| `ProgramRegistry::get_user(command)` | ✅ | 按命令名查找用户 Lua 脚本 |
| TUI `exec <cmd> [k=v ...]` 解析 | ✅ | 已有模式可直接扩展 |

#### 数据流设计

```
┌─────────────────────────────────────────────────────────────────┐
│ 本地节点 A (发起方)                                              │
│                                                                 │
│  TUI: rexec peerB my_script k1=v1 k2=v2                        │
│    │                                                             │
│    ▼ UserCommand::ExecRemote                                    │
│  B1 route_user()                                                │
│    │                                                             │
│    ▼ Serialize_Network_Command() → "EXEC|my_script|{...json}"  │
│  network.send_data(peerB_id, DataType::Command, bytes)          │
│    │                                                             │
│    │ ════════════ 网络 ════════════                              │
│    ▼                                                             │
└─────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────┐
│ 远程节点 B (执行方)                                              │
│                                                                 │
│  B2 route_inbound() ← InboundRequest                            │
│    │                                                             │
│    ▼ Parse_Network_Command(bytes) → ExecRemote{cmd, params_json}│
│  查 ProgramRegistry → 找到 .lua 脚本路径                         │
│    │                                                             │
│    ▼ spawn_lua_script_with_reply(path, params, caps, reply_tx)  │
│  等待脚本执行完成 → 获取结果字符串                                │
│    │                                                             │
│    ▼ send_response(request_id, DataType::Command, result_bytes) │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

#### 待修改内容

##### 2a. `Src/Orchestrator/command.rs` — 新增协议定义

**`NetworkProtocol` 新增变体：**

```rust
/// 请求远程节点执行指定 Lua 脚本
///
/// 格式: `EXEC|{command}|{params_json}`
///   - command: 脚本 COMMAND 名（对应 programs/user/{command}.lua）
///   - params_json: JSON 格式的参数字典（避免 `|` 分隔符冲突）
///
/// 回复: `OK|{result}` 或 `FAIL|{reason}`
ExecRemote {
    command: String,
    /// JSON string of HashMap<String, String>
    params_json: String,
},
```

**`Parse_Network_Command` 新增分支：** 匹配 `"EXEC"` 前缀，3 字段。解析 `command` + `params_json`。

**`Serialize_Network_Command` 新增分支：** 输出 `format!("EXEC|{}|{}", command, params_json)`。

**`UserCommand` 新增变体：**

```rust
/// 在远程节点执行 Lua 脚本
ExecRemote {
    /// 目标节点名称或 ID
    peer: String,
    /// Lua 脚本 COMMAND 名
    command: String,
    /// 参数键值对
    params: HashMap<String, String>,
},
```

**新增测试：** `Parse_Network_Command` 和 roundtrip 对 `ExecRemote` 的测试。

##### 2b. `Src/Orchestrator/core/branch_user.rs` — B1 发起端

新增 `UserCommand::ExecRemote` 分支（在 `route_user` 中，约 line 99 之后）：

1. **解析 peer**：通过 `peer_manager.Get_Peer_By_Name(name)` 将名称解析为 peer_id
2. **序列化参数为 JSON**：`params` → `serde_json::to_string(&params)`
3. **构造并序列化**：`NetworkProtocol::ExecRemote { command, params_json }` → `Serialize_Network_Command()`
4. **发送**：`caps.network.send_data(peer_id, DataType::Command, bytes).await`
5. **显示响应**：将响应的 payload 通过 EventBus 输出到 TUI

##### 2c. `Src/Orchestrator/core/branch_command.rs` — B2 执行端

完整实现 `DataType::Command` 分支（替换当前 stub）：

1. **解析协议**：`Parse_Network_Command(&req.payload)` → `NetworkProtocol`
2. **匹配 `ExecRemote`**：提取 `command` + `params_json`
3. **查找脚本**：`self.program_registry.get_user(&command)` 查找 Lua 脚本路径
4. **执行并等待结果**：调用新增的 `spawn_lua_script_with_reply()` → `oneshot::Receiver`
5. **发送响应**：`self.capabilities.network.send_response(req.request_id, DataType::Command, result_bytes)`

```rust
DataType::Command => {
    match Parse_Network_Command(&req.payload) {
        Ok(NetworkProtocol::ExecRemote { command, params_json }) => {
            let params: HashMap<String, String> =
                serde_json::from_str(&params_json).unwrap_or_default();
            match self.program_registry.get_user(&command) {
                Some(entry) => {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    spawn_lua_script_with_reply(
                        entry.path.clone(), params,
                        self.capabilities.clone(), command.clone(), tx,
                    );
                    let result = tokio::time::timeout(
                        Duration::from_secs(60), rx,
                    ).await
                        .unwrap_or(Ok("TIMEOUT".to_string()))
                        .unwrap_or_else(|_| "ERR".to_string());
                    let _ = self.capabilities.network.send_response(
                        req.request_id, DataType::Command,
                        result.into_bytes(),
                    ).await;
                }
                None => {
                    let _ = self.capabilities.network.send_response(
                        req.request_id, DataType::Command,
                        format!("FAIL|未知命令: {}", command).into_bytes(),
                    ).await;
                }
            }
        }
        Ok(other) => tracing::info!("[B2] 未处理的协议: {:?}", other),
        Err(e) => {
            let _ = self.capabilities.network.send_response(
                req.request_id, DataType::Command,
                format!("FAIL|协议解析: {}", e).into_bytes(),
            ).await;
        }
    }
}
```

##### 2d. `Src/Orchestrator/core/branch_user.rs` — 新增 `spawn_lua_script_with_reply`

新增一个与 `spawn_lua_script` 功能相同但通过 `oneshot::Sender` 返回结果的变体。

与 `spawn_lua_script` 的区别：
- 所有 `EventBus::Output` 发布替换为 `reply.send(...)`
- 成功时 `reply.send("OK|...")`，失败时 `reply.send("FAIL|...")`
- 也可在已有 `spawn_lua_script` 上增加 `Option<oneshot::Sender<String>>` 参数统一接口

##### 2e. `Src/TUI/mod.rs` — TUI 命令解析

在 `Handle_Command_Input` 中新增 `rexec` 命令（仿照 `exec` 解析，约 line 912 之后）：

```
rexec <peer> <command> [key=value ...]
```

解析逻辑：提取 `peer`（第一参数）→ `command`（第二参数）→ 剩余 `key=value` → 构造 `UserCommand::ExecRemote`。

#### 涉及文件

| 文件 | 改动点 |
|------|--------|
| `Src/Orchestrator/command.rs` | `NetworkProtocol::ExecRemote` + `UserCommand::ExecRemote` + 解析/序列化/测试 |
| `Src/Orchestrator/core/branch_user.rs` | B1 `ExecRemote` 分支 + `spawn_lua_script_with_reply` |
| `Src/Orchestrator/core/branch_command.rs` | B2 `DataType::Command` 完整实现 |
| `Src/TUI/mod.rs` | `rexec` 命令解析 |

#### 验证方式

1. 在本地启动两个节点 A 和 B，建立 P2P 连接
2. 准备测试 Lua 脚本 `programs/user/hello.lua`：
   ```lua
   -- COMMAND: hello
   function execute(params)
       local name = params.name or "world"
       return "Hello, " .. name .. "!"
   end
   ```
3. A 的 TUI 中执行 `rexec B hello name=KeJi`
4. 确认 B 的日志显示收到并执行了脚本
5. 确认 A 的 TUI 显示 `[B] 远程执行完成: OK|Hello, KeJi!`

---

### 11.3 Session Inference 支持用户自定义 ML Thread 脚本

#### 背景

当前 `session inference <sid> <model_path>` 固定使用 `programs/builtin/inference.lua`。需要：

1. 将 `inference.lua` 移动到 `programs/user/` 并重命名为 `single_inf.lua`
2. 改为 `session inference <command> <sid> <model_path>`，`command` 对应 `programs/user/` 下的 Lua 脚本

`ProgramRegistry` 的查找逻辑自动支持：`get_user("inference")` 返回 user 下的脚本，`get("inference")` 返回 user 优先→builtin 回退。

#### 改动

##### 3a. 移动并重命名脚本

```
programs/builtin/inference.lua → programs/user/single_inf.lua
```

无代码改动，仅文件移动+重命名。`ProgramRegistry::reload_user()` 会自动扫描新位置，COMMAND 从脚本内容读取。

##### 3b. `Src/Orchestrator/command.rs` — `UserCommand::SessionInference` 扩展

```rust
SessionInference {
    /// 用户 Lua 脚本 COMMAND 名（对应 programs/user/{command}.lua）
    /// 默认 "single_inf" (programs/user/single_inf.lua)
    command: String,
    session_id: u64,
    model_path: String,
},
```

##### 3c. `Src/Orchestrator/core/branch_user.rs` — B1 调度逻辑

当前（约 line 571）：
```rust
UserCommand::SessionInference { session_id, model_path } => {
    let Some(entry) = self.program_registry.get("inference").cloned() else { ... };
```

改为：
```rust
UserCommand::SessionInference { command, session_id, model_path } => {
    // get() 自动优先 user 目录，找不到回退 builtin
    let Some(entry) = self.program_registry.get(&command).cloned() else {
        tracing::error!("ML Thread 脚本未找到: {}", command);
        ...
    };
    spawn_lua_script(entry.path, params, self.capabilities.clone(), ...);
}
```

##### 3d. `Src/TUI/mod.rs` — 命令解析

```
session inference <command> <session_id> <model_path>   // 3 参数
session inference <session_id> <model_path>              // 2 参数（command="single_inf"）
```

解析逻辑：
```rust
if args.len() == 3 {
    command = args[0], session_id = args[1], model_path = args[2]
} else if args.len() == 2 {
    command = "single_inf", session_id = args[0], model_path = args[1]
}
```

#### 涉及文件

| 文件 | 改动点 |
|------|--------|
| `programs/builtin/inference.lua` | 移动+重命名为 `programs/user/single_inf.lua` |
| `Src/Orchestrator/command.rs` | `SessionInference` 加 `command` 字段 |
| `Src/Orchestrator/core/branch_user.rs` | `get("inference")` → `get(&command)` |
| `Src/TUI/mod.rs` | 3 参数新格式 + 2 参数兼容 |

#### 验证方式

1. 移动+重命名文件后 `reload`
2. `session create test` → `session inference 1 test.pgguf`（2 参数，默认 "single_inf"）正常工作
3. `session inference single_inf 1 test.pgguf`（3 参数，显式指定）正常工作
4. 复制 `user/single_inf.lua` → `user/my_ml.lua`，修改如 `caps.print("custom ML thread")`
5. `session inference my_ml 1 test.pgguf` → 日志输出 `custom ML thread`

---

### 11.4 用户 split.lua 脚本 — PGGUF 均分切分

#### 背景

`ml.split_model` 已暴露给 Lua，但缺少一个用户友好的脚本来自动计算均分切分范围。需要创建 `programs/user/split.lua`，用法：

```
exec split xxx.pgguf num
```

将 `xxx.pgguf` 按层均分为 `num` 份，第一份保留 tokenizer 数据（供 encode/decode 节点使用）。

#### 层编号约定

```
Layer 0             = Embedding (token_embd.weight)
Layer 1 .. N        = Transformer blocks (blk.0.* .. blk.(N-1).*)
Layer N+1           = LM Head (output_norm.weight + output.weight)
```

总层数 = N + 2，其中 N = `block_count`（GGUF metadata 中的 `{arch}.block_count`）。

#### 均分算法

```
total_layers = N + 2
base = total_layers / num       // 每份基础层数
remainder = total_layers % num  // 前 remainder 份各多 1 层

for i in 0..num:
    start = sum of previous group sizes
    end = start + base + (if i < remainder { 1 } else { 0 }) - 1
    ml.split_model(path, start, end, dir, keep_tokenizer=(i==0))
```

例：`N=28` → `total=30`，`num=3`：
- Part 0: layers 0–9 (embedding + blk.0–7 + 1 extra)  ← tokenizer
- Part 1: layers 10–19 (blk.8–17)
- Part 2: layers 20–29 (blk.18–27 + LM head)

#### Rust 层改动：`keep_tokenizer` 参数

当前 `GGUF_Split_Model` 无条件跳过 `tokenizer.*` 和 `chat_template`。需要增加 `keep_tokenizer: bool` 参数：

##### 4a. `Src/ML_Engine/gguf_model_manager.rs` — `GGUF_Split_Model`

签名改为：
```rust
pub fn GGUF_Split_Model(
    gguf_file_path: &Path,
    split_start: usize,
    split_end: usize,
    output_gguf_file_path: &Path,
    keep_tokenizer: bool,
) -> Result<()> {
```

metadata 复制逻辑：
```rust
let mut skip_keys: Vec<&str> = vec!["pleiades.layer_bitmap"];
if !keep_tokenizer {
    skip_keys.push("tokenizer.");
    skip_keys.push("chat_template");
}
```

##### 4b. `Src/ML_Engine/capability.rs` — `split_model`

新增参数透传：
```rust
pub async fn split_model(
    gguf_file_path: &Path,
    split_start: usize,
    split_end: usize,
    output_dir: &Path,
    keep_tokenizer: bool,
) -> Result<(), String> {
```

##### 4c. `Src/VM/capability_binding.rs` — Lua 绑定

```rust
ml.set(
    "split_model",
    lua.create_async_function(move |_, (path, start, end, output_dir, keep_tokenizer):
        (String, usize, usize, String, bool)| async move {
        capability::split_model(
            std::path::Path::new(&path), start, end,
            std::path::Path::new(&output_dir), keep_tokenizer,
        ).await.map_err(|e| mlua::Error::runtime(e))
    })?,
)?;
```

##### 4d. `programs/user/split.lua` — 新建脚本

```lua
-- COMMAND: split
-- DESCRIPTION: 将 PGGUF 文件按层均分为 num 份

function execute(params)
    local path = params.path
    local num = tonumber(params.num)

    -- 1. 读取模型架构信息
    local info = ml.analyze_model(path)
    local total_layers = info.num_layers + 2  -- N + embedding + LM head

    -- 2. 计算均分范围
    local base = math.floor(total_layers / num)
    local remainder = total_layers % num

    local start = 0
    for i = 0, num - 1 do
        local group_size = base
        if i < remainder then group_size = group_size + 1 end
        local end_idx = start + group_size - 1

        local keep_tok = (i == 0)
        caps.print(string.format("split: part %d/%d layers %d-%d (tokenizer=%s)",
            i + 1, num, start, end_idx, tostring(keep_tok)))

        ml.split_model(path, start, end_idx, ".", keep_tok)

        start = end_idx + 1
    end

    caps.print("split: 完成")
end
```

#### 涉及文件

| 文件 | 改动点 |
|------|--------|
| `Src/ML_Engine/gguf_model_manager.rs` | `GGUF_Split_Model` 加 `keep_tokenizer: bool` 参数 |
| `Src/ML_Engine/capability.rs` | `split_model` 透传 `keep_tokenizer` |
| `Src/VM/capability_binding.rs` | Lua `ml.split_model` 加第5参数 |
| `programs/user/split.lua` | **新建** — 均分逻辑脚本 |

#### 验证方式

1. `exec split test.pgguf 3`
2. 确认输出 3 个文件：`test_split_0_9.pgguf`、`test_split_10_19.pgguf`、`test_split_20_29.pgguf`
3. 对 Part 0 执行 `ml.analyze_model` → 确认 tokenizer 信息存在
4. 对 Part 1/2 执行 `ml.analyze_model` → 确认无 tokenizer、`layer_bitmap` 正确

---

### 11.5 分布式流水线推理 pipe_1/pipe_2

#### 背景

通过 11.4 的 split 将模型均分为两半后，分别放在两个节点上：
- 节点 A: `Qwen14B_split_0_20.pgguf` (embedding + blk.0–19)
- 节点 B: `Qwen14B_split_21_41.pgguf` (blk.20–39 + LM head)

需实现 pipe_1/pipe_2 两个 Lua 脚本，自动编排为完整的 session 推理流水线。

#### 参考实现

`programs/user/pipeline1.lua` / `pipeline2.lua` — 基于 `caps.network` 的 `open_tensor_stream`/`accept_tensor_stream` 实现的双向张量流。

#### 数据流设计

```
节点 A (pipe_1)                                 节点 B (pipe_2)
                                                              
Session ←─local_tensor─→ pipe_1 ←─network tensor stream─→ pipe_2
 (encode/decode/sample)   (layers 0-20)                    (layers 21-41)
                                                              
1. Session encode → send tensor to pipe_1
2. pipe_1: forward(0..20, hidden) → send hidden to pipe_2
3. pipe_2: forward(21..41, hidden) → send logits to pipe_1
4. pipe_1: relay logits to Session
5. Session: sample → decode → output token
6. Session: tensorize token → send to pipe_1 → GOTO 2
```

#### 启动流程

```
节点 B:
  (pipe_2 由 pipe_1 通过 rexec 自动启动，无需手动操作)

节点 A:
  1. session create Qwen14B_split_0_20
  2. session inference pipe_1 1 Qwen14B_split_0_20.pgguf
     → pipe_1 加载前半模型
     → 通过 caps.network.get_remote_peer() 发现节点 B
     → rexec 远程执行 "EXEC|pipe_2|{"model":"Qwen14B_split_21_41.pgguf"}"
     → 建立双向 tensor stream
     → 连接 Session (local_tensor)
     → 桥接循环开始
```

> **约定**: 网络中只有两个节点，pipe_1 取本地 peer_id 之外的第一个就是远程节点。

#### 待修改内容

##### 5a. `Src/VM/capability_binding.rs` — 暴露 get_all_peers 给 Lua

新增 `caps.network.get_all_peers()`，返回 `{ {name, peer_id}, ... }`：

```rust
network.set("get_all_peers", lua.create_async_function(move |lua, (): ()| {
    let caps = capabilities.clone();
    async move {
        let peers = caps.peer_manager.Get_All_Peers().await
            .map_err(|e| mlua::Error::runtime(format!("get_all_peers: {}", e)))?;
        let result = lua.create_table()?;
        for (i, p) in peers.iter().enumerate() {
            let entry = lua.create_table()?;
            entry.set("name", p.name.clone())?;
            entry.set("peer_id", p.peer_id.to_base58())?;
            result.set(i + 1, entry)?;
        }
        Ok(result)
    }
})?)?;
```

##### 5b. `programs/user/pipe_1.lua` — 新建

```lua
-- COMMAND: pipe_1
-- DESCRIPTION: 分布式流水线前半段，桥接 Session ↔ pipe_2

-- 辅助函数：获取第一个远程节点 peer_id
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("没有找到远程节点")
end

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    -- 1. 加载前半模型
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    local sess = ml.new("cuda")
    sess:load_model(full_path, 0, info.num_layers + 1)  -- embedding + blocks
    handle:release()

    -- 2. 发现远程节点（网络中只有两个节点，取第一个非本地）
    local peer_id = find_remote_peer()
    caps.print("pipe_1: 远程节点 " .. peer_id)

    -- 3. 通过 rexec 启动远端 pipe_2
    local params_json = '{"model":"Qwen14B_split_21_41.pgguf"}'
    caps.network.send_data(peer_id, "Command", "EXEC|pipe_2|" .. params_json)

    -- 4. 连接 Session
    local session_stream = local_tensor.open_stream("ml-" .. session_id)

    -- 5. 建立双向 tensor stream (inference_id = session_id)
    local fwd = caps.network.open_tensor_stream(peer_id, tonumber(session_id))
    local bwd = caps.network.accept_tensor_stream(tonumber(session_id), 120)

    -- 6. 桥接循环
    while true do
        -- 从 Session 收 tensor（prefill 或单 token）→ forward 前半
        local tensor, offset = local_tensor.recv_tensor(session_stream, "cuda")
        if offset == 0 then sess:reset_kv_cache() end
        local hidden = sess:forward(tensor, offset)

        -- 发送 hidden 到 pipe_2
        caps.network.send_tensor(fwd, hidden, offset)

        -- 接收 logits 回传 Session
        local logits, _ = caps.network.recv_tensor(bwd, "cuda")
        local_tensor.send_tensor(session_stream, logits, offset)
    end
end
```

##### 5c. `programs/user/pipe_2.lua` — 新建

```lua
-- COMMAND: pipe_2
-- DESCRIPTION: 分布式流水线后半段，接收 hidden 返回 logits

function execute(params)
    local model_path = params.model

    -- 1. 加载后半模型
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    local sess = ml.new("cuda")
    sess:load_model(full_path, 0, info.num_layers + 1)  -- blocks + LM head
    handle:release()

    -- 2. 发现 pipe_1 节点
    local peer_id = find_remote_peer()
    caps.print("pipe_2: 连接 pipe_1 (" .. peer_id .. ")")

    -- 3. 建立双向 tensor stream
    local fwd = caps.network.accept_tensor_stream(tonumber(params.inference_id), 120)
    local bwd = caps.network.open_tensor_stream(peer_id, tonumber(params.inference_id))

    -- 4. 循环: 收 hidden → forward → 发 logits
    while true do
        local hidden, offset = caps.network.recv_tensor(fwd, "cuda")
        local logits = sess:forward(hidden, offset)
        caps.network.send_tensor(bwd, logits, sess:get_offset())
    end
end
```

##### 5d. `Src/VM/capability_binding.rs` — 暴露 get_peers 给 Lua（可选）

当前 Lua 无法获取节点列表。需要新增：
```lua
caps.network.get_peers() → { {name, peer_id}, ... }
```

或简化：pipe_1 通过 `exec` 参数传入 `peer`，在 B1 的 `SessionInference` 分支中解析 peer_name → peer_id（复用 `Execute` 分支的解析逻辑），然后 `spawn_lua_script` 时传入 `peer`=peer_id。

> **推荐**：在 `branch_user.rs` 的 `SessionInference` 分支中，额外参数中的 `peer=` 自动解析为 peer_id，传入 Lua params。

#### 涉及文件

| 文件 | 改动点 |
|------|--------|
| `Src/VM/capability_binding.rs` | 新增 `caps.network.get_all_peers()` → `{ {name, peer_id}, ... }` |
| `programs/user/pipe_1.lua` | **新建** — 前半段桥接脚本 |
| `programs/user/pipe_2.lua` | **新建** — 后半段 forward 脚本 |

#### 验证方式

1. 节点 A、B 分别 `flush` 确认各自拥有半边模型
2. 节点 B 处于空闲状态（等待 rexec 指令）
3. 节点 A: `session create Qwen14B_split_0_20` → `session inference pipe_1 1 Qwen14B_split_0_20.pgguf`
4. 确认 pipe_2 在 B 上自动启动（日志可见）
5. 通过 API 发 chat 请求 → 确认完整推理链路正常工作

---

### 11.6 CPU/GPU 混合流水线 pipe_3/pipe_4

#### 背景

pipe_1/pipe_2 将每个分片模型全部放在 GPU 上。当 GPU 显存不足以容纳整个分片时，需要将分片内部再拆分为 CPU + GPU 两部分混合加载。

参考 `programs/user/cpu_gpu_run.lua` 的模式：单个模型创建两个 `MlSession`（CPU + GPU），CPU 算前半 → `tensor:to_device("cuda")` 传输 → GPU 算后半。

#### 与 pipe_1/pipe_2 的关系

```
pipe_1/pipe_2:              pipe_3/pipe_4:

节点A: 分片全在GPU           节点A: 分片 split → CPU一半 + GPU一半
节点B: 分片全在GPU           节点B: 分片 split → CPU一半 + GPU一半
```

每个节点内部使用 `tensor:to_device()` 在 CPU↔GPU 之间搬运 tensor，两节点之间仍通过网络 tensor stream 通信。

#### 分片内均分

pipe_3 加载 `Qwen14B_split_0_20.pgguf`（layers 0-20，共 21 层）：
- CPU: `split_start .. mid` = 0..10（11 层）
- GPU: `mid+1 .. split_end` = 11..20（10 层）

pipe_4 加载 `Qwen14B_split_21_41.pgguf`（layers 21-41，共 21 层）：
- CPU: `split_start .. mid` = 21..31（11 层）
- GPU: `mid+1 .. split_end` = 32..41（10 层）

其中 `mid = split_start + floor((split_end - split_start) / 2)`。

#### 数据流

```
Session
  ↕ local_tensor "ml-{sid}"
pipe_3 (节点A)
  cpu_sess:forward(0..10, hidden) → hidden_cpu
  hidden_cpu:to_device("cuda") → hidden_gpu
  gpu_sess:forward(11..20, hidden_gpu) → hidden_out
  caps.network.send_tensor(fwd, hidden_out)   → 发往 pipe_4
  caps.network.recv_tensor(bwd) → logits      ← 接收 logits
  local_tensor.send_tensor(session_stream, logits)
─────────────────────────────────────────────────────
pipe_4 (节点B)
  caps.network.recv_tensor(fwd) → hidden
  cpu_sess:forward(21..31, hidden) → hidden_cpu
  hidden_cpu:to_device("cuda") → hidden_gpu
  gpu_sess:forward(32..41, hidden_gpu) → logits
  caps.network.send_tensor(bwd, logits)
```

#### 待修改内容

##### 6a. `programs/user/pipe_3.lua` — 新建

与 pipe_1 相同的前半段逻辑，区别：
- 创建两个 MlSession：`cpu_sess` + `gpu_sess`
- 按 `mid` 拆分当前分片的层范围
- 每轮：CPU forward → `to_device("cuda")` → GPU forward → 网络发送

```lua
-- 1. 加载模型 → info.split_start / info.split_end
-- 2. 计算 mid
local mid = info.split_start + math.floor((info.split_end - info.split_start) / 2)
-- 3. 创建 CPU + GPU session
cpu_sess:load_model(full_path, info.split_start, mid)
gpu_sess:load_model(full_path, mid + 1, info.split_end)
-- 4. 桥接循环:
--    recv → cpu_sess:forward → to_device("cuda") → gpu_sess:forward → send
--    recv ← ... ← bwd ← send
```

##### 6b. `programs/user/pipe_4.lua` — 新建

与 pipe_2 相同的后半段逻辑，区别：
- 同样创建 `cpu_sess` + `gpu_sess`，按 mid 拆分
- 每轮：网络接收 → CPU forward → `to_device("cuda")` → GPU forward → 网络发回

#### 远程模型名推导

复用 pipe_1 的 stem 提取逻辑（pipe_3 和 pipe_1 用同一组 split 产物，推导方式不变）。

#### 涉及文件

| 文件 | 改动点 |
|------|--------|
| `programs/user/pipe_3.lua` | **新建** — CPU/GPU 混合前半段 |
| `programs/user/pipe_4.lua` | **新建** — CPU/GPU 混合后半段 |

#### 验证方式

1. 节点 A、B 各放半边模型
2. `session inference pipe_3 1 Qwen14B_split_0_20.pgguf`
3. 确认 pipe_4 在 B 上自动启动
4. 确认两个节点各自创建了 CPU + GPU 两个 MlSession
5. API 请求 → 推理结果正确

---

## 备注

- 基分支: `reforge`
- 工作分支: 直接在 `reforge` 上完成
- 后续子任务: 待定
- 额外修复: TUI Network 面板 peer 名称显示为 `name#XXXX` 完整格式 (原只显示裸 name)
