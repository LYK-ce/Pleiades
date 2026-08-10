//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-06

//! Robot 主循环
//!
//! Robot::launch() 启动所有 Device 并 spawn task：
//! - main_loop: select! 收命令 → dispatch
//! - state_notifier: 100ms → 读 robot_state → 广播 Pose
//! - SLAM task: 200ms → 读 lidar_state → update grid → 广播 map_delta

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::command::{AutoCmd, Command, ManualCmd, ModeCmd};
use super::executor::{Executor, ExecutorConfig};
use super::mission::MissionQueue;
use super::mode::OpMode;
use crate::robot::control::device::stm32::STM32Device;
use crate::robot::control::device::lidar::LidarDevice;
use crate::robot::control::types::CarType;
use crate::robot::core::state::{ExecuteState, LidarState, RobotState};
use crate::robot::slam::{self, OccupancyGrid, RobotPose};
use std::time::Instant;

use crate::event_bus::EventBus;
use crate::robot::core::cluster::{cluster_consumer, ClusterInfoTable};
use crate::network::{NodeHandle, TOPIC_ROBOT_POSE, TOPIC_ROBOT_MAP};
use crate::robot::core::protocol::{
    encode_frame, encode_map_delta, encode_pose, now_boot_ms,
    COMPID_ROBOT, MapDeltaEntry, MSGID_MAP_DELTA, MSGID_POSE, PoseData,
};

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

/// 地图增量广播消息
#[derive(Debug, Clone)]
pub struct MapDelta {
    pub gx: i32,
    pub gy: i32,
    pub state: u8,
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
    /// - `origin`: 小车初始世界坐标 (x, y)（默认 64,64），Task 9：RobotState 记录全局唯一坐标
    /// - `node_handle` / `robot_bus`: 网络数据面（Task 9_2），None 则纯本地运行
    /// - `peer_name`: 车名（广播 payload 的 peer_name 字段 + WS vehicle_id）
    pub async fn launch(
        port: &str,
        baudrate: u32,
        car_type: CarType,
        lidar_port: Option<&str>,
        lidar_baudrate: Option<u32>,
        origin: (f32, f32),
        node_handle: Option<Arc<NodeHandle>>,
        robot_bus: Option<Arc<EventBus>>,
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
        }
        let lidar_state = Arc::new(RwLock::new(LidarState::default()));

        // 2. 广播通道
        let (pose_tx, _) = broadcast::channel::<Pose>(32);
        let (map_tx, _) = broadcast::channel::<Vec<MapDelta>>(32);
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));

        // 3. 运行模式 + 任务队列
        let op_mode = Arc::new(RwLock::new(OpMode::default()));
        let mission_queue = Arc::new(RwLock::new(MissionQueue::default()));

        // 3.1 执行器意图状态（Task 13_1：executor 写，state_notifier 读）
        let execute_state = Arc::new(RwLock::new(ExecuteState::default()));

        // 3.2 集群信息表（Task 13_1：入站 POSE → 表）
        let cluster_table = Arc::new(ClusterInfoTable::new());

        // 4. spawn STM32 Device
        let stm32 = STM32Device::spawn(port, baudrate, car_type, robot_state.clone(), origin)?;

        // 4. spawn LiDAR Device（可选）
        let lidar: Option<LidarDevice> = match (lidar_port, lidar_baudrate) {
            (Some(p), Some(b)) => {
                info!("启用 LiDAR: port={p}, baud={b}");
                let dev = match LidarDevice::spawn(p, b, lidar_state.clone()) {
                    Ok(d) => d,
                    Err(e) => {
                        // P2#7：失败路径清理——已 spawn 的 STM32 后台任务必须 shutdown
                        stm32.shutdown();
                        return Err(e);
                    }
                };
                info!("LiDAR 设备已启动，自动开始扫描");
                if let Err(e) = dev.start_scan().await {
                    // P2#7：失败路径清理
                    stm32.shutdown();
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

        // 6. spawn SLAM task
        let slam_cancel = cancel.clone();
        let slam_grid = grid.clone();
        let slam_robot = robot_state.clone();
        let slam_lidar = lidar_state.clone();
        let slam_map_tx = map_tx.clone();
        
        let slam_handle = node_handle.clone();
        let slam_name = peer_name.clone();
        // Task 13 阶段一：同 state_notifier，peer_id 一次计算
        let slam_peer_id = node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes());
        tokio::spawn(async move {
            slam_task(slam_grid, slam_robot, slam_lidar, slam_map_tx, slam_handle, slam_name, slam_peer_id, slam_cancel).await;
        });

        // 7. 集群入站消费者（Task 13_1：robot_bus → ClusterInfo 表，独立 task 数据面）
        let consumer_bus = robot_bus.clone();
        let consumer_peer_id = node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes()).unwrap_or_default();
        let consumer_table = cluster_table.clone();
        let consumer_cancel = cancel.clone();
        tokio::spawn(async move {
            cluster_consumer(consumer_bus, consumer_peer_id, consumer_table, consumer_cancel).await;
        });

        // 8. 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(32);

        // 9. spawn 主循环
        let loop_cancel = cancel.clone();
        let loop_op_mode = op_mode.clone();
        let loop_mission = mission_queue.clone();
        let loop_robot_state = robot_state.clone();
        let loop_lidar_state = lidar_state.clone();
        let loop_grid = grid.clone();
        let loop_execute_state = execute_state.clone();
        tokio::spawn(async move {
            main_loop(
                stm32, lidar, cmd_rx,
                loop_op_mode, loop_mission,
                loop_robot_state, loop_lidar_state, loop_grid,
                loop_execute_state,
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
    peer_name: String,
    local_peer_id: Option<Vec<u8>>,
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        select! {
            _ = interval.tick() => {
                let s = state.read().await;
                // Task 13_1：意图状态（executor 写），两把读锁无交叉
                let es = execute_state.read().await;
                let time_boot_ms = crate::robot::core::protocol::now_boot_ms();
                let _ = pose_tx.send(Pose {
                    time_boot_ms,
                    x: s.x, y: s.y, z: 0.0,
                    yaw: s.attitude.yaw,
                    vx: s.vx, vy: s.vy,
                    sub_target: es.sub_target,
                });

                // Task 9_2：位姿广播到集群（fire-and-forget，Network 只搬运字节）
                // Task 13 阶段一：帧身份 = 完整 peer_id（launch 时一次计算）
                if let (Some(nh), Some(peer_id)) = (&node_handle, &local_peer_id) {
                    let pose = PoseData {
                        time_boot_ms,
                        x: s.x, y: s.y,
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
// SLAM task — 地图更新 (200ms)
// ============================================================

async fn slam_task(
    grid: Arc<RwLock<OccupancyGrid>>,
    robot_state: Arc<RwLock<RobotState>>,
    lidar_state: Arc<RwLock<LidarState>>,
    map_tx: broadcast::Sender<Vec<MapDelta>>,
    node_handle: Option<Arc<NodeHandle>>,
    peer_name: String,
    local_peer_id: Option<Vec<u8>>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(200));
    loop {
        select! {
            _ = interval.tick() => {
                // 同时读位姿和 LiDAR（两个独立锁，无死锁风险）
                let (pose, scan_points) = {
                    let rs = robot_state.read().await;
                    // 位姿 = 全局世界坐标（直读 RobotState，Task 9）
                    let pose = RobotPose {
                        x: rs.x,
                        y: rs.y,
                        yaw: rs.attitude.yaw,
                    };
                    let ls = lidar_state.read().await;
                    let pts: Vec<(f32, f32)> = match &ls.scan {
                        Some(s) => {
                            let mut v = Vec::with_capacity(s.points.len());
                            v.extend(s.points.iter().map(|p| (p.angle, p.range)));
                            v
                        }
                        None => Vec::new(),
                    };
                    (pose, pts)
                };

                if !scan_points.is_empty() {
                    let mut g = grid.write().await;
                    let deltas = slam::update(&mut *g, &pose, &scan_points);
                    if !deltas.is_empty() {
                        let typed: Vec<MapDelta> = deltas.iter().map(|d| MapDelta {
                            gx: d.gx, gy: d.gy, state: d.state,
                        }).collect();
                        let _ = map_tx.send(typed.clone());

                        // Task 9_2：地图增量广播到集群（有 delta 才发）
                        // Task 13 阶段一：帧身份 = 完整 peer_id（launch 时一次计算）
                        if let (Some(nh), Some(peer_id)) = (&node_handle, &local_peer_id) {
                            let entries: Vec<MapDeltaEntry> = typed.iter().map(|d| MapDeltaEntry {
                                gx: d.gx, gy: d.gy, state: d.state,
                            }).collect();
                            // ORION 协议：地图增量帧广播（2026-08-07 协议统一，替代散装 JSON）
                            let payload = encode_map_delta(now_boot_ms(), &entries);
                            let frame = encode_frame(MSGID_MAP_DELTA, peer_id, COMPID_ROBOT, &payload);
                            if let Err(e) = nh.Gossipsub_Publish(TOPIC_ROBOT_MAP, frame).await {
                                warn!("[Robot] 地图增量广播失败: {e}");
                            }
                        }
                    }
                }
            }
            _ = cancel.cancelled() => {
                info!("SLAM task 退出");
                return;
            }
        }
    }
}

// ============================================================
// 主 select! 循环
// ============================================================

async fn main_loop(
    stm32: STM32Device,
    lidar: Option<LidarDevice>,
    mut cmd_rx: mpsc::Receiver<Command>,
    op_mode: Arc<RwLock<OpMode>>,
    mission_queue: Arc<RwLock<MissionQueue>>,
    robot_state: Arc<RwLock<RobotState>>,
    lidar_state: Arc<RwLock<LidarState>>,
    grid: Arc<RwLock<OccupancyGrid>>,
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
) {
    info!("Robot 主循环启动（同步 dispatch + auto_tick）");

    let mut executor = Executor::new(ExecutorConfig::default());
    let auto_tick_ms = 50u64;
    let mut next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);

    loop {
        select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Command::Mode(m)) => {
                        let _ = stm32.stop();
                        match m {
                            ModeCmd::SwitchToManual => {
                                info!("[Robot] 切换到 Manual 模式");
                                *op_mode.write().await = OpMode::Manual;
                                mission_queue.write().await.clear();
                                executor.reset();
                            }
                            ModeCmd::SwitchToAuto => {
                                info!("[Robot] 切换到 Auto 模式");
                                *op_mode.write().await = OpMode::Auto;
                                executor.reset();
                                next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
                            }
                        }
                    }

                    Some(Command::Manual(m)) => {
                        if *op_mode.read().await == OpMode::Manual {
                            dispatch(&stm32, &lidar, m).await;
                        } else {
                            warn!("[Robot] 忽略 Manual 命令：当前为 Auto 模式");
                        }
                    }

                    Some(Command::Auto(a)) => {
                        if *op_mode.read().await == OpMode::Auto {
                            match a {
                                AutoCmd::Set(list) => {
                                    // ORION_TASK_SET 替换语义（2026-08-07）：立即中断当前任务 + 整体替换队列
                                    let _ = stm32.stop();
                                    mission_queue.write().await.replace(list);
                                    executor.reset();
                                    info!("[Robot] Auto 任务队列已替换");
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

            // auto_tick：仅 Auto 模式激活
            _ = tokio::time::sleep_until(next_tick.into()), if *op_mode.read().await == OpMode::Auto => {
                let rs = robot_state.read().await.clone();
                let ls = lidar_state.read().await.clone();
                let g = { let guard = grid.read().await; (*guard).clone() };

                // Task 13_1：execute_state 由 executor 写（step 包装层同步 sub_target）
                executor.step(
                    &stm32, &rs, &ls, &g,
                    &mut *mission_queue.write().await,
                    &mut *execute_state.write().await,
                );

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
    let _ = stm32.stop();
    if let Some(l) = &lidar {
        info!("正在停止 LiDAR...");
        let _ = l.stop_scan().await;
        l.shutdown();
    }
    stm32.shutdown();

    info!("Robot 主循环已退出");
}

// ============================================================
// 命令分发
// ============================================================

async fn dispatch(stm32: &STM32Device, lidar: &Option<LidarDevice>, cmd: ManualCmd) {
    match cmd {
        ManualCmd::Forward(s)   => { if let Err(e) = stm32.forward(s)   { warn!("[Robot] Forward 失败: {e}"); } }
        ManualCmd::Backward(s)  => { if let Err(e) = stm32.backward(s)  { warn!("[Robot] Backward 失败: {e}"); } }
        ManualCmd::SpinLeft(s)  => { if let Err(e) = stm32.spin_left(s)  { warn!("[Robot] SpinLeft 失败: {e}"); } }
        ManualCmd::SpinRight(s) => { if let Err(e) = stm32.spin_right(s) { warn!("[Robot] SpinRight 失败: {e}"); } }
        ManualCmd::Stop         => { if let Err(e) = stm32.stop()        { warn!("[Robot] Stop 失败: {e}"); } }
        ManualCmd::Beep(ms)     => { if let Err(e) = stm32.beep(ms)     { warn!("[Robot] Beep 失败: {e}"); } }
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
