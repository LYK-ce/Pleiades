# Task 6.1: Network 改造 — Session Stream 协议

> Presented by KeJi
> Date: 2026-05-22

---

## 目标

在 Network 模块新增一条专用流 `/pleiades/session/1.0.0`，用于远程节点向本节点申请会话槽位并建立长连接文本通道。

---

## 改动清单

### 1. 新增 `Src/Network/Session_Stream/` 模块

```
Src/Network/Session_Stream/
├── mod.rs          ← 模块入口
└── protocol.rs     ← 协议定义 + 握手读写
```

**protocol.rs 内容：**
- `SESSION_STREAM_PROTOCOL: &str = "/pleiades/session/1.0.0"`
- `async fn read_handshake(stream) → session_id: String` — 读取 `[len: u8][id: bytes]`
- `async fn write_handshake(stream, session_id: &str)` — 写入握手帧

### 2. `Src/Network/mod.rs` — 注册模块

- [ ] `pub mod session_stream;`
- [ ] 公开导出 session_stream 协议常量

### 3. `Network_Service_Capability` 新增成分

- [ ] `session_stream_control: stream::Control` — 被动接受
- [ ] `session_open_control: stream::Control` — 主动发起

### 4. `Network_Capability` trait 新增方法

- [ ] `async fn open_session_stream(peer, session_id) → Result<Stream>` — 建流 + 握手

### 5. `Network_Inbound_Event` 新增变体

- [ ] `SessionStreamArrived { peer: PeerId, stream: Stream, session_id: String }`

### 6. `network_service.rs` — 集成

- [ ] 创建 `session_accept_control` + `session_open_control`
- [ ] select! 新增分支：`incoming_session_streams.next()` → 读握手 → 发送 `SessionStreamArrived`
- [ ] 注入到 `Network_Service_Capability`

### 7. `capability.rs` — Network_Service_Capability 实现

- [ ] 实现 `open_session_stream`：通过 `session_open_control.open_stream()` + 握手

---

## 参照模式

完全复用 `TensorStream` / `FileStream` 已有的模式：
- 协议常量 + 握手帧
- `stream::Control` 双通道（accept + open）
- `Network_Inbound_Event` 变体
- swarm select! 分支

---

## 人类评审

<!-- 在此区域写下评审意见 -->

