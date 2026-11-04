//Presented by KeJi
//Created Date ： 2026-05-27
//Modified Date ： 2026-11-04

//! OpenAI 兼容 HTTP API 模块
//!
//! 提供 `/v1/models` 和 `/v1/chat/completions` 端点，
//! 对标 OpenAI API 格式，便于第三方工具集成。

pub mod types;
pub mod routes;
pub mod server;
pub mod webui;

pub use server::spawn_api_server;
pub use webui::{ensure_webui_server, publish_detection, webui_publish, StreamDet};
