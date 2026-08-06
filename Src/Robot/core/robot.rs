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
use crate::robot::core::state::{LidarState, RobotState};
use crate::robot::slam::{self, OccupancyGrid, RobotPose};
use std::time::Instant;

use crate::event_bus::{Bus_Event, EventBus};
use crate::network::{DataType, NodeHandle};
use serde_json::json;

/// 位姿广播消息
#[derive(Debug, Clone)]
pub struct Pose {
    pub ts: f64,
    pub x: f32, pub y: f32, pub z: f32,
    pub yaw: f32,
    pub vx: f32, pub vy: f32,
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

        // 4. spawn STM32 Device
        let stm32 = STM32Device::spawn(port, baudrate, car_type, robot_state.clone(), origin)?;

        // 4. spawn LiDAR Device（可选）
        let lidar: Option<LidarDevice> = match (lidar_port, lidar_baudrate) {
            (Some(p), Some(b)) => {
                info!("启用 LiDAR: port={p}, baud={b}");
                let dev = LidarDevice::spawn(p, b, lidar_state.clone())?;
                info!("LiDAR 设备已启动，自动开始扫描");
                dev.start_scan().await?;
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
        tokio::spawn(async move {
            state_notifier(notifier_state, notifier_tx, notifier_handle, notifier_name, notifier_cancel).await;
        });

        // 6. spawn SLAM task
        let slam_cancel = cancel.clone();
        let slam_grid = grid.clone();
        let slam_robot = robot_state.clone();
        let slam_lidar = lidar_state.clone();
        let slam_map_tx = map_tx.clone();
        
        let slam_handle = node_handle.clone();
        let slam_name = peer_name.clone();
        tokio::spawn(async move {
            slam_task(slam_grid, slam_robot, slam_lidar, slam_map_tx, slam_handle, slam_name, slam_cancel).await;
        });

        // 7. 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(32);

        // 8. spawn 主循环
        let loop_cancel = cancel.clone();
        let loop_op_mode = op_mode.clone();
        let loop_mission = mission_queue.clone();
        let loop_robot_state = robot_state.clone();
        let loop_lidar_state = lidar_state.clone();
        let loop_grid = grid.clone();
        let loop_robot_bus = robot_bus.clone();
        tokio::spawn(async move {
            main_loop(
                stm32, lidar, cmd_rx,
                loop_op_mode, loop_mission,
                loop_robot_state, loop_lidar_state, loop_grid,
                loop_robot_bus,
                loop_cancel,
            ).await;
        });

        Ok(Self { robot_cmd_tx: cmd_tx, robot_state, lidar_state, grid, pose_tx, map_tx, op_mode, mission_queue, cancel })
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
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        select! {
            _ = interval.tick() => {
                let s = state.read().await;
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let _ = pose_tx.send(Pose {
                    ts,
                    x: s.x, y: s.y, z: 0.0,
                    yaw: s.attitude.yaw,
                    vx: s.vx, vy: s.vy,
                });

                // Task 9_2：位姿广播到集群（fire-and-forget，Network 只搬运字节）
                if let Some(nh) = &node_handle {
                    let payload = json!({
                        "peer_id": nh.Get_Local_Peer_Id().to_string(),
                        "peer_name": &peer_name,
                        "type": "robot_pose",
                        "x": s.x, "y": s.y, "yaw": s.attitude.yaw,
                        "vx": s.vx, "vy": s.vy,
                        "ts": ts,
                    });
                    if let Err(e) = nh.Broadcast(DataType::Robot, payload.to_string().into_bytes()) {
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
                        if let Some(nh) = &node_handle {
                            let payload = json!({
                                "peer_id": nh.Get_Local_Peer_Id().to_string(),
                                "peer_name": &peer_name,
                                "type": "robot_map_delta",
                                "deltas": typed.iter().map(|d| json!({
                                    "gx": d.gx, "gy": d.gy, "state": d.state,
                                })).collect::<Vec<_>>(),
                            });
                            if let Err(e) = nh.Broadcast(DataType::Robot, payload.to_string().into_bytes()) {
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
    robot_bus: Option<Arc<EventBus>>,
    cancel: CancellationToken,
) {
    info!("Robot 主循环启动（同步 dispatch + auto_tick）");

    let mut executor = Executor::new(ExecutorConfig::default());
    let auto_tick_ms = 50u64;
    let mut next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);

    let mut robot_rx = robot_bus.as_ref().map(|b| b.Subscribe());

    loop {
        select! {
            robot_ev = recv_robot_event(&mut robot_rx) => {
                // Task 9_2：入站机器人数据暂不处理，仅打印（融合/避障将来做）
                if let Some(Bus_Event::Stream { payload }) = robot_ev {
                    info!("[Robot] 收到远端机器人数据: {payload}");
                }
            }
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
                                AutoCmd::Push(list) => {
                                    mission_queue.write().await.push(list);
                                }
                                AutoCmd::Cancel => {
                                    let _ = stm32.stop();
                                    mission_queue.write().await.clear();
                                    executor.reset();
                                    info!("[Robot] Auto 任务队列已清空");
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

                executor.step(&stm32, &rs, &ls, &g, &mut *mission_queue.write().await);

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

/// robot_bus 订阅接收（None 时永远 pending；Lagged 打印警告后继续，无忙循环）
async fn recv_robot_event(rx: &mut Option<broadcast::Receiver<Bus_Event>>) -> Option<Bus_Event> {
    match rx.as_mut() {
        Some(rx) => match rx.recv().await {
            Ok(ev) => Some(ev),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("[Robot] robot_bus 订阅落后 {n} 条");
                None
            }
            Err(broadcast::error::RecvError::Closed) => None,
        },
        None => std::future::pending().await,
    }
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
