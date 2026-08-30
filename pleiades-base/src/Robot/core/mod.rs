//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-26

//! Robot 核心（base 共享底座）
//!
//! - command.rs：三层命令定义（Mode / Manual / Auto）
//! - command_consumer.rs：命令入站消费（ORION 帧 → Command）
//! - state.rs：位姿 schema（RobotState / ExecuteState / DecisionState）
//! - mode.rs：运行模式（Manual / Auto）
//! - mission.rs：任务队列
//! - robot.rs：Robot::new() + DeviceHandler trait + 主 select! 循环骨架
//! - protocol/：ORION 帧编解码 + 消息
//! - cluster/：集群数据面（入站 POSE 消费 + 远端车信息表）
//! - world.rs：世界（grid + cluster，纯地图只读接口）
//! - grid.rs：占据栅格（OccupancyGrid）
//! - map_delta.rs：地图增量协议类型

pub mod command;
pub mod grid;
pub mod map_delta;
pub mod command_consumer;
pub mod protocol;
pub mod state;
pub mod mission;
pub mod mode;
pub mod robot;
pub mod cluster;
pub mod world;
