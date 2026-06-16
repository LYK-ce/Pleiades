//Presented by KeJi
//Created Date ： 2026-05-14
//Modified Date ： 2026-06-15

//! 统一存储管理模块
//!
//! 模组等级 Level 1 — 依赖 PeerManagement (L0)、EventBus (L0)、ML_Engine (L0)。
//!
//! StorageManager 是文件访问的唯一入口，严禁绕过 Storage 直接使用 std::fs 或 tokio::fs。
//!
//! ## 模块结构
//! - `file_entry` — 数据结构 (FileEntry)
//! - `capability`  — trait 定义 (StorageCapability)
//! - `guard`       — 读写锁守卫 (ReadGuard / WriteGuard)
//! - `storage_manager` — 核心组件 + trait 实现

pub use capability::{StorageCapability, StorageError, ChecksumAlgorithm};
pub use file_entry::FileEntry;
pub use guard::{ReadGuard, WriteGuard};
pub use storage_manager::StorageManager;

mod file_entry;
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

    async fn test_storage(base_dir: &std::path::Path) -> StorageManager {
        let pm: Arc<dyn Peer_Management_Capability> = Arc::new(PeerManager::default());
        let eb = Arc::new(EventBus::New(1));
        StorageManager::New(base_dir, pm, eb).await.unwrap()
    }

    #[tokio::test]
    async fn test_module_imports_compile() {
        let temp_dir = TempDir::new().unwrap();
        let manager = test_storage(temp_dir.path()).await;
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
        let manager = test_storage(temp_dir.path()).await;

        let (write_path, write_guard) = manager.Acquire_Write("data.bin").await.unwrap();
        tokio::fs::write(&write_path, b"hello world").await.unwrap();
        drop(write_guard);

        let (read_path, read_guard) = manager.Acquire_Read("data.bin").await.unwrap();
        let content = tokio::fs::read(&read_path).await.unwrap();
        assert_eq!(content, b"hello world");
        drop(read_guard);

        manager.Remove("data.bin").await.unwrap();
        assert!(!manager.Exists("data.bin").await.unwrap());
        assert!(!temp_dir.path().join("data.bin").exists());
    }
}
