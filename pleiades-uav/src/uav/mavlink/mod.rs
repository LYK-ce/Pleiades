//Presented by KeJi
//Created Date ： 2026-08-25
//Modified Date ： 2026-11-01

//! MAVLink 飞控设备驱动（Task 22_4）
//!
//! `MavlinkDevice` — 封装 MAVLink 协议 + 串口 TX/RX（Task 23 阶段 C：实现 base 的 `DeviceHandler`）。
//!
//! 自闭环：上层只发**连续速度 `(v, omega)`**（自动与手动同一条出口 `apply_velocity`），设备内部用
//! 「期望速度 + 10Hz 保持 loop」持续下发速度指令（GUIDED 速度指令约 2~3s 无新指令即回落悬停）。

pub mod constants;
pub mod protocol;
pub mod types;

pub use types::Telemetry;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use constants::*;
use pleiades_base::robot::util::serial::port::spawn_port;
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::state::RobotState;

// ============================================================
// MavlinkDevice — 控制句柄
// ============================================================

pub struct MavlinkDevice {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    /// 期望速度（std 锁：trait 方法同步、不可 await；临界区纳秒级）
    desired: Arc<Mutex<DesiredVelocity>>,
    state: Arc<RwLock<Telemetry>>,
    /// 本机（地面站/机载计算机）身份
    system_id: u8,
    component_id: u8,
    /// 姿态流建立时记录的 yaw 偏移（首帧 ATTITUDE 到达时记录，开机机头朝 = 0；None = 尚未记录）
    yaw_offset: Arc<Mutex<Option<f32>>>,
    /// 一次性机动（takeoff/land）进行中：抑制保持 loop 发速度指令，避免零速度打断爬升/降落（Task 22_5 修复）
    takeoff_pending: Arc<AtomicBool>,
    cancel: CancellationToken,
}

/// 期望速度（std 锁临界区纳秒级）：`v` **m/s**、`omega` **rad/s**（`omega > 0` = 左转）
///
/// `(0, 0)` = 停 / 悬停 —— 也是本设备构造时的初值（与旧实现的离散「停」动作同义）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct DesiredVelocity {
    v: f32,
    omega: f32,
}

/// 连续速度 → MAVLink 速度四元组（`vx`/`vy`/`vz` = 机体系 NED m/s，`yaw_rate` = **rad/s**）
///
/// - `vx = v`：执行器给的就是 m/s（task_44 §1.3 真机一行），机体系前向即 NED x，不做换算；
/// - `vy = 0`、`vz = 0`：无横移、`VZ_IGNORE` 锁高度（与改动前一致）；
/// - `yaw_rate = -omega`：MAVLink 的 `yaw_rate` 在 NED 下**正 = 顺时针/右转**，而口径
///   task_44 §1.2 #2 是 **`omega > 0` = 逆时针/左转**，故取反。证据链与改动前的离散表一致
///   （旧 `TurnLeft → -yaw_rate`、`TurnRight → +yaw_rate`）；单测 `test_velocity_to_mavlink_sign` 锁。
///
/// 前进与偏航**同帧下发**：两者在同一个 `SET_POSITION_TARGET_LOCAL_NED` 里（`type_mask` 只
/// ignore 位置/加速度/`yaw`，不 ignore `yaw_rate`，见 `protocol::velocity_msg`）——
/// 旧实现只是把翻译表拆成互斥 5 行，通道本身从来不是互斥的。
fn velocity_to_mavlink(v: f32, omega: f32) -> (f32, f32, f32, f32) {
    (v, 0.0, 0.0, -omega)
}

/// 手动命令 → 连续速度：只有 `Motion` / `Stop` 走速度出口（其余 = `None`，交给一次性命令）
///
/// 单独提成纯函数是为了能在没有串口的单测里断言「`Motion` 路径」（Step 5 实施步骤 5）。
fn manual_cmd_velocity(cmd: &ManualCmd) -> Option<(f32, f32)> {
    match cmd {
        // §1.2 #6：两轴连续量，`(0, 0)` 由地面站松开全部键时发；设备端不再做超时/心跳
        ManualCmd::Motion { v, omega } => Some((*v, *omega)),
        // §1.2 #1：协议里已无 stop 动作，`Stop` 是内部命令 → 设备端解释为 `(0, 0)`
        ManualCmd::Stop => Some((0.0, 0.0)),
        _ => None,
    }
}

/// 坐标对齐：飞控 NED local 坐标 + yaw → 世界坐标（origin 平移 + yaw_offset 旋转）
fn aligned_world_pose(
    local_x: f32,
    local_y: f32,
    yaw: f32,
    offset: f32,
    origin: (f32, f32),
) -> (f32, f32, f32) {
    let wx = origin.0 + local_x * offset.cos() + local_y * offset.sin();
    let wy = origin.1 - local_x * offset.sin() + local_y * offset.cos();
    // yaw 与位置矩阵 R(-offset) 自洽：世界系 +y=南（y 向下），航向「东→南」为顺时针正，
    // 与 NED yaw（北→东顺时针正）同约定；前进方向经位置变换后 = (cos yaw_world, sin yaw_world)。
    // 无需取反（早期「转向符号反」是误判：误把世界系当 y 向上）。
    let yaw_world = yaw - offset;
    (wx, wy, yaw_world)
}

/// 重发 set_mode(GUIDED)（首个 HEARTBEAT 到达后调用，兜底飞控启动时序）
fn resend_set_mode(tx: &mpsc::Sender<Vec<u8>>, system_id: u8, component_id: u8) {
    let msg = protocol::set_mode_msg(FC_SYSTEM_ID, FC_COMPONENT_ID, MODE_GUIDED);
    let bytes = protocol::serialize_message(&msg, system_id, component_id);
    if let Err(e) = tx.try_send(bytes) {
        warn!("[Mavlink] 重发 set_mode 失败: {e}");
    } else {
        info!("[Mavlink] 收到首个 HEARTBEAT，重发 set_mode(GUIDED)");
    }
}

impl MavlinkDevice {
    /// 启动设备：打开串口、挂 RX 解析回调、起 10Hz 保持 loop、发初始化命令。
    ///
    /// 初始化序列（同步 try_send，由 TX loop 异步写出）：
    /// `send_heartbeat` → `set_mode_send(GUIDED)` → `request_telemetry_streams`；
    /// 收到首个 HEARTBEAT 后重发一次 `set_mode_send(GUIDED)`（兜底飞控启动时序）。
    pub fn spawn(
        port: &str,
        baudrate: u32,
        state: Arc<RwLock<Telemetry>>,
        robot_state: Option<Arc<RwLock<RobotState>>>,
        origin: (f32, f32, f32),
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let robot_state_clone = robot_state.clone();
        let desired = Arc::new(Mutex::new(DesiredVelocity::default()));
        let desired_clone = desired.clone();
        let yaw_offset = Arc::new(Mutex::new(None::<f32>));
        let yaw_offset_clone = yaw_offset.clone();
        let takeoff_pending = Arc::new(AtomicBool::new(false));
        let takeoff_pending_clone = takeoff_pending.clone();
        let system_id = GCS_SYSTEM_ID;
        let component_id = GCS_COMPONENT_ID;
        // RX 回调里「首个 HEARTBEAT 后重发 set_mode」需要访问 TX channel（spawn_port 返回后才就绪）
        let cmd_tx_shared: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(None));
        let cmd_tx_for_cb = cmd_tx_shared.clone();
        let set_mode_resent = Arc::new(AtomicBool::new(false));
        let set_mode_resent_cb = set_mode_resent.clone();

        let mut local = Telemetry::default();
        let mut rx_buf: Vec<u8> = Vec::new();

        let serial_cmd_tx = spawn_port(
            port,
            baudrate,
            512,
            move |bytes| {
                rx_buf.extend_from_slice(bytes);
                let was_armed = local.armed;
                let mut updated = false;
                let hb_before = local.heartbeat_count;
                while let Some((header, msg)) = protocol::parse_one_frame(&mut rx_buf) {
                    protocol::handle_message(&mut local, header, msg);
                    updated = true;
                }
                // 首个 HEARTBEAT 到达：重发一次 set_mode(GUIDED)（串口刚开时发的可能被飞控忽略）
                if hb_before == 0 && local.heartbeat_count > 0 {
                    if let Some(tx) = cmd_tx_for_cb.lock().unwrap().as_ref() {
                        resend_set_mode(tx, system_id, component_id);
                        set_mode_resent_cb.store(true, Ordering::SeqCst);
                    }
                }
                // 解锁跳变诊断：armed false→true 时若姿态流尚未建立，说明遥测异常
                if !was_armed && local.armed {
                    if local.attitude_count == 0 {
                        warn!("[Mavlink] 解锁时姿态流尚未建立，遥测链路可能异常");
                    }
                }
                // 姿态流第一次建立时记录 yaw_offset（开机机头朝 = 0，与车一致；解锁不再跳变）。
                // 覆盖「无姿态流 → yaw_offset 永不记录 → 坐标静默按机头朝北」的退化。
                if local.attitude_count > 0 {
                    if let Ok(mut g) = yaw_offset_clone.lock() {
                        if g.is_none() {
                            let off = local.yaw;
                            *g = Some(off);
                            info!("[Mavlink] 记录 yaw_offset = {off}");
                        }
                    }
                }
                if updated {
                    if let Ok(mut guard) = state_clone.try_write() {
                        *guard = local.clone(); // 单一写入者不变式：全量覆盖
                    } else {
                        warn!("[Mavlink] try_write 失败，状态更新丢弃");
                    }
                    // 坐标对齐后写入 RobotState（机设备写 z = 飞控 EKF 高度）
                    if let Some(rs) = &robot_state_clone {
                        let offset = (*yaw_offset_clone.lock().unwrap()).unwrap_or(0.0);
                        let (wx, wy, yaw_world) = aligned_world_pose(
                            local.local_x,
                            local.local_y,
                            local.yaw,
                            offset,
                            (origin.0, origin.1),
                        );
                        if let Ok(mut g) = rs.try_write() {
                            g.x = wx;
                            g.y = wy;
                            g.z = local.relative_alt;
                            g.attitude.roll = local.roll;
                            g.attitude.pitch = local.pitch;
                            g.attitude.yaw = yaw_world;
                            // Task 22_5 修复：速度与位置同坐标系，vx/vy 也做 yaw_offset 旋转（与 aligned_world_pose 一致）
                            g.vx = local.local_vx * offset.cos() + local.local_vy * offset.sin();
                            g.vy = -local.local_vx * offset.sin() + local.local_vy * offset.cos();
                            // Task 22_5 修复：vz 统一向上为正（LOCAL_POSITION_NED 的 vz 向下为正，取反）
                            g.vz = -local.local_vz;
                            g.battery = local.battery_voltage;
                        }
                    }
                }
            },
            cancel.clone(),
        )?;
        *cmd_tx_shared.lock().unwrap() = Some(serial_cmd_tx.clone());
        // 兜底：首个 HEARTBEAT 若在 sender 回填前到达（RX 回调漏发），这里补发一次
        if !set_mode_resent.load(Ordering::SeqCst)
            && state.try_read().map(|s| s.heartbeat_count > 0).unwrap_or(false)
        {
            resend_set_mode(&serial_cmd_tx, system_id, component_id);
            set_mode_resent.store(true, Ordering::SeqCst);
        }

        // 启动初始化命令：HEARTBEAT → 自动切 GUIDED → 请求遥测流
        let init_msgs = [
            protocol::heartbeat_msg(),
            protocol::set_mode_msg(FC_SYSTEM_ID, FC_COMPONENT_ID, MODE_GUIDED),
        ];
        for msg in &init_msgs {
            let bytes = protocol::serialize_message(msg, system_id, component_id);
            if let Err(e) = serial_cmd_tx.try_send(bytes) {
                warn!("[Mavlink] 初始化命令发送失败: {e}");
            }
        }
        for msg in protocol::telemetry_stream_msgs(FC_SYSTEM_ID, FC_COMPONENT_ID) {
            let bytes = protocol::serialize_message(&msg, system_id, component_id);
            let _ = serial_cmd_tx.try_send(bytes);
        }

        // 保持 loop（10Hz）：读期望速度 → 翻译 → 持续下发速度指令
        let keep_tx = serial_cmd_tx.clone();
        let keep_desired = desired_clone.clone();
        let keep_cancel = cancel.clone();
        let keep_takeoff_pending = takeoff_pending_clone.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Task 22_5 修复：起飞/降落期间闭嘴，避免零速度打断爬升/降落
                        if keep_takeoff_pending.load(Ordering::SeqCst) {
                            continue;
                        }
                        let cmd = *keep_desired.lock().unwrap();
                        let (vx, vy, vz, yaw_rate) = velocity_to_mavlink(cmd.v, cmd.omega);
                        let msg = protocol::velocity_msg(FC_SYSTEM_ID, FC_COMPONENT_ID, vx, vy, vz, yaw_rate);
                        let bytes = protocol::serialize_message(&msg, system_id, component_id);
                        if let Err(e) = keep_tx.try_send(bytes) {
                            warn!("[Mavlink] 保持 loop 下发失败: {e}"); // 失败容忍，下一 tick 补发
                        }
                    }
                    _ = keep_cancel.cancelled() => break,
                }
            }
        });

        Ok(Self {
            serial_cmd_tx,
            desired,
            state,
            system_id,
            component_id,
            yaw_offset,
            takeoff_pending,
            cancel,
        })
    }

    /// 序列化并 try_send 一条 MAVLink 消息。
    fn send_message(&self, msg: &mavlink::dialects::ardupilotmega::MavMessage) -> Result<(), String> {
        let bytes = protocol::serialize_message(msg, self.system_id, self.component_id);
        self.serial_cmd_tx
            .try_send(bytes)
            .map_err(|e| format!("TX 通道满: {e}"))
    }

    /// 一次性命令：切模式（spawn 已自动切 GUIDED，此方法保留备用）。
    pub fn set_mode_send(&self, mode: u32) -> Result<(), String> {
        self.send_message(&protocol::set_mode_msg(FC_SYSTEM_ID, FC_COMPONENT_ID, mode))
    }

    /// 一次性命令：起飞（固定 1.8m，纯触发，不带确认回读）。
    pub fn takeoff_send(&self) -> Result<(), String> {
        // 起飞前确保飞控在 GUIDED（遥控器模式开关边沿触发，拨挡位即可强制接管）
        if let Err(e) = self.set_mode_send(MODE_GUIDED) {
            warn!("[Mavlink] takeoff 前 set_mode(GUIDED) 失败: {e}");
        }
        // Task 22_5 修复：抑制保持 loop，避免爬升期间被零速度指令打断
        self.takeoff_pending.store(true, Ordering::SeqCst);
        self.send_message(&protocol::takeoff_msg(FC_SYSTEM_ID, FC_COMPONENT_ID))
    }

    /// 一次性命令：降落（纯触发，不带确认回读）。
    pub fn land_send(&self) -> Result<(), String> {
        // 降落前确保飞控在 GUIDED（遥控器模式开关边沿触发，拨挡位即可强制接管）
        if let Err(e) = self.set_mode_send(MODE_GUIDED) {
            warn!("[Mavlink] land 前 set_mode(GUIDED) 失败: {e}");
        }
        // Task 22_5 修复：抑制保持 loop，避免降落期间被速度指令干扰
        self.takeoff_pending.store(true, Ordering::SeqCst);
        self.send_message(&protocol::land_msg(FC_SYSTEM_ID, FC_COMPONENT_ID))
    }

    /// 读取当前遥测快照。
    pub async fn get_state(&self) -> Telemetry {
        self.state.read().await.clone()
    }

    /// 读取 yaw_offset（解锁时刻记录的朝向偏移）。
    pub fn get_yaw_offset(&self) -> f32 {
        (*self.yaw_offset.lock().unwrap()).unwrap_or(0.0)
    }

    // ─── 命令解析（A6：下沉到设备，设备选择性响应）────────
    pub fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        // 手动与自动**同一条出口**（§1.3：手动 = 同一张表，不再有离散分支）
        if let Some((v, omega)) = manual_cmd_velocity(cmd) {
            self.apply_velocity(v, omega);
            return;
        }
        match cmd {
            ManualCmd::Takeoff => { let _ = self.takeoff_send(); }
            ManualCmd::Land => { let _ = self.land_send(); }
            _ => {} // 机不支持 Beep/LiDAR 命令，忽略
        }
    }

    /// 连续速度出口（自动 `on_tick` 与手动共用）：`v` m/s、`omega` rad/s（`omega > 0` = 左转）
    ///
    /// 只在**非零**速度时解除 `takeoff_pending`（起飞/降落抑制保持 loop 的开关），再写 `desired`。
    /// 真正的发送由 10 Hz 保持 loop 负责（GUIDED 速度指令约 2~3 s 无新指令即回落悬停）。
    /// 无条件解除是回归：旧实现在「无目标」时压根不发命令（`decide_idle → action: None`），抑制
    /// 自然保持；连续栈 `on_tick` 每拍都调本方法，无任务时是 `(0, 0)`，若无条件解除则 10 Hz
    /// 零速度会在起飞/降落期间打断爬升/降落（违反 §二 Step 5 验收 4）。
    /// 无返回值：锁写不会失败；`try_send` 失败由保持 loop `warn!` 并在下一拍照常补发。
    pub fn apply_velocity(&self, v: f32, omega: f32) {
        if v != 0.0 || omega != 0.0 {
            self.takeoff_pending.store(false, Ordering::SeqCst);
        }
        *self.desired.lock().unwrap() = DesiredVelocity { v, omega };
    }

    /// 停车 / 悬停 = 连续量 `(0, 0)`，并**解除**起飞/降落抑制
    /// （等价旧的离散「停车」设定语义；`reset` / `suspend` / `DeviceHandler::stop` 的唯一出口
    /// —— 显式停车不该被上面「零速度不解除抑制」的保护挡住）
    pub fn stop(&self) {
        self.takeoff_pending.store(false, Ordering::SeqCst);
        *self.desired.lock().unwrap() = DesiredVelocity::default();
    }

    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

// ============================================================
// 单测（纯函数：速度翻译 / 坐标对齐 / 遥测解析）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use mavlink::dialects::ardupilotmega::{
        MavAutopilot, MavMessage, MavModeFlag, MavState, MavType, HEARTBEAT_DATA,
    };
    use mavlink::MavHeader;

    /// 测试用设备（不 spawn 串口：TX 通道用一对本地 channel 代替）
    fn test_device() -> (MavlinkDevice, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel(8);
        let dev = MavlinkDevice {
            serial_cmd_tx: tx,
            desired: Arc::new(Mutex::new(DesiredVelocity::default())),
            state: Arc::new(RwLock::new(Telemetry::default())),
            system_id: GCS_SYSTEM_ID,
            component_id: GCS_COMPONENT_ID,
            yaw_offset: Arc::new(Mutex::new(None)),
            takeoff_pending: Arc::new(AtomicBool::new(false)),
            cancel: CancellationToken::new(),
        };
        (dev, rx)
    }

    #[test]
    fn test_velocity_to_mavlink_sign() {
        // 前进：vx = v，偏航零
        assert_eq!(velocity_to_mavlink(0.3, 0.0), (0.3, 0.0, 0.0, 0.0));
        // 左转（omega > 0 = 逆时针）⇒ NED yaw_rate 必须取负（写反必红）
        assert_eq!(velocity_to_mavlink(0.0, 0.5), (0.0, 0.0, 0.0, -0.5));
        // 右转 ⇒ 正
        assert_eq!(velocity_to_mavlink(0.0, -0.5), (0.0, 0.0, 0.0, 0.5));
        // 停 = 全零
        assert_eq!(velocity_to_mavlink(0.0, 0.0), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn test_velocity_and_yaw_simultaneous() {
        // 边走边转：vx 与 yaw_rate 同帧非零（旧离散表做不到，连续量必须能 —— §1.4 弧线档）
        let (vx, _, _, yaw) = velocity_to_mavlink(0.3, 0.2618);
        assert_eq!((vx, yaw), (0.3, -0.2618));
        assert!(vx != 0.0 && yaw != 0.0, "两轴必须同时非零");
    }

    #[test]
    fn test_manual_cmd_velocity_path() {
        // `Motion` 原值透传（不做量级换算、不做 clamp）
        assert_eq!(
            manual_cmd_velocity(&ManualCmd::Motion { v: 0.3, omega: 0.2618 }),
            Some((0.3, 0.2618))
        );
        // 内部 `Stop` = `(0, 0)`
        assert_eq!(manual_cmd_velocity(&ManualCmd::Stop), Some((0.0, 0.0)));
        // Takeoff / Land / Beep / LiDAR 不走速度出口（保持原消息）
        assert_eq!(manual_cmd_velocity(&ManualCmd::Takeoff), None);
        assert_eq!(manual_cmd_velocity(&ManualCmd::Land), None);
        assert_eq!(manual_cmd_velocity(&ManualCmd::Beep(100)), None);
    }

    #[test]
    fn test_handle_manual_cmd_motion_and_takeoff() {
        let (dev, _rx) = test_device();
        // 初始 = 停（与旧的离散「停」动作同义）
        assert_eq!(*dev.desired.lock().unwrap(), DesiredVelocity::default());
        // Motion：两轴原值进 desired
        dev.handle_manual_cmd(&ManualCmd::Motion { v: 0.3, omega: 0.2618 });
        assert_eq!(*dev.desired.lock().unwrap(), DesiredVelocity { v: 0.3, omega: 0.2618 });
        // 内部 Stop：清成 (0, 0)
        dev.handle_manual_cmd(&ManualCmd::Stop);
        assert_eq!(*dev.desired.lock().unwrap(), DesiredVelocity::default());
        // Takeoff：不碰速度设定，只置 takeoff_pending（抑制保持 loop，起飞行为不变）
        dev.handle_manual_cmd(&ManualCmd::Motion { v: 0.2, omega: 0.0 });
        dev.handle_manual_cmd(&ManualCmd::Takeoff);
        assert!(dev.takeoff_pending.load(Ordering::SeqCst));
        assert_eq!(dev.desired.lock().unwrap().v, 0.2, "Takeoff 不得改写速度设定");
    }

    #[test]
    fn test_takeoff_suppression_survives_zero_velocity() {
        let (dev, _rx) = test_device();
        // on_tick 无任务时每拍都发 (0,0) —— 不得打断爬升/降落（§二 Step 5 验收 4）
        dev.takeoff_pending.store(true, Ordering::SeqCst);
        dev.apply_velocity(0.0, 0.0);
        assert!(dev.takeoff_pending.load(Ordering::SeqCst), "零速度不得解除起飞抑制");
        assert_eq!(*dev.desired.lock().unwrap(), DesiredVelocity::default());
        // 任何非零速度 = 导航/手动接管 → 解除抑制
        dev.apply_velocity(0.0, 0.5);
        assert!(!dev.takeoff_pending.load(Ordering::SeqCst), "非零角速度必须解除抑制");
    }

    #[test]
    fn test_stop_clears_takeoff_suppression() {
        let (dev, _rx) = test_device();
        dev.takeoff_pending.store(true, Ordering::SeqCst);
        dev.stop();
        assert!(!dev.takeoff_pending.load(Ordering::SeqCst), "显式停车解除抑制（等价旧离散「停车」设定语义）");
        assert_eq!(*dev.desired.lock().unwrap(), DesiredVelocity::default());
    }

    #[test]
    fn test_aligned_world_pose() {
        // offset=0：无旋转，只平移
        let (wx, wy, yaw) = aligned_world_pose(1.0, 0.0, 0.0, 0.0, (64.0, 64.0));
        assert_eq!(wx, 65.0);
        assert_eq!(wy, 64.0);
        assert_eq!(yaw, 0.0);

        // offset=90°：机头朝东，世界 x 轴 = 东；北向 1m → wy - 1
        let off = std::f32::consts::FRAC_PI_2;
        let (wx, wy, _) = aligned_world_pose(1.0, 0.0, 0.0, off, (64.0, 64.0));
        assert!((wx - 64.0).abs() < 1e-6);
        assert!((wy - 63.0).abs() < 1e-6);
    }

    #[test]
    fn test_aligned_world_pose_yaw_matches_forward_direction() {
        // 一致性：NED 前进方向 (cos ψ, sin ψ) 经位置变换后，应等于世界前进方向 (cos yaw_world, sin yaw_world)。
        // 防止 yaw 与位置矩阵不自洽（曾误把 yaw 取反导致镜像）。
        let off = 0.7f32;
        let psi = 1.2f32;
        let (wx, wy, yaw_world) = aligned_world_pose(psi.cos(), psi.sin(), psi, off, (0.0, 0.0));
        assert!((wx - yaw_world.cos()).abs() < 1e-4, "前进 x 分量应与 cos(yaw_world) 一致: wx={wx}");
        assert!((wy - yaw_world.sin()).abs() < 1e-4, "前进 y 分量应与 sin(yaw_world) 一致: wy={wy}");
    }

    #[test]
    fn test_handle_message_heartbeat_armed() {
        let mut t = Telemetry::default();
        let header = MavHeader::default();

        // 解锁：base_mode 含 SAFETY_ARMED
        protocol::handle_message(
            &mut t,
            header.clone(),
            MavMessage::HEARTBEAT(HEARTBEAT_DATA {
                custom_mode: MODE_GUIDED,
                mavtype: MavType::MAV_TYPE_QUADROTOR,
                autopilot: MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
                base_mode: MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED,
                system_status: MavState::MAV_STATE_ACTIVE,
                mavlink_version: 3,
            }),
        );
        assert!(t.armed);
        assert_eq!(t.custom_mode, MODE_GUIDED);
        assert_eq!(t.heartbeat_count, 1);

        // 上锁：base_mode 空
        protocol::handle_message(
            &mut t,
            header,
            MavMessage::HEARTBEAT(HEARTBEAT_DATA {
                custom_mode: MODE_GUIDED,
                mavtype: MavType::MAV_TYPE_QUADROTOR,
                autopilot: MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
                base_mode: MavModeFlag::empty(),
                system_status: MavState::MAV_STATE_ACTIVE,
                mavlink_version: 3,
            }),
        );
        assert!(!t.armed);
    }
}
