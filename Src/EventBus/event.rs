//Presented by KeJi
//Date ： 2026-05-19

//! EventBus 事件类型定义
//!
//! 定义 `Bus_Event` 枚举，包含 4 种通用事件类型，通过 JSON payload 解耦
//! 领域细节。新增模块无需修改 EventBus 定义。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// ============================================================
// Bus_Event — 4 种通用事件类型
// ============================================================

/// EventBus 事件。与具体模块/面板解耦，payload 采用 JSON 字符串。
///
/// ## 类型语义
/// - `Notify` — 一次性通知（日志、错误、设备切换等）
/// - `State`  — 持久状态变更（peer 增删、job 状态机等）
/// - `Stream` — 高频流式推送（token 输出、文件进度等）
/// - `Output` — 命令输出（命令结果、帮助信息等）
///
/// ## 设计原则
/// - EventBus 不感知 Peer / Job / ML 等任何领域概念
/// - 新增模块只需构造对应的 JSON payload，无需修改此枚举
/// - 消费者自己解析 `type` 字段路由到具体 handler
#[derive(Debug, Clone)]
pub enum Bus_Event {
    /// 一次性通知（对应原 Log / Error / Device_Changed）
    Notify {
        level: NotifyLevel,
        message: String,
    },
    /// 持久状态变更（对应原 Peer / Job / Inference 生命周期事件）
    State {
        /// JSON payload，含 "type" 字段用于路由
        payload: String,
    },
    /// 高频流式推送（对应原 Inference_Token / File_Progress）
    Stream {
        /// JSON payload，含 "type" 字段用于路由
        payload: String,
    },
    /// 高频流式推送——原始字节（二进制帧专用，如 ORION 协议帧）
    /// 2026-08-07：robot_bus 二进制承载（新协议前置依赖）；File_Stream 等 JSON 使用者仍走 Stream
    StreamRaw {
        /// 原始字节 payload（不经过 UTF-8/JSON 处理）
        payload: Vec<u8>,
    },
    /// 命令输出（对应原 CommandResult / HelpInfo）
    Output {
        /// JSON payload，含 "type" 字段用于路由
        payload: String,
    },
}

// ============================================================
// NotifyLevel — 通知级别
// ============================================================

/// 通知级别（仅用于 Notify 事件）
#[derive(Debug, Clone)]
pub enum NotifyLevel {
    Info,
    Warn,
    Error,
}
