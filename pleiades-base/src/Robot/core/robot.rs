//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-10-27

//! Robot 主循环（Task 23 阶段 C：base 骨架 + DeviceHandler trait）
//!
//! - `Robot::new()` 创建共享状态 + 非设备 task（不含设备 spawn / goal_service / 主循环）
//! - `Robot::run(device)` 跑主循环骨架，设备操作走 `DeviceHandler` trait
//! - 设备端（ugv/uav）各自实现 `DeviceHandler`，装配后传入

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use async_trait::async_trait;

use super::behavior_tree::FailCode;
use super::command::{AutoCmd, Command, ManualCmd, ModeCmd};
use super::mission_queue::{MissionQueue, QueuedTask, TaskId};
use super::mode::OpMode;
use crate::robot::core::state::{ExecuteState, RobotState};
use crate::robot::core::grid::OccupancyGrid;
use crate::robot::core::map_delta::MapDelta;

use crate::event_bus::EventBus;
use crate::robot::core::cluster::{cluster_consumer, cluster_table_cleaner, ClusterInfoTable};
use crate::robot::core::command_consumer::command_consumer;
use crate::network::{NodeHandle, TOPIC_ROBOT_POSE};
use crate::robot::core::protocol::{
    encode_frame, encode_pose,
    COMPID_ROBOT, MSGID_POSE, PoseData,
};

/// 决策循环节拍（毫秒）：`main_loop` 用它调度，设备端用它填 `Frame.dt_ms`
pub const AUTO_TICK_MS: u64 = 50;

/// 位姿广播消息
#[derive(Debug, Clone)]
pub struct Pose {
    pub time_boot_ms: u32,
    pub x: f32, pub y: f32, pub z: f32,
    pub yaw: f32,
    pub vx: f32, pub vy: f32,
    /// 本车意图：D* 寻路下一格（Task 13_1，WS 下行带）
    pub sub_target: Option<(i32, i32)>,
}

/// 设备 tick 的结果（Task 36 §12.4.3）
///
/// - `Running`：任务还在跑（或本拍无事可做）
/// - `Terminated(None)`：树给了成功终值
/// - `Terminated(Some(code))`：树给了失败终值 + 原因码（§十一 #31）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    Running,
    Terminated(Option<FailCode>),
}

/// Robot — 机器人系统中枢（base：只含共享状态，不含设备）
pub struct Robot {
    pub robot_cmd_tx: mpsc::Sender<Command>,
    pub robot_state: Arc<RwLock<RobotState>>,
    /// 占据栅格地图（SLAM task 写，WebSocket / 外部读）
    pub grid: Arc<RwLock<OccupancyGrid>>,
    /// 位姿广播
    pub pose_tx: broadcast::Sender<Pose>,
    /// 地图增量广播
    pub map_tx: broadcast::Sender<Vec<MapDelta>>,
    pub op_mode: Arc<RwLock<OpMode>>,
    pub mission_queue: Arc<RwLock<MissionQueue>>,
    /// 集群信息表（Task 13_1：远端车状态，consumer 写 / 未来消费端读）
    pub cluster_table: Arc<ClusterInfoTable>,
    /// 执行器状态（main_loop 唯一写，state_notifier 读）
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
}

/// 设备命令处理器（Task 23 阶段 C：base 定义接口，设备端各自实现，
/// 避免 base 反向依赖 ugv/uav 的具体设备类型）
#[async_trait]
pub trait DeviceHandler: Send + Sync {
    /// 启动设备（内部 spawn 自己需要的设备：车 spawn stm32+lidar，机 spawn mavlink）
    async fn start(&self) -> Result<(), String>;
    /// 处理手动命令（设备自己解析支持的命令子集）
    async fn handle_manual_cmd(&self, cmd: &ManualCmd);
    /// 任务重置（停车 + 清 goal + 清树实例 + 清意图；模式切换 / 任务替换 / 建树失败时调用）
    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>);
    /// 任务挂起（`preempt` 用，§十一 #38 / §12.8.2 第 2 条）：**不停树、不清实例表**。
    ///
    /// 设备只要做三件事：停车 → 清 nav 目标 → 清执行意图。
    /// 树实例与节点进度（RUNNING 位置 / 重试计数 / 计时）原样保留，恢复时继续 tick。
    ///
    /// 与 [`DeviceHandler::reset`] 的分界：`reset` 丢弃实例（`replace` / 切模式），
    /// `suspend` 保留实例（`preempt`）。叶子在 `nav.status() == Idle` 时会重新 `set_goal`，
    /// 所以"清 nav"不会让恢复后的树空转（§十一 #38 的配套要求）。
    async fn suspend(&self, execute_state: &Arc<RwLock<ExecuteState>>);
    /// 任务开始：为该任务建树。
    ///
    /// **幂等**：该 `TaskId` 已有实例（抢占恢复）→ 只激活、不重建（§十一 #34 / #36）。
    async fn build_task(&self, task: &QueuedTask) -> Result<(), String>;
    /// 50 ms 决策循环（急停 → 树 → 决策 → 终值，设备端完整实现）
    ///
    /// 返回 `Terminated` 时 `main_loop` 才会 `pop_next`（唯一出队口，§十一 #16）。
    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) -> TickOutcome;
    /// 停车
    fn stop(&self);
    /// 关设备（停车 + 停雷达 + 关设备）
    async fn shutdown(&self);
}

impl Robot {
    /// 创建共享状态 + 非设备 task，返回 (Self, 命令接收通道)。
    ///
    /// 设备装配（stm32/lidar/mavlink + goal_service + 决策器）下沉到设备端，
    /// 由 ugv/uav 各自的 bootstrap 完成。
    pub async fn new(
        origin: (f32, f32, f32),
        node_handle: Option<Arc<NodeHandle>>,
        robot_bus: Option<Arc<EventBus>>,
        robot_cmd_frame_rx: Option<mpsc::Receiver<Vec<u8>>>,
        peer_name: String,
    ) -> Result<(Self, mpsc::Receiver<Command>), String> {
        let cancel = CancellationToken::new();

        // 1. 创建共享状态
        let robot_state = Arc::new(RwLock::new(RobotState::default()));
        // 立即注入初始世界坐标，关闭 (0,0) 初始化窗口（首帧 RX 覆盖前消费方可见）
        {
            let mut g = robot_state.write().await;
            g.x = origin.0;
            g.y = origin.1;
            g.z = origin.2;
        }

        // 2. 广播通道
        let (pose_tx, _) = broadcast::channel::<Pose>(32);
        let (map_tx, _) = broadcast::channel::<Vec<MapDelta>>(32);
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));

        // 3. 运行模式 + 任务队列
        let op_mode = Arc::new(RwLock::new(OpMode::default()));
        let mission_queue = Arc::new(RwLock::new(MissionQueue::default()));

        // 3.1 执行器状态（main_loop 唯一写，state_notifier 读）
        let execute_state = Arc::new(RwLock::new(ExecuteState::default()));

        // 3.2 集群信息表（Task 13_1：入站 POSE → 表）
        let cluster_table = Arc::new(ClusterInfoTable::new());

        // 5. spawn state_notifier
        let notifier_cancel = cancel.clone();
        let notifier_state = robot_state.clone();
        let notifier_tx = pose_tx.clone();
        let notifier_handle = node_handle.clone();
        let notifier_name = peer_name.clone();
        // Task 13 阶段一：peer_id 为常量，launch 时算一次传闭包（顺带修 P3#9 每拍重算）
        let notifier_peer_id = node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes());
        // Task 13_1：execute_state 供组帧带意图
        let notifier_exec = execute_state.clone();
        tokio::spawn(async move {
            state_notifier(notifier_state, notifier_tx, notifier_handle, notifier_name, notifier_peer_id, notifier_exec, notifier_cancel).await;
        });

        // 7. 集群入站消费者（Task 13_1：robot_bus → ClusterInfo 表，独立 task 数据面）
        let consumer_bus = robot_bus.clone();
        let consumer_peer_id = node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes()).unwrap_or_default();
        let consumer_table = cluster_table.clone();
        // Task 13_2：入站 MAP_DELTA 应用需要 grid（只写 merged）
        let consumer_grid = grid.clone();
        let consumer_cancel = cancel.clone();
        tokio::spawn(async move {
            cluster_consumer(consumer_bus, consumer_peer_id, consumer_table, consumer_grid, consumer_cancel).await;
        });

        // 7.1 集群表维护（周期清理失联车：2s 周期 / 2s 超时，Task 15）
        let cleaner_table = cluster_table.clone();
        let cleaner_cancel = cancel.clone();
        tokio::spawn(async move {
            cluster_table_cleaner(cleaner_table, cleaner_cancel).await;
        });

        // 8. 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(32);

        // 8.1 命令入站消费者（request-response 命令帧 → Command，Task 16）
        if let Some(cmd_frame_rx) = robot_cmd_frame_rx {
            let consumer_tx = cmd_tx.clone();
            let consumer_cancel = cancel.clone();
            tokio::spawn(async move {
                command_consumer(cmd_frame_rx, consumer_tx, consumer_cancel).await;
            });
        }

        Ok((Self {
            robot_cmd_tx: cmd_tx,
            robot_state,
            grid,
            pose_tx,
            map_tx,
            op_mode,
            mission_queue,
            cluster_table,
            execute_state,
            cancel,
        }, cmd_rx))
    }

    /// 跑主循环（阻塞直到退出）。`device` 由设备端装配后传入。
    pub async fn run(&self, device: Arc<dyn DeviceHandler>, cmd_rx: mpsc::Receiver<Command>) {
        main_loop(
            device,
            cmd_rx,
            self.op_mode.clone(),
            self.mission_queue.clone(),
            self.robot_state.clone(),
            self.execute_state.clone(),
            self.cancel.clone(),
        ).await;
    }

    /// 优雅退出
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

// ============================================================
// state_notifier — 位姿广播 (100ms)
// ============================================================

async fn state_notifier(
    state: Arc<RwLock<RobotState>>,
    pose_tx: broadcast::Sender<Pose>,
    node_handle: Option<Arc<NodeHandle>>,
    _peer_name: String,
    local_peer_id: Option<Vec<u8>>,
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        select! {
            _ = interval.tick() => {
                // 先取快照并立即 drop 读锁（Task 23 review 修复：持锁跨 await 会挤压设备端
                // RX 回调的 try_write，造成位姿抖动/滞后）
                let (x, y, z, yaw, vx, vy, rtk_fixed, sub_target) = {
                    let s = state.read().await;
                    let es = execute_state.read().await;
                    (s.x, s.y, s.z, s.attitude.yaw, s.vx, s.vy, s.rtk_fixed, es.sub_target)
                };
                let time_boot_ms = crate::robot::core::protocol::now_boot_ms();
                let _ = pose_tx.send(Pose {
                    time_boot_ms,
                    x, y, z, yaw, vx, vy,
                    sub_target,
                });

                // Task 9_2：位姿广播到集群（fire-and-forget，Network 只搬运字节）
                if let (Some(nh), Some(peer_id)) = (&node_handle, &local_peer_id) {
                    let pose = PoseData {
                        time_boot_ms,
                        x, y, z,
                        vx, vy,
                        yaw,
                        valid: sub_target.is_some(),
                        sub_gx: sub_target.map(|(gx, _)| gx).unwrap_or(0),
                        sub_gy: sub_target.map(|(_, gy)| gy).unwrap_or(0),
                        rtk_fixed,
                    };
                    let frame = encode_frame(MSGID_POSE, peer_id, COMPID_ROBOT, &encode_pose(&pose));
                    if let Err(e) = nh.Gossipsub_Publish(TOPIC_ROBOT_POSE, frame).await {
                        warn!("[Robot] 位姿广播失败: {e}");
                    }
                }
            }
            _ = cancel.cancelled() => {
                info!("state_notifier 退出");
                return;
            }
        }
    }
}

// ============================================================
// 主 select! 循环（单点执行，纯骨架）
// ============================================================

async fn main_loop(
    device: Arc<dyn DeviceHandler>,
    mut cmd_rx: mpsc::Receiver<Command>,
    op_mode: Arc<RwLock<OpMode>>,
    mission_queue: Arc<RwLock<MissionQueue>>,
    robot_state: Arc<RwLock<RobotState>>,
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
) {
    // 统一启动契约：设备自己决定启动哪些硬件
    if let Err(e) = device.start().await {
        warn!("[Robot] 设备启动失败: {e}");
        // 兜底清理：设备 start 内部可能已起部分资源（Task 23 review 修复）
        device.stop();
        device.shutdown().await;
        return;
    }
    info!("Robot 主循环启动（base 骨架 + 设备 handler）");

    let auto_tick_ms = AUTO_TICK_MS;
    let mut next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
    // 当前任务的 TaskId（base 本地状态；设备端不碰队列）
    let mut current_task_id: Option<TaskId> = None;

    loop {
        select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Command::Mode(m)) => {
                        match m {
                            ModeCmd::SwitchToManual => {
                                info!("[Robot] 切换到 Manual 模式");
                                *op_mode.write().await = OpMode::Manual;
                                mission_queue.write().await.clear();
                                device.reset(&execute_state).await;
                                current_task_id = None;
                            }
                            ModeCmd::SwitchToAuto => {
                                info!("[Robot] 切换到 Auto 模式");
                                *op_mode.write().await = OpMode::Auto;
                                device.reset(&execute_state).await;
                                current_task_id = None;
                                next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
                            }
                        }
                    }

                    Some(Command::Manual(m)) => {
                        if *op_mode.read().await == OpMode::Manual {
                            device.handle_manual_cmd(&m).await;
                        } else {
                            warn!("[Robot] 忽略 Manual 命令：当前为 Auto 模式");
                        }
                    }

                    Some(Command::Auto(a)) => {
                        if *op_mode.read().await == OpMode::Auto {
                            // 队列操作（同步、不持锁跨 await）→ 再按返回的动作叫设备
                            let follow = {
                                let mut q = mission_queue.write().await;
                                apply_auto_cmd(&mut q, a).1
                            };
                            match follow {
                                AutoFollowUp::Reset => {
                                    device.reset(&execute_state).await;
                                    current_task_id = None;
                                }
                                AutoFollowUp::Suspend => {
                                    // 队首已换成新任务 → 下一拍 auto_tick 会 build_task（幂等）
                                    device.suspend(&execute_state).await;
                                    current_task_id = None;
                                }
                                AutoFollowUp::None => {}
                            }
                        } else {
                            warn!("[Robot] 忽略 Auto 命令：当前为 Manual 模式");
                        }
                    }

                    None => {
                        info!("命令通道关闭，Robot 退出");
                        break;
                    }
                }
            }

            // auto_tick：仅 Auto 模式激活（决策循环下沉到设备端）
            _ = tokio::time::sleep_until(next_tick.into()), if *op_mode.read().await == OpMode::Auto => {
                auto_tick(&device, &mission_queue, &robot_state, &execute_state, &mut current_task_id).await;

                next_tick += Duration::from_millis(auto_tick_ms);
                if next_tick <= Instant::now() {
                    next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
                }
            }

            _ = cancel.cancelled() => {
                info!("Robot 收到退出信号");
                break;
            }
        }
    }

    // 退出前：停车 → 关设备
    info!("正在停止机器人...");
    device.stop();
    device.shutdown().await;
    info!("Robot 主循环已退出");
}

// ============================================================
// AutoCmd → 队列操作 + 设备后续动作（纯函数，便于单测）
// ============================================================

/// 队列操作做完之后，base 还要对设备做什么
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoFollowUp {
    /// 追加队尾：设备一行不动（当前任务继续跑）
    None,
    /// 整体替换 / 取消：`reset()`（halt 树 + 清实例表 + 停车）
    Reset,
    /// 抢占：`suspend()`（不停树 + 保留实例表 + 停车）
    Suspend,
}

/// 执行一条 `AutoCmd`：**只动队列**，返回（队列条数，设备后续动作）。
///
/// `main_loop` 拿到返回值后再去 await 设备 —— 这样"哪种命令配哪种设备动作"是纯函数，
/// 不会被 `main_loop` 的 select 结构藏起来（§11 #38 的分界就靠这张表）。
fn apply_auto_cmd(queue: &mut MissionQueue, cmd: AutoCmd) -> (usize, AutoFollowUp) {
    match cmd {
        AutoCmd::Set(list) => {
            let n = list.len();
            info!("[Robot] Auto 任务队列已替换（{n} 条）");
            queue.replace(list);
            (n, AutoFollowUp::Reset)
        }
        AutoCmd::Append(list) => {
            let n = list.len();
            info!("[Robot] Auto 任务已追加队尾（{n} 条，当前任务不受影响）");
            queue.append(list);
            (n, AutoFollowUp::None)
        }
        AutoCmd::Preempt(list) => {
            let n = list.len();
            info!("[Robot] Auto 任务已抢占队头（{n} 条，当前任务挂起待恢复）");
            queue.preempt(list);
            (n, AutoFollowUp::Suspend)
        }
    }
}

// ============================================================
// auto_tick — 一拍自动决策（建树 + 设备 tick + 出队）
// ============================================================

/// 一拍自动决策：base 负责「发现队首变化 → 建树」，设备负责「tick → 终值」（§十一 #36）
///
/// 出队只有一条路径：`on_tick` 返回 `Terminated` → 弹出队首（§十一 #16）。
async fn auto_tick(
    device: &Arc<dyn DeviceHandler>,
    mission_queue: &Arc<RwLock<MissionQueue>>,
    robot_state: &Arc<RwLock<RobotState>>,
    execute_state: &Arc<RwLock<ExecuteState>>,
    current_task_id: &mut Option<TaskId>,
) {
    // ① 队首变化 → build_task（队首不离队；同 id 不重复建，§十一 #36）
    let head = mission_queue.read().await.head().cloned();
    match head {
        Some(task) if Some(task.id) != *current_task_id => {
            match device.build_task(&task).await {
                Ok(()) => {
                    info!("[Robot] 开始任务 {:?}：{:?}", task.id, task.mission);
                    *current_task_id = Some(task.id);
                }
                Err(e) => {
                    // 建树失败 = 停住待命（§十一 #35）：清队列 + 停车，不拿后面的任务去试
                    warn!("[Robot] 建树失败（任务 {:?}）：{e}；清空队列 + 停车待命", task.id);
                    mission_queue.write().await.clear();
                    device.reset(execute_state).await;
                    *current_task_id = None;
                }
            }
        }
        Some(_) => {}
        // 队列空了：复位「当前任务」，等下一条命令重新建树
        None => *current_task_id = None,
    }

    // ② 设备 tick（急停 → 树 → 决策 → 终值，全在设备端）
    let rs = robot_state.read().await.clone();
    match device.on_tick(&rs, execute_state).await {
        TickOutcome::Running => {}
        TickOutcome::Terminated(code) => {
            match code {
                None => info!("[Robot] 任务完成"),
                Some(c) => warn!("[Robot] 任务失败，原因码 {}", c.code()),
            }
            // 唯一出队口（§十一 #16）
            let _ = mission_queue.write().await.pop_next();
            *current_task_id = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::robot::core::mission_queue::Mission;
    use crate::robot::core::state::DecisionState;

    /// 记录 build / reset 调用次数的假设备（`on_tick` 返回脚本化终值）
    struct MockHandler {
        build_calls: AtomicU32,
        fail_build: bool,
        built_ids: std::sync::Mutex<Vec<TaskId>>,
        reset_calls: AtomicU32,
        suspend_calls: AtomicU32,
        outcome: TickOutcome,
    }

    impl MockHandler {
        fn new(outcome: TickOutcome, fail_build: bool) -> Self {
            Self {
                build_calls: AtomicU32::new(0),
                fail_build,
                built_ids: std::sync::Mutex::new(Vec::new()),
                reset_calls: AtomicU32::new(0),
                suspend_calls: AtomicU32::new(0),
                outcome,
            }
        }
    }

    #[async_trait]
    impl DeviceHandler for MockHandler {
        async fn start(&self) -> Result<(), String> {
            Ok(())
        }
        async fn handle_manual_cmd(&self, _cmd: &ManualCmd) {}
        async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
            self.reset_calls.fetch_add(1, Ordering::SeqCst);
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        }
        async fn suspend(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
            self.suspend_calls.fetch_add(1, Ordering::SeqCst);
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        }
        async fn build_task(&self, task: &QueuedTask) -> Result<(), String> {
            self.build_calls.fetch_add(1, Ordering::SeqCst);
            self.built_ids.lock().unwrap().push(task.id);
            if self.fail_build { Err("mock build failed".to_string()) } else { Ok(()) }
        }
        async fn on_tick(&self, _rs: &RobotState, _es: &Arc<RwLock<ExecuteState>>) -> TickOutcome {
            self.outcome
        }
        fn stop(&self) {}
        async fn shutdown(&self) {}
    }

    fn goto(x: f32) -> Mission {
        Mission::Goto { x, y: 0.0 }
    }

    /// 建树所需的三个共享状态（每次新建，测试之间不共享）
    fn ctx() -> (Arc<MockHandler>, Arc<dyn DeviceHandler>, Arc<RwLock<MissionQueue>>, Arc<RwLock<RobotState>>, Arc<RwLock<ExecuteState>>) {
        let mock = Arc::new(MockHandler::new(TickOutcome::Running, false));
        let dev: Arc<dyn DeviceHandler> = mock.clone();
        (
            mock,
            dev,
            Arc::new(RwLock::new(MissionQueue::default())),
            Arc::new(RwLock::new(RobotState::default())),
            Arc::new(RwLock::new(ExecuteState::default())),
        )
    }

    /// #9 建树时机：队首 id 变化才调 `build_task`，同 id 不重复调
    #[tokio::test]
    async fn test_build_task_only_when_head_changes() {
        let (mock, dev, mq, rs, es) = ctx();
        let mut current: Option<TaskId> = None;
        mq.write().await.replace(vec![goto(1.0), goto(2.0)]);

        auto_tick(&dev, &mq, &rs, &es, &mut current).await;
        assert_eq!(mock.build_calls.load(Ordering::SeqCst), 1, "首次 auto_tick 应建树");
        let first = mq.read().await.head().map(|t| t.id).expect("队首存在");
        assert_eq!(current, Some(first), "current_task_id 应为队首 id");

        // 同一 id 再来一拍：不重复建
        auto_tick(&dev, &mq, &rs, &es, &mut current).await;
        assert_eq!(mock.build_calls.load(Ordering::SeqCst), 1, "同 id 不应重复建树");

        // 出队后队首换 id → 再建一次
        let _ = mq.write().await.pop_next();
        auto_tick(&dev, &mq, &rs, &es, &mut current).await;
        assert_eq!(mock.build_calls.load(Ordering::SeqCst), 2, "队首 id 变化应重新建树");

        let ids = mock.built_ids.lock().unwrap().clone();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "两次建的应是不同任务");
        assert_eq!(ids[1], current.expect("第二个任务已激活"));
    }

    /// #10 出队：只有 `Terminated` 才 `pop_next` 一次；`Running` 不出队
    #[tokio::test]
    async fn test_pop_next_only_on_terminated() {
        // 终值分支：两条任务，逐拍各出队一次
        let mock = Arc::new(MockHandler::new(TickOutcome::Terminated(None), false));
        let dev: Arc<dyn DeviceHandler> = mock.clone();
        let mq = Arc::new(RwLock::new(MissionQueue::default()));
        let rs = Arc::new(RwLock::new(RobotState::default()));
        let es = Arc::new(RwLock::new(ExecuteState::default()));
        let mut current: Option<TaskId> = None;
        mq.write().await.replace(vec![goto(1.0), goto(2.0)]);

        auto_tick(&dev, &mq, &rs, &es, &mut current).await;
        assert_eq!(mq.read().await.len(), 1, "一次 Terminated 只出队一条");
        assert_eq!(current, None, "出队后 current_task_id 复位");

        auto_tick(&dev, &mq, &rs, &es, &mut current).await;
        assert_eq!(mq.read().await.len(), 0);

        // 队列已空：不再建树，也不 panic
        auto_tick(&dev, &mq, &rs, &es, &mut current).await;
        assert_eq!(mock.build_calls.load(Ordering::SeqCst), 2, "空队列不应再建树");
        assert_eq!(mq.read().await.len(), 0);

        // Running 分支：不出队
        let (mock2, dev2, mq2, rs2, es2) = ctx();
        let mut current2: Option<TaskId> = None;
        mq2.write().await.replace(vec![goto(1.0), goto(2.0)]);
        auto_tick(&dev2, &mq2, &rs2, &es2, &mut current2).await;
        assert_eq!(mq2.read().await.len(), 2, "Running 不出队");
        assert!(current2.is_some());
        assert_eq!(mock2.build_calls.load(Ordering::SeqCst), 1);
    }

    /// #11 建树失败：清空队列 + `reset` 一次 + 当前任务复位
    #[tokio::test]
    async fn test_build_failure_clears_queue_and_resets() {
        let mock = Arc::new(MockHandler::new(TickOutcome::Running, true));
        let dev: Arc<dyn DeviceHandler> = mock.clone();
        let mq = Arc::new(RwLock::new(MissionQueue::default()));
        let rs = Arc::new(RwLock::new(RobotState::default()));
        let es = Arc::new(RwLock::new(ExecuteState::default()));
        let mut current: Option<TaskId> = Some(TaskId(99)); // 故意留个旧值，验证会被复位
        mq.write().await.replace(vec![goto(1.0), goto(2.0)]);

        auto_tick(&dev, &mq, &rs, &es, &mut current).await;

        assert!(mq.read().await.is_empty(), "建树失败应清空队列（不拿后面的任务去试）");
        assert_eq!(mock.reset_calls.load(Ordering::SeqCst), 1, "建树失败应 reset 一次");
        assert_eq!(current, None, "建树失败应复位当前任务");
        assert_eq!(mock.build_calls.load(Ordering::SeqCst), 1, "失败后不再继续建树");
    }

    /// #12 `Set` → 整队替换 + `Reset`（丢弃、不恢复）
    #[tokio::test]
    async fn test_apply_auto_cmd_set_replaces_and_resets() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0), goto(2.0)]);
        let (n, follow) = apply_auto_cmd(&mut q, AutoCmd::Set(vec![goto(3.0)]));
        assert_eq!(n, 1);
        assert_eq!(follow, AutoFollowUp::Reset);
        assert_eq!(q.len(), 1, "Set 必须整队替换");
    }

    /// #13 `Append` → 队尾追加 + `None`（设备一行不动，队首 id 不变）
    #[tokio::test]
    async fn test_apply_auto_cmd_append_keeps_head() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0)]);
        let head_before = q.head().map(|t| t.id);
        let (n, follow) = apply_auto_cmd(&mut q, AutoCmd::Append(vec![goto(2.0)]));
        assert_eq!(n, 1);
        assert_eq!(follow, AutoFollowUp::None, "append 不得触发 reset/suspend");
        assert_eq!(q.head().map(|t| t.id), head_before, "队首不变 = 当前任务不受影响");
        assert_eq!(q.len(), 2);
    }

    /// #14 `Preempt` → 插队头 + `Suspend`（当前任务退到第二位、仍留在队列里）
    #[tokio::test]
    async fn test_apply_auto_cmd_preempt_suspends_and_moves_head() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0)]);
        let old_head = q.head().map(|t| t.id).expect("队首");
        let (n, follow) = apply_auto_cmd(&mut q, AutoCmd::Preempt(vec![goto(9.0)]));
        assert_eq!(n, 1);
        assert_eq!(follow, AutoFollowUp::Suspend, "preempt 必须走 suspend 而不是 reset");
        assert_eq!(q.len(), 2, "被抢占任务**留在队列里**（可恢复）");
        assert_ne!(q.head().map(|t| t.id), Some(old_head), "队首必须换成新任务");

        // 顺带卡死：`Suspend` 这条后续动作确实调设备 `suspend`、且**不**调 `reset`（§十一 #38 分界）
        let (mock, dev, _mq, _rs, es) = ctx();
        dev.suspend(&es).await;
        assert_eq!(mock.suspend_calls.load(Ordering::SeqCst), 1, "suspend 必须调到设备");
        assert_eq!(mock.reset_calls.load(Ordering::SeqCst), 0, "suspend 不得走 reset");
    }

    /// #15 `Set(vec![])` → 取消 + `Reset`（队列空、head 为空）
    #[tokio::test]
    async fn test_apply_auto_cmd_empty_set_is_cancel() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0)]);
        let (n, follow) = apply_auto_cmd(&mut q, AutoCmd::Set(vec![]));
        assert_eq!(n, 0);
        assert_eq!(follow, AutoFollowUp::Reset);
        assert!(q.is_empty());
        assert!(q.head().is_none());
    }
}
