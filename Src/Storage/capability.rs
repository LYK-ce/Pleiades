//Presented by KeJi
//Date ： 2026-04-29

use async_trait::async_trait;
use std::fmt;
use std::path::PathBuf;
use super::guard::{ReadGuard, WriteGuard};
use super::reservation::Reservation;

/// 存储模块错误类型
#[derive(Debug)]
pub enum StorageError {
    /// 文件不存在
    NotFound(String),
    /// 文件正被句柄持有，操作失败
    InUse(String),
    /// 底层 IO 错误
    Io(String),
    /// 配额不足
    QuotaExceeded {
        /// 请求的空间大小（字节）
        requested: u64,
        /// 当前可用空间（字节）
        available: u64,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::NotFound(msg) => write!(f, "NotFound: {}", msg),
            StorageError::InUse(msg) => write!(f, "InUse: {}", msg),
            StorageError::Io(msg) => write!(f, "Io: {}", msg),
            StorageError::QuotaExceeded { requested, available } => {
                write!(
                    f,
                    "QuotaExceeded: requested {} bytes, available {} bytes",
                    requested, available
                )
            }
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

/// 配额使用情况快照
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaInfo {
    /// 配额总量（字节），0 表示不限制
    pub total: u64,
    /// 已使用空间（已完成写入的文件）
    pub used: u64,
    /// 已预留空间（正在写入中的文件）
    pub reserved: u64,
    /// 可分配空间 = total - used - reserved（配额为 0 时返回 u64::MAX）
    pub available: u64,
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
    ///
    /// 注意：此方法不检查配额。需要配额管理时请使用 reserve() + commit() 两阶段流程。
    async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError>;

    /// 删除文件
    ///
    /// 若有活跃的 ReadGuard 或 WriteGuard 则返回 InUse。
    /// 若文件不存在，幂等返回 Ok。
    /// 删除成功后释放该文件占用的配额。
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

    // === 配额管理 ===

    /// 阶段 1：预留空间
    ///
    /// 原子检查并扣减可用配额。成功返回 Reservation 令牌。
    /// 若空间不足，返回 QuotaExceeded 并附带 requested/available 信息。
    /// 若配额为 0（不限制），始终成功。
    ///
    /// Reservation 实现 RAII：
    /// - 正常流程：调用 commit() 消费令牌，将预留转为已用
    /// - 异常流程：Reservation 被 Drop 时自动释放预留空间
    async fn reserve(&self, file_id: &str, size: u64) -> Result<Reservation, StorageError>;

    /// 阶段 2：提交写入
    ///
    /// 调用方已完成文件写入后调用。将预留转为已用，更新文件大小记录。
    /// 通过 stat 获取磁盘上的实际文件大小，若与预留不同则自动修正差额。
    /// 消费 Reservation，阻止 Drop 释放。
    async fn commit(&self, reservation: Reservation) -> Result<(), StorageError>;

    /// 查询配额信息
    ///
    /// 返回当前配额使用情况的快照。
    async fn quota_info(&self) -> QuotaInfo;
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

    #[test]
    fn test_storage_error_quota_exceeded_display() {
        let err = StorageError::QuotaExceeded {
            requested: 5_000_000_000,
            available: 3_000_000_000,
        };
        let msg = err.to_string();
        assert!(msg.contains("QuotaExceeded"));
        assert!(msg.contains("5000000000"));
        assert!(msg.contains("3000000000"));
    }

    #[test]
    fn test_quota_info_fields() {
        let info = QuotaInfo {
            total: 10_000_000_000,
            used: 5_000_000_000,
            reserved: 1_000_000_000,
            available: 4_000_000_000,
        };
        assert_eq!(info.total, 10_000_000_000);
        assert_eq!(info.used, 5_000_000_000);
        assert_eq!(info.reserved, 1_000_000_000);
        assert_eq!(info.available, 4_000_000_000);
    }
}
