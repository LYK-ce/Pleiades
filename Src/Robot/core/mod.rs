//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-26

//! Robot 核心
//!
//! - command.rs：三层命令定义（Mode / Manual / Auto）
//! - state.rs：全局状态（RobotState + LidarState）
//! - mode.rs：运行模式（Manual / Auto）
//! - mission.rs：任务队列
//! - goal.rs：目标服务（到达检测 + 任务切换 + 寻路）
//! - executor.rs：决策执行器（Task 22_5 方案二：三状态机纯决策，回退 Rust）
//! - robot.rs：Robot::launch() + 主 select! 循环
//! - cluster/：集群数据面（Task 13_1：入站 POSE 消费 + 远端车信息表）
//! - planning/：规划层（assignment 任务分配 + pathfinder D* 寻路，Task 14）

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
