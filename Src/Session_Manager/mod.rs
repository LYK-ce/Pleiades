// Presented by KeJi
// Date ： 2026-05-22

//! Session 模块 — 推理会话管理器
//!
//! 负责管理推理会话的生命周期、槽位分配和批处理调度。
//! - `create_session` — 创建 Session（模型级容器）
//! - `open_slot` — 分配 Slot（对话级隔离单元）
//! - `close_slot` — 释放 Slot
//! - run() 主循环 — prompt 录入 / batch flush / token 分发

pub mod capability;
pub mod manager;
pub mod session;
pub mod slot;

// ─── 聚合导出 ───────────────────────────────────────────────

pub use capability::{Session_Error, SlotHandle};
pub use manager::SessionManager;
pub use session::SessionInfo;
pub use slot::Slot;

// ─── 模块级集成测试 ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_imports_compile() {
        use std::sync::Arc;
        use std::sync::Mutex;
        let rendezvous = Arc::new(crate::network::tensor_stream::rendezvous::RendezvousMap::new());
        let event_bus = Arc::new(crate::event_bus::EventBus::New(16));
        let _ = SessionManager::new(4, rendezvous, event_bus);
        let _ = Session_Error::SessionNotFound("test".to_string());
    }

    #[test]
    fn test_slot_state_roundtrip() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::unbounded_channel();
        let slot = Slot {
            id: 0,
            token_buf: vec![1, 2, 3],
            token_tx: tx,
            dirty: true,
            temperature: 0.8,
        };
        assert_eq!(slot.id, 0);
        assert_eq!(slot.token_buf.len(), 3);
        assert!(slot.dirty);
    }
}
