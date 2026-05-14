//Presented by KeJi
//Date ： 2026-05-14

use async_trait::async_trait;
use std::fmt;
use tokio::sync::mpsc;

// ─── 错误类型 ───────────────────────────────────────────────

/// Session 模块错误枚举
#[derive(Debug)]
pub enum Session_Error {
    /// session_id 不存在
    SessionNotFound(String),
    /// 槽位已满（已达 max_slots 上限）
    SlotExhausted(String),
    /// 内部错误
    Internal(String),
}

impl fmt::Display for Session_Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Session_Error::SessionNotFound(id) => write!(f, "SessionNotFound: {}", id),
            Session_Error::SlotExhausted(id) => write!(f, "SlotExhausted: {}", id),
            Session_Error::Internal(msg) => write!(f, "Internal: {}", msg),
        }
    }
}

impl std::error::Error for Session_Error {}

// ─── 通道结构 ───────────────────────────────────────────────

/// 前端持有的外侧端点
#[derive(Debug)]
pub struct IoFrontend {
    /// 发送 Prompt
    pub input_tx: mpsc::Sender<String>,
    /// 接收 Completion
    pub output_rx: mpsc::Receiver<String>,
}

/// ML 侧端点（传给 ML Engine 启动推理线程）
#[derive(Debug)]
pub struct IoHandle {
    /// 接收 Prompt
    pub input_rx: mpsc::Receiver<String>,
    /// 发送 Completion
    pub output_tx: mpsc::Sender<String>,
}

// ─── Trait 定义 ─────────────────────────────────────────────

/// Session 能力 trait
#[async_trait]
pub trait Session_Capability: Send + Sync {
    /// 创建 Session（分配槽位 + channel pair），返回 (session_id, IoHandle)
    /// 调用方自行将 IoHandle 传给 ML Engine 启动线程
    async fn create_session(&self, model_id: String)
        -> Result<(String, IoHandle), Session_Error>;

    /// 销毁 Session，清理所有通道和槽位
    async fn destroy_session(&self, session_id: &str)
        -> Result<(), Session_Error>;

    /// 申请槽位，返回 (slot_id, IoFrontend)
    async fn connect(&self, session_id: &str)
        -> Result<(u32, IoFrontend), Session_Error>;

    /// 列出所有活跃 Session
    fn list_sessions(&self) -> Vec<super::session::SessionInfo>;

    /// 释放槽位（v1 空实现，预留接口）
    async fn release_slot(&self, session_id: &str, slot_id: u32)
        -> Result<(), Session_Error>;
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_error_display() {
        assert_eq!(
            format!("{}", Session_Error::SessionNotFound("sess-1".to_string())),
            "SessionNotFound: sess-1"
        );
        assert_eq!(
            format!("{}", Session_Error::SlotExhausted("sess-1".to_string())),
            "SlotExhausted: sess-1"
        );
    }

    #[tokio::test]
    async fn test_io_endpoints_construct() {
        let (input_tx, input_rx) = mpsc::channel::<String>(64);
        let (output_tx, output_rx) = mpsc::channel::<String>(64);
        let _frontend = IoFrontend { input_tx, output_rx };
        let _ml_side = IoHandle { input_rx, output_tx };
    }
}
