//Presented by KeJi
//Date ： 2026-05-14

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

    /// 列出所有已注册文件的元数据视图
    async fn list(&self) -> Result<Vec<FileEntry>, StorageError>;

    /// 计算文件校验码
    ///
    /// 内部获取共享读锁，自行打开文件计算。
    /// 返回格式: "{algo}:{hex}"
    async fn checksum(
        &self,
        file_id: &str,
        algo: Option<ChecksumAlgorithm>,
    ) -> Result<String, StorageError>;

    /// 重新扫描 base_dir，将磁盘上存在但未纳入索引的文件加入管理，
    /// 同时清理索引中存在但磁盘上已消失的僵尸条目（跳过有活跃锁的条目）。
    ///
    /// 扫描阶段：
    /// - 对 .gguf/.pgguf 文件调用 ML Analyze 获取模型元信息
    /// - 统一 re-stat 刷新所有 FileState.size
    ///
    /// 返回 (新发现文件数, 清理僵尸数)。
    async fn flush(&self) -> Result<(usize, usize), StorageError>;
}

/// 文件元数据视图，通过 list() 返回
///
/// 与 `PeerManagement::SupportedModel` 元信息一致，各自读取。
/// 非模型文件所有 Option 字段为 None。
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// 存储文件名（扁平命名空间）
    pub file_name: String,
    /// 模型唯一标识（xxhash64(pgguf内容)），非模型文件为 None
    pub model_id: Option<u64>,
    /// 磁盘文件大小（字节），flush 时统一刷新
    pub size: u64,
    /// 模型总层数
    pub num_layers: Option<u32>,
    /// 256 位层位图，bit N = 1 表示持有第 N 层
    pub layer_bitmap: Option<[u8; 32]>,
    /// 模型架构名（如 qwen3）
    pub architecture: Option<String>,
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
