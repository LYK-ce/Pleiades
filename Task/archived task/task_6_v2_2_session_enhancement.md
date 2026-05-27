# Task 6 v2.2: Session 功能完善 — 集成 tokenizer

> Presented by KeJi
> Date: 2026-05-25

---

## 背景

Task 6 v2.1 已完成 MlContext 重构，`model` 和 `tokenizer` 已解耦。现在可以将 tokenizer 集成到 Session 中：Session 自己的 spawn task 里加载 tokenizer，收到 chat prompt 后调用 `encode()` 进行编码验证。

当前 Session.spawn() 行为：

```
chat prompt → recv_frame → EventBus 打印 → 结束（one-shot）
```

目标行为：

```
chat prompt → recv_frame → EventBus 打印 → encode() → 打印 token count → 结束
```

---

## 设计原则

**I/O 在 Session 自己的 task 里做，不污染 Core 主循环。** SessionManager 保持纯数据结构（`Arc<Mutex<HashMap<...>>>`），`create_session()` 保持同步，锁只保护 HashMap 插入。

```
branch_user.rs (tokio::spawn)          Session::spawn() 的 task 内部
─────────────────────────────          ────────────────────────────
session_mgr.create_session()            storage.acquire_read()     ← 异步 I/O
  → HashMap::insert (瞬时)              load_tokenizer()           ← 同步
  → session.spawn(storage, ...)         accept_async()             ← 等 TUI 连接
  → 立即返回                            recv_frame → encode()
                                        EventBus 发布结果
```

---

## 改动清单

### 1. `Session::spawn()` — 加 storage 参数，内部加载 tokenizer + encode

```diff
  pub fn spawn(
      &self,
      stream_hub: Arc<LocalStreamHub>,
      event_bus: Arc<EventBus>,
+     storage: Arc<dyn StorageCapability>,
  ) {
      let session_id = self.session_id;
+     let model_path = self.model_id.clone();   // "test.pgguf"
      let stream_id = format!("session-{}", session_id);

      tokio::spawn(async move {
+         // ── 1. 加载 tokenizer ──────────────────────────
+         let (path, _guard) = match storage.acquire_read(&model_path).await {
+             Ok(p) => p,
+             Err(e) => {
+                 event_bus.Publish(Bus_Event::Notify {
+                     level: NotifyLevel::Error,
+                     message: format!("Session {} storage error: {}", session_id, e),
+                 });
+                 return;
+             }
+         };
+
+         let mut ml = match MlSession::new("cpu") {
+             Ok(s) => s,
+             Err(e) => {
+                 event_bus.Publish(Bus_Event::Notify {
+                     level: NotifyLevel::Error,
+                     message: format!("Session {} ml init error: {}", session_id, e),
+                 });
+                 return;
+             }
+         };
+
+         if let Err(e) = ml.load_tokenizer(&path) {
+             event_bus.Publish(Bus_Event::Notify {
+                 level: NotifyLevel::Error,
+                 message: format!("Session {} tokenizer error: {}", session_id, e),
+             });
+             return;
+         }
+         drop(_guard);  // 释放 Storage 读锁
+
+         // ── 2. 等 TUI 连接 ──────────────────────────────
          let mut stream = match stream_hub.accept_async(&stream_id, 30_000).await {
              Ok(s) => s,
              Err(e) => {
                  tracing::warn!("Session {} accept error: {}", session_id, e);
                  return;
              }
          };

+         // ── 3. 收 prompt → encode → 发布 ───────────────
          let mut buf = Tensor_Buffer::New(4096);
          match local_recv_frame(&mut stream, &mut buf).await {
              Ok(_offset) => {
                  let text = String::from_utf8_lossy(buf.As_Slice());
-                 event_bus.Publish(Bus_Event::Notify {
-                     level: NotifyLevel::Info,
-                     message: format!("Session {}: {}", session_id, text),
-                 });
+                 match ml.encode(&text) {
+                     Ok(token_ids) => {
+                         event_bus.Publish(Bus_Event::Notify {
+                             level: NotifyLevel::Info,
+                             message: format!("Session {}: {} ({} tokens)",
+                                 session_id, text, token_ids.len()),
+                         });
+                     }
+                     Err(e) => {
+                         event_bus.Publish(Bus_Event::Notify {
+                             level: NotifyLevel::Error,
+                             message: format!("Session {} encode failed: {}", session_id, e),
+                         });
+                     }
+                 }
              }
              Err(e) => {
                  tracing::warn!("Session {} recv error: {}", session_id, e);
              }
          }
      });
  }
```

### 2. `SessionManager` — 加 storage 字段（仅存储，不涉及 I/O）

```diff
  pub struct SessionManager {
      sessions: HashMap<u64, Session>,
      pub stream_hub: Arc<LocalStreamHub>,
      pub event_bus: Arc<EventBus>,
+     storage: Arc<dyn StorageCapability>,
      max_slots: usize,
      counter: u64,
  }

- pub fn new(max_slots, stream_hub, event_bus) -> Arc<Mutex<Self>>
+ pub fn new(max_slots, stream_hub, event_bus, storage) -> Arc<Mutex<Self>>

  // create_session 内部
- session.spawn(self.stream_hub.clone(), self.event_bus.clone());
+ session.spawn(self.stream_hub.clone(), self.event_bus.clone(), self.storage.clone());
```

`create_session()` 保持同步不变，`storage.clone()` 是 Arc 引用计数加一，零开销。

### 3. `Core::new()` — 传 storage

```diff
  let session_mgr = crate::session::SessionManager::new(
      4,
      capabilities.local_stream_hub.clone(),
      capabilities.event_bus.clone(),
+     capabilities.storage.clone(),
  );
```

### 4. 其他文件

**不需改动**：`branch_user.rs`、`Session` 结构体。

---

## 改动面总结

| 文件 | 改动 |
|------|------|
| `session.rs` | `spawn()` 加 `storage` 参数；task 内加载 tokenizer + encode |
| `manager.rs` | 加 `storage` 字段；`new()` 加参数；`create_session` 传 `storage` 给 `spawn` |
| `core.rs` | `SessionManager::new()` 加 `storage` 参数 |
| `branch_user.rs` | **不变** |
| `Session` 结构体 | **不变** |

---

## 实施步骤

| 步骤 | 内容 | 文件 |
|:--:|------|------|
| 1 | 新建分支 `session-tokenizer` | — |
| 2 | `Session::spawn()` 加 storage 参数 + tokenizer 加载 + encode | `session.rs` |
| 3 | `SessionManager` 加 `storage` 字段，`new()` 加参数 | `manager.rs` |
| 4 | `Core::new()` 传 storage | `core.rs` |
| 5 | 更新测试（`make_mgr()` 加 storage stub） | `manager.rs`、`mod.rs` |
| 6 | `cargo test --lib` 全量通过 | — |
| 7 | 合并回 `session-manager-reforge` | — |

---

## 人类评审

<!-- 在此区域写下评审意见 -->

