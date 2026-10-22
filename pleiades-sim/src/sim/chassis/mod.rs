//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 底盘层（task_31 §六）：动作翻译 / 速度绑定 / 位置更新
//!
//! 与真机同形（§六.1）：真机的底盘设备有 `apply_action` / `handle_manual_cmd` 两个入口，
//! 并且 `RobotState` 的 `x/y/z/yaw/vx/vy` 是「底盘 RX 回调」写的——这三件事在 sim 里由
//! 同一个对象承担，就是本模块的 [`Chassis`]。
//!
//! 三个角色：
//! - [`Chassis`]：唯一设备接口（`Name` / `Apply_Action` / `Handle_Manual_Cmd` / `On_Pose` /
//!   `On_Spawn` / `Pose_Age_Ms` / `Pose_Age_Handle` / `Stop`）
//! - [`ChassisBase`]：差速与四旋翼**共用**的字段与公共实现（动作分发、手动运动命令、
//!   位姿写回、新鲜度），两个具体底盘各持有一个并逐方法转调（§六.5.3 的偏差 D）
//! - [`Build`]：按 `[sim_world].chassis`（世界零件名）构造，未知名字 → `Err`
//!
//! 硬约束：**禁止引用 pleiades-ugv / pleiades-uav**（§六.3-1）。翻译表、速度绑定、位姿写回
//! 全部在本模块内自带；允许 `pleiades-base`（`MotionAction` / `ManualCmd` / `RobotState`）。
//!
//! 位置更新的单写者纪律（§六.2.5）：`RobotState` 只被 [`pose_ingest::PoseIngest`] 写，
//! 而 `PoseIngest` **归底盘持有**；`world_link` 只调 `Chassis::On_Pose` / `Chassis::On_Spawn`。

pub mod action_map;
pub mod pose_ingest;
pub mod uav;
pub mod ugv;

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{debug, info};

use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::state::{MotionAction, RobotState};

use crate::config::ChassisConfig;
// 对外 re-export：`world_link` 只关心 `Chassis` + 两个字段载体，不必知道内部模块路径
pub use crate::sim::chassis::action_map::{Action_To_Wire, SpeedBinding};
pub use crate::sim::chassis::pose_ingest::{PoseFields, SpawnFields};
use crate::sim::chassis::pose_ingest::{PoseHistory, PoseIngest};
use crate::sim::sensors::{Should_Log, Warn_Throttled};
use crate::world_link::MotionSender;

pub use uav::QuadrotorChassis;
pub use ugv::DiffDriveChassis;

/// 底盘设备接口（§六.5.3，仅多一个 `Pose_Age_Handle` —— 见 `tmp/task31/plan.md` 偏差 A）
///
/// `Pose_Age_Handle` 存在的原因是 Step 3 的建图 task 需要的是 `Arc<AtomicU64>`（`MappingCtx.pose_age`），
/// 而 `Pose_Age_Ms()` 只给数值；两者必须是**同一个** `Arc`，否则建图永远不新鲜。
#[async_trait]
pub trait Chassis: Send + Sync {
    /// 世界零件名（`diff_drive_ugv` / `quadrotor_uav`）
    fn Name(&self) -> &str;

    /// 与 `STM32Device::apply_action` / `MavlinkDevice::apply_action` 同形（同步、无返回值）
    fn Apply_Action(&self, action: MotionAction);

    /// 与真机 `handle_manual_cmd` 同形（设备选择性响应：车不响应 Takeoff/Land）
    fn Handle_Manual_Cmd(&self, cmd: &ManualCmd);

    /// 位置更新（真机是底盘 RX 回调写 `RobotState`）——**单写者**
    async fn On_Pose(&self, p: &PoseFields);

    /// 出生点采纳（世界按 `avatar_id` 把车摆在错开的方位）
    async fn On_Spawn(&self, s: &SpawnFields);

    /// 距上次 pose 的毫秒数（未收到过 = `u64::MAX`）
    fn Pose_Age_Ms(&self) -> u64;

    /// 与建图 task 共用的同一个时钟（`MappingCtx.pose_age`）
    fn Pose_Age_Handle(&self) -> Arc<AtomicU64>;

    /// 与建图 task 共用的同一条位姿历史（`MappingCtx.pose_history`）
    fn Pose_History(&self) -> Arc<PoseHistory>;

    /// 停车（安全关键，走 G1 优先通道：通道满则登记补发）
    fn Stop(&self);
}

/// 差速 / 四旋翼共用的底盘内核（字段与公共实现只写一份）
pub struct ChassisBase {
    /// 世界零件名
    pub name: String,
    /// 速度绑定（`[chassis]` → `clamp(0,100)`）
    pub speed: SpeedBinding,
    /// 动作上行口（supervisor 一侧收 `mpsc::Receiver<protocol::MotionCmd>`）
    pub motion: MotionSender,
    /// 位姿写入口（`RobotState` 的唯一世界写者）
    pub pose: PoseIngest,
}

impl ChassisBase {
    pub fn New(
        name: &str,
        speed: SpeedBinding,
        motion: MotionSender,
        robot_state: Arc<RwLock<RobotState>>,
    ) -> Self {
        Self { name: name.to_string(), speed, motion, pose: PoseIngest::New(robot_state) }
    }

    /// `Action_To_Wire` → `MotionSender::Send`（未投递只 `debug!`，绝不 panic）
    ///
    /// 限流 `info!`（1 s 最多一条）是「每条下发的 (action, speed) 与配置一致」的可见证据：
    /// `[chassis] diff_drive_ugv MoveForward → action=forward speed=30（binding forward=30 turn=10）`
    pub fn Dispatch(&self, action: MotionAction) {
        let w = Action_To_Wire(action, &self.speed);
        if Should_Log("chassis.dispatch", 1000) {
            info!(
                "[chassis] {} {action:?} → action={} speed={}（binding forward={} turn={}）",
                self.name, w.action, w.speed, self.speed.forward, self.speed.turn
            );
        }
        if let Err(e) = self.motion.Send(w.action, w.speed) {
            debug!("[chassis] {} 动作未投递：{e}", self.name);
        }
    }

    /// `ManualCmd` 的 5 个运动分支（`Stop` 走 `Send_Stop_Priority`）——差速/四旋翼共用
    ///
    /// 返回 `true` = 已处理（调用方不必再看别的分支）。
    /// `Forward(_)` / `Backward(_)` / `SpinLeft(_)` / `SpinRight(_)` 的**载荷一律忽略**，
    /// 速度用配置（与真机 `stm32/mod.rs:134-147` 一致）。
    pub fn Send_Manual_Motion(&self, cmd: &ManualCmd) -> bool {
        let wire = match cmd {
            ManualCmd::Forward(_) => ("forward", self.speed.forward),
            ManualCmd::Backward(_) => ("backward", self.speed.forward),
            ManualCmd::SpinLeft(_) => ("left", self.speed.turn),
            ManualCmd::SpinRight(_) => ("right", self.speed.turn),
            ManualCmd::Stop => {
                // G1：停车走优先通道（满则登记补发，绝不静默丢）
                if let Err(e) = self.motion.Send_Stop_Priority() {
                    debug!("[chassis] {} stop 未投递（已登记补发）：{e}", self.name);
                }
                return true;
            }
            _ => return false,
        };
        if Should_Log("chassis.manual", 1000) {
            info!(
                "[chassis] {} 手动 {cmd:?} → action={} speed={}（载荷忽略，用配置速度）",
                self.name, wire.0, wire.1
            );
        }
        if let Err(e) = self.motion.Send(wire.0, wire.1) {
            debug!("[chassis] {} 手动命令未投递：{e}", self.name);
        }
        true
    }

    /// `Beep`：世界没有蜂鸣指令 → **不下发**，每进程只 `warn!` 一次
    pub fn Warn_Beep(&self, key: &'static str, cmd: &ManualCmd) {
        Warn_Throttled(key, u64::MAX, || {
            format!(
                "[chassis] {} 收到 {cmd:?}：世界无蜂鸣指令，忽略（不下发任何 motion_cmd）",
                self.name
            )
        });
    }
}

/// 按 `[sim_world].chassis`（世界零件名）构造；未知名字 → `Err`（不让世界踩 1002）
pub fn Build(
    world_name: &str,
    cfg: &ChassisConfig,
    motion: MotionSender,
    robot_state: Arc<RwLock<RobotState>>,
) -> Result<Arc<dyn Chassis>, String> {
    let name = world_name.trim();
    // 名字唯一来源是世界目录镜像（catalog），不在这里另抄一份字符串表
    crate::world_link::catalog::Validate_Chassis(name)?;
    let speed = SpeedBinding::From_Config(Some(cfg));
    let base = ChassisBase::New(name, speed, motion, robot_state);
    let chassis: Arc<dyn Chassis> = match name {
        "diff_drive_ugv" => Arc::new(DiffDriveChassis::New(base)),
        "quadrotor_uav" => Arc::new(QuadrotorChassis::New(base)),
        other => {
            return Err(format!(
                "未知底盘 '{other}'；世界目录只有 {:?}",
                crate::world_link::catalog::CHASSIS
            ))
        }
    };
    info!(
        "[chassis] 构建底盘 name={} forward_speed={} turn_speed={}",
        chassis.Name(),
        speed.forward,
        speed.turn
    );
    Ok(chassis)
}

// ================================================================ 单测（工厂 + 公共内核）

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn state() -> Arc<RwLock<RobotState>> {
        Arc::new(RwLock::new(RobotState::default()))
    }

    fn cfg() -> ChassisConfig {
        ChassisConfig { enabled: Some(true), forward_speed: None, turn_speed: None }
    }

    /// 已装配的通道（底盘动作要真的进通道才能断言）
    fn channel() -> (MotionSender, mpsc::Receiver<crate::world_link::protocol::MotionCmd>) {
        let (tx, rx) = MotionSender::Channel(crate::world_link::MOTION_CHANNEL_CAP);
        tx.Set_Assembled(true);
        (tx, rx)
    }

    #[test]
    fn test_build_diff_and_quad() {
        let (tx, _rx) = channel();
        let c = Build("diff_drive_ugv", &cfg(), tx.clone(), state()).expect("差速必须能建");
        assert_eq!(c.Name(), "diff_drive_ugv");

        let c = Build("quadrotor_uav", &cfg(), tx, state()).expect("四旋翼必须能建");
        assert_eq!(c.Name(), "quadrotor_uav");
    }

    #[test]
    fn test_build_rejects_unknown() {
        let (tx, _rx) = channel();
        for bad in ["tank", "", "X3Plus"] {
            let e = Build(bad, &cfg(), tx.clone(), state()).err().expect("未知名字必须 Err");
            assert!(e.contains(bad.trim()), "错误信息缺传入名字：{e}");
            assert!(e.contains("diff_drive_ugv"), "错误信息缺可用目录：{e}");
        }
    }

    #[test]
    fn test_build_uses_config_speeds() {
        // 配置里的档位必须一路透传到 wire（55 / 7，而不是默认 30 / 10）
        let (tx, mut rx) = channel();
        let c = ChassisConfig { enabled: Some(true), forward_speed: Some(55), turn_speed: Some(7) };
        let ch = Build("diff_drive_ugv", &c, tx, state()).expect("能建");
        ch.Apply_Action(MotionAction::MoveForward);
        let m = rx.try_recv().expect("forward 应下发");
        assert_eq!((m.action, m.speed), ("forward", 55));
        ch.Apply_Action(MotionAction::TurnLeft);
        let m = rx.try_recv().expect("left 应下发");
        assert_eq!((m.action, m.speed), ("left", 7));
    }

    #[tokio::test]
    async fn test_chassis_pose_ingest_shared_with_mapping_clock() {
        // 底盘持有的 PoseIngest 与建图 task 的 pose_age 必须是同一个 Arc
        let (tx, _rx) = channel();
        let ch = Build("diff_drive_ugv", &cfg(), tx, state()).expect("能建");
        let handle = ch.Pose_Age_Handle();
        assert_eq!(
            crate::sim::chassis::pose_ingest::Age_Of(&handle),
            u64::MAX,
            "初始应永不新鲜"
        );
        ch.On_Pose(&PoseFields { world_time: 1.0, x: 66.0, y: 63.0, z: 0.0, yaw: 0.5, vx: 0.1, vy: 0.2 })
            .await;
        assert!(ch.Pose_Age_Ms() < 1000, "写 pose 后底盘自己判新鲜");
        assert!(
            crate::sim::chassis::pose_ingest::Age_Of(&handle) < 1000,
            "建图 task 的 pose_age 必须同步变新鲜（同一个 Arc）"
        );
    }
}
