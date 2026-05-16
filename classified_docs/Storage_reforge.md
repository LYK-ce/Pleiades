# Storage 重构方案

## 0. 核心定义

> **Storage = 文件管理 + 模型统一管理。**
> 统一扁平命名空间，锁与 I/O 解耦。GGUF 文件自动转换为 PGGUF 格式，纳入 model_id 统一管理。
> 只发放 (路径, 锁守卫)，消费方自行决定读写。

## 1. 新类型定义

### 1.1 FileEntry — 文件元数据视图

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

与 `PeerManagement::SupportedModel` 元信息一致，各自读取。非模型文件所有 Option 字段为 None。

## 2. 移除配额管理（过度设计）

### 2.1 删除的类型

| 类型 | 位置 | 说明 |
|------|------|------|
| `QuotaInfo` | capability.rs | 配额快照 DTO |
| `Reservation` | reservation.rs | 整文件删除 |
| `ReservationMap` | reservation.rs | 类型别名，一并删除 |

### 2.2 删除的错误变体

`StorageError::QuotaExceeded` 变体删除，`StorageError` 缩减为 3 变体：NotFound / InUse / Io。

### 2.3 StorageManager 删除的字段/方法

| 删除内容 | 说明 |
|----------|------|
| `quota: u64` 字段 | 配额上限 |
| `used: AtomicU64` 字段 | 已用统计 |
| `reservation_map: ReservationMap` 字段 | 预留表 |
| `New_With_Quota()` 构造器 | 合并入 `New()` |
| `Quota()` | getter |
| `Total_Reserved()` | 内部辅助 |
| `Calc_Available()` | 内部辅助 |

### 2.4 StorageCapability trait 删除的方法

| 方法 | 说明 |
|------|------|
| `reserve()` | 阶段1预留 |
| `commit()` | 阶段2提交 |
| `quota_info()` | 查询配额 |

Trait 从 10 个方法缩减为 7 个。

## 3. list() 升级

原 `list() -> Vec<String>` 升级为 `list() -> Vec<FileEntry>`，返回文件元数据视图。

## 4. flush() 统一刷新 size

去掉 `commit` 后，`FileState.size` 不再有独立更新时机。改为 `flush()` 时统一 re-stat 所有文件目录，同时刷新 size 到 `FileState`。`list()` 时直接读 `FileState.size` 填充 `FileEntry.size`。

## 5. gguf → pgguf 转换 + model_id 计算

### 5.1 职责划分

| 职责 | 负责方 | 说明 |
|------|--------|------|
| 扫描 .gguf/.pgguf 文件 | Storage | flush() 时发现 |
| 格式转换 + 元数据写入 | ML 层 (Analyze) | gguf → pgguf，写入 model_id 等元数据 |
| model_id 计算 | ML 层 (Analyze) | xxhash64 全量内容哈希，写入 PGGUF 元数据 |
| model_id 读取 | Storage | 从 PGGUF 文件元数据中读取，填入 FileEntry |

### 5.2 ML 层 Analyze 所需提供

- 接收文件路径，识别 gguf/pgguf 格式
- 对 gguf 文件：读取内容 → 转换为 pgguf 格式 → 计算 xxhash64 全量哈希 → 写入 PGGUF 元数据（含 model_id）
- 对已有 pgguf 文件：读取元数据 → 返回元信息（含 model_id、层信息等）
- 返回 `AnalyzeResult`，包含 model_id、层数、层位图等元信息

### 5.3 flush() 流程

```
flush()
  ├── 扫描 base_dir，发现新文件
  ├── 对 .gguf/.pgguf 文件 ➜ 调用 ML Analyze
  │     ├── gguf → 转换写入 pgguf，返回 model_id
  │     └── pgguf → 读取元数据，返回 model_id
  ├── 新文件插入 files 索引（含 model_id 记录）
  ├── 统一 re-stat 刷新 FileState.size
  └── 清理僵尸条目（索引有、磁盘无）
```

- `FileEntry.model_id` 从 PGGUF 文件元数据中读取。
- 非模型文件 `model_id` 为 `None`。

## 6. checksum 独立保留

`ChecksumAlgorithm`（Blake3 / Sha256 / XxHash64）保留，供文件传输过程校验使用，与 model_id 计算无关。

---

## 7. 实施计划

> 仅修改 `Src/Storage/` 目录内的代码，不动其他模块对 Storage 的调用（即使编译报错）。

### Phase 1：删除配额管理

| 文件 | 操作 | 说明 |
|------|------|------|
| `reservation.rs` | 删除整文件 | 删除 `Reservation`、`ReservationMap` |
| `capability.rs` | 删除以下内容 | `QuotaInfo` 结构体、`StorageError::QuotaExceeded` 变体、trait 中 `reserve()`/`commit()`/`quota_info()` 三个方法签名及文档 |
| `manager.rs` | 删除以下内容 | `quota`/`used`/`reservation_map` 字段；`New_With_Quota()` 改为委托 `New()`；`Quota()` 方法；`Total_Reserved()`/`Calc_Available()` 辅助方法；`impl StorageCapability` 中 `reserve()`/`commit()`/`quota_info()` 实现；`exists()`/`remove()`/`flush()`/惰性发现中所有 `used.fetch_add`/`fetch_sub` 配额调整代码；`New_With_Quota` 相关测试用例 |
| `mod.rs` | 删除以下内容 | `pub use reservation::Reservation` 导出；`pub use capability::QuotaInfo` 导出；`mod reservation` 声明 |

### Phase 2：新增 FileEntry + 升级 list()

| 文件 | 操作 | 说明 |
|------|------|------|
| **新建** `file_entry.rs` | 创建文件 | 定义 `FileEntry` 结构体（6 字段），实现 `Debug`、`Clone` 等必要 trait |
| `capability.rs` | 修改 `list()` 签名 | `list() -> Result<Vec<String>, StorageError>` → `list() -> Result<Vec<FileEntry>, StorageError>` |
| `manager.rs` | 修改 `list()` 实现 | 从 `files` 索引构造 `Vec<FileEntry>`；`FileState` 可能需要新增 `model_id`/`num_layers`/`layer_bitmap`/`architecture` 字段以缓存 ML 分析结果 |
| | 同步清理 | `exists()` 中僵尸清理不再操作 `used`（Phase 1 已完成）；`remove()` 不再操作 `used`；惰性发现不再操作 `used`；`flush()` 中 `used` 相关代码删除 |
| `mod.rs` | 新增声明与导出 | `mod file_entry` + `pub use file_entry::FileEntry` |

### Phase 3：调整 flush()

| 文件 | 操作 | 说明 |
|------|------|------|
| `manager.rs` | 修改 `flush()` | 扫描阶段：对 `.gguf`/`.pgguf` 文件调用 ML Analyze 获取元信息（若 ML 模块依赖未就绪则先标记 TODO）；统一 re-stat 刷新 `FileState.size`；清理阶段不变 |
| | `Lazy_Discover` 调整 | 去掉配额 `used.fetch_add` 逻辑，仅注册文件到索引 |

### Phase 4：编写设计文档

| 文件 | 操作 | 说明 |
|------|------|------|
| `design_doc/storage_design.md` | 新建文件 | 格式参考 `design_doc/peer_manager_design.md`，完整记录 Storage 模块设计，包含本章节所有重构结论 |

### 最终 trait 方法清单（7 个）

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
