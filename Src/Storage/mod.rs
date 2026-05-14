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

    /// 模块级编译与导入测试
    #[tokio::test]
    async fn test_module_imports_compile() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        // 验证导出类型可用
        let _cap: &dyn StorageCapability = &manager;
        let _err = StorageError::NotFound("test".to_string());
        let _algo = ChecksumAlgorithm::default();
        let _: Option<ReadGuard> = None;
        let _: Option<WriteGuard> = None;
        let _: Option<FileEntry> = None;
    }

    /// 端到端生命周期测试
    ///   acquire_write → 用返回的 PathBuf 创建文件并写入 → drop guard
    ///   acquire_read → 用返回的 PathBuf 打开文件并读取 → 验证内容 → drop guard
    ///   remove → 验证删除
    #[tokio::test]
    async fn test_end_to_end_write_read_remove() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();

        // 写入文件
        let (write_path, write_guard) = manager.acquire_write("data.bin").await.unwrap();
        tokio::fs::write(&write_path, b"hello world").await.unwrap();
        drop(write_guard);

        // 读取文件
        let (read_path, read_guard) = manager.acquire_read("data.bin").await.unwrap();
        let content = tokio::fs::read(&read_path).await.unwrap();
        assert_eq!(content, b"hello world");
        drop(read_guard);

        // 删除文件
        manager.remove("data.bin").await.unwrap();
        assert!(!manager.exists("data.bin").await.unwrap());
        assert!(!temp_dir.path().join("data.bin").exists());
    }
}
