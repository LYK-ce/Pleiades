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

- **风险**：~~当前硬编码 Qwen3 对话格式~~ ✅ **已修复 (Task 9)**。引入 `minijinja` + `minijinja-contrib` 渲染 GGUF metadata 中的 `chat_template`，支持 Qwen3、Llama、Mistral 等多模型格式。Python 风格方法调用 (`split`/`startswith` 等) 通过 `rewrite_python_str_methods()` 改写为 minijinja 过滤器语法。
- **影响范围**：已消除

### 5. encode_messages 长对话性能

- **风险**：每轮 tokenize 完整对话历史（messages 数组），长对话时 tokenize 开销线性增长。
- **影响范围**：Session.spawn() 每轮 prefill 前的 `ml.encode_messages(&messages)`
- **方向**：token 缓存，仅 tokenize 新消息，拼接历史 token

### 6. 多 slot 未启用

- **风险**：slot 机制已准备（allocate_slot + SlotHandle），但 Session.spawn() 当前只维持一个活跃 prompt_rx。多 slot 并发推理需要动态 slot 注册和 KV Cache 管理。
- **影响范围**：Session.spawn() select! loop + slot_notify_rx
- **当前状态**：只使用一个 slot，满足当前需求

### 8. 多 Slot prompt_rx 覆盖导致旧 slot 孤儿化

- **风险**：`Session.spawn()` 动态 slot 注册时 `try_recv` 循环用新的 `prompt_rx` 覆盖旧的。先后分配多个 slot 时，只有最后分配的 slot 能收到 prompt，旧 slot 永久静默。消息 Vec 也被所有 slot 共享，若多 slot 同时活跃会导致对话上下文交叉污染。
- **影响范围**：`Session.spawn()` select! loop (`session.rs:112-117`), `allocate_slot` → `slot_notify_tx`
- **当前状态**：单 slot 使用，满足当前需求。推迟至 Continuous Batching 阶段统一解决。

### 9. Session spawn task 退出无错误传播

- **风险**：Session 的 spawn task 在 tokenizer 加载失败、ML 连接超时等情况下直接 return。SessionManager 的 HashMap 仍保留该条目，`list_sessions()` 返回正常，`allocate_slot` 返回 Ok 但 `slot_notify_tx.send()` 静默失败。外部无法感知 Session 已死。
- **影响范围**：`Session::spawn()` + `SessionManager::create_session()`
- **当前状态**：记录风险，推迟至 Continuous Batching 时一并重构 Session 生命周期管理。

### 10. slot_tokens HashMap 内存泄漏

- **风险**：Session.spawn() 内 `slot_tokens` HashMap 每次 allocate_slot 插入新条目，但 slot 关闭时永不删除，长期运行造成微小内存泄漏。`UnboundedSender` 虽轻量 (< 200B)，但频繁创建/关闭 slot 场景下累积。
- **影响范围**：`session.rs:115 slot_tokens.insert()` — 无对应 remove
- **当前状态**：记录风险，推迟至 Continuous Batching 时修复。

### 12. messages 历史无限增长 OOM

- **风险**：~~`Session.spawn()` 内 `messages: Vec<Message>` 每轮对话累积~~ ✅ **已修复 (Task 9)**。Session 无状态化——对话历史完全由前端管理，Session 不再自管 `Vec<Message>`。每次请求通过 `SessionRequest` 透传完整 `messages[]`。
- **影响范围**：已消除

### 13. MlSession.forward() 自动/显式 offset 语义冗余

- **风险**：`forward(tensor, offset: Option<usize>)` 同时支持自动追踪（`offset=None` 时内部 `self.ctx.offset` 自增）和显式传参。当前 Session/ML Thread 全走显式，自动模式仅旧 `run.lua` 使用。若调用方漏传 offset，静默切到自动模式，拿到过期 offset 导致推理错乱。
- **影响范围**：`MlSession::forward()` (`context.rs:274-291`)
- **方向**：移除自动递增模式，`offset` 改为必传 `usize`

### 14. 自回归 max_tokens 硬编码 300

- **风险**：~~`Session.spawn()` 自回归循环 `for _ in 0..300`，超长输出静默截断~~ ✅ **已修复 (Task 9)**。`max_tokens` 通过 `SessionRequest.max_tokens` 由前端（API/CLI）传入，Session 使用 `gen_limit = max_tokens.min(2048) as usize`。
- **影响范围**：已消除

### 14b. max_tokens 硬上限 2048

- **风险**：`Session.spawn()` 自回归循环 `gen_limit = max_tokens.min(2048)` 硬上限 2048。前端传入 `max_tokens > 2048` 时静默截断，用户无法生成超长回复。
- **影响范围**：`session.rs` 生成 loop
- **当前状态**：记录风险。短回复场景不触发；若需更长输出，改为可配置或取消防护上限。

### 15. strip_think 未闭合标签泄露

- **风险**：~~`strip_think()` 遇到未闭合 `<think>` 标签时泄露尾部内容~~ ✅ **已消除 (Task 9 Code Review)**。`strip_think()` 函数已删除——Task 9 无状态化后不再需要此过滤逻辑。
- **影响范围**：已消除

### 16. Remote Chat bridge 退出无 Session 通知

- **风险**：远端断开后 bridge break，Session 仍持有 `token_tx` sender 不知情，下次推理 send 静默失败。
- **影响范围**：`branch_stream.rs:89-95` bridge 退出 + session.rs spawn task
- **当前状态**：暂时不处理。

### 11. yamux 流缓冲

- **风险**：remote chat 单 task 串行时，recv 结束回 prompt 等待期间无人读 libp2p stream，yamux 缓冲对端数据。等新 prompt 触发 write 才 flush，导致 token 延迟和 Command Output 清空。
- **影响范围**：`branch_user.rs` remote chat handler 的 stream 读写架构
- **当前状态**：已修复 — 改为两 task（`tokio_util::compat` + `tokio::io::split`），读 task 持续 read_exact 避免 yamux 缓冲

## API (Task 7)

### 17. Chat Template 处理粗糙导致模型直接 EOS

- **风险**：~~API 只提取最后一条 user 消息~~ ✅ **已修复 (Task 9)**。API 完整透传 `messages[]` 数组给 Session，system message 和多轮历史均被保留。
- **影响范围**：已消除

### 18. 有状态 Session 与无状态 API 的架构矛盾

- **风险**：~~API 复用有状态 Session 导致上下文混乱~~ ✅ **已修复 (Task 9)**。Session 变为无状态——对话历史完全由前端管理，每次请求透传完整 `messages[]`。`SessionRequest` 替代裸 String prompt。
- **影响范围**：已消除

### 19. API slot 独占导致单并发

- **风险**：当前 API server 启动时分配一个 slot 常驻持有，后台 handler task 串行处理请求。同一 Session 的 API 只支持单并发——前一个请求推理期间（可能数秒到数十秒），后续请求排队等待。
- **影响范围**：`Src/API/routes.rs` spawn_slot_handler → request_rx 串行 loop
- **当前状态**：单用户场景暂时够用。需多并发时改为 slot pool 或每请求独立推理上下文

### 20. CUDA OOM — 长 prompt 预填充显存压力 (Task 9 期间发现)

- **风险**：OpenCode 等前端会注入超长 system prompt（实测 3488 tokens）。`nvidia-smi` 观测峰值 ~5000 MB，模型在 f32 精度下：
  - 模型权重 (0.6B f32) ≈ 2.4 GB
  - KV Cache (28 层 × 2 × 4 kv_heads × 128 dim × 3488 tokens × f32) ≈ 400 MB
  - Attention 矩阵 (prefill 时 `[1, 16 heads, 3488, 3488] × f32`) ≈ 780 MB
  - FFN 中间激活 (SwiGLU, gate+up `[1, 3488, 4864]` × 2 × f32) ≈ 136 MB/层
  - CUDA context / cublas workspace / gemm 缓冲区 ≈ 300-500 MB
  - **总量约 4.5-5 GB**，接近 8GB 显卡的碎片化阈值
- **OOM 可能原因**：CUDA 内存碎片化——多次 tensor allocate/free 后，即使总空闲内存够，找不到足够大的连续块分配给 attention 矩阵或 FFN 中间 tensor
- **影响范围**：`inference.lua` forward loop + CUDA DriverError `out of memory`；短 prompt 正常，长 prompt (>2000 tokens) 高概率触发
- **缓解方向**：
  - 使用 GGUF 量化模型（Q4_K_M 等）而非 f32 精度的 PGGUF
  - 接入 Flash Attention 消除完整 attention 矩阵（~780 MB → ~几 MB）
  - 限制 max_tokens（当前已 clamp 到 2048）
  - 前端控制 system prompt 长度
  - 设置 `CUDA_LAUNCH_BLOCKING=1` + `cuda-memcheck` 精确定位分配失败点
- **当前状态**：记录为已知风险。短 prompt（几十 token）正常，长 prompt 在 f32 精度 8GB 显存下有 OOM 风险。

### 21. std::sync::Mutex 阻塞 tokio worker 线程

- **风险**：`SessionManager::new()` 返回 `Arc<std::sync::Mutex<SessionManager>>`，所有调用方（`branch_user.rs:379/384/431`, `branch_stream.rs:64`）在 tokio async 上下文中调用 `.lock().unwrap()`。若锁竞争发生，`std::sync::Mutex` 会**阻塞整个 tokio worker 线程**而非仅挂起当前 future，可能导致 runtime 停滞。
- **影响范围**：Core 主循环 B1/B3 分支中 `session_mgr.lock().unwrap()` 调用
- **当前状态**：锁持有时间极短（HashMap 插入/删除），当前无并发争用。改为 `tokio::sync::Mutex`（`lock().await`）属防御性规范修复，暂不处理。

