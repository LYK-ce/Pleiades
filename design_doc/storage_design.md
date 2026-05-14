# Storage 设计文档

Presented by KeJi
Date ： 2026-05-14

## 1. 模块概述

`Storage` 模块负责管理 Pleiades 分布式推理系统的**文件存储与模型统一管理**。它不封装 I/O，不参与业务逻辑——只做纯粹的**文件系统层**。

### 核心定义

> **Storage = 文件管理 + 模型统一管理。**
> 统一扁平命名空间，锁与 I/O 解耦。GGUF 文件自动转换为 PGGUF 格式，纳入 model_id 统一管理。
> 只发放 (路径, 锁守卫)，消费方自行决定读写。

### 模块结构

```
Storage/
├── file_entry.rs       ← 数据结构定义（FileEntry）
├── capability.rs       ← Trait 定义（StorageCapability + StorageError + ChecksumAlgorithm）
├── guard.rs            ← 锁守卫定义（ReadGuard, WriteGuard）
├── manager.rs          ← 核心实现（StorageManager: base_dir + files RwLock<HashMap>）
└── mod.rs              ← 模块入口 + 集成测试
```

### 调用关系

```
Network / ML_Engine / Orchestrator
         │
         ▼
  Box<dyn StorageCapability>     ← capability.rs (trait)
         │
         ▼
       StorageManager            ← manager.rs (base_dir + RwLock<HashMap<String, FileState>>)
         │
         ▼
  FileEntry / ReadGuard / WriteGuard ← 数据结构
```

---

## 2. 数据结构

### 2.1 FileEntry — 文件元数据视图

通过 `list()` 返回，与 `PeerManagement::SupportedModel` 元信息一致，各自读取。

```rust
struct FileEntry {
    /// 存储文件名（扁平命名空间）
    file_name: String,
    /// 模型唯一标识（xxhash64(pgguf内容)），非模型文件为 None
    model_id: Option<u64>,
    /// 磁盘文件大小（字节），flush 时统一刷新
    size: u64,
    /// 模型总层数
    num_layers: Option<u32>,
    /// 256 位层位图，bit N = 1 表示持有第 N 层
    layer_bitmap: Option<[u8; 32]>,
    /// 模型架构名（如 qwen3）
    architecture: Option<String>,
}
```

非模型文件所有 `Option` 字段为 `None`。

### 2.2 FileState — 内部文件状态

不对外暴露，用于 StorageManager 内部管理。

```rust
struct FileState {
    lock: Arc<RwLock<()>>,       // 文件读写锁
    size: u64,                   // 磁盘大小，flush 时统一刷新
    model_id: Option<u64>,       // 模型 ID，从 PGGUF 元数据读取
    num_layers: Option<u32>,     // 层数
    layer_bitmap: Option<[u8; 32]>, // 层位图
    architecture: Option<String>,   // 架构名
}
```

### 2.3 ReadGuard / WriteGuard — 锁守卫

```rust
struct ReadGuard {
    file_id: String,
    _guard: OwnedRwLockReadGuard<()>,  // Drop 自动释放
}

struct WriteGuard {
    file_id: String,
    _guard: OwnedRwLockWriteGuard<()>, // Drop 自动释放
}
```

- 多个 `ReadGuard` 可并发共存（共享读）。
- `WriteGuard` 独占，持有期间其他 acquire_read / acquire_write 阻塞。
- Drop 时自动释放锁，无需手动 unlock。

### 2.4 StorageError — 错误类型

```rust
enum StorageError {
    NotFound(String),    // 文件不存在
    InUse(String),       // 文件正被句柄持有
    Io(String),          // 底层 IO 错误
}
```

### 2.5 ChecksumAlgorithm — 校验算法

```rust
enum ChecksumAlgorithm {
    Blake3,              // 默认
    Sha256,
    XxHash64,            // 文件传输校验
}
```

独立于 model_id 计算（model_id 由 ML 层使用 xxhash64 统一计算）。

---

## 3. Trait 定义

### 3.1 StorageCapability

```rust
#[async_trait]
pub trait StorageCapability: Send + Sync {
    // 锁与路径
    async fn acquire_read(&self, file_id: &str) -> Result<(PathBuf, ReadGuard), StorageError>;
    async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError>;

    // 文件操作
    async fn remove(&self, file_id: &str) -> Result<(), StorageError>;
    async fn exists(&self, file_id: &str) -> Result<bool, StorageError>;

    // 文件视图
    async fn list(&self) -> Result<Vec<FileEntry>, StorageError>;

    // 校验码
    async fn checksum(&self, file_id: &str, algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError>;

    // 目录同步
    async fn flush(&self) -> Result<(usize, usize), StorageError>;
}
```

7 个方法（已移除配额管理的 reserve/commit/quota_info 3 个方法）。

---

## 4. 核心实现

### 4.1 StorageManager

```rust
pub struct StorageManager {
    base_dir: PathBuf,
    files: RwLock<HashMap<String, FileState>>,
}
```

**构造器**：
```rust
pub async fn New(base_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
    // 幂等创建目录
    // 扫描目录下现有普通文件，建立索引
    // 忽略隐藏文件（.开头）、子目录
}
```

**锁方案**：`tokio::sync::RwLock<HashMap>`。读多写少场景：query 操作读锁，flush 写锁。

### 4.2 锁与 I/O 解耦

Storage 只发放 `(PathBuf, LockGuard)`，不打开文件。消费方 (Network/ML_Engine) 拿到路径后自行决定同步/异步读写。

约定：
- 持有 `ReadGuard` 期间：文件不会被 remove / acquire_write 修改
- 持有 `WriteGuard` 期间：独占访问
- Drop Guard 后锁释放

### 4.3 路径安全

`Validate_File_Id` 拒绝：
- 空 file_id
- 含有 `/`、`\`、`..`
- 以 `.` 开头（隐藏文件）

### 4.4 惰性发现

`acquire_read` 时若索引不存在但磁盘有文件，自动注册到索引。无需预注册。

### 4.5 flush() 流程

```
flush()
  ├── 扫描 base_dir 下所有普通文件
  ├── 已有文件：re-stat 刷新 FileState.size
  ├── 新文件：插入索引
  │     ├── .gguf/.pgguf 文件 → 调用 ML Analyze 获取模型元信息
  │     └── 非模型文件 → 元信息留空
  └── 清理僵尸条目（索引有、磁盘无，且无活跃锁）
```

返回 `(新发现文件数, 清理僵尸数)`。

### 4.6 exists() 僵尸清理

`exists()` 内部检测：索引存在但磁盘文件消失 → 自动移除索引条目。

---

## 5. 模块导出

```rust
// mod.rs
pub use capability::{StorageCapability, StorageError, ChecksumAlgorithm, FileEntry};
pub use guard::{ReadGuard, WriteGuard};
pub use storage_manager::StorageManager;
```

---

## 6. 与 ML 模块的协作

### 6.1 gguf → pgguf 转换

| 职责 | 负责方 | 说明 |
|------|--------|------|
| 扫描 .gguf/.pgguf 文件 | Storage | flush() 时发现 |
| 格式转换 + 元数据写入 | ML 层 (Analyze) | gguf → pgguf，写入 model_id 等元数据 |
| model_id 计算 | ML 层 (Analyze) | xxhash64 全量内容哈希，写入 PGGUF 元数据 |
| model_id 读取 | Storage | 从 PGGUF 文件元数据中读取，填入 FileEntry |

### 6.2 ML:Analyze 所需提供

- 接收文件路径，识别 gguf/pgguf 格式
- 对 gguf 文件：读取内容 → 转换为 pgguf 格式 → 计算 xxhash64 全量哈希 → 写入 PGGUF 元数据（含 model_id）
- 对已有 pgguf 文件：读取元数据 → 返回元信息（含 model_id、层数、层位图等）
- 返回元信息：model_id、num_layers、layer_bitmap、architecture

---

## 7. 已知风险

### 7.1 flush() 全局写锁

`flush()` 扫描磁盘 + 注册新文件 + 清理僵尸条目，期间持有 `files` 写锁。在包含大量文件的目录中，flush 可能阻塞读操作（`list`, `exists`, `acquire_read`）。

- **当前状态**：flush 非高频操作，影响有限
- **未来优化**：分段释放锁，或使用 DashMap 分片

### 7.2 惰性发现的模型文件

惰性发现 (`Lazy_Discover`) 只为非索引文件建立基本条目。如果文件是 .gguf/.pgguf，model_id 等元信息在惰性发现时为 None，需等待 `flush()` 刷新。

### 7.3 checksum 性能

`checksum()` 实时读取文件内容计算哈希。大模型文件（GB级别）调用代价高。

---

## 8. 重构历史

从 v1 到 v2 的主要变更：

| 变更 | 说明 |
|------|------|
| 删除配额管理 | `QuotaInfo`、`Reservation`、`reserve()`、`commit()`、`quota_info()` |
| 新增 FileEntry | 文件元数据视图，替代 `list() -> Vec<String>` |
| FileState 扩展 | 增加 model_id / num_layers / layer_bitmap / architecture |
| flush 统一刷新 | re-stat 刷新 size，待集成 ML Analyze |
| trait 缩减 | 10 → 7 个方法 |

---

## 9. TODO

### flush() 未集成 ML Analyze

当前 `flush()` 中 `.gguf`/`.pgguf` 模型文件的元信息获取逻辑已标记为 TODO，等待 ML 模块 `Analyze` 就绪后接入：

```rust
// storage_manager.rs flush() 中
if is_model {
    // TODO: Phase 3 — 待 ML 模块 Analyze 就绪后调用
    // let result = ml_analyze(&full_path, &file_name_str).await?;
}
```

当前行为：模型文件在 flush 时仅以 `model_id = None` 等空元信息注册。待 ML Analyze 就绪后，需解除注释并传入 ML 模块引用。

