//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 模拟设备处理器（无硬件，用于测试决策链）
//!
//! 不 spawn stm32/lidar 串口；`on_tick` 仍跑完整决策链（`GoalService` 寻路 + `DecisionExecutor`
//! 三状态机），「执行动作」交给底盘（[`Chassis::Apply_Action`]），由底盘翻译成世界的 `motion_cmd`。
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
//!   逐项一致：**装配门闩 → 无底盘返回 → 急停 → 寻路 → 决策 → 写 ExecuteState → 执行**
//! - `start()` 从 `SensorSet` 里选出急停用 lidar（[`Select_Lidar`]）存进 `SimInner.lidar`；
//!   无雷达 / 未连世界 → `None` → `on_tick` 跳过急停（真机 `lidar: None` 同语义）
//! - 急停**每 tick 复核**、不锁存；触发时 `Stop()` + `ExecuteState = Idle` + **保留 goal**（供绕行）

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::SimConfig;
use crate::sim::chassis::{self, Chassis};
use crate::sim::chassis::pose_ingest::PoseHistory;
use crate::sim::emergency_stop::{self, Check_Emergency_Stop};
use crate::sim::executor::DecisionExecutor;
use crate::sim::goal::{GoalService, ARRIVAL_THRESHOLD_M};
use crate::sim::sensors::lidar::mapping_task::MappingCtx;
use crate::sim::sensors::lidar::LidarSensor;
use crate::sim::sensors::{Sensor, SensorSet};
use crate::world_link::{
    MotionSender, WorldConsumers, WorldLink, WorldLinkHandle, MOTION_CHANNEL_CAP,
};

/// `[sim_world].chassis` 缺省值（世界目录里的差速车）
const DEFAULT_CHASSIS: &str = "diff_drive_ugv";

/// start() 后初始化的「设备组」（无硬件，只有决策组件 + 传感器 + 底盘）
struct SimInner {
    goal_service: Arc<Mutex<GoalService>>,
    executor: DecisionExecutor,
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
}

pub struct SimDeviceHandler {
    robot: Arc<Robot>,
    config: SimConfig,
    node_handle: Arc<NodeHandle>,
    origin: (f32, f32, f32),
    inner: OnceLock<SimInner>,
    /// 世界链路句柄：`None` = 未配置 / `enabled = false`（此时门闩不生效，决策链照跑）
    world: OnceLock<WorldLinkHandle>,
}

impl SimDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: SimConfig,
        node_handle: Arc<NodeHandle>,
        origin: (f32, f32, f32),
    ) -> Self {
        Self { robot, config, node_handle, origin, inner: OnceLock::new(), world: OnceLock::new() }
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

#[async_trait]
impl DeviceHandler for SimDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let obstacle_inflation_radius = self.config.obstacle_inflation_radius.unwrap_or(0.2);

        // 无硬件：只建决策链组件（寻路 + 决策器）
        let mut gs = GoalService::new(
            self.robot.robot_state.clone(),
            self.robot.mission_queue.clone(),
            self.node_handle.Get_Local_Peer_Id().to_bytes(),
            self.robot.grid.clone(),
            self.robot.cluster_table.clone(),
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        );
        // 建图增量 → D* 修补（Step 7：没有这条通路，画出来的墙对规划不可见）
        gs.Subscribe_Map(&self.robot.map_tx);
        let goal_service = Arc::new(Mutex::new(gs));

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
                    "[world] 传感器就绪 slots={}（每个 lidar_2d 自带 200 ms 建图 task）",
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

        // ---- 6) 本地控制口（Step 5）：取消令牌进 `SimInner`，`shutdown()` 先 cancel 它
        let control_cancel = CancellationToken::new();
        self.inner
            .set(SimInner {
                goal_service,
                executor: DecisionExecutor::new(),
                sensors,
                lidar,
                chassis,
                control_cancel: control_cancel.clone(),
            })
            .map_err(|_| "设备已启动".to_string())?;
        info!("[Sim] 模拟设备启动（无硬件：决策链 + 底盘 + 传感器，动作经世界链路下发）");
        // ---- 7) 起控制口（**在 inner.set 之后**：注入的 Command 走到 device.reset() 时 inner 已就绪）
        //         无 `[control]` 段 → `unwrap_or_default()` → 默认开启（§七.3）
        crate::sim::control::Spawn(
            self.config.control.clone().unwrap_or_default(),
            self.robot.clone(),      // robot_cmd_tx / op_mode / robot_state
            world_for_control,       // `status` 的 assembled / avatar_id
            peer_id,
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
        // 停车优先（安全关键），再清意图
        if let Some(c) = d.chassis.as_ref() {
            c.Stop();
        }
        d.goal_service.lock().await.reset();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        info!("[Sim] reset（停车 + 清 goal + 清意图）");
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
        // 装配门闩①（必须最先）：未 Assembled → 写 Idle 直接 return，不寻路、不决策、不消费任务
        if let Some(w) = self.world.get() {
            if !w.Is_Assembled().await {
                *execute_state.write().await =
                    ExecuteState { state: DecisionState::Idle, sub_target: None };
                return;
            }
        }

        let d = self.inner.get().expect("device not started");
        // 门闩②（Step 4）：无底盘 → 保持 Idle（与真机 `enabled=false` / 无 stm32 语义一致）
        let Some(ch) = d.chassis.as_ref() else {
            *execute_state.write().await =
                ExecuteState { state: DecisionState::Idle, sub_target: None };
            return;
        };
        // 急停（Step 6，真机 `on_tick` 的第一步）：无 lidar → 不急停（真机 `match &d.lidar { None => false }`）
        // 第 5 参是传感器外参（Step 7 第 3 轮，偏差 D8）：只用于把回波换成障碍格，
        // 与建图同一个 `Sensor_Pose` + 同一个 `HIT_EPS_M`，保证两处标的格相同。
        let emergency = match d.lidar.as_ref() {
            Some(l) => {
                Check_Emergency_Stop(l.Get_Scan().await, rs, ch.as_ref(), &d.goal_service, l.Mount())
                    .await
            }
            None => false,
        };
        if emergency {
            // 急停：停车 + 清意图，**保留 goal** 供绕行（等价真机 `if emergency { … } else { … }` 的 if 分支）
            ch.Stop();
            *execute_state.write().await =
                ExecuteState { state: DecisionState::Idle, sub_target: None };
            return;
        }

        // 完整决策链：寻路 → 决策（与真车一致）
        let next_cell = d.goal_service.lock().await.get_path().await;
        let current = execute_state.read().await.clone();
        let result = d.executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
        *execute_state.write().await = ExecuteState {
            state: result.state,
            sub_target: result.sub_target,
        };
        // 执行动作：交给底盘翻译成世界 `motion_cmd`（`None` = 保持，不发明令，G4）
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
        if let Some(d) = self.inner.get() {
            d.sensors.iter().for_each(|s| s.Shutdown());
        }
        info!("[Sim] shutdown");
    }
}
