# Task 6 v2.8: 远端 Chat — Session 流协议 + remote chat 命令

> Presented by KeJi
> Date: 2026-05-25

---

## 目标

通过新增 `/pleiades/session/1.0.0` 流协议，使远端节点能够连接本地的 Session 进行推理。新增 `remote chat <peer_name> <session_id>` 命令，从本地连接到远端的 Session。

---

## 背景

当前 tensor stream (`/pleiades/tensor/1.0.0`) 的入站不经过 Core — 网络层收到后直接插入 `RendezvousMap`。文件 stream (`/pleiades/file-stream/1.0.0`) 则通过 `Network_Inbound_Event::FileStreamArrived` 路由到 Core 的 B3 分支。

Session 流需要走 Core 路由（因为需要访问 SessionManager 分配 slot），因此采用类似文件流的模式：网络层解析入站流 → 构造事件 → Core B3 处理。

```
远端 Chat ── /pleiades/session/1.0.0 ──► Core B3 ──► allocate_slot ──► Session
                                                                 │
                                                  bridge: stream ↔ mpsc
```

---

## Session 流协议

```
协议名: /pleiades/session/1.0.0

Handshake (远端→本地):
  [8B BE u64 session_id]

数据帧 (双向):
  [4B BE u32 payload_len][payload UTF-8 bytes]
```

上行（远端→本地）为 prompt，下行（本地→远端）为 token。

---

## 实施计划

### 涉及文件

| 文件 | 改动 |
|------|------|
| `Src/Network/Session_Stream/protocol.rs` | **新建** — `SESSION_STREAM_PROTOCOL` 常量 + handshake 读写 + 帧读写 |
| `Src/Network/capability.rs` | `Network_Inbound_Event` 加 `SessionStreamArrived` 变体；`Network_Capability` trait 加 `open_session_stream` / `send_session_frame` / `recv_session_frame` |
| `Src/Network/network_service.rs` | 注册 session 协议 accept；select! 新增分支解析 handshake → 发送事件 |
| `Src/Network/mod.rs` | 声明 `Session_Stream` 模块 |
| `Src/Orchestrator/core/branch_stream.rs` | B3 新增 `SessionStreamArrived` 分支 → spawn bridge |
| `Src/Orchestrator/core/branch_user.rs` | 新增 `RemoteChat` 命令处理 |
| `Src/Orchestrator/command.rs` | 新增 `UserCommand::RemoteChat { peer_name, session_id }` |
| `Src/TUI/mod.rs` | 解析 `remote chat <name> <id>` |
| `Src/main.rs` | 初始化时注册 session 流协议（如需） |

---

### 阶段 1: Session 流协议实现

| 步骤 | 文件 |
|------|------|
| 新建 `Session_Stream/` 目录 + `protocol.rs` | `Src/Network/Session_Stream/protocol.rs` |
| 常量 `SESSION_STREAM_PROTOCOL = "/pleiades/session/1.0.0"` | 同上 |
| `Read_Session_Handshake(&mut stream) → Result<u64>` — 读 8B session_id | 同上 |
| `Write_Session_Handshake(&mut stream, session_id: u64)` — 写 handshake | 同上 |
| `read_session_frame(&mut stream) → Result<String>` — 读 [4B len][UTF-8] | 同上 |
| `write_session_frame(&mut stream, text: &str)` — 写帧 | 同上 |
| 新建 `mod.rs`，export protocol 模块 | `Src/Network/Session_Stream/mod.rs` |
| `Src/Network/mod.rs` 加 `pub mod Session_Stream;` | `Src/Network/mod.rs` |

### 阶段 2: Network 层适配

| 步骤 | 文件 |
|------|------|
| `Network_Inbound_Event` 加 `SessionStreamArrived { peer: PeerId, session_id: u64, stream: libp2p::Stream }` | `capability.rs` |
| `Network_Capability` trait 加方法签名：`open_session_stream(peer, session_id) → Stream`, `send_session_frame`, `recv_session_frame` | `capability.rs` |
| `Network_Service_Capability` 加 `session_stream_control: stream::Control` 字段 | `capability.rs` |
| `Network_Service_Capability` 实现新 trait 方法 | `capability.rs` |
| `Network_Service` 注册 accept: `self.session_accept_control.accept(StreamProtocol::new(SESSION_STREAM_PROTOCOL))` | `network_service.rs` |
| select! 加分支：`incoming_session_streams.next()` → 读 handshake → `orchestrator_event_tx.send(SessionStreamArrived{...})` | `network_service.rs` |
| `Network_Service::Init()` 创建 `session_stream_control` + `session_accept_control`，返回 capability | `network_service.rs` |

### 阶段 3: Core 处理入站 Session 流

| 步骤 | 文件 |
|------|------|
| `route_stream()` 加 `Network_Inbound_Event::SessionStreamArrived { peer, session_id, mut stream }` 分支 | `branch_stream.rs` |
| 分支逻辑: `self.session_mgr.lock().allocate_slot(session_id)` → 拿到 SlotHandle | `branch_stream.rs` |
| spawn bridge task: loop { `read_session_frame(&mut stream)` → `prompt_tx.send()` } | `branch_stream.rs` |
| bridge task 另一半: loop { `token_rx.recv()` → `write_session_frame(&mut stream, token)` } | `branch_stream.rs` |
| 双向 relay 需要两个 spawn 或 select! 在同一个 task 中 | `branch_stream.rs` |

### 阶段 4: remote chat 命令

| 步骤 | 文件 |
|------|------|
| `UserCommand` 加 `RemoteChat { peer_name: String, session_id: u64 }` | `command.rs` |
| TUI 解析 `remote chat alice 1` | `TUI/mod.rs` |
| `route_user()` 加 RemoteChat 分支 | `branch_user.rs` |
| 分支逻辑: PeerManager 解析 name → PeerId → `network.open_session_stream(peer, session_id)` → spawn relay tasks | `branch_user.rs` |
| relay 1: subscribe `prompt_tx` → `write_session_frame(&mut stream, prompt)` | `branch_user.rs` |
| relay 2: `read_session_frame(&mut stream)` → EventBus::Stream token | `branch_user.rs` |

---

## 不在此范围

- 远端 Chat 的 session close / 断开处理
- 多 slot 远端并发
- Session 流加密（libp2p Noise 已提供传输层加密）

---

## 人类评审

<!-- 在此区域写下评审意见 -->

