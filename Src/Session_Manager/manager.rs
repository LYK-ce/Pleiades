//Presented by KeJi
//Date ： 2026-05-24

//! SessionManager — 会话管理器 (v2: 纯数据结构，无 loop)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::capability::{SessionRequest, Session_Error, SlotHandle};
use super::session::Session;
use crate::storage::StorageCapability;

pub struct SessionManager {
    sessions: HashMap<u64, Session>,
    pub stream_hub: Arc<crate::orchestrator::local_tensor_stream::LocalStreamHub>,
    pub event_bus: Arc<crate::event_bus::EventBus>,
    storage: Arc<dyn StorageCapability>,
    max_slots: usize,
    counter: u64,
}

impl SessionManager {
    pub fn new(
        max_slots: usize,
        stream_hub: Arc<crate::orchestrator::local_tensor_stream::LocalStreamHub>,
        event_bus: Arc<crate::event_bus::EventBus>,
        storage: Arc<dyn StorageCapability>,
    ) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(SessionManager {
            sessions: HashMap::new(),
            stream_hub,
            event_bus,
            storage,
            max_slots,
            counter: 1,
        }))
    }

    pub fn create_session(&mut self, model_id: &str) -> u64 {
        let session_id = self.counter;
        self.counter += 1;

        let (slot_notify_tx, slot_notify_rx) = mpsc::unbounded_channel::<(usize, mpsc::UnboundedReceiver<SessionRequest>, mpsc::UnboundedSender<String>)>();
        let session = Session::new(session_id, model_id.to_string(), self.max_slots, 1, slot_notify_tx);
        session.spawn(slot_notify_rx, self.stream_hub.clone(), self.event_bus.clone(), self.storage.clone());
        self.sessions.insert(session_id, session);

        self.publish_session_event("session_created", Some(session_id), Some(model_id));

        session_id
    }

    pub fn destroy_session(&mut self, session_id: u64) -> Result<(), Session_Error> {
        self.sessions.remove(&session_id);
        self.publish_session_event("session_destroyed", Some(session_id), None);
        Ok(())
    }

    fn publish_session_event(&self, event_type: &str, session_id: Option<u64>, model_id: Option<&str>) {
        let sessions_json: Vec<serde_json::Value> = self.sessions
            .values()
            .map(|s| serde_json::json!({
                "session_id": s.session_id,
                "model_id": s.model_id,
                "occupied_slots": s.occupied_count(),
                "total_slots": s.max_slots,
            }))
            .collect();

        self.event_bus.Publish(crate::event_bus::Bus_Event::State {
            payload: serde_json::json!({
                "type": event_type,
                "session_id": session_id,
                "model_id": model_id,
                "sessions": sessions_json,
            }).to_string(),
        });
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

        let (prompt_tx, prompt_rx) = mpsc::unbounded_channel::<SessionRequest>();
        let (token_tx, token_rx) = mpsc::unbounded_channel::<String>();

        let slot_id = session
            .allocate(token_tx.clone())
            .ok_or_else(|| Session_Error::SlotExhausted(session_id.to_string()))?;

        // 通知 spawn task：新 slot 已分配
        let _ = session.slot_notify_tx.send((slot_id, prompt_rx, token_tx));

        Ok(SlotHandle {
            session_id,
            slot_id,
            prompt_tx,
            token_rx,
        })
    }

    pub fn close_slot(&mut self, session_id: u64, slot_id: usize) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.release(slot_id);
        }
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> Arc<Mutex<SessionManager>> {
        use crate::storage::StorageCapability;
        use crate::storage::StorageError;
        use crate::storage::FileEntry;
        use crate::storage::ChecksumAlgorithm;
        use crate::storage::ReadGuard;
        use crate::storage::WriteGuard;
        use async_trait::async_trait;

        struct StubStorage;
        #[async_trait]
        impl StorageCapability for StubStorage {
            async fn Acquire_Read(&self, _file_id: &str) -> Result<(std::path::PathBuf, ReadGuard), StorageError> {
                unimplemented!("stub")
            }
            async fn Acquire_Write(&self, _file_id: &str) -> Result<(std::path::PathBuf, WriteGuard), StorageError> {
                unimplemented!("stub")
            }
            async fn Remove(&self, _file_id: &str) -> Result<(), StorageError> { unimplemented!("stub") }
            async fn Exists(&self, _file_id: &str) -> Result<bool, StorageError> { unimplemented!("stub") }
            async fn List(&self) -> Result<Vec<FileEntry>, StorageError> { Ok(vec![]) }
            async fn Checksum(&self, _file_id: &str, _algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError> { unimplemented!("stub") }
            async fn Flush(&self) -> Result<(usize, usize), StorageError> { Ok((0, 0)) }
        }

        SessionManager::new(
            4,
            Arc::new(crate::orchestrator::local_tensor_stream::LocalStreamHub::new()),
            Arc::new(crate::event_bus::EventBus::New(16)),
            Arc::new(StubStorage),
        )
    }

    #[tokio::test]
    async fn test_create_and_list_sessions() {
        let mgr = make_mgr();
        let id = mgr.lock().unwrap().create_session("qwen3");
        assert!(id > 0);
        assert_eq!(mgr.lock().unwrap().list_sessions().len(), 1);
    }

    #[tokio::test]
    async fn test_destroy_session() {
        let mgr = make_mgr();
        let id = mgr.lock().unwrap().create_session("qwen3");
        mgr.lock().unwrap().destroy_session(id).unwrap();
        assert!(mgr.lock().unwrap().list_sessions().is_empty());
    }

    #[tokio::test]
    async fn test_open_slot_exhausted() {
        let mgr = make_mgr();
        let sess_id = mgr.lock().unwrap().create_session("qwen3");

        let mut lock = mgr.lock().unwrap();
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(lock.allocate_slot(sess_id).is_ok());
        assert!(matches!(lock.allocate_slot(sess_id), Err(Session_Error::SlotExhausted(_))));
    }

    #[tokio::test]
    async fn test_close_slot_and_reallocate() {
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
