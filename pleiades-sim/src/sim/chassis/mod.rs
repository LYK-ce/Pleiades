//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-11-01

#![allow(non_snake_case)]

//! 底盘层（task_31 §六，task_44 §1.3 起统一连续速度）：速度绑定 / 位姿更新
//!
//! 与真机同形（§六.1）：真机的底盘设备有 `apply_action` / `handle_manual_cmd` 两个入口，
//! 并且 `RobotState` 的 `x/y/z/yaw/vx/vy` 是「底盘 RX 回调」写的——这三件事在 sim 里由
//! 同一个对象承担，就是本模块的 [`Chassis`]。
//!
//! 唯一设备接口 = [`Chassis::Apply_Velocity`]（`v` m/s / `omega` rad/s）：**车与机统一**走这一条
//! （§1.2 #1「离散动作整体删除，不是再加一条通道并存」）。`ManualCmd::Motion { v, omega }`
//! 由 [`ChassisBase::Dispatch_Manual`] 原值转交同一个方法。
//!
//! 三个角色：
//! - [`Chassis`]：唯一设备接口（`Name` / `Apply_Velocity` / `Handle_Manual_Cmd` / `On_Pose` /
//!   `On_Spawn` / `Pose_Age_Ms` / `Pose_Age_Handle` / `Stop`）
//! - [`ChassisBase`]：差速与四旋翼**共用**的字段与公共实现（手动运动命令、位姿写回、新鲜度），
//!   两个具体底盘各持有一个并逐方法转调（§六.5.3 的偏差 D）
//! - [`Build`]：按 `[sim_world].chassis`（世界零件名）构造，未知名字 → `Err`
//!
//! 硬约束：**禁止引用 pleiades-ugv / pleiades-uav**（§六.3-1）。速度绑定、位姿写回
//! 全部在本模块内自带；允许 `pleiades-base`（`ManualCmd` / `RobotState`）。
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
use pleiades_base::robot::core::state::RobotState;

use crate::config::ChassisConfig;
// 对外 re-export：`world_link` 只关心 `Chassis` + 两个字段载体，不必知道内部模块路径
pub use crate::sim::chassis::action_map::SpeedBinding;
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

    /// 连续速度指令（task_39 §四.4，task_44 §1.3 起**车机统一**）：`v` 单位 **m/s**、
    /// `omega` 单位 **rad/s**（物理量，`omega > 0` = 左转）。
    ///
    /// 两个底盘都实现它（差速 `DiffDrive.gd` / 四旋翼 `Quadrotor.gd` 都直接收物理量）；
    /// 默认 `false` 只作「未实现的底盘」兜底 —— 返回 `true` = 本底盘**支持**这路指令
    /// （**不代表已送达**：未装配时 `Send_Velocity` 只 `debug!` 丢弃并返回 `Ok(())`）。
    fn Apply_Velocity(&self, _v: f32, _omega: f32) -> bool {
        false
    }

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
    /// 速度绑定（`[chassis]` → 两个物理量：`cruise_speed` m/s / `cruise_yaw_rate` rad/s）
    pub binding: SpeedBinding,
    /// 动作上行口（supervisor 一侧收 `mpsc::Receiver<protocol::MotionCmd>`）
    pub motion: MotionSender,
    /// 位姿写入口（`RobotState` 的唯一世界写者）
    pub pose: PoseIngest,
}

impl ChassisBase {
    pub fn New(
        name: &str,
        binding: SpeedBinding,
        motion: MotionSender,
        robot_state: Arc<RwLock<RobotState>>,
    ) -> Self {
        Self { name: name.to_string(), binding, motion, pose: PoseIngest::New(robot_state) }
    }

    /// `ManualCmd` 的运动分支（`Stop` 走 `Send_Stop_Priority`）——差速/四旋翼共用
    ///
    /// 返回 `true` = 已处理（调用方不必再看别的分支）。
    /// 运动只有一条：`Motion { v, omega }` → [`Self::Send_Velocity`]（**原值透传**，
    /// 世界侧直接收物理量；不再有 forward / spin 之类的离散分支）。
    pub fn Dispatch_Manual(&self, cmd: &ManualCmd) -> bool {
        match cmd {
            ManualCmd::Motion { v, omega } => {
                if Should_Log("chassis.manual", 1000) {
                    // 一并打出配置绑定：手动是原值透传，绑定只说明「自动档会用多少」
                    info!(
                        "[chassis] {} 手动 Motion → v={} m/s omega={} rad/s（绑定 cruise_speed={:.2} cruise_yaw_rate={:.2}）",
                        self.name, v, omega, self.binding.cruise_speed, self.binding.cruise_yaw_rate
                    );
                }
                self.Send_Velocity(*v, *omega);
                true
            }
            ManualCmd::Stop => {
                // G1：停车走优先通道（满则登记补发，绝不静默丢）
                if let Err(e) = self.motion.Send_Stop_Priority() {
                    debug!("[chassis] {} stop 未投递（已登记补发）：{e}", self.name);
                }
                true
            }
            _ => return false,
        }
    }

    /// 连续速度指令（`action="velocity"` + `v` / `omega` 物理量）：`Err` 只 `debug!`
    /// （未装配 / 通道满），50 ms 下一拍自愈，不在这里升级成错误
    pub(crate) fn Send_Velocity(&self, v: f32, omega: f32) {
        if let Err(e) = self.motion.Send_Velocity(v, omega) {
            debug!("[chassis] {} 速度指令未投递：{e}", self.name);
        }
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
    let binding = SpeedBinding::From_Config(Some(cfg));
    let base = ChassisBase::New(name, binding, motion, robot_state);
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
        "[chassis] 构建底盘 name={} cruise_speed={:.2} m/s cruise_yaw_rate={:.2} rad/s",
        chassis.Name(),
        binding.cruise_speed,
        binding.cruise_yaw_rate
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
        ChassisConfig { enabled: Some(true), cruise_speed: None, cruise_yaw_rate: None }
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
    fn test_build_passes_cruise_through_config() {
        // 配置里的两个物理量必须一路透传到 wire（0.55 / 0.7，而不是缺省 0.45 / 0.5）
        let (tx, mut rx) = channel();
        let c = ChassisConfig { enabled: Some(true), cruise_speed: Some(0.55), cruise_yaw_rate: Some(0.7) };
        let ch = Build("diff_drive_ugv", &c, tx, state()).expect("能建");
        ch.Apply_Velocity(0.55, 0.7);
        let m = rx.try_recv().expect("velocity 应下发");
        assert_eq!(m.action, "velocity");
        assert_eq!((m.v, m.omega), (Some(0.55), Some(0.7)));
        assert_eq!(m.speed, 0, "速度帧不带百分比档位（speed 恒 0）");
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

    #[test]
    fn test_apply_velocity_supported_by_both_chassis() {
        // task_44 §1.2 #1：车与机统一走 `Apply_Velocity(v, omega)` —— 两个底盘都必须支持，
        // 且各自原值下发一帧 `action="velocity"`（这就是「统一」的证据）
        let (tx, mut rx) = channel();
        let diff = Build("diff_drive_ugv", &cfg(), tx.clone(), state()).expect("差速能建");
        assert!(diff.Apply_Velocity(0.45, -0.5), "差速底盘必须支持 Apply_Velocity");
        let m = rx.try_recv().expect("差速速度帧应下发");
        assert_eq!(m.action, "velocity");
        assert_eq!((m.v, m.omega), (Some(0.45), Some(-0.5)));

        let quad = Build("quadrotor_uav", &cfg(), tx, state()).expect("四旋翼能建");
        assert!(quad.Apply_Velocity(0.45, -0.5), "四旋翼必须支持 Apply_Velocity");
        let m = rx.try_recv().expect("四旋翼速度帧应下发");
        assert_eq!(m.action, "velocity");
        assert_eq!((m.v, m.omega), (Some(0.45), Some(-0.5)));
    }
}
