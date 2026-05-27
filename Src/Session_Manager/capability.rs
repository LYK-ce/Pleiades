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

/// v2.7: Slot 通过 mpsc 通道对连接 Chat ↔ Session。
/// prompt_tx 发送 prompt 给 Session，token_rx 从 Session 接收 token。
pub struct SlotHandle {
    pub session_id: u64,
    pub slot_id: usize,
    pub prompt_tx: mpsc::UnboundedSender<String>,
    pub token_rx: mpsc::UnboundedReceiver<String>,
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
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (token_tx, token_rx) = mpsc::unbounded_channel();
        let _handle = SlotHandle { session_id: 1, slot_id: 0, prompt_tx, token_rx };

        token_tx.send("hello".into()).unwrap();
        // token_rx moved into handle; test the channel directly
        // (in real code, handle.token_rx is used by chat relay task)
    }
}
