//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-29

//! 车设备处理器（Task 23 阶段 C：实现 base 的 DeviceHandler；Task 36 §12.4.7 改成树驱动）
//!
//! 统一契约：`start()` 里读配置 + spawn 车自己的设备（stm32 + lidar）。
//! `enabled=false` → 跳过该设备（统一跳过语义，节点照常跑；无运动设备时 on_tick 保持 Idle）。
//!
//! Task 36 起本文件只做**驱动**（§12.4.7 五步）：
//! ① 门闩 → ② 急停（保留 goal，这一拍树不 tick）→ ③ 树 tick（叶子写 `Frame.out.next_cell`）
//! → ④ 决策（`ExecuteState` **唯一写点**）→ ⑤ 终值（halt + 丢实例 + 返回 `Terminated`）。
//!
//! **出队只在 base `main_loop`**：本文件不碰 `MissionQueue`，也**不做出队**（§十一 #16）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use pleiades_base::event_bus::EventBus;
use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::behavior_tree::{Frame, MotionOut, Node, Status};
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::mission_queue::{QueuedTask, TaskId};
use pleiades_base::robot::core::robot::{DeviceHandler, Robot, TickOutcome, AUTO_TICK_MS};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::{DEFAULT_PLAN_BUDGET_MS, UgvConfig};
use crate::device::lg290p::Lg290pDevice;
use crate::ugv::emergency_stop::check_emergency_stop;
use crate::ugv::executor::DecisionExecutor;
use crate::ugv::lidar::LidarDevice;
use crate::ugv::mission::{self, TaskDeps};
use crate::ugv::nav::{Navigator, ObstacleMode, ARRIVAL_THRESHOLD_M};
use crate::ugv::slam::SlamContext;
use crate::ugv::stm32::STM32Device;
use crate::ugv::types::CarType;

/// 树实例表（§十一 #34）：懒创建、按 `TaskId` 存；`active` = 本拍要 tick 的那棵
#[derive(Default)]
struct TreeSet {
    trees: HashMap<TaskId, Box<dyn Node>>,
    /// 当前任务（`build_task` 写；`main_loop` 只认 `Terminated` 才出队）
    active: Option<TaskId>,
}

/// start() 后初始化的设备组
struct CarInner {
    /// 底盘（enabled=false 时为 None）
    stm32: Option<Arc<STM32Device>>,
    lidar: Option<LidarDevice>,
    /// RTK 流动站（enabled=false 时为 None）
    lg290p: Option<Lg290pDevice>,
    /// 导航器（原目标服务；**不再持队列、不再出队**）
    nav: Arc<Mutex<Navigator>>,
    executor: DecisionExecutor,
    trees: Mutex<TreeSet>,
    /// 拍号（`Frame.tick_no`）
    tick_no: AtomicU64,
}

pub struct CarDeviceHandler {
    robot: Arc<Robot>,
    config: UgvConfig,
    origin: (f32, f32, f32),
    node_handle: Arc<NodeHandle>,
    robot_bus: Arc<EventBus>,
    inner: OnceLock<CarInner>,
}

impl CarDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: UgvConfig,
        node_handle: Arc<NodeHandle>,
        robot_bus: Arc<EventBus>,
        origin: (f32, f32, f32),
    ) -> Self {
        Self { robot, config, origin, node_handle, robot_bus, inner: OnceLock::new() }
    }
}

#[async_trait]
impl DeviceHandler for CarDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let r = &self.config;

        // 底盘配置
        let chassis = r.chassis.as_ref();
        let chassis_enabled = chassis.and_then(|c| c.enabled).unwrap_or(true);

        // 雷达配置
        let lidar = r.lidar.as_ref();
        let lidar_enabled = lidar.and_then(|l| l.enabled).unwrap_or(true);
        let lidar_port = if !lidar_enabled { None } else {
            match lidar.and_then(|l| l.port.clone()) {
                Some(s) if !s.trim().is_empty() => Some(s),
                Some(_) => None,
                None => Some("/dev/rplidar".to_string()),
            }
        };
        let lidar_baudrate = lidar.and_then(|l| l.baudrate).or(Some(230400));

        let obstacle_inflation_radius = r.obstacle_inflation_radius.unwrap_or(0.2);

        // 规划时间预算（Task 39 §二）：config 顶层键 `plan_budget_ms`，缺省 20 ms
        let plan_budget = Duration::from_millis(r.plan_budget_ms.unwrap_or(DEFAULT_PLAN_BUDGET_MS));

        // 底盘：enabled=false → 跳过（统一跳过语义）
        let stm32 = if chassis_enabled {
            let serial_port = chassis.and_then(|c| c.port.clone()).unwrap_or_else(|| "/dev/myserial".to_string());
            let baudrate = chassis.and_then(|c| c.baudrate).unwrap_or(115200);
            let car_type = match chassis.and_then(|c| c.car_type.as_deref()).map(CarType::from_str) {
                Some(Some(t)) => t,
                Some(None) => { warn!("car_type 解析失败，回退 X3Plus"); CarType::X3Plus }
                None => CarType::X3Plus,
            };
            let forward_speed = chassis.and_then(|c| c.forward_speed).unwrap_or(30).clamp(0, 100);
            let turn_speed = chassis.and_then(|c| c.turn_speed).unwrap_or(10).clamp(0, 100);

            info!("启用底盘: port={serial_port} baud={baudrate} car={car_type:?}");
            Some(Arc::new(STM32Device::spawn(&serial_port, baudrate, car_type, self.robot.robot_state.clone(), self.origin, forward_speed, turn_speed)?))
        } else {
            info!("底盘未启用（enabled=false），跳过");
            None
        };

        // 导航器（无底盘时也创建；不持队列，出队在 main_loop）
        let mut nav = Navigator::new(
            self.robot.robot_state.clone(),
            self.robot.grid.clone(),
            self.robot.cluster_table.clone(),
            ObstacleMode::Respect,
            None, // 建图增量：真机不订阅（sim 才订，Step 6）
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        );
        // 预算来自 config（不是硬编码；`Navigator::new` 的缺省只是兜底）
        nav.set_plan_budget(plan_budget);
        let nav = Arc::new(Mutex::new(nav));

        // 雷达（含 SLAM）：enabled=false / 未配置 → 跳过
        let lidar = match (lidar_port, lidar_baudrate) {
            (Some(p), Some(b)) => {
                info!("启用 LiDAR: port={p}, baud={b}（含 SLAM 建图）");
                let slam_ctx = SlamContext {
                    grid: self.robot.grid.clone(),
                    robot_state: self.robot.robot_state.clone(),
                    cluster_table: self.robot.cluster_table.clone(),
                    map_tx: self.robot.map_tx.clone(),
                    node_handle: Some(self.node_handle.clone()),
                    local_peer_id: Some(self.node_handle.Get_Local_Peer_Id().to_bytes()),
                    obstacle_inflation_radius,
                };
                let d = match LidarDevice::spawn(&p, b, Some(slam_ctx)) {
                    Ok(d) => d,
                    Err(e) => {
                        // 雷达 spawn 失败：清理已启动的底盘，避免后台 task 泄漏（Task 23 review 修复）
                        if let Some(s) = &stm32 { s.shutdown(); }
                        return Err(e);
                    }
                };
                if let Err(e) = d.start_scan().await {
                    d.shutdown();
                    if let Some(s) = &stm32 { s.shutdown(); }
                    return Err(e);
                }
                Some(d)
            }
            _ => { info!("LiDAR 未配置，跳过"); None }
        };

        // RTK 流动站（LG290P）：enabled=false → 跳过（Task 24）
        let lg290p_cfg = r.lg290p.as_ref();
        let lg290p_enabled = lg290p_cfg.and_then(|l| l.enabled).unwrap_or(false);
        let lg290p = if lg290p_enabled {
            let port = lg290p_cfg
                .and_then(|l| l.port.clone())
                .unwrap_or_else(|| "/dev/ttyUSB2".to_string());
            let baudrate = lg290p_cfg.and_then(|l| l.baudrate).unwrap_or(460800);
            info!("启用 LG290P: port={port} baud={baudrate}");
            match Lg290pDevice::spawn(
                &port,
                baudrate,
                self.robot_bus.clone(),
                self.robot.robot_state.clone(),
                self.origin,
            ) {
                Ok(d) => Some(d),
                Err(e) => {
                    // spawn 失败：清理已启动的 stm32/lidar，避免后台 task 泄漏
                    if let Some(s) = &stm32 {
                        s.shutdown();
                    }
                    if let Some(l) = &lidar {
                        l.shutdown();
                    }
                    return Err(e);
                }
            }
        } else {
            info!("LG290P 未启用（enabled=false），跳过");
            None
        };

        self.inner.set(CarInner {
            stm32,
            lidar,
            lg290p,
            nav,
            executor: DecisionExecutor::new(),
            trees: Mutex::new(TreeSet::default()),
            tick_no: AtomicU64::new(0),
        })
            .map_err(|_| "设备已启动".to_string())?;
        Ok(())
    }

    async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        let d = self.inner.get().expect("device not started");
        match cmd {
            ManualCmd::StartLidarScan => {
                if let Some(l) = &d.lidar {
                    if let Err(e) = l.start_scan().await { warn!("[Robot] LiDAR 启动失败: {e}"); }
                }
            }
            ManualCmd::StopLidarScan => {
                if let Some(l) = &d.lidar {
                    if let Err(e) = l.stop_scan().await { warn!("[Robot] LiDAR 停止失败: {e}"); }
                }
            }
            _ => {
                if let Some(s) = &d.stm32 {
                    s.handle_manual_cmd(cmd);
                }
            }
        }
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        if let Some(s) = &d.stm32 { let _ = s.stop(); }
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
        // §十一 #38：停车优先（安全关键）→ 清 nav 目标 → 清意图；
        // **不 halt 树、不清实例表** —— 被抢占任务的节点进度必须原样留着（用例 5/6 卡这条）。
        if let Some(s) = &d.stm32 { let _ = s.stop(); }
        d.nav.lock().await.clear_goal();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        info!("[Car] suspend（停车 + 清 nav；树实例保留，等恢复）");
    }

    async fn build_task(&self, task: &QueuedTask) -> Result<(), String> {
        let d = self.inner.get().expect("device not started");
        let mut ts = d.trees.lock().await;
        // 幂等（§12.4.10 #6）：已有实例 → 只激活、不重建（抢占恢复靠这条，§十一 #36）
        if ts.trees.contains_key(&task.id) {
            ts.active = Some(task.id);
            info!("[Car] 任务 {:?} 已有树实例，只激活", task.id);
            return Ok(());
        }
        let deps = TaskDeps { nav: d.nav.clone(), robot_state: self.robot.robot_state.clone() };
        let tree = mission::build(&task.mission, &deps)?;
        ts.trees.insert(task.id, tree);
        ts.active = Some(task.id);
        info!("[Car] 任务 {:?} 建树完成（实例数 {}）", task.id, ts.trees.len());
        Ok(())
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) -> TickOutcome {
        let d = self.inner.get().expect("device not started");

        // ① 门闩：无底盘 → 无运动设备，保持 Idle
        let Some(stm32) = &d.stm32 else {
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
            return TickOutcome::Running;
        };

        // ② 急停：命中 → 停车 + 写 Idle，**保留 goal**；树这一拍不 tick（时间冻结，§12.4.7 ②）
        let emergency = match &d.lidar {
            Some(l) => check_emergency_stop(l, rs, stm32, &d.nav).await,
            None => false,
        };
        if emergency {
            let _ = stm32.stop();
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        }

        // ③ 树：推进当前任务的树（急停这一拍由 tick_active_tree 挡住）
        let mut out = MotionOut::default();
        let tick_no = d.tick_no.fetch_add(1, Ordering::Relaxed);
        let mut frame = Frame { dt_ms: AUTO_TICK_MS as u32, tick_no, out: &mut out };
        let status = {
            let mut ts = d.trees.lock().await;
            tick_active_tree(&mut ts, &mut frame, emergency).await
        };
        if emergency {
            return TickOutcome::Running; // ④⑤ 跳过：本拍不决策、不判终值
        }

        // ④ 决策：叶子给的「下一格」→ ExecuteState（**唯一写点**）+ 动作
        let current = execute_state.read().await.clone();
        let result = d.executor.decide(out.next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
        *execute_state.write().await = ExecuteState {
            state: result.state,
            sub_target: result.sub_target,
        };
        if let Some(action) = result.action {
            stm32.apply_action(action);
        }

        // ⑤ 终值：halt + 丢实例 → 交给 base 出队（唯一出队口，§十一 #16）
        let Some((id, st)) = status else { return TickOutcome::Running; };
        let code = match st {
            Status::Running => return TickOutcome::Running,
            Status::Success => None,
            Status::Failure(c) => {
                warn!("[Car] 任务 {:?} 失败，原因码 {}", id, c.code());
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
            if let Some(s) = &d.stm32 { let _ = s.stop(); }
        }
    }

    async fn shutdown(&self) {
        let Some(d) = self.inner.get() else { return; };
        if let Some(l) = &d.lidar {
            info!("正在停止 LiDAR...");
            let _ = l.stop_scan().await;
            l.shutdown();
        }
        if let Some(s) = &d.stm32 { s.shutdown(); }
        if let Some(g) = &d.lg290p { g.shutdown(); }
    }
}

/// ③ 段的树推进：`emergency == true` 时**不 tick**（§12.4.7 ② / §12.4.10 #5）
///
/// 抽成函数是为了让「急停不进树」可单测（传 `emergency` 即刻返回），而不是只靠读 if。
async fn tick_active_tree(
    ts: &mut TreeSet,
    frame: &mut Frame<'_>,
    emergency: bool,
) -> Option<(TaskId, Status)> {
    if emergency {
        return None;
    }
    let id = ts.active?;
    let tree = ts.trees.get_mut(&id)?;
    Some((id, tree.tick(frame).await))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    use async_trait::async_trait;

    /// 只记 tick 次数、恒返回 `Running` 的节点
    struct CountingNode(Arc<AtomicU32>);

    #[async_trait]
    impl Node for CountingNode {
        async fn tick(&mut self, _f: &mut Frame<'_>) -> Status {
            self.0.fetch_add(1, Ordering::SeqCst);
            Status::Running
        }
        fn halt(&mut self) {}
    }

    /// §12.4.10 #5：急停那一拍树 **0 次** tick；非急停拍 1 次且返回 active 的 id
    #[tokio::test]
    async fn test_emergency_does_not_tick_tree() {
        let ticks = Arc::new(AtomicU32::new(0));
        let mut ts = TreeSet::default();
        ts.trees.insert(TaskId(7), Box::new(CountingNode(ticks.clone())));
        ts.active = Some(TaskId(7));

        let mut out = MotionOut::default();
        let mut frame = Frame { dt_ms: AUTO_TICK_MS as u32, tick_no: 0, out: &mut out };

        // 急停拍：不 tick
        assert!(tick_active_tree(&mut ts, &mut frame, true).await.is_none());
        assert_eq!(ticks.load(Ordering::SeqCst), 0, "急停拍不得 tick 树");

        // 非急停拍：tick 1 次，返回 active 的 id
        let got = tick_active_tree(&mut ts, &mut frame, false).await;
        assert_eq!(got, Some((TaskId(7), Status::Running)));
        assert_eq!(ticks.load(Ordering::SeqCst), 1);

        // 无 active：不 tick、返回 None
        ts.active = None;
        assert!(tick_active_tree(&mut ts, &mut frame, false).await.is_none());
        assert_eq!(ticks.load(Ordering::SeqCst), 1);
    }
}