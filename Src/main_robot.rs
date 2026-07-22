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

    let robot = Robot::launch("/dev/myserial", 115200, CarType::X3Plus, Some("/dev/rplidar"), Some(230400))?;
    info!("Robot 已启动");

    let ws_bind = "0.0.0.0:9090";
    pleiades::websocket::start(
        ws_bind, "orion_robot", robot.cmd_tx.clone(),
        robot.pose_tx.subscribe(), robot.map_tx.subscribe(),
        robot.map_full_tx.subscribe(),
    );

    info!("WebSocket 遥控服务已启动: ws://{ws_bind}");
    info!("打开 Tool/robot_control.html 开始遥控");
    info!("按 Ctrl-C 退出");

    tokio::signal::ctrl_c().await?;
    robot.shutdown();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    info!("Orion Robot 已退出");

    Ok(())
}
