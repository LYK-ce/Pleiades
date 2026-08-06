//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-06

//! Orion Robot — 独立调试入口
//!
//! 只启动机器人控制相关组件（Robot + WebSocket 遥控），
//! 不加载 Pleiades 分布式推理系统。
//!
//! 用法: orion-robot [x y]    可选的小车初始世界坐标（默认 64 64）
//! 例:   orion-robot 66.5 63.25

use tracing::{info, warn};
use pleiades::robot::{CarType, Robot};

/// 地图范围（世界坐标 [0,128)m，与 OccupancyGrid 256 格 × 0.5m 对应）
const WORLD_MIN: f32 = 0.0;
const WORLD_MAX: f32 = 128.0;
/// 默认初始世界坐标（地图 Chunk 中心）
const DEFAULT_ORIGIN: (f32, f32) = (64.0, 64.0);

/// 解析 CLI 位置参数，非法/越界回退默认
fn parse_origin(args: Vec<String>) -> (f32, f32) {
    match args.as_slice() {
        [] => DEFAULT_ORIGIN,
        [x, y] => match (x.parse::<f32>(), y.parse::<f32>()) {
            (Ok(x), Ok(y)) if (WORLD_MIN..WORLD_MAX).contains(&x) && (WORLD_MIN..WORLD_MAX).contains(&y) => {
                (x, y)
            }
            (Ok(x), Ok(y)) => {
                warn!("origin ({x},{y}) 超出地图范围 [{WORLD_MIN},{WORLD_MAX})，回退默认 ({}, {})", DEFAULT_ORIGIN.0, DEFAULT_ORIGIN.1);
                DEFAULT_ORIGIN
            }
            _ => {
                warn!("origin 参数解析失败，回退默认 ({}, {})", DEFAULT_ORIGIN.0, DEFAULT_ORIGIN.1);
                DEFAULT_ORIGIN
            }
        },
        _ => {
            warn!("参数数量错误，用法: orion-robot [x y]，回退默认 ({}, {})", DEFAULT_ORIGIN.0, DEFAULT_ORIGIN.1);
            DEFAULT_ORIGIN
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    info!("Orion Robot 启动中...");

    let origin = parse_origin(std::env::args().skip(1).collect());
    info!("初始世界坐标 origin = ({}, {})", origin.0, origin.1);

    let robot = Robot::launch("/dev/myserial", 115200, CarType::X3Plus, Some("/dev/rplidar"), Some(230400), origin).await?;
    info!("Robot 已启动");

    let ws_bind = "0.0.0.0:9090";
    pleiades::websocket::start(
        ws_bind, "orion_robot", robot.robot_cmd_tx.clone(),
        robot.pose_tx.subscribe(), robot.map_tx.subscribe(),
        robot.grid.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_origin_default() {
        assert_eq!(parse_origin(vec![]), (64.0, 64.0));
    }

    #[test]
    fn test_parse_origin_valid() {
        assert_eq!(parse_origin(vec!["66.5".into(), "63.25".into()]), (66.5, 63.25));
    }

    #[test]
    fn test_parse_origin_invalid_fallback() {
        // 非法数字 → 默认
        assert_eq!(parse_origin(vec!["abc".into(), "63".into()]), (64.0, 64.0));
        // 越界 → 默认
        assert_eq!(parse_origin(vec!["200".into(), "63".into()]), (64.0, 64.0));
        // 参数个数错误 → 默认
        assert_eq!(parse_origin(vec!["1".into()]), (64.0, 64.0));
        assert_eq!(parse_origin(vec!["1".into(), "2".into(), "3".into()]), (64.0, 64.0));
    }
}
