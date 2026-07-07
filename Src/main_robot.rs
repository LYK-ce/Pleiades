//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Orion Robot — 独立调试入口
//!
//! 只启动机器人控制相关组件（Robot + WebSocket 遥控），
//! 不加载 Pleiades 分布式推理系统。

use std::sync::Arc;
use tracing::info;
use pleiades::robot::{CarType, Robot};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    info!("Orion Robot 启动中...");

    let robot = match Robot::launch("/dev/myserial", 115200, CarType::X3Plus) {
        Ok(r) => {
            info!("Robot 已启动");
            r
        }
        Err(e) => {
            tracing::warn!("Robot 启动失败: {e}，继续运行（无硬件）");
            Robot::launch("/dev/null", 9600, CarType::X3Plus)?
        }
    };

    let event_bus = Arc::new(pleiades::event_bus::EventBus::New(64));
    pleiades::robot::server::spawn_robot_ws_server(
        9090, event_bus, robot.cmd_tx.clone(), robot.state.clone(),
    );

    info!("WebSocket 遥控服务已启动: ws://0.0.0.0:9090");
    info!("打开 Tool/robot_control.html 开始遥控");
    info!("按 Ctrl-C 退出");

    tokio::signal::ctrl_c().await?;
    info!("Orion Robot 已退出");

    Ok(())
}
