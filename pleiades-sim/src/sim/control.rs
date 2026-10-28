//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-27

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

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use pleiades_base::robot::core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
use pleiades_base::robot::core::mission_queue::TaskId;
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

/// 一条控制口命令最多带多少条任务（防手抖灌爆队列；`TASK_SET` 的上限是 u8，这里取小值）
const MAX_MISSIONS_PER_CMD: usize = 16;

// ================================================================ 请求模型

/// 控制口的请求（[`Parse_Control`] 的产物）
#[derive(Debug, Clone, PartialEq)]
pub enum ControlCmd {
    Goto { x: f32, y: f32 },
    Circle { x: f32, y: f32 },
    /// 整体替换队列（= `AutoCmd::Set`）：空数组 = 取消（与 `stop` 等价）
    Queue { missions: Vec<Mission> },
    /// 追加队尾（= `AutoCmd::Append`，§十一 #15）
    Append { missions: Vec<Mission> },
    /// 插队头（= `AutoCmd::Preempt`，§十一 #15/#17）
    Preempt { missions: Vec<Mission> },
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
    monitor: Arc<TaskMonitor>,
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
    tokio::spawn(async move { Run(r, robot, world, Arc::new(peer_id), monitor, cancel).await });
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
    monitor: Arc<TaskMonitor>,
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
                    monitor.clone(),
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
    monitor: Arc<TaskMonitor>,
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
                let reply = Handle_Text(&t, &robot, world.as_ref(), &peer_id, &monitor).await;
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
        "queue" | "append" | "preempt" => {
            let missions = Param_Missions(obj)?;
            if missions.is_empty() && cmd != "queue" {
                return Err(format!(
                    "{cmd} 的 missions 不得为空（取消请用 {{\"cmd\":\"queue\",\"missions\":[]}} 或 stop）"
                ));
            }
            Ok(match cmd.as_str() {
                "append" => ControlCmd::Append { missions },
                "preempt" => ControlCmd::Preempt { missions },
                _ => ControlCmd::Queue { missions }, // 空数组合法 = 取消
            })
        }
        "goto" | "circle" => {
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
            "未知 cmd '{other}'（支持 goto/circle/queue/append/preempt/stop/mode/manual/status/help）"
        )),
    }
}

/// `x`/`y` 取数：缺失 / 非数字 / 非有限 / 越界 各自不同错误文本
fn Param_F32(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<f32, String> {
    Param_F32_As(obj, key, key)
}

/// 同 [`Param_F32`]，但**错误文本用 `label`**（`missions[0].x` 这类带下标的写法：
/// 查的键是 `x`，报给人看的是 `missions[0].x`）
fn Param_F32_As(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    label: &str,
) -> Result<f32, String> {
    let v = obj.get(key).ok_or_else(|| format!("缺少 {label} 字段"))?;
    let f = v.as_f64().ok_or_else(|| format!("{label} 必须是数字"))? as f32;
    if !f.is_finite() {
        // `1e39` 反序列化成 f64 合法，但 as f32 变 inf → 必须当非法（R11）
        return Err(format!("{label} 必须是有限数"));
    }
    if !(CONTROL_WORLD_MIN..CONTROL_WORLD_MAX).contains(&f) {
        // 右开区间：`128.0` 必须拒绝
        return Err(format!("{label} 超范围 [{CONTROL_WORLD_MIN},{CONTROL_WORLD_MAX})"));
    }
    Ok(f)
}

/// `missions` 解析：必须是数组，逐条 `{x,y}`（同一套范围/有限性校验，错误文本带下标）
fn Param_Missions(obj: &serde_json::Map<String, serde_json::Value>) -> Result<Vec<Mission>, String> {
    let arr = match obj.get("missions") {
        Some(v) => v.as_array().ok_or_else(|| "missions 必须是数组".to_string())?,
        None => return Err("缺少 missions 字段（应为 [{x,y}, ...]）".to_string()),
    };
    if arr.len() > MAX_MISSIONS_PER_CMD {
        return Err(format!(
            "missions 最多 {MAX_MISSIONS_PER_CMD} 条，实际 {}",
            arr.len()
        ));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, m) in arr.iter().enumerate() {
        let o = m
            .as_object()
            .ok_or_else(|| format!("missions[{i}] 必须是对象 {{x,y}}"))?;
        let x = Param_F32_As(o, "x", &format!("missions[{i}].x"))?;
        let y = Param_F32_As(o, "y", &format!("missions[{i}].y"))?;
        out.push(Mission::Goto { x, y });
    }
    Ok(out)
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

/// 三种队列命令 → `AutoCmd`（纯函数：控制口只负责投递，语义映射必须可用单测卡住）
fn Queue_Auto_Cmd(kind: &str, missions: Vec<Mission>) -> Command {
    Command::Auto(match kind {
        "append" => AutoCmd::Append(missions),
        "preempt" => AutoCmd::Preempt(missions),
        _ => AutoCmd::Set(missions), // "queue"
    })
}

/// 纯函数（可单测）：§七.4 的 9 个键 + §12.8.2 第 4 条的 4 个观测键（共 13 个）
#[allow(clippy::too_many_arguments)]
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
    queue_len: usize,
    task_id: Option<u64>,
    task_ticks: u64,
    nav_status: &str,
) -> Value {
    json!({
        "peer_id": peer_id,                 // 本机 libp2p PeerId 的 base58
        "assembled": assembled,             // WorldLinkHandle::State() == Assembled
        "avatar_id": avatar_id,             // Option<u16> → 数字 / null
        "mode": Mode_Name(mode),
        "x": x, "y": y, "z": z, "yaw": yaw,
        "rtk_fixed": rtk_fixed,             // 世界无 RTK → 恒 false（§七；§六.2.5）
        // ---- Task 36 §12.8.2 第 4 条（队列语义判定用）----
        "queue_len": queue_len,             // 含正在执行的队首（§十一 #16）
        "task_id": task_id,                 // 队首 TaskId；空队列 null
        "task_ticks": task_ticks,           // 队首任务累计 tick（挂起期间冻结）
        "nav_status": nav_status,           // idle | navigating | arrived | blocked
    })
}

fn Help_Data() -> Value {
    json!({ "commands": [
        {"cmd":"goto",    "args":["x","y"],    "note":"单车前往目标点（米，[0,128)）"},
        {"cmd":"circle",  "args":["x","y"],    "note":"单车围圈（x/y 为圆心，米）"},
        {"cmd":"queue",   "args":["missions"], "note":"整体替换队列（missions=[] = 取消）"},
        {"cmd":"append",  "args":["missions"], "note":"追加队尾（当前任务不受影响）"},
        {"cmd":"preempt", "args":["missions"], "note":"插队头（当前任务挂起、可恢复）"},
        {"cmd":"stop",    "args":[],           "note":"按当前模式停车（Auto→清空任务，Manual→Manual(Stop)）"},
        {"cmd":"mode",    "args":["value"],    "note":"value = auto | manual"},
        {"cmd":"manual",  "args":["action"],   "note":"forward|backward|spin_left|spin_right|stop（仅 Manual 模式）"},
        {"cmd":"status",  "args":[],           "note":"回状态 JSON（含 queue_len/task_id/task_ticks/nav_status）"},
        {"cmd":"help",    "args":[],           "note":"本列表"},
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

// ================================================================ 任务观测器（§12.8.2 第 4 条）

/// `status` 需要、但 `MissionQueue` 里给不出的两个观测值，由设备端每拍写、控制口只读。
///
/// - **`task_ticks`**：按 `TaskId` 累计"该任务的树被 tick 了几拍"。挂起期间不 tick → 值**冻结**；
///   恢复后从冻结值继续。用例 5/6 就靠这一条区分"同一实例恢复"与"重建后从 0 开始"
///   （§12.8.4：判定只认 `task_ticks`）。
/// - **`nav_status`**：最近一拍的导航器状态（`idle|navigating|arrived|blocked`），只作观测/取证。
///
/// 写者唯一（`SimDeviceHandler::on_tick`），读者唯一（控制口 `status`）。
/// 用 `std::sync::Mutex` 而不是 tokio 锁：临界区里没有 `await`，也不该把控制口的读
/// 塞进 async 排队；锁中毒不 panic（取回内层继续用，与本 crate 其它锁同纪律）。
#[derive(Debug)]
pub struct TaskMonitor {
    inner: StdMutex<MonitorInner>,
}

#[derive(Debug)]
struct MonitorInner {
    /// `TaskId.0` → 累计 tick 数
    ticks: HashMap<u64, u64>,
    /// 导航状态编码：0 idle / 1 navigating / 2 arrived / 3 blocked
    nav: u8,
}

impl Default for TaskMonitor {
    fn default() -> Self {
        Self { inner: StdMutex::new(MonitorInner { ticks: HashMap::new(), nav: 0 }) }
    }
}

impl TaskMonitor {
    /// 锁中毒不 panic（与本 crate 其它锁同纪律）
    fn Lock(&self) -> std::sync::MutexGuard<'_, MonitorInner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// 该任务的树被 tick 了**一拍**（`on_tick` 里树真的推进过才调；无活动树不调）
    pub fn Note_Tick(&self, id: TaskId) {
        *self.Lock().ticks.entry(id.0).or_insert(0) += 1;
    }

    /// 该任务拿到终值（出队后清一条，避免计数表随任务数无限长）
    pub fn Forget(&self, id: TaskId) {
        self.Lock().ticks.remove(&id.0);
    }

    /// `reset`（replace / 切模式）→ 整表清空
    pub fn Clear(&self) {
        self.Lock().ticks.clear();
    }

    /// 该 `TaskId` 的累计 tick 数（不存在 / 还没开始 → 0）
    pub fn Ticks_Of(&self, id: u64) -> u64 {
        *self.Lock().ticks.get(&id).unwrap_or(&0)
    }

    /// 记录本拍导航状态（只认这 4 个字面量，省掉跨 crate 的枚举依赖）
    pub fn Set_Nav(&self, nav: &'static str) {
        let code = match nav {
            "navigating" => 1,
            "arrived" => 2,
            "blocked" => 3,
            _ => 0, // "idle"
        };
        self.Lock().nav = code;
    }

    /// 最近一次记录的导航状态
    pub fn Nav(&self) -> &'static str {
        match self.Lock().nav {
            1 => "navigating",
            2 => "arrived",
            3 => "blocked",
            _ => "idle",
        }
    }
}

// ================================================================ 分派与投递

/// 文本帧 → 回包（**永远有回包**，绝不静默）
async fn Handle_Text(
    text: &str,
    robot: &Arc<Robot>,
    world: Option<&WorldLinkHandle>,
    peer_id: &str,
    monitor: &TaskMonitor,
) -> Value {
    // 尽力从原始 JSON 取 "cmd"（错误回包也要带）；取不到 → "?"
    let name = Reply_Cmd_Name(text);
    match Parse_Control(text) {
        Err(e) => {
            warn!("[control] 拒绝请求：{e}");
            Err_Reply(&name, e)
        }
        Ok(cmd) => Handle_Cmd(cmd, robot, world, peer_id, monitor).await,
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
    monitor: &TaskMonitor,
) -> Value {
    // Copy 出来，读锁立即释放（绝不持锁跨 await —— 挤压设备端 RX）
    let mode = *robot.op_mode.read().await;
    match cmd {
        ControlCmd::Queue { missions } => {
            if mode != OpMode::Auto {
                warn!("[control] 当前为 Manual 模式，queue 会被 main_loop 忽略（先发 {{\"cmd\":\"mode\",\"value\":\"auto\"}}）");
            }
            info!("[control] queue（整体替换）{} 条 → cmd_tx", missions.len());
            let c = Queue_Auto_Cmd("queue", missions);
            Dispatch(robot, c, "queue").await
        }
        ControlCmd::Append { missions } => {
            if mode != OpMode::Auto {
                warn!("[control] 当前为 Manual 模式，append 会被 main_loop 忽略（先发 {{\"cmd\":\"mode\",\"value\":\"auto\"}}）");
            }
            info!("[control] append {} 条 → cmd_tx", missions.len());
            let c = Queue_Auto_Cmd("append", missions);
            Dispatch(robot, c, "append").await
        }
        ControlCmd::Preempt { missions } => {
            if mode != OpMode::Auto {
                warn!("[control] 当前为 Manual 模式，preempt 会被 main_loop 忽略（先发 {{\"cmd\":\"mode\",\"value\":\"auto\"}}）");
            }
            info!("[control] preempt {} 条 → cmd_tx", missions.len());
            let c = Queue_Auto_Cmd("preempt", missions);
            Dispatch(robot, c, "preempt").await
        }
        ControlCmd::Goto { x, y } => {
            if mode != OpMode::Auto {
                warn!(
                    "[control] 当前为 Manual 模式，goto 会被 main_loop 忽略（先发 {{\"cmd\":\"mode\",\"value\":\"auto\"}}）"
                );
            }
            info!("[control] goto ({x:.1}, {y:.1}) → cmd_tx");
            let c = Command::Auto(AutoCmd::Set(vec![Mission::Goto { x, y }]));
            Dispatch(robot, c, "goto").await
        }
        ControlCmd::Circle { x, y } => {
            if mode != OpMode::Auto {
                warn!(
                    "[control] 当前为 Manual 模式，circle 会被 main_loop 忽略（先发 {{\"cmd\":\"mode\",\"value\":\"auto\"}}）"
                );
            }
            info!("[control] circle ({x:.1}, {y:.1}) → cmd_tx（Task 36：Circle 已并入 Goto，去圆心）");
            let c = Command::Auto(AutoCmd::Set(vec![Mission::Goto { x, y }]));
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
            // 队列快照：队首 = 正在执行的任务位（§十一 #16），`len` 含队首
            let (queue_len, task_id) = {
                let q = robot.mission_queue.read().await;
                (q.len(), q.head().map(|t| t.id.0))
            };
            let task_ticks = task_id.map(|id| monitor.Ticks_Of(id)).unwrap_or(0);
            let nav_status = monitor.Nav();
            let (assembled, avatar_id) = match world {
                Some(w) => (w.State().await == WorldState::Assembled, w.Avatar_Id().await),
                None => (false, None), // 不连世界：assembled=false、avatar=null
            };
            debug!("[control] status queue={queue_len} task={task_id:?} ticks={task_ticks} nav={nav_status}");
            Ok_Reply_Data(
                "status",
                Status_Data(
                    peer_id, assembled, avatar_id, mode, x, y, z, yaw, rtk_fixed,
                    queue_len, task_id, task_ticks, nav_status,
                ),
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
        assert!(e.contains("goto/circle/queue/append/preempt/stop/mode/manual/status/help"), "应列出支持的命令：{e}");
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
    fn test_parse_goto_tolerates_unknown_fields() {
        // Task 36 §十二 Step 3：协议已无群发段，控制口不再校验该字段（未知键一律忽略）
        assert_eq!(
            Parse_Control(r#"{"cmd":"goto","x":36,"y":92,"legacy":["abc"]}"#),
            Ok(ControlCmd::Goto { x: 36.0, y: 92.0 })
        );
        assert_eq!(
            Parse_Control(r#"{"cmd":"goto","x":36,"y":92}"#),
            Ok(ControlCmd::Goto { x: 36.0, y: 92.0 })
        );
    }

    // ---------------------------------------------------------- 队列命令（Task 36 §12.8.2 第 3 条）

    #[test]
    fn test_parse_queue_single() {
        assert_eq!(
            Parse_Control(r#"{"cmd":"queue","missions":[{"x":36,"y":92}]}"#),
            Ok(ControlCmd::Queue { missions: vec![Mission::Goto { x: 36.0, y: 92.0 }] })
        );
    }

    #[test]
    fn test_parse_queue_multi_keeps_order() {
        let got = Parse_Control(r#"{"cmd":"queue","missions":[{"x":1,"y":2},{"x":3.5,"y":4.5}]}"#)
            .expect("两条合法");
        match got {
            ControlCmd::Queue { missions } => assert_eq!(
                missions,
                vec![
                    Mission::Goto { x: 1.0, y: 2.0 },
                    Mission::Goto { x: 3.5, y: 4.5 },
                ]
            ),
            other => panic!("应是 Queue，实际 {other:?}"),
        }
    }

    #[test]
    fn test_parse_queue_empty_is_cancel() {
        assert_eq!(
            Parse_Control(r#"{"cmd":"queue","missions":[]}"#),
            Ok(ControlCmd::Queue { missions: vec![] })
        );
    }

    #[test]
    fn test_parse_append_preempt_reject_empty() {
        for cmd in ["append", "preempt"] {
            let text = format!(r#"{{"cmd":"{cmd}","missions":[]}}"#);
            let e = Parse_Control(&text).expect_err("空 missions 必须报错");
            assert!(e.contains("不得为空"), "{cmd} 错误文本：{e}");
        }
    }

    #[test]
    fn test_parse_missions_missing() {
        for cmd in ["queue", "append", "preempt"] {
            let text = format!(r#"{{"cmd":"{cmd}"}}"#);
            let e = Parse_Control(&text).expect_err("缺 missions 必须报错");
            assert!(e.contains("缺少 missions 字段"), "错误文本：{e}");
        }
    }

    #[test]
    fn test_parse_missions_not_array() {
        let e = Parse_Control(r#"{"cmd":"queue","missions":{"x":1,"y":2}}"#)
            .expect_err("missions 非数组必须报错");
        assert_eq!(e, "missions 必须是数组");
    }

    #[test]
    fn test_parse_missions_bad_elem() {
        let e = Parse_Control(r#"{"cmd":"queue","missions":[1]}"#).expect_err("元素非对象必须报错");
        assert!(e.contains("missions[0] 必须是对象"), "错误文本：{e}");
        let e = Parse_Control(r#"{"cmd":"queue","missions":[{"x":1}]}"#)
            .expect_err("元素缺 y 必须报错");
        assert!(e.contains("missions[0].y"), "错误文本要带下标：{e}");
    }

    #[test]
    fn test_parse_missions_out_of_range() {
        let e = Parse_Control(r#"{"cmd":"append","missions":[{"x":128,"y":0}]}"#)
            .expect_err("x 越界必须报错");
        assert!(e.contains("missions[0].x 超范围 [0,128)"), "错误文本：{e}");
    }

    #[test]
    fn test_parse_missions_too_many() {
        let items: Vec<String> = (0..MAX_MISSIONS_PER_CMD + 1)
            .map(|i| format!(r#"{{"x":{},"y":0}}"#, i))
            .collect();
        let text = format!(r#"{{"cmd":"queue","missions":[{}]}}"#, items.join(","));
        let e = Parse_Control(&text).expect_err("超条数必须报错");
        assert!(e.contains(&format!("最多 {MAX_MISSIONS_PER_CMD} 条")), "错误文本：{e}");
    }

    /// 关键缝：控制口**不许**把 append / preempt 写成 Set（否则整个 step 白做）
    #[test]
    fn test_queue_auto_cmd_mapping() {
        let one = vec![Mission::Goto { x: 1.0, y: 2.0 }];
        match Queue_Auto_Cmd("queue", one.clone()) {
            Command::Auto(AutoCmd::Set(v)) => assert_eq!(v, one),
            other => panic!("queue 应映射 Set，实际 {other:?}"),
        }
        match Queue_Auto_Cmd("append", one.clone()) {
            Command::Auto(AutoCmd::Append(v)) => assert_eq!(v, one),
            other => panic!("append 应映射 Append，实际 {other:?}"),
        }
        match Queue_Auto_Cmd("preempt", one.clone()) {
            Command::Auto(AutoCmd::Preempt(v)) => assert_eq!(v, one),
            other => panic!("preempt 应映射 Preempt，实际 {other:?}"),
        }
    }

    /// `task_ticks` 按 `TaskId` 独立累计：A 冻在不 tick，B 继续涨
    #[test]
    fn test_task_monitor_ticks_are_per_task() {
        let m = TaskMonitor::default();
        for _ in 0..20 { m.Note_Tick(TaskId(1)); }
        for _ in 0..5 { m.Note_Tick(TaskId(2)); }
        assert_eq!(m.Ticks_Of(1), 20);
        assert_eq!(m.Ticks_Of(2), 5);
        // A 挂起（不再 Note_Tick）→ 值冻结；B 继续涨
        for _ in 0..3 { m.Note_Tick(TaskId(2)); }
        assert_eq!(m.Ticks_Of(1), 20, "挂起期间不 tick → 冻结（用例 5/6 的判据基础）");
        assert_eq!(m.Ticks_Of(2), 8);
        assert_eq!(m.Ticks_Of(99), 0, "不存在的任务 → 0");
    }

    #[test]
    fn test_task_monitor_forget_and_clear() {
        let m = TaskMonitor::default();
        m.Note_Tick(TaskId(1));
        m.Note_Tick(TaskId(1));
        m.Forget(TaskId(1));
        assert_eq!(m.Ticks_Of(1), 0, "终值后清一条");
        m.Note_Tick(TaskId(1));
        m.Note_Tick(TaskId(2));
        m.Clear();
        assert_eq!(m.Ticks_Of(1), 0, "reset 后整表清空");
        assert_eq!(m.Ticks_Of(2), 0);
    }

    #[test]
    fn test_task_monitor_nav_default_and_roundtrip() {
        let m = TaskMonitor::default();
        assert_eq!(m.Nav(), "idle", "默认 idle");
        for name in ["navigating", "arrived", "blocked", "idle"] {
            m.Set_Nav(name);
            assert_eq!(m.Nav(), name);
        }
        m.Set_Nav("什么鬼"); // 非法值一律降级为 idle，不 panic
        assert_eq!(m.Nav(), "idle");
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
        let v = Status_Data(
            "12D3KooW", true, Some(1), OpMode::Auto, 64.02, 63.98, 0.0, 0.12, false,
            2, Some(7), 123, "navigating",
        );
        let obj = v.as_object().expect("status.data 必须是 JSON 对象");
        // 恰好 13 个键，不多不少（§七.4 的 9 个 + §12.8.2 第 4 条的 4 个）
        assert_eq!(obj.len(), 13, "status.data 键数必须是 13，实际 {obj:?}");
        for k in [
            "peer_id", "assembled", "avatar_id", "mode", "x", "y", "z", "yaw", "rtk_fixed",
            // Task 36 §12.8.2 第 4 条
            "queue_len", "task_id", "task_ticks", "nav_status",
        ] {
            assert!(obj.contains_key(k), "status.data 缺键 {k}：{obj:?}");
        }
        assert_eq!(v["peer_id"], json!("12D3KooW"));
        assert_eq!(v["assembled"], json!(true));
        assert_eq!(v["avatar_id"], json!(1));
        assert_eq!(v["mode"], json!("auto"));
        assert_eq!(v["rtk_fixed"], json!(false));
        assert_eq!(v["queue_len"], json!(2));
        assert_eq!(v["task_id"], json!(7));
        assert_eq!(v["task_ticks"], json!(123));
        assert_eq!(v["nav_status"], json!("navigating"));

        // 未装配 / 未连世界：avatar_id = None → JSON null；空队列：task_id = null、task_ticks = 0
        let v = Status_Data(
            "p", false, None, OpMode::Manual, 1.0, 2.0, 3.0, 4.0, false, 0, None, 0, "idle",
        );
        assert_eq!(v["avatar_id"], Value::Null, "None 必须序列化成 null");
        assert_eq!(v["assembled"], json!(false));
        assert_eq!(v["mode"], json!("manual"));
        assert_eq!(v["task_id"], Value::Null, "空队列 task_id 必须序列化成 null");
        assert_eq!(v["task_ticks"], json!(0));
        assert_eq!(v["nav_status"], json!("idle"));
    }

    #[test]
    fn test_help_data_has_all_commands() {
        let v = Help_Data();
        let arr = v["commands"].as_array().expect("help.data.commands 必须是数组");
        assert_eq!(arr.len(), 10, "help 必须列 10 条命令");
        let names: Vec<&str> = arr.iter().filter_map(|c| c["cmd"].as_str()).collect();
        assert_eq!(
            names,
            vec!["goto", "circle", "queue", "append", "preempt", "stop", "mode", "manual", "status", "help"]
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
