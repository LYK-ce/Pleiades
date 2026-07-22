//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-21

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

use super::command::Command;
use crate::robot::control::device::stm32::STM32Device;
use crate::robot::control::device::lidar::LidarDevice;
use crate::robot::control::types::CarType;
use crate::robot::state::{LidarState, RobotState};
use crate::robot::slam::{self, OccupancyGrid, RobotPose};

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
    pub cmd_tx: mpsc::Sender<Command>,
    pub robot_state: Arc<RwLock<RobotState>>,
    pub lidar_state: Arc<RwLock<LidarState>>,
    /// 位姿广播
    pub pose_tx: broadcast::Sender<Pose>,
    /// 地图增量广播
    pub map_tx: broadcast::Sender<Vec<MapDelta>>,
    /// 全量地图广播 (二进制，预留)
    pub map_full_tx: broadcast::Sender<Vec<u8>>,
    cancel: CancellationToken,
}

impl Robot {
    /// 启动 Robot：创建各设备独立状态，spawn Device，启动所有 task
    ///
    /// - `lidar_port` / `lidar_baudrate`: 可选 LiDAR 配置，None 则不启用
    pub fn launch(
        port: &str,
        baudrate: u32,
        car_type: CarType,
        lidar_port: Option<&str>,
        lidar_baudrate: Option<u32>,
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();

        // 1. 创建各设备独立状态
        let robot_state = Arc::new(RwLock::new(RobotState::default()));
        let lidar_state = Arc::new(RwLock::new(LidarState::default()));

        // 2. 广播通道
        let (pose_tx, _) = broadcast::channel::<Pose>(32);
        let (map_tx, _) = broadcast::channel::<Vec<MapDelta>>(32);
        let (map_full_tx, _) = broadcast::channel::<Vec<u8>>(4);

        // 3. spawn STM32 Device
        let stm32 = STM32Device::spawn(port, baudrate, car_type, robot_state.clone())?;

        // 4. spawn LiDAR Device（可选）
        let lidar: Option<LidarDevice> = match (lidar_port, lidar_baudrate) {
            (Some(p), Some(b)) => {
                info!("启用 LiDAR: port={p}, baud={b}");
                Some(LidarDevice::spawn(p, b, lidar_state.clone())?)
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
        tokio::spawn(async move {
            state_notifier(notifier_state, notifier_tx, notifier_cancel).await;
        });

        // 6. spawn SLAM task
        let slam_cancel = cancel.clone();
        let slam_grid = OccupancyGrid::new();
        let slam_robot = robot_state.clone();
        let slam_lidar = lidar_state.clone();
        let slam_map_tx = map_tx.clone();
        let slam_full_tx = map_full_tx.clone();
        tokio::spawn(async move {
            slam_task(slam_grid, slam_robot, slam_lidar, slam_map_tx, slam_full_tx, slam_cancel).await;
        });

        // 7. 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(32);

        // 8. spawn 主循环
        let loop_cancel = cancel.clone();
        tokio::spawn(async move {
            main_loop(stm32, lidar, cmd_rx, loop_cancel).await;
        });

        Ok(Self { cmd_tx, robot_state, lidar_state, pose_tx, map_tx, map_full_tx, cancel })
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
                    x: 64.0, y: 64.0, z: 0.0,
                    yaw: s.attitude.yaw,
                    vx: s.vx, vy: s.vy,
                });
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
    mut grid: OccupancyGrid,
    robot_state: Arc<RwLock<RobotState>>,
    lidar_state: Arc<RwLock<LidarState>>,
    map_tx: broadcast::Sender<Vec<MapDelta>>,
    map_full_tx: broadcast::Sender<Vec<u8>>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(200));
    let mut full_interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        select! {
            _ = interval.tick() => {
                // 同时读位姿和 LiDAR（两个独立锁，无死锁风险）
                let (pose, scan_points) = {
                    let rs = robot_state.read().await;
                    let pose = RobotPose {
                        x: 64.0,  // Chunk(0,0) 中心，待里程计
                        y: 64.0,
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
                    let deltas = slam::update(&mut grid, &pose, &scan_points);
                    if !deltas.is_empty() {
                        let typed: Vec<MapDelta> = deltas.iter().map(|d| MapDelta {
                            gx: d.gx, gy: d.gy, state: d.state,
                        }).collect();
                        let _ = map_tx.send(typed);
                    }
                }
            }
            _ = full_interval.tick() => {
                let data = grid.build_map_full();
                info!("[SLAM] 发送 map_full: {} 字节", data.len());
                let _ = map_full_tx.send(data);
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
    cancel: CancellationToken,
) {
    info!("Robot 主循环启动");

    loop {
        select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(c) => dispatch(&stm32, &lidar, c).await,
                    None => {
                        info!("命令通道关闭，Robot 退出");
                        break;
                    }
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
    let _ = stm32.stop().await;
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

async fn dispatch(stm32: &STM32Device, lidar: &Option<LidarDevice>, cmd: Command) {
    match cmd {
        Command::Forward(s)   => { if let Err(e) = stm32.forward(s).await { warn!("[Robot] Forward 失败: {e}"); } }
        Command::Backward(s)  => { if let Err(e) = stm32.backward(s).await { warn!("[Robot] Backward 失败: {e}"); } }
        Command::SpinLeft(s)  => { if let Err(e) = stm32.spin_left(s).await { warn!("[Robot] SpinLeft 失败: {e}"); } }
        Command::SpinRight(s) => { if let Err(e) = stm32.spin_right(s).await { warn!("[Robot] SpinRight 失败: {e}"); } }
        Command::Stop         => { if let Err(e) = stm32.stop().await { warn!("[Robot] Stop 失败: {e}"); } }
        Command::Beep(ms)     => { if let Err(e) = stm32.beep(ms).await { warn!("[Robot] Beep 失败: {e}"); } }
        Command::StartLidarScan => {
            if let Some(l) = lidar {
                match l.start_scan().await {
                    Ok(()) => info!("LiDAR 扫描已启动"),
                    Err(e) => warn!("[Robot] LiDAR 启动失败: {e}"),
                }
            } else {
                info!("LiDAR 未启用，忽略 StartLidarScan");
            }
        }
        Command::StopLidarScan => {
            if let Some(l) = lidar {
                match l.stop_scan().await {
                    Ok(()) => info!("LiDAR 扫描已停止"),
                    Err(e) => info!("LiDAR 停止失败: {e}"),
                }
            }
        }
    }
}
