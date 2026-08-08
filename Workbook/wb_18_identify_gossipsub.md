# wb_18_identify_gossipsub

## 概述

Task 18：接入 libp2p 标准 identify + gossipsub，替换自研 Info 交换（request-response）与 broadcast_local_info 广播。

方案文档：`Task/task_18_identify_gossipsub.md`（2026-08-08 定稿）

## 实施（2026-08-08）

### 改动文件（13 处）

| 文件 | 改动 |
|------|------|
| Cargo.toml | features +"identify" +"gossipsub" |
| Cargo.lock | 新增 libp2p-identify 0.47.0 / libp2p-gossipsub 0.49.5 / async-channel / hashlink 0.9 / hex_fmt |
| Src/Network/network_service.rs | use + PleiadesNetworkBehaviour 加 identify/gossipsub 字段 + 闭包创建（identify Config / gossipsub Signed）+ Start 订阅 3 topic |
| Src/Network/swarm_events.rs | Identify/Gossipsub 事件分支 + Handle_Identify_Event + Handle_Gossipsub_Event；删 ConnectionEstablished Info 发送（45-52）；删入站 Info 解析（195-253）→ 加 Info 忽略分支 |
| Src/Network/mod.rs | 删 build_local_info_payload + broadcast_local_info；新增 TOPIC_PEER_INFO/MODELS/SESSIONS 常量 + publish_peer_info/publish_models/publish_sessions |
| Src/Network/node_handle.rs | NodeCommand::GossipsubPublish + NodeHandle::Gossipsub_Publish |
| Src/Network/command_handler.rs | GossipsubPublish 分支（TopicHash::from_raw + publish） |
| Src/Network/capability.rs | Network_Capability::publish_gossipsub trait + impl |
| Src/VM/capability_binding.rs | 删 "Info" => DataType::Info 映射 |
| Src/Orchestrator/core.rs | do_flush: broadcast → publish_models + publish_sessions |
| Src/Orchestrator/core/branch_user.rs | session 创建后: broadcast → publish_sessions |
| Src/Orchestrator/mod.rs | StubNetwork 补 publish_gossipsub stub |
| Src/PeerManagement/peer_manager.rs | upsert_peer 保留 sessions（gossipsub 分 topic 消息防清空） |

### 关键实现决策（实施中确认）

1. **gossipsub 0.49.5 API**（实际解析版本，非 0.49.0）：
   - `Behaviour::new(MessageAuthenticity::Signed(key), Config) -> Result<Self, &'static str>`
   - `subscribe<H: Hasher>(&Topic<H>)` — 用 `IdentTopic::new(name)`（原始字符串，匹配默认 hash_topics=false）
   - `publish(impl Into<TopicHash>, data)` — 用 `TopicHash::from_raw(String)`
2. **TopicHash 一致性**：订阅用 `IdentTopic`（IdentityHash=原始串），发布用 `TopicHash::from_raw(同名串)` — 两者相等
3. **upsert_peer 保留 sessions**：peer-info 消息（仅 name）不会清掉 sessions/models（models 已有保留逻辑，sessions 本次补上）
4. TUI 兼容：3 个 publish 函数分别发 peer_info_updated（is_local=true，name/models/sessions 各自带），Update_Peer 空 name 不覆盖 → 兼容

### 保留（用户决策）
- `DataType::Info` 枚举变体 + From_U8（codec.rs）保留——唯一保留项
- 入站 Info 消息 → warn 忽略分支
- tests/t07 TC-05 保持 #[ignore]（DataType::Info 在可编译）

### 编译验证
- `./build.sh check` ✅（无 error；修改文件无 warning）
- `./build.sh`（release）✅ Finished
- lib 单元测试：112 passed / 1 failed ✅（唯一失败 test_sandbox_os_blocked 为预存问题，os 沙箱禁用被注释，见 review_problem #7）
- 集成测试 t07 等：预存编译错误（PeerHandle 已删 / create_peer_management 签名变 / BandwidthTest 变体删）——非本次引入，长期未编译

## 二次 Code Review 修复（2026-08-08）

- 子agent review 发现 R1-R11 问题（详见 Task/review_problem.md 二次记录）
- 按用户指示只修复 R1/R2，其余记录待后续：

### ✅ R1 修复：连接建立/Subscribed 补发
- 新增 `Gossipsub` 子目录：`Src/Network/Gossipsub/mod.rs`（topic 常量 + Build_*_Payload 纯函数 + publish_* 高层函数），mod.rs 删旧函数 + pub mod Gossipsub + re-export
- `swarm_events.rs` 新增 `publish_local_state_to_gossipsub()`（发 3 个 topic）：
  - ConnectionEstablished 分支调用（连接建立后发布）
  - `Event::Subscribed` 分支调用（对方订阅后补发）
- 修复 borrow 错误：逐次 behaviour_mut() 而非持有引用

### ✅ R2 修复：name 传播链路
- `branch_user.rs` SetName：改名成功后调 publish_peer_info 广播
- `core.rs` do_flush：补调 publish_peer_info（与 models/sessions 并列）

### 📝 记录待修（review_problem.md）
- R3 TUI partial 事件互相清空 / R4 addresses 被清 / R5 启动竞态（R1 已基本根治）/ R6 命名规范 / R7 头注释日期 / R8 过时注释 / R9 错误级别 / R10 空列表不传播 / R11 payload peer_id 冗余

### ✅ 部署验证（2026-08-08，不跑模型）

`./build.sh deploy` ✅ → 节点 1/2 TUI 启动验证：

| 验证项 | 结果 |
|--------|------|
| GossipSub 订阅 3 topic | ✅ 双方日志 "GossipSub 已订阅 3 个 topic" |
| mDNS 双向发现 | ✅ |
| identify 自动交换（双向） | ✅ agent=pleiades/0.1.0，proto 含 /meshsub/1.x + /pleiades/* 全套，listen 含 3 网卡地址 |
| 连接建立后 publish（R1） | ✅ 节点 2 收到 "节点信息: A" |
| name 双向广播（R2） | ✅ A→B "节点信息: A"；B→A "连接建立: node-b#3Dt8" |
| SetName 实时广播（R2） | ✅ set-name node-b 即时传播到节点 1 面板 |
| models 广播（含层位图） | ✅ 节点 2 dp 显示 A 的 4 个模型 [layers:0-93] 等 |
| TUI 模型子行显示 | ⚠️ R3 已知问题（partial 事件互相清空），PeerManager 数据正确 |

**发现**：节点 1 Qwen3-8B.pgguf analyze 失败（"failed to fill whole buffer"，文件可能损坏，与本次改动无关）

### ✅ 单机 0.6B 推理验证（2026-08-08）

`session create Qwen3-0.6B-Q8_0.pgguf` → `session inference single_inf 1` → `api 1`（端口 8080）→ chat.py 对话：
- ✅ tokenizer 加载、ML Thread 就绪、Session ML connected
- ✅ API server listening on 127.0.0.1:8080
- ✅ chat.py 正常回复（"你好呀！有什么可以帮助你的吗？😊"，含 think 块）
- **结论：identify/gossipsub 改动未影响单机推理链路**

### 待办
- [ ] 修复 R3（TUI partial 事件清空显示）
- [ ] 修复 R4/R6-R11（review_problem.md 记录）
- [ ] 双卡 2 节点 / 单卡 3 节点推理回归（需模型）
- [ ] os 沙箱测试修复（预存，与 Task 18 无关）
