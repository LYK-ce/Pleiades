//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-26

//! Robot 主循环
//!
//! Robot::launch() 启动所有 Device 并 spawn task：
//! - main_loop: select! 收命令 → dispatch；单点执行（get_path + 写 ExecuteState + 发命令 + 急停仲裁）
//! - state_notifier: 100ms → 读 robot_state → 广播 Pose
//! - SLAM task: 200ms → 读 lidar_state → update grid → 广播 map_delta
//! - Rust 决策器（executor）: 三状态机纯决策（读状态 + next_cell → 返回动作意图，零副作用）

use std::f32::consts::PI;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::command::{AutoCmd, Command, ManualCmd, ModeCmd};
use super::mission::MissionQueue;
use super::mode::OpMode;
use crate::robot::control::device::stm32::STM32Device;
use crate::robot::control::device::lidar::LidarDevice;
use crate::robot::control::device::mavlink::{MavlinkDevice, Telemetry};
use crate::robot::control::types::CarType;
use crate::robot::core::goal::GoalService;
use crate::robot::core::executor::DecisionExecutor;
use crate::robot::core::state::{
    DecisionState, ExecuteState, LidarState, MotionAction, RobotState,
};
use crate::robot::device::MotionDevice;
use crate::robot::slam::{OccupancyGrid, MapDelta, SlamContext, CELL_RESOLUTION};

use crate::event_bus::EventBus;
use crate::robot::core::cluster::{cluster_consumer, cluster_table_cleaner, ClusterInfoTable};
use crate::robot::core::command_consumer::command_consumer;
use crate::robot::world::World;
use crate::network::{NodeHandle, TOPIC_ROBOT_POSE};
use crate::robot::core::protocol::{
    encode_frame, encode_pose,
    COMPID_ROBOT, MSGID_POSE, PoseData,
};

/// 急停距离阈值（米）
const OBSTACLE_THRESHOLD_M: f32 = 0.3;
/// 到达判定阈值（米）
const ARRIVAL_THRESHOLD_M: f32 = 0.3;


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

/// Robot — 机器人系统中枢
pub struct Robot {
    pub robot_cmd_tx: mpsc::Sender<Command>,
    pub robot_state: Arc<RwLock<RobotState>>,
    pub lidar_state: Arc<RwLock<LidarState>>,
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
    cancel: CancellationToken,
}

impl Robot {
    /// 启动 Robot：创建各设备独立状态，spawn Device，启动所有 task
    ///
    /// - `lidar_port` / `lidar_baudrate`: 可选 LiDAR 配置，None 则不启用
    /// - `origin`: 小车初始世界坐标 (x, y, z)（默认 64,64,0），Task 9：RobotState 记录全局唯一坐标
    /// - `node_handle` / `robot_bus`: 网络数据面（Task 9_2），None 则纯本地运行
    /// - `peer_name`: 车名（广播 payload 的 peer_name 字段 + WS vehicle_id）
    pub async fn launch(
        chassis_enabled: bool,
        port: &str,
        baudrate: u32,
        car_type: CarType,
        lidar_enabled: bool,
        lidar_port: Option<&str>,
        lidar_baudrate: Option<u32>,
        flight_ctrl_port: Option<&str>,
        flight_ctrl_baudrate: Option<u32>,
        origin: (f32, f32, f32),
        node_handle: Option<Arc<NodeHandle>>,
        robot_bus: Option<Arc<EventBus>>,
        robot_cmd_frame_rx: Option<mpsc::Receiver<Vec<u8>>>,
        obstacle_inflation_radius: f32,
        forward_speed: i16,
        turn_speed: i16,
        vel_fwd: f32,
        yaw_rate_deg: f32,
        peer_name: String,
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();

        // 1. 创建各设备独立状态
        let robot_state = Arc::new(RwLock::new(RobotState::default()));
        // 立即注入初始世界坐标，关闭 (0,0) 初始化窗口（首帧 RX 覆盖前消费方可见）
        {
            let mut g = robot_state.write().await;
            g.x = origin.0;
            g.y = origin.1;
            g.z = origin.2;
        }
        let lidar_state = Arc::new(RwLock::new(LidarState::default()));

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

        // 3.3 世界模块（Task 22：静态地图 + 动态设备 + 寻路 D*）
        let world = Arc::new(World::new(grid.clone(), cluster_table.clone()));

        // 3.4 目标服务（Task 22：完整目标服务 get_path）
        let goal_service = Arc::new(tokio::sync::Mutex::new(GoalService::new(
            robot_state.clone(),
            mission_queue.clone(),
            node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes()).unwrap_or_default(),
            grid.clone(),
            cluster_table.clone(),
            world.clone(),
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        )));

        // 4. spawn 底盘设备（chassis 开关）
        let stm32: Option<Arc<STM32Device>> = if chassis_enabled {
            Some(Arc::new(STM32Device::spawn(port, baudrate, car_type, robot_state.clone(), origin, forward_speed, turn_speed)?))
        } else {
            None
        };

        // 4.0 spawn 飞控设备（flight_ctrl 开关，Task 22_4）
        let flight_state = Arc::new(RwLock::new(Telemetry::default()));
        let mavlink: Option<Arc<MavlinkDevice>> = match flight_ctrl_port {
            Some(p) => {
                let baud = flight_ctrl_baudrate.unwrap_or(921600);
                info!("启用飞控: port={p}, baud={baud}");
                match MavlinkDevice::spawn(p, baud, flight_state, Some(robot_state.clone()), origin, vel_fwd, yaw_rate_deg) {
                    Ok(d) => Some(Arc::new(d)),
                    Err(e) => {
                        // P2#7：失败路径清理——已 spawn 的 STM32 后台任务必须 shutdown
                        if let Some(s) = &stm32 {
                            s.shutdown();
                        }
                        return Err(e);
                    }
                }
            }
            None => None,
        };

        // 4.1 确定运动设备（车或机，二选一）
        let motion: Arc<dyn MotionDevice> = if let Some(s) = &stm32 {
            s.clone() as Arc<dyn MotionDevice>
        } else if let Some(m) = &mavlink {
            m.clone() as Arc<dyn MotionDevice>
        } else {
            return Err("chassis 与 flight_ctrl 至少启用一个".to_string());
        };


        // 4. spawn 雷达设备（lidar 开关，含 SLAM 建图）
        let lidar: Option<LidarDevice> = match (lidar_enabled, lidar_port, lidar_baudrate) {
            (true, Some(p), Some(b)) => {
                info!("启用 LiDAR: port={p}, baud={b}（含 SLAM 建图）");
                // Task 22：SLAM 归雷达设备，打包 SlamContext 传入
                let slam_ctx = SlamContext {
                    grid: grid.clone(),
                    robot_state: robot_state.clone(),
                    cluster_table: cluster_table.clone(),
                    map_tx: map_tx.clone(),
                    node_handle: node_handle.clone(),
                    local_peer_id: node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes()),
                    obstacle_inflation_radius,
                };
                let dev = match LidarDevice::spawn(p, b, lidar_state.clone(), Some(slam_ctx)) {
                    Ok(d) => d,
                    Err(e) => {
                        // P2#7：失败路径清理——已 spawn 的 STM32/飞控后台任务必须 shutdown
                        if let Some(s) = &stm32 { s.shutdown(); }
                        if let Some(m) = &mavlink { m.shutdown(); }
                        return Err(e);
                    }
                };
                info!("LiDAR 设备已启动，自动开始扫描");
                if let Err(e) = dev.start_scan().await {
                    // P2#7：失败路径清理
                    dev.shutdown();
                    if let Some(s) = &stm32 { s.shutdown(); }
                    if let Some(m) = &mavlink { m.shutdown(); }
                    return Err(e);
                }
                Some(dev)
            }
            _ => {
                info!("LiDAR 未配置，跳过");
                None
            }
        };

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

        // 9. spawn 主循环
        let loop_cancel = cancel.clone();
        let loop_op_mode = op_mode.clone();
        let loop_mission = mission_queue.clone();
        let loop_robot_state = robot_state.clone();
        let loop_lidar_state = lidar_state.clone();
        let loop_execute_state = execute_state.clone();
        let loop_world = world.clone();
        let loop_goal = goal_service.clone();
        tokio::spawn(async move {
            main_loop(
                motion, stm32, mavlink, lidar, cmd_rx,
                loop_op_mode, loop_mission,
                loop_robot_state, loop_lidar_state,
                loop_execute_state,
                loop_world,
                loop_goal,
                loop_cancel,
            ).await;
        });

        Ok(Self { robot_cmd_tx: cmd_tx, robot_state, lidar_state, grid, pose_tx, map_tx, op_mode, mission_queue, cluster_table, cancel })
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
                let s = state.read().await;
                // Task 13_1：意图状态（main_loop 写），两把读锁无交叉
                let es = execute_state.read().await;
                let time_boot_ms = crate::robot::core::protocol::now_boot_ms();
                let _ = pose_tx.send(Pose {
                    time_boot_ms,
                    x: s.x, y: s.y, z: s.z,
                    yaw: s.attitude.yaw,
                    vx: s.vx, vy: s.vy,
                    sub_target: es.sub_target,
                });

                // Task 9_2：位姿广播到集群（fire-and-forget，Network 只搬运字节）
                // Task 13 阶段一：帧身份 = 完整 peer_id（launch 时一次计算）
                if let (Some(nh), Some(peer_id)) = (&node_handle, &local_peer_id) {
                    let pose = PoseData {
                        time_boot_ms,
                        x: s.x, y: s.y, z: s.z,
                        vx: s.vx, vy: s.vy,
                        yaw: s.attitude.yaw,
                        // Task 13_1：意图广播（下一格）；无任务 valid=false
                        valid: es.sub_target.is_some(),
                        sub_gx: es.sub_target.map(|(gx, _)| gx).unwrap_or(0),
                        sub_gy: es.sub_target.map(|(_, gy)| gy).unwrap_or(0),
                    };
                    // ORION 协议：位姿帧广播（2026-08-07 协议统一，替代散装 JSON）
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
// 主 select! 循环（单点执行）
// ============================================================

async fn main_loop(
    motion: Arc<dyn MotionDevice>,
    stm32: Option<Arc<STM32Device>>,
    mavlink: Option<Arc<MavlinkDevice>>,
    lidar: Option<LidarDevice>,
    mut cmd_rx: mpsc::Receiver<Command>,
    op_mode: Arc<RwLock<OpMode>>,
    mission_queue: Arc<RwLock<MissionQueue>>,
    robot_state: Arc<RwLock<RobotState>>,
    lidar_state: Arc<RwLock<LidarState>>,
    execute_state: Arc<RwLock<ExecuteState>>,
    world: Arc<World>,
    goal_service: Arc<tokio::sync::Mutex<GoalService>>,
    cancel: CancellationToken,
) {
    info!("Robot 主循环启动（单点执行 + Rust 决策器）");

    let auto_tick_ms = 50u64;
    let mut next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
    let executor = DecisionExecutor::new();

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
                                goal_service.lock().await.reset();
                                invalidate(&execute_state, &*motion).await;
                            }
                            ModeCmd::SwitchToAuto => {
                                info!("[Robot] 切换到 Auto 模式");
                                *op_mode.write().await = OpMode::Auto;
                                goal_service.lock().await.reset();
                                invalidate(&execute_state, &*motion).await;
                                next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
                            }
                        }
                    }

                    Some(Command::Manual(m)) => {
                        if *op_mode.read().await == OpMode::Manual {
                            dispatch(&*motion, stm32.as_deref(), mavlink.as_deref(), &lidar, m).await;
                        } else {
                            warn!("[Robot] 忽略 Manual 命令：当前为 Auto 模式");
                        }
                    }

                    Some(Command::Auto(a)) => {
                        if *op_mode.read().await == OpMode::Auto {
                            match a {
                                AutoCmd::Set(list) => {
                                    // ORION_TASK_SET 替换语义（2026-08-07）：立即中断当前任务 + 整体替换队列
                                    info!("[Robot] Auto 任务队列已替换");
                                    mission_queue.write().await.replace(list);
                                    goal_service.lock().await.reset();
                                    invalidate(&execute_state, &*motion).await;
                                }
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

            // auto_tick：仅 Auto 模式激活（急停 + get_path + Rust 决策）
            _ = tokio::time::sleep_until(next_tick.into()), if *op_mode.read().await == OpMode::Auto => {
                let rs = robot_state.read().await.clone();
                let ls = lidar_state.read().await.clone();

                let emergency = check_emergency_stop(&ls, &rs, &*motion, &world).await;
                if emergency {
                    // 急停：invalidate（停车 + 清意图），保留 goal 供绕行
                    invalidate(&execute_state, &*motion).await;
                } else {
                    // 方案二：main_loop 自己调 get_path + Rust 决策器（纯决策，无跨线程/代际/看门狗）
                    let next_cell = goal_service.lock().await.get_path().await;

                    let current = execute_state.read().await.clone();
                    let result = executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
                    *execute_state.write().await = ExecuteState {
                        state: result.state,
                        sub_target: result.sub_target,
                    };
                    if let Some(action) = result.action {
                        apply_action(&*motion, action);
                    }
                }

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

    // 退出前：停车 → 停 LiDAR → 关设备
    info!("正在停止机器人...");
    let _ = motion.stop();
    if let Some(l) = &lidar {
        info!("正在停止 LiDAR...");
        let _ = l.stop_scan().await;
        l.shutdown();
    }
    motion.shutdown();

    info!("Robot 主循环已退出");
}

// ============================================================
// 作废助手（Task 22_3：急停 / 任务替换 / 模式切换统一调用）
// ============================================================

/// 作废助手（方案二）：停车 + 清意图。mission/goal 清理由调用点保留。
async fn invalidate(
    execute_state: &Arc<RwLock<ExecuteState>>,
    motion: &dyn MotionDevice,
) {
    let _ = motion.stop();
    *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
}

// ============================================================
// 动作应用（单点发命令）
// ============================================================

fn apply_action(motion: &dyn MotionDevice, action: MotionAction) {
    let r = match action {
        MotionAction::MoveForward => motion.move_forward(),
        MotionAction::MoveBackward => motion.move_backward(),
        MotionAction::TurnLeft => motion.turn_left(),
        MotionAction::TurnRight => motion.turn_right(),
        MotionAction::Stop => motion.stop(),
    };
    if let Err(e) = r {
        warn!("[Robot] 动作执行失败: {e}");
    }
}

// ============================================================
// 命令分发
// ============================================================

async fn dispatch(
    motion: &dyn MotionDevice,
    stm32: Option<&STM32Device>,
    mavlink: Option<&MavlinkDevice>,
    lidar: &Option<LidarDevice>,
    cmd: ManualCmd,
) {
    match cmd {
        ManualCmd::Forward(_)   => { if let Err(e) = motion.move_forward()   { warn!("[Robot] Forward 失败: {e}"); } }
        ManualCmd::Backward(_)  => { if let Err(e) = motion.move_backward()  { warn!("[Robot] Backward 失败: {e}"); } }
        ManualCmd::SpinLeft(_)  => { if let Err(e) = motion.turn_left()      { warn!("[Robot] SpinLeft 失败: {e}"); } }
        ManualCmd::SpinRight(_) => { if let Err(e) = motion.turn_right()     { warn!("[Robot] SpinRight 失败: {e}"); } }
        ManualCmd::Stop         => { if let Err(e) = motion.stop()            { warn!("[Robot] Stop 失败: {e}"); } }
        ManualCmd::Beep(ms)     => {
            match stm32 {
                Some(s) => { if let Err(e) = s.beep(ms) { warn!("[Robot] Beep 失败: {e}"); } }
                None => warn!("[Robot] 无底盘设备，忽略 Beep"),
            }
        }
        ManualCmd::Takeoff => {
            match mavlink {
                Some(m) => { if let Err(e) = m.takeoff_send() { warn!("[Robot] Takeoff 失败: {e}"); } }
                None => warn!("[Robot] 无飞控设备，忽略 Takeoff"),
            }
        }
        ManualCmd::Land => {
            match mavlink {
                Some(m) => { if let Err(e) = m.land_send() { warn!("[Robot] Land 失败: {e}"); } }
                None => warn!("[Robot] 无飞控设备，忽略 Land"),
            }
        }
        ManualCmd::StartLidarScan => {
            if let Some(l) = lidar {
                match l.start_scan().await {
                    Ok(()) => info!("LiDAR 扫描已启动"),
                    Err(e) => warn!("[Robot] LiDAR 启动失败: {e}"),
                }
            } else {
                info!("LiDAR 未启用，忽略 StartLidarScan");
            }
        }
        ManualCmd::StopLidarScan => {
            if let Some(l) = lidar {
                match l.stop_scan().await {
                    Ok(()) => info!("LiDAR 扫描已停止"),
                    Err(e) => info!("LiDAR 停止失败: {e}"),
                }
            }
        }
    }
}

// ============================================================
// 急停检查（Rust 侧，Task 22 步骤 5）
// ============================================================

/// 前方障碍急停检查：LiDAR 前方距离 < 阈值 → 立即停车 + 标记障碍。
/// 返回 true = 已触发急停（跳过本 tick 的决策）。
async fn check_emergency_stop(
    lidar_state: &LidarState,
    robot_state: &RobotState,
    motion: &dyn MotionDevice,
    world: &World,
) -> bool {
    let Some(ref scan) = lidar_state.scan else {
        return false;
    };
    let closest = scan
        .points
        .iter()
        .filter(|p| {
            let a = if p.angle < 0.0 { p.angle + 2.0 * PI } else { p.angle };
            p.range >= 0.1 && (a < PI / 4.0 || a >= 7.0 * PI / 4.0)
        })
        .min_by(|a, b| a.range.partial_cmp(&b.range).unwrap_or(std::cmp::Ordering::Equal));
    let Some(p) = closest else {
        return false;
    };

    if p.range < OBSTACLE_THRESHOLD_M {
        warn!("[Robot] 前方障碍 {:.2}m < {:.2}m，急停", p.range, OBSTACLE_THRESHOLD_M);
        if let Err(e) = motion.stop() {
            warn!("[Robot] 急停失败: {e}");
        }

        let (wx, wy) = (robot_state.x, robot_state.y);
        let yaw = robot_state.attitude.yaw;
        let ob_angle = yaw + p.angle;
        let ob_wx = wx + p.range * ob_angle.cos();
        let ob_wy = wy + p.range * ob_angle.sin();
        let ob_gx = (ob_wx / CELL_RESOLUTION).floor() as i32;
        let ob_gy = (ob_wy / CELL_RESOLUTION).floor() as i32;
        world.mark_obstacle((ob_gx, ob_gy)).await;
        return true;
    }
    false
}

