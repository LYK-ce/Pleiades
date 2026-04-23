//Presented by KeJi
//Date ： 2026-04-23

use async_trait::async_trait;
use std::fmt;
use std::path::PathBuf;
use super::guard::{ReadGuard, WriteGuard};

/// 存储模块错误类型
#[derive(Debug)]
pub enum StorageError {
    /// 文件不存在
    NotFound(String),
    /// 文件正被句柄持有，操作失败
    InUse(String),
    /// 底层 IO 错误
    Io(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::NotFound(msg) => write!(f, "NotFound: {}", msg),
            StorageError::InUse(msg) => write!(f, "InUse: {}", msg),
            StorageError::Io(msg) => write!(f, "Io: {}", msg),
        }
    }
}

impl std::error::Error for StorageError {}

/// 校验算法枚举
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Blake3,
    Sha256,
    XxHash64,
}

impl Default for ChecksumAlgorithm {
    fn default() -> Self {
        Self::XxHash64
    }
}

/// 存储能力 trait
#[async_trait]
pub trait StorageCapability: Send + Sync {
    /// 获取文件物理路径 + 共享读锁守卫
    ///
    /// 惰性发现：如果索引中不存在但磁盘存在，自动加入索引。
    /// 若文件正在被写入（WriteGuard 存活），阻塞等待写锁释放。
    ///
    /// 调用方持有 ReadGuard 期间：
    /// - 可用返回的 PathBuf 自行打开文件进行同步或异步读取
    /// - 文件不会被 remove() 或 acquire_write() 修改
    ///
    /// Drop ReadGuard 后锁释放。
    async fn acquire_read(&self, file_id: &str) -> Result<(PathBuf, ReadGuard), StorageError>;

    /// 获取文件物理路径 + 独占写锁守卫
    ///
    /// 若文件不存在，创建索引条目（但不创建磁盘文件——调用方自行创建）。
    /// 若文件正在被读取或写入，阻塞等待所有锁释放。
    ///
    /// 调用方持有 WriteGuard 期间：
    /// - 可用返回的 PathBuf 自行创建/写入文件
    /// - 独占访问，其他 acquire_read/acquire_write 阻塞等待
    ///
    /// Drop WriteGuard 后锁释放。
    async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError>;

    /// 删除文件
    ///
    /// 若有活跃的 ReadGuard 或 WriteGuard 则返回 InUse。
    /// 若文件不存在，幂等返回 Ok。
    async fn remove(&self, file_id: &str) -> Result<(), StorageError>;

    /// 检查文件是否存在
    async fn exists(&self, file_id: &str) -> Result<bool, StorageError>;

    /// 列出所有已注册的 file_id
    async fn list(&self) -> Result<Vec<String>, StorageError>;

    /// 计算文件校验码
    ///
    /// 内部获取共享读锁，自行打开文件计算。
    /// 返回格式: "{algo}:{hex}"
    async fn checksum(
        &self,
        file_id: &str,
        algo: Option<ChecksumAlgorithm>,
    ) -> Result<String, StorageError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_algorithm_default() {
        assert!(matches!(ChecksumAlgorithm::default(), ChecksumAlgorithm::XxHash64));
    }

    #[test]
    fn test_storage_error_display() {
        let not_found = StorageError::NotFound("test.txt".to_string());
        assert!(not_found.to_string().contains("NotFound"));

        let in_use = StorageError::InUse("test.txt".to_string());
        assert!(in_use.to_string().contains("InUse"));

        let io = StorageError::Io("permission denied".to_string());
        assert!(io.to_string().contains("Io"));
    }
}
