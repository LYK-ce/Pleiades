//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-21

//! pleiades-sim — 模拟车/机节点（无硬件，用于测试决策链）
//!
//! 完整 bootstrap + Core 推理循环 + Robot 循环；设备端无硬件，
//! `on_tick` 跑完整决策链（寻路 + 三状态机），动作只打印。
//!
//! 用法: pleiades-sim [--headless] [x y [z]]    可选初始世界坐标（默认 64 64 0）
//!
//! `--headless`（task_31 Step 7）：不启动 TUI，复用 base 已有的 `run_headless()`
//! （与真机一样没有终端时 TUI 会 panic，端到端验收需要后台起进程）。默认行为不变。

mod bootstrap;
mod config;
mod sim;
// 世界链路（task_31 Step 2）：`[sim_world]` 段 → 1 条底盘连接 + N 条传感器连接
mod world_link;

use std::sync::Arc;

use tracing::{info, warn};

use pleiades_base::bootstrap::core_bootstrap;

/// 地图范围（世界坐标 [0,128)m）
const WORLD_MIN: f32 = 0.0;
const WORLD_MAX: f32 = 128.0;
/// 默认初始世界坐标
const DEFAULT_ORIGIN: (f32, f32, f32) = (64.0, 64.0, 0.0);

/// `--headless` 开关的 argv 名
const HEADLESS_FLAG: &str = "--headless";

/// 把 `--headless` 从任意位置摘出（重复出现幂等），其余参数原序返回给 `parse_origin`。
///
/// 纯函数：不读环境、不打印，便于单测。摘除后 `parse_origin` 看到的仍是 `[x y [z]]`。
fn split_args(args: Vec<String>) -> (bool, Vec<String>) {
    let mut headless = false;
    let mut rest = Vec::with_capacity(args.len());
    for a in args {
        if a == HEADLESS_FLAG {
            headless = true;
        } else {
            rest.push(a);
        }
    }
    (headless, rest)
}

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
        _ => fallback("参数数量错误（用法: pleiades-sim [--headless] [x y [z]]）".into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut boot = core_bootstrap().await?;
    info!("Core bootstrap 完成");

    let (headless, rest) = split_args(std::env::args().skip(1).collect());
    let origin = parse_origin(rest);
    info!("初始世界坐标 origin = ({}, {}, {})", origin.0, origin.1, origin.2);
    info!("运行模式: {}", if headless { "headless（无 TUI）" } else { "TUI" });

    let config = config::Ensure_Sim_Config()?;

    let robot_cmd_frame_rx = boot.robot_cmd_frame_rx.take().expect("robot_cmd_frame_rx 未初始化");
    let robot = bootstrap::sim_bootstrap(
        &config,
        Arc::new(boot.node_handle.clone()),
        boot.robot_bus.clone(),
        robot_cmd_frame_rx,
        origin,
    ).await?;

    if headless {
        // §九 端到端验收：后台起进程（无 tty），TUI 会 panic；base 的 run_headless 不启 TUI
        boot.run_headless().await;
    } else {
        boot.run().await;
    }

    info!("主循环退出，关闭 Robot...");
    robot.shutdown();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    info!("pleiades-sim 已退出");
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
        assert_eq!(parse_origin(vec!["66.5".into(), "63.25".into(), "1.5".into()]), (66.5, 63.25, 1.5));
    }

    #[test]
    fn test_parse_origin_invalid_fallback() {
        assert_eq!(parse_origin(vec!["abc".into(), "63".into()]), (64.0, 64.0, 0.0));
        assert_eq!(parse_origin(vec!["200".into(), "63".into()]), (64.0, 64.0, 0.0));
        assert_eq!(parse_origin(vec!["1".into()]), (64.0, 64.0, 0.0));
    }

    // ─── split_args（D3：--headless 只在 sim 侧，不改 base）─────────────

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_split_args_empty() {
        assert_eq!(split_args(vec![]), (false, Vec::<String>::new()));
    }

    #[test]
    fn test_split_args_only_flag() {
        assert_eq!(split_args(v(&["--headless"])), (true, Vec::<String>::new()));
    }

    #[test]
    fn test_split_args_flag_then_origin() {
        assert_eq!(split_args(v(&["--headless", "64", "64", "1"])), (true, v(&["64", "64", "1"])));
    }

    #[test]
    fn test_split_args_flag_anywhere() {
        // flag 在中间：位置参数保持原序
        assert_eq!(split_args(v(&["66", "63", "--headless"])), (true, v(&["66", "63"])));
        assert_eq!(split_args(v(&["66", "--headless", "63"])), (true, v(&["66", "63"])));
    }

    #[test]
    fn test_split_args_duplicate_flag_idempotent() {
        assert_eq!(split_args(v(&["--headless", "--headless"])), (true, Vec::<String>::new()));
    }

    #[test]
    fn test_split_args_no_flag_keeps_all_args() {
        assert_eq!(split_args(v(&["66", "63"])), (false, v(&["66", "63"])));
    }

    #[test]
    fn test_split_args_composes_with_parse_origin() {
        // 摘 flag 后再解析：等价于不带 flag 的 `[x y]`
        let (headless, rest) = split_args(v(&["--headless", "66.5", "63.25"]));
        assert!(headless);
        assert_eq!(parse_origin(rest), (66.5, 63.25, 0.0));
    }
}
