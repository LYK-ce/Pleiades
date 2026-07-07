//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Orion Robot — 独立调试入口
//!
//! 只启动机器人控制相关组件（STM32 串口 + WebSocket 遥控），
//! 不加载 Pleiades 分布式推理系统。
//!
//! ```bash
//! cargo run --release --bin orion-robot --no-default-features
//! ```

use std::sync::Arc;
use tracing::info;
use pleiades::robot::{CarType, STM32Device};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    info!("Orion Robot 启动中...");

    // 启动 STM32 设备
    let stm32 = match STM32Device::spawn("/dev/myserial", 115200, CarType::X3Plus) {
        Ok(d) => {
            info!("STM32 设备已启动");
            Arc::new(d)
        }
        Err(e) => {
            tracing::warn!("STM32 设备启动失败: {e}，继续运行（无硬件）");
            Arc::new(STM32Device::spawn("/dev/null", 9600, CarType::X3Plus)?)
        }
    };
    pleiades::robot::init_stm32(stm32);

    // 启动 WebSocket 遥控服务
    let event_bus = Arc::new(pleiades::event_bus::EventBus::New(64));
    pleiades::robot::server::spawn_robot_ws_server(9090, event_bus);

    info!("WebSocket 遥控服务已启动: ws://0.0.0.0:9090");
    info!("打开 Tool/robot_control.html 开始遥控");
    info!("按 Ctrl-C 退出");

    tokio::signal::ctrl_c().await?;
    info!("收到退出信号");
    info!("Orion Robot 已退出");

    Ok(())
}
