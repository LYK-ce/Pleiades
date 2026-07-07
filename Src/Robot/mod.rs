//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Robot 模块 —— 机器人控制
//!
//! - state.rs：全局状态
//! - control/serial/：通用串口抽象
//! - control/device/：各设备驱动
//! - server.rs：WebSocket 遥控服务

pub mod state;
pub mod control;
pub mod server;

use std::sync::Arc;
use tokio::sync::OnceCell;

pub use control::device::stm32::STM32Device;
pub use control::types::CarType;
pub use state::RobotState;

/// 全局 STM32 设备实例
static GLOBAL_STM32: OnceCell<Arc<STM32Device>> = OnceCell::const_new();

pub fn init_stm32(device: Arc<STM32Device>) {
    let _ = GLOBAL_STM32.set(device);
}

pub fn get_stm32() -> Option<Arc<STM32Device>> {
    GLOBAL_STM32.get().cloned()
}
