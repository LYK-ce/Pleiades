# Storage 设计文档

Presented by KeJi
Created Date ： 2026-05-19
Modified Date ： 2026-06-16

---

## 目录

- [1. 模块概述](#1-模块概述)
- [2. 结构体定义](#2-结构体定义)
  - [FileEntry](#fileentry)
  - [StorageManager](#storagemanager)
  - [ReadGuard / WriteGuard](#readguard--writeguard)
  - [StorageError](#storageerror)
  - [ChecksumAlgorithm](#checksumalgorithm)
- [3. 模块方法](#3-模块方法)
  - [内部辅助](#内部辅助)
  - [trait 方法](#trait-方法)
- [4. 使用示例](#4-使用示例)
- [5. 已知限制](#5-已知限制)

---

## 1. 模块概述

**模组等级：Level 1** — 依赖 PeerManagement (L0)、EventBus (L0)、ML_Engine (L0)。

Storage 是 Pleiades 文件访问的**唯一入口**。严禁绕过 Storage 直接使用 `std::fs` 或 `tokio::fs`。

StorageManager 不封装 I/O——调用方通过 `Acquire_Read`/`Acquire_Write` 获取 `PathBuf` 和锁守卫后自行读写。索引采用两层锁设计：`files: Arc<RwLock<HashMap>>` 保护结构变更，`FileEntry.lock: Arc<RwLock<()>>` 保护单文件读写互斥。两层分离使并发操作不同文件时互不阻塞。

`Flush()` 扫描磁盘目录，通过 `analyze_model` 解析模型元信息，并自动同步到 PeerManager（集群调度）和 EventBus（TUI 刷新）。

模块结构：

```
Storage/
├── mod.rs             ← 模块入口 + Level 1 标注 + re-export
├── file_entry.rs      ← 数据类型: FileEntry + ChecksumAlgorithm
├── capability.rs      ← StorageCapability trait + StorageError
├── guard.rs           ← ReadGuard / WriteGuard（RAII 锁守卫，含 Drop 回调）
└── storage_manager.rs ← StorageManager 实现
```

---

## 2. 结构体定义

### FileEntry

Storage 模块统一使用的文件描述结构。对内承担锁管理，对外通过 `List()` 暴露（`lock` 字段不可见）。

```rust
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub file_name: String,              // 扁平命名空间中的文件名
    pub size: u64,                      // 磁盘文件大小（字节）
    pub model_id: Option<u32>,          // 模型唯一标识（xxhash32），非模型文件为 None
    pub num_layers: Option<u32>,        // 模型总层数
    pub layer_bitmap: Option<[u8; 32]>, // 256 位层位图，bit N=1 表示持有第 N 层
    pub architecture: Option<String>,   // 模型架构名（如 qwen3）
    pub(crate) lock: Arc<RwLock<()>>,   // 内部读写锁，外部不可见
}
```

> `FileEntry` 合并了原内部结构 `FileState` 和对外视图 `FileEntry`，消除字段重复与 `Build_File_Entry` 样板代码。`List()` 直接 `values().cloned().collect()`。

### StorageManager

```rust
pub struct StorageManager {
    base_dir: PathBuf,                             // 工作目录根路径
    files: Arc<RwLock<HashMap<String, FileEntry>>>, // 文件索引（两层锁的 Map 层）
    peer_manager: Arc<dyn Peer_Management_Capability>, // flush 后同步模型信息
    event_bus: Arc<EventBus>,                      // flush 后通知 TUI
}
```

| 字段 | 作用 |
|------|------|
| `base_dir` | 所有文件操作的物理根目录，`file_id` 通过 `Resolve_Path` 拼接得到磁盘路径 |
| `files` | 内存索引，key=文件名，value=元数据+锁。`Arc` 包裹以支持 `WriteGuard::on_drop` 回调捕获 |
| `peer_manager` | `Flush()` 后把本机 `SupportedModel` 列表推送给 PeerManager，供分布式调度 |
| `event_bus` | `Flush()` 后广播 `peer_info_updated` 事件，TUI 面板刷新模型列表 |

### ReadGuard / WriteGuard

基于 `tokio::sync::OwnedRwLockReadGuard` / `OwnedRwLockWriteGuard` 的 RAII 守卫。持有期间阻止 `remove` 和互斥操作，drop 时自动释放。

```rust
pub struct ReadGuard {
    pub(crate) file_id: String,
    pub(crate) _guard: OwnedRwLockReadGuard<()>,
}

pub struct WriteGuard {
    pub(crate) file_id: String,
    pub(crate) on_drop: Option<Box<dyn FnOnce() + Send>>,  // Drop 回调
    pub(crate) _guard: OwnedRwLockWriteGuard<()>,
}
```

| Guard | 语义 | 并发 | Drop 行为 |
|-------|------|:---:|-----------|
| `ReadGuard` | 共享读 | 多个可共存 | 释放文件读锁 |
| `WriteGuard` | 排他写 | 同时仅一个 | `on_drop` 先执行（stat → `files.try_write()` 更新 size），`_guard` 后析构释放写锁 |

> **Drop 顺序**：字段按声明顺序析构。`on_drop` 在 `_guard` 之前声明，因此 stat 在写锁保护下进行，结果准确。回调内用 `files.try_write()`（非阻塞）更新 HashMap——若 `Flush()` 正持有 Map 写锁则跳过，`flush` 结束自然会刷新。这是「尽力而为」优化，不影响正确性。
>
> **死锁安全**：所有同时涉及 `files` Map 锁和 `entry.lock` 文件锁的路径中，至少有一方使用非阻塞 `try_*`。`WriteGuard::drop` 用 `try_write`，`remove`/`flush` 用 `try_write_owned`。只要环上有一条边是非阻塞的，就不可能形成循环等待。

### StorageError

```rust
pub enum StorageError {
    NotFound(String),   // 文件不存在
    InUse(String),      // 文件正被守卫持有
    Io(String),         // 底层 I/O 错误
}
```

所有 trait 方法的返回类型均使用 `StorageError`。位于 `capability.rs`，与 trait 共生。

### ChecksumAlgorithm

```rust
pub enum ChecksumAlgorithm {
    Blake3,
    Sha256,
    XxHash64,  // 默认
}
```

`checksum()` 方法的参数类型。位于 `file_entry.rs`，作为纯数据类型与 `FileEntry` 共存。

---

## 3. 模块方法

### 内部辅助

#### resolve_path

```rust
fn resolve_path(&self, file_id: &str) -> Result<PathBuf, StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` — 扁平文件名 |
| **输出** | 验证通过后的完整磁盘路径 |
| **内部逻辑** | 验证合法性（拒绝空串、`/`、`\`、`..`、`.` 开头）→ `base_dir.join(file_id)` |

> 合并了原 `Validate_File_Id` + `Full_Path`，验证和拼接一步完成。所有接受 `file_id` 的 trait 方法均通过此方法构造路径。`Flush()` 中文件名来自 `read_dir`，天然安全，但统一使用 `Resolve_Path` 保持一致性。

#### Lazy_Discover

```rust
async fn Lazy_Discover(&self, file_id: &str) -> Result<Arc<RwLock<()>>, StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` |
| **输出** | 文件的 `Arc<RwLock<()>>`，供调用方获取读/写锁 |
| **内部逻辑** | 索引命中 → 直接返回 `Arc::clone(&entry.lock)`<br>索引未命中 → `Resolve_Path` → `fs::metadata`<br> 磁盘存在 → 双重检查后插入索引（仅填 `size`，元信息全 `None`）→ 返回新 lock<br> 磁盘不存在 → `NotFound` |

> **只做最小注册**：仅填 `size`（来自 `fs::metadata`），模型字段全为 `None`。`analyze_model` 是重操作（GGUF→PGGUF 转换读 18GB+ 算 hash+写新文件+删旧文件），不应在 `Acquire_Read` 路径触发。元信息由 `Flush()` 统一填充。
>
> 双重检查锁模式：读锁查找 → 未命中 → 释放读锁 → 获取写锁 → 再次查找 → 仍未命中才插入。防止并发重复注册。

#### Ensure_Entry

```rust
async fn Ensure_Entry(&self, file_id: &str) -> Arc<RwLock<()>>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` |
| **输出** | 文件的 `Arc<RwLock<()>>`（永不为 Err） |
| **内部逻辑** | 索引命中 → 直接返回<br>索引未命中 → 双重检查后插入空条目（`size=0`，元信息全 `None`）→ 返回新 lock |

> 不检查磁盘——写入方的语义是「我要创建文件」，磁盘现在有没有不重要。`size` 在 drop `WriteGuard` 时通过 `on_drop` 回调自动刷新。

#### Is_Model_File

```rust
fn Is_Model_File(file_name: &str) -> bool;
```

| | 说明 |
|------|------|
| **输入** | 文件名 |
| **输出** | 是否为 `.gguf` 或 `.pgguf`（大小写不敏感） |
| **内部逻辑** | `to_lowercase()` → `ends_with(".gguf")` \|\| `ends_with(".pgguf")` |

> 仅在 `Flush()` 中使用，决定是否对文件调用 `analyze_model`。

#### Sync_Models_To_Peer_Manager

```rust
async fn Sync_Models_To_Peer_Manager(&self);
```

| | 说明 |
|------|------|
| **输入** | 无 |
| **输出** | 无（副作用方法） |
| **内部逻辑** | `List()` → `filter_map` 过滤出模型文件（有 `model_id` + `layer_bitmap`）→ 转为 `SupportedModel` → `peer_manager.Update_Supported_Models()` → `event_bus.Publish(State{...})` |

> 仅在 `Flush()` 末尾调用。只更新本机信息，远程节点由 Network 模块独立维护。

#### hash_file

自由函数，位于 `impl StorageManager` 块外部。

```rust
async fn hash_file(
    file: &mut tokio::fs::File,
    mut update: impl FnMut(&[u8]),
) -> Result<(), StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file` — 已打开的文件句柄；`update` — 回调，每读一块数据调用一次 |
| **输出** | `Ok(())` 或读取错误 |
| **内部逻辑** | 循环读取 8KB 块 → `update(&buf[..n])` → 直到 EOF |

> 提取 `checksum` 中三种算法（Blake3/SHA256/XXH64）的重复读取循环。每分支从 ~15 行减为 3 行。

---

### trait 方法

#### acquire_read

```rust
async fn acquire_read(&self, file_id: &str) -> Result<(PathBuf, ReadGuard), StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` — 扁平文件名 |
| **输出** | `(PathBuf, ReadGuard)` — 磁盘路径 + 读锁守卫 |
| **内部逻辑** | `Resolve_Path` → `Lazy_Discover`（惰性注册）→ `lock.read_owned().await`（阻塞等待所有写锁释放）→ 打包返回 |
| **失败** | `NotFound`（文件不存在）、`Io`（非法 file_id） |

调用方用 `PathBuf` 自行读取文件，drop `ReadGuard` 释放锁。多个 `ReadGuard` 可并发共存。

#### acquire_write

```rust
async fn acquire_write(&self, file_id: &str) -> Result<(PathBuf, WriteGuard), StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` — 扁平文件名 |
| **输出** | `(PathBuf, WriteGuard)` — 磁盘路径 + 写锁守卫（含 `on_drop` 回调） |
| **内部逻辑** | `Resolve_Path` → `Ensure_Entry`（不查磁盘，确保索引有条目）→ `lock.write_owned().await`（阻塞等待所有锁释放）→ 构造 `on_drop`（捕获 `Arc<files>`、`path`、`file_id`）→ 返回 |
| **失败** | `Io`（非法 file_id）。文件不存在不是错误 |

独占访问。drop `WriteGuard` 时自动 stat 刷新 `entry.size`。

#### remove

```rust
async fn remove(&self, file_id: &str) -> Result<(), StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` |
| **输出** | `Ok(())`，幂等 |
| **内部逻辑** | `Resolve_Path` → 快速路径（索引无→直接 `fs::remove_file` 磁盘）→ 慢速路径（索引有→`try_write_owned` 探针→成功则删磁盘+删索引） |
| **失败** | `InUse`（有活跃守卫）、`Io` |

> 快速路径覆盖了「文件在磁盘但未被索引注册」的场景（惰性发现窗口期）。`try_write_owned` 探针和 `files.remove` 在同一 `files.write()` 临界区内，消除 TOCTOU 竞态。对不存在文件返回 `Ok`（幂等）。

#### exists

```rust
async fn exists(&self, file_id: &str) -> Result<bool, StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` |
| **输出** | 文件是否存在于磁盘 |
| **内部逻辑** | `Resolve_Path` → 索引无→`false` → 索引有→`fs::metadata` → 磁盘有→`true` → 磁盘无→拿写锁双重确认→`files.remove`（僵尸清理）→`false` |
| **副作用** | 检测到僵尸条目（索引有但磁盘无）时自动清理索引 |

> 双重检查避免 TOCTOU：释放读锁后拿写锁之间，文件可能被重建。二次 `metadata` 确认后才决定是否清理。

#### list

```rust
async fn list(&self) -> Result<Vec<FileEntry>, StorageError>;
```

| | 说明 |
|------|------|
| **输入** | 无 |
| **输出** | 所有已注册文件的 `FileEntry` 副本 |
| **内部逻辑** | `files.read().await.values().cloned().collect()` |

> `Clone` 对 `Arc` 字段只增加引用计数，不深拷贝锁。迭代顺序由 `HashMap` 决定，不稳定，调用方不应依赖顺序。

#### checksum

```rust
async fn checksum(&self, file_id: &str, algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError>;
```

| | 说明 |
|------|------|
| **输入** | `file_id` + 可选算法（默认 XxHash64） |
| **输出** | `"algo:hex"` 格式字符串 |
| **内部逻辑** | `Acquire_Read`（验证+拿读锁）→ `File::open` → `Hash_File` 分块读取 → hasher 计算 → 格式化 |
| **失败** | `NotFound`、`Io` |

> 每次实时计算，不缓存。大文件（>1GB GGUF）耗时显著。三种算法的读取循环通过 `Hash_File` 自由函数消除重复（55 行 → 25 行）。

#### flush

```rust
async fn flush(&self) -> Result<(usize, usize), StorageError>;
```

| | 说明 |
|------|------|
| **输入** | 无 |
| **输出** | `(新发现文件数, 清理僵尸数)` |
| **内部逻辑** | 三阶段：<br>**Phase 1** 扫描磁盘 → 对比索引 → 已有文件刷新 `size`+元信息 / 新文件 `analyze_model`+插入<br>**Phase 2** 僵尸清理 → `files.keys() - disk_files` → `try_write_owned` 探针 → 删除无人用的条目<br>**Phase 3** 调用 `Sync_Models_To_Peer_Manager()` 同步集群+TUI |
| **失败** | `Io`（磁盘读取失败） |

> **Phase 1 已有文件刷新元信息**：对 PGGUF 走快速路径（元信息已在文件头，零 I/O），对 GGUF 触发格式转换。
> **Phase 1 新文件**：`analyze_model` 对 GGUF 做完整转换（读 18GB+ 算 hash+写 PGGUF+删 GGUF），对 PGGUF 直接读 header。
> **Phase 2**：`try_write_owned` 失败表示有活跃守卫，跳过不删，下次 `flush` 或 `exists` 会处理。
> 幂等——二次调用返回 `(0, 0)`。

---

## 4. 使用示例

### Rust 侧

```rust
use std::sync::Arc;
use pleiades::storage::{StorageManager, StorageCapability, ChecksumAlgorithm};

// 创建
let storage: Arc<dyn StorageCapability> = Arc::new(
    StorageManager::New("/workspace", peer_manager, event_bus).await?
);

// 写入文件
let (path, guard) = storage.acquire_write("data.bin").await?;
tokio::fs::write(&path, b"hello world").await?;
drop(guard);  // ← 自动 stat 刷新 size

// 读取文件
let (path, guard) = storage.acquire_read("data.bin").await?;
let content = tokio::fs::read(&path).await?;
drop(guard);

// 校验
let hash = storage.checksum("data.bin", Some(ChecksumAlgorithm::Sha256)).await?;
// → "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"

// 同步磁盘
let (new, cleaned) = storage.flush().await?;

// 列出所有文件
for entry in storage.list().await? {
    println!("{} — {} bytes", entry.file_name, entry.size);
}
```

### Lua 侧

```lua
-- 读取
local handle = caps.storage_acquire_read("model.pgguf")
local path = handle:path()
-- 用 path 读取文件...
handle:release()  -- 或超出作用域自动 drop

-- 列出
local files = caps.storage_list()
for _, f in ipairs(files) do
    print(f.file_name, f.size)
end

-- 校验
local hash = caps.storage_checksum("model.pgguf", "blake3")
```

---

## 5. 已知限制

| 限制 | 说明 |
|------|------|
| 单目录 | 所有文件必须位于同一 `base_dir`，不支持子目录组织 |
| checksum 无缓存 | 每次实时计算全文件哈希，大文件（>1GB GGUF）耗时显著 |
| remove 持全局写锁跨 I/O | `Remove()` 在持有 `files.write()` 期间执行 `fs::remove_file`。高频删除场景可优化为先释放锁再删磁盘 |
| 外部写入后 size 不自动刷新 | 外部进程直接写磁盘后 size 需 `Flush()` 才能更新。通过 `Acquire_Write` 写入则 drop 时自动刷新 |
| Lazy_Discover 可产生短暂僵尸 | `fs::metadata` 成功到 `files.insert` 之间文件可能被外部删除。`Exists()` 和 `Flush()` 自动清理，无数据风险 |
| 无磁盘配额 | 不限制总文件大小或数量 |
| 无访问统计 | 不记录读/写/删除次数和锁等待时间 |
