//Presented by KeJi
//Date ： 2026-05-22

//! Session 能力定义 — 错误类型 + SlotHandle

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

/// 返回给接入方的槽位句柄
///
/// 接入方通过 `submit()` 发 prompt，通过 `recv_token()` 收 token。
/// Handle drop 时自动释放槽位。
pub struct SlotHandle {
    pub prompt_tx: mpsc::UnboundedSender<(String, usize, String)>,
    pub token_rx: mpsc::UnboundedReceiver<String>,
}

impl SlotHandle {
    /// 发送 prompt（自动带 session_id + slot_id）
    pub fn submit(&self, session_id: &str, slot_id: usize, text: String) {
        self.prompt_tx
            .send((session_id.to_string(), slot_id, text))
            .ok();
    }

    /// 异步读取下一个 token
    pub async fn recv_token(&mut self) -> Option<String> {
        self.token_rx.recv().await
    }
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
    async fn test_slot_handle_submit_and_recv() {
        let (prompt_tx, mut prompt_rx) = mpsc::unbounded_channel();
        let (token_tx, token_rx) = mpsc::unbounded_channel();

        let mut handle = SlotHandle { prompt_tx, token_rx };
        handle.submit("sess-7", 2, "hello".into());

        let (sid, slot_id, text) = prompt_rx.recv().await.unwrap();
        assert_eq!(sid, "sess-7");
        assert_eq!(slot_id, 2);
        assert_eq!(text, "hello");

        token_tx.send("world".into()).unwrap();
        assert_eq!(handle.token_rx.recv().await, Some("world".into()));
    }
}
