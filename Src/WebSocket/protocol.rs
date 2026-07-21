//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-07-21

//! WebSocket 协议编解码

use serde_json::Value;
use crate::robot::core::command::Command;

/// 解析客户端 JSON 控制指令
pub fn parse_command(text: &str) -> Option<Command> {
    let v: Value = serde_json::from_str(text).ok()?;
    let cmd = v["cmd"].as_str()?;
    let speed = v["speed"].as_i64().unwrap_or(50) as i16;
    match cmd {
        "forward"    => Some(Command::Forward(speed)),
        "backward"   => Some(Command::Backward(speed)),
        "spin_left" | "left"   => Some(Command::SpinLeft(speed)),
        "spin_right" | "right" => Some(Command::SpinRight(speed)),
        "stop"       => Some(Command::Stop),
        "beep"       => Some(Command::Beep(v["ms"].as_u64().unwrap_or(200) as u16)),
        _ => { tracing::warn!("[WS] 未知命令: {cmd}"); None }
    }
}
