//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-29

//! 机设备处理器（Task 23 阶段 C：实现 base 的 DeviceHandler；Task 36 §12.5 改成树驱动）
//!
//! 统一契约：`start()` 里读配置 + spawn 机自己的设备（mavlink + 导航器 + 决策器）。
//! `enabled=false` → 跳过飞控（统一跳过语义，节点照常跑；无运动设备时 on_tick 保持 Idle）。
//!
//! 与 `pleiades-ugv` 的 `CarDeviceHandler` **同构**（§12.5.4），差别只有两处、
//! 且都落在**参数/步骤取舍**上，不 fork 逻辑：
//! 1. **无第 ② 步 2D 急停**（机无 LiDAR，§12.5.2）→ ① 门闩 → ② 树 → ③ 决策 → ④ 终值；
//! 2. 导航器用 `ObstacleMode::Ignore` + 建图增量订阅 `None`（§12.5.2）。
//!
//! **出队只在 base `main_loop`**：本文件不碰 `MissionQueue`，也**不做出队**（§十一 #16）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::behavior_tree::{Frame, MotionOut, Node, Status};
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::mission_queue::{QueuedTask, TaskId};
use pleiades_base::robot::core::robot::{DeviceHandler, Robot, TickOutcome, AUTO_TICK_MS};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::{DEFAULT_PLAN_BUDGET_MS, UavConfig};
use crate::device::camera_uav::Uav_Camera;
use crate::uav::executor::DecisionExecutor;
use crate::uav::mavlink::{MavlinkDevice, Telemetry};
use crate::uav::mission::{self, TaskDeps};
use crate::uav::nav::{Navigator, ObstacleMode, ARRIVAL_THRESHOLD_M};

// 设备目录新增 `device/detector/`（task_37 Step 2），但本 step 的允许清单里没有
// `pleiades-uav/src/device/mod.rs`（`tmp/task37/step.md:20-30`），故从本文件声明
// （与 `sim_handler.rs:51-58` 声明 `device_cap` 同款解法）。`#[path]` 相对**本文件所在目录**
// （`src/uav/`）解析 → `src/device/detector/mod.rs`。
// 规范路径因此是 `crate::uav::robot_handler::detector`。
#[path = "../device/detector/mod.rs"]
pub mod detector;

/// 树实例表（§十一 #34）：懒创建、按 `TaskId` 存；`active` = 本拍要 tick 的那棵
#[derive(Default)]
struct TreeSet {
    trees: HashMap<TaskId, Box<dyn Node>>,
    /// 当前任务（`build_task` 写；`main_loop` 只认 `Terminated` 才出队）
    active: Option<TaskId>,
}

/// start() 后初始化的设备组
struct UavInner {
    /// 飞控（enabled=false 时为 None）
    mavlink: Option<Arc<MavlinkDevice>>,
    /// 导航器（原目标服务；**不再持队列、不再出队**）
    nav: Arc<Mutex<Navigator>>,
    executor: DecisionExecutor,
    trees: Mutex<TreeSet>,
    /// 拍号（`Frame.tick_no`）
    tick_no: AtomicU64,
    /// detector 设备（task_37 Step 2）：`enabled=false` / 无相机 / 权重缺失 → `None`
    /// **只被 `shutdown()` 用**，决策链一行不碰它
    detector: Option<Arc<detector::DetectorDevice>>,
}

pub struct UavDeviceHandler {
    robot: Arc<Robot>,
    config: UavConfig,
    /// Task 36：`Navigator::new` 不再要本车 peer_id，本字段暂无人读（`bootstrap` 仍照传）
    #[allow(dead_code)]
    node_handle: Arc<NodeHandle>,
    origin: (f32, f32, f32),
    /// 机载相机（task_37 Step 2）：`bootstrap` 在 push `device_caps` 之前留的那一份；
    /// `None` = 相机没开 / 打开失败 / 同 index 被 camera_pc 占了
    camera: Option<Arc<Uav_Camera>>,
    inner: OnceLock<UavInner>,
}

impl UavDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: UavConfig,
        node_handle: Arc<NodeHandle>,
        origin: (f32, f32, f32),
        camera: Option<Arc<Uav_Camera>>,
    ) -> Self {
        Self { robot, config, node_handle, origin, camera, inner: OnceLock::new() }
    }
}

#[async_trait]
impl DeviceHandler for UavDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let flight_ctrl = self.config.flight_ctrl.as_ref();
        let flight_ctrl_enabled = flight_ctrl.and_then(|f| f.enabled).unwrap_or(false);
        let obstacle_inflation_radius = self.config.obstacle_inflation_radius.unwrap_or(0.2);

        // 规划时间预算（Task 39 §二）：config 顶层键 `plan_budget_ms`，缺省 20 ms
        let plan_budget =
            Duration::from_millis(self.config.plan_budget_ms.unwrap_or(DEFAULT_PLAN_BUDGET_MS));

        // 飞控：enabled=false → 跳过（统一跳过语义）
        let mavlink = if flight_ctrl_enabled {
            let port = match flight_ctrl.and_then(|f| f.connection.clone()) {
                Some(s) if !s.trim().is_empty() => Some(s),
                _ => None,
            };
            let Some(port) = port else {
                return Err("flight_ctrl.enabled=true 但 connection 未配置".to_string());
            };
            let baudrate = flight_ctrl.and_then(|f| f.baudrate).unwrap_or(921600);
            let vel_fwd = flight_ctrl.and_then(|f| f.vel_fwd).unwrap_or(0.3).max(0.0);
            let yaw_rate_deg = flight_ctrl.and_then(|f| f.yaw_rate_deg).unwrap_or(15.0).max(0.0);

            info!("启用飞控: port={port}, baud={baudrate}, infl_r={obstacle_inflation_radius}");
            let flight_state = Arc::new(RwLock::new(Telemetry::default()));
            Some(Arc::new(MavlinkDevice::spawn(
                &port,
                baudrate,
                flight_state,
                Some(self.robot.robot_state.clone()),
                self.origin,
                vel_fwd,
                yaw_rate_deg,
            )?))
        } else {
            info!("飞控未启用（enabled=false），跳过");
            None
        };

        // 导航器（无飞控时也创建；不持队列，出队在 main_loop）
        //
        // 机端差异全部走构造参数（§12.5.2 / §12.5.4，**不 fork 代码**）：
        // - `ObstacleMode::Ignore`：喂空栅格、不注入他车动态障碍（Task 27「天上没人」）；
        // - 建图增量订阅 `None`：机无 SLAM（`slam/` 只在 ugv）。
        // ⚠️ 过渡参数：机的导航模式后续会改版，届时整体替换（不要当最终形态）。
        let mut nav = Navigator::new(
            self.robot.robot_state.clone(),
            self.robot.grid.clone(),
            self.robot.cluster_table.clone(),
            ObstacleMode::Ignore,
            None,
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        );
        // 预算来自 config（不是硬编码；`Navigator::new` 的缺省只是兜底）
        nav.set_plan_budget(plan_budget);
        let nav = Arc::new(Mutex::new(nav));

        // ③ detector（task_37 Step 2）：只读相机帧槽位 → 推理 → 覆盖自己的结果槽位。
        //    **失败一律 warn + None，绝不把 Err 透给 start()**（否则会打断整个机器人启动）。
        let detector = match (self.config.detector.as_ref(), self.camera.as_ref()) {
            (None, _) => {
                info!("[detector] 未配置 [detector] 段：不 spawn");
                None
            }
            (Some(cfg), _) if !cfg.Enabled() => {
                info!("[detector] enabled=false：不 spawn（零线程、不占 CPU）");
                None
            }
            (Some(_), None) => {
                warn!("[detector] 已启用但没有可用的 camera_uav（未开 / 打开失败）：detector 不启动");
                None
            }
            (Some(cfg), Some(cam)) => {
                let source: Arc<dyn detector::FrameSource> = cam.clone();
                match detector::DetectorDevice::spawn(
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
                        warn!("[detector] 未启动（只 warn，不影响其它设备）：{e}");
                        None
                    }
                }
            }
        };

        self.inner.set(UavInner {
            mavlink,
            nav,
            executor: DecisionExecutor::new(),
            trees: Mutex::new(TreeSet::default()),
            tick_no: AtomicU64::new(0),
            detector,
        })
            .map_err(|_| "设备已启动".to_string())?;
        Ok(())
    }

    async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        let d = self.inner.get().expect("device not started");
        if let Some(m) = &d.mavlink {
            m.handle_manual_cmd(cmd);
        }
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        if let Some(m) = &d.mavlink { let _ = m.stop(); }
        // 丢实例（§十一 #38：replace / 切模式 → 清实例表；抢占走 suspend，Step 8 引入）
        {
            let mut ts = d.trees.lock().await;
            for tree in ts.trees.values_mut() { tree.halt(); }
            ts.trees.clear();
            ts.active = None;
        }
        d.nav.lock().await.clear_goal();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    }

    async fn suspend(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        // §十一 #38：与 CarDeviceHandler::suspend 同构（机无 2D 急停，其余一致）
        if let Some(m) = &d.mavlink { let _ = m.stop(); }
        d.nav.lock().await.clear_goal();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        info!("[Uav] suspend（停车 + 清 nav；树实例保留，等恢复）");
    }

    async fn build_task(&self, task: &QueuedTask) -> Result<(), String> {
        let d = self.inner.get().expect("device not started");
        let mut ts = d.trees.lock().await;
        // 幂等（§12.4.10 #6）：已有实例 → 只激活、不重建（抢占恢复靠这条，§十一 #36）
        if ts.trees.contains_key(&task.id) {
            ts.active = Some(task.id);
            info!("[Uav] 任务 {:?} 已有树实例，只激活", task.id);
            return Ok(());
        }
        let deps = TaskDeps { nav: d.nav.clone(), robot_state: self.robot.robot_state.clone() };
        let tree = mission::build(&task.mission, &deps)?;
        ts.trees.insert(task.id, tree);
        ts.active = Some(task.id);
        info!("[Uav] 任务 {:?} 建树完成（实例数 {}）", task.id, ts.trees.len());
        Ok(())
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) -> TickOutcome {
        let d = self.inner.get().expect("device not started");

        // ① 门闩：无飞控 → 无运动设备，保持 Idle（节点照常广播位姿/遥测）
        let Some(mavlink) = &d.mavlink else {
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
            return TickOutcome::Running;
        };

        // ② 树：推进当前任务的树（机无 2D 急停 → 没有「急停拍不 tick」这一步，§12.5.2）
        let mut out = MotionOut::default();
        let tick_no = d.tick_no.fetch_add(1, Ordering::Relaxed);
        let mut frame = Frame { dt_ms: AUTO_TICK_MS as u32, tick_no, out: &mut out };
        let status = {
            let mut ts = d.trees.lock().await;
            tick_active_tree(&mut ts, &mut frame).await
        };

        // ③ 决策：叶子给的「下一格」→ ExecuteState（**唯一写点**）+ 动作
        let current = execute_state.read().await.clone();
        let result = d.executor.decide(out.next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
        *execute_state.write().await = ExecuteState {
            state: result.state,
            sub_target: result.sub_target,
        };
        if let Some(action) = result.action {
            mavlink.apply_action(action);
        }

        // ④ 终值：halt + 丢实例 → 交给 base 出队（唯一出队口，§十一 #16）
        let Some((id, st)) = status else { return TickOutcome::Running; };
        let code = match st {
            Status::Running => return TickOutcome::Running,
            Status::Success => None,
            Status::Failure(c) => {
                warn!("[Uav] 任务 {:?} 失败，原因码 {}", id, c.code());
                Some(c)
            }
        };
        {
            let mut ts = d.trees.lock().await;
            if let Some(mut tree) = ts.trees.remove(&id) { tree.halt(); }
            if ts.active == Some(id) { ts.active = None; }
        }
        d.nav.lock().await.clear_goal();
        TickOutcome::Terminated(code)
    }

    fn stop(&self) {
        if let Some(d) = self.inner.get() {
            if let Some(m) = &d.mavlink { let _ = m.stop(); }
        }
    }

    async fn shutdown(&self) {
        if let Some(d) = self.inner.get() {
            if let Some(det) = &d.detector {
                det.shutdown(); // 先停推理 task（它与飞控无关，同步 cancel）
            }
            if let Some(m) = &d.mavlink { m.shutdown(); }
        }
    }
}

/// ② 段的树推进：无 active 就跳过（机端没有急停，故不需要 `emergency` 开关）
async fn tick_active_tree(ts: &mut TreeSet, frame: &mut Frame<'_>) -> Option<(TaskId, Status)> {
    let id = ts.active?;
    let tree = ts.trees.get_mut(&id)?;
    Some((id, tree.tick(frame).await))
}
