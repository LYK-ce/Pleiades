//Presented by KeJi
//Date ： 2026-05-14

use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use super::capability::{IoFrontend, IoHandle, Session_Capability, Session_Error};
use super::session::{Session, SessionInfo};
use super::slot::SlotState;

// ─── 常量 ───────────────────────────────────────────────────

const CHANNEL_BUFFER_SIZE: usize = 64;

// ─── SessionManager ─────────────────────────────────────────

/// Session 管理器，负责会话生命周期、槽位分配和 IO 通道管理。
pub struct SessionManager {
    sessions: Mutex<HashMap<String, Session>>,
    max_slots: usize,
}

impl SessionManager {
    pub fn new(max_slots: usize) -> Self {
        SessionManager {
            sessions: Mutex::new(HashMap::new()),
            max_slots,
        }
    }
}

#[async_trait::async_trait]
impl Session_Capability for SessionManager {
    async fn create_session(
        &self,
        model_id: String,
    ) -> Result<(String, IoHandle), Session_Error> {
        let session_id = generate_session_id();
        let mut session = Session::new(session_id.clone(), model_id, self.max_slots);

        let (input_tx, input_rx) = mpsc::channel::<String>(CHANNEL_BUFFER_SIZE);
        let (output_tx, output_rx) = mpsc::channel::<String>(CHANNEL_BUFFER_SIZE);

        let io_handle = IoHandle { input_rx, output_tx };
        // IoFrontend 由 connect() 按需创建（每个槽位独立通道）
        drop(IoFrontend { input_tx, output_rx });

        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_id.clone(), session);

        Ok((session_id, io_handle))
    }

    async fn destroy_session(&self, session_id: &str) -> Result<(), Session_Error> {
        let mut sessions = self.sessions.lock().await;
        sessions.remove(session_id);
        Ok(())
    }

    async fn connect(&self, session_id: &str) -> Result<(u32, IoFrontend), Session_Error> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(session_id).ok_or_else(|| {
            Session_Error::SessionNotFound(session_id.to_string())
        })?;

        if session.slot_counter as usize >= session.max_slots {
            return Err(Session_Error::SlotExhausted(session_id.to_string()));
        }

        let slot_id = session.slot_counter;
        session.slots[slot_id as usize].state = SlotState::Occupied {
            owner: format!("frontend-{}", slot_id),
        };
        session.slot_counter += 1;

        let (input_tx, _input_rx) = mpsc::channel::<String>(CHANNEL_BUFFER_SIZE);
        let (_output_tx, output_rx) = mpsc::channel::<String>(CHANNEL_BUFFER_SIZE);

        Ok((slot_id, IoFrontend { input_tx, output_rx }))
    }

    fn list_sessions(&self) -> Vec<SessionInfo> {
        if let Ok(sessions) = self.sessions.try_lock() {
            sessions.values().map(|s| SessionInfo {
                session_id: s.session_id.clone(),
                model_id: s.model_id.clone(),
                total_slots: s.max_slots,
                occupied_slots: s.occupied_count(),
            }).collect()
        } else {
            Vec::new()
        }
    }

    async fn release_slot(&self, session_id: &str, slot_id: u32) -> Result<(), Session_Error> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(session_id).ok_or_else(|| {
            Session_Error::SessionNotFound(session_id.to_string())
        })?;

        if (slot_id as usize) >= session.slots.len() {
            return Err(Session_Error::Internal(format!(
                "slot_id {} out of range (max {})",
                slot_id,
                session.slots.len()
            )));
        }

        session.slots[slot_id as usize].state = SlotState::Vacant;
        Ok(())
    }
}

fn generate_session_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("sess-{}", id)
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_session() {
        let mgr = SessionManager::new(4);
        let (session_id, _io) = mgr.create_session("qwen3".to_string()).await.unwrap();
        assert!(session_id.starts_with("sess-"));
    }

    #[tokio::test]
    async fn test_list_sessions_empty() {
        let mgr = SessionManager::new(4);
        assert!(mgr.list_sessions().is_empty());
    }

    #[tokio::test]
    async fn test_list_sessions_after_create() {
        let mgr = SessionManager::new(4);
        let (session_id, _io) = mgr.create_session("qwen3".to_string()).await.unwrap();
        let infos = mgr.list_sessions();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].session_id, session_id);
        assert_eq!(infos[0].total_slots, 4);
        assert_eq!(infos[0].occupied_slots, 0);
    }

    #[tokio::test]
    async fn test_connect_allocates_slot() {
        let mgr = SessionManager::new(4);
        let (session_id, _io) = mgr.create_session("qwen3".to_string()).await.unwrap();
        let (slot_id, _frontend) = mgr.connect(&session_id).await.unwrap();
        assert_eq!(slot_id, 0);
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 1);
    }

    #[tokio::test]
    async fn test_connect_slot_exhausted() {
        let mgr = SessionManager::new(2);
        let (session_id, _io) = mgr.create_session("qwen3".to_string()).await.unwrap();
        mgr.connect(&session_id).await.unwrap();
        mgr.connect(&session_id).await.unwrap();
        let result = mgr.connect(&session_id).await;
        assert!(matches!(result, Err(Session_Error::SlotExhausted(_))));
    }

    #[tokio::test]
    async fn test_connect_session_not_found() {
        let mgr = SessionManager::new(4);
        assert!(matches!(mgr.connect("no-such").await, Err(Session_Error::SessionNotFound(_))));
    }

    #[tokio::test]
    async fn test_destroy_session() {
        let mgr = SessionManager::new(4);
        let (session_id, _io) = mgr.create_session("qwen3".to_string()).await.unwrap();
        mgr.destroy_session(&session_id).await.unwrap();
        assert!(mgr.list_sessions().is_empty());
    }

    #[tokio::test]
    async fn test_release_slot() {
        let mgr = SessionManager::new(4);
        let (session_id, _io) = mgr.create_session("qwen3".to_string()).await.unwrap();
        let (slot_id, _frontend) = mgr.connect(&session_id).await.unwrap();
        mgr.release_slot(&session_id, slot_id).await.unwrap();
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 0);
    }
}
