//Presented by KeJi
//Date ： 2026-04-07

//! Control 模块 - 调度核心
//!
//! 负责协调 CLI 用户输入、网络入站请求和网络事件，
//! 向下调用 ML_Service 和 NodeHandle 的接口。
//!
//! ## 子模块
//! - cli_command: CLI 用户交互，读取命令并发送给 Control
//! - network_control_command: 节点间命令协议定义与序列化
//! - control: Control 核心事件循环与调度逻辑

pub mod cli_command;
pub mod network_control_command;
pub mod control;
pub mod ui_message;

pub use cli_command::CLI_Command;
pub use network_control_command::{Control_Command, Serialize_Command, Deserialize_Command};
pub use control::{Control_Loop, Node_State};
pub use ui_message::Ui_Message;
