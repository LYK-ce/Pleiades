//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-08-20

//! 设备抽象层（Task 22）
//!
//! `MotionDevice` 统一「运动设备」的运行期行为（车/机/船等底盘、飞控）。
//! 设备 = 自包含能力单元：各自构造（`start` 不进 trait，参数为各自的 config 类型），
//! 运行期只暴露统一动作 + shutdown。

/// 运动设备统一运行期接口
///
/// - 统一动作：`move_forward` / `move_backward` / `turn_left` / `turn_right` / `stop`
///   （车机语义一致：前进 = 朝车头/机头方向，停 = 停车/悬停）
/// - 生命周期：`shutdown()`
///
/// 底层协议翻译由各实现自己完成（车走 `FUNC_CAR_RUN`，机走 MAVLink）。
/// `start()`（构造）不进 trait——各设备的 start 参数是各自的 config 类型，签名天然不同。
pub trait MotionDevice: Send + Sync {
    fn move_forward(&self, speed: i16) -> Result<(), String>;
    fn move_backward(&self, speed: i16) -> Result<(), String>;
    fn turn_left(&self, rate: i16) -> Result<(), String>;
    fn turn_right(&self, rate: i16) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn shutdown(&self);
}
