Presented by KeJi
Created Date ： 2026-06-15
Modified Date ： 2026-06-15

# Task 17.4: Storage Code Review

> 状态：审查中
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

### 1. `flush_and_sync` 和 `flush` 重复

**位置**: `storage_manager.rs:228-232`

```rust
pub async fn flush_and_sync(&self) -> Result<(usize, usize), String> {
    let (discovered, cleaned) = <Self as StorageCapability>::flush(self).await
        .map_err(|e| format!("flush: {e}"))?;
    self.sync_models_to_peer_manager().await;
    Ok((discovered, cleaned))
}
```

但 `flush()` 内部**已经调了** `sync_models_to_peer_manager()`。所以 `flush_and_sync()` 会 sync 两遍。

---

### 2. 头注释格式未更新

全部文件仍用旧 `//Date` 格式。

---

### 3. `FileState` 和 `FileEntry` 字段高度重叠

```rust
// FileState (内部)
model_id, num_layers, layer_bitmap, architecture, size

// FileEntry (对外)
model_id, num_layers, layer_bitmap, architecture, size
```

5 个字段完全一致。`Build_File_Entry` 就是逐字段拷贝。可考虑简化。

---

### 4. `StorageCapability` trait 缺少 Level 标注

与 EventBus/PeerManagement 不同，模块注释未标注模块等级和依赖关系。

---

## 人类评审

<!-- 在此区域写下评审意见 -->

