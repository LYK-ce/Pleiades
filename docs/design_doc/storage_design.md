# Storage 设计文档

Presented by KeJi
Date ： 2026-05-19

## 1. 模块概述

`Storage` 是 Pleiades 的**本地文件存储管理器**，负责文件锁管理、路径解析、索引维护和模型元信息追踪。

### 核心定义

> **StorageManager = 扁平文件命名空间 + RwLock 索引 + Guard 模式。**
> 不封装 I/O，消费模块通过返回的 `PathBuf` 自行读写。
> 基于 `tokio::sync::RwLock` 实现并发安全的文件访问控制。
> 模型文件 (.gguf/.pgguf) 的层位图、架构名等元信息通过 `flush()` 统一刷新。

### 模块结构

```
Storage/
├── mod.rs               ← 模块入口 + re-export
├── capability.rs        ← StorageCapability trait + StorageError + FileEntry
├── guard.rs             ← ReadGuard / WriteGuard（RAII 锁守卫）
└── storage_manager.rs   ← StorageManager 实现
```

以及与 Lua 桥接的适配层：

```
Lua/
└── storage_handle.rs    ← StorageReadHandle / StorageWriteHandle（Lua UserData）
```

### 架构总览

```
┌──────────┐  ┌──────────┐  ┌──────────┐
│   main   │  │  Lua     │  │ Network  │
│ (初始化)  │  │caps.storage│ │File_Stream│
└────┬─────┘  └────┬─────┘  └────┬─────┘
     │             │              │
     ▼             ▼              ▼
┌─────────────────────────────────────────────────┐
│              Arc<dyn StorageCapability>          │
│                                                 │
│  ┌─────────────────────────────────────────┐    │
│  │          StorageManager                 │    │
│  │  ┌──────────────────────────────────┐   │    │
│  │  │  files: RwLock<HashMap<String,   │   │    │
│  │  │          FileState>>              │   │    │
│  │  │  ┌─────────┐ ┌─────────┐         │   │    │
│  │  │  │FileState│ │FileState│ ...     │   │    │
│  │  │  │lock:Arc │ │lock:Arc │         │   │    │
│  │  │  │size:u64 │ │size:u64 │         │   │    │
│  │  │  └────┬────┘ └────┬────┘         │   │    │
│  │  │       │           │              │   │    │
│  │  │       ▼           ▼              │   │    │
│  │  │  ┌──────────────────────────┐    │   │    │
│  │  │  │   base_dir (文件系统)    │    │   │    │
│  │  │  └──────────────────────────┘    │   │    │
│  │  └──────────────────────────────────┘   │    │
│  └─────────────────────────────────────────┘    │
└─────────────────────────────────────────────────┘
```

---

## 2. 数据结构

### 2.1 FileState — 内部文件状态

```rust
#[derive(Debug)]
struct FileState {
    lock: Arc<RwLock<()>>,          // 文件级读写锁（共享读/排他写）
    size: u64,                       // 磁盘大小（字节），flush 时统一刷新
    model_id: Option<u64>,           // 模型唯一标识（xxhash64）
    num_layers: Option<u32>,         // 模型总层数
    layer_bitmap: Option<[u8; 32]>,  // 256 位层位图，bit N = 1 表示持有第 N 层
    architecture: Option<String>,    // 模型架构名（如 qwen3）
}
```

### 2.2 StorageManager — 管理器

```rust
#[derive(Debug)]
pub struct StorageManager {
    base_dir: PathBuf,                          // 文件存储根目录
    files: RwLock<HashMap<String, FileState>>,   // file_id → 文件状态索引
}
```

### 2.3 ReadGuard / WriteGuard — 锁守卫

```rust
pub struct ReadGuard {
    file_id: String,
    _guard: OwnedRwLockReadGuard<()>,
}

pub struct WriteGuard {
    file_id: String,
    _guard: OwnedRwLockWriteGuard<()>,
}
```

| Guard | 语义 | 并发数 | Drop 行为 |
|-------|------|--------|-----------|
| `ReadGuard` | 共享读 | 多个可共存 | 释放 `FileState.lock` 读锁 |
| `WriteGuard` | 排他写 | 同时仅一个 | 释放 `FileState.lock` 写锁 |

### 2.4 FileEntry — 外部视图

```rust
pub struct FileEntry {
    pub file_name: String,           // 扁平命名空间中的文件名
    pub model_id: Option<u64>,       // 模型唯一标识
    pub size: u64,                   // 磁盘文件大小
    pub num_layers: Option<u32>,     // 模型总层数
    pub layer_bitmap: Option<[u8; 32]>,
    pub architecture: Option<String>,
}
```

通过 `list()` 返回，供 TUI / Network 等消费方读取文件元信息。与 `PeerManagement::SupportedModel` 元信息字段一致，各自独立读取。

### 2.5 StorageError — 错误类型

```rust
pub enum StorageError {
    NotFound(String),   // 文件不存在
    InUse(String),      // 文件被守卫持有
    Io(String),         // 底层 I/O 错误
}
```

### 2.6 ChecksumAlgorithm — 校验算法

```rust
pub enum ChecksumAlgorithm {
    Blake3,
    Sha256,
    XxHash64,  // 默认
}
```

---

## 3. 设计原则

### 3.1 不封装 I/O

`acquire_read` / `acquire_write` 返回 `PathBuf`，不返回文件句柄。消费模块自行决定打开方式（同步/异步、mmap、分层读取等）。这使得 StorageManager 可以管理任意类型文件，不需要泛型读写 trait。

### 3.2 扁平命名空间

`file_id` 是纯文件名（不允许 `/` `\` `..` `.`），所有文件位于同一 `base_dir`。这简化了路径安全校验，避免了目录遍历攻击。

### 3.3 惰性发现

文件可通过外部进程直接写入 `base_dir`。`acquire_read` 在索引未命中时自动检查磁盘（`Lazy_Discover`），将文件纳入管理。`flush()` 提供批量同步。

### 3.4 两阶段锁

| 层级 | 锁 | 作用 |
|------|-----|------|
| 索引锁 | `files: RwLock<HashMap<…>>` | 保护索引条目的增删改查 |
| 文件锁 | `FileState.lock: Arc<RwLock<()>>` | 保护单个文件的读写互斥 |

两阶段设计使得并发操作不同文件时互不阻塞——只有操作同一文件或修改索引时才会争用。

### 3.5 size 惰性刷新

`FileState.size` 仅在 `New()` 初始化扫描和 `flush()` 时更新。`acquire_write` 不触发 re-stat。这避免了频繁 I/O，但意味着 `list()` 返回的 size 可能滞后于实际磁盘大小，直到下次 `flush()`。

### 3.6 幂等性

`remove()` 对不存在的文件返回 `Ok(())`，`New()` 对已存在的目录调用 `create_dir_all`（幂等），`flush()` 二次调用返回 `(0, 0)`。

---

## 4. 核心流程

### 4.1 acquire_read

```
Validate_File_Id(file_id)
    │
    ▼
Lazy_Discover(file_id)
    │
    ├── 索引命中 ──► 返回 FileState.lock 的 Arc clone
    │
    └── 索引未命中
           │
           ▼
        fs::metadata(full_path)
           │
           ├── 磁盘存在 ──► 插入索引 → 返回新 lock
           │
           └── 磁盘不存在 ──► Err(NotFound)
    │
    ▼
lock.read_owned().await   ← 阻塞等待所有 WriteGuard 释放
    │
    ▼
Ok(path, ReadGuard { file_id, _guard })
```

### 4.2 acquire_write

```
Validate_File_Id(file_id)
    │
    ▼
Ensure_Entry(file_id)     ← 索引不存在则插入（size=0）
    │
    ▼
lock.write_owned().await  ← 阻塞等待所有 ReadGuard / WriteGuard 释放
    │
    ▼
Ok(path, WriteGuard { file_id, _guard })
```

### 4.3 remove

```
Validate_File_Id(file_id)
    │
    ▼
快速路径：索引中不存在？
    ├── 是 ──► 检查磁盘 → 存在则 fs::remove_file → Ok
    └── 否
         │
         ▼
    files.write().await       ← 获取索引写锁（原子化探针+删除）
         │
         ▼
    file_lock.try_write_owned()  ← 非阻塞探测
         │
         ├── Ok ──► fs::remove_file → files.remove → Ok
         └── Err ──► Err(InUse)
```

> **设计要点**：探针（`try_write_owned`）和索引删除（`files.remove`）在同一个 `files.write()` 临界区内，消除 TOCTOU 竞态窗口。

### 4.4 exists（含僵尸清理）

```
Validate_File_Id(file_id)
    │
    ▼
files.read() → 索引命中？
    ├── 否 ──► Ok(false)
    └── 是
         │
         ▼
    fs::metadata(full_path)
         │
         ├── 磁盘存在 ──► Ok(true)
         └── 磁盘不存在（潜在僵尸）
              │
              ▼
         files.write()          ← 获取写锁进行原子化清理
              │
              ▼
         二次 metadata 检查
              │
              ├── 仍不存在 ──► files.remove + Ok(false)  ← 真僵尸，清理
              └── 已存在 ────► Ok(true)                   ← 等锁期间被重建
```

> **设计要点**：二次检查避免 TOCTOU——在获取写锁期间文件可能被重建。

### 4.5 flush（两阶段同步）

```
Phase 1: 磁盘 → 索引
    for each 磁盘文件（忽略隐藏/目录）:
        ├── 索引已存在 ──► 刷新 FileState.size
        └── 索引不存在 ──► 插入新 FileState（含 size + 模型元信息 TODO）

Phase 2: 清理僵尸（索引有但磁盘无）
    zombies = 索引 keys - 磁盘文件集合
    for each zombie:
        files.write()
            ├── try_write_owned() 成功 ──► files.remove + cleaned++
            └── try_write_owned() 失败 ──► 跳过（有活跃守卫）

返回 (discovered, cleaned)
```

### 4.6 checksum

```
Validate_File_Id → acquire_read（获取读锁 + 路径）
    │
    ▼
自行打开文件 → 流式哈希 → "{algo}:{hex}"
```

不缓存校验码，每次实时计算。

---

## 5. 并发模型

### 5.1 锁层级

```
files.write()
    │
    └── 持有期间可安全访问 FileState.lock.try_write_owned()
        （非阻塞，不构成锁顺序反转）
```

**规则**：绝不持有一个文件的 `FileState.lock` 的守卫再去获取 `files` 锁。这避免了死锁。

### 5.2 并发场景分析

| 场景 | 行为 |
|------|------|
| 多线程 `acquire_read` 同文件 | 共享读，全部并发成功 |
| `acquire_read` + `acquire_write` 同文件 | `write` 阻塞等待所有 `read` 释放 |
| `acquire_write` + `acquire_write` 同文件 | 第二个阻塞等待第一个释放 |
| `acquire_read` 文件A + `acquire_write` 文件B | 完全并发，互不影响 |
| `remove` + `acquire_read` 同文件 | `remove` 的 `try_write_owned` 失败 → `InUse` |
| `remove` + 无守卫 | `remove` 成功，阻止新的 `acquire_*`（持有 `files.write()`） |

---

## 6. 消费方集成

### 6.1 main.rs — 初始化

```rust
let storage: Arc<dyn StorageCapability> = Arc::new(
    StorageManager::New(&workspace_dir).await?
);
```

通过 `Arc<dyn StorageCapability>` 共享给 Orchestrator 和 Lua 引擎。

### 6.2 Lua 桥接

`storage_handle.rs` 将 `ReadGuard` / `WriteGuard` 包装为 Lua UserData：

```lua
local handle = caps.storage.acquire_read("model.gguf")
local path = handle:path()
-- 用 path 读取文件...
-- handle 超出作用域时自动 drop → 释放锁
```

Lua 侧通过 `caps.storage` 调用 `acquire_read` / `acquire_write` / `remove` / `exists` / `list` / `checksum` / `flush`。

### 6.3 Network 文件传输

`File_Stream` 模块通过 `StorageManager.acquire_read` 获取源文件路径和读锁，通过 `acquire_write` 获取目标路径和写锁，在锁保护下完成传输。

### 6.4 TUI

通过 `list()` 获取当前文件清单和模型元信息，展示给用户。

---

## 7. 已知限制

### 7.1 size 字段滞后

通过 `acquire_write` 写入的文件，其 `FileState.size` 直到下次 `flush()` 才更新。高频写入场景下 `list()` 返回的 size 为 0。消费方如需精确大小，应调用 `flush()` 后再 `list()`。

### 7.2 remove 持有全局写锁跨 I/O

`remove()` 在持有 `files.write()` 期间执行 `fs::remove_file`。对极大量文件并发删除的场景，这可能成为瓶颈。优化方向：先 `files.remove()` 释放写锁，再 `fs::remove_file`，磁盘删除失败时仅日志警告（索引已清理）。

### 7.3 Lazy_Discover 可产生短暂僵尸

`Lazy_Discover` 在 `fs::metadata` 成功后到 `files.insert` 之间，文件可能被外部删除。导致索引短暂存在一条指向不存在的文件的条目。`exists()` 和 `flush()` 会自动清理此类僵尸，无数据风险。

### 7.4 模型元信息未实现

`FileState` 的 `model_id` / `num_layers` / `layer_bitmap` / `architecture` 始终为 `None`。`flush()` 中有 TODO 标记等待 ML 模块 `Analyze` 就绪后填充。

### 7.5 checksum 无缓存

每次 `checksum()` 重新读取全文件计算哈希。大文件（>1GB GGUF）场景下耗时显著。可考虑在 `FileState` 中缓存最近一次校验码及时间戳。

### 7.6 单目录限制

所有文件必须位于同一 `base_dir`。不支持子目录组织。对于大量模型文件（数十个 GGUF），扁平命名空间可能导致管理不便。

---

## 8. 安全设计

### 8.1 路径穿越防护

`Validate_File_Id` 拒绝：
- 空字符串
- 含 `/` 或 `\`（目录分隔符）
- 含 `..`（父目录引用）
- 以 `.` 开头（隐藏文件）

所有路径通过 `Full_Path` 基于 `base_dir` 拼接，不直接使用用户输入构造路径。

### 8.2 锁保护的文件生命周期

```
acquire_write ──► WriteGuard 存活 ──► acquire_read 阻塞
                                      remove 返回 InUse
acquire_read  ──► ReadGuard 存活  ──► acquire_write 阻塞
                                      remove 返回 InUse
```

文件在有任何守卫存活时不能被删除或修改，保证了消费方持有 `ReadGuard` 期间文件内容稳定。

---

## 9. 接口

```rust
#[async_trait]
pub trait StorageCapability: Send + Sync {
    async fn acquire_read(&self, file_id: &str) -> Result<(PathBuf, ReadGuard), StorageError>;
    async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError>;
    async fn remove(&self, file_id: &str) -> Result<(), StorageError>;
    async fn exists(&self, file_id: &str) -> Result<bool, StorageError>;
    async fn list(&self) -> Result<Vec<FileEntry>, StorageError>;
    async fn checksum(&self, file_id: &str, algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError>;
    async fn flush(&self) -> Result<(usize, usize), StorageError>;
}
```

---

## 10. 测试覆盖

| 类别 | 测试 | 说明 |
|------|------|------|
| 生命周期 | `test_end_to_end_write_read_remove` | 完整 write → read → remove 链路 |
| 并发 | `test_multiple_read_guards_concurrent` | 10 并发读 |
| 并发 | `test_write_guard_blocks_read` | 写锁阻止读 |
| 并发 | `test_write_guard_blocks_second_write` | 写锁互斥 |
| 竞态 | `test_remove_returns_inuse_when_read_guard_alive` | ReadGuard 阻止 remove |
| 竞态 | `test_remove_returns_inuse_when_write_guard_alive` | WriteGuard 阻止 remove |
| 惰性发现 | `test_lazy_discover_on_acquire_read` | 外部文件自动发现 |
| 僵尸清理 | `test_exists_cleans_zombie_entry` | exists 清理索引残留 |
| 同步 | `test_flush_discovers_externally_added_files` | flush 发现新文件 |
| 同步 | `test_flush_refreshes_file_size` | flush 刷新 size |
| 同步 | `test_flush_cleans_zombie_entries` | flush 清理僵尸 |
| 安全 | `test_reject_path_traversal` | 拒绝 `/` |
| 安全 | `test_reject_hidden_file` | 拒绝 `.` 开头 |
| 安全 | `test_reject_empty_file_id` | 拒绝空串 |
| 校验 | `test_checksum_*` ×3 | 三种算法 + 实时性验证 |
| 幂等 | `test_flush_idempotent` + `test_remove_idempotent_on_missing` | 重复调用无副作用 |

---

## 11. TODO

### 模型元信息（Phase 3）

- `flush()` 中对 `.gguf` / `.pgguf` 文件调用 ML Analyze，填充 `model_id` / `num_layers` / `layer_bitmap` / `architecture`
- `.gguf` 文件自动转换为 `.pgguf` 格式

### 性能优化

- `checksum` 缓存（带失效时间戳）
- `remove` 中先释放索引锁再做磁盘 I/O
- `FileState.size` 在 WriteGuard drop 时自动 re-stat

### 运维

- 添加文件访问统计（读/写/删除次数，锁等待时间）
- 添加磁盘配额检查
