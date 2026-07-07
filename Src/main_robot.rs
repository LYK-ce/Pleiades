//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Orion Robot — 独立调试入口
//!
//! 只启动机器人控制相关组件（串口 + WebSocket 遥控），
//! 不加载 Pleiades 分布式推理系统。
//!
//! ```bash
//! cargo run --release --bin orion-robot
//! ```

use std::sync::Arc;
use tracing::info;
use pleiades::robot::control::types::CarType;
use pleiades::robot::RobotCapability;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ══════════════════════════════════════════════════════
    // 初始化日志
    // ══════════════════════════════════════════════════════

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    info!("Orion Robot 启动中...");

    // ══════════════════════════════════════════════════════
    // 初始化 Robot 并打开串口
    // ══════════════════════════════════════════════════════

    let robot = Arc::new(pleiades::robot::Robot::new());
    pleiades::robot::init_robot(robot.clone());

    match robot.open("/dev/myserial", 115200, CarType::X3Plus).await {
        Ok(()) => info!("串口已打开"),
        Err(e) => tracing::warn!("打开串口失败: {e}"),
    }

    // ══════════════════════════════════════════════════════
    // 启动 WebSocket 遥控服务
    // ══════════════════════════════════════════════════════

    // 创建一个空的 EventBus，telemetry 只打日志不推 TUI
    let event_bus = Arc::new(pleiades::event_bus::EventBus::New(64));
    pleiades::robot::server::spawn_robot_ws_server(9090, event_bus);

    info!("WebSocket 遥控服务已启动: ws://0.0.0.0:9090");
    info!("打开 Tool/robot_control.html 开始遥控");
    info!("按 Ctrl-C 退出");

    // ══════════════════════════════════════════════════════
    // 等待退出信号
    // ══════════════════════════════════════════════════════

    tokio::signal::ctrl_c().await?;
    info!("收到退出信号，停车...");
    let _ = robot.stop().await;
    info!("Orion Robot 已退出");

    Ok(())
}
