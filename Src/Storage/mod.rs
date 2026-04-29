//Presented by KeJi
//Date ： 2026-04-29

pub use capability::{StorageCapability, StorageError, ChecksumAlgorithm, QuotaInfo};
pub use guard::{ReadGuard, WriteGuard};
pub use reservation::Reservation;
pub use manager::StorageManager;

mod capability;
mod guard;
mod reservation;
mod manager;

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
        // 确保守卫类型可命名
        let _: Option<ReadGuard> = None;
        let _: Option<WriteGuard> = None;
        // 确保配额类型可命名
        let _: Option<Reservation> = None;
        let _info = QuotaInfo {
            total: 0,
            used: 0,
            reserved: 0,
            available: u64::MAX,
        };
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

    /// 端到端配额流程测试
    ///   reserve → acquire_write → 写入 → commit → 验证配额
    #[tokio::test]
    async fn test_end_to_end_quota_flow() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 50_000).await.unwrap();

        // 预留
        let reservation = manager.reserve("model.bin", 10_000).await.unwrap();
        let info = manager.quota_info().await;
        assert_eq!(info.reserved, 10_000);

        // 写入
        let (path, wg) = manager.acquire_write("model.bin").await.unwrap();
        tokio::fs::write(&path, vec![0u8; 5_000]).await.unwrap();
        drop(wg);

        // 提交
        manager.commit(reservation).await.unwrap();
        let info = manager.quota_info().await;
        assert_eq!(info.used, 5_000);
        assert_eq!(info.reserved, 0);
        assert_eq!(info.available, 45_000);

        // 删除后释放配额
        manager.remove("model.bin").await.unwrap();
        let info = manager.quota_info().await;
        assert_eq!(info.used, 0);
        assert_eq!(info.available, 50_000);
    }
}
