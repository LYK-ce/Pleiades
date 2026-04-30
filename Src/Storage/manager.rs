//Presented by KeJi
//Date ： 2026-04-29

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;

use super::capability::{StorageCapability, StorageError, ChecksumAlgorithm, QuotaInfo};
use super::guard::{ReadGuard, WriteGuard};
use super::reservation::{Reservation, ReservationMap};

/// 文件状态，内部用于管理锁和大小
struct FileState {
    lock: Arc<RwLock<()>>,
    /// 文件磁盘大小（字节），初始扫描或 commit 时更新
    size: u64,
}

/// 存储管理器
///
/// 职责：锁管理 + 路径解析 + 文件注册表 + 配额管理。
/// 不封装 I/O，消费模块自行决定如何读写文件。
pub struct StorageManager {
    base_dir: PathBuf,
    files: RwLock<HashMap<String, FileState>>,
    // === 配额管理 ===
    /// 配额上限（字节），0 表示不限制
    quota: u64,
    /// 当前已用空间（字节）— 仅统计已 commit 的文件
    used: AtomicU64,
    /// 预留表：file_id → 预留字节数（与 Reservation 共享）
    reservation_map: ReservationMap,
}

impl StorageManager {
    /// 新建存储管理器（无配额限制），扫描指定目录下的现有文件并建立索引。
    /// 如果目录不存在，会创建它。
    pub async fn New(base_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        Self::New_With_Quota(base_dir, 0).await
    }

    /// 新建存储管理器（带配额限制），扫描指定目录下的现有文件并建立索引。
    /// 如果目录不存在，会创建它。
    ///
    /// # 参数
    /// - `base_dir`: 存储目录路径
    /// - `quota`: 配额上限（字节），0 表示不限制
    pub async fn New_With_Quota(base_dir: impl Into<PathBuf>, quota: u64) -> Result<Self, StorageError> {
        let base_dir = base_dir.into();
        // 幂等创建目录
        fs::create_dir_all(&base_dir).await.map_err(|e| {
            StorageError::Io(format!("create_dir_all failed: {}", e))
        })?;

        let mut files = HashMap::new();
        let mut initial_used: u64 = 0;
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
                let size = metadata.len();
                files.insert(
                    file_name_str.to_string(),
                    FileState {
                        lock: Arc::new(RwLock::new(())),
                        size,
                    },
                );
                initial_used += size;
            }
        }

        Ok(Self {
            base_dir,
            files: RwLock::new(files),
            quota,
            used: AtomicU64::new(initial_used),
            reservation_map: Arc::new(std::sync::Mutex::new(HashMap::new())),
        })
    }

    /// 获取基础目录（用于测试）
    pub fn Base_Dir(&self) -> &Path {
        &self.base_dir
    }

    /// 获取配额上限（字节），0 表示不限制
    pub fn Quota(&self) -> u64 {
        self.quota
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
        match fs::metadata(&full_path).await {
            Ok(metadata) if metadata.is_file() => {
                let size = metadata.len();
                // 磁盘存在，插入索引
                let mut files = self.files.write().await;
                // 双重检查
                if let Some(state) = files.get(file_id) {
                    return Ok(Arc::clone(&state.lock));
                }
                let lock = Arc::new(RwLock::new(()));
                files.insert(file_id.to_string(), FileState {
                    lock: Arc::clone(&lock),
                    size,
                });
                // 惰性发现的文件也要计入已用空间
                self.used.fetch_add(size, Ordering::Relaxed);
                Ok(lock)
            }
            _ => {
                Err(StorageError::NotFound(format!("file not found: {}", file_id)))
            }
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
        files.insert(file_id.to_string(), FileState {
            lock: Arc::clone(&lock),
            size: 0,
        });
        lock
    }

    /// 内部辅助：计算当前预留总量
    fn Total_Reserved(&self) -> u64 {
        match self.reservation_map.lock() {
            Ok(map) => map.values().sum(),
            Err(_) => 0,
        }
    }

    /// 内部辅助：计算可用空间
    fn Calc_Available(&self) -> u64 {
        if self.quota == 0 {
            return u64::MAX;
        }
        let used = self.used.load(Ordering::Relaxed);
        let reserved = self.Total_Reserved();
        self.quota.saturating_sub(used).saturating_sub(reserved)
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
                let file_size = state.size;
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
                    // 释放已用配额
                    if file_size > 0 {
                        self.used.fetch_sub(file_size, Ordering::Relaxed);
                    }
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
                            // 僵尸条目：释放配额并移除索引
                            if let Some(state) = files.get(file_id) {
                                let file_size = state.size;
                                if file_size > 0 {
                                    self.used.fetch_sub(file_size, Ordering::Relaxed);
                                }
                            }
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

    // === 配额管理 ===

    async fn reserve(&self, file_id: &str, size: u64) -> Result<Reservation, StorageError> {
        Self::Validate_File_Id(file_id)?;

        // 配额为 0 表示不限制
        if self.quota == 0 {
            let mut map = self.reservation_map.lock()
                .map_err(|e| StorageError::Io(format!("reservation lock poisoned: {}", e)))?;
            map.insert(file_id.to_string(), size);
            return Ok(Reservation::New(
                file_id.to_string(),
                size,
                Arc::clone(&self.reservation_map),
            ));
        }

        // 有配额限制：原子检查并扣减
        let mut map = self.reservation_map.lock()
            .map_err(|e| StorageError::Io(format!("reservation lock poisoned: {}", e)))?;

        let used = self.used.load(Ordering::Relaxed);
        let current_reserved: u64 = map.values().sum();
        let available = self.quota.saturating_sub(used).saturating_sub(current_reserved);

        if size > available {
            return Err(StorageError::QuotaExceeded {
                requested: size,
                available,
            });
        }

        // 预留成功
        map.insert(file_id.to_string(), size);
        Ok(Reservation::New(
            file_id.to_string(),
            size,
            Arc::clone(&self.reservation_map),
        ))
    }

    async fn commit(&self, reservation: Reservation) -> Result<(), StorageError> {
        let file_id = reservation.file_id.clone();
        let _reserved_size = reservation.size;

        // 标记为已提交，阻止 Drop 释放预留
        reservation.Mark_Committed();

        // 获取磁盘上的实际文件大小
        let full_path = self.Full_Path(&file_id);
        let actual_size = match fs::metadata(&full_path).await {
            Ok(metadata) => metadata.len(),
            Err(e) => {
                // 文件不存在或无法读取，回退：从预留表移除，调整已用空间
                if let Ok(mut map) = self.reservation_map.lock() {
                    map.remove(&file_id);
                }
                return Err(StorageError::Io(format!(
                    "commit failed, cannot stat file '{}': {}",
                    file_id, e
                )));
            }
        };

        // 从预留表移除
        if let Ok(mut map) = self.reservation_map.lock() {
            map.remove(&file_id);
        }

        // 更新 FileState 中的 size 并调整 used
        {
            let mut files = self.files.write().await;
            if let Some(state) = files.get_mut(&file_id) {
                let old_size = state.size;
                state.size = actual_size;
                // 调整 used：减去旧大小，加上实际大小
                if old_size > 0 {
                    self.used.fetch_sub(old_size, Ordering::Relaxed);
                }
            }
            // 注意：如果 file_id 不在索引中（不应发生，因为 acquire_write 会创建），
            // 这里不做处理
        }
        self.used.fetch_add(actual_size, Ordering::Relaxed);

        Ok(())
    }

    async fn quota_info(&self) -> QuotaInfo {
        let used = self.used.load(Ordering::Relaxed);
        let reserved = self.Total_Reserved();
        let available = if self.quota == 0 {
            u64::MAX
        } else {
            self.quota.saturating_sub(used).saturating_sub(reserved)
        };

        QuotaInfo {
            total: self.quota,
            used,
            reserved,
            available,
        }
    }

    async fn flush(&self) -> Result<(usize, usize), StorageError> {
        let mut discovered: usize = 0;
        let mut cleaned: usize = 0;

        // Phase 1: 扫描磁盘，发现新文件
        let mut disk_files = std::collections::HashSet::new();
        let mut entries = fs::read_dir(&self.base_dir).await.map_err(|e| {
            StorageError::Io(format!("flush read_dir failed: {}", e))
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            StorageError::Io(format!("flush next_entry failed: {}", e))
        })? {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy().to_string();
            // 忽略隐藏文件
            if file_name_str.starts_with('.') {
                continue;
            }
            let metadata = entry.metadata().await.map_err(|e| {
                StorageError::Io(format!("flush metadata failed: {}", e))
            })?;
            if !metadata.is_file() {
                continue;
            }
            disk_files.insert(file_name_str.clone());

            // 检查是否已在索引中
            let files = self.files.read().await;
            if files.contains_key(&file_name_str) {
                continue;
            }
            drop(files);

            // 不在索引中，插入
            let mut files = self.files.write().await;
            // 双重检查
            if !files.contains_key(&file_name_str) {
                let size = metadata.len();
                files.insert(file_name_str, FileState {
                    lock: Arc::new(RwLock::new(())),
                    size,
                });
                self.used.fetch_add(size, Ordering::Relaxed);
                discovered += 1;
            }
        }

        // Phase 2: 清理僵尸条目（索引有但磁盘无）
        let files = self.files.read().await;
        let zombies: Vec<String> = files.keys()
            .filter(|k| !disk_files.contains(*k))
            .cloned()
            .collect();
        drop(files);

        for zombie_id in zombies {
            let mut files = self.files.write().await;
            if let Some(state) = files.get(&zombie_id) {
                // 确保没有活跃锁才清理
                match Arc::clone(&state.lock).try_write_owned() {
                    Ok(_guard) => {
                        let size = state.size;
                        drop(_guard);
                        files.remove(&zombie_id);
                        if size > 0 {
                            self.used.fetch_sub(size, Ordering::Relaxed);
                        }
                        cleaned += 1;
                    }
                    Err(_) => {
                        // 有活跃锁，跳过
                    }
                }
            }
        }

        Ok((discovered, cleaned))
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

    // === 配额管理测试 ===

    #[tokio::test]
    async fn test_reserve_within_quota_succeeds() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();
        let reservation = manager.reserve("model.gguf", 5_000).await.unwrap();
        assert_eq!(reservation.File_Id(), "model.gguf");
        assert_eq!(reservation.Size(), 5_000);
    }

    #[tokio::test]
    async fn test_reserve_exceeding_quota_returns_quota_exceeded() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();
        let err = manager.reserve("big_model.gguf", 15_000).await.unwrap_err();
        match err {
            StorageError::QuotaExceeded { requested, available } => {
                assert_eq!(requested, 15_000);
                assert_eq!(available, 10_000);
            }
            _ => panic!("expected QuotaExceeded, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_reserve_zero_quota_means_unlimited() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap(); // quota=0
        // 应始终成功，无论大小
        let reservation = manager.reserve("huge.gguf", u64::MAX / 2).await.unwrap();
        assert_eq!(reservation.File_Id(), "huge.gguf");
    }

    #[tokio::test]
    async fn test_quota_info_reflects_current_state() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();

        let info = manager.quota_info().await;
        assert_eq!(info.total, 10_000);
        assert_eq!(info.used, 0);
        assert_eq!(info.reserved, 0);
        assert_eq!(info.available, 10_000);

        // 预留一些空间
        let _reservation = manager.reserve("model.gguf", 3_000).await.unwrap();
        let info = manager.quota_info().await;
        assert_eq!(info.reserved, 3_000);
        assert_eq!(info.available, 7_000);
    }

    #[tokio::test]
    async fn test_reserve_then_commit_updates_used() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 100_000).await.unwrap();

        // 预留
        let reservation = manager.reserve("data.bin", 50_000).await.unwrap();

        // 写入文件
        let (path, wg) = manager.acquire_write("data.bin").await.unwrap();
        let data = vec![0u8; 1024]; // 写入 1024 字节
        tokio::fs::write(&path, &data).await.unwrap();
        drop(wg);

        // 提交
        manager.commit(reservation).await.unwrap();

        let info = manager.quota_info().await;
        assert_eq!(info.used, 1024); // 实际文件大小
        assert_eq!(info.reserved, 0); // 预留已清除
        assert_eq!(info.available, 100_000 - 1024);
    }

    #[tokio::test]
    async fn test_reservation_drop_without_commit_releases_space() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();

        {
            let _reservation = manager.reserve("model.gguf", 8_000).await.unwrap();
            let info = manager.quota_info().await;
            assert_eq!(info.reserved, 8_000);
            assert_eq!(info.available, 2_000);
            // _reservation drops here without commit
        }

        // Drop 后预留应被释放
        let info = manager.quota_info().await;
        assert_eq!(info.reserved, 0);
        assert_eq!(info.available, 10_000);
    }

    #[tokio::test]
    async fn test_concurrent_reserves_do_not_exceed_quota() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();

        // 第一个预留成功
        let _r1 = manager.reserve("model_a.gguf", 6_000).await.unwrap();
        // 第二个预留成功（6000 + 3000 = 9000 <= 10000）
        let _r2 = manager.reserve("model_b.gguf", 3_000).await.unwrap();
        // 第三个预留失败（9000 + 2000 = 11000 > 10000）
        let err = manager.reserve("model_c.gguf", 2_000).await.unwrap_err();
        match err {
            StorageError::QuotaExceeded { requested, available } => {
                assert_eq!(requested, 2_000);
                assert_eq!(available, 1_000);
            }
            _ => panic!("expected QuotaExceeded"),
        }
    }

    #[tokio::test]
    async fn test_remove_file_frees_quota() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();

        // 写入文件并提交
        let reservation = manager.reserve("data.bin", 5_000).await.unwrap();
        let (path, wg) = manager.acquire_write("data.bin").await.unwrap();
        let data = vec![0u8; 2048];
        tokio::fs::write(&path, &data).await.unwrap();
        drop(wg);
        manager.commit(reservation).await.unwrap();

        let info = manager.quota_info().await;
        assert_eq!(info.used, 2048);

        // 删除文件
        manager.remove("data.bin").await.unwrap();

        let info = manager.quota_info().await;
        assert_eq!(info.used, 0);
        assert_eq!(info.available, 10_000);
    }

    #[tokio::test]
    async fn test_new_scans_existing_files_sizes_into_used() {
        let temp_dir = TempDir::new().unwrap();
        // 先写入一些文件
        tokio::fs::write(temp_dir.path().join("file_a.bin"), vec![0u8; 1000]).await.unwrap();
        tokio::fs::write(temp_dir.path().join("file_b.bin"), vec![0u8; 2000]).await.unwrap();

        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();
        let info = manager.quota_info().await;
        assert_eq!(info.used, 3000);
        assert_eq!(info.available, 7000);
    }

    #[tokio::test]
    async fn test_new_with_existing_files_exceeding_quota_still_works() {
        let temp_dir = TempDir::new().unwrap();
        // 写入超过配额的文件
        tokio::fs::write(temp_dir.path().join("big.bin"), vec![0u8; 5000]).await.unwrap();

        // 配额只有 3000，但已有文件 5000 — 初始化不应失败
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 3_000).await.unwrap();
        let info = manager.quota_info().await;
        assert_eq!(info.used, 5000);
        assert_eq!(info.available, 0); // saturating_sub

        // 新预留应被拒绝
        let err = manager.reserve("new.bin", 100).await.unwrap_err();
        assert!(matches!(err, StorageError::QuotaExceeded { .. }));
    }

    #[tokio::test]
    async fn test_quota_info_unlimited() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap(); // quota=0
        let info = manager.quota_info().await;
        assert_eq!(info.total, 0);
        assert_eq!(info.available, u64::MAX);
    }

    #[tokio::test]
    async fn test_commit_adjusts_for_actual_size_difference() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 100_000).await.unwrap();

        // 预留 50000 但只写入 100 字节
        let reservation = manager.reserve("small.bin", 50_000).await.unwrap();
        let (path, wg) = manager.acquire_write("small.bin").await.unwrap();
        tokio::fs::write(&path, vec![0u8; 100]).await.unwrap();
        drop(wg);
        manager.commit(reservation).await.unwrap();

        // used 应反映实际大小，而非预留大小
        let info = manager.quota_info().await;
        assert_eq!(info.used, 100);
        assert_eq!(info.available, 100_000 - 100);
    }

    // --- flush 测试 ---

    #[tokio::test]
    async fn test_flush_discovers_externally_added_files() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();

        // 初始应无文件
        let list = manager.list().await.unwrap();
        assert!(list.is_empty());

        // 外部直接写入文件（模拟其他进程或网络层落盘）
        tokio::fs::write(temp_dir.path().join("ext_a.bin"), vec![0u8; 1000]).await.unwrap();
        tokio::fs::write(temp_dir.path().join("ext_b.bin"), vec![0u8; 2000]).await.unwrap();

        // flush 发现新文件
        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 2);
        assert_eq!(cleaned, 0);

        // 文件应在索引中
        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(manager.exists("ext_a.bin").await.unwrap());
        assert!(manager.exists("ext_b.bin").await.unwrap());
    }

    #[tokio::test]
    async fn test_flush_updates_used_quota() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 50_000).await.unwrap();

        // 外部写入文件
        tokio::fs::write(temp_dir.path().join("data.bin"), vec![0u8; 3000]).await.unwrap();

        let info_before = manager.quota_info().await;
        assert_eq!(info_before.used, 0);

        let (discovered, _) = manager.flush().await.unwrap();
        assert_eq!(discovered, 1);

        let info_after = manager.quota_info().await;
        assert_eq!(info_after.used, 3000);
        assert_eq!(info_after.available, 47_000);
    }

    #[tokio::test]
    async fn test_flush_cleans_zombie_entries() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();

        // 通过 manager 写入文件
        let (path, wg) = manager.acquire_write("victim.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);

        assert!(manager.exists("victim.txt").await.unwrap());

        // 外部直接删除磁盘文件（绕过 manager）
        tokio::fs::remove_file(temp_dir.path().join("victim.txt")).await.unwrap();

        // flush 应清理僵尸条目
        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 0);
        assert_eq!(cleaned, 1);

        // 索引应不再包含该文件
        let list = manager.list().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_flush_cleans_zombie_frees_quota() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New_With_Quota(temp_dir.path(), 10_000).await.unwrap();

        // 通过 reserve+write+commit 流程建立带配额的文件
        let reservation = manager.reserve("old.bin", 5_000).await.unwrap();
        let (path, wg) = manager.acquire_write("old.bin").await.unwrap();
        tokio::fs::write(&path, vec![0u8; 2000]).await.unwrap();
        drop(wg);
        manager.commit(reservation).await.unwrap();

        let info = manager.quota_info().await;
        assert_eq!(info.used, 2000);

        // 外部删除文件
        tokio::fs::remove_file(temp_dir.path().join("old.bin")).await.unwrap();

        // flush 清理并释放配额
        let (_, cleaned) = manager.flush().await.unwrap();
        assert_eq!(cleaned, 1);

        let info = manager.quota_info().await;
        assert_eq!(info.used, 0);
        assert_eq!(info.available, 10_000);
    }

    #[tokio::test]
    async fn test_flush_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();

        tokio::fs::write(temp_dir.path().join("stable.bin"), vec![0u8; 500]).await.unwrap();

        // 第一次 flush
        let (d1, c1) = manager.flush().await.unwrap();
        assert_eq!(d1, 1);
        assert_eq!(c1, 0);

        // 第二次 flush（无变化）
        let (d2, c2) = manager.flush().await.unwrap();
        assert_eq!(d2, 0);
        assert_eq!(c2, 0);

        // 索引不变
        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_flush_ignores_hidden_files_and_directories() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();

        // 创建隐藏文件和子目录
        tokio::fs::write(temp_dir.path().join(".hidden"), b"secret").await.unwrap();
        tokio::fs::create_dir(temp_dir.path().join("subdir")).await.unwrap();
        // 正常文件
        tokio::fs::write(temp_dir.path().join("normal.bin"), b"ok").await.unwrap();

        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 1); // 只发现 normal.bin
        assert_eq!(cleaned, 0);

        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list.contains(&"normal.bin".to_string()));
    }

    #[tokio::test]
    async fn test_flush_mixed_discover_and_clean() {
        let temp_dir = TempDir::new().unwrap();
        let manager = StorageManager::New(temp_dir.path()).await.unwrap();

        // 通过 manager 写入两个文件
        let (p1, wg1) = manager.acquire_write("keep.bin").await.unwrap();
        tokio::fs::write(&p1, b"keep").await.unwrap();
        drop(wg1);

        let (p2, wg2) = manager.acquire_write("remove.bin").await.unwrap();
        tokio::fs::write(&p2, b"remove").await.unwrap();
        drop(wg2);

        // 外部添加新文件、删除已有文件
        tokio::fs::write(temp_dir.path().join("new.bin"), b"new").await.unwrap();
        tokio::fs::remove_file(temp_dir.path().join("remove.bin")).await.unwrap();

        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 1); // new.bin
        assert_eq!(cleaned, 1);   // remove.bin

        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&"keep.bin".to_string()));
        assert!(list.contains(&"new.bin".to_string()));
    }
}
