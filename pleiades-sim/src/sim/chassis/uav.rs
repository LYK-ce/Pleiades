//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-11-01

#![allow(non_snake_case)]

//! 四旋翼（世界零件 `quadrotor_uav`）—— task_31 §六.5.5 / task_44 §1.3
//!
//! 机栈只走 [`Chassis::Apply_Velocity`] `(v, omega)` —— `action="velocity"` + `v` m/s /
//! `omega` rad/s，世界 `Quadrotor.gd` 的 `"velocity"` 分支直接 `clampf` 收下（逐字同现状，
//! 本 step 一行不改）。`ManualCmd::Motion { v, omega }` 与车栈一样**原值透传**。
//!
//! 唯一还走**百分比档**的是手动起降：`Takeoff → "up"`、`Land → "down"`（世界 `up/down` 仍是
//! `max_climb_rate · pct`，Step 5 不动世界）。档位取本文件的 [`CLIMB_PCT`] 常量
//! （= 旧 `[chassis].turn_speed` 缺省值 10，配置键已删 → 常量；G3 语义近似保留）。
//!
//! `Beep` 与差速一致：世界无蜂鸣指令 → 不下发，`warn!` 一次（§六.5.5「忽略」= 不下发）。

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use pleiades_base::robot::core::command::ManualCmd;

use crate::sim::chassis::pose_ingest::{PoseFields, SpawnFields};
use crate::sim::chassis::{Chassis, ChassisBase};

/// 起降档位（0~100 百分比，世界 `up/down` 的 `max_climb_rate · pct`）。
///
/// 取旧 `[chassis].turn_speed` 的缺省值 10（task_31 §六.2.6 G3 的「语义近似、非等价」）；
/// 该配置键已随 task_44 §1.2 #12 删除，故改成本文件常量。
const CLIMB_PCT: u8 = 10;

/// 四旋翼（`up`/`down` 用 [`CLIMB_PCT`] 档位；G3 近似）
pub struct QuadrotorChassis {
    base: ChassisBase,
}

impl QuadrotorChassis {
    pub fn New(base: ChassisBase) -> Self {
        Self { base }
    }

    /// `Takeoff` / `Land` 的世界指令：`up` / `down` + 爬升档位（G3 近似）
    fn Send_Climb(&self, action: &'static str) {
        if let Err(e) = self.base.motion.Send(action, CLIMB_PCT) {
            debug!("[chassis] {} {action} 未投递：{e}", self.base.name);
        }
    }
}

#[async_trait]
impl Chassis for QuadrotorChassis {
    fn Name(&self) -> &str {
        &self.base.name
    }

    /// 连续速度指令（task_39 §四.4）：`v` / `omega` 物理量直接送世界（`action="velocity"`）。
    ///
    /// 恒返回 `true`（= **支持**这路指令）：`Err` 只表示这一帧没进通道（未装配 / 通道满），
    /// 速度指令每 50 ms 重发一次，下一拍自愈，不需要在这里升级成错误。
    fn Apply_Velocity(&self, v: f32, omega: f32) -> bool {
        self.base.Send_Velocity(v, omega);
        true
    }

    fn Handle_Manual_Cmd(&self, cmd: &ManualCmd) {
        // Motion / Stop（与差速同一套运动分支）
        if self.base.Dispatch_Manual(cmd) {
            return;
        }
        match cmd {
            // 四旋翼独有的两条：`up` / `down` + 爬升档位（[`CLIMB_PCT`]，G3 的近似）
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

    /// 已装配的四旋翼（缺省 0.45 m/s / 0.5 rad/s）
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
        // 四旋翼唯一还走百分比的两条：up / down + CLIMB_PCT（= 旧 turn_speed 缺省 10）
        let (ch, mut rx) = quad();

        ch.Handle_Manual_Cmd(&ManualCmd::Takeoff);
        let m = rx.try_recv().expect("Takeoff 应下发 up");
        assert_eq!((m.action, m.speed), ("up", 10));

        ch.Handle_Manual_Cmd(&ManualCmd::Land);
        let m = rx.try_recv().expect("Land 应下发 down");
        assert_eq!((m.action, m.speed), ("down", 10));
    }

    #[test]
    fn test_uav_manual_motion() {
        // task_44 §1.3：手动运动统一走物理量（原值透传，不再有 forward/spin 分支）
        let (ch, mut rx) = quad();

        ch.Handle_Manual_Cmd(&ManualCmd::Motion { v: 0.45, omega: -0.5 });
        let m = rx.try_recv().expect("Motion 应下发");
        assert_eq!(m.action, "velocity");
        assert_eq!((m.v, m.omega), (Some(0.45), Some(-0.5)), "原值透传，不取反");

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
