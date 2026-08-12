# Workbook — Task 13_2: CRDT 地图

> 对应任务：`Task/task_13_2_crdt_map.md`
> 创建日期：2026-08-10

---

## 2026-08-10 任务创建（骨架）

**背景**：Task 13_1 人类指示 MAP_DELTA 走 CRDT 地图重构（挂起）。人类要求建 task_13_2 空文档 + 纳入 robot_bus 容量问题。

**事实核查**（人类要求扩容，核查后发现）：
- `robot_bus` = `bootstrap.rs:112` `EventBus::New(1024)` —— **已是 1024**（此前误记为 32）
- 真正 32 的是 `robot.rs:117-118` 的 `pose_tx`/`map_tx`（本地广播 → WS）
- `EventBus::New` 容量参数化，注释建议 1024（`event_bus.rs:43`）

**动作**：任务文档已建（骨架），容量问题已纳入"已纳入问题"节。扩容目标容量/通道待讨论。

**2026-08-10 容量问题评估 → 移除**：
- 评估结论：三个通道均无触发改动的真实场景（状态型数据可丢中间帧 / 消费快于生产 / 唯一消费者），扩容无实际收益
- 人类决策：**不扩容，从任务文档移除**（task_13_2 已清理为纯骨架）
- 触发条件（未来才需动）：事件型消息（P1 task）加入走 robot_bus、多消费端实测 Lagged、WS 慢消费者频繁掉队
- 事实记录（核查结论，防误记）：`robot_bus`=1024（bootstrap.rs:112）、`pose_tx`/`map_tx`=32（robot.rs:117-118）

---

## 2026-08-10 CRDT 地图方案讨论收敛（含调研）

**调研**（子 agent，web）：无 occupancy grid CRDT 直接先例；最接近 = 2026 微机器人 stigmergic SLAM（有界计数器+幂等瓦片合并）；Kimera-Multi 模式 = 共享位姿图约束而非复制网格；δ-CRDT/anti-entropy 是理论模板。报告：`调研报告_分布式地图一致性方案.md`。

**讨论收敛链**（人类逐项拍板）：
1. 目标澄清：多车共同构建同一张地图 = 分布式一致性（非拼接）
2. 坐标系：全局物理世界坐标系（RTK 维持）——无需对齐/回环/epoch（已写入 multi_robot_control.md）
3. 存储不动：i8 有界 log-odds（±8/阈值±6/+3/-1）合理；不做无界、不做衰减/分层（人类否）
4. 广播内容：三态格 → **观测增量 +3/-1，含饱和格**（扫到就发，接收方需补证据）
5. 丢包：有界饱和 = 丢包免疫（最终状态由边界决定）；增量低频丢一期 = 信息延迟，接受
6. 增量历史 vs 全量历史：增量（变化，只加不换，无缓存）vs 全量（累积，需替换，要缓存）——人类最终定案：**平时增量 + 新节点全量**
7. 周期全量方案被否（缓存 N×64KB + 重复累计/震荡格发散）
8. 带宽核算：低频增量 0.7~1.8KB/s/车（~400 格/周期 × 9B）；全量 64KB 仅新节点事件
9. 内存：own 64KB + merged 128KB，无缓存
10. 规模化：空间/任务局部性，非全连接

**定案**：增量历史（低频 2~5s 打包，含饱和格）+ 新节点全量初始化。已写入 task_13_2（决策链表 + 协议改动 + 待决策）。

**实施前置待定**：增量周期值；MAP_FULL 改 log-odds 语义 vs 新 msgid；merged 更新粒度。

---

## 2026-08-12 步骤 4/5 方案收敛（人类逐项拍板）+ 实施开始

**讨论收敛链**：
1. 入站只写 merged(chunk)、own 绝不碰（CRDT 正确性命门：own 是上报数据源，混入他人贡献→终端 Σ 双倍计数）
2. 分工：MAP_DELTA（车↔车 gossip）→ consumer 处理；MAP_FULL（终端→车 WS）→ WS 分支处理
3. **不做全局周期对账**（人类定）：收敛 = 理想情况（无丢包/无重连/同启动）下的增量流完整性；对账后续再补
4. **新车初始化 = 方案二（终端下发全量）**：新车接入终端 → 终端把全局图下发 → decode_map_full → set_log_odds 替换 merged；否决方案一（车对车广播 FULL：无权威、多份冲突、gossipsub 无点对点请求）
5. 车对车 FULL 不需要：启动=入网，own 从 0 开始，Δ 流即完整历史（前提：同时启动 + 连接秒建，晚启动缺历史由终端初始化补）
6. MAP_DELTA 入站 = merged += Δ clamp ±8，越界忽略；广播归 gossipsub，业务层不转发
7. apply_delta 独立 API（不改 update：update 双写 own+chunk 是观测语义）
8. decode_map_full 宽松校验（长度 20+65536 结构性检查；origin 先按 0,0）
9. WS 处理形态 A（就地处理，不进命令通道）；Pictor 改动清单（帧解析升级 4 项 + 全局图维护/下发 2 项）已列入方案

**子 agent 审查修正（2026-08-12）**：4 改动全部可行；采纳 3 修正——① OccupancyGrid::apply_delta 包装（结构性防误写 own）+ 测试断言 own 不变；② FULL origin/size 不符 warn；③ 边界标注（FULL 吞在途 Δ、终端离线数据过期归重连场景）

**已知边界（接受，标注）**：重连重复计数 / 丢包偏差残留 / 终端离线缺历史 / 晚启动车靠终端初始化 / FULL 吞在途 Δ

**实施开始（2026-08-12）**：步骤 4 车端四件套——grid::apply_delta、messages::decode_map_full、consumer MAP_DELTA 分支、server WS 入站 msgid=2。

**实施完成（2026-08-12）**：
- 改动 7 文件：`grid.rs`（Chunk::apply_delta + OccupancyGrid::apply_delta 包装 + 4 测试）、`messages.rs`（decode_map_full + 1 测试）、`protocol/mod.rs`（re-export）、`consumer.rs`（grid 参数 + MAP_DELTA 分支 + 测试改造 6 个）、`robot.rs`（spawn 传 grid）、`WebSocket/protocol.rs`（parse_orion_frame 改收 &Frame，避免 65KB 双解）、`WebSocket/server.rs`（入站 msgid==2 就地处理 + handle_map_full：元数据不符 warn 忽略）
- 验证：cargo check ✅（27 存量警告，无新增）→ cargo test --lib robot **69/69**（原 63 + 6）→ cargo build --release --bin orion-robot ✅
- 文档：orion_protocol.md §3.2（接收语义 + 方向语义 + 总览表）、task_13_2（状态 🟢 + 步骤 4/6/7/8 ✅ + 已知边界 5 项）
- 遗留：Pictor 端同步（帧解析升级 4 项 + 全局图维护/下发 2 项，需人类协调）；步骤 5 终端对账挂起；双车联调

**提交（2026-08-12）**：`1e712a8`（含本次 7 代码 + 3 文档 + multi_robot_control.md 坐标系约定增补），已推送 `origin/Pleiades-Orion`（SSH 交互式密钥）。

**提交后文档补充（2026-08-12）**：task_13_2 补「实施记录（步骤 4）」节 + 待决策点 1/2/3 标记已定；待决策点 5（终端对账细节）随周期对账挂起。
