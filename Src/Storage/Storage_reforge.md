# Storage Reforge 设计文档

## 一、背景

当前 Storage 设计将**锁管理**与**文件 I/O**绑定在一起：`ReadHandle` / `WriteHandle` 内部同时持有 `tokio::fs::File`（异步文件句柄）和 `OwnedRwLockGuard`（锁守卫）。

这在实际使用中产生了问题：

1. **ML Engine 需要同步 I/O** — Candle 的 GGUF 解析器要求 `std::io::Read + Seek`，无法使用 `AsyncRead`
2. **Network 模块有自己的 I/O 模式** — 文件传输走 libp2p 流协议，不直接使用 tokio::fs::File
3. **多次打开同一文件** — ML Engine 的模型加载过程中同一文件会被 `std::fs::File::open()` 打开多次（每层一次），无法由单个 ReadHandle 覆盖

**核心洞察**：Storage 的真正价值是**锁管理 + 路径解析 + 文件注册表**，而不是封装 I/O。消费模块应自行决定如何读写文件。

---

## 二、设计原则

1. **锁与 I/O 解耦** — Storage 只管发放 `(路径, 锁守卫)`，不打开文件
2. **透明守卫** — `ReadGuard` / `WriteGuard` 仅包含锁，持有即锁定，Drop 即释放
3. **模块自主 I/O** — ML Engine 用 `std::fs::File`，Network 用 `tokio::fs::File`，各取所需
4. **保持并发安全** — 读写锁语义不变：多读互斥写，写独占

---

## 三、新架构

```
┌──────────────────────────────────────────────────────┐
│                   消费模块                             │
│                                                      │
│  ML Engine                    Network                │
│  ┌─────────────────┐         ┌─────────────────┐     │
│  │ std::fs::File:: │         │ tokio::fs::File::│     │
│  │ open(path)      │         │ open(path).await │     │
│  └────────┬────────┘         └────────┬────────┘     │
│           │                           │              │
│           ▼                           ▼              │
│       PathBuf                     PathBuf            │
│       + ReadGuard                 + WriteGuard       │
└──────────┬───────────────────────────┬───────────────┘
           │                           │
           ▼                           ▼
┌──────────────────────────────────────────────────────┐
│              StorageManager                           │
│                                                      │
│  acquire_read(file_id)  → (PathBuf, ReadGuard)       │
│  acquire_write(file_id) → (PathBuf, WriteGuard)      │
│                                                      │
│  ┌────────────────────────────────────────────┐      │
│  │  files: RwLock<HashMap<String, FileState>>  │      │
│  │                                             │      │
│  │  FileState { lock: Arc<RwLock<()>> }        │      │
│  └────────────────────────────────────────────┘      │
│                                                      │
│  base_dir: PathBuf                                   │
│  resolve: file_id → base_dir.join(file_id)           │
└──────────────────────────────────────────────────────┘
```

---

## 四、文件组成

```
Src/Storage/
├── mod.rs              # 模块入口：聚合导出
├── capability.rs       # 契约层：StorageCapability trait、StorageError、ChecksumAlgorithm
├── guard.rs            # 守卫层：ReadGuard / WriteGuard（替代原 handle.rs）
└── manager.rs          # 实现层：StorageManager
```

### 变更对比

| 旧设计 | 新设计 | 说明 |
|--------|--------|------|
| `handle.rs` — ReadHandle / WriteHandle | `guard.rs` — ReadGuard / WriteGuard | 移除内嵌的 `tokio::fs::File`，只保留锁守卫 |
| `open_read()` → ReadHandle | `acquire_read()` → (PathBuf, ReadGuard) | 不打开文件，只返回路径+锁 |
| `open_write()` → WriteHandle | `acquire_write()` → (PathBuf, WriteGuard) | 不打开文件，只返回路径+锁 |
| ReadHandle impl AsyncRead + AsyncSeek | 移除 | 模块自行打开文件 |
| WriteHandle impl AsyncWrite + AsyncSeek | 移除 | 模块自行打开文件 |

---

## 五、类型定义

### 5.1 guard.rs — 守卫层

```rust
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard};

/// 读锁守卫 — 持有期间文件不会被 remove / acquire_write 修改
///
/// 多个 ReadGuard 可并发共存（共享读）。
/// Drop 时自动释放锁。
pub struct ReadGuard {
    pub(crate) file_id: String,
    pub(crate) _guard: OwnedRwLockReadGuard<()>,
}

/// 写锁守卫 — 持有期间文件独占
///
/// 同一时刻只能有一个 WriteGuard（排他写）。
/// Drop 时自动释放锁。
pub struct WriteGuard {
    pub(crate) file_id: String,
    pub(crate) _guard: OwnedRwLockWriteGuard<()>,
}

impl ReadGuard {
    /// 返回受保护的 file_id
    pub fn file_id(&self) -> &str {
        &self.file_id
    }
}

impl WriteGuard {
    /// 返回受保护的 file_id
    pub fn file_id(&self) -> &str {
        &self.file_id
    }
}
```

与旧设计对比：

| | 旧 ReadHandle | 新 ReadGuard |
|--|--|--|
| 字段 | file_id, **file: File**, _guard | file_id, _guard |
| 大小 | 大（含文件句柄） | 小（仅锁守卫） |
| impl | AsyncRead, AsyncSeek | 无 I/O trait |
| 用途 | 锁 + I/O | 仅锁 |

### 5.2 capability.rs — 契约层

```rust
use std::path::PathBuf;
use async_trait::async_trait;
use super::guard::{ReadGuard, WriteGuard};

/// 存储模块错误枚举
#[derive(Debug)]
pub enum StorageError {
    /// 文件不存在
    NotFound(String),
    /// 文件正被句柄持有，操作失败
    InUse(String),
    /// 底层 IO 错误
    Io(String),
}

/// 校验算法枚举（不变）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Blake3,
    Sha256,
    XxHash64,
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
```

### 5.3 manager.rs — 实现层

```rust
pub struct StorageManager {
    base_dir: PathBuf,
    files: RwLock<HashMap<String, FileState>>,
}

struct FileState {
    lock: Arc<RwLock<()>>,
}
```

核心方法实现变更：

```rust
#[async_trait]
impl StorageCapability for StorageManager {
    async fn acquire_read(&self, file_id: &str) -> Result<(PathBuf, ReadGuard), StorageError> {
        Self::validate_file_id(file_id)?;
        let lock = self.lazy_discover(file_id).await?;
        let guard = lock.read_owned().await;
        let path = self.full_path(file_id);
        Ok((path, ReadGuard {
            file_id: file_id.to_string(),
            _guard: guard,
        }))
    }

    async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError> {
        Self::validate_file_id(file_id)?;
        let lock = self.ensure_entry(file_id).await;
        let guard = lock.write_owned().await;
        let path = self.full_path(file_id);
        Ok((path, WriteGuard {
            file_id: file_id.to_string(),
            _guard: guard,
        }))
    }

    // remove, exists, list — 逻辑基本不变（移除 File 相关部分）
    
    async fn checksum(&self, file_id: &str, algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError> {
        Self::validate_file_id(file_id)?;
        let algo = algo.unwrap_or_default();
        // 获取读锁
        let (path, _guard) = self.acquire_read(file_id).await?;
        // 自行打开文件计算校验码（std::fs::File 或 tokio::fs::File 均可）
        let mut file = tokio::fs::File::open(&path).await
            .map_err(|e| StorageError::Io(format!("open failed: {}", e)))?;
        // ... 计算哈希（逻辑不变）
    }
}
```

### 5.4 mod.rs — 聚合导出

```rust
pub use capability::{StorageCapability, StorageError, ChecksumAlgorithm};
pub use guard::{ReadGuard, WriteGuard};
pub use manager::StorageManager;

mod capability;
mod guard;
mod manager;
```

---

## 六、使用示例

### 6.1 ML Engine（同步 I/O）

```rust
// ML_Engine_Service 内部
async fn Create_Session(&self, config: ML_Session_Config, io_handle: IoHandle) -> ... {
    // 1. 通过 Storage 获取路径 + 读锁
    let (model_path, read_guard) = self.storage
        .acquire_read(&config.model_file_id)
        .await
        .map_err(|e| ML_Engine_Error::SessionCreationFailed(e.to_string()))?;

    // 2. 启动 OS 线程，传入 model_path
    //    线程内用 std::fs::File::open(model_path) 读模型（多次）
    //    Candle 的同步 I/O 正常工作
    
    // 3. 保存 read_guard 到 Session 注册表
    //    Session 存活期间，文件受锁保护
    self.sessions.lock().await.insert(session_id, SessionEntry {
        handle: session_handle,
        _storage_guard: read_guard,
    });
}

// Shutdown 时
async fn Shutdown_Session(&self, session_id: &str) -> ... {
    let entry = self.sessions.lock().await.remove(session_id);
    // entry drop → read_guard drop → Storage 读锁释放
    if let Some(entry) = entry {
        entry.handle.Shutdown().await?;
    }
}
```

### 6.2 Network 文件传输（异步 I/O）

```rust
// 网络模块接收文件
async fn receive_file(storage: &StorageManager, file_id: &str, stream: &mut TcpStream) {
    // 1. 获取路径 + 写锁
    let (path, _write_guard) = storage.acquire_write(file_id).await?;
    
    // 2. 自行创建文件并写入
    let mut file = tokio::fs::File::create(&path).await?;
    tokio::io::copy(stream, &mut file).await?;
    
    // 3. _write_guard drop → 写锁释放
}

// 网络模块发送文件
async fn send_file(storage: &StorageManager, file_id: &str, stream: &mut TcpStream) {
    // 1. 获取路径 + 读锁
    let (path, _read_guard) = storage.acquire_read(file_id).await?;
    
    // 2. 自行打开文件并读取
    let mut file = tokio::fs::File::open(&path).await?;
    tokio::io::copy(&mut file, stream).await?;
}
```

### 6.3 模型切分（读源文件 + 写输出文件）

```rust
async fn Split_Model(&self, source_id: &str, start: usize, end: usize, output_id: &str) -> ... {
    // 读锁保护源文件
    let (source_path, _read_guard) = self.storage.acquire_read(source_id).await?;
    // 写锁保护输出文件
    let (output_path, _write_guard) = self.storage.acquire_write(output_id).await?;
    
    // 底层用 std::fs::File 进行同步切分
    gguf_model_manager::GGUF_Split_Model(&source_path, start, end, &output_path)?;
}
```

---

## 七、并发语义（不变）

| 操作 | ReadGuard 存在 | WriteGuard 存在 | 无守卫 |
|------|--------------|----------------|--------|
| acquire_read | ✅ 立即返回 | ⏳ 阻塞等待 | ✅ 立即返回 |
| acquire_write | ⏳ 阻塞等待 | ⏳ 阻塞等待 | ✅ 立即返回 |
| remove | ❌ InUse | ❌ InUse | ✅ 删除 |

多个 ReadGuard 可并发共存（共享读）。WriteGuard 独占。

---

## 八、迁移清单

| 变更 | 文件 | 说明 |
|------|------|------|
| 删除 | `Src/Storage/handle.rs` | ReadHandle / WriteHandle 移除 |
| 新增 | `Src/Storage/guard.rs` | ReadGuard / WriteGuard |
| 修改 | `Src/Storage/capability.rs` | `open_read` → `acquire_read`，`open_write` → `acquire_write`；移除 ReadHandle/WriteHandle 引用 |
| 修改 | `Src/Storage/manager.rs` | impl 改为返回 `(PathBuf, Guard)`；checksum 内部自行打开文件 |
| 修改 | `Src/Storage/mod.rs` | 导出 ReadGuard/WriteGuard 替代 ReadHandle/WriteHandle |
| 修改 | 所有 Storage 消费方 | 适配新 API（当前主要是测试代码） |

---

## 九、测试策略

### 9.1 guard.rs 测试

```rust
// test_read_guard_file_id        — file_id() 返回正确
// test_write_guard_file_id       — file_id() 返回正确
// test_guard_drop_releases_lock  — Drop 后锁释放（通过 try_write 验证）
```

### 9.2 manager.rs 测试

```rust
// --- 生命周期 ---
// test_acquire_read_returns_valid_path_and_guard
// test_acquire_write_returns_valid_path_and_guard
// test_acquire_write_creates_index_entry
// test_remove_deletes_file_and_index
// test_remove_idempotent_on_missing

// --- 并发锁语义 ---
// test_multiple_read_guards_concurrent
// test_write_guard_blocks_read
// test_write_guard_blocks_second_write
// test_remove_returns_inuse_when_read_guard_alive
// test_remove_returns_inuse_when_write_guard_alive

// --- 惰性发现 ---
// test_lazy_discover_on_acquire_read
// test_lazy_discover_not_found

// --- 校验码 ---
// test_checksum_xxhash64
// test_checksum_sha256
// test_checksum_blake3

// --- 路径安全 ---
// test_reject_path_traversal
// test_reject_hidden_file
// test_reject_empty_file_id
```

### 9.3 mod.rs 集成测试

```rust
// test_module_imports_compile
// test_end_to_end_write_read_remove
//   acquire_write → 用返回的 PathBuf 创建文件并写入 → drop guard
//   acquire_read → 用返回的 PathBuf 打开文件并读取 → 验证内容 → drop guard
//   remove → 验证删除
```
