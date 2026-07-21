//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-21

//! Robot 主循环
//!
//! Robot::launch() 启动所有 Device 并 spawn 主 select! 任务。

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::command::Command;
use crate::robot::control::device::stm32::STM32Device;
use crate::robot::control::device::lidar::LidarDevice;
use crate::robot::control::types::CarType;
use crate::robot::state::{LidarState, RobotState};

/// Robot — 机器人系统中枢
pub struct Robot {
    pub cmd_tx: mpsc::Sender<Command>,
    pub robot_state: Arc<RwLock<RobotState>>,
    pub lidar_state: Arc<RwLock<LidarState>>,
    cancel: CancellationToken,
}

impl Robot {
    /// 启动 Robot：创建各设备独立状态，spawn Device，启动主循环
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

        // 2. spawn STM32 Device
        let stm32 = STM32Device::spawn(port, baudrate, car_type, robot_state.clone())?;

        // 2.5 spawn LiDAR Device（可选）
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

        // 3. 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(32);

        // 4. spawn 主循环
        let loop_cancel = cancel.clone();
        tokio::spawn(async move {
            main_loop(stm32, lidar, cmd_rx, loop_cancel).await;
        });

        Ok(Self { cmd_tx, robot_state, lidar_state, cancel })
    }

    /// 优雅退出
    pub fn shutdown(&self) {
        self.cancel.cancel();
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

    let mut tick = tokio::time::interval(Duration::from_millis(200));

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

            _ = tick.tick() => {
                // 预留：低电量告警、异常检测等
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
                    Err(e) => info!("LiDAR 启动失败: {e}"),
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
