# wb_20_gossipsub_refactor_snapshot

## 任务
`Task/task_20_gossipsub_refactor_snapshot.md`：GossipSub 发送侧解耦 + 快照机制（SnapshotCache）。

## 开始
2026-08-08（可行性分析阶段）

## 可行性分析（2026-08-08，Explore 子 agent）
结论：**可行，需少量调整**。已核对 9 个涉及文件 + libp2p-gossipsub-0.49.5 源码（/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libp2p-gossipsub-0.49.5/）。

### 关键验证（源码级）
1. `Event::Subscribed{peer_id,topic}` = 远端订阅时触发（behaviour.rs ~L2011）；本地 subscribe 不触发 → 新节点订阅 3 topic 老节点收 3 次事件 ✅
2. publish 无订阅者 → `PublishError::NoPeersSubscribedToTopic` → 「失败仍更新快照」合理 ✅
3. 订阅后必达时序：`peer.topics.insert`（~L1961）先于事件入队（~L2011）→ 处理 Subscribed 时重放 publish 必达 ✅
4. SnapshotCache 无需 Mutex：Start() 单 task，select! 每轮一个分支，&mut self 顺序执行 ✅
5. 删 ConnectionEstablished 补发安全：竞态被快照兜底（无遗漏场景）✅
6. ⚠️ do_flush 本地事件缺口：Storage::Flush 内部 Sync_Models_To_Peer_Manager（storage_manager.rs:498）只发 models 事件；name/sessions 本地事件需 do_flush 显式补发

### 调用点全量核对（无遗漏）
- publish_peer_info: core.rs:137, branch_user.rs:158
- publish_models: core.rs:140
- publish_sessions: core.rs:143, branch_user.rs:399
- publish_local_state_to_gossipsub: swarm_events.rs:46(ConnectionEstablished), :411(Subscribed)
- Build_*_Payload: 仅 Gossipsub/mod.rs 内部 + swarm_events.rs:280-282

### 调整点
1. core.rs do_flush：Build_* + publish_gossipsub ×3 + 显式补发 name/sessions 本地事件
2. Gossipsub/mod.rs：删 publish_* 后清理 L7-8 遗留 import（EventBus/Peer_Management_Capability）
3. Build_*_Payload 做 `impl PeerManager` 无 &self 关联函数（local: &PeerInfo）
4. command_handler.rs GossipsubPublish（L60-68）：Ok/Err 都 update 快照（payload 先 clone）
5. swarm_events.rs：删 L46 调用 + L274-294 方法；Subscribed（L408-412）按 topic 精准重放 `snapshot_cache.get(topic.as_str())`
6. Network/mod.rs L80-84：删 3 publish_*，保留 TOPIC_*，补 SnapshotCache
7. branch_user.rs：SetName（L158-162）/ session（L399）改 Build_* + publish_gossipsub，保留本地事件
8. Orchestrator/mod.rs StubNetwork 无需改（publish_gossipsub L72 已有）
9. network_service.rs：字段 + Init（L323-340）初始化 SnapshotCache::new()

### 重放冗余（可接受）
Subscribed 重放广播给该 topic 全部订阅者（非仅新 peer）；gossipsub duplicate_cache 不拦截（新 seq_no→新 msg_id）。3 节点场景可接受。

## 依赖
- Task 18（wb_18）：gossipsub 现状基础
- libp2p-gossipsub 0.49.5：Subscribed/NoPeersSubscribedToTopic 语义

## 实施（2026-08-08，按可行性分析+2 修正点落地）

8 个文件改动（全部完成，`./build.sh check` ✅ 无 error、无新增 warning）：

| 文件 | 改动 |
|------|------|
| Src/PeerManagement/peer_manager.rs | 新增 `impl PeerManager` 关联函数 Build_Peer_Info/Models/Sessions_Payload（无 &self，纯函数，原样迁移 Gossipsub/mod.rs 序列化逻辑） |
| Src/Network/Gossipsub/mod.rs | 重写：新增 SnapshotCache（New/Update/Get，HashMap<String,Vec<u8>>）；删 3×Build_*_Payload + 3×publish_*；保留 TOPIC_* 常量；清理 import |
| Src/Network/network_service.rs | 结构体加 snapshot_cache: SnapshotCache + use + Init 初始化 |
| Src/Network/command_handler.rs | GossipsubPublish：publish 无论 Ok/Err 都 snapshot_cache.Update（payload 先 clone） |
| Src/Network/swarm_events.rs | 删 publish_local_state_to_gossipsub（方法+ConnectionEstablished 调用）；Subscribed{peer_id,topic} 按 topic 精准重放 snapshot_cache.Get(topic.as_str()) |
| Src/Network/mod.rs | re-export：删 publish_peer_info/models/sessions，保留 TOPIC_*，补 SnapshotCache |
| Src/Orchestrator/core.rs | do_flush：Build_* + publish_gossipsub ×3 + 显式补发 name/sessions 本地 peer_info_updated 事件（models 已由 Storage::Flush 内部发） |
| Src/Orchestrator/core/branch_user.rs | SetName：Build_Peer_Info_Payload + publish_gossipsub + 本地事件；session 创建：Build_Sessions_Payload + publish_gossipsub + 本地事件 |

头注释：7 个文件升级为 Created/Modified Date 格式（Modified = 2026-08-08）。

### 实现要点
- SnapshotCache 方法用 New/Update/Get（Pascal snake case，避免 R6 命名规范问题）
- 无订阅者 publish 失败也更新快照（NoPeersSubscribedToTopic 属正常）
- Subscribed 重放广播给该 topic 全部订阅者（gossipsub 无定向 API，幂等可接受）
- StubNetwork::publish_gossipsub 已存在无需改（Orchestrator/mod.rs:72）

## 部署 + 双卡 2 节点实测（2026-08-08）

`./build.sh deploy` ✅（release 编译 16.65s + 部署）

### 快照机制验证（Task 20 核心）✅ 全部通过

启动顺序 A(13:22:08) → B(13:22:27)：

| 验证项 | 结果 | 证据 |
|--------|------|------|
| A flush 发布 3 topic 无订阅者 | ✅ 预期 | A 日志 3× `WARN gossipsub 发布失败 ... NoPeersSubscribedToTopic`（快照仍更新） |
| B 后启动拿到 A 的快照 | ✅ **核心验证** | B TUI Network 面板显示 A 的 4 模型（Qwen3-0.6B[0-27]/30B[0-47]/235B[0-93]/DeepSeek[0-60] 层位图）+ 节点名 A |
| Subscribed 精准重放 | ✅ 隐含验证 | A 的 ConnectionEstablished 补发已删，B 仍拿到全量状态 → 重放生效 |
| mDNS 互发现 / identify | ✅ | 双向连接建立 + identify 协议交换 |
| ConnectionEstablished 不重放 | ✅ | A 日志无连接时发布记录 |

### 双卡推理验证（pipeline）✅ 主链路通过

`session create Qwen3-30B-A3B-Q4_K_M.pgguf` → `session inference pipeline 1 ...` → `api 1`（:8080）→ chat.py：

- 阶段 1-4：模型识别(qwen3moe 50层 ID 2035505870) → 集群发现(1 远程 node-b) → 分配层 [0-49] 给 B → EXEC|pipe_worker ✓
- B 双 GPU：GPU:0 层[0,24] 12.06s / GPU:1 层[25,49] 13.17s 加载完成 → Worker 就绪
- A：Session 1 ML connected → ✓ 流水线就绪
- 第一轮 256 tokens 完整生成（Prefill 0.09s，~0.00s/tok 高速）
- 第二轮 meow~ 上下文记忆 ✅（"...天气吧！meow~~"）
- /clear 清除 ✅（prompt 364 → 25 tokens）
- quit 正常退出 ✅

### ⚠️ 观察到的问题（与 Task 20 无关，推理链路侧，待立项）
1. **think 块超长**：30B Q4_K_M 的 think 块吃掉全部 max_tokens=256，第一轮正文为空——需调大 max_tokens 或加 think 截断
2. **第三轮空回复**：`autoregression start tokens=364` → 立即 EOS（0 token），模型偶发行为
3. **第四轮推理挂起** ~2 分钟（offset 280 处停滞），后自行恢复；API 仍响应
4. **chat_template render failed**：minijinja 报 `string has no method named split (chat:35)` → 回退 qwen3 硬编码模板（前两轮仍正常，但格式受限）
5. **SSE 中文流式显示**：chat.py 逐 token 打印中文出现乱序/重复字样（显示层问题）
6. Qwen3-8B-Q4_K_M.pgguf analyze 失败（预存，文件损坏，wb_18 已记录）

## 单卡 3 节点实测（2026-08-08，pipeline_coord_single）✅ 全部通过

启动顺序 A(13:38:30) → B/C(13:38:59)，每节点 1 GPU：

| 验证项 | 结果 | 证据 |
|--------|------|------|
| 3 节点互发现 | ✅ | 每节点发现另外 2 节点 + identify |
| 快照传播至第 3 节点 | ✅ | C `dp` 显示 node-b 3 模型含层位图（235B[0-93]/30B[0-47]/0.6B[0-27]）+ name |
| 层分配 | ✅ | node-b 层 [0-24]，C 层 [25-49]（按 peer_id 排序均分 50 层） |
| Worker 加载 | ✅ | B cuda:0 13.46s / C cuda:0 14.51s |
| 张量流桥接 | ✅ | fwd: → node-b ✓ bwd: ← C ✓ Session ↔ coord ✓ → 流水线就绪 |
| 第一轮回复 | ✅ | "Hello Meow~"（think 块 + 正文带 meow~，288 tokens） |
| 第二轮上下文记忆 | ✅ | 量子力学总结以 "meow~" 结尾，think 块也记得指令 |
| /clear 清除 | ✅ | 颜色问题回复完全不带 meow~（"...背后的故事呢！😊"） |
| Prefill | ✅ | C 日志 Prefill FWD=0.17s 总=0.23s |
| quit 退出 ×3 | ✅ | 全部进程退出（进程数 0） |

备注：本次 chat.py 用 `--max-tokens 512`，think 块未吃满额度，正文正常输出——验证了双卡测试中"正文为空"为 max_tokens 不足导致。

## 状态
- ✅ **Task 20 全部完成**：实施 + 编译通过 + 双卡 2 节点 + 单卡 3 节点实测全部通过（快照机制为核心验证）
- 待办：上述推理侧问题立项评估（think 块超长/空回复/挂起/chat_template split）
- 结束: 2026-08-08
