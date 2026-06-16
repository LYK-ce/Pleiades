Presented by KeJi
Created Date ： 2026-06-15
Modified Date ： 2026-06-15

# Task 17.4: Storage Code Review

> 状态：方案已确定，待实现
> 父任务：Task 17 (Code Review)

---

## 模块概要

`Src/Storage/` — 4 个文件，~1100 行。文件锁管理 + 路径解析 + 索引注册表 + 模型分析/同步。

**模组等级：Level 1** — 依赖 PeerManagement (L0)、EventBus (L0)、ML_Engine (L0)。

### 对外 API (StorageCapability trait)

| 方法 | 说明 |
|------|------|
| `acquire_read(file_id)` | 获取读锁 + 路径，惰性发现外部文件 |
| `acquire_write(file_id)` | 获取写锁 + 路径，自动注册索引 |
| `remove(file_id)` | 删除文件（幂等） |
| `exists(file_id)` | 检查是否存在（僵尸清理） |
| `list()` | 列出所有文件元数据 |
| `checksum(file_id, algo)` | 实时计算文件校验码 |
| `flush()` | 扫描磁盘 → 模型分析 → 同步 PeerManager + EventBus |

### 调用方

- `main.rs` — 初始化 + flush
- `Session_Manager` — 加载模型/tokenizer
- Network file transfer — 接收文件写入
- Lua 脚本 — storage_acquire_read/write

---

## 发现的问题

### 1. `flush_and_sync` — 死代码，应删除

**位置**: `storage_manager.rs:202`

```rust
pub async fn flush_and_sync(&self) -> Result<(usize, usize), String> {
    let (discovered, cleaned) = <Self as StorageCapability>::flush(self).await
        .map_err(|e| format!("flush: {e}"))?;
    self.sync_models_to_peer_manager().await;   // flush() 内部已调，二次 sync
    Ok((discovered, cleaned))
}
```

- `flush()` 内部已调用 `sync_models_to_peer_manager()`
- 全局搜索无任何外部调用方
- **结论**：死代码，直接删除。

---

### 2. 头注释格式未更新

全部文件仍用旧 `//Date` 格式，未改为 `//Created Date / Modified Date`。

---

### 3. `FileState` 和 `FileEntry` 合并 + 提取为独立文件

**现状**：`FileState`（storage_manager.rs 内部）和 `FileEntry`（capability.rs 对外）5 个业务字段完全重叠，`Build_File_Entry` 纯样板代码。

**决策**（2026-06-15）：合并两者为统一的 `FileEntry`，并仿照 `PeerManagement/peer_info.rs` 模式提取为独立文件 `file_entry.rs`。

参考 `PeerManagement` 模块结构：
```
PeerManagement/
├── mod.rs          ← 模块声明 + re-export
├── capability.rs   ← trait 定义
├── peer_info.rs    ← 数据结构 (PeerInfo, SupportedModel, ...)
└── peer_manager.rs ← 管理器 + trait impl
```

Storage 对齐后：
```
Storage/
├── mod.rs             ← 模块声明 + re-export
├── capability.rs      ← trait 定义 (FileEntry 移出)
├── guard.rs           ← ReadGuard / WriteGuard
├── file_entry.rs      ← 🆕 FileEntry 数据结构
└── storage_manager.rs ← 管理器 + trait impl (删 FileState, Build_File_Entry, flush_and_sync)
```

#### 详细实施计划

**Step 1**: 新建 `Src/Storage/file_entry.rs`

```rust
//Presented by KeJi
//Created Date ： 2026-06-15
//Modified Date ： 2026-06-15

//! 文件元数据数据结构
//!
//! FileEntry 是 Storage 模块统一使用的文件描述结构，
//! 对内承担原 FileState 的锁管理职责，对外通过 list() 暴露。

use std::sync::Arc;
use tokio::sync::RwLock;

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
```

**Step 2**: `capability.rs` — 移除 `FileEntry` 定义，改为 `pub use` re-export

```rust
// 删除 FileEntry struct 定义（约 20 行）
// 改为
pub use super::file_entry::FileEntry;
```

`StorageCapability` trait 签名不变。

**Step 3**: `storage_manager.rs` — 合并改动

| 改动 | 说明 |
|------|------|
| 删 `struct FileState` | 第 22–33 行 |
| `HashMap<String, FileState>` → `HashMap<String, FileEntry>` | 第 42 行 |
| 删 `Build_File_Entry` 函数 | 第 189–198 行 |
| 删 `flush_and_sync` 函数 | 第 202–207 行 |
| 4 处 `FileState { ... }` → `FileEntry { file_name: ..., ... }` | New / Lazy_Discover / Ensure_Entry / flush |
| 3 处 `state.lock` → `entry.lock` | Lazy_Discover / Ensure_Entry / flush 僵尸清理 |
| `list()`: `.map(Build_File_Entry)` → `.values().cloned().collect()` | 第 339–344 行 |
| 变量重命名 `state` → `entry` | flush 元信息刷新处 |

**Step 4**: `mod.rs` — 添加模块声明

```rust
// 新增
mod file_entry;
// pub use 更新
pub use file_entry::FileEntry;
```

**Step 5**: 头注释修正（全部 4 文件）

| 文件 | 旧格式 | 新格式 |
|------|--------|--------|
| `mod.rs` | `//Date ： 2026-05-14` | `//Created Date ： 2026-05-14` / `//Modified Date ： 2026-06-15` |
| `capability.rs` | 无头注释 → 补全 | 标准化 |
| `guard.rs` | 无头注释 → 补全 | 标准化 |
| `storage_manager.rs` | `//Date ： 2026-05-14` | 标准化 |

---

### 4. `StorageCapability` trait 缺少 Level 标注

与 EventBus/PeerManagement 不同，模块注释未标注模块等级和依赖关系。

**修正**: `mod.rs` 模块文档添加 Level 1 标注 + 依赖说明。

---

## 人类评审

<!-- 在此区域写下评审意见 -->

