//Presented by KeJi
//Date ： 2026-05-22

//! SessionManager — 会话槽位管理器

use std::collections::HashMap;
use tokio::sync::mpsc;

use super::capability::{Session_Error, SlotHandle};
use super::session::Session;
use super::batch::{BatchRequest, BatchResult, assemble_batch, sample_batch};

// ─── 消息类型 ───────────────────────────────────────────────

pub(crate) struct OpenSlotRequest {
    pub session_id: u64,
    pub reply_tx: tokio::sync::oneshot::Sender<Result<SlotHandle, Session_Error>>,
}

pub(crate) struct CreateSessionRequest {
    pub model_id: String,
    pub reply_tx: tokio::sync::oneshot::Sender<u64>,
}

// ─── SessionManagerHandle ───────────────────────────────────

pub struct SessionManagerHandle {
    pub open_slot_tx: mpsc::UnboundedSender<OpenSlotRequest>,
    pub logits_tx: mpsc::Sender<BatchResult>,
}

// ─── SessionManager ─────────────────────────────────────────

pub struct SessionManager {
    pub sessions: HashMap<u64, Session>,

    shared_prompt_tx: mpsc::UnboundedSender<(u64, usize, String)>,
    pub shared_prompt_rx: mpsc::UnboundedReceiver<(u64, usize, String)>,

    open_slot_tx: mpsc::UnboundedSender<OpenSlotRequest>,
    open_slot_rx: mpsc::UnboundedReceiver<OpenSlotRequest>,

    close_slot_tx: mpsc::UnboundedSender<(u64, usize)>,
    close_slot_rx: mpsc::UnboundedReceiver<(u64, usize)>,

    batch_tx: mpsc::Sender<BatchRequest>,
    batch_rx: mpsc::Receiver<BatchRequest>,

    logits_tx: mpsc::Sender<BatchResult>,
    logits_rx: mpsc::Receiver<BatchResult>,

    max_slots: usize,
    session_counter: u64,
    running: bool,
}

impl SessionManager {
    pub fn new(max_slots: usize) -> Self {
        let (shared_prompt_tx, shared_prompt_rx) = mpsc::unbounded_channel();
        let (open_slot_tx, open_slot_rx) = mpsc::unbounded_channel();
        let (close_slot_tx, close_slot_rx) = mpsc::unbounded_channel();
        let (batch_tx, batch_rx) = mpsc::channel(1);
        let (logits_tx, logits_rx) = mpsc::channel(1);

        SessionManager {
            sessions: HashMap::new(),
            shared_prompt_tx,
            shared_prompt_rx,
            open_slot_tx,
            open_slot_rx,
            close_slot_tx,
            close_slot_rx,
            batch_tx,
            batch_rx,
            logits_tx,
            logits_rx,
            max_slots,
            session_counter: 1,
            running: false,
        }
    }

    pub fn open_slot_sender(&self) -> mpsc::UnboundedSender<OpenSlotRequest> {
        self.open_slot_tx.clone()
    }

    pub fn close_slot_sender(&self) -> mpsc::UnboundedSender<(u64, usize)> {
        self.close_slot_tx.clone()
    }

    pub fn take_batch_rx(&mut self) -> mpsc::Receiver<BatchRequest> {
        std::mem::replace(&mut self.batch_rx, mpsc::channel(1).1)
    }

    pub fn logits_tx(&self) -> mpsc::Sender<BatchResult> {
        self.logits_tx.clone()
    }

    pub fn handle(&self) -> SessionManagerHandle {
        SessionManagerHandle {
            open_slot_tx: self.open_slot_tx.clone(),
            logits_tx: self.logits_tx.clone(),
        }
    }

    pub fn create_session(&mut self, model_id: &str) -> u64 {
        let session_id = self.session_counter;
        self.session_counter += 1;

        let session = Session::new(
            session_id,
            model_id.to_string(),
            self.max_slots,
            1, // placeholder eos
        );

        self.sessions.insert(session_id, session);
        session_id
    }

    pub fn destroy_session(&mut self, session_id: u64) -> Result<(), Session_Error> {
        self.sessions.remove(&session_id);
        Ok(())
    }

    pub fn list_sessions(&self) -> Vec<super::session::SessionInfo> {
        self.sessions
            .values()
            .map(|s| super::session::SessionInfo {
                session_id: s.session_id,
                model_id: s.model_id.clone(),
                total_slots: s.max_slots,
                occupied_slots: s.occupied_count(),
            })
            .collect()
    }

    pub fn allocate_slot(&mut self, session_id: u64) -> Result<SlotHandle, Session_Error> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| Session_Error::SessionNotFound(session_id.to_string()))?;

        let (token_tx, token_rx) = mpsc::unbounded_channel();

        let slot_id = session
            .allocate(token_tx)
            .ok_or_else(|| Session_Error::SlotExhausted(session_id.to_string()))?;

        Ok(SlotHandle::new(
            session_id,
            slot_id,
            self.shared_prompt_tx.clone(),
            self.close_slot_tx.clone(),
            token_rx,
        ))
    }

    pub fn close_slot(&mut self, session_id: u64, slot_id: usize) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.release(slot_id);
        }
    }

    pub fn handle_prompt(&mut self, session_id: u64, slot_id: usize, text: &str) {
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => return,
        };

        let slot = match session.get_slot_mut(slot_id) {
            Some(s) => s,
            None => return,
        };

        let tokens: Vec<u32> = text.bytes().map(|b| b as u32).collect();
        slot.token_buf.extend(tokens);
        slot.dirty = true;
    }

    async fn flush_and_dispatch(&mut self) {
        let mut dirty: Vec<(u64, usize, Vec<u32>)> = Vec::new();
        for (sess_id, session) in self.sessions.iter() {
            for (slot_id, state) in session.slots.iter().enumerate() {
                if let super::slot::SlotState::Occupied(ref slot) = state {
                    if slot.dirty && !slot.token_buf.is_empty() {
                        dirty.push((*sess_id, slot_id, slot.token_buf.clone()));
                    }
                }
            }
        }

        if let Some(batch) = assemble_batch(&dirty) {
            if self.batch_tx.send(batch).await.is_ok() {
                for (sess_id, slot_id, _) in &dirty {
                    if let Some(session) = self.sessions.get_mut(sess_id) {
                        if let Some(slot) = session.get_slot_mut(*slot_id) {
                            slot.token_buf.clear();
                            slot.dirty = false;
                        }
                    }
                }
            }
        }
    }

    async fn dispatch_logits(&mut self, result: BatchResult) {
        let mut temperatures = std::collections::HashMap::new();
        for (sess_id, slot_id) in &result.slot_order {
            if let Some(session) = self.sessions.get(sess_id) {
                if let Some(slot) = session.get_slot(*slot_id) {
                    temperatures.insert((*sess_id, *slot_id), slot.temperature);
                }
            }
        }

        let tokens = sample_batch(&result, &temperatures);

        for ((sess_id, slot_id), token) in tokens {
            let text = format!("[{}]", token);

            if let Some(session) = self.sessions.get_mut(&sess_id) {
                if let Some(slot) = session.get_slot_mut(slot_id) {
                    slot.token_tx.send(text).ok();
                    slot.token_buf.push(token);
                }
            }
        }
    }

    pub async fn run(mut self) {
        self.running = true;
        let mut flush_timer = tokio::time::interval(std::time::Duration::from_millis(100));

        loop {
            if !self.running {
                break;
            }

            tokio::select! {
                Some(req) = self.open_slot_rx.recv() => {
                    let result = self.allocate_slot(req.session_id);
                    req.reply_tx.send(result).ok();
                }

                Some((session_id, slot_id, text)) = self.shared_prompt_rx.recv() => {
                    self.handle_prompt(session_id, slot_id, &text);
                }

                _ = flush_timer.tick() => {
                    self.flush_and_dispatch().await;
                }

                Some(result) = self.logits_rx.recv() => {
                    self.dispatch_logits(result).await;
                }

                Some((session_id, slot_id)) = self.close_slot_rx.recv() => {
                    self.close_slot(session_id, slot_id);
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.running = false;
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> SessionManager {
        SessionManager::new(4)
    }

    #[test]
    fn test_create_and_list_sessions() {
        let mut mgr = make_mgr();
        let id = mgr.create_session("qwen3");
        assert!(id > 0);

        let list = mgr.list_sessions();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_id, id);
        assert_eq!(list[0].model_id, "qwen3");
        assert_eq!(list[0].total_slots, 4);
        assert_eq!(list[0].occupied_slots, 0);
    }

    #[test]
    fn test_destroy_session() {
        let mut mgr = make_mgr();
        let id = mgr.create_session("qwen3");
        mgr.destroy_session(id).unwrap();
        assert!(mgr.list_sessions().is_empty());
    }

    #[test]
    fn test_open_slot_success() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        let handle = mgr.allocate_slot(sess_id).expect("should allocate");
        handle.submit("hello".into());
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 1);
    }

    #[test]
    fn test_open_slot_session_not_found() {
        let mut mgr = make_mgr();
        let result = mgr.allocate_slot(999);
        assert!(matches!(result, Err(Session_Error::SessionNotFound(_))));
    }

    #[test]
    fn test_open_slot_exhausted() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        assert!(mgr.allocate_slot(sess_id).is_ok());
        assert!(mgr.allocate_slot(sess_id).is_ok());
        assert!(mgr.allocate_slot(sess_id).is_ok());
        assert!(mgr.allocate_slot(sess_id).is_ok());
        assert!(matches!(
            mgr.allocate_slot(sess_id),
            Err(Session_Error::SlotExhausted(_))
        ));
    }

    #[test]
    fn test_close_slot_and_reallocate() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        mgr.allocate_slot(sess_id).unwrap();
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 1);

        mgr.close_slot(sess_id, 0);
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 0);

        assert!(mgr.allocate_slot(sess_id).is_ok());
    }

    #[test]
    fn test_handle_prompt_appends_tokens() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");
        mgr.allocate_slot(sess_id).unwrap();

        mgr.handle_prompt(sess_id, 0, "hi");

        let session = mgr.sessions.get(&sess_id).unwrap();
        let slot = session.get_slot(0).unwrap();
        assert!(!slot.token_buf.is_empty());
        assert!(slot.dirty);
    }

    #[tokio::test]
    async fn test_flush_assembles_and_sends_batch() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");
        mgr.allocate_slot(sess_id).unwrap();
        mgr.allocate_slot(sess_id).unwrap();

        mgr.handle_prompt(sess_id, 0, "a");
        mgr.handle_prompt(sess_id, 1, "bc");

        let mut batch_rx = mgr.take_batch_rx();
        mgr.flush_and_dispatch().await;

        let req = batch_rx.try_recv().expect("should receive batch");
        assert_eq!(req.slot_order.len(), 2);
        let session = mgr.sessions.get(&sess_id).unwrap();
        assert!(session.get_slot(0).unwrap().token_buf.is_empty());
        assert!(session.get_slot(1).unwrap().token_buf.is_empty());
    }

    #[tokio::test]
    async fn test_dispatch_logits_sends_tokens_to_slots() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");
        mgr.allocate_slot(sess_id).unwrap();
        mgr.allocate_slot(sess_id).unwrap();

        let result = BatchResult {
            logits_batches: vec![
                vec![0.1, 0.9, 0.0],
                vec![0.5, 0.1, 0.4],
            ],
            slot_order: vec![
                (sess_id, 0),
                (sess_id, 1),
            ],
        };

        {
            let session = mgr.sessions.get_mut(&sess_id).unwrap();
            session.get_slot_mut(0).unwrap().temperature = 0.01;
            session.get_slot_mut(1).unwrap().temperature = 0.01;
        }

        mgr.dispatch_logits(result).await;

        let session = mgr.sessions.get(&sess_id).unwrap();
        assert_eq!(session.get_slot(0).unwrap().token_buf, vec![1]);
        assert_eq!(session.get_slot(1).unwrap().token_buf, vec![0]);
    }

    #[tokio::test]
    async fn test_flush_no_dirty_slots_sends_nothing() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        let mut batch_rx = mgr.take_batch_rx();
        mgr.flush_and_dispatch().await;

        assert!(batch_rx.try_recv().is_err());
    }
}
