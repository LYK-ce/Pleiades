# Config 设计文档

Presented by KeJi
Created Date ： 2026-06-15
Modified Date ： 2026-06-15

---

## 目录

- [1. 模块概述](#1-模块概述)
- [2. 结构体定义](#2-结构体定义)
  - [Pleiades_Config](#pleiades_config)
  - [Log_Config](#log_config)
  - [Network_Config](#network_config)
  - [Storage_Config](#storage_config)
  - [Identity_Config](#identity_config)
- [3. 模块方法](#3-模块方法)
  - [配置读取](#配置读取)
  - [配置写入](#配置写入)
  - [身份管理](#身份管理)
- [4. 使用示例](#4-使用示例)
- [5. 已知限制](#5-已知限制)

---

## 1. 模块概述

**模组等级：Level 0** — 仅依赖 serde、toml、toml_edit 外部 crate 和 std::fs，不调用任何其他项目模块。

Config 负责读取和解析 `.config/config.toml` 配置文件，管理节点 Ed25519 密钥对。首次运行时自动生成默认配置。

---

## 2. 结构体定义

### Pleiades_Config

```rust
pub struct Pleiades_Config {
    pub Log: Option<Log_Config>,
    pub Network: Option<Network_Config>,
    pub Storage: Option<Storage_Config>,
    pub Identity: Option<Identity_Config>,
}
```

反序列化自 `config.toml`。每个段均为 `Option`——TOML 文件中缺失某段时反序列化为 `None`。

**便捷方法**：

| 方法 | 输入 | 输出 | 默认值 |
|------|------|------|--------|
| `workspace_dir()` | `&self` | `PathBuf` | `"Pleiades_Workspace"` |
| `log_dir(workspace)` | `&self, &Path` | `PathBuf` | `<workspace_dir>/Log` |
| `log_level()` | `&self` | `&str` | `"info"` |

---

### Log_Config

```rust
pub struct Log_Config {
    pub level: Option<String>,        // 日志级别: trace/debug/info/warn/error
    pub log_file_path: Option<String>, // 日志文件路径
}
```

---

### Network_Config

```rust
pub struct Network_Config {
    pub LAN: Option<bool>,
    pub WAN: Option<bool>,
    pub Transport_Protocol: Option<String>,  // TCP/QUIC
    pub cleanup_interval: Option<u64>,        // 清理间隔（秒），默认 300
    pub timeout_interval: Option<u64>,        // 超时间隔（秒），默认 300
    pub heartbeat_interval: Option<u64>,      // 心跳间隔（秒），默认 60
    pub heartbeat_timeout: Option<u64>,       // 心跳超时（秒），默认 10
    pub request_response_timeout: Option<u64>, // 请求响应超时（秒），默认 300
}
```

---

### Storage_Config

```rust
pub struct Storage_Config {
    pub workspace_dir: Option<String>,  // 工作目录
    pub kvcache_dir: Option<String>,    // KV Cache 目录
}
```

---

### Identity_Config

```rust
pub struct Identity_Config {
    pub peer_name: Option<String>,  // 节点名称
}
```

---

## 3. 模块方法

### 配置读取

| 方法 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `Ensure_Config()` | 无 | `(Pleiades_Config, PathBuf)` | 若 `.config/config.toml` 不存在则创建并写入默认配置；返回解析结果和文件路径 |
| `Read_Config(path)` | `&Path` | `Pleiades_Config` | 读取并解析 TOML 文件 |
| `Get_Peer_Name(config)` | `&Pleiades_Config` | `String` | 读节点名，默认 `"new_peer"` |
| `kvcache_dir()` | 无 | `&PathBuf` | KV Cache 目录，首次调用读配置（OnceLock 缓存），默认 `".kvcache"` |

### 配置写入

| 方法 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `Set_Peer_Name(path, name)` | `&Path, &str` | `Result<()>` | 持久化节点名到 config.toml，内部调用 Update_Config |
| `Update_Config(path, section, key, value)` | `&Path, &str, &str, &str` | `Result<()>` | 修改 TOML 任意字段（字符串），使用 toml_edit 保留注释和格式 |

### 身份管理

| 方法 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `Ensure_Identity(dir)` | `&Path` | `Keypair` | 若 `.config/keypair.bin` 存在则读取，否则生成 Ed25519 密钥对并持久化 |

`Keypair` 决定节点的 `PeerId`，持久化后每次启动使用相同 PeerId。格式为 libp2p protobuf 编码。

---

## 4. 使用示例

```rust
use pleiades::config::{Ensure_Config, Ensure_Identity, kvcache_dir};

// 读取配置（首次运行自动生成默认 .config/config.toml）
let (config, config_path) = Ensure_Config()?;

// 加载节点身份密钥
let keypair = Ensure_Identity(Path::new(".config"))?;
let peer_id = PeerId::from(keypair.public());

// 读取配置值（自动处理 None → 默认值）
let ws = config.workspace_dir();        // "Pleiades_Workspace"
let log = config.log_dir(&ws);          // "Pleiades_Workspace/Log"
let lvl = config.log_level();          // "info"
let name = Get_Peer_Name(&config);     // "new_peer"
let kv = kvcache_dir();                // ".kvcache"

// 修改并持久化节点名
Set_Peer_Name(&config_path, "gpu-node-0")?;
```

---

## 5. 已知限制

| 限制 | 说明 |
|------|------|
| 无热重载 | 运行时修改 config.toml 需要重启。仅 `set-name` 通过 Update_Config 支持运行时持久化 |
| 默认值散落 | `workspace_dir` / `log_dir` / `log_level` 的默认值在 `Pleiades_Config` impl 中硬编码，与 DEFAULT_CONFIG 字符串中的值需手动保持一致 |
| eprintln 而非 tracing | `Ensure_Config` 使用 `eprintln!` 输出初始化信息，此时 tracing 尚未初始化 |
