//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Robot 主循环
//!
//! Robot::launch() 启动所有 Device 并 spawn 主 select! 任务。

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::command::Command;
use crate::robot::control::device::stm32::STM32Device;
use crate::robot::control::types::CarType;
use crate::robot::state::RobotState;

/// Robot — 机器人系统中枢
pub struct Robot {
    pub cmd_tx: mpsc::Sender<Command>,
    pub state: Arc<RwLock<RobotState>>,
    cancel: CancellationToken,
}

impl Robot {
    /// 启动 Robot：创建共享状态，spawn Device，启动主循环
    pub fn launch(
        port: &str,
        baudrate: u32,
        car_type: CarType,
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();

        // 1. 创建全局状态
        let state = Arc::new(RwLock::new(RobotState::default()));

        // 2. spawn Device（内部启动 TX + RX tokio task）
        let stm32 = STM32Device::spawn(port, baudrate, car_type, state.clone())?;

        // 3. 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(32);

        // 4. spawn 主循环
        let loop_cancel = cancel.clone();
        let loop_state = state.clone();
        tokio::spawn(async move {
            main_loop(stm32, cmd_rx, loop_state, loop_cancel).await;
        });

        Ok(Self { cmd_tx, state, cancel })
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
    mut cmd_rx: mpsc::Receiver<Command>,
    state: Arc<RwLock<RobotState>>,
    cancel: CancellationToken,
) {
    info!("Robot 主循环启动");

    let mut tick = tokio::time::interval(Duration::from_millis(200));

    loop {
        select! {
            // 命令
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(c) => dispatch(&stm32, c).await,
                    None => {
                        info!("命令通道关闭，Robot 退出");
                        break;
                    }
                }
            }

            // 定时状态检查
            _ = tick.tick() => {
                let s = state.read().await;
                // 未来：低电量告警、异常检测等
                let _ = s;
            }

            // 退出信号
            _ = cancel.cancelled() => {
                info!("Robot 收到退出信号");
                break;
            }
        }
    }

    info!("Robot 主循环已退出");
}

// ============================================================
// 命令分发
// ============================================================

async fn dispatch(stm32: &STM32Device, cmd: Command) {
    match cmd {
        Command::Forward(s)   => { let _ = stm32.forward(s).await; }
        Command::Backward(s)  => { let _ = stm32.backward(s).await; }
        Command::SpinLeft(s)  => { let _ = stm32.spin_left(s).await; }
        Command::SpinRight(s) => { let _ = stm32.spin_right(s).await; }
        Command::Stop         => { let _ = stm32.stop().await; }
        Command::Beep(ms)     => { let _ = stm32.beep(ms).await; }
    }
}
