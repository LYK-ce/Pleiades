Presented by KeJi
Date: 2026-06-09

# Task 17: Code Review — 消除硬编码，统一配置管理

> 状态：17.1~17.8 方案已确定，待实现

---

## 背景

Code Review 发现 `Src/main.rs` 中存在三处硬编码常量：

```rust
const CONFIG_DIR: &str = ".config";
const DEFAULT_WORKSPACE: &str = "Pleiades_Workspace";
const LOG_SUBDIR: &str = "Log";
```

同时 `Src/Config/config.rs` 中也有自己的 `CONFIG_DIR` / `CONFIG_FILE` 常量，形成重复定义。

Review 意见：**不应在代码中硬编码，改为从 config.toml 读取，当前值作为默认值**。

---

## 现状分析

### 硬编码分布

| 常量 | 位置 | 用途 |
|------|------|------|
| `CONFIG_DIR` (`.config`) | `main.rs:44` + `config.rs:17` | 定位配置目录，调用 `Ensure_Identity()` |
| `DEFAULT_WORKSPACE` (`Pleiades_Workspace`) | `main.rs:47` | `Storage.workspace_dir` 未配置时的兜底值 |
| `LOG_SUBDIR` (`Log`) | `main.rs:50` | `Log.log_file_path` 未配置时拼出 `<workspace>/Log/` |

### 配置现状

```toml
# config.toml 中已有的相关字段
[Storage]
workspace_dir = "Pleiades_Workspace"    # ← 已可配置

[Log]
log_file_path = "Log/"                  # ← 已可配置，但值是相对路径
```

### 核心矛盾：鸡生蛋问题

`CONFIG_DIR` 是定位 `config.toml` 的前提——必须先知道 `.config` 目录在哪，才能读到配置文件。因此 `CONFIG_DIR` **不能从 config.toml 读取**。

---

## 重构方案

### 策略总览

| 常量 | 策略 | 说明 |
|------|------|------|
| `CONFIG_DIR` | **保留在 config.rs 为唯一权威**，main.rs 不再重复定义 | 鸡生蛋问题，无法从配置读取。main.rs 通过 `config` 模块的公开函数/常量获取 |
| `DEFAULT_WORKSPACE` | 移入 `config.rs`，作为 `Storage_Config` 的默认值方法 | `config.toml` 中已有默认值 `"Pleiades_Workspace"`，Rust 侧也应统一 |
| `LOG_SUBDIR` | 改为 `Log_Config` 的默认值方法，移除 main.rs 中的拼接逻辑 | 当 `log_file_path` 未设置时，由 config 模块提供默认路径 |
| `.kvcache` | 传入 `Storage_Config` + `MlContext`，默认 `".kvcache"` | main.rs 创建目录 + context.rs 读写均改为从配置/MlContext 获取 |

### 17.1 统一 CONFIG_DIR

**问题**：`config.rs` 和 `main.rs` 各自定义了 `CONFIG_DIR`，形成重复定义。

**方案**：`config.rs` 是配置模块的权威来源，main.rs 不应重复定义。将 `config.rs` 中的 `CONFIG_DIR` 改为 `pub(crate)`，main.rs 删除自己的定义并引用 config 模块。

#### config.rs 改动

```rust
// Before (line 17):
const CONFIG_DIR: &str = ".config";

// After:
pub(crate) const CONFIG_DIR: &str = ".config";
```

#### main.rs 改动

```rust
// ===== 删除 (line 44) =====
const CONFIG_DIR: &str = ".config";

// ===== Phase 1 中 Ensure_Identity 调用处 (line 62) =====
// Before:
let keypair = Ensure_Identity(Path::new(CONFIG_DIR))?;

// After:
let keypair = Ensure_Identity(Path::new(pleiades::config::CONFIG_DIR))?;
```

> 注：`config.rs` 内部的 `Ensure_Config()` 已经在使用自己的 `CONFIG_DIR`，不受影响。

**涉及文件**：
- `Src/Config/config.rs` — 第 17 行：`const` → `pub(crate) const`
- `Src/main.rs` — 第 44 行删除 + 第 62 行改为引用 `pleiades::config::CONFIG_DIR`

---

### 17.2 移除 DEFAULT_WORKSPACE 硬编码

**问题**：当 `config.Storage.workspace_dir == None` 时，main.rs 第 47 行使用硬编码 `"Pleiades_Workspace"`。

**方案**：在 `Pleiades_Config` 上新增 `workspace_dir()` 方法封装默认值逻辑，main.rs 删除常量并调用该方法。

#### config.rs 改动（在 `Get_Peer_Name` 函数之后新增 impl 块）

```rust
impl Pleiades_Config {
    /// 获取工作目录，默认 "Pleiades_Workspace"
    pub fn workspace_dir(&self) -> PathBuf {
        self.Storage.as_ref()
            .and_then(|s| s.workspace_dir.as_deref())
            .unwrap_or("Pleiades_Workspace")
            .into()
    }
}
```

#### main.rs 改动

```rust
// ===== 删除 (line 47) =====
const DEFAULT_WORKSPACE: &str = "Pleiades_Workspace";

// ===== Phase 1 中 workspace_dir 赋值 (lines 66-70) =====
// Before:
let workspace_dir: PathBuf = config.Storage.as_ref()
    .and_then(|s| s.workspace_dir.as_deref())
    .unwrap_or(DEFAULT_WORKSPACE)
    .into();

// After:
let workspace_dir = config.workspace_dir();
```

**涉及文件**：
- `Src/Config/config.rs` — `Get_Peer_Name` 之后新增 `impl Pleiades_Config` 块，含 `workspace_dir()` 方法
- `Src/main.rs` — 第 47 行删除常量 + 第 66-70 行简化为 `config.workspace_dir()`

---

### 17.3 移除 LOG_SUBDIR 硬编码

**问题**：当 `config.Log.log_file_path == None` 时，main.rs 第 50 行硬编码 `"Log"` 拼接出 `<workspace>/Log/`。

**方案**：在 `Pleiades_Config` 上新增 `log_dir()` 和 `log_level()` 方法封装默认值。

#### config.rs 改动（追加到 17.2 的 impl 块中）

```rust
impl Pleiades_Config {
    // ... workspace_dir() 见 17.2 ...

    /// 获取日志目录，默认 <workspace_dir>/Log
    pub fn log_dir(&self, workspace_dir: &Path) -> PathBuf {
        self.Log.as_ref()
            .and_then(|l| l.log_file_path.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_dir.join("Log"))
    }

    /// 获取日志级别，默认 "info"
    pub fn log_level(&self) -> &str {
        self.Log.as_ref()
            .and_then(|l| l.level.as_deref())
            .unwrap_or("info")
    }
}
```

#### main.rs 改动

```rust
// ===== 删除 (line 50) =====
const LOG_SUBDIR: &str = "Log";

// ===== Phase 2 中 log_dir 赋值 (lines 82-86) =====
// Before:
let log_dir: PathBuf = config.Log.as_ref()
    .and_then(|l| l.log_file_path.as_deref())
    .map(PathBuf::from)
    .unwrap_or_else(|| workspace_dir.join(LOG_SUBDIR));

// After:
let log_dir = config.log_dir(&workspace_dir);

// ===== Phase 2 中 log_level 赋值 (lines 88-91) =====
// Before:
let log_level = config.Log.as_ref()
    .and_then(|l| l.level.as_deref())
    .unwrap_or("info");

// After:
let log_level = config.log_level();
```

**涉及文件**：
- `Src/Config/config.rs` — 在 17.2 的 impl 块中追加 `log_dir()` + `log_level()` 方法
- `Src/main.rs` — 第 50 行删除常量 + 第 82-86 行简化为 `config.log_dir(&workspace_dir)` + 第 88-91 行简化为 `config.log_level()`

---

### 17.4 配置文件默认值同步

确保 `config.toml` 中的默认值与 Rust 代码中的默认值一致：

| 字段 | config.toml 默认值 | Rust 默认值 | 状态 |
|------|-------------------|-------------|------|
| `Storage.workspace_dir` | `"Pleiades_Workspace"` | `"Pleiades_Workspace"` | ✅ 一致 |
| `Log.log_file_path` | `"Log/"` | `<workspace>/Log` | ⚠️ 语义不同（相对 vs 绝对） |
| `Log.level` | `"info"` | `"info"` | ✅ 一致 |

**`log_file_path` 语义说明**：
- config.toml 中 `log_file_path = "Log/"` 是相对路径，实际使用时由 `tracing_appender::rolling::daily(&log_dir, "pleiades.log")` 处理
- Rust 默认值是 `<workspace_dir>/Log`（绝对路径拼接）
- 两者实际效果一致（因为程序工作目录就是项目根目录），但建议统一为绝对路径语义

---

### 17.5 移除 `.kvcache` 硬编码

**问题**：`.kvcache` 字符串在三处硬编码：

| 位置 | 行号 | 用途 |
|------|------|------|
| `main.rs` | 73 | `create_dir_all(".kvcache")` 创建目录 |
| `context.rs` | 630 | `Path::new(".kvcache").join(file_id)` — `offload_save` |
| `context.rs` | 661 | `Path::new(".kvcache").join(file_id)` — `offload_load` |

**方案**：`config.rs` 提供一个全局可访问的函数 `kvcache_dir()`，内部用 `OnceLock` 懒加载读取 config.toml，缓存结果。所有调用点直接调这个函数获取路径。

- 优势：context.rs 只需改两行，不涉及 `MlContext` 字段、不涉及 Lua 绑定、不需要传参
- `OnceLock` 确保只读一次 config.toml，后续调用零开销（返回缓存引用）

#### config.rs 改动

```rust
// 文件顶部新增 use
use std::sync::OnceLock;

// 在 CONFIG_FILE 常量之后新增静态变量 + 函数
static KVCACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 获取 kvcache 目录（懒加载，只读一次 config.toml，之后返回缓存）
pub fn kvcache_dir() -> &'static PathBuf {
    KVCACHE_DIR.get_or_init(|| {
        let config_path = Path::new(CONFIG_DIR).join(CONFIG_FILE);
        match Read_Config(&config_path) {
            Ok(config) => {
                config.Storage.as_ref()
                    .and_then(|s| s.kvcache_dir.as_deref())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(".kvcache"))
            }
            Err(_) => PathBuf::from(".kvcache"),
        }
    })
}
```

```rust
// Storage_Config 新增字段
pub struct Storage_Config {
    pub workspace_dir: Option<String>,
    pub quota_gb: Option<u64>,
    pub kvcache_dir: Option<String>,   // ← 新增
}
```

```toml
# config.toml 默认配置新增
[Storage]
workspace_dir = "Pleiades_Workspace"
quota_gb = 0
kvcache_dir = ".kvcache"              # ← 新增
```

#### main.rs 改动

```rust
// ===== Phase 1 中 kvcache 目录创建 (line 73) =====
// Before:
std::fs::create_dir_all(".kvcache")?;

// After:
std::fs::create_dir_all(pleiades::config::kvcache_dir())?;
```

#### context.rs 改动

```rust
// ===== offload_save (line 630) =====
// Before:
let file_path = Path::new(".kvcache").join(file_id);

// After:
let file_path = pleiades::config::kvcache_dir().join(file_id);

// ===== offload_load (line 661) =====
// Before:
let file_path = Path::new(".kvcache").join(file_id);

// After:
let file_path = pleiades::config::kvcache_dir().join(file_id);
```

`MlContext` 结构体、`MlSession::new` 签名、Lua 绑定 —— **全部不动**。

**涉及文件**：
- `Src/Config/config.toml` — `[Storage]` 新增 `kvcache_dir = ".kvcache"`
- `Src/Config/config.rs` — `Storage_Config` 新增字段 + 新增 `OnceLock` 全局静态 + `kvcache_dir()` 函数
- `Src/main.rs` — 第 73 行改为 `pleiades::config::kvcache_dir()`
- `Src/ML_Engine/context.rs` — 第 630、661 行改为 `pleiades::config::kvcache_dir()`

---

### 17.6 日志文件命名改为启动时间戳

**问题**：当前使用 `tracing_appender::rolling::daily` 按天滚动：

```rust
let file_appender = tracing_appender::rolling::daily(&log_dir, "pleiades.log");
```

存在两个缺陷：
1. **同一天重启会覆盖** — `pleiades.log.YYYY-MM-DD` 每次打开都是同一个文件名，新的 `File::create` 会清空旧内容
2. **跨天切断不自然** — 长时间运行的推理任务可能在凌晨被切到新文件，排查问题时需要跨文件追踪

**方案**：改为每次启动用当前时间戳生成唯一文件名，不覆盖、不滚动。

```
Pleiades_Workspace/Log/
├── pleiades.log.2026-06-09-14-30-05
├── pleiades.log.2026-06-09-15-12-33
└── pleiades.log.2026-06-10-08-00-01
```

`chrono` 已在 `Cargo.toml` 中（`chrono = "0.4"`），可以直接用。

#### main.rs 改动

```rust
// ===== Phase 2 日志初始化 (lines 93-94) =====
// Before:
let file_appender = tracing_appender::rolling::daily(&log_dir, "pleiades.log");
let (non_blocking, _log_guard) = tracing_appender::non_blocking(file_appender);

// After:
let timestamp = chrono::Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();
let log_path = log_dir.join(format!("pleiades.log.{timestamp}"));
let log_file = std::fs::File::create(&log_path)?;
let (non_blocking, _log_guard) = tracing_appender::non_blocking(log_file);
```

> `non_blocking` 接受 `impl Write + Send + 'static`，`File` 满足 trait bound，无需 `rolling::daily` 包装。

同时 main.rs 头部不再需要 `rolling` 功能，但 `tracing_appender` 本身仍用，Cargo.toml 无需改动（`non_blocking` 是核心 API）。

**涉及文件**：
- `Src/main.rs` — 第 93-94 行：`rolling::daily` → `File::create` + 时间戳

---

### 17.7 修正用户命令通道注释

**问题**：Phase 5 中 `user_cmd_tx → user_cmd_rx` 通道的注释只写了 TUI：

```rust
// 12. 用户命令通道 (TUI → Core)
let (user_cmd_tx, user_cmd_rx) = mpsc::channel::<UserCommand>(64);
```

实际上 CLI 模式也使用同一通道（`cmd_tx = user_cmd_tx.clone()`），发送端不同但通道共用。当前注释会误导读者以为 CLI 模式不用这个通道。

**方案**：注释改为模式无关的描述。

#### main.rs 改动

```rust
// Before:
// 12. 用户命令通道 (TUI → Core)

// After:
// 12. 用户命令通道 (用户输入 → Core)
```

> 具体输入源：TUI 模式 = `TUI_Loop` 键盘事件，CLI 模式 = `std::thread::spawn` stdin。

**涉及文件**：
- `Src/main.rs` — 注释 1 处修正

---

### 17.8 提取 CLI 模式到独立模块

**问题**：CLI 模式的实现（EventBus → stdout + stdin REPL）约 60 行直接写在 `main.rs` Phase 6 中，而 TUI 模式有独立的 `Src/TUI/` 模块。main.rs 应该只做编排，不包含具体实现细节。

**方案**：新建 `Src/CLI/mod.rs`，将 CLI 逻辑封装为一个函数 `cli_run()`，main.rs 只调用一行。

#### 新建 Src/CLI/mod.rs

```rust
//! CLI 模式：EventBus → stdout + stdin REPL

use std::io::{self, BufRead, Write};
use tokio::sync::mpsc;

use crate::event_bus::{EventBus, Bus_Event, NotifyLevel};
use crate::orchestrator::command::UserCommand;
use crate::tui::parse_user_command;

/// 启动 CLI 模式的两个并发任务 + 返回 stdin 的 sender
pub fn spawn_stdout_subscriber(event_bus: &EventBus) {
    let mut notify_rx = event_bus.Subscribe();
    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(Bus_Event::Notify { level, message }) => {
                    match level {
                        NotifyLevel::Info  => println!("{message}"),
                        NotifyLevel::Warn  => eprintln!("[WARN] {message}"),
                        NotifyLevel::Error => eprintln!("[ERROR] {message}"),
                    }
                }
                Ok(Bus_Event::Output { payload }) => {
                    println!("{payload}");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[CLI] 丢失 {} 条事件", n);
                }
                _ => {}
            }
        }
    });
}

/// 启动 stdin REPL 线程，返回 JoinHandle
pub fn spawn_stdin_repl(cmd_tx: mpsc::Sender<UserCommand>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        println!("Pleiades CLI. 输入 'quit' 退出, 'help' 查看命令.");
        loop {
            print!("> ");
            let _ = stdout.flush();
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => {
                    let (reply_tx, _) = tokio::sync::oneshot::channel();
                    let _ = cmd_tx.blocking_send(UserCommand::Quit { reply: reply_tx });
                    break;
                }
                Err(_) => break,
                Ok(_) => {}
            }
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            if line == "quit" || line == "exit" {
                let (reply_tx, _) = tokio::sync::oneshot::channel();
                let _ = cmd_tx.blocking_send(UserCommand::Quit { reply: reply_tx });
                break;
            }
            match parse_user_command(&line) {
                Ok(Some(cmd)) => {
                    if cmd_tx.blocking_send(cmd).is_err() {
                        eprintln!("[CLI] Orchestrator 已关闭");
                        break;
                    }
                }
                Ok(None) => {}
                Err(msg) => eprintln!("{msg}"),
            }
        }
    })
}
```

#### main.rs 改动

```rust
// ===== Phase 6 CLI 分支 (lines 245~310) =====
// Before (~60 行):
if cli_mode {
    // ① EventBus → stdout { let mut notify_rx = ... tokio::spawn ... }
    // ② stdin → UserCommand { let cmd_tx = ... std::thread::spawn ... }
    // ③ Core 主循环
    core.run().await;
} else { ... }

// After (~10 行):
if cli_mode {
    pleiades::cli::spawn_stdout_subscriber(&event_bus);
    pleiades::cli::spawn_stdin_repl(user_cmd_tx.clone());
    info!("进入 Orchestrator 主循环 (CLI 模式)");
    core.run().await;
} else { ... }
```

#### 需要同步的改动

- `Src/lib.rs` — 添加 `pub mod cli;`
- `Src/main.rs` — 删除 `use pleiades::tui::parse_user_command;`（现在由 cli 模块内部引用）

**涉及文件**：
- `Src/CLI/mod.rs` — 新建，~70 行
- `Src/lib.rs` — 添加 `pub mod cli;`
- `Src/main.rs` — Phase 6 CLI 分支约 60 行 → 约 10 行

---

## 变更总览

```
修改：
  Src/Config/config.toml  — [Storage] 新增 kvcache_dir 字段
  Src/Config/config.rs    — 暴露 CONFIG_DIR + Storage_Config 新增字段
                           + 新增 workspace_dir()/log_dir()/log_level() 方法
                           + 新增 OnceLock 全局静态 + kvcache_dir() 函数
  Src/main.rs             — 删除四处硬编码常量/字符串，改为调用 config 模块
                           + 日志文件命名：rolling::daily → File::create + 启动时间戳
                           + 注释修正 + CLI 模式提取到独立模块
  Src/ML_Engine/context.rs — 两处 ".kvcache" 改为 pleiades::config::kvcache_dir()
  Src/CLI/mod.rs          — 新建，收拢 CLI 模式的 ~60 行实现
  Src/lib.rs              — 添加 pub mod cli

新增：
  Task/task_17_codereview_summary.md  — 本文档
```

---

## 待讨论：组件间调用 vs. 编排层协调

### 背景

Phase 5.5 中 `storage.flush()` 之后的一系列操作（同步 PeerManager、通知 EventBus、广播给远程 peer）当前散落在 main.rs 中：

```rust
tokio::spawn(async move {
    storage.flush()
      → storage.list()
        → filter_map → SupportedModel
          → peer_manager.update_supported_models()
            → event_bus.Publish()
              → network::broadcast_local_info()
});
```

同样的逻辑在 `Src/Orchestrator/core/branch_user.rs` 中也重复了一次。

### 两条架构路线

| | A: 编排层协调（现状） | B: 组件间调用 |
|------|------|------|
| **谁负责串联** | Orchestrator / main.rs 手写每一步 | Storage 内部持有依赖，自己调 |
| **Storage 依赖** | 零依赖（纯文件管理） | 持有 `Arc<dyn Peer_Management_Capability>` + `Arc<EventBus>` |
| **main.rs Phase 5.5** | ~40 行 | 1 行 `storage.flush()` |
| **branch_user.rs** | 重复 ~15 行 | 1 行 `storage.flush()` |
| **StorageCapability trait** | 干净（7 个方法，无外部依赖） | `flush()` 签名不变，但构造时需注入依赖 |
| **测试** | Stub 只需模拟文件 | Stub 还需模拟 PeerManager + EventBus |

### 核心权衡

- **路 A** 的代价：调用方代码臃肿 + 多处重复
- **路 B** 的代价：Storage 不再是纯文件管理器，耦合了两个外部组件

### 待决策

1. Storage 是否应该持有 `peer_manager` 和 `event_bus` 的引用？
2. 如果走 B，`StorageCapability` trait 的 `flush()` 签名是否保持不变（仅内部实现变化）？
3. 对现有的 `StubStorage` 测试实现影响多大？

---

## 人类评审

<!-- 在此区域写下评审意见 -->

