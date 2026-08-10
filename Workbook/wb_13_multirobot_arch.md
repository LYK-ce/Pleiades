# Workbook — Task 13: Multi-Robot Control

> 对应任务：`Task/task_13_multirobot_arch.md`
> 创建日期：2026-08-10

---

## 2026-08-10 任务创建 + Task 10/11/12 归档

**归档清单**：
- `Task/archived task/`：task_10_robot_info_handle.md、task_11_robot_update.md、task_12_broadcast.md
- `Workbook/archived workbook/`：wb_10、wb_12（wb_11 不存在）
- `robot_review_problem.md` 保留为唯一问题池（未归档）

**Task 10/11/12 终态**：
- Task 10（Robot Info Handle）：待启动即归档——目标方向（入站位姿存储/消费）由 Task 13 承接
- Task 11（Robot Update）：进行中即归档——A~E 方向中与多车相关者由 Task 13 承接，其余留问题池
- Task 12（gossipsub 广播）：✅ 完成（commit 7a363d5）——遗留双车联调未做

---

## 2026-08-10 阶段一定案：sysid → 完整 peer_id（协议调整）

**讨论过程摘要**（人类逐项确认）：
1. 架构梳理（子 agent）：单车架构全景 + gossipsub 入站链路已通但"仅打印"；4 个多车断层（D* 动态障碍接口/入站消费路径/gossip map 无全量/入站丢 author）
2. state_notifier 与 slam_task 合并讨论 → **不合并**（tokio task 轻量，非性能瓶颈；10Hz pose 实时性不能与重计算 SLAM 绑定；改走"锁外组包 + 共享发布逻辑"）
3. sysid 身份讨论：MAVLink sysid=编址非身份；我们拿它当身份 → 1 字节不够（1/256 碰撞）
4. 决策：**sysid 扩展为完整 peer_id**（u8 长度前缀 + 变长 bytes，方案 A）；带宽影响可忽略（libp2p Signed 已背 34B from + 64B 签名）
5. 协议设计对比：gossipsub ≈ 轻量 DDS topic（已仿照）；msgid 双保险（WS 链路无 topic 层，msgid 必须保留）
6. 子 agent 全量排查 sysid 涉及点：核心 frame.rs（10B 头→变长 12+N），9 处 encode 调用，6 处 decode，5 组测试，orion_protocol.md §2/§4

**定案要点**（详见 task_13 文档"阶段一"）：
- 新帧布局：magic|len|seq|sysid_len(1)|sysid(N)|compid|msgid|payload|checksum
- WS 上行：sysid_len=0 + compid=200（启用 COMPID_GROUND_STATION）；下行填本车 peer_id
- 顺带修：robot.rs launch 时取一次 peer_id 传闭包（问题池 P3#9）
- 保持 protocol 模块不依赖 libp2p（&[u8] 接口）
- 风险：frame.rs 硬编码偏移、长度校验动态化、日志 hex 格式化、SnapshotCache 旧帧、Pictor 外部仓库同步

**待实施**：步骤见 task_13 文档（frame.rs → sysid.rs → robot.rs → bootstrap/WS → 测试 → 编译 → 文档 → 联调）

## 依赖关系

- 前置：gossip 基础设施（ML_review 已 merge：bd0ec43/387ad72）
- 边界：本阶段全部改动在 `Src/Robot/` + `Src/WebSocket/` + `Src/bootstrap.rs`（本分支职责内）；不碰 Network/EventBus
- 跨仓库：Pictor（GodotProject）帧解析需同步升级
