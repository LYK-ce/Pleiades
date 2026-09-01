//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-30

//! pleiades-ugv — Jetson 车载完整节点（Task 23 C2）
//!
//! 完整 bootstrap + Core 推理循环 + Robot 循环 + 网络数据面（位姿/地图广播）。
//!
//! 用法: pleiades-ugv [x y [z]]    可选的小车初始世界坐标（默认 64 64 0）
//! 例:   pleiades-ugv 66.5 63.25
//!       pleiades-ugv 66.5 63.25 1.5

mod bootstrap;
mod config;
mod device;
mod ugv;

use std::sync::Arc;

use tracing::{info, warn};

use pleiades_base::bootstrap::core_bootstrap;

/// 地图范围（世界坐标 [0,128)m，与 OccupancyGrid 256 格 × 0.5m 对应）
const WORLD_MIN: f32 = 0.0;
const WORLD_MAX: f32 = 128.0;
/// 默认初始世界坐标（地图 Chunk 中心）
const DEFAULT_ORIGIN: (f32, f32, f32) = (64.0, 64.0, 0.0);

/// 解析 CLI 位置参数 `[x y [z]]`，非法/越界回退默认。z 可选，默认 0。
fn parse_origin(args: Vec<String>) -> (f32, f32, f32) {
    let in_world = |v: f32| (WORLD_MIN..WORLD_MAX).contains(&v);
    let fallback = |reason: String| -> (f32, f32, f32) {
        warn!("origin {reason}，回退默认 ({}, {}, {})", DEFAULT_ORIGIN.0, DEFAULT_ORIGIN.1, DEFAULT_ORIGIN.2);
        DEFAULT_ORIGIN
    };
    match args.as_slice() {
        [] => DEFAULT_ORIGIN,
        [x, y] => match (x.parse::<f32>(), y.parse::<f32>()) {
            (Ok(x), Ok(y)) if in_world(x) && in_world(y) => (x, y, 0.0),
            (Ok(x), Ok(y)) => fallback(format!("({x},{y}) 超出地图范围 [{WORLD_MIN},{WORLD_MAX})")),
            _ => fallback("参数解析失败".into()),
        },
        [x, y, z] => match (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>()) {
            (Ok(x), Ok(y), Ok(z)) if in_world(x) && in_world(y) => (x, y, z),
            (Ok(x), Ok(y), Ok(_)) => fallback(format!("({x},{y}) 超出地图范围 [{WORLD_MIN},{WORLD_MAX})")),
            _ => fallback("参数解析失败".into()),
        },
        _ => fallback("参数数量错误（用法: pleiades-ugv [x y [z]]）".into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 完整 bootstrap（含 tracing 日志初始化，base）
    let mut boot = core_bootstrap().await?;
    info!("Core bootstrap 完成");

    let origin = parse_origin(std::env::args().skip(1).collect());
    info!("初始世界坐标 origin = ({}, {}, {})", origin.0, origin.1, origin.2);

    // 读取 UGV 配置（共享段 + chassis/lidar）
    let config = config::Ensure_Ugv_Config()?;

    // Robot + 网络数据面（launch 内 main_loop/slam/notifier 后台跑）
    let robot_cmd_frame_rx = boot.robot_cmd_frame_rx.take().expect("robot_cmd_frame_rx 未初始化");
    let robot = bootstrap::ugv_bootstrap(
        &config,
        Arc::new(boot.node_handle.clone()),
        boot.robot_bus.clone(),
        robot_cmd_frame_rx,
        origin,
    ).await?;

    // 进入主循环（阻塞：network 事件循环 + TUI + Core；Robot 循环已在后台并行）
    boot.run().await;

    info!("主循环退出，关闭 Robot...");
    robot.shutdown();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    info!("pleiades-ugv 已退出");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_origin_default() {
        assert_eq!(parse_origin(vec![]), (64.0, 64.0, 0.0));
    }

    #[test]
    fn test_parse_origin_valid() {
        assert_eq!(parse_origin(vec!["66.5".into(), "63.25".into()]), (66.5, 63.25, 0.0));
        // 三参数：z 显式指定
        assert_eq!(parse_origin(vec!["66.5".into(), "63.25".into(), "1.5".into()]), (66.5, 63.25, 1.5));
    }

    #[test]
    fn test_parse_origin_invalid_fallback() {
        // 非法数字 → 默认
        assert_eq!(parse_origin(vec!["abc".into(), "63".into()]), (64.0, 64.0, 0.0));
        // 越界 → 默认
        assert_eq!(parse_origin(vec!["200".into(), "63".into()]), (64.0, 64.0, 0.0));
        // 参数个数错误 → 默认
        assert_eq!(parse_origin(vec!["1".into()]), (64.0, 64.0, 0.0));
        assert_eq!(parse_origin(vec!["1".into(), "2".into(), "3".into(), "4".into()]), (64.0, 64.0, 0.0));
    }
}
