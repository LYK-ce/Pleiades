//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-29

#![allow(non_snake_case)]

//! 四旋翼（世界零件 `quadrotor_uav`）—— task_31 §六.5.5
//!
//! 与差速**共用**同一张翻译表（§六.5.5「同表」）：`MoveForward/Backward → forward/backward`、
//! `TurnLeft/Right → left/right`（世界 `Quadrotor` 的 `left/right` 也是纯偏航）、`Stop → stop`。
//!
//! 唯一多出来的两行是**手动命令**：`Takeoff → "up"`、`Land → "down"`。
//! 世界侧 `up/down` 用 `max_climb_rate · pct`，与 `left/right` 的 `max_yaw_rate · pct` 共用
//! `SpeedPct`，所以爬升速度复用 `turn_speed`。
//!
//! **G3 已知差异**（§六.2.6）：真机 UAV 的速度是 m/s 常量（`vel_fwd` / `yaw_rate_deg`），
//! 世界是 0~100 百分比 —— 这里只是「语义近似、非等价」的绑定：
//! `vel_fwd ≈ forward_speed`、`yaw_rate_deg ≈ turn_speed`，**不给 UAV 增加新配置项**。
//!
//! `Beep` 与差速一致：世界无蜂鸣指令 → 不下发，`warn!` 一次（§六.5.5「忽略」= 不下发）。

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::state::MotionAction;

use crate::sim::chassis::pose_ingest::{PoseFields, SpawnFields};
use crate::sim::chassis::{Chassis, ChassisBase};

/// 四旋翼（`up`/`down` 用 `turn_speed` 档位；G3 近似）
pub struct QuadrotorChassis {
    base: ChassisBase,
}

impl QuadrotorChassis {
    pub fn New(base: ChassisBase) -> Self {
        Self { base }
    }

    /// `Takeoff` / `Land` 的世界指令：`up` / `down` + 爬升档位（复用 `turn`，G3）
    fn Send_Climb(&self, action: &'static str) {
        if let Err(e) = self.base.motion.Send(action, self.base.speed.turn) {
            debug!("[chassis] {} {action} 未投递：{e}", self.base.name);
        }
    }
}

#[async_trait]
impl Chassis for QuadrotorChassis {
    fn Name(&self) -> &str {
        &self.base.name
    }

    fn Apply_Action(&self, action: MotionAction) {
        self.base.Dispatch(action);
    }

    /// 连续速度指令（task_39 §四.4）：档位原值直接送世界（`action="velocity"` + `v` / `omega`）。
    ///
    /// 恒返回 `true`（= **支持**这路指令）：`Err` 只表示这一帧没进通道（未装配 / 通道满），
    /// 速度指令每 50 ms 重发一次，下一拍自愈，不需要在这里升级成错误。
    fn Apply_Velocity(&self, v: f32, omega: f32) -> bool {
        if let Err(e) = self.base.motion.Send_Velocity(v, omega) {
            debug!("[chassis] {} 速度指令未投递：{e}", self.base.name);
        }
        true
    }

    fn Handle_Manual_Cmd(&self, cmd: &ManualCmd) {
        // Forward / Backward / SpinLeft / SpinRight / Stop（与差速同一套运动分支）
        if self.base.Send_Manual_Motion(cmd) {
            return;
        }
        match cmd {
            // 四旋翼独有的两条：`up` / `down` + 爬升档位（= turn_speed，G3 的近似）
            ManualCmd::Takeoff => self.Send_Climb("up"),
            ManualCmd::Land => self.Send_Climb("down"),
            // 世界没有蜂鸣指令：不下发，只 warn 一次（§六.5.5 / 硬验收 3）
            ManualCmd::Beep(_) => self.base.Warn_Beep("chassis.uav.beep", cmd),
            // StartLidarScan / StopLidarScan：由 SimDeviceHandler 拦截，这里兜底不响应
            _ => {}
        }
    }

    async fn On_Pose(&self, p: &PoseFields) {
        self.base.pose.On_Pose(p).await;
    }

    async fn On_Spawn(&self, s: &SpawnFields) {
        self.base.pose.On_Spawn(s).await;
    }

    fn Pose_Age_Ms(&self) -> u64 {
        self.base.pose.Age_Ms()
    }

    fn Pose_Age_Handle(&self) -> Arc<AtomicU64> {
        self.base.pose.Age_Handle()
    }

    fn Pose_History(&self) -> Arc<crate::sim::chassis::pose_ingest::PoseHistory> {
        self.base.pose.History_Handle()
    }

    fn Stop(&self) {
        // G1：通道满则登记补发，绝不静默丢
        let _ = self.base.motion.Send_Stop_Priority();
    }
}

// ================================================================ 单测（硬验收 3）

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChassisConfig;
    use crate::sim::chassis::action_map::SpeedBinding;
    use crate::sim::chassis::{Build, ChassisBase};
    use crate::world_link::{MotionSender, MOTION_CHANNEL_CAP};
    use pleiades_base::robot::core::state::RobotState;
    use tokio::sync::{mpsc, RwLock};

    type Rx = mpsc::Receiver<crate::world_link::protocol::MotionCmd>;

    /// 已装配的四旋翼（默认 30/10）
    fn quad() -> (Arc<dyn Chassis>, Rx) {
        let (tx, rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        tx.Set_Assembled(true);
        let base = ChassisBase::New(
            "quadrotor_uav",
            SpeedBinding::From_Config(Some(&ChassisConfig::default())),
            tx,
            Arc::new(RwLock::new(RobotState::default())),
        );
        (Arc::new(QuadrotorChassis::New(base)), rx)
    }

    fn assert_empty(rx: &mut Rx) {
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(c) => panic!("不该下发任何命令，却收到 action={} speed={}", c.action, c.speed),
            Err(e) => panic!("通道意外关闭：{e}"),
        }
    }

    #[test]
    fn test_uav_takeoff_land() {
        // 四旋翼唯一多出来的两条：up / down + turn_speed（10）
        let (ch, mut rx) = quad();

        ch.Handle_Manual_Cmd(&ManualCmd::Takeoff);
        let m = rx.try_recv().expect("Takeoff 应下发 up");
        assert_eq!((m.action, m.speed), ("up", 10));

        ch.Handle_Manual_Cmd(&ManualCmd::Land);
        let m = rx.try_recv().expect("Land 应下发 down");
        assert_eq!((m.action, m.speed), ("down", 10));
    }

    #[test]
    fn test_uav_forward_backward_spin() {
        let (ch, mut rx) = quad();

        ch.Handle_Manual_Cmd(&ManualCmd::Forward(500));
        let m = rx.try_recv().expect("Forward 应下发");
        assert_eq!((m.action, m.speed), ("forward", 30), "载荷忽略，用配置速度");

        ch.Handle_Manual_Cmd(&ManualCmd::Backward(-1));
        let m = rx.try_recv().expect("Backward 应下发");
        assert_eq!((m.action, m.speed), ("backward", 30));

        ch.Handle_Manual_Cmd(&ManualCmd::SpinRight(100));
        let m = rx.try_recv().expect("SpinRight 应下发");
        assert_eq!((m.action, m.speed), ("right", 10));

        ch.Handle_Manual_Cmd(&ManualCmd::Stop);
        let m = rx.try_recv().expect("Stop 应下发");
        assert_eq!((m.action, m.speed), ("stop", 0));
    }

    #[test]
    fn test_uav_beep_no_send() {
        // Beep：不下发（只 warn 一次）
        let (ch, mut rx) = quad();
        ch.Handle_Manual_Cmd(&ManualCmd::Beep(1));
        assert_empty(&mut rx);
    }

    #[test]
    fn test_uav_apply_action_table() {
        // §六.5.5「同表」：五个动作与差速完全一致（含 Stop 恒 0）
        let (ch, mut rx) = quad();
        let expect = [
            (MotionAction::MoveForward, ("forward", 30)),
            (MotionAction::MoveBackward, ("backward", 30)),
            (MotionAction::TurnLeft, ("left", 10)),
            (MotionAction::TurnRight, ("right", 10)),
            (MotionAction::Stop, ("stop", 0)),
        ];
        for (action, want) in expect {
            ch.Apply_Action(action);
            let m = rx.try_recv().expect("每个动作都要下发一帧");
            assert_eq!((m.action, m.speed), want, "{action:?} 的 wire 错");
        }
    }

    #[test]
    fn test_uav_lidar_cmds_not_answered() {
        // 雷达命令由 SimDeviceHandler 拦截；四旋翼本身也不响应
        let (ch, mut rx) = quad();
        ch.Handle_Manual_Cmd(&ManualCmd::StartLidarScan);
        ch.Handle_Manual_Cmd(&ManualCmd::StopLidarScan);
        assert_empty(&mut rx);
    }

    #[test]
    fn test_uav_build_via_factory() {
        let (tx, mut rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        tx.Set_Assembled(true);
        let ch = Build("quadrotor_uav", &ChassisConfig::default(), tx, Arc::new(RwLock::new(RobotState::default())))
            .expect("quadrotor_uav 必须能建");
        assert_eq!(ch.Name(), "quadrotor_uav");
        ch.Handle_Manual_Cmd(&ManualCmd::Takeoff);
        let m = rx.try_recv().expect("Takeoff 应下发 up");
        assert_eq!((m.action, m.speed), ("up", 10));
    }

    #[test]
    fn test_uav_apply_velocity_dispatch() {
        // task_39 §四.4：四旋翼支持速度指令 → 一帧 action="velocity" + v / omega（speed 恒 0）
        let (ch, mut rx) = quad();
        assert!(ch.Apply_Velocity(0.5, -0.25), "四旋翼必须支持 Apply_Velocity");
        let m = rx.try_recv().expect("速度指令应下发一帧");
        assert_eq!(m.action, "velocity");
        assert_eq!(m.speed, 0);
        assert_eq!((m.v, m.omega), (Some(0.5), Some(-0.25)));
    }

    #[test]
    fn test_uav_apply_velocity_before_assembled_still_supported() {
        // 未装配：返回值仍 true（= 支持），但通道里什么都没有（Send_Velocity 只 debug 丢弃）
        let (tx, mut rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        let base = ChassisBase::New(
            "quadrotor_uav",
            SpeedBinding::From_Config(Some(&ChassisConfig::default())),
            tx,
            Arc::new(RwLock::new(RobotState::default())),
        );
        let ch: Arc<dyn Chassis> = Arc::new(QuadrotorChassis::New(base));
        assert!(ch.Apply_Velocity(0.5, -0.25), "未装配也算「支持」（bool 不表示已送达）");
        assert_empty(&mut rx);
    }
}
