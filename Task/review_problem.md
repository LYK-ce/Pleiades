# Review Problem 记录

> 全源码梳理（2026-08-08）发现的问题清单，按严重程度排序。
> 来源：子agent 全量梳理 `Src/`（约 18,800 行），对照架构文档与 Task 17 规范核对。

---

## 🔴 严重

### 1. Lua 绑定闭包内大量业务逻辑 + 设备硬编码

- **位置**: `Src/VM/capability_binding.rs:383-608`（`register_network_caps` 的 `list_model_peers` 闭包）
- **问题**:
  - 闭包内嵌 3 个 helper 函数（`parse_split_range`/`bitmap_range`/`adjust_range`）、2 个内部结构体（`RawEntry`/`Ranged`）、重叠分片合并算法，约 220 行
  - 违反架构文档「错误 4：Lua 绑定闭包内不写业务逻辑」，也违反 instructions.md Lua 绑定规范
  - 闭包内硬编码 `devices = {cuda:0, cuda:1}`（注释「所有节点固定 2 GPU」）
- **建议**: 业务逻辑提取为独立模块函数；设备列表进入配置

### 2. Session 推理设备硬编码 "cpu"

- **位置**: `Src/Session_Manager/session.rs:80`（`MlContext::new("cpu")`）
- **问题**: Session 推理管线写死 CPU；`UserCommand::SetDevice` 的 `device_preference` 字段（`core.rs:88` 定义）只在 `route_user` 里赋值，从未传给 Session，命令不生效
- **建议**: 设备偏好传入 Session / 纳入配置

### 3. Tensor_Buffer 容量硬编码（magic number）

- **位置**:
  - `Src/VM/capability_binding.rs:712` — `New(256 * 1024)`
  - `Src/VM/local_stream.rs:106` — `New(256 * 1024)`
  - `Src/Session_Manager/session.rs:132` — `New(16 * 1024 * 1024)`
- **建议**: 提取为配置项或模块常量统一管理

---

## 🟠 中等

### 4. Core 主循环内同步阻塞 I/O（SetName）

- **位置**: `Src/Orchestrator/core/branch_user.rs:153-154`
- **问题**: `SetName` 分支直接在 Core 主循环执行 `config::Set_Peer_Name`（内部 `std::fs::write` 同步写 `.config/config.toml`），违反「主循环纯路由 + 毫秒级返回」原则；且路径硬编码 `Path::new(".config").join("config.toml")`
- **其余分支已正确 spawn**（Flush/Dial/Send/Session/Api/ExecRemote），仅此一处违规
- **建议**: 改为 `tokio::spawn` + 引用 `config::CONFIG_DIR`

### 5. bootstrap 节点 PeerId::random()

- **位置**: `Src/Network/network_service.rs:318`
- **问题**: `// TODO: 从地址中提取PeerId`，Kademlia `add_address` 使用随机 PeerId，引导节点实际无法正确入网（当前 `bootstrap_peers` 为空，未触发）
- **建议**: 从 multiaddr 解析 PeerId

### 6. 未实现功能残留为文本占位

- **位置**: `Src/Orchestrator/core/branch_user.rs:95-99`（`Run`）、`:193-197`（`DistributeModel`）、`:318-322`（`Profile`）仅发布 "not yet implemented" 文本；`job.rs:13` 注释 `Generic (todo!() 占位)`
- **影响**: 用户可见但功能缺失

### 7. os 沙箱被打开，测试与实现矛盾

- **位置**: `Src/VM/engine.rs:22`（`globals.set("os", Value::Nil)` 被注释掉，注释「pipe_7/8 计时需要」）；`engine.rs:48-56` `test_sandbox_os_blocked` 断言 os 被禁用
- **问题**: 文档（3.7.1 禁用 os/io/require/dofile/loadfile）与代码矛盾，测试与实现冲突
- **建议**: 确认 `pipe_7/8` 是否已归档（`programs/archived/`），若已归档应恢复禁用并修正测试

### 8. 主循环 SHUTDOWN_GRACE 期间 registry 非空即强制退出

- **位置**: `Src/Orchestrator/core.rs:152-158`
- **问题**: 30s 超时后直接 break，可能中断未完成推理（设计如此，但无告警级别选择）

---

## 🟡 轻微

### 9. 死配置项 quota_gb

- **位置**: `Src/Config/config.rs:35` — DEFAULT_CONFIG 有 `quota_gb = 0`，`Storage_Config` 无该字段（serde 静默忽略），Task 17.3 已指出但未清理

### 10. find_available_port(8080) 硬编码起始端口

- **位置**: `Src/API/server.rs:47`；`main.rs:96` `listen_port: 0` 无配置来源

### 11. programs/ 路径硬编码

- **位置**: `Src/VM/registry.rs:16-17`（相对工作目录，未纳入 Config）

### 12. max_tokens 硬编码

- **位置**: `Src/Orchestrator/core/branch_stream.rs:93`（`max_tokens: 256`）；`session.rs:198`（`max_tokens.min(2048)`）

### 13. Session 槽位数硬编码

- **位置**: `Src/Orchestrator/core.rs:105-108`（`SessionManager::new(4, ...)`）；`Session_Config.max_slots` 已删但未配置化

### 14. LocalStreamHub::accept 同步 sleep 轮询

- **位置**: `Src/Orchestrator/local_tensor_stream/mod.rs:70`（`std::thread::sleep(10ms)`）；`VM/local_stream.rs:53-56` 阻塞调用线程
- **建议**: `accept_async` 已是正确做法，旧 `accept` 可淘汰

### 15. gguf_model_legacy.rs 疑似死代码

- **位置**: `Src/ML_Engine/gguf_model_legacy.rs`（722 行，`AnyModel`）与 `gguf_model.rs`（新版 stage 模型）并存；`ml_engine/mod.rs` 只导出 `gguf_model`，legacy 疑似死代码

### 16. 冻结模型代码

- **位置**: `GGUF_Models/mod.rs:12-18` 注释停用 `deepseek_v4`（1577 行）/`deepseek_v3.rs`（942 行）/`llama.rs`（239 行），约 2,758 行冻结代码保留源码

### 17. 重复代码

- `Causal_Mask` 三处重复：`qwen3/qwen3.rs:286-312`、`qwen3/qwen3_moe.rs:269-285`、`llama.rs:194-203`
- `Build_From_Extracted` 类辅助函数四处重复：`qwen3.rs:57-76`、`qwen3_moe.rs:96-100`、`llama.rs:117-127`、`deepseek_v3.rs:199-232`
- offload 序列化手写 LE 编码（`context.rs:605-760`）未复用 bincode

## Task 18 二次 Code Review 问题记录（2026-08-08，待修复）

> 来源：Task 18 实施后 code review。R1/R2 已修复，其余记录待后续处理。

### R3 🟠 TUI 三个 partial 事件互相清空显示
- **位置**: `Src/TUI/mod.rs:203-215`（handle_state peer_info_updated）
- **问题**: peer-info 事件带 `models:[] sessions:[]`，models 事件带 `sessions:[]`，sessions 事件带 `models:[]`；`Update_Peer_Models/Update_Peer_Sessions` 整体覆盖 → 任一 topic 到达都会清空另一字段的 TUI 显示
- **影响**: 仅 TUI 显示层；PeerManager 数据不受影响
- **建议**: TUI handler 对空数组跳过更新（与 Update_Peer 空 name 不覆盖同策略）；或接收端合并后再发 TUI

### R4 🟠 peer-info 分支 upsert 清空 addresses
- **位置**: `Src/Network/swarm_events.rs:311-313`（peer-info 分支 `PeerInfo::new(author, vec![])`）
- **问题**: `upsert_peer` 不保留 addresses（只保留 name/profile/models/sessions 等），identify 写入的 listen_addrs 被后续 peer-info 消息覆盖成空
- **影响**: addresses 当前无读取方，实际危害低；未来 dial-back/重连会踩坑
- **建议**: upsert_peer 增加"addresses 为空时保留旧值"（与 sessions 同款逻辑）

### R5 🟠 启动竞态：初始 flush 广播先于连接建立丢失
- **位置**: `Src/main.rs:110`（spawn_initial_flush）vs `Src/main.rs:126`（network Start）
- **问题**: 初始 flush 的 publish 若在连接建立前执行 → 无订阅者被丢弃且无补发
- **现状**: R1 修复（连接建立/Subscribed 补发）已基本根治，此条可关闭或保留观察

### R6 🟡 命名规范违反（instructions.md 函数须 Pascal snake case）
- **位置**: `Src/Network/Gossipsub/mod.rs`（publish_peer_info/publish_models/publish_sessions）、`Src/Network/capability.rs`（publish_gossipsub）
- **建议**: 改名为 Publish_Peer_Info / Publish_Models / Publish_Sessions / Publish_Gossipsub（含 trait 方法 + StubNetwork + 所有调用方）

### R7 🟡 文件头 Modified Date 未更新
- **位置**: peer_manager.rs 等 13 个修改文件头注释仍为旧日期/旧格式（仅 Date 行）
- **建议**: 按 instructions.md 维护 Created/Modified Date

### R8 🟡 过时/误导注释
- `network_service.rs:24` "Data / Info → Network 内部直接回复"（Info 已改 warn 忽略）
- `swarm_events.rs:170-172` Handle_Request_Response_Event doc 同款文案
- `swarm_events.rs:103-106` mDNS 注释 "→ Info 交换 (name + models)"
- `inbound.rs:11-12` "BandwidthTest / Data / Info 由 Network 内部直接回复"
- `network_service.rs` 结构体 doc 重复两行
- `codec.rs:44` DataType::Info doc 可加 [deprecated]

### R9 🟡 错误处理细节
- `command_handler.rs:61-65`: publish 失败统一 warn，其中 NoPeersSubscribedToTopic 是正常情况（单节点/启动期高频）→ 建议 debug
- `swarm_events.rs` identify Error 事件仅 debug → 建议 warn

### R10 🟡 空列表不传播（peer 清空后远端残留旧值）
- `swarm_events.rs:333-334, 351-352`: `if !models.is_empty()` / `if !sessions.is_empty()` 守卫 → peer 删除全部模型/会话后远端仍显示旧数据
- **建议**: 移除守卫（配合 R3 的 TUI 空数组跳过）

### R11 🟡 其他
- `Gossipsub/mod.rs` Build_Peer_Info_Payload 含 peer_id 字段，但接收端用 message.source → 冗余，建议删除或用于防伪校验
- `Orchestrator/mod.rs` StubNetwork 补 publish_gossipsub ✅（已做）

### 测试建议
- upsert_peer "保留 sessions" 逻辑无单测覆盖（t04 预存编译错误），建议补单测
- t07 TC-05 doc 注释过时（Info 不再回复）

## Task 18 测试验证发现（2026-08-08，待修复）

### G1 🔴 MoE router gate 权重硬编码反量化到 CPU（device mismatch）

- **位置**: `Src/ML_Engine/GGUF_Models/qwen3/qwen3_moe.rs:117`（`Qwen3MoE_Layer::Build_From_Extracted`）
- **错误**: 双卡 2 节点 30B pipeline 推理时 worker 报 `Forward failed: device mismatch in matmul, lhs: Cuda{gpu_id:0}, rhs: Cpu` → worker 崩溃 → coordinator `recv_tensor: unexpected end of file`
- **根因**:
  ```rust
  // 当前代码（117 行）:
  let gate_ws = gate_qt.dequantize(&Device::Cpu)?.to_dtype(DType::F32)?;  // ← 硬编码 CPU
  let gate = candle_nn::Linear::new(gate_ws, None);                        // gate 权重在 CPU
  ```
  30B-A3B 是 qwen3moe（MoE）模型。`FusedMoeGGUF.forward` 中 `self.gate.forward(&xs)`：xs 来自 attention 输出（Cuda）× gate_ws（CPU）→ device mismatch。**注意 `Build_From_Extracted` 没有 device 参数**，无法加载到目标设备。
- **时间线**: git 对比 `b134dea`（2026-06-17）该行已存在——**非 Task 18 引入，非 7 月 ML_Engine 重构引入**。6/17 wb 记录的"双卡 30B 多轮对话 ✅"存疑（可能当时未真正走到 MoE 推理路径）。单机 0.6B（dense，无 MoE）不受影响。
- **修复方案**:
  1. `Qwen3MoE_Layer::Build_From_Extracted` 签名加 `device: &Device`
  2. `gate_qt.dequantize(device)?.to_dtype(DType::F32)?`
  3. `Load_Stages`（qwen3_moe.rs:240-251）调用处传 `device`（参数已有）
- **影响**: 阻塞双卡/单卡 MoE 模型（30B）pipeline 推理验证；dense 模型（0.6B）不受影响

### G2 📝 双卡 30B pipeline 测试过程备注
- 网络层（identify/gossipsub/张量流建立/EXEC 派发/流水线就绪）全部正常 ✅，失败仅在 worker ML forward
- 测试时 chat.py 必须交互式输入、不带 `-s` system prompt（用户要求）
- 管道式 printf 输入会残留 python 进程占用 Session slot（需 pkill 清理）

---

## 架构文档 vs 代码差异（文档过时项）
---

## 架构文档 vs 代码差异（文档过时项）
---

## 架构文档 vs 代码差异（文档过时项）

| 差异点 | 文档 | 代码实际 |
|--------|------|----------|
| `Capabilities.peer_manager` 类型 | `Box<dyn ...>` | `Arc<dyn ...>`（`Orchestrator/mod.rs:34`） |
| StorageCapability 方法命名 | 小写 snake_case | 全部 UpperCamelCase（`Acquire_Read` 等） |
| 配置段 | `[Runtime]`/`[Session]`/`[Scheduler]` | 已删除，仅 Log/Network/Storage/Identity 4 段 |
| `MlSession` | 3.4.5 节 API | 已改名 `MlContext`（`context.rs:67`） |
| 沙箱禁用 os | 3.7.1 禁用 | `engine.rs:22` 被注释（见问题 7） |
| `test_bandwidth` 签名 | `&self, peer: &PeerId` | `&self, peer: PeerId` |

---

## Task 17 落地情况核对（✅ 全部落地）

| 项 | 结论 | 证据 |
|----|------|------|
| 17.1 CONFIG_DIR 唯一权威 | ✅ | `config.rs:43` `pub const CONFIG_DIR`；main.rs 引用无重复 |
| 17.2 workspace_dir() 方法 | ✅ | `config.rs:125-148` |
| 17.3 log_dir()/log_level() | ✅ | 同上 |
| 17.4 默认值同步 | ✅ 部分 | `quota_gb` 死配置项未清理（见问题 9） |
| 17.5 kvcache_dir() OnceLock | ✅ | `config.rs:49-65`；context.rs 两处已改 |
| 17.6 日志时间戳命名 | ✅ | `main.rs:45-46` |
| 17.7 注释修正 | ✅ | main.rs |
| 17.8 CLI 模块化 | ✅ | `CLI/mod.rs` 82 行；`lib.rs:48-49` |
| Storage 方案 B（持有依赖） | ✅ | `storage_manager.rs:29-30`，`Flush()` 内 `Sync_Models_To_Peer_Manager()` |
