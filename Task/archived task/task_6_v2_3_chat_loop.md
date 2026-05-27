# Task 6 v2.3: Chat 持久连接 + Session select! loop

> Presented by KeJi
> Date: 2026-05-25

---

## 背景

当前 `chat <session_id> <prompt>` 是 one-shot：发一帧 prompt → Session 收一帧 → encode → 结束。需要改为持久连接模式：

- `chat <session_id>` 只建立连接，不带 prompt
- prompt 从 TUI 的 Prompt 输入框实时输入
- Session 做 `select!` loop，持续接收并编码

---

## 设计

### 整体数据流

```
TUI Prompt> 你好                 
  │                              
  │ prompt_tx.send("你好")      broadcast channel（TUI 初始化时创建）
  ▼                              
chat task                       
  │ rx.recv() → "你好"          
  │ local_send_frame(stream, 0, "你好")
  ▼                              
┌──────────────────────────┐     
│ Session.spawn() task     │     
│                          │     
│ loop {                   │     
│   select! {              │     
│     recv_frame(stream)   │ ← 来自 chat 的新 prompt
│       → encode → EventBus│     
│   }                      │     
│ }                        │     
└──────────────────────────┘     
```

### 通道设计（关键决策）

`prompt_tx`（`tokio::sync::broadcast`）在 **TUI 初始化时** 创建，一直存活。

```
App::new()
  prompt_tx = broadcast::channel(16).0

chat <session_id> 时：
  let rx = app.prompt_tx.subscribe();   // 拿新接收端
  // rx 随 chat task 移动

Prompt 框回车时：
  Handle_Prompt_Submit → app.prompt_tx.send(text)
  // 无订阅者时 send 不阻塞，消息自动丢弃
```

### Chat task 行为

```
chat 1 启动 →
  1. hub.open("session-1") → stream（连上 Session）
  2. rx = prompt_tx.subscribe()
  3. loop {
       prompt = rx.recv()           // 等 Prompt 框输入
       local_send_frame(stream, 0, prompt.as_bytes())
       // 也可以 recv 回应（后续 Phase）
     }
```

### Session.spawn() 行为

```
改前（one-shot）：
  accept_async → recv_frame → encode → 结束

改后（loop）：
  accept_async → loop { recv_frame → encode → EventBus }
```

---

## 改动清单

### 1. `Src/TUI/app.rs` — App 加 prompt_tx

```diff
+ use tokio::sync::broadcast;

  pub struct App {
+     pub prompt_tx: broadcast::Sender<String>,
      // ... 其他字段
  }

  impl App {
      pub fn new(...) -> Self {
+         let (prompt_tx, _) = broadcast::channel(16);
          App {
+             prompt_tx,
              // ...
          }
      }
  }
```

### 2. `Src/TUI/mod.rs` — Handle_Prompt_Submit 改为发送

```diff
  fn Handle_Prompt_Submit(app: &mut App) {
      let prompt = app.Take_Prompt();
      if prompt.is_empty() { return; }
-     app.Add_Log("[提示] 无活跃推理会话，请先执行 run <model_path>".to_string());
+     let _ = app.prompt_tx.send(prompt);
  }
```

### 3. `Src/TUI/mod.rs` — chat 命令解析改为不带 prompt

```diff
- if trimmed.starts_with("chat ") {
-     let args: Vec<&str> = trimmed.strip_prefix("chat ").unwrap_or("").splitn(2, ' ').collect();
-     if args.len() < 2 {
-         app.command_output.output_text =
-             "错误: 参数不足\n用法: chat <session_id> <prompt>".to_string();
-     } else {
-         let sid = args[0].parse::<u64>();
-         // ...
-         let cmd = UserCommand::Chat { session_id, prompt: args[1].to_string() };
+ if trimmed.starts_with("chat ") {
+     let sid_str = trimmed.strip_prefix("chat ").unwrap_or("").trim();
+     let sid = sid_str.parse::<u64>();
+     match sid {
+         Ok(session_id) => {
+             let cmd = UserCommand::Chat { session_id };
+             // ...
+         }
+         Err(_) => { /* 错误提示 */ }
+     }
  }
```

### 4. `Src/Orchestrator/command.rs` — UserCommand::Chat 去掉 prompt 字段

```diff
- Chat { session_id: u64, prompt: String },
+ Chat { session_id: u64 },
```

### 5. `Src/Orchestrator/core/branch_user.rs` — Chat handler 改为持久模式

```diff
  UserCommand::Chat { session_id, prompt } => {
-     let hub = self.capabilities.local_stream_hub.clone();
-     tokio::spawn(async move {
-         let stream_id = format!("session-{}", session_id);
-         match hub.open(&stream_id) {
-             Ok(mut stream) => {
-                 let data = prompt.as_bytes();
-                 local_send_frame(&mut stream, 0, data).await;
-             }
-             Err(e) => { ... }
-         }
-     });
+ UserCommand::Chat { session_id } => {
+     let hub = self.capabilities.local_stream_hub.clone();
+     let event_bus = self.capabilities.event_bus.clone();
+     let prompt_rx = self.prompt_tx.subscribe();   // 需要把 prompt_tx 传到 Core
+     tokio::spawn(async move {
+         let stream_id = format!("session-{}", session_id);
+         let mut stream = match hub.open(&stream_id) {
+             Ok(s) => s,
+             Err(e) => { ... return; }
+         };
+         let mut rx = prompt_rx;
+         loop {
+             match rx.recv().await {
+                 Ok(prompt) => {
+                     local_send_frame(&mut stream, 0, prompt.as_bytes()).await;
+                 }
+                 Err(_) => break,  // sender 关闭
+             }
+         }
+     });
  }
```

> **但**：Core 目前没有 `prompt_tx`。需要传入。或者：让 TUI 把 `prompt_tx` 传进 Core。

### 6. `Src/Orchestrator/core.rs` — Core 加 prompt_tx

```diff
  pub struct Core {
+     prompt_tx: broadcast::Sender<String>,
      // ...
  }

  pub fn new(
      // ...
+     prompt_tx: broadcast::Sender<String>,
  ) -> Self { ... }
```

### 7. `main.rs`（或 Core 构造处）— 传入 prompt_tx

App 和 Core 共享同一个 `prompt_tx`。构造时由调用方创建 channel 并分别传入。

### 8. `Src/Session_Manager/session.rs` — spawn() 改为 loop

```diff
  // 改前：one-shot
- match local_recv_frame(&mut stream, &mut buf).await {
-     Ok(_offset) => { encode → publish }
-     Err(e) => { ... }
- }

+ // 改后：loop
+ loop {
+     match local_recv_frame(&mut stream, &mut buf).await {
+         Ok(_offset) => { encode → publish }
+         Err(e) => { ... break; }
+     }
+ }
```

---

## 改动面总结

| 文件 | 改动 |
|------|------|
| `app.rs` | 加 `prompt_tx: broadcast::Sender<String>` |
| `TUI/mod.rs` | `Handle_Prompt_Submit` 改为 `prompt_tx.send()`；chat 命令解析简化为 `chat <id>` |
| `command.rs` | `Chat` 变体去 `prompt` 字段 |
| `core.rs` | 加 `prompt_tx`，`new()` 加参数 |
| `main.rs`（或构造处） | 创建 `broadcast::channel`，分别传给 App 和 Core |
| `branch_user.rs` | Chat handler：`subscribe()` → loop → `local_send_frame` |
| `session.rs` | `spawn()`：one-shot → `loop { recv → encode }` |

---

## 实施步骤

| 步骤 | 内容 | 文件 |
|:--:|------|------|
| 1 | 创建 `chat-loop` 分支 | — |
| 2 | App 加 `prompt_tx`，初始化 channel | `app.rs` |
| 3 | `Handle_Prompt_Submit` 改为 `prompt_tx.send()` | `TUI/mod.rs` |
| 4 | Chat 命令解析改为 `chat <id>`（不带 prompt） | `TUI/mod.rs` |
| 5 | `UserCommand::Chat` 去掉 `prompt` 字段 | `command.rs` |
| 6 | Core 加 `prompt_tx` 字段，`new()` 加参数 | `core.rs` |
| 7 | Core 构造处传入 `prompt_tx` | `main.rs` |
| 8 | Chat handler：`subscribe()` → loop → `local_send_frame` | `branch_user.rs` |
| 9 | `Session::spawn()`：one-shot → loop | `session.rs` |
| 10 | `cargo test --lib` 全量通过 | — |
| 11 | 合并回 `session-manager-reforge` | — |

---

## 人类评审

<!-- 在此区域写下评审意见 -->

