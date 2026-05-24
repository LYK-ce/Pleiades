//Presented by KeJi
//Date ： 2026-05-24

//! Session 能力定义 — 错误类型 + SlotHandle (v2: 最小化)

use std::fmt;
use tokio::sync::mpsc;

// ─── 错误类型 ───────────────────────────────────────────────

#[derive(Debug)]
pub enum Session_Error {
    SessionNotFound(String),
    SlotExhausted(String),
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

// ─── SlotHandle ─────────────────────────────────────────────

/// v2: Slot 不再有自己的 prompt 通道。
/// 只暴露 token_rx 给接入方读返回 token。
/// prompt 走 Tensor Stream → Session.select! 直接处理。
pub struct SlotHandle {
    pub token_rx: Option<mpsc::UnboundedReceiver<String>>,
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
    }

    #[tokio::test]
    async fn test_slot_handle_token_recv() {
        let (token_tx, token_rx) = mpsc::unbounded_channel();
        let mut handle = SlotHandle { token_rx: Some(token_rx) };

        token_tx.send("hello".into()).unwrap();
        let val = match &mut handle.token_rx {
            Some(rx) => rx.recv().await,
            None => None,
        };
        assert_eq!(val, Some("hello".into()));
    }
}
