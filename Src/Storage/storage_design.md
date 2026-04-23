# StorageManager 设计文档（Phase 2）

## 一、设计概述

StorageManager 为系统提供**统一扁平命名空间**下的文件存储与并发访问能力。所有文件存放于单一目录，支持外部直接注入；通过进程内文件级读写锁实现并发安全；提供分离的读写句柄供网络模块与 ML Engine 进行流式 IO；支持按需实时计算文件校验码。

---

## 二、文件组成

模块位于 `src/storage/` 目录，由 4 个源文件组成。所有单元测试均以 `#[cfg(test)]` 内联模块形式直接写在对应源文件底部。

```
src/storage/
├── mod.rs              # 模块入口：聚合导出
├── capability.rs       # 契约层：trait、错误类型、校验算法、句柄声明
├── manager.rs          # 实现层：StorageManager 结构体与全部业务逻辑
└── handle.rs           # 句柄层：ReadHandle / WriteHandle 定义与标准 IO trait 实现
```

| 文件 | 职责 | 公开内容 |
|------|------|---------|
| `capability.rs` | 定义 `StorageCapability` trait、`StorageError`、`ChecksumAlgorithm`、`ReadHandle`/`WriteHandle` 的结构声明 | 全部公开 |
| `handle.rs` | 实现 `ReadHandle` 与 `WriteHandle`，封装 `tokio::fs::File` + `OwnedRwLockGuard`，实现 `AsyncRead`/`AsyncWrite`/`AsyncSeek` | `ReadHandle`、`WriteHandle` |
| `manager.rs` | `StorageManager` 结构体；目录扫描重建、惰性发现、锁表管理、`remove` 探测、校验码实时计算 | `StorageManager` |
| `mod.rs` | 通过 `pub use` 将上述类型提升到模块级，供外部统一导入；底部可附模块级集成测试 | 模块聚合 |

---

## 三、capability.rs — 契约层

### 3.1 错误类型

```rust
pub enum StorageError {
    /// 文件不存在（open_read 时；或惰性发现时磁盘也不存在）
    NotFound(String),
    /// 文件正被句柄持有，remove 失败
    InUse(String),
    /// 底层 IO 错误，保留原始错误信息
    Io(String),
}
```

- 实现了 `Debug` trait，便于日志输出。
- 实现了 `std::error::Error` trait，可向上层传递。
- 实现了 `Display` trait，错误信息格式为 `"NotFound: {msg}"`、`"InUse: {msg}"`、`"Io: {msg}"`。

### 3.2 校验算法

```rust
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
```

- 支持三种哈希算法：Blake3（快速、安全）、SHA-256（兼容性）、XxHash64（默认，高性能）。
- 派生 `Clone`、`Copy`、`Debug`、`PartialEq`、`Eq`，便于配置传递与比较。

### 3.3 句柄类型引用

`StorageCapability` trait 中使用的 `ReadHandle` 和 `WriteHandle` 类型定义于 `handle.rs` 模块，通过 `use super::handle::{ReadHandle, WriteHandle};` 引入。外部用户应通过 `mod.rs` 的统一导出使用这些类型。

### 3.4 Trait 定义

```rust
#[async_trait]
pub trait StorageCapability: Send + Sync {
    /// 共享读模式打开。惰性发现；写锁占用时阻塞等待。
    async fn open_read(&self, file_id: &str) -> Result<ReadHandle, StorageError>;

    /// 独占写模式打开。不存在则创建，存在则截断；任何锁占用时阻塞等待。
    async fn open_write(&self, file_id: &str) -> Result<WriteHandle, StorageError>;

    /// 删除文件。try_write 探测：有活跃句柄返回 InUse，无则删文件并清索引。
    async fn remove(&self, file_id: &str) -> Result<(), StorageError>;

    /// 检查文件是否存在于索引且磁盘存在。
    async fn exists(&self, file_id: &str) -> Result<bool, StorageError>;

    /// 列出索引中所有 file_id。
    async fn list(&self) -> Result<Vec<String>, StorageError>;

    /// 实时计算校验码。不缓存。计算时持有共享读锁。
    /// 返回格式: "{algo}:{hex}"，如 "xxhash64:deadbeef..."。
    async fn checksum(
        &self,
        file_id: &str,
        algo: Option<ChecksumAlgorithm>,
    ) -> Result<String, StorageError>;
}
```

### 3.5 内联测试（capability.rs 底部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_algorithm_default() {
        assert!(matches!(ChecksumAlgorithm::default(), ChecksumAlgorithm::XxHash64));
    }

    #[test]
    fn test_storage_error_display() {
        assert_eq!(
            StorageError::NotFound("test.txt".to_string()).to_string(),
            "NotFound: test.txt"
        );
        assert_eq!(
            StorageError::InUse("test.txt".to_string()).to_string(),
            "InUse: test.txt"
        );
        assert_eq!(
            StorageError::Io("Permission denied".to_string()).to_string(),
            "Io: Permission denied"
        );
    }
}
```

---

## 四、handle.rs — 句柄层

### 4.1 结构定义

```rust
pub struct ReadHandle {
    file_id: String,
    file: tokio::fs::File,
    _guard: tokio::sync::OwnedRwLockReadGuard<()>,
}

pub struct WriteHandle {
    file_id: String,
    file: tokio::fs::File,
    _guard: tokio::sync::OwnedRwLockWriteGuard<()>,
}
```

### 4.2 公开方法

```rust
impl ReadHandle {
    pub fn file_id(&self) -> &str { &self.file_id }
}

impl WriteHandle {
    pub fn file_id(&self) -> &str { &self.file_id }
}
```

### 4.3 标准 IO Trait 实现

- `ReadHandle`：`impl AsyncRead for ReadHandle`、`impl AsyncSeek for ReadHandle`
- `WriteHandle`：`impl AsyncWrite for WriteHandle`、`impl AsyncSeek for WriteHandle`

### 4.4 内联测试（handle.rs 底部）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, AsyncSeekExt};

    // 测试 ReadHandle / WriteHandle 的 file_id() 返回正确
    // 测试 ReadHandle 通过 AsyncRead 能正确读取内容
    // 测试 WriteHandle 通过 AsyncWrite 写入后，配合 AsyncSeek 能回读验证
    // 测试句柄 Drop 后，对应 FileState 的锁被释放（通过 manager 侧二次获取验证）
}
```

---

## 五、manager.rs — 实现层

### 5.1 内部结构

```rust
pub struct StorageManager {
    base_dir: PathBuf,
    files: tokio::sync::RwLock<HashMap<String, FileState>>,
}

struct FileState {
    lock: Arc<tokio::sync::RwLock<()>>,
}
```

### 5.2 构造与扫描

`StorageManager::new(base_dir)`：
1. `create_dir_all(&base_dir)` 幂等创建。
2. 遍历目录，忽略以 `.` 开头的隐藏文件/目录。
3. 对每个文件名创建 `FileState`（含 `Arc<RwLock<()>>`），插入 `files`。
4. **仅扫描文件名，不读取内容，不计算校验码。**

### 5.3 惰性发现

`open_read` / `open_write` 流程：
1. 读锁查 `files`，命中则直接对该 `FileState` 申请 `lock.read()` 或 `lock.write()`。
2. 未命中则释放读锁，检查磁盘 `base_dir.join(file_id)` 是否存在。
   - 存在：创建 `FileState`，写锁插入索引，再申请文件锁。
   - 不存在：`Read` 返回 `NotFound`；`Write` 创建新条目并持有写锁，同时创建磁盘文件。

### 5.4 路径安全

`file_id` sanitize 规则：
- 拒绝包含 `/`、`\`、`..`、空字符串。
- 拒绝以 `.` 开头的 ID。
- 使用 `PathBuf::push` 拼接，禁止字符串拼接。

### 5.5 remove 语义

1. 查表获取 `FileState`。
2. 对 `FileState.lock` 执行 `try_write()`。
   - 成功：获取外层索引写锁，移除 `FileState`，删除磁盘文件，返回 `Ok`。
   - 失败：返回 `InUse`。
3. 若索引和磁盘均无该文件，幂等返回 `Ok`。

### 5.6 校验码计算

1. `open_read` 获取共享读锁（或直接对 `FileState` 获取读锁）。
2. 全量读取文件内容，边读边计算哈希。
3. 返回 `{algo}:{hex}` 字符串。
4. **不写入缓存，不修改索引。**

### 5.7 内联测试（manager.rs 底部）

这是测试的核心文件。所有测试均使用 `tokio::test` 与 `tempfile::TempDir` 保证隔离。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- 生命周期与幂等性 ---
    // test_open_write_creates_file_and_index_entry
    // test_open_write_truncates_existing
    // test_open_read_not_found_returns_error
    // test_remove_deletes_file_and_removes_index
    // test_remove_idempotent_on_missing_file

    // --- 并发与锁语义 ---
    // test_multiple_readers_concurrent_succeed
    // test_write_blocks_read_until_drop
    // test_write_blocks_second_write_until_drop
    // test_remove_returns_inuse_when_read_handle_alive
    // test_remove_returns_inuse_when_write_handle_alive
    // test_read_unblocks_after_write_handle_dropped

    // --- 惰性发现 ---
    // test_lazy_discovery_on_read_for_externally_created_file
    // test_lazy_discovery_on_write_for_externally_deleted_file

    // --- 校验码 ---
    // test_checksum_default_xxhash64_format
    // test_checksum_sha256_format
    // test_checksum_realtime_no_cache
    // test_checksum_holds_read_lock_blocking_write

    // --- 错误传播 ---
    // test_io_error_mapped_to_storage_error
}
```

---

## 六、mod.rs — 模块入口

### 6.1 聚合导出

```rust
pub use capability::{StorageCapability, StorageError, ChecksumAlgorithm};
pub use handle::{ReadHandle, WriteHandle};
pub use manager::StorageManager;
```

### 6.2 内联测试（mod.rs 底部）

模块级集成测试，验证外部视角的导入与基础端到端流程。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // test_module_imports_compile_and_are_accessible
    // test_end_to_end_write_read_remove_lifecycle
}
```

---

## 七、开放接口汇总

### 7.1 核心 Trait

```rust
#[async_trait]
pub trait StorageCapability: Send + Sync {
    async fn open_read(&self, file_id: &str) -> Result<ReadHandle, StorageError>;
    async fn open_write(&self, file_id: &str) -> Result<WriteHandle, StorageError>;
    async fn remove(&self, file_id: &str) -> Result<(), StorageError>;
    async fn exists(&self, file_id: &str) -> Result<bool, StorageError>;
    async fn list(&self) -> Result<Vec<String>, StorageError>;
    async fn checksum(
        &self,
        file_id: &str,
        algo: Option<ChecksumAlgorithm>,
    ) -> Result<String, StorageError>;
}
```

### 7.2 句柄公开能力

- `ReadHandle`：`file_id()`、`AsyncRead`、`AsyncSeek`
- `WriteHandle`：`file_id()`、`AsyncWrite`、`AsyncSeek`

### 7.3 接口用法示例

#### open_read

打开文件进行共享读取。如果文件不存在，返回 `StorageError::NotFound`；如果文件正被写入锁占用，调用者将阻塞直到锁释放。

```rust
use pleiades::storage::{StorageCapability, StorageManager};

let manager = StorageManager::new("/tmp/storage").await?;
let handle = manager.open_read("data.bin").await?;
let mut buf = Vec::new();
tokio::io::copy(&mut handle, &mut buf).await?;
```

#### open_write

打开文件进行独占写入。如果文件不存在，则创建；如果文件已存在，则截断为0字节。如果文件正被任何读锁或写锁占用，调用者将阻塞直到锁释放。

```rust
let handle = manager.open_write("output.bin").await?;
handle.write_all(b"hello world").await?;
// 句柄丢弃时自动刷新并释放锁
```

#### remove

删除文件。如果文件正被任何句柄持有（读或写），返回 `StorageError::InUse`；否则删除磁盘文件并清理内部索引。如果文件不存在（索引和磁盘均无），幂等返回 `Ok(())`。

```rust
manager.remove("temp.bin").await?;
```

#### exists

检查文件是否存在。返回 `Ok(true)` 当且仅当文件在索引中且磁盘文件存在。如果索引中存在但磁盘文件已被外部删除，本方法会惰性清理索引并返回 `Ok(false)`。

```rust
let exists = manager.exists("data.bin").await?;
```

#### list

列出索引中所有已知的文件 ID。注意：可能包含“僵尸条目”（索引中存在但磁盘文件已被外部删除且尚未被 `exists` 或 `open_read` 清理）。外部用户应结合 `exists` 或直接尝试 `open_read` 来验证文件实际可用性。

```rust
let ids = manager.list().await?;
for id in ids {
    println!("{}", id);
}
```

#### checksum

实时计算文件的校验码。计算过程中持有共享读锁，保证计算期间文件内容不变。返回格式为 `"{algo}:{hex}"`，例如 `"xxhash64:deadbeef..."`。

```rust
let sum = manager.checksum("data.bin", None).await?; // 使用默认算法（XxHash64）
let sum_sha = manager.checksum("data.bin", Some(ChecksumAlgorithm::Sha256)).await?;
```

### 7.4 错误处理

所有方法均返回 `Result<_, StorageError>`，错误变体包括：

- `NotFound(String)`：文件不存在（open_read 或惰性发现时磁盘也不存在）。
- `InUse(String)`：文件正被句柄持有，remove 操作被拒绝。
- `Io(String)`：底层 IO 错误（如权限不足、磁盘满等），原始错误信息被保留。

外部调用者应使用 `match` 或 `?` 运算符进行错误处理。

---

## 八、单元测试编写规范

### 8.1 位置
每个源文件底部均包含：
```rust
#[cfg(test)]
mod tests {
    use super::*;
    // ...
}
```

### 8.2 异步运行时
所有测试使用 `#[tokio::test]`。

### 8.3 目录隔离
每个测试函数内部独立创建 `tempfile::TempDir`，作为 `StorageManager::new(temp_dir.path()).await` 的 `base_dir`，确保：
- 测试并行执行不冲突。
- 测试结束后临时目录自动清理。

### 8.4 并发测试防挂死
涉及"阻塞等待"的测试（如 `test_write_blocks_read_until_drop`）必须使用 `tokio::time::timeout(Duration::from_secs(5), ...).await` 包装，确保测试失败时不会永久挂死。

### 8.5 外部文件模拟
惰性发现测试中，使用 `tokio::fs::write(manager.base_dir().join("injected.bin"), b"data").await` 模拟外部进程直接落盘，验证 StorageManager 在不重启情况下的发现能力。

### 8.6 锁释放验证
句柄 Drop 测试不直接测锁内部状态，而是通过行为验证：
1. `open_write` 获取独占锁。
2. 在另一任务尝试 `open_read`，确认其阻塞。
3. 主动 `drop(write_handle)`。
4. 确认 `open_read` 在短暂延迟后成功解除阻塞。

---

## 九、异常情况处理

### 9.1 外部文件注入

当外部进程（或用户）直接向存储目录拷贝文件时，StorageManager 通过**惰性发现**机制将其纳入管理：

- 在 `open_read` 或 `open_write` 调用时，若索引中无对应条目，会检查磁盘文件是否存在。
- 若存在，则创建 `FileState` 并插入索引，后续操作与普通文件无异。
- 若不存在，`open_read` 返回 `NotFound`，`open_write` 创建新文件。

**注意**：外部注入的文件在首次被访问前不会出现在 `list()` 结果中（因为索引尚未建立）。调用 `list()` 后立即注入的文件，在下次 `list()` 前不会被发现。

### 9.2 外部文件删除（僵尸条目）

当外部进程删除磁盘文件后，索引中对应的 `FileState` 成为“僵尸条目”。StorageManager 提供以下惰性清理机制：

- `exists()`：发现磁盘文件不存在时，自动清理索引中的僵尸条目，返回 `false`。
- `open_read()`：发现磁盘文件不存在时，返回 `NotFound`，同时清理索引。
- `list()`：**不会**触发清理，因此返回的列表可能包含僵尸条目。外部用户应结合 `exists()` 或直接尝试 `open_read()` 来验证文件实际可用性。

清理过程采用**双重检查**避免竞态：先释放索引读锁，再检查磁盘，最后获取写锁删除条目。这保证了在 `open_write` 并发创建同名文件时不会误删新条目。

### 9.3 open_write 失败后的索引回滚

如果 `File::create` 失败（磁盘满、权限拒绝等），`open_write` 会在返回 `Io` 错误前，将之前插入索引的 `FileState` 移除，避免留下僵尸条目。

### 9.4 remove 对外部注入文件的幂等删除

当调用 `remove(file_id)` 而索引中无对应条目时，StorageManager 会额外检查磁盘文件是否存在。若存在，则直接删除磁盘文件；若不存在，直接返回 `Ok(())`。这保证了即使文件从未被 StorageManager 管理（外部注入），也能被正确删除。

### 9.5 竞态窗口与一致性保证

- **exists 清理竞态**：在释放读锁后、获取写锁前，另一个任务可能通过 `open_write` 重新创建了该文件。此时清理索引会导致索引与磁盘不一致。当前实现通过二次磁盘检查避免此问题：在获取写锁后再次确认文件是否仍然不存在，若已存在则放弃清理。
- **并发锁保证**：文件级读写锁基于 `tokio::sync::RwLock`，保证同一文件内读写互斥，不同文件之间互不干扰。
- **索引与磁盘的最终一致性**：索引是磁盘状态的缓存，可能短暂不一致（如外部删除后、下次 `exists` 前）。但所有对外接口均以磁盘最终状态为准，不会返回已删除文件的内容。

---

## 十、Phase 2 范围

| 功能 | 状态 |
|------|------|
| 统一扁平目录与惰性发现 | ✅ 必须实现 |
| 文件级 `RwLock` 并发控制 | ✅ 必须实现 |
| 分离读写句柄（`ReadHandle` / `WriteHandle`） | ✅ 必须实现 |
| `remove` 的 `try_write` 探测 | ✅ 必须实现 |
| 实时校验码（默认 XxHash64，可配置） | ✅ 必须实现 |
| 引用计数（Phase 3 共享缓存） | ⏳ 待设计 |
| 磁盘配额/限流 | ⏳ 待设计 |
| 加密/沙箱 | ⏳ 待设计 |