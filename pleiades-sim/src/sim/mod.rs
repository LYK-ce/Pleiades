//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-21

//! SIM 设备端模块（无硬件）
//!
//! - sim_handler.rs：SimDeviceHandler（实现 base 的 DeviceHandler；动作交底盘）
//! - goal.rs / executor.rs / planning/：复制车 2D 决策（寻路 + 三状态机）
//! - chassis/：底盘层（Step 4）——`Chassis` trait + 动作翻译（唯一入口）+ 速度绑定 +
//!   位置更新（`pose` → `RobotState`，**底盘是唯一世界写者**）+ 差速/四旋翼两个实现
//! - sensors/：传感器层（本 step 只有 lidar_2d）——`Sensor` trait / 工厂 / 连接端点 /
//!   激光建图 task。**写 `robot.grid` 的是传感器，不是决策链**
//! - emergency_stop.rs：前方障碍急停（Step 6）——`Check_Emergency_Stop`（与真机同语义，
//!   代码独立）+ `Obstacle_Cell`（障碍点 → 网格坐标）。**每 tick 复核、不锁存**
//! - control.rs：本地控制口（Step 5）——测试夹具，把请求翻成 `Command` 注入
//!   `robot.robot_cmd_tx`（与地面站同一条下游），**不进决策链、不走 ORION 帧**

pub mod chassis;
pub mod control;
pub mod emergency_stop;
pub mod executor;
pub mod goal;
pub mod planning;
pub mod sensors;
pub mod sim_handler;
