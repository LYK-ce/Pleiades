//Presented by KeJi
//Created Date ： 2026-06-15
//Modified Date ： 2026-06-15

//! 文件元数据与校验算法类型
//!
//! FileEntry 是 Storage 模块统一使用的文件描述结构，
//! 对内承担原 FileState 的锁管理职责，对外通过 list() 暴露。
//! ChecksumAlgorithm 定义 StorageCapability::checksum 的校验算法选项。

use std::sync::Arc;
use tokio::sync::RwLock;

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

/// 文件元数据
///
/// 对内：HashMap 值，lock 字段管理读写并发
/// 对外：list() 返回 Vec<FileEntry>（lock 不可见）
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// 存储文件名（扁平命名空间）
    pub file_name: String,
    /// 磁盘文件大小（字节）
    pub size: u64,
    /// 模型唯一标识（xxhash32），非模型文件为 None
    pub model_id: Option<u32>,
    /// 模型总层数
    pub num_layers: Option<u32>,
    /// 256 位层位图
    pub layer_bitmap: Option<[u8; 32]>,
    /// 模型架构名
    pub architecture: Option<String>,
    /// 内部读写锁（pub(crate)，仅 StorageManager 使用）
    pub(crate) lock: Arc<RwLock<()>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_algorithm_default() {
        assert!(matches!(ChecksumAlgorithm::default(), ChecksumAlgorithm::XxHash64));
    }
}
