//Presented by KeJi
//Date ： 2026-04-17

//! Control 模块 - 调度核心
//!
//! 负责协调 CLI 用户输入、网络入站请求和网络事件，
//! 向下调用 ML_Service 和 NodeHandle 的接口。
//!
//! ## 子模块
//! - cli_command: CLI 用户交互，读取命令并发送给 Control
//! - network_control_command: 节点间命令协议定义与序列化
//! - control: Control 核心事件循环与调度逻辑
//! - ui_message: UI 消息定义
//! - cli_handler: CLI 命令处理器，专门处理 CLI 命令
//! - network_control_handler: 网络控制命令处理器，专门处理网络控制命令
//! - network_inbound_handler: 网络入站请求处理器，专门处理入站请求

pub mod cli_command;
pub mod network_control_command;
pub mod control;
pub mod ui_message;
pub mod cli_handler;
pub mod network_control_handler;
pub mod network_inbound_handler;

pub use cli_command::CLI_Command;
pub use network_control_command::{Control_Command, Serialize_Command, Deserialize_Command};
pub use control::{Control_Loop, Node_State, Inference_Context};
pub use ui_message::Ui_Message;
pub use cli_handler::{CLI_Handler, CLI_Action};
pub use network_control_handler::NetworkCommandHandler;
pub use network_inbound_handler::NetworkInboundHandler;
