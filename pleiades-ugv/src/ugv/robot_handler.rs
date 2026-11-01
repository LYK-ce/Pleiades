//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-11-01

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
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use pleiades_base::event_bus::EventBus;
use pleiades_base::network::{NodeHandle, TOPIC_ROBOT_ASSIST_REQUEST};
use pleiades_base::robot::core::behavior_tree::{Frame, MotionOut, Node, Status};
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::mission_queue::{QueuedTask, TaskId};
use pleiades_base::robot::core::protocol::{
    encode_assist_request, encode_frame, COMPID_ROBOT, MSGID_ASSIST_REQUEST,
};
use pleiades_base::robot::core::robot::{DeviceHandler, Robot, TickOutcome, AUTO_TICK_MS};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::{DEFAULT_CRUISE_SPEED, DEFAULT_CRUISE_YAW_RATE, DEFAULT_PLAN_BUDGET_MS, UgvConfig};
use crate::device::camera_ugv::Ugv_Camera;
use crate::device::detector::{DetectorDevice, FrameSource};
use crate::device::lg290p::Lg290pDevice;
use crate::ugv::emergency_stop::check_emergency_stop;
use crate::ugv::executor::UgvVelocityExecutor;
use crate::ugv::lidar::LidarDevice;
use crate::ugv::mission::containment::AssistPublisher;
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
    /// 车载相机（enabled=false 时为 None）。同时是 detector 的帧源（task_44 Step 7）：
    /// `start()` 里 `Arc<dyn FrameSource> = cam.clone()`，帧来自 `latest_frame()`
    camera: Option<Arc<Ugv_Camera>>,
    /// detector（task_44 Step 7）：`enabled=false` / 无相机 / 权重缺失 → `None`
    detector: Option<Arc<DetectorDevice>>,
    /// 协助请求发布者（口径 #8：本 crate 自带一份；`None` = 树不发，只判条件）
    assist: Option<Arc<dyn AssistPublisher>>,
    /// 导航器（原目标服务；**不再持队列、不再出队**）
    nav: Arc<Mutex<Navigator>>,
    executor: UgvVelocityExecutor,
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
    /// 车载相机（bootstrap 已打开成功才为 Some；本 step 只持有，不消费）
    camera: Option<Arc<Ugv_Camera>>,
    inner: OnceLock<CarInner>,
}

impl CarDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: UgvConfig,
        node_handle: Arc<NodeHandle>,
        robot_bus: Arc<EventBus>,
        origin: (f32, f32, f32),
        camera: Option<Arc<Ugv_Camera>>,
    ) -> Self {
        Self { robot, config, origin, node_handle, robot_bus, camera, inner: OnceLock::new() }
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
        // task_44 §1.2 #11：执行器量级是定值，直接取 m/s / rad/s（不做百分比换算）
        let cruise_speed = chassis.and_then(|c| c.cruise_speed).unwrap_or(DEFAULT_CRUISE_SPEED);
        let cruise_yaw_rate = chassis.and_then(|c| c.cruise_yaw_rate).unwrap_or(DEFAULT_CRUISE_YAW_RATE);

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
            // task_44 §1.2 #9：本车是麦克纳姆 M1，缺省与回退都用 M1（不再是 X3Plus）
            let car_type = match chassis.and_then(|c| c.car_type.as_deref()).map(CarType::from_str) {
                Some(Some(t)) => t,
                Some(None) => { warn!("car_type 解析失败，回退 M1"); CarType::M1 }
                None => CarType::M1,
            };
            info!("启用底盘: port={serial_port} baud={baudrate} car={car_type:?} cruise={cruise_speed}m/s yaw={cruise_yaw_rate}rad/s");
            Some(Arc::new(STM32Device::spawn(&serial_port, baudrate, car_type, self.robot.robot_state.clone(), self.origin)?))
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
            None, // 建图增量：建好导航器后由下方订阅接上（`map_rx` 由它填）
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        );
        // 预算来自 config（不是硬编码；`Navigator::new` 的缺省只是兜底）
        nav.set_plan_budget(plan_budget);
        // task_44 Step 4：接上建图增量（`map_tx` 是 SLAM 的唯一发布口）。
        // **必须早于 `LidarDevice::spawn`** —— `broadcast` 只投给已存在的 receiver，
        // 晚订阅会让首批 delta 丢掉。
        nav.subscribe_map(&self.robot.map_tx);
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

        // detector（task_44 Step 7）：只读相机帧槽位 → 推理 → 覆盖自己的结果槽位。
        // **必须在 lidar / lg290p 之后**：那两段失败会 `return Err` 而不 `inner.set`，
        // detector 先起就会漏一个推理 task。
        // **失败一律 warn + None，绝不把 Err 透给 start()**（否则会打断整个机器人启动）。
        let detector = match (r.detector.as_ref(), self.camera.as_ref()) {
            (None, _) => {
                info!("[detector] 未配置 [detector] 段：不 spawn");
                None
            }
            (Some(cfg), _) if !cfg.Enabled() => {
                info!("[detector] enabled=false：不 spawn（零线程、不占 CPU）");
                None
            }
            (Some(_), None) => {
                warn!("[detector] 已启用但没有可用的 camera_ugv（未开 / 打开失败）：detector 不启动");
                None
            }
            (Some(cfg), Some(cam)) => {
                let source: Arc<dyn FrameSource> = cam.clone();
                match DetectorDevice::spawn(cfg.clone(), source, self.robot.robot_state.clone()).await
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

        // 协助请求发布者（task_44 §二 Step 7）：**无条件构造**（与 sim 同：`None` 只是「树不发」，
        // 不是「无网络」）。传输细节全在这里，`containment.rs` 仍不引 `tracing`。
        let assist: Arc<dyn AssistPublisher> = Arc::new(NodeHandlePublisher {
            node_handle: self.node_handle.clone(),
            local_peer_id: self.node_handle.Get_Local_Peer_Id().to_bytes(),
        });

        self.inner.set(CarInner {
            stm32,
            lidar,
            lg290p,
            camera: self.camera.clone(),
            detector,
            assist: Some(assist),
            nav,
            executor: UgvVelocityExecutor::new(cruise_speed, cruise_yaw_rate),
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
        let deps = TaskDeps {
            nav: d.nav.clone(),
            robot_state: self.robot.robot_state.clone(),
            detector: d.detector.clone(),
            assist: d.assist.clone(),
        };
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

        // ④ 决策（`ExecuteState` **唯一写点**）：叶子给的「下一格」+ 位姿 → (v, omega)
        //    连续执行器无状态，只写 `sub_target`、`state` 留 `Idle`
        //    （唯一的读点 `base/Robot/core/robot.rs` 只取 `sub_target`）
        let cmd = d.executor.decide(out.next_cell, rs.x, rs.y, rs.attitude.yaw);
        *execute_state.write().await = ExecuteState {
            state: DecisionState::Idle,
            sub_target: out.next_cell,
        };
        if let Err(e) = stm32.apply_velocity(cmd.v, cmd.omega) {
            warn!("[Car] 速度下发失败: {e}");
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
        // detector 最先停：推理从相机槽位取帧，先停推理 task 再关相机（task_44 Step 7）
        if let Some(det) = &d.detector {
            det.shutdown();
        }
        // 相机先关：独占 USB 的设备早一点释放，别等到进程退出（task_43）
        if let Some(c) = &d.camera {
            info!("正在关闭 camera_ugv...");
            c.Close();
        }
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

// ---------------------------------------------------------------- 限流日志（口径 #8：本 crate 自带一份）

/// 同 key 每 `period_ms` 最多一条（纯同步，不持锁跨 await）
///
/// 实现：`OnceLock<Mutex<HashMap<key, Instant>>>`；`Instant` 是进程内单调钟。
/// 与 `pleiades-sim/src/sim/sensors/mod.rs:44-59` 逐字相同（sim 靠那个模块的
/// `#![allow(non_snake_case)]`，本 crate 根没有该属性 → 按项放行）。
#[allow(non_snake_case)]
fn Should_Log(key: &'static str, period_ms: u64) -> bool {
    static LAST: OnceLock<StdMutex<HashMap<&'static str, Instant>>> = OnceLock::new();
    let table = LAST.get_or_init(|| StdMutex::new(HashMap::new()));
    // 限流表只是日志节流，锁中毒也不该拖垮收包 / 控制链：直接取回内层数据
    let mut g = match table.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    match g.get(key) {
        Some(last) if now.duration_since(*last).as_millis() < period_ms as u128 => false,
        _ => {
            g.insert(key, now);
            true
        }
    }
}

/// 限流 warning：`Should_Log` 通过才求值 `msg`（闭包内的 `format!` 不进热路径）
///
/// 与 `pleiades-sim/src/sim/sensors/mod.rs:61-65` 逐字相同（`#[allow(non_snake_case)]` 原因同上）。
#[allow(non_snake_case)]
fn Warn_Throttled(key: &'static str, period_ms: u64, msg: impl FnOnce() -> String) {
    if Should_Log(key, period_ms) {
        warn!("{}", msg());
    }
}

// ---------------------------------------------------------------- 协助请求（task_44 §二 Step 7）

/// 协助请求帧（**纯函数**，便于单测钉死字节契约）：msgid 7 + sysid=本车 peer_id + compid=COMPID_ROBOT
///
/// 与 `pleiades-sim/src/sim/sim_handler.rs:391-396` 逐字相同（口径 #8：不进 base）。
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
    /// 本机 PeerId 字节（= 帧的 `sysid`，地面站侧据此知道请求者）
    local_peer_id: Vec<u8>,
}

impl AssistPublisher for NodeHandlePublisher {
    fn publish(&self, x: f32, y: f32) {
        let frame = assist_frame(&self.local_peer_id, x, y);
        if let Err(e) = self.node_handle.Gossipsub_Publish_Try(TOPIC_ROBOT_ASSIST_REQUEST, frame) {
            // 失败只限流 warn（通道满 = 丢一帧，目标下次移动再发；不影响树）
            Warn_Throttled("ugv.assist", 1000, || {
                format!("[Ugv] 协助请求广播失败：{e}")
            });
        }
    }
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
        assert_eq!(dec.sysid, sysid, "sysid = 本机 PeerId（地面站据此认请求者）");
        assert_eq!(
            dec.payload,
            vec![0x3F, 0x80, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00],
            "x=1.0 / y=2.0 大端金标"
        );
    }
}