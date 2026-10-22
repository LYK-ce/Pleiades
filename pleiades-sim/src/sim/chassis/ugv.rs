//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 差速底盘（世界零件 `diff_drive_ugv`）—— task_31 §六.5.4
//!
//! 只写**真正不同**的部分：`Handle_Manual_Cmd` 里的 `Beep` / `Takeoff` / `Land` / `Lidar` 分支。
//! 其余（动作分发、手动运动命令、位姿写回、停车）全在 [`ChassisBase`] 里，两个底盘共用。
//!
//! 真机对照（`pleiades-ugv/src/ugv/stm32/mod.rs:134-164`，只读参考、不引用）：
//! - `apply_action`：`MoveForward → forward(forward_speed)`、`TurnLeft → spin_left(turn_speed)`（原地偏航）、
//!   `Stop → stop()`（有界重试）
//! - `handle_manual_cmd`：`Forward(_)/Backward(_)` **忽略载荷**、`Stop → stop()`、
//!   `Beep(ms) → beep(ms)`（车支持蜂鸣，**世界不支持**）、`_ => return`（车不响应 Takeoff/Land/LiDAR）
//!
//! 与世界的对应：`TurnLeft → "left"`、`TurnRight → "right"`（世界侧也是「`speed=0` + 纯偏航」，
//! 语义严格对齐，符号**不取反**，§六.2.2）。

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;

use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::state::MotionAction;

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

    fn Apply_Action(&self, action: MotionAction) {
        self.base.Dispatch(action);
    }

    fn Handle_Manual_Cmd(&self, cmd: &ManualCmd) {
        // Forward / Backward / SpinLeft / SpinRight / Stop（5 个运动分支，共用实现）
        if self.base.Send_Manual_Motion(cmd) {
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

    /// 已装配的差速底盘（默认 30/10）
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
    fn test_diff_manual_ignores_payload() {
        // 载荷（i16）一律忽略，速度永远取配置（与真机 forward_speed / turn_speed 一致）
        let (ch, mut rx) = diff();

        ch.Handle_Manual_Cmd(&ManualCmd::Forward(999));
        let m = rx.try_recv().expect("Forward 应下发");
        assert_eq!((m.action, m.speed), ("forward", 30), "载荷 999 不能变成速度");

        ch.Handle_Manual_Cmd(&ManualCmd::Backward(-5));
        let m = rx.try_recv().expect("Backward 应下发");
        assert_eq!((m.action, m.speed), ("backward", 30), "负载荷必须被忽略");

        ch.Handle_Manual_Cmd(&ManualCmd::SpinLeft(1000));
        let m = rx.try_recv().expect("SpinLeft 应下发");
        assert_eq!((m.action, m.speed), ("left", 10));

        ch.Handle_Manual_Cmd(&ManualCmd::SpinRight(0));
        let m = rx.try_recv().expect("SpinRight 应下发");
        assert_eq!((m.action, m.speed), ("right", 10));

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
    fn test_diff_apply_action_wire() {
        // §六.2.1 的表经 apply_action 走到底（动作 → action/speed）
        let (ch, mut rx) = diff();

        ch.Apply_Action(MotionAction::TurnLeft);
        let m = rx.try_recv().expect("TurnLeft 应下发");
        assert_eq!((m.action, m.speed), ("left", 10));

        ch.Apply_Action(MotionAction::MoveForward);
        let m = rx.try_recv().expect("MoveForward 应下发");
        assert_eq!((m.action, m.speed), ("forward", 30));

        ch.Apply_Action(MotionAction::Stop);
        let m = rx.try_recv().expect("Stop 应下发");
        assert_eq!((m.action, m.speed), ("stop", 0));
    }

    #[test]
    fn test_diff_stop_uses_config_and_channel() {
        // `Chassis::Stop` 也是走同一条通道（安全关键路径）
        let (ch, mut rx) = diff();
        ch.Stop();
        let m = rx.try_recv().expect("Stop 应下发");
        assert_eq!((m.action, m.speed), ("stop", 0));
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
