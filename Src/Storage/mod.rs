//Presented by KeJi
//Date ： 2026-05-14

pub use capability::{StorageCapability, StorageError, ChecksumAlgorithm, FileEntry};
pub use guard::{ReadGuard, WriteGuard};
pub use storage_manager::StorageManager;

mod capability;
mod guard;
mod storage_manager;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::event_bus::EventBus;
    use crate::peer_management::{PeerManager, Peer_Management_Capability};
    use std::sync::Arc;

    fn test_storage(base_dir: &std::path::Path) -> StorageManager {
        let pm: Arc<dyn Peer_Management_Capability> = Arc::new(PeerManager::default());
        let eb = Arc::new(EventBus::New(1));
        StorageManager::New(base_dir, pm, eb).await.unwrap()
    }

    #[tokio::test]
    async fn test_module_imports_compile() {
        let temp_dir = TempDir::new().unwrap();
        let manager = test_storage(temp_dir.path());
        let _cap: &dyn StorageCapability = &manager;
        let _err = StorageError::NotFound("test".to_string());
        let _algo = ChecksumAlgorithm::default();
        let _: Option<ReadGuard> = None;
        let _: Option<WriteGuard> = None;
        let _: Option<FileEntry> = None;
    }

    #[tokio::test]
    async fn test_end_to_end_write_read_remove() {
        let temp_dir = TempDir::new().unwrap();
        let manager = test_storage(temp_dir.path());

        let (write_path, write_guard) = manager.acquire_write("data.bin").await.unwrap();
        tokio::fs::write(&write_path, b"hello world").await.unwrap();
        drop(write_guard);

        let (read_path, read_guard) = manager.acquire_read("data.bin").await.unwrap();
        let content = tokio::fs::read(&read_path).await.unwrap();
        assert_eq!(content, b"hello world");
        drop(read_guard);

        manager.remove("data.bin").await.unwrap();
        assert!(!manager.exists("data.bin").await.unwrap());
        assert!(!temp_dir.path().join("data.bin").exists());
    }
}
