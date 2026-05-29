# Task 11: Branch and ML Refine

> Presented by KeJi
> Date: 2026-05-29

## 描述

分支管理与 ML 引擎精细化。

## 子任务总览

| # | 任务 | 涉及文件数 | 状态 |
|---|------|-----------|------|
| 11.1 | Split Model 适配 PGGUF 格式 | 1 | ⬜ |
| 11.2 | 恢复 Network Command Branch — 支持远程 exec Lua 脚本 | 4 | ⬜ |

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

## 备注

- 基分支: `reforge`
- 工作分支: `task11_branch_ml_refine`
- 后续子任务: 待定
