Presented by KeJi
Created Date ： 2026-06-15
Modified Date ： 2026-06-15

# Task 17.3: Config Code Review

> 状态：✅ 已完成
> 父任务：Task 17 (Code Review)

---

## 模块概要

`Src/Config/` — 4 个文件，~300 行。读取和解析 `config.toml`，管理节点身份密钥。

**模组等级：Level 0** — 仅依赖 serde、toml、toml_edit 外部 crate 和 std::fs，不调用任何其他项目模块。

### 对外 API

| 函数 | 调用方 | 说明 |
|------|:---:|------|
| `Ensure_Config()` | main.rs | 确保 `.config/config.toml` 存在 |
| `Read_Config(path)` | Ensure_Config / kvcache_dir | 读取并解析 TOML |
| `Get_Peer_Name(config)` | main.rs | 读节点名，默认 `"new_peer"` |
| `Set_Peer_Name(path, name)` | branch_user.rs `set-name` | 持久化节点名到文件 |
| `Update_Config(...)` | Set_Peer_Name | 通用 TOML 字段修改，保留注释 |
| `kvcache_dir()` | main.rs / context.rs | KV Cache 目录（OnceLock 懒加载） |
| `Ensure_Identity(dir)` | main.rs | Ed25519 密钥对 |
| `workspace_dir()` | main.rs | 工作目录 |
| `log_dir(ws)` | main.rs | 日志目录 |
| `log_level()` | main.rs | 日志级别 |

---

## 发现的问题

### 1. `#![allow(dead_code)]` 模块级屏蔽死代码警告

**位置**: `config.rs:4`

```rust
#![allow(non_snake_case, non_camel_case_types, dead_code)]
```

`dead_code` 作用域是整个模块，掩盖了可能存在的死代码。应去掉，按需加 `#[allow(dead_code)]`。

---

### 2. `Pleiades_Config` 字段全是 `Option`

**位置**: `config.rs:46-53`

```rust
pub struct Pleiades_Config {
    pub Log: Option<Log_Config>,
    pub Network: Option<Network_Config>,
    pub Runtime: Option<Runtime_Config>,
    pub Storage: Option<Storage_Config>,
    pub Session: Option<Session_Config>,
    pub Identity: Option<Identity_Config>,
}
```

每个段都是 `Option`，因为 config.toml 可以缺失某个段。但 `Ensure_Config` 写入的默认模板包含所有段，实际运行中不会出现 `None`。导致所有读取方都要写 `.and_then()` 链。

---

### 3. `CONFIG_DIR` 和 `CONFIG_FILE` 可见性不一致

**位置**: `config.rs:12-16`

```rust
pub const CONFIG_DIR: &str = ".config";    // pub
const CONFIG_FILE: &str = "config.toml";   // 私有
```

`CONFIG_DIR` 因 `main.rs` 需要而设为 `pub`，但 `CONFIG_FILE` 保持私有。

---

### 4. `include_str!("config.toml")` 应改为硬编码字符串

**位置**: `config.rs:11`

```rust
const DEFAULT_CONFIG: &str = include_str!("config.toml");
```

当前默认配置内容分散在两个文件：`config.toml`（模板内容）+ `config.rs`（引用它）。应合并到 `config.rs` 为 `const DEFAULT_CONFIG: &str = r#"..."#;`，删除 `Src/Config/config.toml`。

- 默认值单一来源，改一处即可
- 少一个文件
- 功能不变——`Ensure_Config` 首次启动时仍写入相同内容

---

### 5. `Runtime_Config`、`Session_Config`、`[Scheduler]` 死代码

| 项目 | 位置 | 说明 |
|------|------|------|
| `Runtime_Config` | `config.rs:73-78` | 整个结构体 + `Pleiades_Config.Runtime` 字段，全代码库零读取 |
| `Session_Config` | `config.rs:94-96` | 同上，零读取 |
| `[Scheduler]` | `config.toml:43-45` | 死段，无反序列化目标 |

`Runtime_Config` 的 `device`/`max_token`/`temperature`/`seed` 在 ML_Engine context 中有独立默认值，未走配置。`Session_Config.max_slots` 在 SessionManager 中是硬编码的。

**修复**: 删除 `Runtime_Config`、`Session_Config` 结构体 + `Pleiades_Config` 对应字段 + config.toml 中 `[Scheduler]` 段。

---

## 人类评审

<!-- 在此区域写下评审意见 -->

