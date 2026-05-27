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
        use crate::storage::{StorageCapability, StorageError, FileEntry, ChecksumAlgorithm, ReadGuard, WriteGuard};

        struct StubStorage;
        #[async_trait::async_trait]
        impl StorageCapability for StubStorage {
            async fn acquire_read(&self, _file_id: &str) -> Result<(std::path::PathBuf, ReadGuard), StorageError> { unimplemented!() }
            async fn acquire_write(&self, _file_id: &str) -> Result<(std::path::PathBuf, WriteGuard), StorageError> { unimplemented!() }
            async fn remove(&self, _file_id: &str) -> Result<(), StorageError> { unimplemented!() }
            async fn exists(&self, _file_id: &str) -> Result<bool, StorageError> { unimplemented!() }
            async fn list(&self) -> Result<Vec<FileEntry>, StorageError> { Ok(vec![]) }
            async fn checksum(&self, _file_id: &str, _algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError> { unimplemented!() }
            async fn flush(&self) -> Result<(usize, usize), StorageError> { Ok((0, 0)) }
        }

        let hub = Arc::new(crate::orchestrator::local_tensor_stream::LocalStreamHub::new());
        let event_bus = Arc::new(crate::event_bus::EventBus::New(16));
        let storage = Arc::new(StubStorage);
        let _ = SessionManager::new(4, hub, event_bus, storage);
        let _ = Session_Error::SessionNotFound("test".to_string());
    }

    #[test]
    fn test_slot_state_roundtrip() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::unbounded_channel();
        let slot = Slot {
            id: 0,
            token_tx: tx,
        };
        assert_eq!(slot.id, 0);
    }
}
