# Task 6.4: 系统集成

> Presented by KeJi
> Date: 2026-05-22

---

## 目标

将 Session Manager 主循环接入 `main.rs` 启动流程，注入 `Capabilities`，打通 Network / Core 的调用路径。

---

## 改动清单

### 1. SessionManager::run() — 主循环

```rust
impl SessionManager {
    pub async fn run(mut self) {
        let mut flush_timer = tokio::time::interval(FLUSH_INTERVAL);

        loop {
            tokio::select! {
                // A — 本地 open_slot（Core / TUI / Lua）
                Some((sess_id, prompt_rx, token_tx, reply_tx)) = self.open_slot_rx.recv() => {
                    let result = self.allocate_slot(&sess_id, prompt_rx, token_tx);
                    reply_tx.send(result).ok();  // 通过 oneshot 返回结果
                }

                // B — 收 Prompt
                Some((slot_id, text)) = self.shared_prompt_rx.recv() => {
                    self.handle_prompt(slot_id, text);
                }

                // C — Flush Batch
                _ = flush_timer.tick() => {
                    if let Some(batch) = self.assemble_batch() {
                        self.batch_tx.send(batch).await.ok();
                    }
                }

                // D — 收 Logits + Sample + 分发
                Some(result) = self.logits_rx.recv() => {
                    self.dispatch_logits(result).await;
                }

                // E — 网络 open_slot
                Some((sess_id, stream, reply_tx)) = self.network_slot_rx.recv() => {
                    let result = self.allocate_slot(&sess_id, /* ... */);
                    match &result {
                        Ok(handle) => { spawn_bridge(stream, handle); }
                        Err(_)     => { /* write FULL, close stream */ }
                    }
                    reply_tx.send(result).ok();
                }

                // F — 销毁 Slot
                Some((sess_id, slot_id)) = self.close_slot_rx.recv() => {
                    self.close_slot(&sess_id, slot_id);
                }
            }
        }
    }
}
```

### 2. main.rs — spawn Session Manager

```rust
// Phase 3 新增：
let session_manager = SessionManager::new(config.session.max_slots);
let session_mgr_arc = Arc::new(session_manager);

// Phase 4/5 中间：
tokio::spawn({
    let mgr = session_mgr_arc.clone();
    async move { mgr.run().await }
});
```

### 3. Capabilities 新增字段

```rust
// Src/Orchestrator/mod.rs
pub struct Capabilities {
    pub storage: Arc<dyn StorageCapability>,
    pub network: Box<dyn Network_Capability>,
    pub peer_manager: Box<dyn PeerHandle>,
    pub event_bus: Arc<EventBus>,
    pub local_stream_hub: Arc<LocalStreamHub>,
    pub session_manager: Arc<SessionManager>,  // ← 新增
}
```

### 4. Core — 网络 SessionStreamArrived 路由

```rust
// Core route_stream() 新增分支：
Network_Inbound_Event::SessionStreamArrived { peer, stream, session_id } => {
    let caps = self.capabilities.clone();
    tokio::spawn(async move {
        caps.session_manager.open_slot_network(session_id, stream).await;
    });
}
```

### 5. 网络桥接（spawn_bridge）

```rust
fn spawn_bridge(stream: libp2p::Stream, handle: SlotHandle) {
    let (mut reader, mut writer) = stream.split();

    // 方向 1: stream → prompt
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let n = reader.read(&mut buf).await?;
            let text = String::from_utf8_lossy(&buf[..n]);
            handle.submit(text.to_string());
        }
    });

    // 方向 2: token → stream
    tokio::spawn(async move {
        while let Some(token) = handle.recv_token().await {
            writer.write_all(token.as_bytes()).await.ok();
        }
    });
}
```

---

## 启动顺序

```
main.rs:

Phase 3: SessionManager::new()
Phase 4: Network 初始化
Phase 5: Capabilities 组装（含 session_manager）
         tokio::spawn(session_manager.run())    ← 先跑起来
         tokio::spawn(network_service)
Phase 6: tokio::spawn(core.run())
         tokio::spawn_blocking(TUI)
```

Session Manager 的 loop 必须先启动，这样 Core/Network 发来的 `open_slot` 请求才能被接收。

---

## 配置

```toml
# config.toml 新增
[Session]
max_slots = 4            # 每个 Session 最大槽位数
flush_interval_ms = 100  # flush 间隔
```

---

## 人类评审

<!-- 在此区域写下评审意见 -->

