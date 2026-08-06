# Workbook — Task 9_2: Robot Loop Network 接入

> 对应任务：`Task/task_9_2_robot_loop.md`
> 创建日期：2026-08-06

---

## 2026-08-06 部署形态确认 + main.rs Robot 移除

**部署形态**（人类确认）：
- `orion-robot`（main_robot.rs）→ 装在车上：Robot 控制 + 网络数据面（Task 9_2 注入点）
- `Pleiades`（main.rs）→ 跑在 PC 机：分布式推理系统（纯推理，无 Robot）
- 两个二进制分工，互不干扰

**main.rs Robot 移除**（4 处，约 17 行）：
1. `use pleiades::robot::{CarType, Robot};` 删除
2. `let vehicle_id = peer_name.clone();` 删除（仅 WS 用）
3. Phase 5.6 块（ws_bind / Robot::launch / websocket::start）删除——**硬耦合问题随之消失**（PC 机不再有串口依赖）
4. `robot.shutdown();` 删除

**保留**：
- `robot_bus`（Task 9_1 需要，main.rs:68 创建 + Init 传参 :110）✅
- `Robot_Config`（config.rs，未来配置 robot 用）✅——用户明确要求保留

**注意**：不能用 master 的 main.rs 直接覆盖——master 无 robot_bus（Task 9_1 改动），覆盖会丢且 Init 签名不匹配。正确做法 = 当前 main.rs 删 Robot 部分。

**验证**：`cargo check` ✅；`cargo build --bin Pleiades` ✅；grep 确认 main.rs 无 Robot 引用（仅 robot_bus）

## 待办（Task 9_2 主体）

1. `main_robot.rs`：注入 `Arc<NodeHandle>` + `robot_bus`
2. `state_notifier`：发 pose_tx 同时 `NodeHandle::Broadcast(DataType::Robot, pose_json)`
3. `slam_task`：发 map_tx 同时 broadcast map_delta（有 delta 才发）
4. `main_loop`：订阅 robot_bus 处理入站（存 remote_vehicles / 展示）
5. payload 协议定义（位姿/地图 JSON，含 peer_id）
6. 全量地图发布时机
7. 联调验证（多节点完整系统）
