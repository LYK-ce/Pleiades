//Presented by KeJi
//Date ： 2026-05-14

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;

use super::capability::{StorageCapability, StorageError, ChecksumAlgorithm, FileEntry};
use super::guard::{ReadGuard, WriteGuard};
use crate::event_bus::{EventBus, Bus_Event};
use crate::peer_management::{
    Peer_Management_Capability, SupportedModel,
};
use crate::ml_engine::capability::analyze_model;

/// 文件状态，内部用于管理锁和模型元信息
#[derive(Debug)]
struct FileState {
    lock: Arc<RwLock<()>>,
    /// 文件磁盘大小（字节），flush 时统一刷新
    size: u64,
    /// 模型唯一标识（从 PGGUF 元数据读取），非模型文件为 None
    model_id: Option<u32>,
    /// 模型总层数
    num_layers: Option<u32>,
    /// 256 位层位图
    layer_bitmap: Option<[u8; 32]>,
    /// 模型架构名
    architecture: Option<String>,
}

/// 存储管理器
///
/// 职责：锁管理 + 路径解析 + 文件注册表 + 模型统一管理。
/// 不封装 I/O，消费模块自行决定如何读写文件。
pub struct StorageManager {
    base_dir: PathBuf,
    files: RwLock<HashMap<String, FileState>>,
    peer_manager: Arc<dyn Peer_Management_Capability>,
    event_bus: Arc<EventBus>,
}

impl StorageManager {
    /// 新建存储管理器
    pub async fn New(
        base_dir: impl Into<PathBuf>,
        peer_manager: Arc<dyn Peer_Management_Capability>,
        event_bus: Arc<EventBus>,
    ) -> Result<Self, StorageError> {
        let base_dir = base_dir.into();
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
            if file_name_str.starts_with('.') {
                continue;
            }
            let metadata = entry.metadata().await.map_err(|e| {
                StorageError::Io(format!("metadata failed: {}", e))
            })?;
            if metadata.is_file() {
                files.insert(
                    file_name_str.to_string(),
                    FileState {
                        lock: Arc::new(RwLock::new(())),
                        size: metadata.len(),
                        model_id: None,
                        num_layers: None,
                        layer_bitmap: None,
                        architecture: None,
                    },
                );
            }
        }

        Ok(Self {
            base_dir,
            files: RwLock::new(files),
            peer_manager,
            event_bus,
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
                    model_id: None,
                    num_layers: None,
                    layer_bitmap: None,
                    architecture: None,
                });
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
            model_id: None,
            num_layers: None,
            layer_bitmap: None,
            architecture: None,
        });
        lock
    }

    /// 判断文件名是否为模型文件（.gguf 或 .pgguf）
    fn Is_Model_File(file_name: &str) -> bool {
        let lower = file_name.to_lowercase();
        lower.ends_with(".gguf") || lower.ends_with(".pgguf")
    }

    /// 内部辅助：从 FileState 构造 FileEntry
    fn Build_File_Entry(&self, file_name: &str, state: &FileState) -> FileEntry {
        FileEntry {
            file_name: file_name.to_string(),
            model_id: state.model_id,
            size: state.size,
            num_layers: state.num_layers,
            layer_bitmap: state.layer_bitmap,
            architecture: state.architecture.clone(),
        }
    }

    /// flush + 同步模型信息到 PeerManager + 通知 TUI + 广播给远程 peer
    pub async fn flush_and_sync(&self) -> Result<(usize, usize), String> {
        let (discovered, cleaned) = <Self as StorageCapability>::flush(self).await
            .map_err(|e| format!("flush: {e}"))?;
        self.sync_models_to_peer_manager().await;
        Ok((discovered, cleaned))
    }

    /// 同步本地模型信息到 PeerManager + 通知 TUI
    async fn sync_models_to_peer_manager(&self) {
        if let Ok(entries) = <Self as StorageCapability>::list(self).await {
            let models: Vec<_> = entries.iter().filter_map(|e| {
                Some(SupportedModel {
                    id: e.model_id?,
                    file_name: e.file_name.clone(),
                    layer_bitmap: e.layer_bitmap?,
                })
            }).collect();
            if let Ok(local) = self.peer_manager.Get_Local_Peer().await {
                let _ = self.peer_manager.Update_Supported_Models(&local.peer_id, models.clone()).await;
                let models_display: Vec<serde_json::Value> = models.iter().map(|m| {
                    serde_json::json!({"file_name": m.file_name, "layer_range": m.layer_range()})
                }).collect();
                self.event_bus.Publish(Bus_Event::State {
                    payload: serde_json::json!({
                        "type": "peer_info_updated",
                        "peer_id": local.peer_id.to_string(),
                        "peer_name": local.name,
                        "is_local": true,
                        "models": models_display,
                        "sessions": serde_json::json!([]),
                    }).to_string(),
                });
            }
        }
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

        // 快速路径：索引中不存在的文件，直接检查磁盘并删除
        {
            let files = self.files.read().await;
            if !files.contains_key(file_id) {
                drop(files);
                let full_path = self.Full_Path(file_id);
                match fs::metadata(&full_path).await {
                    Ok(metadata) if metadata.is_file() => {
                        if let Err(e) = fs::remove_file(&full_path).await {
                            if e.kind() != std::io::ErrorKind::NotFound {
                                return Err(StorageError::Io(format!("remove_file failed: {}", e)));
                            }
                        }
                    }
                    _ => {} // 磁盘也不存在，幂等返回成功
                }
                return Ok(());
            }
        }

        // 文件在索引中：持有 files 写锁，原子化探针 + 删除
        let mut files = self.files.write().await;
        let file_lock = match files.get(file_id) {
            Some(state) => Arc::clone(&state.lock),
            None => return Ok(()), // 在等锁期间被其他线程删除
        };

        match file_lock.try_write_owned() {
            Ok(_guard) => {
                drop(_guard);
                let full_path = self.Full_Path(file_id);
                if let Err(e) = fs::remove_file(&full_path).await {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        return Err(StorageError::Io(format!("remove_file failed: {}", e)));
                    }
                }
                files.remove(file_id);
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
                            // 僵尸条目：移除索引
                            files.remove(file_id);
                            Ok(false)
                        } else {
                            // 在等锁期间文件被重新创建到磁盘上
                            Ok(true)
                        }
                    } else {
                        Ok(false)
                    }
                }
            }
        } else {
            Ok(false)
        }
    }

    async fn list(&self) -> Result<Vec<FileEntry>, StorageError> {
        let files = self.files.read().await;
        let entries: Vec<FileEntry> = files.iter()
            .map(|(name, state)| self.Build_File_Entry(name, state))
            .collect();
        Ok(entries)
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

    async fn flush(&self) -> Result<(usize, usize), StorageError> {
        let mut discovered: usize = 0;
        let mut cleaned: usize = 0;

        // Phase 1: 扫描磁盘，发现新文件 + 刷新已有文件 size
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
            let actual_size = metadata.len();
            disk_files.insert(file_name_str.clone());

            // 检查是否已在索引中
            {
                let files = self.files.read().await;
                if files.contains_key(&file_name_str) {
                    // 已有文件：刷新 size + 模型元信息（若适用）
                    drop(files);
                    let mut files = self.files.write().await;
                    if let Some(state) = files.get_mut(&file_name_str) {
                        state.size = actual_size;
                    }
                    drop(files);
                    // 对模型文件刷新元信息（已转换 PGGUF 走快速路径，零 I/O）
                    if Self::Is_Model_File(&file_name_str) {
                        let full_path = self.Full_Path(&file_name_str);
                        match analyze_model(&full_path).await {
                            Ok(arch_info) => {
                                let mut files = self.files.write().await;
                                if let Some(state) = files.get_mut(&file_name_str) {
                                    state.model_id = arch_info.model_id;
                                    state.num_layers = Some(arch_info.num_layers as u32);
                                    state.layer_bitmap = arch_info.layer_bitmap;
                                    state.architecture = Some(arch_info.architecture);
                                }
                            }
                            Err(e) => {
                                tracing::info!(
                                    "analyze_model failed for {}: {}",
                                    file_name_str,
                                    e
                                );
                            }
                        }
                    }
                    continue;
                }
            }

            // 不在索引中，插入
            let is_model = Self::Is_Model_File(&file_name_str);
            let mut model_id: Option<u32> = None;
            let mut num_layers: Option<u32> = None;
            let mut layer_bitmap: Option<[u8; 32]> = None;
            let mut architecture: Option<String> = None;

            if is_model {
                // 调用 ML Analyze 获取模型元信息（GGUF 自动转换为 PGGUF）
                let full_path = self.Full_Path(&file_name_str);
                match analyze_model(&full_path).await {
                    Ok(arch_info) => {
                        model_id = arch_info.model_id;
                        num_layers = Some(arch_info.num_layers as u32);
                        layer_bitmap = arch_info.layer_bitmap;
                        architecture = Some(arch_info.architecture);
                    }
                    Err(e) => {
                        // 解析失败不中断 flush，仅跳过元信息填充
                        tracing::warn!("analyze_model failed for {}: {}", file_name_str, e);
                    }
                }
            }

            let mut files = self.files.write().await;
            // 双重检查
            if !files.contains_key(&file_name_str) {
                files.insert(file_name_str.clone(), FileState {
                    lock: Arc::new(RwLock::new(())),
                    size: actual_size,
                    model_id,
                    num_layers,
                    layer_bitmap,
                    architecture,
                });
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
                        drop(_guard);
                        files.remove(&zombie_id);
                        cleaned += 1;
                    }
                    Err(_) => {
                        // 有活跃锁，跳过
                    }
                }
            }
        }

        // 同步模型信息到 PeerManager + 通知 TUI
        self.sync_models_to_peer_manager().await;

        Ok((discovered, cleaned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::time::{timeout, Duration};
    use crate::event_bus::EventBus;
    use crate::peer_management::{PeerManager, Peer_Management_Capability};
    use libp2p::PeerId;

    /// 测试辅助：创建带 stub peer_manager 的 StorageManager
    async fn new_for_test(base_dir: &Path) -> StorageManager {
        let peer_mgr: Arc<dyn Peer_Management_Capability> = Arc::new(PeerManager::default());
        let eb = Arc::new(EventBus::New(1));
        StorageManager::New(base_dir, peer_mgr, eb).await.unwrap()
    }

    // --- 生命周期 ---
    #[tokio::test]
    async fn test_acquire_read_returns_valid_path_and_guard() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
        let (path, wg) = manager.acquire_write("new.txt").await.unwrap();
        // 索引中应存在（即使磁盘文件还没创建）
        tokio::fs::write(&path, b"").await.unwrap();
        drop(wg);
        assert!(manager.exists("new.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_remove_deletes_file_and_index() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
        manager.remove("ghost.txt").await.unwrap();
    }

    // --- 并发锁语义 ---
    #[tokio::test]
    async fn test_multiple_read_guards_concurrent() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
        let (path, wg) = manager.acquire_write("test.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        // 写锁存在时，acquire_read 应阻塞
        let result = timeout(Duration::from_millis(100), manager.acquire_read("test.txt")).await;
        assert!(result.is_err()); // 超时
        drop(wg);
        let (_p, rg) = manager.acquire_read("test.txt").await.unwrap();
        assert_eq!(rg.file_id(), "test.txt");
    }

    #[tokio::test]
    async fn test_write_guard_blocks_second_write() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
        let (_path, _wg) = manager.acquire_write("test.txt").await.unwrap();
        let err = manager.remove("test.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::InUse(_)));
    }

    // --- 惰性发现 ---
    #[tokio::test]
    async fn test_lazy_discover_on_acquire_read() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
        let err = manager.acquire_read("nonexistent.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    // --- 校验码 ---
    #[tokio::test]
    async fn test_checksum_xxhash64() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
        let (path, wg) = manager.acquire_write("test.bin").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(wg);
        let sum = manager.checksum("test.bin", None).await.unwrap();
        assert!(sum.starts_with("xxhash64:"));
    }

    #[tokio::test]
    async fn test_checksum_sha256() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
        let (path, wg) = manager.acquire_write("test.bin").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(wg);
        let sum = manager.checksum("test.bin", Some(ChecksumAlgorithm::Sha256)).await.unwrap();
        assert!(sum.starts_with("sha256:"));
    }

    #[tokio::test]
    async fn test_checksum_blake3() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
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
        let manager = new_for_test(temp_dir.path()).await;
        let err = manager.acquire_read("invalid/path.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::Io(_)));
    }

    #[tokio::test]
    async fn test_reject_hidden_file() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
        let err = manager.acquire_read(".hidden").await.unwrap_err();
        assert!(matches!(err, StorageError::Io(_)));
    }

    #[tokio::test]
    async fn test_reject_empty_file_id() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
        let err = manager.acquire_read("").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    // --- 外部文件处理 ---
    #[tokio::test]
    async fn test_remove_externally_injected_file() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
        tokio::fs::write(temp_dir.path().join("injected.bin"), b"data").await.unwrap();
        let list = manager.list().await.unwrap();
        assert!(!list.iter().any(|e| e.file_name == "injected.bin"));
        manager.remove("injected.bin").await.unwrap();
        assert!(!temp_dir.path().join("injected.bin").exists());
    }

    #[tokio::test]
    async fn test_exists_cleans_zombie_entry() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
        let (path, wg) = manager.acquire_write("zombie.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);
        // 外部删除文件
        tokio::fs::remove_file(temp_dir.path().join("zombie.txt")).await.unwrap();
        assert!(!manager.exists("zombie.txt").await.unwrap());
        let list = manager.list().await.unwrap();
        assert!(!list.iter().any(|e| e.file_name == "zombie.txt"));
    }

    // --- 实时校验码 ---
    #[tokio::test]
    async fn test_checksum_realtime_no_cache() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
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

    // --- list 返回 FileEntry ---
    #[tokio::test]
    async fn test_list_returns_file_entries() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;
        let (path, wg) = manager.acquire_write("test.txt").await.unwrap();
        tokio::fs::write(&path, b"hello").await.unwrap();
        drop(wg);
        manager.flush().await.unwrap(); // size 由 flush 统一刷新
        let entries = manager.list().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_name, "test.txt");
        assert_eq!(entries[0].size, 5);
        assert_eq!(entries[0].model_id, None);
    }

    // --- flush 测试 ---
    #[tokio::test]
    async fn test_flush_discovers_externally_added_files() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;

        // 初始应无文件
        let list = manager.list().await.unwrap();
        assert!(list.is_empty());

        // 外部直接写入文件
        tokio::fs::write(temp_dir.path().join("ext_a.bin"), vec![0u8; 1000]).await.unwrap();
        tokio::fs::write(temp_dir.path().join("ext_b.bin"), vec![0u8; 2000]).await.unwrap();

        // flush 发现新文件
        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 2);
        assert_eq!(cleaned, 0);

        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(manager.exists("ext_a.bin").await.unwrap());
        assert!(manager.exists("ext_b.bin").await.unwrap());
    }

    #[tokio::test]
    async fn test_flush_refreshes_file_size() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;

        // 通过 manager 写入文件
        let (path, wg) = manager.acquire_write("data.bin").await.unwrap();
        tokio::fs::write(&path, vec![0u8; 100]).await.unwrap();
        drop(wg);

        // size 由 flush 统一刷新，直接 list 拿到的是 Ensure_Entry 的初始值 0
        manager.flush().await.unwrap();
        let entries = manager.list().await.unwrap();
        assert_eq!(entries[0].size, 100);

        // 外部增大文件
        tokio::fs::write(temp_dir.path().join("data.bin"), vec![0u8; 5000]).await.unwrap();

        // flush 刷新 size
        manager.flush().await.unwrap();
        let entries = manager.list().await.unwrap();
        assert_eq!(entries[0].size, 5000);
    }

    #[tokio::test]
    async fn test_flush_cleans_zombie_entries() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;

        let (path, wg) = manager.acquire_write("victim.txt").await.unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();
        drop(wg);

        assert!(manager.exists("victim.txt").await.unwrap());

        // 外部直接删除磁盘文件
        tokio::fs::remove_file(temp_dir.path().join("victim.txt")).await.unwrap();

        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 0);
        assert_eq!(cleaned, 1);

        let list = manager.list().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_flush_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;

        tokio::fs::write(temp_dir.path().join("stable.bin"), vec![0u8; 500]).await.unwrap();

        let (d1, c1) = manager.flush().await.unwrap();
        assert_eq!(d1, 1);
        assert_eq!(c1, 0);

        let (d2, c2) = manager.flush().await.unwrap();
        assert_eq!(d2, 0);
        assert_eq!(c2, 0);

        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_flush_ignores_hidden_files_and_directories() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;

        tokio::fs::write(temp_dir.path().join(".hidden"), b"secret").await.unwrap();
        tokio::fs::create_dir(temp_dir.path().join("subdir")).await.unwrap();
        tokio::fs::write(temp_dir.path().join("normal.bin"), b"ok").await.unwrap();

        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 1);
        assert_eq!(cleaned, 0);

        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file_name, "normal.bin");
    }

    #[tokio::test]
    async fn test_flush_mixed_discover_and_clean() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_for_test(temp_dir.path()).await;

        let (p1, wg1) = manager.acquire_write("keep.bin").await.unwrap();
        tokio::fs::write(&p1, b"keep").await.unwrap();
        drop(wg1);

        let (p2, wg2) = manager.acquire_write("remove.bin").await.unwrap();
        tokio::fs::write(&p2, b"remove").await.unwrap();
        drop(wg2);

        tokio::fs::write(temp_dir.path().join("new.bin"), b"new").await.unwrap();
        tokio::fs::remove_file(temp_dir.path().join("remove.bin")).await.unwrap();

        let (discovered, cleaned) = manager.flush().await.unwrap();
        assert_eq!(discovered, 1);
        assert_eq!(cleaned, 1);

        let list = manager.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|e| e.file_name == "keep.bin"));
        assert!(list.iter().any(|e| e.file_name == "new.bin"));
    }
}
