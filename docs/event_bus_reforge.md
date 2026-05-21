# EventBus Reforge 实施计划

## 目标

将 `Bus_Event` 从 14 个领域耦合的 flat variant 重构为 4 个通用类型（与具体模块/面板解耦），payload 采用 JSON 字符串。

## 最终形态

```rust
pub enum Bus_Event {
    Notify { level: NotifyLevel, message: String },
    State  { payload: String },
    Stream { payload: String },
    Output { payload: String },
}

pub enum NotifyLevel { Info, Warn, Error }
```

## 改动文件清单 (8)

| # | 文件 | 角色 | 改动 |
|---|------|------|------|
| 1 | `Src/EventBus/event.rs` | 核心定义 | Bus_Event 14→4 variant + 新增 NotifyLevel |
| 2 | `Src/EventBus/mod.rs` | 导出 | 导出 NotifyLevel |
| 3 | `Src/Orchestrator/core/branch_user.rs` | 生产者 | 13 处 Publish 改写 |
| 4 | `Src/Orchestrator/core/branch_command.rs` | 生产者 | 2 处 Publish 改写 |
| 5 | `Src/Orchestrator/core/branch_lifecycle.rs` | 生产者 | 1 处 Publish 改写 |
| 6 | `Src/Orchestrator/core/branch_stream.rs` | 生产者 | 1 处 Publish 改写 |
| 7 | `Src/Network/swarm_events.rs` | 生产者 | 4 处 Publish 改写 |
| 8 | `Src/VM/capability_binding.rs` | 生产者 | 1 处 Publish 改写 |
| 9 | `Src/TUI/mod.rs` | 消费者 | Handle_Bus_Event 14分支→4分支+JSON解析 |
| 10 | `tests/t03_event_bus_integration.rs` | 测试 | 全部 event 构造改写 |
| 11 | `tests/t09_lua_integration.rs` | 测试 | event 构造改写 (如有) |

## 生产者映射表

### Notify (原 Log / Error / Device_Changed → NotifyLevel)

| 当前 | 新写法 |
|------|--------|
| `Bus_Event::Log { message }` | `Bus_Event::Notify { level: NotifyLevel::Info, message }` |
| `Bus_Event::Error { message }` | `Bus_Event::Notify { level: NotifyLevel::Error, message }` |
| `Bus_Event::Device_Changed { device }` | `Bus_Event::Notify { level: NotifyLevel::Warn, message: format!("设备已切换: {}", device.to_uppercase()) }` |

### State (原 Peer + Job 生命周期)

| 当前 | JSON payload |
|------|-------------|
| `Peer_Discovered { peer_id }` | `{"type":"peer_discovered","peer_id":"..."}` |
| `Peer_Left { peer_id }` | `{"type":"peer_left","peer_id":"..."}` |
| `Connection_Established { peer_id }` | `{"type":"peer_connected","peer_id":"..."}` |
| `Connection_Closed { peer_id }` | `{"type":"peer_disconnected","peer_id":"..."}` |
| `Job_Completed { job_id, result }` | `{"type":"job_completed","job_id":1,"result":"Success"}` |
| `Job_Created { job_id, kind, model_name }` | `{"type":"job_created","job_id":1,"kind":"Run","model":"..."}` |
| `Job_State_Changed { job_id, phase }` | `{"type":"job_phase_changed","job_id":1,"phase":"Executing"}` |
| `Inference_Started { ... }` | `{"type":"inference_started","model":"...","devices":1,"layers":"0-15"}` |
| `Inference_Completed { ... }` | `{"type":"inference_completed","text":"...","tokens":42}` |

### Stream (原 Inference_Token + File_Progress)

| 当前 | JSON payload |
|------|-------------|
| `Inference_Token { job_id, token }` | `{"type":"token","text":"Hello","count":1}` |
| `File_Progress { file_name, direction, peer, sent, total }` | `{"type":"file_progress","name":"x.gguf","dir":"send","peer":"...","sent":1024,"total":4096}` |

### Output (原 CommandResult + HelpInfo)

| 当前 | JSON payload |
|------|-------------|
| `CommandResult { text, completed }` | `{"type":"cmd_result","text":"...","completed":true}` |
| `HelpInfo { builtin, user }` | `{"type":"help","text":"[内置命令]\n  ..."}` |

## TUI 消费者重写

```rust
fn Handle_Bus_Event(app: &mut App, event: Bus_Event) {
    match event {
        Bus_Event::Notify { level, message } => {
            let msg = match level {
                NotifyLevel::Error => format!("[错误] {message}"),
                NotifyLevel::Warn  => format!("[警告] {message}"),
                NotifyLevel::Info  => message,
            };
            app.Add_Log(msg);
        }
        Bus_Event::State { payload } => {
            let v: serde_json::Value = parse_or_log(app, &payload);
            match v["type"].as_str() {
                Some("peer_discovered")    => handle_peer_discovered(app, &v),
                Some("peer_left")          => handle_peer_left(app, &v),
                Some("peer_connected")     => handle_peer_connected(app, &v),
                Some("peer_disconnected")  => handle_peer_disconnected(app, &v),
                Some("job_created")        => handle_job_created(app, &v),
                Some("job_phase_changed")  => handle_job_phase_changed(app, &v),
                Some("job_completed")      => handle_job_completed(app, &v),
                Some("inference_started")  => handle_inference_started(app, &v),
                Some("inference_completed")=> handle_inference_completed(app, &v),
                _ => {}
            }
        }
        Bus_Event::Stream { payload } => {
            let v: serde_json::Value = parse_or_log(app, &payload);
            match v["type"].as_str() {
                Some("token")         => handle_token_stream(app, &v),
                Some("file_progress") => handle_file_stream(app, &v),
                _ => {}
            }
        }
        Bus_Event::Output { payload } => {
            let v: serde_json::Value = parse_or_log(app, &payload);
            match v["type"].as_str() {
                Some("cmd_result") => {
                    app.command_output.output_text = v["text"].as_str().unwrap_or("").to_string();
                    app.command_output.completed = v["completed"].as_bool().unwrap_or(true);
                    app.command_scroll = 0;
                }
                Some("help") => {
                    app.command_output.output_text = v["text"].as_str().unwrap_or("").to_string();
                    app.command_output.completed = true;
                }
                _ => {}
            }
        }
    }
}
```

## 新增依赖

`serde_json` 不在 `Cargo.toml` 中，需要添加：

```toml
serde_json = "1"
```

生产者（构造 JSON payload）和消费者（解析 JSON payload）都需要。

## 受影响文件完整清单 (11)

| # | 文件 | 角色 | 改动 |
|---|------|------|------|
| 1 | `Cargo.toml` | 依赖 | 新增 `serde_json` |
| 2 | `Src/EventBus/event.rs` | 核心定义 | Bus_Event 14→4 variant + 新增 NotifyLevel + 删除 HelpEntry |
| 3 | `Src/EventBus/mod.rs` | 导出 | 导出 NotifyLevel |
| 4 | `Src/Orchestrator/core/branch_user.rs` | 生产者 | 13 处 Publish 改写，HelpEntry 私有化或内联 |
| 5 | `Src/Orchestrator/core/branch_command.rs` | 生产者 | 2 处 Publish 改写 |
| 6 | `Src/Orchestrator/core/branch_lifecycle.rs` | 生产者 | 1 处 Publish 改写 |
| 7 | `Src/Orchestrator/core/branch_stream.rs` | 生产者 | 1 处 Publish 改写 |
| 8 | `Src/Network/swarm_events.rs` | 生产者 | 4 处 Publish 改写 |
| 9 | `Src/VM/capability_binding.rs` | 生产者 | 1 处 Publish 改写 |
| 10 | `Src/TUI/mod.rs` | 消费者 | Handle_Bus_Event 14分支→4分支+JSON解析 |
| 11 | `tests/t03_event_bus_integration.rs` | 测试 | 全部 event 构造改写 |

## 实施顺序

1. 修改 `event.rs` + `mod.rs` (核心定义)
2. 修改 `TUI/mod.rs` (消费者 — 先改消费者让编译能通过)
3. 逐个修改生产者 (branch_user → branch_command → branch_lifecycle → branch_stream → swarm_events → capability_binding)
4. 修改测试
5. 编译验证 `cargo check`
6. 运行测试 `cargo test`
