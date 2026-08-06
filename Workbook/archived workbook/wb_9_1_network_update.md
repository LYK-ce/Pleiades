# Workbook — Task 9_1: Network Update

> 对应任务：`Task/task_9_1_network_update.md`
> 创建日期：2026-08-06

---

## 2026-08-06 实施完成（S1~S6）

**目标**：为主枝 Network 增加三个能力——robot_bus（独立事件总线）、DataType::Robot、Broadcast。

**改动文件**（6 个，全部在主枝范围）：
1. `Src/Network/Request_Response/codec.rs`：`DataType::Robot = 3`（补 3 空缺）+ `From_U8` 加臂
2. `Src/Network/node_handle.rs`：`NodeCommand::Broadcast { data_type, payload }` 变体 + `NodeHandle::Broadcast` 方法（try_send 立即返回，fire-and-forget）
3. `Src/Network/command_handler.rs`：`Handle_Command` 加 Broadcast 臂——`Get_All_Peers().await` 遍历（跳过 local）→ 逐个 `send_request`（不注册 oneshot）
4. `Src/Network/network_service.rs`：`Network_Service` 加 `robot_bus: Arc<EventBus>` 字段 + `Init` 签名加参数 + Self 构造
5. `Src/Network/swarm_events.rs`：`Handle_Request_Response_Event` 分流加 Robot 臂——`robot_bus.Publish(Bus_Event::Stream{payload: 原样})` + 回 "OK" 闭合协议状态机（不走 Core/inbound）
6. `Src/main.rs`：创建 `robot_bus = EventBus::New(1024)` + `Network_Service::Init` 传参

**关键决策**（与人类确认）：
- robot_bus 直接加参数（EventBus 核心不动、不数组化）
- 入站走 Info 模式（绕过 Core，防高频污染主循环）
- 回 OK 是协议必需（fire-and-forget 只保发送侧 API 不等；不回 → 发送方 30s 超时风暴 + error! 日志）
- payload 原样转发（序列化/解析是上层职责，协议格式归 Task 9）
- Broadcast 在事件循环层遍历 peers（NodeHandle 不需要 peer 列表）

**验证**：
- `cargo check` ✅ 无新增 warning（unused imports 均为原有）
- `cargo test --lib network` 4 passed ✅
- `cargo test --lib`：148 passed / 1 failed——**`vm::engine::tests::test_sandbox_os_blocked` 为原有失败**（VM 沙箱，与本次改动无关，engine.rs:55 "os 未被正确禁用"）
- `cargo build --bin Pleiades --bin orion-robot` ✅

**待办**：
- Task 9（Robot 侧）联调：Robot::launch 注入 `Arc<NodeHandle>` + robot_bus；state_notifier/slam_task 顺手广播；main_loop 订阅 robot_bus 处理入站
- payload 协议定义（位姿/地图 JSON，含 peer_id）
- 实车/多节点验证
- 可顺带修：VM 沙箱失败测试（非本任务范围）
