# Task 6: Session Manager Implementation

> Presented by KeJi
> Date: 2026-05-22

---

## 设计目标

Session Manager 是**接入层与 ML Thread 之间的中间层**，负责：

1. **槽位分配** — 管理有限推理槽位，接入方申请/释放
2. **批处理输入** — 收集各 session 的 prompt，tokenize 后拼成 `[B, seqlen]` batch tensor 送入 ML thread
3. **流式返回** — 接收 ML thread 的 `[B]` tokens 输出，decode 后按槽位分发回各接入方

---

## 架构分层

```
   HTTP API        libp2p RPC         TUI         ← 接入层（Task 7+，本任务不实现）
      │                │               │
      ▼                ▼               ▼
┌─────────────────────────────────────────────────┐
│              Session Manager                     │
│                                                 │
│  • 槽位分配 / 生命周期管理                        │
│  • 自持 tokenizer（从模型文件加载）                │
│  • tokenize 各 slot prompt → 拼 batch tensor     │
│  • sample(logits) per slot → decode → 分发回流   │
└──────────────────────┬──────────────────────────┘
                       │
                       ▼  batch tensor [B, seqlen]
                       ◄──  logits [B, vocab]
┌─────────────────────────────────────────────────┐
│              ML Thread                           │
│                                                 │
│  • 持模型权重 + KV cache（candle ConcatKvCache）  │
│  • forward([B, seqlen]) → logits [B, vocab]     │
└─────────────────────────────────────────────────┘
```

---

## 核心设计

### 会话模型（长期占用，多轮对话）

- 每个 session 建立后长期占用一个槽位
- 接入方通过 `mpsc` 通道持续发送 prompt / 接收 token
- session 销毁时释放槽位和通道

### 批处理流程

```
slot_0: "你好..."  ──tokenize──► tokens_0 [seqlen_0]
slot_1: "写诗..."  ──tokenize──► tokens_1 [seqlen_1]
slot_2: "翻译..."  ──tokenize──► tokens_2 [seqlen_2]
                       │
                       ▼
               拼成 batch_tensor [3, max_seqlen]
               padding + attention mask
                       │
                       ▼
               ML Thread forward(batch_tensor)
                       │
                       ▼
               logits [3, vocab]
                       │
           ┌───────────┼───────────┐
           ▼           ▼           ▼
     slot_0        slot_1        slot_2
  sample(0.8)    sample(0.3)    sample(1.2)
     "你"          "春"          "The"
   decode + 流回   decode + 流回   decode + 流回

### KV Cache（candle 原生支持 batch 隔离）

- `ConcatKvCache` 存储形状 `[B, total_seqlen, heads, dim]`
- 每个 batch index 独立维护自己的历史 KV
- session_0 有 537 tokens，session_1 有 42 tokens — 互不干扰
- attention 配合 causal mask + padding mask 自动隔离

### 职责边界

| 组件 | 负责 | 不负责 |
|------|------|--------|
| Session Manager | 槽位、tokenize/decode、拼 batch、sample、分发 | HTTP/libp2p 协议、SSE、forward |
| ML Thread | forward | tokenize、batch 拼合、sample |
| 接入层 | 协议解析、请求路由 | 槽位管理、推理 |

---

## 子任务

### 6.1 [Network 改造 — Session Stream](task_6_1_network_session_stream.md)

新增 `/pleiades/session/1.0.0` 专用流协议。远程节点通过 `open_session_stream(peer, session_id)` 建流，附带 session_id 握手。到达后产生 `SessionStreamArrived` 事件，由 Core 路由到 Session Manager 分配 slot 并 spawn 桥接协程。

### 6.2 [Slot 管理](task_6_2_slot_management.md)

Session/Slot 分配、录入、分发、销毁的完整生命周期。包含主循环 6 个分支：本地 open_slot / 收 prompt / flush batch / 分发 token / 网络 open_slot / 销毁 slot。

### 6.3 [ML Thread Batch 接口](task_6_3_ml_thread_batch.md)

定义双通道接口：`BatchRequest { tensor, slot_order }` → `BatchResult { logits, slot_order }`。ML Thread 只做 `forward([B, seqlen]) → logits [B, vocab]`，sample 移到 Session Manager，各 slot 独立 temperature。本子任务只定义接口，不实现 ML Thread 侧。

### 6.4 [系统集成](task_6_4_system_integration.md)

`main.rs` spawn Session Manager 主循环，注入 `Capabilities`，Core 路由 `SessionStreamArrived`，网络桥接 stream ↔ SlotHandle。

### 6.5 测试

- [ ] 单 session 基本对话
- [ ] 多 session 并发批处理
- [ ] KV cache 隔离验证
- [ ] 槽位耗尽处理

---

## 依赖

- Task 5 (Lua 绑定重构) — 已完成
- `Src/ML_Engine/GGUF_Models/qwen3.rs` — ConcatKvCache 已有
- candle 0.10.x — batch forward + KV cache 天然支持

---

## 人类评审

<!-- 在此区域写下评审意见 -->

