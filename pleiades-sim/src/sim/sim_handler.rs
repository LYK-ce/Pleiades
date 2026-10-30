//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-30

#![allow(non_snake_case)]

//! 模拟设备处理器（无硬件，用于测试决策链）
//!
//! 不 spawn stm32/lidar 串口；`on_tick` 跑完整决策链（`TaskStack` 推进树 + 执行器），
//! 「执行动作」交给底盘（[`Chassis::Apply_Action`] / [`Chassis::Apply_Velocity`] /
//! [`Chassis::Apply_Velocity_Pct`]），由底盘翻译成世界的 `motion_cmd`。
//!
//! 执行器三态（task_39 §四 / task_40 §二）：
//! - 车栈：`sim/ugv_sim/executor.rs` 的 `UgvVelocityExecutor`（三档连续**档位**）+ [`Chassis::Apply_Velocity_Pct`]；
//! - 机栈：`sim/uav_sim/executor.rs` 的 `UavExecutor`（三档连续**物理量**）+ [`Chassis::Apply_Velocity`]；
//! - 车栈遗留离散路径（`sim/executor.rs` + [`Chassis::Apply_Action`]）保留为 A/B 回退点，Step 1 起不再选用。
//! 两个连续分支都**只写 `ExecuteState.sub_target`**、`state` 留 `Idle`（唯一的读点
//! `base/Robot/core/robot.rs:250-253` 的 POSE 广播只取 `sub_target`，故 POSE / 集群不受影响）。
//!
//! 世界链路（task_31 Step 2）：
//! - `start()` 里**只 spawn**（[`WorldLink::Spawn`]），立即返回 —— 世界没起也必须能启动 tick
//! - `on_tick` 开头是**装配门闩**：未 `Assembled` 就写 `Idle` 直接 return（不寻路/不决策/不发命令）
//! - `stop()` / `shutdown()` 尽力发一条 `stop` 并停链路；两者对「`inner` 未初始化」必须安全
//!   （`start()` 因配置非法返回 `Err` 后，base 主循环会立刻调 stop() + shutdown()）
//!
//! 传感器（task_31 Step 3）：
//! - `start()` 按 `[sim_world.sensor]` 构造传感器并**在 `WorldLink::Spawn` 之前**拉起各自的建图 task
//!   （`SensorSet::Build` 内含 `Spawn_Mapping`）；仍然**绝不 await 网络**
//! - `on_tick` **一行不碰 grid**（写 grid 的是 lidar 传感器，§五.1）
//! - `shutdown()` 逐个 `Sensor::Shutdown`（cancel 建图 task）
//!
//! 底盘（task_31 Step 4，§六.5.7）：
//! - `start()` 先建动作通道（[`MotionSender::Channel`]）再建底盘，最后才 `Spawn` 世界链路
//!   （顺序即正确性：底盘要发送端 ← 通道 ← `Spawn` 要传感器 ← 传感器要 `pose_age` ← 归底盘）
//! - `pose` / `chassis_ready.spawn` 由底盘写 `RobotState`（**单写者**），本文件只持有底盘
//! - `on_tick` 门闩②：无底盘（无 `[chassis]` 段或 `enabled=false`）→ 保持 Idle
//! - `handle_manual_cmd` / `reset` / `stop` / `shutdown` 全部交底盘（`Beep` 只 warn、
//!   差速不响应 `Takeoff/Land`、四旋翼 `up/down`、雷达命令在此拦截）
//!
//! 本地控制口（task_31 Step 5，§七）：
//! - `start()` 的**最后一步**才 `control::Spawn`（在 `inner.set` 之后：注入的 `Command` 走到
//!   `device.reset()` 时 `inner` 必已就绪，不存在 `expect("device not started")` 竞态）
//! - 持有 `Arc<Robot>` + `Option<WorldLinkHandle>`：前者是注入/只读的唯一来源，后者供 `status`
//! - `shutdown()` **先** cancel 控制口（不再接受新注入），再走原有的停车/停链路收尾
//! - `on_tick` / `handle_manual_cmd` / `reset` / `stop` **一行不改**（控制口不进决策链）
//!
//! 急停（task_31 Step 6，§八）：
//! - `on_tick` 的分支顺序与真车 `CarDeviceHandler::on_tick`（`pleiades-ugv/src/ugv/robot_handler.rs:217-244`）
//!   逐项一致：**装配门闩 → 无底盘返回 → 急停 → 树 → 决策 → 写 ExecuteState → 执行 → 终值**
//! - `start()` 从 `SensorSet` 里选出急停用 lidar（[`Select_Lidar`]）存进 `SimInner.lidar`；
//!   无雷达 / 未连世界 → `None` → `on_tick` 跳过急停（真机 `lidar: None` 同语义）
//! - 急停**每 tick 复核**、不锁存；触发时 `Stop()` + `ExecuteState = Idle` + **保留 goal**（供绕行）
//!
//! 任务栈（Task 36 Step 6，§12.6）：
//! - `start()` 按底盘名选栈（[`Kind_Of_World`]）：差速车 → `ugv_sim::Stack`，UAV → `uav_sim::Stack`
//! - `build_task` / `on_tick` / `reset` 三个方法**只做转发**（栈内部才是 nav + 树实例表）
//! - 急停只挂车栈（`TaskStack::ugv_nav`），机栈不接 2D 急停（§12.6.4）

// 设备能力注册（task_34 Step 4）：`CameraSimCap` 实现 base 的 `DeviceCapability`。
//
// 为什么用 `#[path]` 而不在 `sim/mod.rs` 里加一行 `pub mod device_cap;`：
// 本 step 的允许清单里没有 `sim/mod.rs`（只有 `sim/device_cap.rs` 与 `sim/sensors/`）。
// `#[path]` 相对**本文件所在目录**（`src/sim/`）解析 → `src/sim/device_cap.rs`。
// 注意：模块的规范路径因此是 `crate::sim::sim_handler::device_cap`，别处引用要写全路径。
#[path = "device_cap.rs"]
pub mod device_cap;

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use pleiades_base::network::{NodeHandle, TOPIC_ROBOT_ASSIST_REQUEST};
use pleiades_base::orchestrator::{Capabilities, DeviceCapability};
use pleiades_base::robot::core::behavior_tree::{Frame, MotionOut, Status};
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::mission_queue::{QueuedTask, TaskId};
use pleiades_base::robot::core::protocol::{
    encode_assist_request, encode_frame, COMPID_ROBOT, MSGID_ASSIST_REQUEST,
};
use pleiades_base::robot::core::robot::{DeviceHandler, Robot, TickOutcome, AUTO_TICK_MS};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::{
    DEFAULT_PLAN_BUDGET_MS, DEFAULT_VEL_FWD, DEFAULT_YAW_RATE_DEG, SimConfig,
};
use crate::sim::chassis::{self, Chassis, SpeedBinding};
use crate::sim::chassis::pose_ingest::PoseHistory;
use crate::sim::control::TaskMonitor;
use crate::sim::emergency_stop::{self, Check_Emergency_Stop};
use crate::sim::executor::DecisionExecutor;
use crate::sim::sensors::camera::CameraSensor;
use crate::sim::sensors::lidar::mapping_task::MappingCtx;
use crate::sim::sensors::lidar::LidarSensor;
use crate::sim::sensors::{Sensor, SensorSet, Should_Log, Warn_Throttled};
use crate::sim::uav_sim;
use crate::sim::uav_sim::executor::{UavExecutor, DEADBAND_DEG, SPIN_DEG};
use crate::sim::uav_sim::mission::containment::AssistPublisher;
use crate::sim::ugv_sim;
// 车栈连续档位常量起别名：`DEADBAND_DEG` 已被上面机栈的同名常量占用（同名不同值，各自口径）
use crate::sim::ugv_sim::executor::{
    UgvVelocityExecutor, ARC_MAX_DEG as UGV_ARC_MAX_DEG, DEADBAND_DEG as UGV_DEADBAND_DEG,
    PASS_THROUGH_M as UGV_PASS_THROUGH_M,
};

use self::device_cap::CameraSimCap;
use crate::world_link::{
    MotionSender, WorldConsumers, WorldLink, WorldLinkHandle, MOTION_CHANNEL_CAP,
};

/// `[sim_world].chassis` 缺省值（世界目录里的差速车）
const DEFAULT_CHASSIS: &str = "diff_drive_ugv";

/// 底盘类型 → 活动栈（§12.6.5 选项 2：按世界零件名判，**不改 `Chassis` trait**）
///
/// 为什么不用 `Chassis::Kind()`：`12.6.2` 明确「不动 `chassis/`」，而 `Chassis::Name()`
/// 已经是校验过的世界零件名（`chassis::Build` 里 `Validate_Chassis` 之后才构造），足够判型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackKind {
    Ugv,
    Uav,
}

/// 世界零件名 → 栈。`[sim_world]` 缺省时 `world_name` 恒为 [`DEFAULT_CHASSIS`]（差速车）
fn Kind_Of_World(world_name: &str) -> StackKind {
    match world_name.trim() {
        "quadrotor_uav" => StackKind::Uav,
        // `diff_drive_ugv` 以及「无 [sim_world] 段」的缺省值都走车栈
        _ => StackKind::Ugv,
    }
}

/// 当前活动栈：`start()` 里按底盘类型定死，三个方法逐个转发（§12.6.1-2）
enum TaskStack {
    Ugv(ugv_sim::Stack),
    Uav(uav_sim::Stack),
}

impl TaskStack {
    async fn build(&self, task: &QueuedTask) -> Result<(), String> {
        match self {
            Self::Ugv(s) => s.build(task).await,
            Self::Uav(s) => s.build(task).await,
        }
    }

    async fn tick(&self, frame: &mut Frame<'_>) -> Option<(TaskId, Status)> {
        match self {
            Self::Ugv(s) => s.tick(frame).await,
            Self::Uav(s) => s.tick(frame).await,
        }
    }

    async fn finish(&self, id: TaskId) {
        match self {
            Self::Ugv(s) => s.finish(id).await,
            Self::Uav(s) => s.finish(id).await,
        }
    }

    async fn reset(&self) {
        match self {
            Self::Ugv(s) => s.reset().await,
            Self::Uav(s) => s.reset().await,
        }
    }

    /// `suspend`（§十一 #38）：只清 nav 目标，**不动实例表**（停车由调用方做）
    ///
    /// 两栈的 `nav` 字段同名同类型形状（`Arc<Mutex<Navigator>>`），但 `Navigator` 是两份
    /// 逐字拷贝的类型 → 必须分别 match，不能抽公共函数。
    async fn suspend(&self) {
        match self {
            Self::Ugv(s) => s.nav.lock().await.clear_goal(),
            Self::Uav(s) => s.nav.lock().await.clear_goal(),
        }
    }

    /// 设规划时间预算（Task 39 §二）：值来自 config 顶层键 `plan_budget_ms`
    ///
    /// 两栈的 `Navigator` 是两份逐字拷贝的类型 → 同 `suspend` 一样分别 match。
    async fn set_plan_budget(&self, budget: Duration) {
        match self {
            Self::Ugv(s) => s.nav.lock().await.set_plan_budget(budget),
            Self::Uav(s) => s.nav.lock().await.set_plan_budget(budget),
        }
    }

    /// 当前导航状态（只读观测；`status.nav_status` 用，**不参与决策**）
    async fn nav_status(&self) -> &'static str {
        match self {
            Self::Ugv(s) => match s.nav.lock().await.status() {
                ugv_sim::nav::NavStatus::Idle => "idle",
                ugv_sim::nav::NavStatus::Navigating => "navigating",
                ugv_sim::nav::NavStatus::Arrived => "arrived",
                ugv_sim::nav::NavStatus::Blocked(_) => "blocked",
            },
            Self::Uav(s) => match s.nav.lock().await.status() {
                uav_sim::nav::NavStatus::Idle => "idle",
                uav_sim::nav::NavStatus::Navigating => "navigating",
                uav_sim::nav::NavStatus::Arrived => "arrived",
                uav_sim::nav::NavStatus::Blocked(_) => "blocked",
            },
        }
    }

    fn next_tick_no(&self) -> u64 {
        match self {
            Self::Ugv(s) => s.next_tick_no(),
            Self::Uav(s) => s.next_tick_no(),
        }
    }

    /// 急停只挂车栈（§12.6.4「急停：有 / 无」）
    fn ugv_nav(&self) -> Option<&Arc<Mutex<ugv_sim::nav::Navigator>>> {
        match self {
            Self::Ugv(s) => Some(&s.nav),
            Self::Uav(_) => None,
        }
    }
}

/// 执行器三态（task_40 §二.8）：选型只由栈决定，`on_tick` 一次 `match` 分派
enum Actuator {
    /// 车栈遗留的离散三状态机（`sim/executor.rs`，与 `pleiades-ugv` 逐字相同）
    /// Step 1 起 sim 车栈改走 [`Self::UgvVelocity`]；本变体**保留为 A/B 回退点**
    /// （§二.8「不加配置开关，回退 = 回退代码」），只有回退时才由 [`Actuator_For`] 返回。
    #[allow(dead_code)]
    UgvDiscrete(DecisionExecutor),
    /// 车栈连续档位执行器（`ugv_sim/executor.rs`）
    UgvVelocity(UgvVelocityExecutor),
    /// 机栈连续速度执行器（`uav_sim/executor.rs`）
    Uav(UavExecutor),
}

/// 按栈装配执行器（task_40 §二.8）：车 → 连续档位（档位来自 `[chassis]`）；机 → 连续物理量（`[flight_ctrl]`）
///
/// 判据是**栈**（`Kind_Of_World` ← `[sim_world].chassis`），不借 `stack.ugv_nav()`（那个的语义是
/// 「急停支不支持」，不是「执行器用哪个」）。
fn Actuator_For(kind: StackKind, binding: SpeedBinding, vel_fwd: f32, yaw_rate_rps: f32) -> Actuator {
    match kind {
        StackKind::Ugv => {
            Actuator::UgvVelocity(UgvVelocityExecutor::new(binding.forward, binding.turn))
        }
        StackKind::Uav => Actuator::Uav(UavExecutor::new(vel_fwd, yaw_rate_rps)),
    }
}

/// A/B 回退点（§二.8）：车栈回到遗留离散三状态机的**唯一**构造点。
///
/// Step 1 起 `Actuator_For` 不再返回它；回退 = 让 `Actuator_For` 的 `Ugv` 分支返回本函数的结果。
/// 之所以保留成独立函数：`sim/executor.rs` 本 step **一行不改**（不能在那边加 `allow`），
/// 而 `DecisionExecutor::new` 一旦没人调用就变成死代码（新增 warning）—— 这里保留构造点即保住它。
#[allow(dead_code)]
fn Discrete_Actuator() -> Actuator {
    Actuator::UgvDiscrete(DecisionExecutor::new())
}

/// start() 后初始化的「设备组」（无硬件，只有决策组件 + 传感器 + 底盘）
struct SimInner {
    /// 任务栈（§12.6：按底盘类型二选一；持有 nav + 树实例表）
    stack: TaskStack,
    /// 执行器（三态，`start()` 按栈装配一次）：见 [`Actuator`]
    actuator: Actuator,
    /// 按配置构造的传感器（`shutdown()` 逐个停；建图 task 归它们所有）
    sensors: Vec<Arc<dyn Sensor>>,
    /// 急停用 lidar（Step 6）：`None` = 没连世界 / 没有 `lidar_2d` / 无 `[sim_world]` 段
    /// → `on_tick` 跳过急停（与真机 `lidar: None` 同语义，§八.2 第 6 行）。
    /// 与 `sensors` 里那个是**同一个 `Arc` 实例**（同一份扫描，不复制）
    lidar: Option<Arc<LidarSensor>>,
    /// 底盘（Step 4）：动作下发 + 手动命令 + 位姿写回（`RobotState` 单写者）。
    /// `None` = 无 `[chassis]` 段或 `enabled = false`（真机 `enabled=false` 同义）
    chassis: Option<Arc<dyn Chassis>>,
    /// 本地控制口的取消令牌（§七.6 要点 7：cancel 时全部断开；不依赖 Drop）
    control_cancel: CancellationToken,
    /// 任务观测器（§12.8.2 第 4 条）：`on_tick` 单写者，控制口 `status` 只读
    monitor: Arc<TaskMonitor>,
    /// detector 设备（task_37 Step 2）：`enabled=false` / 无相机 / 权重缺失 → `None`
    /// **只被 `shutdown()` 用**，`on_tick` 一行不碰
    detector: Option<Arc<crate::sim::detector::DetectorDevice>>,
}

pub struct SimDeviceHandler {
    robot: Arc<Robot>,
    config: SimConfig,
    node_handle: Arc<NodeHandle>,
    origin: (f32, f32, f32),
    inner: OnceLock<SimInner>,
    /// 世界链路句柄：`None` = 未配置 / `enabled = false`（此时门闩不生效，决策链照跑）
    world: OnceLock<WorldLinkHandle>,
    /// base 的能力容器（task_34 Step 4）：`start()` 里把相机 cap 推进 `device_caps`
    /// → `spawn_lua_script` 遍历时注册出 Lua 表 `camera_sim`。
    /// **只写 `device_caps`：不碰 `user_cmd_tx`、不碰控制口、不新增任何命令通路。**
    capabilities: Arc<Capabilities>,
}

impl SimDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: SimConfig,
        node_handle: Arc<NodeHandle>,
        origin: (f32, f32, f32),
        capabilities: Arc<Capabilities>,
    ) -> Self {
        Self {
            robot,
            config,
            node_handle,
            origin,
            inner: OnceLock::new(),
            world: OnceLock::new(),
            capabilities,
        }
    }

    /// 按 `[chassis]` 建底盘（§六.5.7 的取舍：**无段 → 不建底盘**；段存在但没写 `enabled` → 默认 true）
    ///
    /// 返回 `(底盘, 建图用位姿时钟)`：无底盘时给一个「从未收到」的时钟（`0`），
    /// 于是 `Age_Of` 恒 `u64::MAX` → 建图永远跳过（与真机「无底盘 → 无位姿 → 不建图」一致）。
    fn Build_Chassis(
        &self,
        world_name: &str,
        motion: MotionSender,
    ) -> Result<(Option<Arc<dyn Chassis>>, Arc<AtomicU64>, Arc<PoseHistory>), String> {
        let cfg = match self.config.chassis.as_ref() {
            None => {
                warn!(
                    "[chassis] 未配置 [chassis] 段：不建底盘，on_tick 保持 Idle（车不会动；请在 .config/config.toml 加 [chassis] forward_speed/turn_speed）"
                );
                return Ok((None, Arc::new(AtomicU64::new(0)), PoseHistory::New()));
            }
            Some(c) if !c.enabled.unwrap_or(true) => {
                warn!("[chassis] 未启用（[chassis].enabled = false）：不建底盘，on_tick 保持 Idle");
                return Ok((None, Arc::new(AtomicU64::new(0)), PoseHistory::New()));
            }
            Some(c) => c,
        };
        // 名字非法 → Err（catalog 已在 start() 里先拦一次，这里由 chassis::Build 再兜一次）
        let chassis = chassis::Build(world_name, cfg, motion, self.robot.robot_state.clone())?;
        let pose_age = chassis.Pose_Age_Handle();
        let pose_history = chassis.Pose_History();
        Ok((Some(chassis), pose_age, pose_history))
    }
}

/// 急停用哪个 lidar（§八.5.2 注）
///
/// 规则：**优先 slot 名以 `lidar_` 开头的第一个，否则回退到声明顺序第一个**。
/// （纯前缀规则会让「有雷达但 slot 不叫 lidar_*」的车**永不急停**——静默失效比"用错一个"更危险。）
/// 多于一个时打一条 `warn!`（急停只吃一个；多雷达融合不在本 step，§八.8）。
fn Select_Lidar(lidars: &[Arc<LidarSensor>]) -> Option<Arc<LidarSensor>> {
    if lidars.len() > 1 {
        warn!(
            "[Sim] 配置了 {} 个 lidar（{}）：急停只使用其中 1 个（不实现多雷达融合）",
            lidars.len(),
            lidars.iter().map(|l| l.Slot()).collect::<Vec<_>>().join(",")
        );
    }
    lidars
        .iter()
        .find(|l| l.Slot().starts_with("lidar_"))
        .or_else(|| lidars.first())
        .cloned()
}

/// 相机用哪个 `slot`（`camera_sim` 只能暴露**一个**，与急停只吃一个 lidar 同一取舍）
///
/// 规则与 [`Select_Lidar`] 同法：**优先 slot 名以 `camera_` 开头的第一个，否则回退到声明顺序第一个**
/// （纯前缀规则会让"有相机但 slot 不叫 camera_*"的车**永远没有 camera_sim** ——
/// 静默失效比"用错一个"更危险）。
/// 多于一个时打一条 `warn!`：脚本里只有 `camera_sim` 一个表名，不做 `camera_sim_<slot>` 之类的新别名
/// （多相机不在本 task）。
fn Select_Camera(set: &SensorSet) -> Option<Arc<CameraSensor>> {
    let cameras = set.Cameras();
    if cameras.len() > 1 {
        warn!(
            "[camera] 配置了 {} 个 camera（{}）：camera_sim 只暴露其中 1 个（不实现多相机别名）",
            cameras.len(),
            cameras.iter().map(|c| c.Slot()).collect::<Vec<_>>().join(",")
        );
    }
    let picked = cameras
        .iter()
        .find(|c| c.Slot().starts_with("camera_"))
        .or_else(|| cameras.first())
        .cloned();
    // 用 `Get_Camera` 再取一次同一个实例（同 slot 同实例）：顺带把类型化访问器变成真实调用点
    match picked {
        Some(c) => set.Get_Camera(c.Slot()).or(Some(c)),
        None => None,
    }
}

/// 协助请求帧（**纯函数**，便于单测钉死字节契约）：msgid 7 + sysid=UAV peer_id + compid=COMPID_ROBOT
fn assist_frame(sysid: &[u8], x: f32, y: f32) -> Vec<u8> {
    let payload = encode_assist_request(x, y);
    encode_frame(MSGID_ASSIST_REQUEST, sysid, COMPID_ROBOT, &payload)
}

/// 协助请求的生产发布者（task_41 §5.5.7）：树 → gossipsub `TOPIC_ROBOT_ASSIST_REQUEST`
///
/// 为什么在这里而不在 `containment.rs`：发送是**传输层**的事（要 `tracing` 限流告警、要 `NodeHandle`），
/// 而 `containment.rs` 有「不引 `tracing`」的纪律（10 Hz tick 防刷屏）。
struct NodeHandlePublisher {
    node_handle: Arc<NodeHandle>,
    /// 本机 PeerId 字节（= 帧的 `sysid`，Pictor 侧据此知道请求者）
    local_peer_id: Vec<u8>,
}

impl AssistPublisher for NodeHandlePublisher {
    fn publish(&self, x: f32, y: f32) {
        let frame = assist_frame(&self.local_peer_id, x, y);
        if let Err(e) = self.node_handle.Gossipsub_Publish_Try(TOPIC_ROBOT_ASSIST_REQUEST, frame) {
            // 失败只限流 warn（通道满 = 丢一帧，目标下次移动再发；不影响树）
            Warn_Throttled("sim.uav.assist", 1000, || {
                format!("[Sim/uav] 协助请求广播失败：{e}")
            });
        }
    }
}

#[async_trait]
impl DeviceHandler for SimDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let obstacle_inflation_radius = self.config.obstacle_inflation_radius.unwrap_or(0.2);
        // 规划时间预算（Task 39 §二）：config 顶层键 `plan_budget_ms`，缺省 20 ms
        let plan_budget_ms = self.config.plan_budget_ms.unwrap_or(DEFAULT_PLAN_BUDGET_MS);

        // ---- 1) 世界链路配置：**必须最先**解出（底盘名来自 `[sim_world].chassis`）
        //   - 配置非法 → start() 直接 Err（base 会打印「设备启动失败」并停掉主循环）
        //   - chassis 名不在本地目录 → 同样 Err（不让世界踩 1002）
        //   - sensor 名不在本地目录 → 跳过该 slot + warn（不建连接、不进 manifest）
        //   - enabled = false → 不连世界（门闩不生效，决策链照跑，等价现状）
        let world_cfg = match self.config.sim_world.as_ref() {
            Some(cfg) if cfg.Enabled() => {
                let mut resolved = cfg.Resolved()?;
                crate::world_link::catalog::Validate_Chassis(&resolved.chassis)?;
                resolved.sensors.retain(|s| {
                    match crate::world_link::catalog::Validate_Sensor(&s.sensor) {
                        Ok(()) => true,
                        Err(e) => {
                            warn!(
                                "[world] 跳过传感器 slot='{}'：{e}（不建连接、不进 manifest）",
                                s.slot
                            );
                            false
                        }
                    }
                });
                if resolved.sensors.is_empty() {
                    warn!(
                        "[world] 传感器清单为空（slots=0）：世界会立刻 assembly_ready，这辆车没有任何传感器"
                    );
                }
                info!(
                    "[world] 世界链路启动 url={} chassis={} slots={}",
                    resolved.url,
                    resolved.chassis,
                    resolved.sensors.len()
                );
                Some(resolved)
            }
            Some(_) => {
                info!("[world] 已禁用（[sim_world].enabled = false）：只跑决策链");
                None
            }
            None => None,
        };
        // 无 `[sim_world]` 时底盘名取默认（此时也连不上世界，动作只会被丢弃 + debug）
        let world_name = world_cfg
            .as_ref()
            .map(|w| w.chassis.clone())
            .unwrap_or_else(|| DEFAULT_CHASSIS.to_string());

        // ---- 2) 动作上行通道：先建通道，发送端给底盘 + `WorldConsumers`，接收端给 `Spawn`
        //        （§六.5.6 偏差 B：建通道必须从 `Spawn` 内部挪到外面，否则形成死循环）
        let (motion, motion_rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);

        // ---- 3) 底盘：动作翻译 / 速度绑定 / 位置更新（Step 4）
        let (chassis, pose_age, pose_history) = self.Build_Chassis(&world_name, motion.clone())?;

        // ---- 4) 传感器（在 `WorldLink::Spawn` 之前建好：Spawn 要 `Arc<SensorSet>`）
        //      `pose_age` 与底盘是**同一个** `Arc`（建图 task 判新鲜度用）
        let mut sensors: Vec<Arc<dyn Sensor>> = Vec::new();
        // 急停用 lidar 句柄（Step 6）：与 `set` 里那个是同一个实例，`set` 随后被 move 进
        // `WorldConsumers`，所以要在这里先取出来
        let lidar: Option<Arc<LidarSensor>>;
        // 相机句柄（task_34 Step 4）：与 `set` 里那个是同一个实例；`set` 随后被 move 进
        // `WorldConsumers`，所以必须在 `self.inner.set(...)` 之前取出来（推 cap 用）
        let camera: Option<Arc<CameraSensor>>;
        // 本机 PeerId 的 base58：世界 hello 与控制口的 `status` **共用**（Step 5 起只算一次）
        let peer_id = self.node_handle.Get_Local_Peer_Id().to_base58();
        let handle = match world_cfg {
            Some(resolved) => {
                let ctx = MappingCtx {
                    grid: self.robot.grid.clone(),
                    robot_state: self.robot.robot_state.clone(),
                    cluster_table: self.robot.cluster_table.clone(),
                    map_tx: self.robot.map_tx.clone(),
                    node_handle: Some(self.node_handle.clone()),
                    local_peer_id: Some(self.node_handle.Get_Local_Peer_Id().to_bytes()),
                    obstacle_inflation_radius,
                    pose_age,
                    pose_history: pose_history.clone(),
                    // 兜底默认：`SensorSet::Build` 会按每个传感器自己的 `[sim_world.sensor.mount]` 覆盖
                    mount: Default::default(),
                };
                let set = Arc::new(SensorSet::Build(&resolved.sensors, &ctx)?);
                info!(
                    "[world] 传感器就绪 slots={}（lidar_2d 自带 200 ms 建图 task；camera 只保留最新一帧）",
                    set.Len()
                );
                // 急停的雷达（§八.5.2 注）：必须先于 `set` 被 move 取出
                lidar = Select_Lidar(set.Lidars());
                match &lidar {
                    Some(l) => info!(
                        "[Sim] 急停使用 lidar slot={}（前方 ±45°，阈值 {} m）",
                        l.Slot(),
                        emergency_stop::OBSTACLE_THRESHOLD_M
                    ),
                    None => info!(
                        "[Sim] 无 lidar（未配置 lidar_2d）：on_tick 跳过急停（真机 lidar=None 同语义）"
                    ),
                }
                // 相机（task_34 Step 4）：与 lidar 同时机取出（`set` 马上被 move）
                camera = Select_Camera(&set);
                match &camera {
                    Some(c) => info!(
                        "[camera] camera_sim 使用 slot={} rate_hz={}（世界按 rate_hz 推 type=2 帧）",
                        c.Slot(),
                        c.Params().rate_hz
                    ),
                    None => info!(
                        "[camera] 无 camera 零件（未配置 sensor=\"camera\"）：Lua 里不会有 camera_sim"
                    ),
                }
                sensors = set.All().to_vec();
                // ---- 5) 世界链路：只 spawn，**绝不 await 网络**
                Some(WorldLink::Spawn(
                    resolved,
                    peer_id.clone(),
                    WorldConsumers { sensors: set, chassis: chassis.clone(), motion },
                    motion_rx,
                ))
            }
            None => {
                lidar = None;
                camera = None;
                info!("[Sim] 未连世界：无传感器 → on_tick 跳过急停（真机 lidar=None 同语义）");
                // 不连世界：接收端在此丢弃。`MotionSender` 未装配 → 动作只 `debug!` 丢弃，
                // 不会落到 `try_send` 报「通道关闭」，因此无日志噪音（保留纯决策链空跑观感）
                drop(motion_rx);
                debug!("[world] 未连世界：动作通道接收端丢弃（MotionSender 未装配，动作不进通道）");
                None
            }
        };
        // 控制口的 `status` 需要世界链路句柄：先留一份，再把原件放进 `self.world`
        let world_for_control = handle.clone();
        if let Some(h) = handle {
            let _ = self.world.set(h);
        }

        // ---- 5.5) detector 设备（task_37 Step 2）：帧源 = 本车相机的时间槽位
        //          只读相机槽位、只写自己的 `Mutex<Option<Detections>>`；
        //          **不进 on_tick、不写 RobotState / ExecuteState、不碰树**（§三.6 判据 5）。
        let detector = match (self.config.detector.as_ref(), camera.as_ref()) {
            (None, _) => {
                info!("[detector] 未配置 [detector] 段：不 spawn");
                None
            }
            (Some(cfg), _) if !cfg.Enabled() => {
                info!("[detector] enabled=false：不 spawn（零线程、不占 CPU）");
                None
            }
            (Some(_), None) => {
                warn!("[detector] 已启用但没有 camera 传感器（未配 sensor=\"camera\"）：detector 不启动");
                None
            }
            (Some(cfg), Some(cam)) => {
                let source: Arc<dyn crate::sim::detector::FrameSource> = cam.clone();
                match crate::sim::detector::DetectorDevice::spawn(
                    cfg.clone(),
                    source,
                    self.robot.robot_state.clone(),
                )
                .await
                {
                    Ok(d) => {
                        info!(
                            "[detector] detector 设备已 spawn（model={} device={}）",
                            cfg.Model(),
                            cfg.Device()
                        );
                        Some(d)
                    }
                    Err(e) => {
                        warn!("[detector] 未启动（只 warn，不影响仿真）：{e}");
                        None
                    }
                }
            }
        };

        // ---- 5.6) 任务栈（§12.6.5）：底盘名 → 车栈 / 机栈，二选一后全程转发
        //      **必须放在 5.5 之后**：机栈的 `TaskDeps` 要 detector（task_37 Step 4 跟随）。
        //      本块原先在 3.5，与 4)/5.5) 无任何相互依赖 → 纯代码下移，语义不变。
        //      车栈订建图增量（`map_tx`），机栈不订（D6：真机两边都不订，sim 只有车栈订）
        let kind = Kind_Of_World(&world_name);
        let stack = match kind {
            StackKind::Ugv => TaskStack::Ugv(ugv_sim::Stack::new(
                self.robot.grid.clone(),
                self.robot.cluster_table.clone(),
                self.robot.robot_state.clone(),
                obstacle_inflation_radius,
                Some(&self.robot.map_tx),
            )),
            StackKind::Uav => TaskStack::Uav(uav_sim::Stack::new(
                self.robot.grid.clone(),
                self.robot.cluster_table.clone(),
                self.robot.robot_state.clone(),
                obstacle_inflation_radius,
                None,
                detector.clone(), // 车栈不传：ugv_sim 收到 Follow 走平凡树
                // 协助请求发布者（task_41 §5.3）：sysid = 本机 PeerId，与 POSE/MAP 广播同源
                Some(Arc::new(NodeHandlePublisher {
                    node_handle: self.node_handle.clone(),
                    local_peer_id: self.node_handle.Get_Local_Peer_Id().to_bytes(),
                }) as Arc<dyn AssistPublisher>),
            )),
        };
        // 规划时间预算（Task 39 §二）：`Stack::new` 不收预算（它的调用点不在本 step 允许路径内），
        // 故建栈之后紧跟注入；值来自 config 顶层键 `plan_budget_ms`。
        stack.set_plan_budget(Duration::from_millis(plan_budget_ms)).await;
        info!("[Sim] 任务栈 = {:?}（底盘 {}）", kind, world_name);

        // ---- 5.7) 执行器三态（task_40 §二.8）：车 → 连续档位（取 `[chassis]` forward/turn，百分比）；
        //      机 → 连续物理量（取 `[flight_ctrl]` 的 vel_fwd / yaw_rate_deg；缺段默认 0.3 m/s / 15°/s）。
        //      **口径与真机一致**：车的档位是百分比（换算在世界侧 `DiffDrive.gd`），
        //      机的速度是物理量（§四 修订），两边各自沿用真机的单位，不再互相借。
        let binding = SpeedBinding::From_Config(self.config.chassis.as_ref());
        let (vel_fwd, yaw_rate_deg) = self
            .config
            .flight_ctrl
            .as_ref()
            .map(|c| c.Resolved())
            .unwrap_or((DEFAULT_VEL_FWD, DEFAULT_YAW_RATE_DEG));
        let actuator = Actuator_For(kind, binding, vel_fwd, yaw_rate_deg.to_radians());
        match &actuator {
            Actuator::UgvVelocity(_) => info!(
                "[Sim] 车栈执行器 = 连续档位（forward_speed={} turn_speed={}，DEADBAND={:.0}°，ARC_MAX={:.0}°，PASS_THROUGH={:.2} m）",
                binding.forward, binding.turn, UGV_DEADBAND_DEG, UGV_ARC_MAX_DEG, UGV_PASS_THROUGH_M
            ),
            Actuator::Uav(_) => info!(
                "[Sim] 机栈执行器 = 连续速度（vel_fwd={:.2} m/s，yaw_rate={:.1}°/s={:.3} rad/s，DEADBAND={:.0}°，SPIN={:.0}°）",
                vel_fwd, yaw_rate_deg, yaw_rate_deg.to_radians(), DEADBAND_DEG, SPIN_DEG
            ),
            Actuator::UgvDiscrete(_) => {
                info!("[Sim] 车栈执行器 = 离散三状态机（A/B 回退路径）")
            }
        }

        // ---- 6) 本地控制口（Step 5）：取消令牌进 `SimInner`，`shutdown()` 先 cancel 它
        let control_cancel = CancellationToken::new();
        // ---- 6.1) 任务观测器（§12.8.2 第 4 条）：控制口与 on_tick 共用同一个 `Arc`
        let monitor = Arc::new(TaskMonitor::default());
        self.inner
            .set(SimInner {
                stack,
                actuator,
                sensors,
                lidar,
                chassis,
                control_cancel: control_cancel.clone(),
                monitor: monitor.clone(),
                detector,
            })
            .map_err(|_| "设备已启动".to_string())?;
        // ---- 6.5) 相机 cap 推进 `device_caps`（task_34 Step 4）
        //          只写 base 的能力容器：Lua 表 `camera_sim` 由 `spawn_lua_script` 遍历时注册。
        //          **不新增 `user_cmd_tx`、不碰控制口、不改 on_tick。**
        if let Some(cam) = camera {
            let cap: Arc<dyn DeviceCapability> = Arc::new(CameraSimCap::New(cam));
            match self.capabilities.device_caps.write() {
                Ok(mut g) => g.push(cap),
                // 锁中毒不 panic（与本 crate 其它锁同纪律），取回内层继续写
                Err(poisoned) => poisoned.into_inner().push(cap),
            }
            info!("[camera] camera_sim 已注入 Lua（capture() / info()）");
        }
        info!("[Sim] 模拟设备启动（无硬件：决策链 + 底盘 + 传感器，动作经世界链路下发）");
        // ---- 7) 起控制口（**在 inner.set 之后**：注入的 Command 走到 device.reset() 时 inner 已就绪）
        //         无 `[control]` 段 → `unwrap_or_default()` → 默认开启（§七.3）
        crate::sim::control::Spawn(
            self.config.control.clone().unwrap_or_default(),
            self.robot.clone(),      // robot_cmd_tx / op_mode / robot_state / mission_queue
            world_for_control,       // `status` 的 assembled / avatar_id
            peer_id,
            monitor,                 // §12.8.2 #4：task_ticks / nav_status
            control_cancel,
        );
        Ok(())
    }

    async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        // 与真机 CarDeviceHandler 同构：**先拦截雷达命令**，其余交底盘（设备选择性响应）
        if matches!(cmd, ManualCmd::StartLidarScan | ManualCmd::StopLidarScan) {
            info!("[Sim] 雷达由世界驱动，忽略手动雷达命令: {cmd:?}");
            return;
        }
        let d = match self.inner.get() {
            Some(d) => d,
            None => {
                info!("[Sim] 设备未启动，忽略手动命令: {cmd:?}");
                return;
            }
        };
        match d.chassis.as_ref() {
            Some(c) => c.Handle_Manual_Cmd(cmd),
            None => info!("[Sim] 无底盘（[chassis] 未启用）：手动命令忽略: {cmd:?}"),
        }
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        // 停车优先（安全关键），再 halt 树 + 清实例表 + 清 nav（§十一 #38）
        if let Some(c) = d.chassis.as_ref() {
            c.Stop();
        }
        d.stack.reset().await;
        d.monitor.Clear(); // §十一 #38：replace / 切模式会**丢弃实例** → 计数表一并对齐
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        info!("[Sim] reset（停车 + halt 树 + 清实例表 + 清 nav + 清意图）");
    }

    async fn suspend(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        // §十一 #38：停车优先（安全关键）→ 清 nav 目标 → 清意图；
        // **不动 `d.stack` 的实例表**（`TaskStack::suspend` 只清 nav），
        // **不清 monitor**（被挂起任务的 `task_ticks` 必须留着，用例 5/6 靠它判定）。
        if let Some(c) = d.chassis.as_ref() {
            c.Stop();
        }
        d.stack.suspend().await;
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        info!("[Sim] suspend（停车 + 清 nav；树实例与 tick 计数保留，等恢复）");
    }

    async fn build_task(&self, task: &QueuedTask) -> Result<(), String> {
        let d = self.inner.get().expect("device not started");
        d.stack.build(task).await
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) -> TickOutcome {
        // 装配门闩①（必须最先）：未 Assembled → 写 Idle 直接 return，不寻路、不决策、不消费任务
        if let Some(w) = self.world.get() {
            if !w.Is_Assembled().await {
                *execute_state.write().await =
                    ExecuteState { state: DecisionState::Idle, sub_target: None };
                return TickOutcome::Running;
            }
        }

        let d = self.inner.get().expect("device not started");
        // 门闩②（Step 4）：无底盘 → 保持 Idle（与真机 `enabled=false` / 无 stm32 语义一致）
        let Some(ch) = d.chassis.as_ref() else {
            *execute_state.write().await =
                ExecuteState { state: DecisionState::Idle, sub_target: None };
            return TickOutcome::Running;
        };

        // ③ 急停（真机 `on_tick` 的第一步；只车栈有，§12.6.4）：命中 → 停车 + Idle，
        //    **保留 goal**，树这一拍不 tick（时间冻结）
        let emergency = match (d.stack.ugv_nav(), d.lidar.as_ref()) {
            (Some(nav), Some(l)) => {
                Check_Emergency_Stop(l.Get_Scan().await, rs, ch.as_ref(), nav, l.Mount()).await
            }
            _ => false,
        };
        if emergency {
            ch.Stop();
            *execute_state.write().await =
                ExecuteState { state: DecisionState::Idle, sub_target: None };
            return TickOutcome::Running;
        }

        // ④ 树：推进当前任务的树（叶子写 `Frame.out.next_cell`）
        let mut out = MotionOut::default();
        let tick_no = d.stack.next_tick_no();
        let mut frame = Frame { dt_ms: AUTO_TICK_MS as u32, tick_no, out: &mut out };
        let status = d.stack.tick(&mut frame).await;

        // ④.5 观测（§12.8.2 #4）：只记「这一拍哪棵树被 tick 了」+ nav 状态，**不影响决策**
        if let Some((id, _)) = status {
            d.monitor.Note_Tick(id);
        }
        d.monitor.Set_Nav(d.stack.nav_status().await);

        // ⑤ 决策（`ExecuteState` **唯一写点**）+ 执行动作（交底盘翻译成世界 `motion_cmd`）
        //    分派（task_40 §二.8）：三态 `Actuator` —— 车栈连续档位 / 机栈连续速度各自一臂，
        //    车栈遗留离散三状态机保留为 A/B 回退臂（语义一行不改）。
        //    两个连续臂**只写 `sub_target`**，`state` 留 `Idle`（唯一的读点
        //    `base/Robot/core/robot.rs:250-253` 只取 `sub_target`）。
        match &d.actuator {
            Actuator::UgvDiscrete(ex) => {
                // 离散三状态机读当前 `ExecuteState`（连续两臂不读，故 `current` 只在本臂克隆）
                let current = execute_state.read().await.clone();
                let result = ex.decide(out.next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
                *execute_state.write().await = ExecuteState {
                    state: result.state,
                    sub_target: result.sub_target,
                };
                if let Some(action) = result.action {
                    info!(
                        "[Sim] 位置=({:.2},{:.2}) yaw={:.2}° 状态={:?} 动作={:?} sub_target={:?} 位姿龄={}ms",
                        rs.x,
                        rs.y,
                        rs.attitude.yaw.to_degrees(),
                        result.state,
                        action,
                        result.sub_target,
                        ch.Pose_Age_Ms()
                    );
                    ch.Apply_Action(action);
                }
            }
            Actuator::UgvVelocity(ex) => {
                let cmd = ex.decide(out.next_cell, rs.x, rs.y, rs.attitude.yaw);
                *execute_state.write().await =
                    ExecuteState { state: DecisionState::Idle, sub_target: out.next_cell };
                // 20 Hz 全程有输出 → Should_Log 压到 5 Hz；**字段名固定**
                // （Step 2 的 `[Sim] UGV` 行解析依赖它）
                if Should_Log("sim.ugv.exec", 200) {
                    info!(
                        "[Sim] UGV 位置=({:.2},{:.2}) yaw={:.1}° next={:?} → speed={} turn={} 位姿龄={}ms",
                        rs.x,
                        rs.y,
                        rs.attitude.yaw.to_degrees(),
                        out.next_cell,
                        cmd.speed,
                        cmd.turn,
                        ch.Pose_Age_Ms()
                    );
                }
                if !ch.Apply_Velocity_Pct(cmd.speed, cmd.turn) {
                    // 判型与底盘构造同一个来源（`stack kind` ← `[sim_world].chassis`），理论上不会发生；
                    // 真发生就按最安全的方式收场：停车 + 1 s 限流告警（不静默，也不自动回落离散）
                    Warn_Throttled("sim.ugv.no_velocity", 1000, || {
                        "[Sim] 底盘不支持 Apply_Velocity_Pct：按停车处理（检查 [sim_world].chassis 与栈是否一致）"
                            .to_string()
                    });
                    ch.Stop();
                }
            }
            Actuator::Uav(ex) => {
                let cmd = ex.decide(out.next_cell, rs.x, rs.y, rs.attitude.yaw);
                *execute_state.write().await =
                    ExecuteState { state: DecisionState::Idle, sub_target: out.next_cell };
                // 20 Hz 全程有输出，用 Should_Log 压到 5 Hz（日志仍够查"有没有 100~200 ms 抖动"）
                if Should_Log("sim.uav.exec", 200) {
                    info!(
                        "[Sim] UAV 位置=({:.2},{:.2}) yaw={:.1}° next={:?} → v={} omega={} 位姿龄={}ms",
                        rs.x,
                        rs.y,
                        rs.attitude.yaw.to_degrees(),
                        out.next_cell,
                        cmd.v,
                        cmd.omega,
                        ch.Pose_Age_Ms()
                    );
                }
                if !ch.Apply_Velocity(cmd.v, cmd.omega) {
                    // 判型与底盘构造同一个来源（`stack kind` ← `[sim_world].chassis`），理论上不会发生；
                    // 真发生就按最安全的方式收场：停车 + 1 s 限流告警（不静默）
                    Warn_Throttled("sim.uav.no_velocity", 1000, || {
                        "[Sim] 底盘不支持 Apply_Velocity：按停车处理（检查 [sim_world].chassis 与栈是否一致）"
                            .to_string()
                    });
                    ch.Stop();
                }
            }
        }

        // ⑥ 终值：halt + 丢实例 + 清 nav → 交给 base 出队（唯一出队口，§十一 #16）
        let Some((id, st)) = status else { return TickOutcome::Running; };
        let code = match st {
            Status::Running => return TickOutcome::Running,
            Status::Success => None,
            Status::Failure(c) => {
                warn!("[Sim] 任务 {:?} 失败，原因码 {}", id, c.code());
                Some(c)
            }
        };
        d.stack.finish(id).await;
        d.monitor.Forget(id); // 终值 → 该任务的 tick 计数作废（下一次是新实例）
        TickOutcome::Terminated(code)
    }

    fn stop(&self) {
        // 同步 fn：底盘走 G1 优先通道（通道满则登记补发，绝不静默丢）
        match self.inner.get().and_then(|d| d.chassis.as_ref()) {
            Some(c) => c.Stop(),
            None => debug!("[Sim] 无底盘：stop 无需下发（没有任何动作通路）"),
        }
        info!("[Sim] stop");
    }

    async fn shutdown(&self) {
        // 先停控制口（不再接受新注入），再走原有的停车 / 停链路收尾（Step 5）
        if let Some(d) = self.inner.get() {
            d.control_cancel.cancel();
        }
        if let Some(c) = self.inner.get().and_then(|d| d.chassis.as_ref()) {
            c.Stop(); // 先发 stop（进通道），再停链路
        }
        if let Some(w) = self.world.get() {
            w.Shutdown(); // 同步 cancel + 关闭动作门闩
            tokio::time::sleep(std::time::Duration::from_millis(100)).await; // 给 recv task 收尾窗口
        }
        // 传感器：cancel 各自的建图 task（与 WorldLink 的取消互相独立）
        // detector：先停推理 task（它只读相机槽位，停它不影响相机与决策链）
        if let Some(d) = self.inner.get() {
            if let Some(det) = &d.detector {
                det.shutdown();
            }
            d.sensors.iter().for_each(|s| s.Shutdown());
        }
        info!("[Sim] shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §12.6.7 #3：差速底盘 → 车栈；UAV 零件 → 机栈；`[sim_world]` 缺省 → 车栈
    #[test]
    fn test_stack_kind_by_world_name() {
        assert_eq!(Kind_Of_World("diff_drive_ugv"), StackKind::Ugv);
        assert_eq!(Kind_Of_World("quadrotor_uav"), StackKind::Uav);
        assert_eq!(Kind_Of_World(" diff_drive_ugv "), StackKind::Ugv, "名字要 trim");
        assert_eq!(Kind_Of_World(DEFAULT_CHASSIS), StackKind::Ugv, "[sim_world] 缺省 = 差速车");
    }

    /// §二.8：车的选型只能是连续档位（离散路径不再被 `start()` 选中），机的选型只能是连续速度
    #[test]
    fn test_actuator_select_by_stack_kind() {
        let binding = SpeedBinding { forward: 30, turn: 30 };
        let ugv = Actuator_For(StackKind::Ugv, binding, 0.3, 0.26);
        assert!(
            matches!(ugv, Actuator::UgvVelocity(_)),
            "车栈必须选连续档位执行器（离散路径留给回退）"
        );
        if let Actuator::UgvVelocity(ex) = &ugv {
            // 无目标 = 停；档位原值来自 `[chassis]`
            assert_eq!(
                ex.decide(None, 0.25, 0.25, 0.0),
                crate::sim::ugv_sim::executor::VelocityCmd { speed: 0, turn: 0 }
            );
        }
        let uav = Actuator_For(StackKind::Uav, binding, 0.3, 0.26);
        assert!(matches!(uav, Actuator::Uav(_)), "机栈必须选连续速度执行器");
    }

    /// 回退路径（A/B）：离散变体仍可用、语义一行未被改坏（位姿在格心正南 0.35 m、朝北 ⇒ Δθ = 0）
    #[test]
    fn test_ugv_discrete_variant_still_works() {
        let act = Discrete_Actuator();
        let Actuator::UgvDiscrete(ex) = &act else { unreachable!("`Discrete_Actuator` 只产出离散变体") };
        let result = ex.decide(
            Some((20, 20)),
            10.25,
            9.90,
            std::f32::consts::FRAC_PI_2,
            &ExecuteState::default(),
        );
        assert_eq!(result.state, DecisionState::Moving);
        assert_eq!(
            result.action,
            Some(pleiades_base::robot::core::state::MotionAction::MoveForward)
        );
        assert_eq!(result.sub_target, Some((20, 20)));
    }

    /// task_41 §5.3：字节契约 —— `assist_frame(34 B sysid, 1.0, 2.0)` → 54 B，msgid=7 / compid=1 / payload 8 B 金标
    #[test]
    fn test_assist_frame_contract() {
        let sysid = vec![0x11u8; 34];
        let frame = assist_frame(&sysid, 1.0, 2.0);
        assert_eq!(frame.len(), 10 + 34 + 8 + 2, "10 头 + sysid + 8 payload + 2 checksum");
        let dec = pleiades_base::robot::core::protocol::decode_frame(&frame).expect("帧可解");
        assert_eq!(dec.msgid, MSGID_ASSIST_REQUEST);
        assert_eq!(dec.msgid, 7, "msgid 恒 7");
        assert_eq!(dec.compid, COMPID_ROBOT);
        assert_eq!(dec.sysid, sysid, "sysid = 本机 PeerId（Pictor 据此认请求者）");
        assert_eq!(
            dec.payload,
            vec![0x3F, 0x80, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00],
            "x=1.0 / y=2.0 大端金标"
        );
    }
}
