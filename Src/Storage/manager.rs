//Presented by KeJi
//Date ： 2026-04-23

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;

use super::capability::{StorageCapability, StorageError, ChecksumAlgorithm};
use super::guard::{ReadGuard, WriteGuard};

/// 文件状态，内部用于管理锁
struct FileState {
    lock: Arc<RwLock<()>>,
}

/// 存储管理器
///
/// 职责：锁管理 + 路径解析 + 文件注册表。
/// 不封装 I/O，消费模块自行决定如何读写文件。
pub struct StorageManager {
    base_dir: PathBuf,
    files: RwLock<HashMap<String, FileState>>,
}

impl StorageManager {
    /// 新建存储管理器，扫描指定目录下的现有文件并建立索引。
    /// 如果目录不存在，会创建它。
    pub async fn New(base_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let base_dir = base_dir.into();
        // 幂等创建目录
        fs::create_dir_all(&base_dir).await.map_err(|e| {
            StorageError::Io(format!("create_dir_all failed: {}", e))
        })?;

        let mut files = HashMap::new();
        let mut entries = fs::read_dir(&base_dir).await.map_err(|e| {
            StorageError::Io(format!("read_dir failed: {}", e))
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            StorageError::Io(format!("next_entry failed: {}", e))
        })? {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            // 忽略隐藏文件（以 . 开头）
            if file_name_str.starts_with('.') {
                continue;
            }
            // 忽略子目录（只处理普通文件）
            let metadata = entry.metadata().await.map_err(|e| {
                StorageError::Io(format!("metadata failed: {}", e))
            })?;
            if metadata.is_file() {
                files.insert(
                    file_name_str.to_string(),
                    FileState {
                        lock: Arc::new(RwLock::new(())),
                    },
                );
            }
        }

        Ok(Self {
            base_dir,
            files: RwLock::new(files),
        })
    }

    /// 获取基础目录（用于测试）
    pub fn Base_Dir(&self) -> &Path {
        &self.base_dir
    }

    /// 内部辅助：验证 file_id 合法性
    fn Validate_File_Id(file_id: &str) -> Result<(), StorageError> {
        if file_id.is_empty() {
            return Err(StorageError::NotFound("empty file_id".to_string()));
        }
        if file_id.contains('/') || file_id.contains('\\') || file_id.contains("..") {
            return Err(StorageError::Io(format!("invalid file_id: {}", file_id)));
        }
        if file_id.starts_with('.') {
            return Err(StorageError::Io(format!("hidden file_id not allowed: {}", file_id)));
        }
        Ok(())
    }

    /// 内部辅助：构造完整路径
    fn Full_Path(&self, file_id: &str) -> PathBuf {
        let mut path = self.base_dir.clone();
        path.push(file_id);
        path
    }

    /// 内部辅助：惰性发现，如果索引中不存在但磁盘存在，则插入索引
    async fn Lazy_Discover(&self, file_id: &str) -> Result<Arc<RwLock<()>>, StorageError> {
        let files = self.files.read().await;
        if let Some(state) = files.get(file_id) {
            return Ok(Arc::clone(&state.lock));
        }
        drop(files); // 释放读锁，准备获取写锁

        let full_path = self.Full_Path(file_id);
        if fs::metadata(&full_path).await.is_ok() {
            // 磁盘存在，插入索引
            let mut files = self.files.write().await;
            // 双重检查
            if let Some(state) = files.get(file_id) {
                return Ok(Arc::clone(&state.lock));
            }
            let lock = Arc::new(RwLock::new(()));
            files.insert(file_id.to_string(), FileState { lock: Arc::clone(&lock) });
            Ok(lock)
        } else {
            Err(StorageError::NotFound(format!("file not found: {}", file_id)))
        }
    }

    /// 内部辅助：确保索引条目存在（不检查磁盘），用于 acquire_write
    async fn Ensure_Entry(&self, file_id: &str) -> Arc<RwLock<()>> {
        let files = self.files.read().await;
        if let Some(state) = files.get(file_id) {
            return Arc::clone(&state.lock);
        }
        drop(files);

        let mut files = self.files.write().await;
        // 双重检查
        if let Some(state) = files.get(file_id) {
            return Arc::clone(&state.lock);
        }
        let lock = Arc::new(RwLock::new(()));
        files.insert(file_id.to_string(), FileState { lock: Arc::clone(&lock) });
        lock
    }
}

#[async_trait]
impl StorageCapability for StorageManager {
    async fn acquire_read(&self, file_id: &str) -> Result<(PathBuf, ReadGuard), StorageError> {
        Self::Validate_File_Id(file_id)?;
        let lock = self.Lazy_Discover(file_id).await?;
        let guard = lock.read_owned().await;
        let path = self.Full_Path(file_id);
        Ok((path, ReadGuard {
            file_id: file_id.to_string(),
            _guard: guard,
        }))
    }

    async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError> {
        Self::Validate_File_Id(file_id)?;
        let lock = self.Ensure_Entry(file_id).await;
        let guard = lock.write_owned().await;
        let path = self.Full_Path(file_id);
        Ok((path, WriteGuard {
            file_id: file_id.to_string(),
            _guard: guard,
        }))
    }

    async fn remove(&self, file_id: &str) -> Result<(), StorageError> {
        Self::Validate_File_Id(file_id)?;
        let files = self.files.read().await;
        let state = match files.get(file_id) {
            Some(s) => s,
            None => {
                // 索引中不存在，检查磁盘
                let full_path = self.Full_Path(file_id);
                match fs::metadata(&full_path).await {
                    Ok(metadata) if metadata.is_file() => {
                        // 磁盘存在文件，删除它
                        if let Err(e) = fs::remove_file(&full_path).await {
                            if e.kind() != std::io::ErrorKind::NotFound {
                                return Err(StorageError::Io(format!("remove_file failed: {}", e)));
                            }
                        }
                        return Ok(());
                    }
                    _ => {
                        // 磁盘也不存在，幂等返回成功
                        return Ok(());
                    }
                }
            }
        };
        // 尝试获取写锁（探测是否有活跃守卫）
        match Arc::clone(&state.lock).try_write_owned() {
            Ok(_guard) => {
                // 没有活跃守卫，可以删除
                drop(_guard);
                drop(files);
                let mut files = self.files.write().await;
                // 再次检查，防止竞争
                if files.contains_key(file_id) {
                    let full_path = self.Full_Path(file_id);
                    if let Err(e) = fs::remove_file(&full_path).await {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            return Err(StorageError::Io(format!("remove_file failed: {}", e)));
                        }
                    }
                    files.remove(file_id);
                }
                Ok(())
            }
            Err(_) => Err(StorageError::InUse(format!("file {} is in use", file_id))),
        }
    }

    async fn exists(&self, file_id: &str) -> Result<bool, StorageError> {
        Self::Validate_File_Id(file_id)?;
        let files = self.files.read().await;
        if files.contains_key(file_id) {
            let full_path = self.Full_Path(file_id);
            match fs::metadata(&full_path).await {
                Ok(metadata) if metadata.is_file() => Ok(true),
                _ => {
                    drop(files);
                    let mut files = self.files.write().await;
                    if files.contains_key(file_id) {
                        let full_path = self.Full_Path(file_id);
                        if fs::metadata(&full_path).await.is_err() {
                            files.remove(file_id);
                        }
                    }
                    Ok(false)
                }
            }
        } else {
            Ok(false)
        }
    }

    async fn list(&self) -> Result<Vec<String>, StorageError> {
        let files = self.files.read().await;
        Ok(files.keys().cloned().collect())
    }

    async fn checksum(
        &self,
        file_id: &str,
        algo: Option<ChecksumAlgorithm>,
    ) -> Result<String, StorageError> {
        Self::Validate_File_Id(file_id)?;
        let algo = algo.unwrap_or_default();
        // 获取读锁
        let (path, _guard) = self.acquire_read(file_id).await?;
        // 自行打开文件计算校验码
        let mut file = tokio::fs::File::open(&path).await
            .map_err(|e| StorageError::Io(format!("open failed: {}", e)))?;
        let hasher = match algo {
            ChecksumAlgorithm::Blake3 => {
                use blake3::Hasher;
                let mut hasher = Hasher::new();
                let mut buf = vec![0; 8192];
                loop {
                    let n = file.read(&mut buf).await.map_err(|e| {
                        StorageError::Io(format!("read failed: {}", e))
                    })?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                let hash = hasher.finalize();
                format!("blake3:{}", hash.to_hex())
            }
            ChecksumAlgorithm::Sha256 => {
                use sha2::{Sha256, Digest};
                let mut hasher = Sha256::new();
                let mut buf = vec![0; 8192];
                loop {
                    let n = file.read(&mut buf).await.map_err(|e| {
                        StorageError::Io(format!("read failed: {}", e))
                    })?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                let hash = hasher.finalize();
                format!("sha256:{}", hex::encode(hash))
            }
            ChecksumAlgorithm::XxHash64 => {
                use twox_hash::xxh3;
                use std::hash::Hasher;
                let mut hasher = xxh3::Hash64::with_seed(0);
                let mut buf = vec![0; 8192];
                loop {
                    let n = file.read(&mut buf).await.map_err(|e| {
                        StorageError::Io(format!("read failed: {}", e))
                    })?;
                    if n == 0 {
                        break;
                    }
                    hasher.write(&buf[..n]);
                }
                let hash = hasher.finish();
                format!("xxhash64:{:016x}", hash)
            }
        };
        Ok(hasher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::time::{timeout, Duration};

    // --- 生命周期 ---
    #[tokio::test]
    async fn test_acquire_read_returns_valid_path_and_guard() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        // 先创建文件（通过 acquire_write 注册索引，然后手动写文件）
        let (path, _wg) = manager.acquire_write("test.txt").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(_wg);
        // acquire_read
        let (path, rg) = manager.acquire_read("test.txt").await.unwrap();
        assert_eq!(rg.file_id(), "test.txt");
        assert!(path.ends_with("test.txt"));
        // 调用方自行打开文件
        let content = tokio::fs::read(&path).await.unwrap();
        assert_eq!(&content, b"hello");
    }

    #[tokio::test]
    async fn test_acquire_write_returns_valid_path_and_guard() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.txt").await.unwrap();
        assert_eq!(wg.file_id(), "test.txt");
        assert!(path.ends_with("test.txt"));
        // 调用方自行创建文件
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);
        let content = tokio::fs::read(&path).await.unwrap();
        assert_eq!(&content, b"data");
    }

    #[tokio::test]
    async fn test_acquire_write_creates_index_entry() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("new.txt").await.unwrap();
        // 索引中应存在（即使磁盘文件还没创建）
        // 注意：exists 检查磁盘，这里先创建磁盘文件
        tokio::fs::write(&path, b"").await.unwrap();
        drop(wg);
        assert!(manager.exists("new.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_remove_deletes_file_and_index() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);
        manager.remove("test.txt").await.unwrap();
        assert!(!manager.exists("test.txt").await.unwrap());
        assert!(!manager.Base_Dir().join("test.txt").exists());
    }

    #[tokio::test]
    async fn test_remove_idempotent_on_missing() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        manager.remove("ghost.txt").await.unwrap();
    }

    // --- 并发锁语义 ---
    #[tokio::test]
    async fn test_multiple_read_guards_concurrent() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);
        let (_p1, rg1) = manager.acquire_read("test.txt").await.unwrap();
        let (_p2, rg2) = manager.acquire_read("test.txt").await.unwrap();
        assert_eq!(rg1.file_id(), "test.txt");
        assert_eq!(rg2.file_id(), "test.txt");
    }

    #[tokio::test]
    async fn test_write_guard_blocks_read() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        // 写锁存在时，acquire_read 应阻塞
        let result = timeout(Duration::from_millis(100), manager.acquire_read("test.txt")).await;
        assert!(result.is_err()); // 超时
        drop(wg);
        // 写锁释放后应成功
        let (_p, rg) = manager.acquire_read("test.txt").await.unwrap();
        assert_eq!(rg.file_id(), "test.txt");
    }

    #[tokio::test]
    async fn test_write_guard_blocks_second_write() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (_path, wg1) = manager.acquire_write("test.txt").await.unwrap();
        let result = timeout(Duration::from_millis(100), manager.acquire_write("test.txt")).await;
        assert!(result.is_err());
        drop(wg1);
        let (_p, wg2) = manager.acquire_write("test.txt").await.unwrap();
        assert_eq!(wg2.file_id(), "test.txt");
    }

    #[tokio::test]
    async fn test_remove_returns_inuse_when_read_guard_alive() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);
        let (_p, _rg) = manager.acquire_read("test.txt").await.unwrap();
        let err = manager.remove("test.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::InUse(_)));
    }

    #[tokio::test]
    async fn test_remove_returns_inuse_when_write_guard_alive() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (_path, _wg) = manager.acquire_write("test.txt").await.unwrap();
        let err = manager.remove("test.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::InUse(_)));
    }

    // --- 惰性发现 ---
    #[tokio::test]
    async fn test_lazy_discover_on_acquire_read() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        // 外部创建文件
        let file_path = temp_dir.path().join("external.bin");
        tokio::fs::write(&file_path, b"data").await.unwrap();
        // 应能通过惰性发现读取
        let (path, rg) = manager.acquire_read("external.bin").await.unwrap();
        assert_eq!(rg.file_id(), "external.bin");
        assert!(manager.exists("external.bin").await.unwrap());
        let content = tokio::fs::read(&path).await.unwrap();
        assert_eq!(&content, b"data");
    }

    #[tokio::test]
    async fn test_lazy_discover_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let err = manager.acquire_read("nonexistent.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    // --- 校验码 ---
    #[tokio::test]
    async fn test_checksum_xxhash64() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.bin").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(wg);
        let sum = manager.checksum("test.bin", None).await.unwrap();
        assert!(sum.starts_with("xxhash64:"));
    }

    #[tokio::test]
    async fn test_checksum_sha256() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.bin").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(wg);
        let sum = manager.checksum("test.bin", Some(ChecksumAlgorithm::Sha256)).await.unwrap();
        assert!(sum.starts_with("sha256:"));
    }

    #[tokio::test]
    async fn test_checksum_blake3() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.bin").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(wg);
        let sum = manager.checksum("test.bin", Some(ChecksumAlgorithm::Blake3)).await.unwrap();
        assert!(sum.starts_with("blake3:"));
    }

    // --- 路径安全 ---
    #[tokio::test]
    async fn test_reject_path_traversal() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let err = manager.acquire_read("invalid/path.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::Io(_)));
    }

    #[tokio::test]
    async fn test_reject_hidden_file() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let err = manager.acquire_read(".hidden").await.unwrap_err();
        assert!(matches!(err, StorageError::Io(_)));
    }

    #[tokio::test]
    async fn test_reject_empty_file_id() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let err = manager.acquire_read("").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    // --- 外部文件处理 ---
    #[tokio::test]
    async fn test_remove_externally_injected_file() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        tokio::fs::write(temp_dir.path().join("injected.bin"), b"data").await.unwrap();
        assert!(!manager.list().await.unwrap().contains(&"injected.bin".to_string()));
        manager.remove("injected.bin").await.unwrap();
        assert!(!temp_dir.path().join("injected.bin").exists());
    }

    #[tokio::test]
    async fn test_exists_cleans_zombie_entry() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("zombie.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);
        // 外部删除文件
        tokio::fs::remove_file(temp_dir.path().join("zombie.txt")).await.unwrap();
        assert!(!manager.exists("zombie.txt").await.unwrap());
        assert!(!manager.list().await.unwrap().contains(&"zombie.txt".to_string()));
    }

    // --- 实时校验码 ---
    #[tokio::test]
    async fn test_checksum_realtime_no_cache() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();
        let (path, wg) = manager.acquire_write("test.bin").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(wg);
        let sum1 = manager.checksum("test.bin", None).await.unwrap();
        // 修改文件内容
        let (path, wg) = manager.acquire_write("test.bin").await.unwrap();
        tokio::fs::write(&path, b"world").await.unwrap();
        drop(wg);
        let sum2 = manager.checksum("test.bin", None).await.unwrap();
        assert_ne!(sum1, sum2);
    }
}
