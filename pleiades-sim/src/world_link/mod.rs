//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-29

#![allow(non_snake_case)]
//! 世界链路：装配状态机 + 事件循环（task_31 §四.5.6 / §四.6）
//!
//! 一条底盘连接 + N 条传感器连接，每个连接一个 tokio task 只负责**收包**，
//! 发包（hello / motion_cmd）与状态全部留在 supervisor，原因见 [`link`] 的模块头（R5）。
//!
//! ```text
//! 世界侧（orion-simulation-world）             sim 侧（本模块）
//!   ws://127.0.0.1:9999
//!   ◀── chassis_hello(peer_id, chassis, params, manifest) ── 连接 #1
//!   ──▶ chassis_ready(avatar_id, spawn, world_time) ─────▶
//!   ◀── sensor_hello(peer_id, slot, sensor, mount, params) ─ 连接 #2..#N+1
//!   ──▶ sensor_ready(slot, sensor_id) ───────────────────▶
//!   ──▶ assembly_ready(world_time) ──────────────────────▶ Assembled
//!   ──▶ pose(8 键, 10 Hz) ──────────────────────────────▶ PoseIngest → RobotState
//!   ──▶ 二进制 lidar 帧(1455 B) ───────────────────────▶ 该 slot 的传感器（就地投递）
//! ```
//!
//! 六态状态机：`Disabled → Connecting → ChassisReady → Assembling → Assembled`，
//! 任一连接断/超时 → `Reconnecting` → 退避 200 ms→3 s（±20% 抖动）→ `Connecting`。
//!
//! 关闭码分流（与 Step 1 的 8 s 握手超时配套）：
//! | 码 | 含义 | 处理 |
//! |---|---|---|
//! | 1002 | 协议错 / hello timeout / bad json | `error!` + 固定 3 s 退避；**连续 3 次同因**停重连 |
//! | 1008 | 传感器等底盘 pending 超时 | `warn!` + 重建全部连接 |
//! | 1011 | 世界侧装配失败（场景缺失等） | `error!` + 停重连（`Disabled`） |
//! | 1001 | peer removed / manager stopped | `info!` + 静默重连 |
//!
//! 纪律：
//! - `WorldShared` **只有 supervisor 写**；写锁持有期间不做任何 `await`
//! - `start()` 侧只 `Spawn()`，绝不 `await` 网络（task_31 §四.2 差异 1）
//! - Step 3 起：`Binary` → 该 slot 的传感器（`SensorEndpoint` 校验帧头 `sensor_id`，
//!   就地投递、不进本 supervisor）；`Pose` → 底盘的 `On_Pose`（`RobotState` 单写者）
//! - Step 4 起：动作上行由 [`MotionSender`] 承担（底盘持有发送端，本 supervisor 持有接收端）；
//!   **`stop` 绝不静默丢**（G1：通道满 → `pending_stop` → 100 ms 内补发）

pub mod catalog;
pub mod config;
pub mod link;
pub mod parts;
pub mod protocol;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::sim::chassis::{Chassis, PoseFields, SpawnFields};
use crate::sim::sensors::{SensorEndpoint, SensorSet, Warn_Throttled};
use crate::world_link::config::ResolvedWorld;

// ================================================================ 动作上行通道（Step 4）

/// 动作上行通道容量（§六.6：mpsc 32）。世界只取最新一条，不补发历史（R20）
pub const MOTION_CHANNEL_CAP: usize = 32;

/// `stop` 补发检查周期（毫秒）：G1 —— 通道满时登记的 stop 最多滞后这么久
pub const STOP_RESEND_MS: u64 = 100;

/// 底盘一侧的动作上行口（supervisor 一侧收 [`mpsc::Receiver`]）
///
/// 三态语义（§六.5.6 / §六.8-5）：
/// - **未装配**（`assembled == false`）：一律丢弃 + `debug!`（不静默，也不刷 `warn`）
/// - **已装配**：`try_send` 进容量 32 的通道；满 → 限流 `warn!` + `Err`
/// - **`stop`**：满时不丢，登记 `pending_stop` 由 supervisor 下一拍（≤100 ms）补发
#[derive(Clone)]
pub struct MotionSender {
    tx: mpsc::Sender<protocol::MotionCmd>,
    /// `seq` 从 1 起（与 Step 2/3 的现状一致，世界忽略它，只做日志对账）
    seq: Arc<AtomicU64>,
    /// supervisor 写；`false` = 丢弃（未装配 / 会话已结束）
    assembled: Arc<AtomicBool>,
    /// G1：`stop` 撞通道满时登记，supervisor 100 ms 内补发；置 `false` 时一并清掉
    pending_stop: Arc<AtomicBool>,
}

impl MotionSender {
    /// 建通道，返回 `(底盘用发送端, supervisor 用接收端)`
    pub fn Channel(cap: usize) -> (MotionSender, mpsc::Receiver<protocol::MotionCmd>) {
        let (tx, rx) = mpsc::channel(cap.max(1));
        (
            Self {
                tx,
                seq: Arc::new(AtomicU64::new(1)),
                assembled: Arc::new(AtomicBool::new(false)),
                pending_stop: Arc::new(AtomicBool::new(false)),
            },
            rx,
        )
    }

    /// 装配状态由 supervisor 写：`true` = 可以下发；`false` = 丢弃
    ///
    /// 置 `false` 时**同时清掉** `pending_stop`：会话已结束，不该在断线期间攒标志
    /// （否则重连后会把上一次会话的停车补发到新会话）
    pub fn Set_Assembled(&self, on: bool) {
        self.assembled.store(on, Ordering::SeqCst);
        if !on {
            self.pending_stop.store(false, Ordering::SeqCst);
        }
    }

    pub fn Is_Assembled(&self) -> bool {
        self.assembled.load(Ordering::SeqCst)
    }

    /// 投递一条动作（`try_send`，**绝不阻塞**）
    ///
    /// - 未装配 → `debug!` 丢弃 + `Ok(())`（Step 2 的「纯决策链空跑」不产生噪音）
    /// - 通道满/关闭 → 限流 `warn!` + `Err`；`action == "stop"` 走 `pending_stop` 补发
    pub fn Send(&self, action: &'static str, speed: u8) -> Result<(), String> {
        if !self.Is_Assembled() {
            debug!("[world] 未装配，丢弃 motion_cmd action={action} speed={speed}");
            return Ok(());
        }
        match self.Push(action, speed) {
            Ok(()) => Ok(()),
            Err(msg) => {
                if action == "stop" {
                    // G1：停车绝不静默丢 —— 登记标志，supervisor 下一拍补发
                    self.pending_stop.store(true, Ordering::SeqCst);
                    Warn_Throttled("world.stop_pending", 1000, || {
                        format!("[world] stop 未投递（{msg}）：已登记，100 ms 内补发")
                    });
                } else {
                    Warn_Throttled("world.motion_dropped", 1000, || {
                        format!("[world] {msg}（世界只取最新一条，不补发历史）")
                    });
                }
                Err(msg)
            }
        }
    }

    /// G1：停车（安全关键）。等价 `Send("stop", 0)`，单独入口便于 `DeviceHandler::stop()`
    pub fn Send_Stop_Priority(&self) -> Result<(), String> {
        self.Send("stop", 0)
    }

    /// 连续速度指令（task_39 §四.4）：`v` 单位 **m/s**、`omega` 单位 **rad/s**
    /// （物理量，与真机 UAV 的 `vel_fwd` / `yaw_rate_deg` 同口径；§四 修订）。
    ///
    /// - 未装配 → `debug!` 丢弃 + `Ok(())`（与 [`Send`] 同语义，Step 2 的空跑不产生噪音）
    /// - 通道满/关闭 → 限流 `warn!`（`world.velocity_dropped`）+ `Err`
    /// - **不登记 `pending_stop`**：速度指令按 `AUTO_TICK_MS`（50 ms）周期重发，掉一帧下一拍自愈；
    ///   安全停车仍走 [`Self::Send_Stop_Priority`]（reset / stop / shutdown / 急停那条路，G1 不变）
    pub fn Send_Velocity(&self, v: f32, omega: f32) -> Result<(), String> {
        if !self.Is_Assembled() {
            debug!(
                "[world] 未装配，丢弃 motion_cmd action={} v={v} omega={omega}",
                protocol::ACTION_VELOCITY
            );
            return Ok(());
        }
        match self.Push_Velocity(v, omega) {
            Ok(()) => Ok(()),
            Err(msg) => {
                Warn_Throttled("world.velocity_dropped", 1000, || {
                    format!("[world] {msg}（世界只取最新一条，不补发历史）")
                });
                Err(msg)
            }
        }
    }

    /// 车栈连续档位指令（task_40 §二.6）：`speed` = 前进档位 0~100、`turn` = 带符号转向档位 −100..+100
    /// （**档位，不是物理量**；百分比 → m/s / rad/s 只在世界侧 `DiffDrive.gd` 换算）。
    ///
    /// 与 [`Self::Send_Velocity`] **同构**（只换字段）：
    /// - 未装配 → `debug!` 丢弃 + `Ok(())`；
    /// - 通道满/关闭 → 限流 `warn!`（`world.velocity_pct_dropped`）+ `Err`；
    /// - **不登记 `pending_stop`**：50 ms 下一拍自愈，安全停车仍走 [`Self::Send_Stop_Priority`]；
    /// - `seq` 与其它帧**共用同一个计数器**；
    /// - `v` / `omega` 恒 `None`，`turn` 恒 `Some(clamp(−100, 100))`。
    pub fn Send_Velocity_Pct(&self, speed: u8, turn: i16) -> Result<(), String> {
        if !self.Is_Assembled() {
            debug!(
                "[world] 未装配，丢弃 motion_cmd action={} speed={speed} turn={turn}",
                protocol::ACTION_VELOCITY
            );
            return Ok(());
        }
        match self.Push_Velocity_Pct(speed, turn) {
            Ok(()) => Ok(()),
            Err(msg) => {
                Warn_Throttled("world.velocity_pct_dropped", 1000, || {
                    format!("[world] {msg}（世界只取最新一条，不补发历史）")
                });
                Err(msg)
            }
        }
    }

    /// supervisor 每 [`STOP_RESEND_MS`] 取一次：`Some` = 需要立即补发这条 stop
    /// （**已带新 `seq`，标志已清**，绝不重复补发）
    pub fn Take_Pending_Stop(&self) -> Option<protocol::MotionCmd> {
        if !self.pending_stop.swap(false, Ordering::SeqCst) {
            return None;
        }
        Some(self.Next_Cmd("stop", 0))
    }

    /// 组一条 `motion_cmd`（`speed` 保留现有的 `min(100)` 兜底）+ 取下一个 `seq`
    fn Next_Cmd(&self, action: &'static str, speed: u8) -> protocol::MotionCmd {
        protocol::MotionCmd {
            msg: protocol::MSG_MOTION_CMD,
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            action,
            speed: speed.min(100),
            // 离散动作帧不带速度字段（`skip_serializing_if` ⇒ 旧 JSON 逐字不变）
            v: None,
            omega: None,
            turn: None,
        }
    }

    fn Push(&self, action: &'static str, speed: u8) -> Result<(), String> {
        let cmd = self.Next_Cmd(action, speed);
        self.tx
            .try_send(cmd)
            .map_err(|e| format!("motion_cmd 未投递（action={action}）：{e}"))
    }

    /// 组一条速度帧（`action = ACTION_VELOCITY`；`speed` 仍带 0 —— 世界侧该分支不读它）
    ///
    /// `seq` 与 [`Self::Next_Cmd`] **共用同一个计数器**（世界只做日志对账，不要求连续）
    fn Next_Velocity_Cmd(&self, v: f32, omega: f32) -> protocol::MotionCmd {
        protocol::MotionCmd {
            msg: protocol::MSG_MOTION_CMD,
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            action: protocol::ACTION_VELOCITY,
            speed: 0,
            v: Some(v),
            omega: Some(omega),
            turn: None,
        }
    }

    fn Push_Velocity(&self, v: f32, omega: f32) -> Result<(), String> {
        let cmd = self.Next_Velocity_Cmd(v, omega);
        self.tx.try_send(cmd).map_err(|e| {
            format!("motion_cmd 未投递（action={} v={v} omega={omega}）：{e}", protocol::ACTION_VELOCITY)
        })
    }

    /// 组一条车栈连续档位帧（`action = ACTION_VELOCITY`；`v` / `omega` 恒 `None`）
    ///
    /// `turn` 是**带符号**档位：`clamp(−100, 100)`（负 = 右转，符号见 `sim::ugv_sim::executor` 文件头）；
    /// `speed` 是前进档位：`min(100)`。`seq` 与 [`Self::Next_Cmd`] **共用同一个计数器**。
    fn Next_Velocity_Pct_Cmd(&self, speed: u8, turn: i16) -> protocol::MotionCmd {
        protocol::MotionCmd {
            msg: protocol::MSG_MOTION_CMD,
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            action: protocol::ACTION_VELOCITY,
            speed: speed.min(100),
            v: None,
            omega: None,
            turn: Some(turn.clamp(-100, 100)),
        }
    }

    fn Push_Velocity_Pct(&self, speed: u8, turn: i16) -> Result<(), String> {
        let cmd = self.Next_Velocity_Pct_Cmd(speed, turn);
        self.tx.try_send(cmd).map_err(|e| {
            format!(
                "motion_cmd 未投递（action={} speed={speed} turn={turn}）：{e}",
                protocol::ACTION_VELOCITY
            )
        })
    }
}


// ================================================================ 状态机

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorldState {
    /// 不再重连（致命错误：1011 装配失败 / 1002×3 同因）
    #[default]
    Disabled,
    Connecting,
    ChassisReady,
    Assembling,
    Assembled,
    Reconnecting,
}

/// supervisor 独占写、`on_tick` 只读的快照
#[derive(Debug, Clone, Copy, Default)]
struct WorldShared {
    state: WorldState,
    avatar_id: Option<u16>,
    /// 每次 `chassis_ready` 都重新采纳（重连 = 一台新车，spawn 会变，R8 / 风险 4）
    spawn: Option<protocol::Spawn>,
    world_time: f64,
}

// ================================================================ 连接事件

#[derive(Debug)]
struct ConnEvent {
    kind: ConnKind,
    ev: link::LinkEvent,
}

/// 连接身份（日志与分流用）
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnKind {
    Chassis,
    Sensor(String),
}

// ================================================================ 句柄（设备端持有）

/// `start()` 之后设备端握着的把手：只读状态 + 取消（动作上行已交给底盘的 [`MotionSender`]）
#[derive(Clone)]
pub struct WorldLinkHandle {
    shared: Arc<RwLock<WorldShared>>,
    /// 动作上行口（与 `Spawn` 收到的 `motion_rx` 是同一条通道的两端）
    motion: MotionSender,
    cancel: CancellationToken,
}

impl WorldLinkHandle {
    /// 装配状态机的当前态（`control::Handle_Cmd` 的 `status` 用它判 `assembled`，Step 5）
    pub async fn State(&self) -> WorldState {
        self.shared.read().await.state
    }

    /// 世界给的 avatar 编号（`status` 用，Step 5）：未握手 / 未装配过 → `None`
    ///
    /// 注意（R7）：重连期间 `avatar_id` 会残留上一次会话的值（`WorldLink` 收尾只清
    /// `motion` 的装配门闩，不清 `avatar_id`）——消费方必须先看 `State() == Assembled`。
    pub async fn Avatar_Id(&self) -> Option<u16> {
        self.shared.read().await.avatar_id
    }

    /// `on_tick` 门闩：只有 `Assembled` 允许决策
    pub async fn Is_Assembled(&self) -> bool {
        self.shared.read().await.state == WorldState::Assembled
    }

    /// 投递一条 motion_cmd（内部 `try_send`，**不阻塞**；满/关闭 → `Err`）
    ///
    /// Step 5 起控制口**不走这里**：它注入的是 `Command`（`robot.robot_cmd_tx`），
    /// 与地面站同一条下游；动作上行仍只由底盘持有（单写者）。
    #[allow(dead_code)] // 动作只走底盘的 MotionSender；此处保留给未建底盘的兜底调用方
    pub async fn Send_Motion(&self, action: &'static str, speed: u8) -> Result<(), String> {
        self.motion.Send(action, speed)
    }

    /// 同步版：`DeviceHandler::stop()` 是同步 fn（R3）
    #[allow(dead_code)] // Step 4 起由底盘 Stop() 承担；保留给未建底盘的兜底调用方
    pub fn Try_Send_Motion(&self, action: &'static str, speed: u8) -> Result<(), String> {
        self.motion.Send(action, speed)
    }

    /// 停止链路（同步 cancel；收尾由 supervisor 的 session 退出路径完成）
    ///
    /// 先 `Set_Assembled(false)`：会话结束不该再收动作，也不会把旧 `pending_stop` 带进新会话
    pub fn Shutdown(&self) {
        self.motion.Set_Assembled(false);
        self.cancel.cancel();
    }

    /// `(state, avatar_id, spawn)`：供 Step 4/5 采纳 spawn
    #[allow(dead_code)] // Step 4/5 接入
    pub async fn Snapshot(&self) -> (WorldState, Option<u16>, Option<protocol::Spawn>) {
        let g = self.shared.read().await;
        (g.state, g.avatar_id, g.spawn)
    }
}

// ================================================================ supervisor

/// 世界链路的消费端（Step 4 的接线口）
///
/// - `sensors`：按 slot 索引的传感器实例（连接任务用它把 `Binary` 直接交给该 slot 的传感器）
/// - `chassis`：`pose` / `chassis_ready.spawn` 的唯一写入口（`None` = 无底盘 = 无位姿写者，
///   与真机 `enabled=false` 一致；此时 pose 只计数 + 首次 `debug!`）
/// - `motion`：动作上行口（与 `Spawn` 的 `motion_rx` 是同一条通道的两端）
#[derive(Clone)]
pub struct WorldConsumers {
    pub sensors: Arc<SensorSet>,
    pub chassis: Option<Arc<dyn Chassis>>,
    pub motion: MotionSender,
}

pub struct WorldLink {
    cfg: ResolvedWorld,
    peer_id: String,
    shared: Arc<RwLock<WorldShared>>,
    cancel: CancellationToken,
    consumers: WorldConsumers,
}

impl WorldLink {
    /// **只 spawn，绝不 await 网络**（世界没起也必须让 `start()` 立刻返回，§四.2 差异 1）
    ///
    /// 第 4 个参数是动作通道的接收端：通道本身在 `start()` 里先建（§六.5.6 偏差 B），
    /// 因为「底盘要发送端 ← 通道在 Spawn 里建 ← Spawn 要传感器 ← 传感器要 pose_age ← 归底盘」
    /// 会形成死循环。
    pub fn Spawn(
        cfg: ResolvedWorld,
        peer_id: String,
        consumers: WorldConsumers,
        motion_rx: mpsc::Receiver<protocol::MotionCmd>,
    ) -> WorldLinkHandle {
        let shared = Arc::new(RwLock::new(WorldShared::default()));
        let cancel = CancellationToken::new();
        let (ev_tx, ev_rx) = mpsc::channel(64);
        let motion = consumers.motion.clone();

        let me = WorldLink { cfg, peer_id, shared: shared.clone(), cancel: cancel.clone(), consumers };
        tokio::spawn(async move { me.Run(ev_tx, ev_rx, motion_rx).await });

        WorldLinkHandle { shared, motion, cancel }
    }

    /// 采纳 `chassis_ready`（唯一写入口；锁内不 await）
    async fn Set_Adopted(
        shared: &Arc<RwLock<WorldShared>>,
        avatar_id: u16,
        spawn: protocol::Spawn,
        world_time: f64,
    ) {
        let mut g = shared.write().await;
        g.avatar_id = Some(avatar_id);
        g.spawn = Some(spawn);
        g.world_time = world_time;
    }

    async fn Set_State(&self, s: WorldState) {
        let mut g = self.shared.write().await;
        if g.state != s {
            debug!("[world] state {:?} -> {:?}", g.state, s);
            g.state = s;
        }
    }

    /// 外层：重连与终止（1002/1008/1011/1001 分流在这里）
    async fn Run(
        self,
        ev_tx: mpsc::Sender<ConnEvent>,
        mut ev_rx: mpsc::Receiver<ConnEvent>,
        mut cmd_rx: mpsc::Receiver<protocol::MotionCmd>,
    ) {
        let mut backoff = self.cfg.reconnect_ms_min;
        let mut last_1002: Option<String> = None;
        let mut streak_1002: u32 = 0;

        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            self.Set_State(WorldState::Connecting).await;

            let end = {
                let mut cx = SessionCtx::New(
                    ev_tx.clone(),
                    &mut ev_rx,
                    &mut cmd_rx,
                    self.cancel.child_token(),
                    self.shared.clone(),
                    self.consumers.chassis.clone(),
                );
                self.Session(&mut cx, &mut backoff).await
            };

            // 会话结束 = 世界那边已经没有底盘连接：动作一律丢弃，也不补发旧 stop（G1 的收尾）
            self.consumers.motion.Set_Assembled(false);

            let is_1002 = matches!(&end, SessionEnd::Closed { code: 1002, .. });
            let next = match end {
                SessionEnd::Cancelled => Next::Stop,
                SessionEnd::Fatal { why } => {
                    error!("[world] 停止重连（state=Disabled）：{why}");
                    Next::Disabled
                }
                SessionEnd::Closed { code, reason } => match code {
                    1002 => {
                        error!("[world] 协议错误 1002 reason='{reason}'（我们的 hello 不合法）");
                        if last_1002.as_deref() == Some(reason.as_str()) {
                            streak_1002 += 1;
                        } else {
                            last_1002 = Some(reason.clone());
                            streak_1002 = 1;
                        }
                        if streak_1002 >= 3 {
                            error!(
                                "[world] 连续 3 次 1002 同因，停止重连：检查 [sim_world] chassis / slot / sensor 名字与 manifest"
                            );
                            Next::Disabled
                        } else {
                            // §四.5.6：1002 固定退避 3 s（我们自己不合法，刷屏没意义）
                            Next::Retry(self.cfg.reconnect_ms_max)
                        }
                    }
                    1008 => {
                        warn!("[world] 世界侧 pending timeout（1008）：重建全部连接");
                        Next::Retry(backoff)
                    }
                    1011 => {
                        error!(
                            "[world] 世界侧装配失败（1011：{reason}），不再重连（检查底盘场景 / Vehicles 容器）"
                        );
                        Next::Disabled
                    }
                    1001 => {
                        info!("[world] 世界侧要求离开（1001：{reason}），准备重连");
                        Next::Retry(backoff)
                    }
                    other => {
                        info!("[world] 连接关闭 code={other} reason='{reason}'，准备重连");
                        Next::Retry(backoff)
                    }
                },
                SessionEnd::Timeout { what } => {
                    warn!(
                        "[world] {what} 超时（{} ms），重建全部连接",
                        self.cfg.handshake_timeout_ms
                    );
                    Next::Retry(backoff)
                }
                SessionEnd::Lost { why } => {
                    warn!("[world] {why}，{} ms 后重连", Jitter(backoff));
                    Next::Retry(backoff)
                }
            };

            // 1002 的计数只看**连续同因**：其它结束方式一律重置（R21）
            if !is_1002 {
                last_1002 = None;
                streak_1002 = 0;
            }

            let delay = match next {
                Next::Stop => break,
                Next::Disabled => {
                    self.Set_State(WorldState::Disabled).await;
                    break;
                }
                Next::Retry(d) => d,
            };

            self.Set_State(WorldState::Reconnecting).await;
            let wait = Jitter(delay);
            tokio::select! {
                _ = self.cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
            }
            backoff = Next_Backoff(backoff, self.cfg.reconnect_ms_max);
        }
        self.Set_State(WorldState::Disabled).await;
        debug!("[world] supervisor 退出");
    }

    /// 一次 session 的收尾统一走这里（cancel 子 token + join recv task）
    async fn Session(&self, cx: &mut SessionCtx<'_>, backoff: &mut u64) -> SessionEnd {
        let end = self.Session_Body(cx, backoff).await;
        cx.Close().await;
        end
    }

    async fn Session_Body(&self, cx: &mut SessionCtx<'_>, backoff: &mut u64) -> SessionEnd {
        let hs = self.cfg.handshake_timeout_ms;
        let unit = Duration::from_millis(hs);

        // ---- 1) 底盘连接
        self.Set_State(WorldState::Connecting).await;
        let mut link = match link::Link::Connect(&self.cfg.url, hs).await {
            Ok(l) => l,
            Err(e) => return SessionEnd::Lost { why: e },
        };

        // ---- 2) chassis_hello（manifest 与传感器连接清单同源）
        let manifest = parts::Build_Manifest(&self.cfg.sensors);
        let hello = parts::Build_Hello(&parts::PartDecl::Chassis {
            peer_id: &self.peer_id,
            chassis: &self.cfg.chassis,
            params: &self.cfg.chassis_params,
            manifest,
        });
        if hello.is_null() {
            return SessionEnd::Fatal {
                why: "chassis_hello 序列化失败（chassis_params 里有 NaN / infinity？）".to_string(),
            };
        }
        if let Err(e) = link.Send_Json(&hello).await {
            return SessionEnd::Lost { why: format!("发送 chassis_hello 失败：{e}") };
        }
        let (mut chassis_tx, chassis_rx) = link.Split();
        // 底盘连接没有传感器端点：二进制帧仍走 supervisor（限流告警 + 丢弃）
        cx.Spawn_Recv(ConnKind::Chassis, chassis_rx, None);

        // ---- 3) 等 chassis_ready（期间会持续收到 pose，不影响等待）
        match cx.Await(Instant::now() + unit, Want::ChassisReady).await {
            Pumped::Got(protocol::Inbound::ChassisReady { avatar_id, spawn, world_time }) => {
                self.Set_State(WorldState::ChassisReady).await;
                info!(
                    "[world] chassis_ready avatar={avatar_id} spawn=({:.2}, {:.2}, {:.2}) world_time={world_time:.3}",
                    spawn.x, spawn.y, spawn.z
                );
            }
            Pumped::Timeout => return SessionEnd::Timeout { what: "等待 chassis_ready".to_string() },
            Pumped::Cancelled => return SessionEnd::Cancelled,
            Pumped::Ended(e) => return e,
            Pumped::Got(_) => {
                return SessionEnd::Fatal { why: "等待 chassis_ready 却收到其它消息".to_string() }
            }
        }

        // ---- 4) 传感器连接 ×N（顺序建立：世界侧 pending 10 s 只是兜底）
        self.Set_State(WorldState::Assembling).await;
        // 发送半必须活到 session 结束（否则底层的写半被拆掉，close 帧发不出去）
        let mut sensor_txs: Vec<link::LinkSender> = Vec::new();
        for s in &self.cfg.sensors {
            let mut l = match link::Link::Connect(&self.cfg.url, hs).await {
                Ok(l) => l,
                Err(e) => {
                    return SessionEnd::Lost { why: format!("连接传感器 slot='{}' 失败：{e}", s.slot) }
                }
            };
            let hello = parts::Build_Hello(&parts::PartDecl::Sensor {
                peer_id: &self.peer_id,
                slot: &s.slot,
                sensor: &s.sensor,
                mount: s.mount,
                params: &s.params,
            });
            if hello.is_null() {
                return SessionEnd::Fatal {
                    why: format!("sensor_hello（slot='{}'）序列化失败", s.slot),
                };
            }
            if let Err(e) = l.Send_Json(&hello).await {
                return SessionEnd::Lost {
                    why: format!("发送 sensor_hello（slot='{}'）失败：{e}", s.slot),
                };
            }
            let (tx, rx) = l.Split();
            sensor_txs.push(tx);
            // 一条传感器连接 = 一个端点（持有 slot + 连接实例）；该 slot 没建传感器时 ep = None，
            // 此时帧走 supervisor 的兜底分支（限流告警 + 丢弃），不会静默丢
            let ep = self.consumers.sensors.Get(&s.slot).map(SensorEndpoint::New);
            cx.Spawn_Recv(ConnKind::Sensor(s.slot.clone()), rx, ep.clone());

            match cx.Await(Instant::now() + unit, Want::SensorReady(s.slot.clone())).await {
                Pumped::Got(protocol::Inbound::SensorReady { slot, sensor_id }) => {
                    // sensor_ready 命中后**立即**登记 sensor_id，早于任何稳态收包（R6）
                    if let Some(ep) = &ep {
                        ep.Set_Sensor_Id(sensor_id);
                        cx.endpoints.push(ep.clone());
                    }
                    info!("[world] sensor_ready slot={slot} sensor_id={sensor_id}");
                }
                Pumped::Timeout => {
                    return SessionEnd::Timeout {
                        what: format!("等待 sensor_ready（slot='{}'）", s.slot),
                    }
                }
                Pumped::Cancelled => return SessionEnd::Cancelled,
                Pumped::Ended(e) => return e,
                Pumped::Got(_) => {
                    return SessionEnd::Fatal {
                        why: format!("等待 sensor_ready（slot='{}'）却收到其它消息", s.slot),
                    }
                }
            }
        }

        // ---- 5) assembly_ready（空 manifest 时世界在 chassis_hello 末尾就发过，已暂存 → R9）
        let world_time = match cx.stash.take() {
            Some(t) => {
                debug!("[world] 消费提前到达的 assembly_ready（world_time={t:.3}）");
                t
            }
            None => match cx.Await(Instant::now() + unit, Want::AssemblyReady).await {
                Pumped::Got(protocol::Inbound::AssemblyReady { world_time }) => world_time,
                Pumped::Timeout => {
                    return SessionEnd::Timeout { what: "等待 assembly_ready".to_string() }
                }
                Pumped::Cancelled => return SessionEnd::Cancelled,
                Pumped::Ended(e) => return e,
                Pumped::Got(_) => {
                    return SessionEnd::Fatal {
                        why: "等待 assembly_ready 却收到其它消息".to_string(),
                    }
                }
            },
        };
        // Step 4：装配完成 = 可以下发动作。**先开动作门闩、再写 Assembled 状态**：
        // 顺序反过来的话，`on_tick` 可能在两者之间看到 Assembled 并 Dispatch，而 `MotionSender`
        // 还没装配 → 那条动作被丢弃；决策器的 Turning/Moving 之后是「保持不发」（G4），
        // 丢一条 turn 就会卡死。故门闩先开。
        self.consumers.motion.Set_Assembled(true);
        {
            let mut g = self.shared.write().await;
            g.state = WorldState::Assembled;
            g.world_time = world_time;
        }
        *backoff = self.cfg.reconnect_ms_min;

        let (avatar, spawn) = {
            let g = self.shared.read().await;
            (g.avatar_id, g.spawn)
        };
        let (sx, sy) = spawn.map(|s| (s.x, s.y)).unwrap_or((0.0, 0.0));
        // 验收 4 的日志串（slots=N 必须带上：N=0 就是「这车没有传感器」）
        info!(
            "[world] assembled avatar={} slots={} spawn=({:.2}, {:.2})",
            avatar.unwrap_or(0),
            self.cfg.sensors.len(),
            sx,
            sy
        );

        // ---- 6) 稳态收包循环
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // 跳过立刻触发的第一次
        // G1：`stop` 补发节拍（通道满时登记的那条，最多滞后 STOP_RESEND_MS）
        let mut stop_tick = tokio::time::interval(Duration::from_millis(STOP_RESEND_MS));
        stop_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        stop_tick.tick().await; // 跳过立刻触发的第一次

        loop {
            let wake = tokio::select! {
                _ = self.cancel.cancelled() => Wake::Cancel,
                _ = cx.session_cancel.cancelled() => Wake::Cancel,
                _ = tick.tick() => Wake::Tick,
                _ = stop_tick.tick() => Wake::Drain,
                m = cx.ev_rx.recv(), if cx.ev_open => Wake::Ev(m),
                c = cx.cmd_rx.recv(), if cx.cmd_open => Wake::Cmd(c),
            };

            match wake {
                Wake::Cancel => return SessionEnd::Cancelled,
                Wake::Tick => {
                    debug!(
                        "[world] 5s 统计：pose={} 帧 / lidar={} 帧",
                        cx.pose_frames,
                        cx.Lidar_Frames()
                    );
                }
                Wake::Drain => {
                    // G1：通道满时登记下来的 stop，下一拍必发（绝不静默丢）
                    if let Some(cmd) = self.consumers.motion.Take_Pending_Stop() {
                        debug!("[world] 补发 stop（seq={}，通道当时已满）", cmd.seq);
                        if let Err(end) = Send_Out(&mut chassis_tx, &cmd).await {
                            return end;
                        }
                    }
                }
                Wake::Cmd(None) => {
                    // 命令通道关闭（句柄被丢）：链路本身照旧，只是没有动作上行
                    cx.cmd_open = false;
                }
                Wake::Cmd(Some(cmd)) => {
                    if let Err(end) = Send_Out(&mut chassis_tx, &cmd).await {
                        return end;
                    }
                }
                Wake::Ev(None) => {
                    cx.ev_open = false;
                    return SessionEnd::Lost { why: "连接事件通道关闭".to_string() };
                }
                Wake::Ev(Some(ev)) => {
                    let ConnEvent { kind, ev } = ev;
                    match SessionCtx::To_Parsed(&kind, ev) {
                        Parsed::Drop => {}
                        Parsed::End(end) => return end,
                        Parsed::Inbound(inb) => {
                            let _ = cx.Handle_Inbound(&kind, &inb, &Want::Never).await;
                        }
                    }
                }
            }
        }
    }
}

// ================================================================ session 收包上下文

/// 一次 session 内的收包上下文（只有 supervisor 用；不含 self，避免与 `&self` 借冲突）
struct SessionCtx<'a> {
    ev_tx: mpsc::Sender<ConnEvent>,
    ev_rx: &'a mut mpsc::Receiver<ConnEvent>,
    cmd_rx: &'a mut mpsc::Receiver<protocol::MotionCmd>,
    session_cancel: CancellationToken,
    shared: Arc<RwLock<WorldShared>>,
    /// `pose` / `chassis_ready.spawn` 的写入口 = 底盘（`None` = 无底盘 = 无位姿写者，§六.5.7）
    chassis: Option<Arc<dyn Chassis>>,
    /// 无底盘时每个 session 只 `debug!` 一次（否则 10 Hz pose 会刷屏）
    pose_no_chassis_logged: bool,
    /// 提前到的 `assembly_ready.world_time`（R9：空 manifest 时它紧跟 chassis_ready）
    stash: Option<f64>,
    pose_frames: u64,
    /// 本 session 的传感器连接端点（每个传感器连接一个，跨 session 重建 → 计数归零）
    endpoints: Vec<Arc<SensorEndpoint>>,
    ev_open: bool,
    cmd_open: bool,
    joins: Vec<tokio::task::JoinHandle<()>>,
}

impl<'a> SessionCtx<'a> {
    fn New(
        ev_tx: mpsc::Sender<ConnEvent>,
        ev_rx: &'a mut mpsc::Receiver<ConnEvent>,
        cmd_rx: &'a mut mpsc::Receiver<protocol::MotionCmd>,
        session_cancel: CancellationToken,
        shared: Arc<RwLock<WorldShared>>,
        chassis: Option<Arc<dyn Chassis>>,
    ) -> Self {
        Self {
            ev_tx,
            ev_rx,
            cmd_rx,
            session_cancel,
            shared,
            chassis,
            pose_no_chassis_logged: false,
            stash: None,
            pose_frames: 0,
            endpoints: Vec::new(),
            ev_open: true,
            cmd_open: true,
            joins: Vec::new(),
        }
    }

    /// 本 session 已投递的 lidar 帧数（5 s 统计用；跨 session 重新从 0 计）
    fn Lidar_Frames(&self) -> u64 {
        self.endpoints.iter().map(|e| e.Frames()).sum()
    }

    /// 每条连接一个 recv task：`LinkEvent` → `ConnEvent` → mpsc(64) → supervisor
    ///
    /// 传感器连接（`endpoint = Some`）的 `Message::Binary` **就地投递**给该 slot 的传感器，
    /// 不进 supervisor 的 mpsc（§五.4.7「直接交给该 slot 的传感器」；R7）。收包 task 只做同步
    /// 调用（`try_write`），不 `await` → 不阻塞心跳 poll。
    fn Spawn_Recv(
        &mut self,
        kind: ConnKind,
        mut rx: link::LinkReceiver,
        endpoint: Option<Arc<SensorEndpoint>>,
    ) {
        let ev_tx = self.ev_tx.clone();
        let token = self.session_cancel.clone();
        let jh = tokio::spawn(async move {
            loop {
                let next = tokio::select! {
                    _ = token.cancelled() => break,
                    ev = rx.Next() => ev,
                };
                let ev = match next {
                    Some(ev) => ev,
                    None => {
                        // 没有 Close 帧的收尾 = 1006（异常关闭）
                        let _ = ev_tx
                            .send(ConnEvent {
                                kind: kind.clone(),
                                ev: link::LinkEvent::Closed {
                                    code: 1006,
                                    reason: "连接流结束".to_string(),
                                },
                            })
                            .await;
                        break;
                    }
                };
                // 传感器连接：二进制帧就地交给该 slot 的传感器（含 sensor_id 校验 / 限流告警）
                if let (link::LinkEvent::Binary(bytes), Some(ep)) = (&ev, &endpoint) {
                    ep.Push_Binary(bytes);
                    continue;
                }
                let closed = matches!(ev, link::LinkEvent::Closed { .. });
                if ev_tx.send(ConnEvent { kind: kind.clone(), ev }).await.is_err() {
                    break;
                }
                if closed {
                    break;
                }
            }
        });
        self.joins.push(jh);
    }

    /// tungstenite 事件 → 入站消息 / 丢弃 / 结束
    fn To_Parsed(kind: &ConnKind, ev: link::LinkEvent) -> Parsed {
        match ev {
            link::LinkEvent::Text(t) => match protocol::Parse_Text(&t) {
                Some(inb) => Parsed::Inbound(inb),
                None => {
                    debug!("[world] {kind:?} 收到非法/无用文本帧（{} B），丢弃", t.len());
                    Parsed::Drop
                }
            },
            link::LinkEvent::Binary(bytes) => Parsed::Inbound(protocol::Inbound::Binary { bytes }),
            link::LinkEvent::Closed { code, reason } => {
                warn!("[world] {kind:?} 连接关闭 code={code} reason='{reason}'");
                Parsed::End(SessionEnd::Closed { code, reason })
            }
        }
    }

    /// 处理一条入站消息；`true` = 命中当前等待目标。
    ///
    /// 等待期间对 `pose` / 重复 `sensor_ready` / 重复或提前的 `assembly_ready` **一律继续等**，
    /// 只有关闭 / 超时 / cancel 才结束本次 session（R12）。
    ///
    /// `kind` 只用于日志与"底盘连接不该收到二进制帧"的兜底告警。
    async fn Handle_Inbound(
        &mut self,
        kind: &ConnKind,
        inb: &protocol::Inbound,
        want: &Want,
    ) -> bool {
        match inb {
            protocol::Inbound::ChassisReady { avatar_id, spawn, world_time } => {
                // R8：世界按 avatar_id 错开 spawn，每次 chassis_ready 都重新采纳
                WorldLink::Set_Adopted(&self.shared, *avatar_id, *spawn, *world_time).await;
                // 世界决定的出生点覆盖 origin（多车时世界会把车摆在错开的方位）
                // Step 4：写入口是底盘（`PoseIngest` 归底盘持有，RobotState 单写者）
                if let Some(c) = self.chassis.as_ref() {
                    c.On_Spawn(&SpawnFields {
                        x: spawn.x,
                        y: spawn.y,
                        z: spawn.z,
                        yaw: spawn.yaw,
                    })
                    .await;
                } else {
                    debug!("[world] 无底盘：chassis_ready.spawn 丢弃（RobotState 无写者）");
                }
                if matches!(want, Want::ChassisReady) {
                    true
                } else {
                    debug!(
                        "[world] 重复 chassis_ready avatar={avatar_id} spawn=({:.2}, {:.2})（已重新采纳）",
                        spawn.x, spawn.y
                    );
                    false
                }
            }
            protocol::Inbound::SensorReady { slot, sensor_id } => {
                if matches!(want, Want::SensorReady(w) if w == slot) {
                    true
                } else {
                    debug!("[world] sensor_ready slot={slot} sensor_id={sensor_id}（非当前等待项）");
                    false
                }
            }
            protocol::Inbound::AssemblyReady { world_time } => {
                if matches!(want, Want::AssemblyReady) {
                    true
                } else {
                    // 幂等：重复只更新 world_time + debug；提前到则暂存供第 5 步消费（R9）
                    {
                        let mut g = self.shared.write().await;
                        g.world_time = *world_time;
                    }
                    self.stash = Some(*world_time);
                    debug!("[world] assembly_ready 重复/提前到达（world_time={world_time:.3}）");
                    false
                }
            }
            protocol::Inbound::Pose { world_time, x, y, z, yaw, vx, vy } => {
                // Step 4：`pose` → 底盘 `On_Pose` → RobotState{x,y,z,attitude.yaw,vx,vy}
                // + 新鲜度（计龄用本地单调钟）。无底盘 = 无位姿写者（§六.2.5），只计数不写。
                if let Some(c) = self.chassis.as_ref() {
                    c.On_Pose(&PoseFields {
                        world_time: *world_time,
                        x: *x,
                        y: *y,
                        z: *z,
                        yaw: *yaw,
                        vx: *vx,
                        vy: *vy,
                    })
                    .await;
                } else if !self.pose_no_chassis_logged {
                    self.pose_no_chassis_logged = true;
                    debug!("[world] 无底盘：pose 丢弃（RobotState 无写者）");
                }
                self.pose_frames += 1;
                if self.pose_frames == 1 {
                    info!("[world] 收到首个 pose：x={x:.2} y={y:.2} z={z:.2} yaw={yaw:.2} world_time={world_time:.3}");
                }
                false
            }
            protocol::Inbound::Binary { bytes } => {
                // 传感器连接的二进制帧已在收包 task 里就地投递（Spawn_Recv），走到这里只可能是
                // 底盘连接发来二进制（世界不会这么发）→ 限流告警后丢弃
                Warn_Throttled("world.chassis_binary", 5000, || {
                    format!(
                        "[world] {kind:?} 收到二进制帧（{} B）但本连接没有登记传感器（底盘不会下发二进制；该 slot 未建传感器时也走这里），丢弃",
                        bytes.len()
                    )
                });
                false
            }
            protocol::Inbound::Unknown { msg } => {
                debug!("[world] 忽略未知 msg '{msg}'");
                false
            }
        }
    }

    /// 等到 **deadline**（每个阶段都用 `handshake_timeout_ms`）或命中 `want`
    async fn Await(&mut self, deadline: Instant, want: Want) -> Pumped {
        loop {
            let wake = tokio::select! {
                _ = self.session_cancel.cancelled() => Wake::Cancel,
                _ = tokio::time::sleep_until(deadline) => Wake::Tick,
                m = self.ev_rx.recv(), if self.ev_open => Wake::Ev(m),
                c = self.cmd_rx.recv(), if self.cmd_open => Wake::Cmd(c),
            };

            match wake {
                Wake::Cancel => return Pumped::Cancelled,
                Wake::Tick => return Pumped::Timeout,
                // 握手/装配阶段没有 stop 补发节拍（`stop` 只在稳态循环里补发）
                Wake::Drain => {}
                Wake::Cmd(None) => self.cmd_open = false,
                Wake::Cmd(Some(cmd)) => {
                    // 装配前的动作一律丢弃（R20：世界只取最新一条，不补发历史）
                    debug!(
                        "[world] 未装配，丢弃 motion_cmd action={} seq={}",
                        cmd.action, cmd.seq
                    );
                }
                Wake::Ev(None) => {
                    self.ev_open = false;
                    return Pumped::Ended(SessionEnd::Lost { why: "连接事件通道关闭".to_string() });
                }
                Wake::Ev(Some(ev)) => {
                    let ConnEvent { kind, ev } = ev;
                    match Self::To_Parsed(&kind, ev) {
                        Parsed::Drop => continue,
                        Parsed::End(end) => return Pumped::Ended(end),
                        Parsed::Inbound(inb) => {
                            if self.Handle_Inbound(&kind, &inb, &want).await {
                                return Pumped::Got(inb);
                            }
                        }
                    }
                }
            }
        }
    }

    /// 退出 session：cancel 子 token → 500 ms 内等各 recv task，超时 abort（不依赖 Drop）
    async fn Close(&mut self) {
        self.session_cancel.cancel();
        for mut jh in self.joins.drain(..) {
            if tokio::time::timeout(Duration::from_millis(500), &mut jh).await.is_err() {
                jh.abort();
            }
        }
    }
}

// ================================================================ 内部枚举

enum Want {
    ChassisReady,
    SensorReady(String),
    AssemblyReady,
    /// 稳态：不命中任何目标（只处理副作用）
    Never,
}

enum Pumped {
    Got(protocol::Inbound),
    Timeout,
    Cancelled,
    Ended(SessionEnd),
}

enum Parsed {
    Inbound(protocol::Inbound),
    Drop,
    End(SessionEnd),
}

enum SessionEnd {
    Cancelled,
    Fatal { why: String },
    Closed { code: u16, reason: String },
    Timeout { what: String },
    Lost { why: String },
}

enum Next {
    /// 不再重连，但状态保持（正常 shutdown）
    Stop,
    /// 不再重连，且进 `Disabled`（1011 / 1002×3 同因 / 致命配置）
    Disabled,
    /// 退避 delay 后重连
    Retry(u64),
}

enum Wake {
    Cancel,
    /// 5 s 统计
    Tick,
    /// 100 ms：检查 G1 的 `pending_stop` 是否需要补发
    Drain,
    Ev(Option<ConnEvent>),
    Cmd(Option<protocol::MotionCmd>),
}

// ================================================================ 出站发送（普通命令与补发 stop 共用）

/// 把一条 `motion_cmd` 写到 junction 的底盘连接上（1 s 超时；失败/超时 → 1006 结束 session）
///
/// 从 Step 2/3 的 `Wake::Cmd(Some(cmd))` 分支原样提出来，供「普通命令」与「G1 补发 stop」
/// 两条路径共用 —— 行为逐字不变。
async fn Send_Out(
    tx: &mut link::LinkSender,
    cmd: &protocol::MotionCmd,
) -> Result<(), SessionEnd> {
    let send = tx.Send_Json(cmd);
    match tokio::time::timeout(Duration::from_millis(1000), send).await {
        Ok(Ok(())) => {
            debug!(
                "[world] motion_cmd 已发送 action={} speed={} v={:?} omega={:?} seq={}",
                cmd.action, cmd.speed, cmd.v, cmd.omega, cmd.seq
            );
            Ok(())
        }
        Ok(Err(e)) => Err(SessionEnd::Closed {
            code: 1006,
            reason: format!("发送 motion_cmd 失败：{e}"),
        }),
        Err(_) => Err(SessionEnd::Closed {
            code: 1006,
            reason: "发送 motion_cmd 超时（1 s）".to_string(),
        }),
    }
}

// ================================================================ 退避与抖动（纯函数，单测）

/// 退避翻倍，上限 `max`（200 → 400 → 800 → 1600 → 3000 → 3000）
fn Next_Backoff(cur: u64, max: u64) -> u64 {
    let max = max.max(1);
    cur.max(1).saturating_mul(2).min(max)
}

/// ±20% 抖动（`SystemTime` 亚秒纳秒做伪随机，不引入 `rand`）
fn Jitter(ms: u64) -> u64 {
    if ms == 0 {
        return 0;
    }
    let span = ms / 5;
    if span == 0 {
        return ms;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let off = nanos % (2 * span + 1);
    (ms - span).saturating_add(off)
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_world_state_default_disabled() {
        assert_eq!(WorldState::default(), WorldState::Disabled);
        let shared = WorldShared::default();
        assert_eq!(shared.state, WorldState::Disabled);
        assert_eq!(shared.avatar_id, None);
        assert_eq!(shared.spawn, None);
    }

    #[test]
    fn test_backoff_doubles_and_caps() {
        let mut b = 200;
        let seq = [400u64, 800, 1600, 3000, 3000];
        for expect in seq {
            b = Next_Backoff(b, 3000);
            assert_eq!(b, expect);
        }
        // 起点已到上限：保持
        assert_eq!(Next_Backoff(3000, 3000), 3000);
        // 0 也要能推进（防御：配置校验已拦 min=0）
        assert_eq!(Next_Backoff(0, 3000), 2);
    }

    #[test]
    fn test_jitter_within_20pct() {
        for _ in 0..100 {
            let j = Jitter(1000);
            assert!((800..=1200).contains(&j), "抖动越界：{j}");
        }
        assert_eq!(Jitter(0), 0);
        // 极小值不 panic、不越界
        for _ in 0..10 {
            let j = Jitter(4);
            assert!(j <= 4, "抖动越界：{j}");
        }
    }

    // ============================================================ MotionSender（Step 4 / G1）

    /// 已装配的发送端 + 接收端（容量按需）
    fn assembled(cap: usize) -> (MotionSender, mpsc::Receiver<protocol::MotionCmd>) {
        let (tx, rx) = MotionSender::Channel(cap);
        tx.Set_Assembled(true);
        (tx, rx)
    }

    #[test]
    fn test_motion_sender_drops_before_assembled() {
        // 未装配：返回 Ok(())（不吓人）但绝不进通道（§六.8-5）
        let (tx, mut rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        assert!(!tx.Is_Assembled());
        assert_eq!(tx.Send("forward", 30), Ok(()));
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        // 置 true 后才真正下发
        tx.Set_Assembled(true);
        assert!(tx.Is_Assembled());
        assert_eq!(tx.Send("forward", 30), Ok(()));
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_motion_sender_sends_when_assembled() {
        let (tx, mut rx) = assembled(MOTION_CHANNEL_CAP);
        tx.Send("forward", 30).expect("装配后必须投递成功");
        let c1 = rx.try_recv().expect("应收到第一条 motion_cmd");
        assert_eq!(c1.action, "forward");
        assert_eq!(c1.speed, 30);
        assert_eq!(c1.seq, 1, "seq 从 1 起");
        assert_eq!(c1.msg, protocol::MSG_MOTION_CMD);

        tx.Send("left", 10).expect("第二条");
        let c2 = rx.try_recv().expect("应收到第二条");
        assert_eq!(c2.action, "left");
        assert_eq!(c2.speed, 10);
        assert_eq!(c2.seq, 2, "seq 单调递增");
    }

    #[test]
    fn test_motion_send_clamps_speed() {
        // 保留 Step 2/3 的 speed.min(100) 行为（配置被 clamp，这里只是兜底）
        let (tx, mut rx) = assembled(MOTION_CHANNEL_CAP);
        tx.Send("forward", 250).expect("投递");
        assert_eq!(rx.try_recv().expect("收到").speed, 100);
    }

    #[test]
    fn test_motion_stop_priority_survives_full_channel() {
        // G1：通道塞满 → stop 投递失败，但**必须**登记补发（绝不静默丢）
        let (tx, mut rx) = assembled(2);
        tx.Send("forward", 30).expect("塞第 1 条");
        tx.Send("forward", 30).expect("塞第 2 条");
        let err = tx.Send_Stop_Priority().expect_err("满通道必须报 Err");
        assert!(err.contains("stop"), "错误信息应带 action：{err}");
        // 通道里只有两条普通命令，没有 stop（被丢了？不，是登记了）
        assert_eq!(rx.try_recv().expect("第 1 条").action, "forward");
        assert_eq!(rx.try_recv().expect("第 2 条").action, "forward");
        // 补发口：拿到一条新的 stop（seq 已分配、speed=0）
        let cmd = tx.Take_Pending_Stop().expect("stop 必须被登记下来补发");
        assert_eq!(cmd.action, "stop");
        assert_eq!(cmd.speed, 0);
        assert!(cmd.seq > 2, "补发的 stop 应带新的 seq：{}", cmd.seq);
    }

    #[test]
    fn test_pending_stop_taken_once() {
        let (tx, _rx) = assembled(1);
        tx.Send("forward", 30).expect("塞满");
        let _ = tx.Send_Stop_Priority();
        assert!(tx.Take_Pending_Stop().is_some(), "第一次必须拿到");
        assert!(tx.Take_Pending_Stop().is_none(), "第二次必须是 None（不重复补发）");
    }

    #[test]
    fn test_set_assembled_false_clears_pending() {
        // 会话结束：不该把上一次会话的 stop 带进新会话（G1 的收尾）
        let (tx, _rx) = assembled(1);
        tx.Send("forward", 30).expect("塞满");
        let _ = tx.Send_Stop_Priority();
        tx.Set_Assembled(false);
        assert!(tx.Take_Pending_Stop().is_none(), "置 false 必须清掉 pending_stop");
        // 且此后 Send 一律丢弃
        assert_eq!(tx.Send("stop", 0), Ok(()));
    }

    // ============================================================ 连续速度指令（task_39 §四.4）

    #[test]
    fn test_motion_sender_velocity_when_assembled() {
        // 装配后：一帧 `action="velocity"`，带 v / omega，speed 恒 0，seq 与 Send 共用计数器
        let (tx, mut rx) = assembled(MOTION_CHANNEL_CAP);
        tx.Send("forward", 30).expect("普通动作先占掉 seq=1");
        tx.Send_Velocity(0.5, -0.25).expect("装配后必须投递成功");
        let c1 = rx.try_recv().expect("应收到普通动作帧");
        assert_eq!((c1.action, c1.speed, c1.v, c1.omega), ("forward", 30, None, None));
        let c2 = rx.try_recv().expect("应收到速度帧");
        assert_eq!(c2.action, protocol::ACTION_VELOCITY);
        assert_eq!(c2.speed, 0, "速度帧的 speed 恒 0（世界该分支不读它）");
        assert_eq!((c2.v, c2.omega), (Some(0.5), Some(-0.25)));
        assert_eq!(c2.msg, protocol::MSG_MOTION_CMD);
        assert_eq!((c1.seq, c2.seq), (1, 2), "seq 与 Send 共用同一个计数器");
    }

    #[test]
    fn test_motion_sender_velocity_dropped_before_assembled() {
        // 未装配：返回 Ok(())（不吓人）但绝不进通道 —— 与 Send 同语义
        let (tx, mut rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        assert!(!tx.Is_Assembled());
        assert_eq!(tx.Send_Velocity(0.5, -0.25), Ok(()));
        assert!(matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        // 置 true 后才真正下发
        tx.Set_Assembled(true);
        assert_eq!(tx.Send_Velocity(0.5, -0.25), Ok(()));
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_motion_velocity_does_not_register_pending_stop() {
        // 速度帧掉帧不登记补发（50 ms 下一拍自愈）；G1 的 pending_stop 只服务 stop
        let (tx, mut rx) = assembled(1);
        tx.Send("forward", 30).expect("先塞满");
        let err = tx.Send_Velocity(0.5, -0.25).expect_err("满通道必须报 Err");
        assert!(err.contains(protocol::ACTION_VELOCITY), "错误信息应带 action：{err}");
        assert!(
            tx.Take_Pending_Stop().is_none(),
            "速度帧不许登记 pending_stop（安全停车走 Send_Stop_Priority）"
        );
        // 通道里仍然只有那一条普通动作
        assert_eq!(rx.try_recv().expect("第 1 条").action, "forward");
        assert!(matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    // ============================================================ 车栈连续档位指令（task_40 §二.6）

    #[test]
    fn test_motion_sender_velocity_pct_when_assembled() {
        // 装配后：一帧 `action="velocity"` + speed / turn，v / omega 恒 None，seq 与 Send 共用计数器
        let (tx, mut rx) = assembled(MOTION_CHANNEL_CAP);
        tx.Send("forward", 30).expect("普通动作先占掉 seq=1");
        tx.Send_Velocity_Pct(30, -30).expect("装配后必须投递成功");
        let c1 = rx.try_recv().expect("应收到普通动作帧");
        assert_eq!((c1.action, c1.speed, c1.turn), ("forward", 30, None));
        let c2 = rx.try_recv().expect("应收到档位帧");
        assert_eq!(c2.action, protocol::ACTION_VELOCITY);
        assert_eq!(c2.speed, 30);
        assert_eq!(c2.turn, Some(-30), "Δθ>0（右）⇒ turn 为负，原值透传");
        assert_eq!((c2.v, c2.omega), (None, None), "车栈帧不带物理量字段");
        assert_eq!(c2.msg, protocol::MSG_MOTION_CMD);
        assert_eq!((c1.seq, c2.seq), (1, 2), "seq 与 Send 共用同一个计数器");
    }

    #[test]
    fn test_motion_sender_velocity_pct_clamps() {
        // 两维都夹：speed → 0..100、turn → −100..+100（二次兜底，`SpeedBinding` 已夹过一次）
        let (tx, mut rx) = assembled(MOTION_CHANNEL_CAP);
        tx.Send_Velocity_Pct(250, -500).expect("投递");
        let c = rx.try_recv().expect("收到");
        assert_eq!((c.speed, c.turn), (100, Some(-100)));
        tx.Send_Velocity_Pct(0, 300).expect("投递");
        let c = rx.try_recv().expect("收到");
        assert_eq!((c.speed, c.turn), (0, Some(100)));
    }

    #[test]
    fn test_motion_sender_velocity_pct_dropped_before_assembled() {
        // 未装配：返回 Ok(()) 但绝不进通道；且**不登记 pending_stop**（与 Send_Velocity 同款）
        let (tx, mut rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        assert!(!tx.Is_Assembled());
        assert_eq!(tx.Send_Velocity_Pct(30, -30), Ok(()));
        assert!(matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        assert!(tx.Take_Pending_Stop().is_none(), "档位帧不许登记 pending_stop");
        // 置 true 后才真正下发
        tx.Set_Assembled(true);
        assert_eq!(tx.Send_Velocity_Pct(30, -30), Ok(()));
        assert_eq!(rx.try_recv().expect("收到").turn, Some(-30));
    }
}
