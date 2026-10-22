//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 本地控制口（task_31 §七）：**测试夹具**，不是产品接口。
//!
//! 只绑 `127.0.0.1`、无鉴权、不进决策链、不参与 P2P、不走 ORION 帧。
//! 注入路径 = 地面站同一条下游：直接构造 `Command` → `robot.robot_cmd_tx.send()` → `main_loop`
//! （`op_mode` 门闩 → `mission_queue.replace` → `device.reset()`）。
//!
//! ```text
//! 地面站：TASK_SET 帧 ─▶ command_consumer ─▶ parse_orion_frame ─▶ Command ─┐
//!                                                                         ├─▶ cmd_rx ─▶ main_loop
//! 控制口：                      直接构造 Command ─▶ robot.robot_cmd_tx.send ┘
//! ```
//!
//! 纪律：
//! - **一问一答**，无订阅、无推送（`status` 也是拉取式，Step 7 每 200 ms 采一次）
//! - 只有收到请求后才写回包，所以**不需要** `Split()`（`select!` 的借用在该 arm 结束时释放）
//! - `bind` 失败只 `warn!`，**不阻断仿真**（控制口挂了不该拖死决策链）
//! - `cancel` 时 accept 循环退出、已建立连接各自 `select!` 到同一个 token 全部断开
//! - 可判定逻辑全部抽成**纯函数**（[`Parse_Control`] / [`Stop_Command`] / [`Manual_Command`] /
//!   [`Status_Data`] / [`Help_Data`] / `Ok_Reply*` / [`Err_Reply`] / [`Bind_Listener`]），
//!   异步壳只做 IO —— `Arc<Robot>` 太重，不拖进单测

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use pleiades_base::robot::core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
use pleiades_base::robot::core::mode::OpMode;
use pleiades_base::robot::core::robot::Robot;

use crate::config::{ControlConfig, Is_Loopback, ResolvedControl};
use crate::world_link::{WorldLinkHandle, WorldState};

// ================================================================ 常量

/// WS 握手超时（客户端连上但不说 WS 话时，别把 task 挂死）
pub const CONTROL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// `cmd_tx` 投递超时（通道容量 32，满了不该把这条连接 task 挂死）
pub const CONTROL_SEND_TIMEOUT: Duration = Duration::from_millis(1000);
/// `mode` 投递后等 `op_mode` 真正切换的上限（消除「切完立刻发 manual 被按旧模式误拒」的竞态）
pub const CONTROL_MODE_SETTLE: Duration = Duration::from_millis(500);

/// 世界范围（米，右开）：与 `main.rs` 的 `WORLD_MIN/WORLD_MAX` **逐字一致**
const CONTROL_WORLD_MIN: f32 = 0.0;
const CONTROL_WORLD_MAX: f32 = 128.0;

// ================================================================ 请求模型

/// 控制口的请求（[`Parse_Control`] 的产物）
#[derive(Debug, Clone, PartialEq)]
pub enum ControlCmd {
    Goto { x: f32, y: f32 },
    Circle { x: f32, y: f32 },
    Stop,
    Mode { auto: bool },
    Manual { action: String },
    Status,
    Help,
}

// ================================================================ 入口与生命周期

/// 起控制口：绑 `127.0.0.1:port`，**失败只 warn**（不阻断仿真）。
///
/// `enabled = false` 时只打一行 `info!` 后返回。
/// 注：设计签名 §七.6 为 `(cfg, robot, world, cancel)`；`status` 需要 `peer_id`（§七.4 的回包字段），
///     故在 `cancel` 前插一个 `peer_id: String` —— 唯一签名补充（见 tmp/task31/plan.md §10-R4）。
pub fn Spawn(
    cfg: ControlConfig,
    robot: Arc<Robot>,
    world: Option<WorldLinkHandle>,
    peer_id: String,
    cancel: CancellationToken,
) {
    if !cfg.Enabled() {
        // step.md 逐字要求（关掉时必须能一眼看出是「没开」而不是「连不上」）
        info!("[control] 已关闭（[control].enabled = false）");
        return;
    }
    let r = cfg.Resolved();
    // 只对「写了且非空」的 bind 做回环校验（空串按默认处理，不该误报）
    if let Some(b) = cfg.bind.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        if !Is_Loopback(b) {
            warn!(
                "[control] bind='{b}' 不是本机回环地址，强制改为 127.0.0.1（§七.8-1：控制口只绑本机，严禁 0.0.0.0）"
            );
        }
    }
    tokio::spawn(async move { Run(r, robot, world, Arc::new(peer_id), cancel).await });
}

/// 绑定监听（单测缝：端口被占必须 `Err` 而不是 panic）
async fn Bind_Listener(bind: &str, port: u16) -> Result<TcpListener, String> {
    TcpListener::bind((bind, port)).await.map_err(|e| e.to_string())
}

/// accept 循环（cancel → 退出；accept 出错 → warn + 100 ms 退避防忙等）
async fn Run(
    cfg: ResolvedControl,
    robot: Arc<Robot>,
    world: Option<WorldLinkHandle>,
    peer_id: Arc<String>,
    cancel: CancellationToken,
) {
    let listener = match Bind_Listener(&cfg.bind, cfg.port).await {
        Ok(l) => l,
        Err(e) => {
            warn!(
                "[control] 监听失败 {}:{}：{e}（控制口不可用，仿真继续；检查端口是否被占）",
                cfg.bind, cfg.port
            );
            return; // 只 warn，不阻断仿真（§七.6 要点 1）
        }
    };
    let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
    info!("[control] 监听 ws://{addr}（测试夹具：仅本机、无鉴权、不进决策链/不走 ORION 帧）");

    loop {
        let accepted = tokio::select! {
            _ = cancel.cancelled() => None,
            r = listener.accept() => Some(r),
        };
        match accepted {
            None => {
                info!("[control] 控制口关闭");
                return;
            }
            Some(Err(e)) => {
                // 不 sleep 会忙等刷屏（EMFILE 等瞬时错误应当退避）
                warn!("[control] accept 失败：{e}（100 ms 后重试）");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Some(Ok((stream, peer))) => {
                debug!("[control] 连接建立 peer={peer}");
                tokio::spawn(Serve_Conn(
                    stream,
                    robot.clone(),
                    world.clone(),
                    peer_id.clone(),
                    cancel.clone(),
                ));
            }
        }
    }
}

/// 一条连接 = 一个 task（WS 握手 5 s 超时 → 逐帧读 → 一问一答）
async fn Serve_Conn(
    stream: TcpStream,
    robot: Arc<Robot>,
    world: Option<WorldLinkHandle>,
    peer_id: Arc<String>,
    cancel: CancellationToken,
) {
    let mut ws = match tokio::time::timeout(CONTROL_HANDSHAKE_TIMEOUT, accept_async(stream)).await {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            debug!("[control] WS 握手失败：{e}");
            return;
        }
        Err(_) => {
            debug!("[control] WS 握手超时（5 s）");
            return;
        }
    };
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => break,
            n = ws.next() => n,
        };
        match next {
            None => break, // 对端关闭
            Some(Err(e)) => {
                debug!("[control] 读帧失败：{e}");
                break;
            }
            Some(Ok(Message::Text(t))) => {
                let reply = Handle_Text(&t, &robot, world.as_ref(), &peer_id).await;
                let s = reply.to_string();
                if let Err(e) = ws.send(Message::Text(s)).await {
                    debug!("[control] 回包失败：{e}");
                    break;
                }
            }
            Some(Ok(Message::Binary(_))) => {
                debug!("[control] 忽略二进制帧（本口只收文本 JSON）")
            }
            Some(Ok(Message::Close(_))) => break,
            Some(Ok(_)) => {} // Ping / Pong / Frame
        }
    }
}

// ================================================================ 解析（纯函数）

/// 纯函数：解析一帧文本请求。所有错误都是可直接回给客户端的**可操作中文**文本。
pub fn Parse_Control(text: &str) -> Result<ControlCmd, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("非法 JSON：{e}"))?;
    let obj = v.as_object().ok_or_else(|| "请求必须是 JSON 对象".to_string())?;
    let cmd = obj
        .get("cmd")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "缺少 cmd 字段（应为字符串）".to_string())?
        .trim()
        .to_string();
    match cmd.as_str() {
        "goto" | "circle" => {
            // 现状不支持群发 members（§七.4 / §七.8-6）：非空数组 → 明确拒绝，不假装成功
            if let Some(m) = obj.get("members") {
                let non_empty = m.as_array().map(|a| !a.is_empty()).unwrap_or(true);
                if non_empty {
                    return Err("members 不支持，请用地面站".to_string());
                }
            }
            let x = Param_F32(obj, "x")?;
            let y = Param_F32(obj, "y")?;
            Ok(if cmd == "goto" { ControlCmd::Goto { x, y } } else { ControlCmd::Circle { x, y } })
        }
        "stop" => Ok(ControlCmd::Stop),
        "mode" => match obj.get("value").and_then(|v| v.as_str()) {
            Some("auto") => Ok(ControlCmd::Mode { auto: true }),
            Some("manual") => Ok(ControlCmd::Mode { auto: false }),
            Some(other) => Err(format!("mode.value 非法：'{other}'（只能是 auto 或 manual）")),
            None => Err("缺少 value 字段（应为 \"auto\" 或 \"manual\"）".to_string()),
        },
        "manual" => {
            let a = obj
                .get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    "缺少 action 字段（forward|backward|spin_left|spin_right|stop）".to_string()
                })?
                .trim();
            match a {
                "forward" | "backward" | "spin_left" | "spin_right" | "stop" => {
                    Ok(ControlCmd::Manual { action: a.to_string() })
                }
                other => Err(format!(
                    "manual.action 非法：'{other}'（forward|backward|spin_left|spin_right|stop）"
                )),
            }
        }
        "status" => Ok(ControlCmd::Status),
        "help" => Ok(ControlCmd::Help),
        other => Err(format!(
            "未知 cmd '{other}'（支持 goto/circle/stop/mode/manual/status/help）"
        )),
    }
}

/// `x`/`y` 取数：缺失 / 非数字 / 非有限 / 越界 各自不同错误文本
fn Param_F32(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<f32, String> {
    let v = obj.get(key).ok_or_else(|| format!("缺少 {key} 字段"))?;
    let f = v.as_f64().ok_or_else(|| format!("{key} 必须是数字"))? as f32;
    if !f.is_finite() {
        // `1e39` 反序列化成 f64 合法，但 as f32 变 inf → 必须当非法（R11）
        return Err(format!("{key} 必须是有限数"));
    }
    if !(CONTROL_WORLD_MIN..CONTROL_WORLD_MAX).contains(&f) {
        // 右开区间：`128.0` 必须拒绝
        return Err(format!("{key} 超范围 [{CONTROL_WORLD_MIN},{CONTROL_WORLD_MAX})"));
    }
    Ok(f)
}

// ================================================================ 命令 → 注入（纯函数缝）

/// `stop` 按当前模式选命令（否则会被 `main_loop` 的模式门闩吞掉，§七.4 / §七.8-3）
fn Stop_Command(mode: OpMode) -> Command {
    match mode {
        OpMode::Auto => Command::Auto(AutoCmd::Set(Vec::new())),
        OpMode::Manual => Command::Manual(ManualCmd::Stop),
    }
}

/// 真机忽略载荷（`stm32/mod.rs:134-147`），一律传 0，实际速度由 `[chassis]` 配置决定。
/// 模式不匹配 → `Err`（控制口据此**不发**，避免「静默无效」，§七.4）
fn Manual_Command(mode: OpMode, action: &str) -> Result<Command, String> {
    if mode != OpMode::Manual {
        return Err("当前为 Auto 模式，manual 命令不会生效".to_string());
    }
    const P: i16 = 0;
    Ok(Command::Manual(match action {
        "forward" => ManualCmd::Forward(P),
        "backward" => ManualCmd::Backward(P),
        "spin_left" => ManualCmd::SpinLeft(P),
        "spin_right" => ManualCmd::SpinRight(P),
        "stop" => ManualCmd::Stop,
        other => return Err(format!("manual.action 非法：'{other}'")),
    }))
}

/// 纯函数（可单测）：与 §七.4 的示例**字段完全一致**（9 个键，不多不少）
fn Status_Data(
    peer_id: &str,
    assembled: bool,
    avatar_id: Option<u16>,
    mode: OpMode,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    rtk_fixed: bool,
) -> Value {
    json!({
        "peer_id": peer_id,                 // 本机 libp2p PeerId 的 base58
        "assembled": assembled,             // WorldLinkHandle::State() == Assembled
        "avatar_id": avatar_id,             // Option<u16> → 数字 / null
        "mode": Mode_Name(mode),
        "x": x, "y": y, "z": z, "yaw": yaw,
        "rtk_fixed": rtk_fixed,             // 世界无 RTK → 恒 false（§七；§六.2.5）
    })
}

fn Help_Data() -> Value {
    json!({ "commands": [
        {"cmd":"goto",   "args":["x","y"],  "note":"单车前往目标点（米，[0,128)）"},
        {"cmd":"circle", "args":["x","y"],  "note":"单车围圈（x/y 为圆心，米）"},
        {"cmd":"stop",   "args":[],         "note":"按当前模式停车（Auto→清空任务，Manual→Manual(Stop)）"},
        {"cmd":"mode",   "args":["value"],  "note":"value = auto | manual"},
        {"cmd":"manual", "args":["action"], "note":"forward|backward|spin_left|spin_right|stop（仅 Manual 模式）"},
        {"cmd":"status", "args":[],         "note":"回状态 JSON（只读）"},
        {"cmd":"help",   "args":[],         "note":"本列表"},
    ]})
}

fn Ok_Reply(cmd: &str) -> Value {
    json!({"ok": true, "cmd": cmd})
}

fn Ok_Reply_Data(cmd: &str, data: Value) -> Value {
    json!({"ok": true, "cmd": cmd, "data": data})
}

fn Err_Reply(cmd: &str, err: impl Into<String>) -> Value {
    let err: String = err.into();
    json!({"ok": false, "cmd": cmd, "err": err})
}

fn Mode_Name(mode: OpMode) -> &'static str {
    match mode {
        OpMode::Auto => "auto",
        OpMode::Manual => "manual",
    }
}

// ================================================================ 分派与投递

/// 文本帧 → 回包（**永远有回包**，绝不静默）
async fn Handle_Text(
    text: &str,
    robot: &Arc<Robot>,
    world: Option<&WorldLinkHandle>,
    peer_id: &str,
) -> Value {
    // 尽力从原始 JSON 取 "cmd"（错误回包也要带）；取不到 → "?"
    let name = Reply_Cmd_Name(text);
    match Parse_Control(text) {
        Err(e) => {
            warn!("[control] 拒绝请求：{e}");
            Err_Reply(&name, e)
        }
        Ok(cmd) => Handle_Cmd(cmd, robot, world, peer_id).await,
    }
}

/// 回包的 `cmd` 字段：解析失败时也要能回（例如非法 JSON → `"?"`）
fn Reply_Cmd_Name(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("cmd").and_then(|c| c.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| "?".to_string())
}

async fn Handle_Cmd(
    cmd: ControlCmd,
    robot: &Arc<Robot>,
    world: Option<&WorldLinkHandle>,
    peer_id: &str,
) -> Value {
    // Copy 出来，读锁立即释放（绝不持锁跨 await —— 挤压设备端 RX）
    let mode = *robot.op_mode.read().await;
    match cmd {
        ControlCmd::Goto { x, y } => {
            if mode != OpMode::Auto {
                warn!(
                    "[control] 当前为 Manual 模式，goto 会被 main_loop 忽略（先发 {{\"cmd\":\"mode\",\"value\":\"auto\"}}）"
                );
            }
            info!("[control] goto ({x:.1}, {y:.1}) → cmd_tx");
            let c = Command::Auto(AutoCmd::Set(vec![Mission::Goto { x, y, members: Vec::new() }]));
            Dispatch(robot, c, "goto").await
        }
        ControlCmd::Circle { x, y } => {
            if mode != OpMode::Auto {
                warn!(
                    "[control] 当前为 Manual 模式，circle 会被 main_loop 忽略（先发 {{\"cmd\":\"mode\",\"value\":\"auto\"}}）"
                );
            }
            info!("[control] circle ({x:.1}, {y:.1}) → cmd_tx");
            let c =
                Command::Auto(AutoCmd::Set(vec![Mission::Circle { x, y, members: Vec::new() }]));
            Dispatch(robot, c, "circle").await
        }
        ControlCmd::Stop => {
            let kind = match mode {
                OpMode::Auto => "Auto(Set(vec![]))",
                OpMode::Manual => "Manual(Stop)",
            };
            info!("[control] stop（{} 模式 → {kind}）→ cmd_tx", Mode_Name(mode));
            Dispatch(robot, Stop_Command(mode), "stop").await
        }
        ControlCmd::Mode { auto } => {
            let want = if auto { OpMode::Auto } else { OpMode::Manual };
            let c = if auto {
                Command::Mode(ModeCmd::SwitchToAuto)
            } else {
                Command::Mode(ModeCmd::SwitchToManual)
            };
            info!("[control] mode → {} → cmd_tx", Mode_Name(want));
            let reply = Dispatch(robot, c, "mode").await;
            if reply["ok"].as_bool().unwrap_or(false) {
                // 补充（不改协议）：等 op_mode 真正切换（≤500 ms），
                // 消除「切完立刻发 manual 被按旧模式误拒」的竞态（R5）
                let _ = tokio::time::timeout(CONTROL_MODE_SETTLE, async {
                    loop {
                        if *robot.op_mode.read().await == want {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await;
            }
            reply
        }
        ControlCmd::Manual { action } => match Manual_Command(mode, &action) {
            Err(e) => {
                info!("[control] 拒绝 manual：{e}");
                Err_Reply("manual", e)
            }
            Ok(c) => {
                info!("[control] manual {action} → cmd_tx");
                Dispatch(robot, c, "manual").await
            }
        },
        ControlCmd::Status => {
            // 先取 RobotState 快照并 drop 锁（base 的纪律：持锁跨 await 会挤压设备端 RX）
            let (x, y, z, yaw, rtk_fixed) = {
                let s = robot.robot_state.read().await;
                (s.x, s.y, s.z, s.attitude.yaw, s.rtk_fixed)
            };
            let (assembled, avatar_id) = match world {
                Some(w) => (w.State().await == WorldState::Assembled, w.Avatar_Id().await),
                None => (false, None), // 不连世界：assembled=false、avatar=null
            };
            debug!("[control] status assembled={assembled} avatar={avatar_id:?}");
            Ok_Reply_Data(
                "status",
                Status_Data(peer_id, assembled, avatar_id, mode, x, y, z, yaw, rtk_fixed),
            )
        }
        ControlCmd::Help => Ok_Reply_Data("help", Help_Data()),
    }
}

/// 投递一条 `Command` 进 `robot_cmd_tx`（唯一副作用）；1 s 超时防止连接 task 被通道拥塞挂死
async fn Dispatch(robot: &Arc<Robot>, cmd: Command, name: &str) -> Value {
    match tokio::time::timeout(CONTROL_SEND_TIMEOUT, robot.robot_cmd_tx.send(cmd)).await {
        Ok(Ok(())) => Ok_Reply(name),
        Ok(Err(_)) => Err_Reply(name, "cmd 通道关闭"), // §七.6 要点 3
        Err(_) => Err_Reply(
            name,
            format!("cmd 通道拥塞（{} ms 未投递）", CONTROL_SEND_TIMEOUT.as_millis()),
        ),
    }
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------- 四种合法命令

    #[test]
    fn test_parse_valid_goto() {
        assert_eq!(
            Parse_Control(r#"{"cmd":"goto","x":36,"y":92}"#),
            Ok(ControlCmd::Goto { x: 36.0, y: 92.0 })
        );
        // 浮点 + 边界：x=0 合法、x=127.999 合法（右开区间只拒 128.0）
        assert_eq!(
            Parse_Control(r#"{"cmd":"goto","x":0,"y":127.999}"#),
            Ok(ControlCmd::Goto { x: 0.0, y: 127.999 })
        );
        assert_eq!(
            Parse_Control(r#"{"cmd":"goto","x":64.5,"y":63.25}"#),
            Ok(ControlCmd::Goto { x: 64.5, y: 63.25 })
        );
        // 多余空白容忍
        assert_eq!(
            Parse_Control("  {\"cmd\" : \"goto\" , \"x\" : 1 , \"y\" : 2 }  "),
            Ok(ControlCmd::Goto { x: 1.0, y: 2.0 })
        );
    }

    #[test]
    fn test_parse_valid_circle() {
        assert_eq!(
            Parse_Control(r#"{"cmd":"circle","x":40,"y":80}"#),
            Ok(ControlCmd::Circle { x: 40.0, y: 80.0 })
        );
    }

    #[test]
    fn test_parse_valid_mode_auto_manual() {
        assert_eq!(
            Parse_Control(r#"{"cmd":"mode","value":"auto"}"#),
            Ok(ControlCmd::Mode { auto: true })
        );
        assert_eq!(
            Parse_Control(r#"{"cmd":"mode","value":"manual"}"#),
            Ok(ControlCmd::Mode { auto: false })
        );
    }

    #[test]
    fn test_parse_valid_manual_all_actions() {
        for a in ["forward", "backward", "spin_left", "spin_right", "stop"] {
            let text = format!(r#"{{"cmd":"manual","action":"{a}"}}"#);
            assert_eq!(
                Parse_Control(&text),
                Ok(ControlCmd::Manual { action: a.to_string() }),
                "action='{a}' 必须被接受且原样保留"
            );
        }
    }

    #[test]
    fn test_parse_valid_stop_status_help() {
        assert_eq!(Parse_Control(r#"{"cmd":"stop"}"#), Ok(ControlCmd::Stop));
        assert_eq!(Parse_Control(r#"{"cmd":"status"}"#), Ok(ControlCmd::Status));
        assert_eq!(Parse_Control(r#"{"cmd":"help"}"#), Ok(ControlCmd::Help));
    }

    // ---------------------------------------------------------- 非法请求（错误文本）

    #[test]
    fn test_parse_invalid_json() {
        for bad in ["not json", "{", "", "{\"cmd\":}"] {
            let e = Parse_Control(bad).expect_err("非法 JSON 必须报错");
            assert!(e.contains("非法 JSON"), "错误文本应含「非法 JSON」：{e}");
        }
    }

    #[test]
    fn test_parse_not_object() {
        for bad in ["[1,2]", "\"goto\"", "42", "null"] {
            let e = Parse_Control(bad).expect_err("顶层不是对象必须报错");
            assert!(e.contains("请求必须是 JSON 对象"), "错误文本：{e}");
        }
    }

    #[test]
    fn test_parse_missing_cmd() {
        for bad in [r#"{"x":1}"#, r#"{}"#, r#"{"cmd":123}"#, r#"{"cmd":null}"#] {
            let e = Parse_Control(bad).expect_err("缺 cmd 必须报错");
            assert!(e.contains("缺少 cmd 字段"), "错误文本：{e}");
        }
    }

    #[test]
    fn test_parse_unknown_cmd() {
        let e = Parse_Control(r#"{"cmd":"fly"}"#).expect_err("未知 cmd 必须报错");
        assert!(e.contains("未知 cmd 'fly'"), "错误文本：{e}");
        assert!(e.contains("goto/circle/stop/mode/manual/status/help"), "应列出支持的命令：{e}");
    }

    #[test]
    fn test_parse_goto_missing_x() {
        let e = Parse_Control(r#"{"cmd":"goto","y":92}"#).expect_err("缺 x 必须报错");
        assert_eq!(e, "缺少 x 字段");
        // circle 同口径
        let e = Parse_Control(r#"{"cmd":"circle","y":92}"#).expect_err("缺 x 必须报错");
        assert_eq!(e, "缺少 x 字段");
    }

    #[test]
    fn test_parse_goto_missing_y() {
        let e = Parse_Control(r#"{"cmd":"goto","x":36}"#).expect_err("缺 y 必须报错");
        assert_eq!(e, "缺少 y 字段");
        let e = Parse_Control(r#"{"cmd":"circle","x":36}"#).expect_err("缺 y 必须报错");
        assert_eq!(e, "缺少 y 字段");
    }

    #[test]
    fn test_parse_x_not_number() {
        let e = Parse_Control(r#"{"cmd":"goto","x":"36","y":92}"#).expect_err("x 非数字必须报错");
        assert_eq!(e, "x 必须是数字");
        let e = Parse_Control(r#"{"cmd":"goto","x":true,"y":92}"#).expect_err("x 非数字必须报错");
        assert_eq!(e, "x 必须是数字");
    }

    #[test]
    fn test_parse_x_out_of_range() {
        for bad in ["-1", "128", "200", "-0.001", "1e3"] {
            let text = format!(r#"{{"cmd":"goto","x":{bad},"y":92}}"#);
            let e = Parse_Control(&text).expect_err("x 越界必须报错");
            assert_eq!(e, "x 超范围 [0,128)", "x={bad} 的错误文本");
        }
        // y 同口径
        for bad in ["-1", "128", "200"] {
            let text = format!(r#"{{"cmd":"goto","x":36,"y":{bad}}}"#);
            let e = Parse_Control(&text).expect_err("y 越界必须报错");
            assert_eq!(e, "y 超范围 [0,128)", "y={bad} 的错误文本");
        }
    }

    #[test]
    fn test_parse_x_not_finite() {
        // 1e39 在 f64 里合法，as f32 → inf；必须当「有限数」错误而不是越界
        let e = Parse_Control(r#"{"cmd":"goto","x":1e39,"y":92}"#).expect_err("非有限必须报错");
        assert_eq!(e, "x 必须是有限数");
        let e = Parse_Control(r#"{"cmd":"goto","x":36,"y":-1e39}"#).expect_err("非有限必须报错");
        assert_eq!(e, "y 必须是有限数");
    }

    #[test]
    fn test_parse_mode_bad_value() {
        let e = Parse_Control(r#"{"cmd":"mode","value":"bogus"}"#).expect_err("非法 value 必须报错");
        assert!(e.contains("mode.value 非法：'bogus'"), "错误文本：{e}");
        assert!(e.contains("auto 或 manual"), "应指路：{e}");
        // 缺 value / value 非字符串
        let e = Parse_Control(r#"{"cmd":"mode"}"#).expect_err("缺 value 必须报错");
        assert!(e.contains("缺少 value 字段"), "错误文本：{e}");
        let e = Parse_Control(r#"{"cmd":"mode","value":1}"#).expect_err("value 非字符串必须报错");
        assert!(e.contains("缺少 value 字段"), "错误文本：{e}");
    }

    #[test]
    fn test_parse_manual_bad_action() {
        let e = Parse_Control(r#"{"cmd":"manual","action":"jump"}"#).expect_err("非法 action 必须报错");
        assert!(e.contains("manual.action 非法：'jump'"), "错误文本：{e}");
        let e = Parse_Control(r#"{"cmd":"manual"}"#).expect_err("缺 action 必须报错");
        assert!(e.contains("缺少 action 字段"), "错误文本：{e}");
    }

    #[test]
    fn test_parse_members_rejected() {
        // §七.8-6：群发不支持，明确拒绝（不假装成功）
        let e = Parse_Control(r#"{"cmd":"goto","x":36,"y":92,"members":["abc"]}"#)
            .expect_err("非空 members 必须拒绝");
        assert_eq!(e, "members 不支持，请用地面站");
        let e = Parse_Control(r#"{"cmd":"circle","x":36,"y":92,"members":[1,2]}"#)
            .expect_err("非空 members 必须拒绝");
        assert_eq!(e, "members 不支持，请用地面站");
        // 空数组 / 缺失 = 单车语义，合法
        assert_eq!(
            Parse_Control(r#"{"cmd":"goto","x":36,"y":92,"members":[]}"#),
            Ok(ControlCmd::Goto { x: 36.0, y: 92.0 })
        );
        assert_eq!(
            Parse_Control(r#"{"cmd":"goto","x":36,"y":92}"#),
            Ok(ControlCmd::Goto { x: 36.0, y: 92.0 })
        );
    }

    // ---------------------------------------------------------- 命令 → 注入

    #[test]
    fn test_stop_command_by_mode() {
        match Stop_Command(OpMode::Auto) {
            Command::Auto(AutoCmd::Set(v)) => assert!(v.is_empty(), "Auto 模式 stop = 清空任务队列"),
            other => panic!("Auto 模式应回 Auto(Set(vec![]))，实际 {other:?}"),
        }
        match Stop_Command(OpMode::Manual) {
            Command::Manual(ManualCmd::Stop) => {}
            other => panic!("Manual 模式应回 Manual(Stop)，实际 {other:?}"),
        }
    }

    #[test]
    fn test_manual_command_requires_manual_mode() {
        let e = Manual_Command(OpMode::Auto, "forward").expect_err("Auto 模式必须拒绝 manual");
        assert_eq!(e, "当前为 Auto 模式，manual 命令不会生效");
        // 且绝不把命令包出来（返回的是 Err，调用方不发）
        for a in ["forward", "backward", "spin_left", "spin_right", "stop"] {
            assert!(Manual_Command(OpMode::Manual, a).is_ok(), "Manual 模式下 '{a}' 必须放行");
        }
        // 兜底：未知 action 即使在 Manual 模式也是 Err
        assert!(Manual_Command(OpMode::Manual, "jump").is_err());
    }

    #[test]
    fn test_manual_command_payload_zero() {
        // 真机忽略载荷（用 [chassis] 配置速度）→ 一律传 0
        match Manual_Command(OpMode::Manual, "forward").expect("Manual 放行") {
            Command::Manual(ManualCmd::Forward(v)) => assert_eq!(v, 0),
            other => panic!("forward 应映射 Forward(0)，实际 {other:?}"),
        }
        match Manual_Command(OpMode::Manual, "backward").expect("Manual 放行") {
            Command::Manual(ManualCmd::Backward(v)) => assert_eq!(v, 0),
            other => panic!("backward 应映射 Backward(0)，实际 {other:?}"),
        }
        match Manual_Command(OpMode::Manual, "spin_left").expect("Manual 放行") {
            Command::Manual(ManualCmd::SpinLeft(v)) => assert_eq!(v, 0),
            other => panic!("spin_left 应映射 SpinLeft(0)，实际 {other:?}"),
        }
        match Manual_Command(OpMode::Manual, "spin_right").expect("Manual 放行") {
            Command::Manual(ManualCmd::SpinRight(v)) => assert_eq!(v, 0),
            other => panic!("spin_right 应映射 SpinRight(0)，实际 {other:?}"),
        }
        match Manual_Command(OpMode::Manual, "stop").expect("Manual 放行") {
            Command::Manual(ManualCmd::Stop) => {}
            other => panic!("stop 应映射 Manual(Stop)，实际 {other:?}"),
        }
    }

    // ---------------------------------------------------------- 回包形态

    #[test]
    fn test_status_data_schema() {
        let v = Status_Data("12D3KooW", true, Some(1), OpMode::Auto, 64.02, 63.98, 0.0, 0.12, false);
        let obj = v.as_object().expect("status.data 必须是 JSON 对象");
        // 恰好 9 个键，不多不少（§七.4 的示例）
        assert_eq!(obj.len(), 9, "status.data 键数必须是 9，实际 {obj:?}");
        for k in ["peer_id", "assembled", "avatar_id", "mode", "x", "y", "z", "yaw", "rtk_fixed"] {
            assert!(obj.contains_key(k), "status.data 缺键 {k}：{obj:?}");
        }
        assert_eq!(v["peer_id"], json!("12D3KooW"));
        assert_eq!(v["assembled"], json!(true));
        assert_eq!(v["avatar_id"], json!(1));
        assert_eq!(v["mode"], json!("auto"));
        assert_eq!(v["rtk_fixed"], json!(false));

        // 未装配 / 未连世界：avatar_id = None → JSON null
        let v = Status_Data("p", false, None, OpMode::Manual, 1.0, 2.0, 3.0, 4.0, false);
        assert_eq!(v["avatar_id"], Value::Null, "None 必须序列化成 null");
        assert_eq!(v["assembled"], json!(false));
        assert_eq!(v["mode"], json!("manual"));
    }

    #[test]
    fn test_help_data_has_all_commands() {
        let v = Help_Data();
        let arr = v["commands"].as_array().expect("help.data.commands 必须是数组");
        assert_eq!(arr.len(), 7, "help 必须列 7 条命令");
        let names: Vec<&str> = arr.iter().filter_map(|c| c["cmd"].as_str()).collect();
        assert_eq!(
            names,
            vec!["goto", "circle", "stop", "mode", "manual", "status", "help"]
        );
        // 每条都有 args 数组
        for c in arr {
            assert!(c["args"].is_array(), "每条命令都要有 args：{c:?}");
        }
    }

    #[test]
    fn test_reply_shapes() {
        let ok = Ok_Reply("goto");
        let obj = ok.as_object().expect("对象");
        assert_eq!(obj.len(), 2, "ok 回包只有 ok + cmd：{obj:?}");
        assert_eq!(ok["ok"], json!(true));
        assert_eq!(ok["cmd"], json!("goto"));
        assert!(ok.get("err").is_none());

        let ok = Ok_Reply_Data("status", json!({"a": 1}));
        assert_eq!(ok["ok"], json!(true));
        assert_eq!(ok["cmd"], json!("status"));
        assert_eq!(ok["data"]["a"], json!(1));

        let err = Err_Reply("goto", "x 超范围 [0,128)");
        let obj = err.as_object().expect("对象");
        assert_eq!(obj.len(), 3, "err 回包有 ok + cmd + err：{obj:?}");
        assert_eq!(err["ok"], json!(false));
        assert_eq!(err["cmd"], json!("goto"));
        assert_eq!(err["err"], json!("x 超范围 [0,128)"));

        // impl Into<String>：&str 与 String 都能进
        let err = Err_Reply("manual", "cmd 通道关闭".to_string());
        assert_eq!(err["err"], json!("cmd 通道关闭"));
    }

    #[test]
    fn test_reply_cmd_name_fallback() {
        assert_eq!(Reply_Cmd_Name("not json"), "?");
        assert_eq!(Reply_Cmd_Name("{"), "?");
        assert_eq!(Reply_Cmd_Name(r#"{"x":1}"#), "?");
        assert_eq!(Reply_Cmd_Name(r#"{"cmd":123}"#), "?");
        assert_eq!(Reply_Cmd_Name(r#"{"cmd":"goto","x":36,"y":92}"#), "goto");
        assert_eq!(Reply_Cmd_Name(r#"{"cmd":"bogus"}"#), "bogus");
    }

    // ---------------------------------------------------------- 绑定的两条分支（只 warn 不 panic）

    #[tokio::test]
    async fn test_bind_listener_port_taken() {
        // 先占 127.0.0.1:0 拿到一个真实端口，再拿同端口绑定 → 必须 Err（而不是 panic）
        let holder = TcpListener::bind(("127.0.0.1", 0)).await.expect("占位绑定");
        let port = holder.local_addr().expect("local_addr").port();
        assert!(port > 0);
        let e = Bind_Listener("127.0.0.1", port).await.expect_err("端口被占必须 Err");
        assert!(!e.is_empty(), "Err 必须带原因（进 warn! 日志）");
        drop(holder); // 释放后再绑应当成功（证明失败原因是端口占用）
        assert!(Bind_Listener("127.0.0.1", port).await.is_ok(), "释放后应能绑定");
    }

    #[tokio::test]
    async fn test_bind_listener_ok() {
        // port = 0：OS 分配，local_addr 打真实端口（自动化测试避开端口冲突）
        let l = Bind_Listener("127.0.0.1", 0).await.expect("port=0 必须成功");
        assert!(l.local_addr().expect("local_addr").port() > 0);
    }
}
