//Presented by KeJi
//Created Date ： 2026-08-25
//Modified Date ： 2026-08-26

//! MAVLink 飞控设备驱动（Task 22_4）
//!
//! `MavlinkDevice` — 封装 MAVLink 协议 + 串口 TX/RX（Task 23 阶段 C：实现 base 的 `DeviceHandler`）。
//!
//! 自闭环：上层只发离散动作（`move_forward` / `stop`），设备内部用
//! 「期望动作 + 10Hz 保持 loop」持续下发速度指令（GUIDED 速度指令约 2~3s 无新指令即回落悬停）。

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
use pleiades_base::robot::core::state::{MotionAction, RobotState};

// ============================================================
// MavlinkDevice — 控制句柄
// ============================================================

pub struct MavlinkDevice {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    /// 期望动作（std 锁：trait 方法同步、不可 await；临界区纳秒级）
    desired: Arc<Mutex<MotionAction>>,
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

/// 期望动作 → 速度四元组（速度由设备层绑定，Task 22_5 D2）
fn action_to_velocity(a: MotionAction, vel_fwd: f32, yaw_rate_deg: f32) -> (f32, f32, f32, f32) {
    let yaw_rate = yaw_rate_deg.to_radians();
    match a {
        MotionAction::MoveForward => (vel_fwd, 0.0, 0.0, 0.0),
        MotionAction::MoveBackward => (-vel_fwd, 0.0, 0.0, 0.0),
        // 机体系 NED：yaw_rate 正 = 顺时针/右转（与 UAV turn_degrees 一致）
        MotionAction::TurnLeft => (0.0, 0.0, 0.0, -yaw_rate),
        MotionAction::TurnRight => (0.0, 0.0, 0.0, yaw_rate),
        MotionAction::Stop => (0.0, 0.0, 0.0, 0.0), // 停 = 悬停 = 零速度
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
        vel_fwd: f32,
        yaw_rate_deg: f32,
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let robot_state_clone = robot_state.clone();
        let desired = Arc::new(Mutex::new(MotionAction::Stop));
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

        // 保持 loop（10Hz）：读期望动作 → 翻译 → 持续下发速度指令
        let keep_tx = serial_cmd_tx.clone();
        let keep_desired = desired_clone.clone();
        let keep_cancel = cancel.clone();
        let keep_vel_fwd = vel_fwd;
        let keep_yaw_rate = yaw_rate_deg;
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
                        let action = keep_desired.lock().unwrap().clone();
                        let (vx, vy, vz, yaw_rate) = action_to_velocity(action, keep_vel_fwd, keep_yaw_rate);
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

    // ─── 命令/动作解析（A6：下沉到设备，设备选择性响应）────────
    pub fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        match cmd {
            ManualCmd::Forward(_) => { let _ = self.set_desired(MotionAction::MoveForward); }
            ManualCmd::Backward(_) => { let _ = self.set_desired(MotionAction::MoveBackward); }
            ManualCmd::SpinLeft(_) => { let _ = self.set_desired(MotionAction::TurnLeft); }
            ManualCmd::SpinRight(_) => { let _ = self.set_desired(MotionAction::TurnRight); }
            ManualCmd::Stop => { let _ = self.set_desired(MotionAction::Stop); }
            ManualCmd::Takeoff => { let _ = self.takeoff_send(); }
            ManualCmd::Land => { let _ = self.land_send(); }
            _ => {} // 机不支持 Beep/LiDAR 命令，忽略
        }
    }

    pub fn apply_action(&self, action: MotionAction) {
        let _ = self.set_desired(action);
    }

    pub fn stop(&self) -> Result<(), String> {
        self.set_desired(MotionAction::Stop)
    }

    fn set_desired(&self, action: MotionAction) -> Result<(), String> {
        self.takeoff_pending.store(false, Ordering::SeqCst);
        *self.desired.lock().unwrap() = action;
        Ok(())
    }

    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

// ============================================================
// 单测（纯函数：翻译表 / 坐标对齐 / 遥测解析）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use mavlink::dialects::ardupilotmega::{
        MavAutopilot, MavMessage, MavModeFlag, MavState, MavType, HEARTBEAT_DATA,
    };
    use mavlink::MavHeader;

    #[test]
    fn test_action_to_velocity_translation() {
        // 前进/后退：固定 VEL_FWD
        assert_eq!(
            action_to_velocity(MotionAction::MoveForward, VEL_FWD, YAW_RATE_DEG),
            (VEL_FWD, 0.0, 0.0, 0.0)
        );
        assert_eq!(
            action_to_velocity(MotionAction::MoveBackward, VEL_FWD, YAW_RATE_DEG),
            (-VEL_FWD, 0.0, 0.0, 0.0)
        );
        // 转向：固定 YAW_RATE
        let yaw = YAW_RATE_DEG.to_radians();
        assert_eq!(
            action_to_velocity(MotionAction::TurnLeft, VEL_FWD, YAW_RATE_DEG),
            (0.0, 0.0, 0.0, -yaw)
        );
        assert_eq!(
            action_to_velocity(MotionAction::TurnRight, VEL_FWD, YAW_RATE_DEG),
            (0.0, 0.0, 0.0, yaw)
        );
        // 停：零速度（悬停）
        assert_eq!(
            action_to_velocity(MotionAction::Stop, VEL_FWD, YAW_RATE_DEG),
            (0.0, 0.0, 0.0, 0.0)
        );
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
