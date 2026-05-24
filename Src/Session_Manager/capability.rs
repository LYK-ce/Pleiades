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

pub struct SlotHandle {
    session_id: u64,
    slot_id: usize,
    prompt_tx: mpsc::UnboundedSender<(u64, usize, String)>,
    close_slot_tx: Option<mpsc::UnboundedSender<(u64, usize)>>,
    token_rx: Option<mpsc::UnboundedReceiver<String>>,
}

impl SlotHandle {
    pub fn new(
        session_id: u64,
        slot_id: usize,
        prompt_tx: mpsc::UnboundedSender<(u64, usize, String)>,
        close_slot_tx: mpsc::UnboundedSender<(u64, usize)>,
        token_rx: mpsc::UnboundedReceiver<String>,
    ) -> Self {
        SlotHandle {
            session_id,
            slot_id,
            prompt_tx,
            close_slot_tx: Some(close_slot_tx),
            token_rx: Some(token_rx),
        }
    }

    pub fn submit(&self, text: String) {
        self.prompt_tx
            .send((self.session_id, self.slot_id, text))
            .ok();
    }

    pub async fn recv_token(&mut self) -> Option<String> {
        match &mut self.token_rx {
            Some(rx) => rx.recv().await,
            None => None,
        }
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub fn slot_id(&self) -> usize {
        self.slot_id
    }

    pub fn take_token_rx(mut self) -> mpsc::UnboundedReceiver<String> {
        self.close_slot_tx.take();
        self.token_rx.take().expect("token_rx already taken")
    }
}

impl Drop for SlotHandle {
    fn drop(&mut self) {
        if let Some(tx) = &self.close_slot_tx {
            tx.send((self.session_id, self.slot_id)).ok();
        }
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
        let (close_tx, _close_rx) = mpsc::unbounded_channel();
        let (token_tx, token_rx) = mpsc::unbounded_channel();

        let mut handle = SlotHandle::new(7, 2, prompt_tx, close_tx, token_rx);
        handle.submit("hello".into());

        let (sid, slot_id, text) = prompt_rx.recv().await.unwrap();
        assert_eq!(sid, 7);
        assert_eq!(slot_id, 2);
        assert_eq!(text, "hello");

        token_tx.send("world".into()).unwrap();
        assert_eq!(handle.recv_token().await, Some("world".into()));
    }

    #[test]
    fn test_slot_handle_drop_sends_close() {
        let (close_tx, mut close_rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (_token_tx, token_rx) = mpsc::unbounded_channel();

        let handle = SlotHandle::new(9, 3, prompt_tx, close_tx, token_rx);
        drop(handle);

        let (sid, slot) = close_rx.try_recv().expect("Drop should send close");
        assert_eq!(sid, 9);
        assert_eq!(slot, 3);
    }

    #[test]
    fn test_take_token_rx_prevents_drop_close() {
        let (close_tx, mut close_rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (_token_tx, token_rx) = mpsc::unbounded_channel();

        let handle = SlotHandle::new(9, 3, prompt_tx, close_tx, token_rx);
        let _rx = handle.take_token_rx();

        assert!(close_rx.try_recv().is_err());
    }
}
