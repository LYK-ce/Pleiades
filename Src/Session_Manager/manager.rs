//Presented by KeJi
//Date ： 2026-05-24

//! SessionManager — 会话管理器 (v2: 纯数据结构，无 loop)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::capability::{Session_Error, SlotHandle};
use super::session::Session;

pub struct SessionManager {
    sessions: HashMap<u64, Session>,
    pub stream_hub: Arc<crate::orchestrator::local_tensor_stream::LocalStreamHub>,
    max_slots: usize,
    counter: u64,
}

impl SessionManager {
    pub fn new(max_slots: usize) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(SessionManager {
            sessions: HashMap::new(),
            stream_hub: Arc::new(crate::orchestrator::local_tensor_stream::LocalStreamHub::new()),
            max_slots,
            counter: 1,
        }))
    }

    pub fn create_session(&mut self, model_id: &str) -> u64 {
        let session_id = self.counter;
        self.counter += 1;

        let session = Session::new(session_id, model_id.to_string(), self.max_slots, 1);
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

        let _slot_id = session
            .allocate(token_tx)
            .ok_or_else(|| Session_Error::SlotExhausted(session_id.to_string()))?;

        Ok(SlotHandle {
            token_rx: Some(token_rx),
        })
    }

    pub fn close_slot(&mut self, session_id: u64, slot_id: usize) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.release(slot_id);
        }
    }
}

impl Drop for SlotHandle {
    fn drop(&mut self) {
        // v2: SlotHandle no longer auto-closes via channel.
        // close_slot is called directly by SessionManager.
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> Arc<Mutex<SessionManager>> {
        SessionManager::new(4)
    }

    #[test]
    fn test_create_and_list_sessions() {
        let mgr = make_mgr();
        let id = mgr.lock().unwrap().create_session("qwen3");
        assert!(id > 0);
        assert_eq!(mgr.lock().unwrap().list_sessions().len(), 1);
    }

    #[test]
    fn test_destroy_session() {
        let mgr = make_mgr();
        let id = mgr.lock().unwrap().create_session("qwen3");
        mgr.lock().unwrap().destroy_session(id).unwrap();
        assert!(mgr.lock().unwrap().list_sessions().is_empty());
    }

    #[test]
    fn test_open_slot_exhausted() {
        let mgr = make_mgr();
        let sess_id = mgr.lock().unwrap().create_session("qwen3");

        let mut lock = mgr.lock().unwrap();
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(matches!(lock.allocate_slot(sess_id), Err(Session_Error::SlotExhausted(_))));
    }

    #[test]
    fn test_close_slot_and_reallocate() {
        let mgr = make_mgr();
        let sess_id = mgr.lock().unwrap().create_session("qwen3");

        {
            let mut lock = mgr.lock().unwrap();
            lock.allocate_slot(sess_id).unwrap();
            assert_eq!(lock.list_sessions()[0].occupied_slots, 1);
            lock.close_slot(sess_id, 0);
            assert_eq!(lock.list_sessions()[0].occupied_slots, 0);
        }

        assert!(mgr.lock().unwrap().allocate_slot(sess_id).is_ok());
    }
}
