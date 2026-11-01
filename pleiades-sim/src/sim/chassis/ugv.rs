//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-11-01

#![allow(non_snake_case)]

//! 差速底盘（世界零件 `diff_drive_ugv`）—— task_31 §六.5.4 / task_44 §1.3
//!
//! 只写**真正不同**的部分：`Handle_Manual_Cmd` 里的 `Beep` / `Takeoff` / `Land` / `Lidar` 分支。
//! 其余（手动运动命令、位姿写回、停车）全在 [`ChassisBase`] 里，两个底盘共用。
//!
//! 与世界的对应（task_44 §1.3）：[`Chassis::Apply_Velocity`] `(v, omega)` →
//! `motion_cmd{action:"velocity", v, omega}` → 世界 `DiffDrive.gd` 直接收物理量
//! （`v` m/s、`omega` rad/s，`omega > 0` = 左转；clamp 上限 `max_speed` / `max_turn_rate`）。
//! 手动就是同一条路：`ManualCmd::Motion { v, omega }` **原值透传**（`W` → `(0.45, 0)`、
//! `W+A` → `(0.45, +0.5)`、`A` → `(0, +0.5)`、全松 → `(0, 0)`，见 task_44 §1.4）。
//!
//! 真机对照（`pleiades-ugv`，只读参考、不引用）：`Handle_Manual_Cmd` 里 `Stop → stop()`、
//! `Beep(ms) → beep(ms)`（车支持蜂鸣，**世界不支持**）、`_ => return`（车不响应 Takeoff/Land/LiDAR）。
//!
//! 符号**不取反**：`omega` 的符号证据链四步在 `sim/ugv_sim/executor.rs` 文件头，本文件只透传。

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;

use pleiades_base::robot::core::command::ManualCmd;

use crate::sim::chassis::pose_ingest::{PoseFields, SpawnFields};
use crate::sim::chassis::{Chassis, ChassisBase};

/// 差速底盘（两轮差速 + 原地偏航）
pub struct DiffDriveChassis {
    base: ChassisBase,
}

impl DiffDriveChassis {
    pub fn New(base: ChassisBase) -> Self {
        Self { base }
    }
}

#[async_trait]
impl Chassis for DiffDriveChassis {
    fn Name(&self) -> &str {
        &self.base.name
    }

    /// 连续速度指令（task_44 §1.3）：一帧 `action="velocity"` + `v` / `omega` 物理量，
    /// 恒返回 `true`（= 支持）；`Err` 只表示这一帧没进通道（未装配 / 通道满），50 ms 下一拍自愈。
    fn Apply_Velocity(&self, v: f32, omega: f32) -> bool {
        self.base.Send_Velocity(v, omega);
        true
    }

    fn Handle_Manual_Cmd(&self, cmd: &ManualCmd) {
        // Motion / Stop（两个运动分支，共用实现）
        if self.base.Dispatch_Manual(cmd) {
            return;
        }
        match cmd {
            // 世界没有蜂鸣指令：不下发，只 warn 一次（§六.5.4；硬验收 3）
            ManualCmd::Beep(_) => self.base.Warn_Beep("chassis.ugv.beep", cmd),
            // Takeoff / Land：车不支持（与真机 `_ => return` 一致，硬验收 3）
            // StartLidarScan / StopLidarScan：正常路径由 SimDeviceHandler 拦截，这里兜底不响应
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

    /// 已装配的差速底盘（缺省 0.45 m/s / 0.5 rad/s）
    fn diff() -> (Arc<dyn Chassis>, Rx) {
        let (tx, rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        tx.Set_Assembled(true);
        let base = ChassisBase::New(
            "diff_drive_ugv",
            SpeedBinding::From_Config(Some(&ChassisConfig::default())),
            tx,
            Arc::new(RwLock::new(RobotState::default())),
        );
        (Arc::new(DiffDriveChassis::New(base)), rx)
    }

    /// 断言通道为空（什么都没发）
    fn assert_empty(rx: &mut Rx) {
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(c) => panic!("不该下发任何命令，却收到 action={} speed={}", c.action, c.speed),
            Err(e) => panic!("通道意外关闭：{e}"),
        }
    }

    #[test]
    fn test_diff_ignores_takeoff_land() {
        let (ch, mut rx) = diff();
        ch.Handle_Manual_Cmd(&ManualCmd::Takeoff);
        ch.Handle_Manual_Cmd(&ManualCmd::Land);
        assert_empty(&mut rx);
    }

    #[test]
    fn test_diff_ignores_lidar_cmds() {
        // 兜底：正常路径由 SimDeviceHandler 拦截；底盘本身绝不响应
        let (ch, mut rx) = diff();
        ch.Handle_Manual_Cmd(&ManualCmd::StartLidarScan);
        ch.Handle_Manual_Cmd(&ManualCmd::StopLidarScan);
        assert_empty(&mut rx);
    }

    #[test]
    fn test_diff_manual_motion_passthrough() {
        // task_44 §1.4：手动就是同一张定值表 —— (v, omega) **原值透传**，不做换算
        let (ch, mut rx) = diff();

        ch.Handle_Manual_Cmd(&ManualCmd::Motion { v: 0.45, omega: 0.5 });
        let m = rx.try_recv().expect("Motion 应下发");
        assert_eq!(m.action, "velocity");
        assert_eq!((m.v, m.omega), (Some(0.45), Some(0.5)), "W+A ⇒ (0.45, +0.5)");
        assert_eq!(m.speed, 0, "速度帧不带百分比档位");

        // 后退：v 取负是合法的（世界侧 clamp 到 ±max_speed）
        ch.Handle_Manual_Cmd(&ManualCmd::Motion { v: -0.45, omega: 0.0 });
        let m = rx.try_recv().expect("后退 Motion 应下发");
        assert_eq!((m.v, m.omega), (Some(-0.45), Some(0.0)));

        // 全松键 (0, 0) 也是合法指令（不是 stop 帧：世界侧两者都归零，语义等价）
        ch.Handle_Manual_Cmd(&ManualCmd::Motion { v: 0.0, omega: 0.0 });
        let m = rx.try_recv().expect("(0,0) Motion 应下发");
        assert_eq!((m.v, m.omega), (Some(0.0), Some(0.0)));

        ch.Handle_Manual_Cmd(&ManualCmd::Stop);
        let m = rx.try_recv().expect("Stop 应下发");
        assert_eq!((m.action, m.speed), ("stop", 0));
    }

    #[test]
    fn test_diff_beep_only_warns() {
        // Beep：世界无蜂鸣 → 不下发任何 motion_cmd（只 warn 一次，硬验收 3）
        let (ch, mut rx) = diff();
        ch.Handle_Manual_Cmd(&ManualCmd::Beep(200));
        assert_empty(&mut rx);
    }

    #[test]
    fn test_diff_stop_uses_config_and_channel() {
        // `Chassis::Stop` 也是走同一条通道（安全关键路径），与配置无关（恒 speed=0）
        let (ch, mut rx) = diff();
        ch.Stop();
        let m = rx.try_recv().expect("Stop 应下发");
        assert_eq!((m.action, m.speed), ("stop", 0));
    }

    #[test]
    fn test_diff_apply_velocity_dispatch() {
        // task_44 §1.3：物理量原值直接透传到 wire（不做任何换算），两向都过一遍
        let (ch, mut rx) = diff();
        assert!(ch.Apply_Velocity(0.45, -0.5), "差速底盘必须支持 Apply_Velocity");

        let m = rx.try_recv().expect("速度帧应下发");
        assert_eq!(m.action, "velocity");
        assert_eq!(m.speed, 0, "速度帧不带百分比档位");
        assert_eq!(m.v, Some(0.45));
        assert_eq!(m.omega, Some(-0.5), "右转 ⇒ omega < 0（符号见 ugv_sim/executor.rs 文件头）");

        assert!(ch.Apply_Velocity(0.0, 0.5));
        let m = rx.try_recv().expect("原地左转应下发");
        assert_eq!((m.v, m.omega), (Some(0.0), Some(0.5)));
    }

    #[test]
    fn test_diff_apply_velocity_before_assembled_still_supported() {
        // 未装配：仍返回 true（= 底盘支持），只是这一帧被 `Send_Velocity` 静默丢弃
        let (tx, mut rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        let base = ChassisBase::New(
            "diff_drive_ugv",
            SpeedBinding::From_Config(Some(&ChassisConfig::default())),
            tx,
            Arc::new(RwLock::new(RobotState::default())),
        );
        let ch = DiffDriveChassis::New(base);
        assert!(ch.Apply_Velocity(0.45, -0.5));
        assert_empty(&mut rx);
    }

    #[test]
    fn test_build_via_factory() {
        // 工厂造出来的也是差速（`Build` 与直接 `New` 行为一致）
        let (tx, mut rx) = MotionSender::Channel(MOTION_CHANNEL_CAP);
        tx.Set_Assembled(true);
        let ch = Build("diff_drive_ugv", &ChassisConfig::default(), tx, Arc::new(RwLock::new(RobotState::default())))
            .expect("diff_drive_ugv 必须能建");
        assert_eq!(ch.Name(), "diff_drive_ugv");
        ch.Handle_Manual_Cmd(&ManualCmd::Takeoff);
        assert_empty(&mut rx);
    }
}
