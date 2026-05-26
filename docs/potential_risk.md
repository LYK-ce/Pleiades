# 潜在风险记录

## PeerManager

### 1. 心跳全局写锁竞争

- **风险**：`Update_Profile` 高频调用时，每次心跳都需要获取 `RwLock<HashMap>` 的全局写锁。多节点场景下，心跳更新会阻塞所有读操作（`Get_Peer`、`Get_Peers`、`Count` 等），造成不必要的排队延迟。
- **影响范围**：Network 心跳 → Orchestrator/Scheduler 查询 + Lua 调用链路
- **当前状态**：记录为已知风险，沿用现有 `tokio::sync::RwLock<HashMap>` 方案
- **备用方案**：`DashMap` 分片锁（引入新依赖，同步锁在 tokio 上下文中不完美）

## Tensor_Stream

### 1. 单 inference_id 多流

- **风险**：当前 Rendezvous 设计一个 `inference_id` 对应一 inbound + 一 outbound。若未来需冗余备份或多路传输（一个推理会话对应多条 tensor 流），rendezvous 需支持一个 id 匹配多条流。
- **影响范围**：`RendezvousMap` 数据结构 + Lua API 语义
- **当前状态**：先不考虑，当前单 inbound + 单 outbound 满足需求

## Session / 推理

### 1. logits 全量传输

- **风险**：每 token 传输 ~600KB logits（整个词表 150k+ 维度），通过 local_tensor_stream 在 Session ↔ ML Thread 间传递。高频推理时带宽占用大，影响生成速度。
- **影响范围**：Session.spawn() 自回归 loop + inference.lua forward loop
- **当前状态**：sample 放在 Session 侧，ML Thread 只做 forward。每 token 需完整 logits tensor
- **方向**：ML Thread 侧 sample，只回传 token_id (4B)，消除 600KB 开销

### 2. stop string 检测缺失

- **风险**：当前仅靠 EOS token (151645) 终止生成。temperature=0 (greedy) 时正常，非 greedy 采样时可能错过停止条件，生成冗长或无意义内容。
- **影响范围**：Session.spawn() 自回归 loop 的 EOS 检测
- **方向**：增加 stop string 检测（如 `<|im_end|>`、`</think>` 等），提前终止

### 3. KV Cache 每轮全量重建

- **风险**：多轮对话每轮清空 KV Cache → 重新 prefill 完整 history。think 过滤后历史内容变化，无法安全复用前缀。对话轮次多时 prefill 计算浪费。
- **影响范围**：Session.spawn() + inference.lua offset=0 reset
- **方向**：对 think 过滤前后 assistant 内容做前缀一致性校验，仅变化时重建

### 4. chat template 硬编码

- **风险**：当前硬编码 Qwen3 对话格式（`<|im_start|>user\n...`），换模型（Llama/Mistral）时模板不匹配，导致输出异常。GGUF metadata 已有 `tokenizer.chat_template` 但需 Jinja 引擎解析。
- **影响范围**：`MlSession::apply_chat_template()`
- **方向**：引入 `minijinja`/`tera` 或直接映射特殊 token ID

### 5. encode_messages 长对话性能

- **风险**：每轮 tokenize 完整对话历史（messages 数组），长对话时 tokenize 开销线性增长。
- **影响范围**：Session.spawn() 每轮 prefill 前的 `ml.encode_messages(&messages)`
- **方向**：token 缓存，仅 tokenize 新消息，拼接历史 token

### 6. 多 slot 未启用

- **风险**：slot 机制已准备（allocate_slot + SlotHandle），但 Session.spawn() 当前只维持一个活跃 prompt_rx。多 slot 并发推理需要动态 slot 注册和 KV Cache 管理。
- **影响范围**：Session.spawn() select! loop + slot_notify_rx
- **当前状态**：只使用一个 slot，满足当前需求

### 7. yamux 流缓冲

- **风险**：remote chat 单 task 串行时，recv 结束回 prompt 等待期间无人读 libp2p stream，yamux 缓冲对端数据。等新 prompt 触发 write 才 flush，导致 token 延迟和 Command Output 清空。
- **影响范围**：`branch_user.rs` remote chat handler 的 stream 读写架构
- **当前状态**：已修复 — 改为两 task（`tokio_util::compat` + `tokio::io::split`），读 task 持续 read_exact 避免 yamux 缓冲
