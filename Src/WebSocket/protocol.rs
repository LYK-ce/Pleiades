//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-07-28

//! WebSocket 协议编解码
//!
//! 三层 JSON 协议：
//! - mode:   {"cmd":"mode",  "action":"switch_to_auto"|"switch_to_manual"}
//! - manual: {"cmd":"manual","action":"forward"|...,"speed":50}
//! - auto:   {"cmd":"auto",  "action":"push"|"cancel","missions":[...]}

use serde_json::Value;
use crate::robot::core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};

/// 解析客户端 JSON 控制指令
pub fn parse_command(text: &str) -> Option<Command> {
    let v: Value = serde_json::from_str(text).ok()?;
    let cmd = v["cmd"].as_str()?;

    match cmd {
        "mode"   => parse_mode(&v),
        "manual" => parse_manual(&v),
        "auto"   => parse_auto(&v),
        other    => { tracing::warn!("[WS] 未知命令类型: {other}"); None }
    }
}

fn parse_mode(v: &Value) -> Option<Command> {
    let action = v["action"].as_str()?;
    match action {
        "switch_to_manual" => Some(Command::Mode(ModeCmd::SwitchToManual)),
        "switch_to_auto"   => Some(Command::Mode(ModeCmd::SwitchToAuto)),
        _ => { tracing::warn!("[WS] 未知 mode action: {action}"); None }
    }
}

fn parse_manual(v: &Value) -> Option<Command> {
    let action = v["action"].as_str()?;
    let speed = v["speed"].as_i64().unwrap_or(50) as i16;
    match action {
        "forward"    => Some(Command::Manual(ManualCmd::Forward(speed))),
        "backward"   => Some(Command::Manual(ManualCmd::Backward(speed))),
        "spin_left"  => Some(Command::Manual(ManualCmd::SpinLeft(speed))),
        "spin_right" => Some(Command::Manual(ManualCmd::SpinRight(speed))),
        "stop"       => Some(Command::Manual(ManualCmd::Stop)),
        "beep"       => {
            let ms = v["ms"].as_u64().unwrap_or(200) as u16;
            Some(Command::Manual(ManualCmd::Beep(ms)))
        }
        "start_lidar" => Some(Command::Manual(ManualCmd::StartLidarScan)),
        "stop_lidar"  => Some(Command::Manual(ManualCmd::StopLidarScan)),
        _ => { tracing::warn!("[WS] 未知 manual action: {action}"); None }
    }
}

fn parse_auto(v: &Value) -> Option<Command> {
    let action = v["action"].as_str()?;
    match action {
        "push" => {
            let missions = v["missions"].as_array()?;
            let mut list = Vec::with_capacity(missions.len());
            for m in missions {
                let mtype = m["type"].as_str()?;
                match mtype {
                    "goto" => {
                        let x = m["x"].as_f64()? as f32;
                        let y = m["y"].as_f64()? as f32;
                        list.push(Mission::Goto(x, y));
                    }
                    _ => { tracing::warn!("[WS] 未知 mission type: {mtype}"); }
                }
            }
            if list.is_empty() { None }
            else { Some(Command::Auto(AutoCmd::Push(list))) }
        }
        "cancel" => Some(Command::Auto(AutoCmd::Cancel)),
        _ => { tracing::warn!("[WS] 未知 auto action: {action}"); None }
    }
}