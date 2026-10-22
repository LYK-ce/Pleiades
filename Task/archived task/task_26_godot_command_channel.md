# task_26_godot_command_channel — Godot 命令通道（Pictor 通过 terminal 下发 UserCommand）

> Created Date ： 2026-09-01
> Modified Date ： 2026-09-03
> 状态：方案已定（最小改动：仅命令通道，无结果回传），⚠️ 暂且搁置（2026-09-03 与人类讨论后暂缓实施）
> 关联文档：`Architecture/robot_arch.md`、`docs/design_doc/pictor_bridge_sync.md`、`.github/instructions.md`

---

## 一、目标

让 Pictor（Godot 地面站）能像 TUI 一样，向 `pleiades-terminal`（GDExtension 内核）下发命令字符串（如 `session create xxx` / `session inference single_inf 1 xxx` / `api 1` / `exec xxx`），由内核解析后经 `user_cmd_tx` 送入 Core 主循环执行，从而在 Godot 侧启动大模型推理服务（OpenAI 兼容 API）。

## 二、设计决策（已定稿）

| # | 决策 |
|---|------|
| D1 | 命令通道 = 复用现有 `user_cmd_tx`（`mpsc::Sender<UserCommand>`），在 terminal 里 clone 一份，不在 base 新增任何通道 |
| D2 | Godot 传命令字符串（与 TUI 同语法），复用现成 `parse_user_command()` 解析，不做结构化参数、不在 Godot 侧写解析器 |
| D3 | 新增 `#[func] send_user_command(command_line: GString) -> GString`，内部 `parse_user_command` + `try_send`，同步返回 "OK" / "ERR: ..." |
| D4 | 不做结果回传通道（不加 `log_message` / `command_result` 信号） |
| D5 | session_id / API 端口由 Godot 侧写死（session_id=1，端口=8080），不在本任务处理 |
| D6 | 不修改 `pleiades-base`：`parse_user_command`（`pleiades_base::tui`）与 `UserCommand`（`pleiades_base::orchestrator::command`）均已 pub，可直接引用 |
| D7 | CUDA / `.gdextension` 库名 / 模型加载时序等问题不在本任务范围（由人类另行处理） |

## 三、数据流

```
Godot 文本框 → 命令字符串("session create xxx") → PleiadesKernel.send_user_command(command_line)
  → parse_user_command(command_line) → Result<Option<UserCommand>, String>
  → user_cmd_tx.try_send(cmd)         (mpsc::Sender<UserCommand>，clone 自 CoreBootstrap)
  → Core 主循环 B1 分支 user_cmd_rx.recv()
  → route_user(cmd)
  → 各命令处理（Execute / Session / SessionInference / Api / Flush / ...）
```

## 四、涉及文件（Orion 侧）

| # | 文件 | 改动性质 |
|---|------|---------|
| 1 | `pleiades-terminal/src/lib.rs` | 全部改动（见第五节） |
| — | `pleiades-base/**` | 无改动 |

## 五、具体改动点（`pleiades-terminal/src/lib.rs`）

### 5.1 import（顶部 use 区追加）

```rust
use tokio::sync::mpsc;
use pleiades_base::orchestrator::command::UserCommand;
use pleiades_base::tui::parse_user_command;
```

### 5.2 `PleiadesKernel` 结构体加字段

在 `node_handle` 字段旁追加：

```rust
/// 命令通道发送端（core_bootstrap 就绪后填充，clone 自 CoreBootstrap.user_cmd_tx）
user_cmd_tx: Arc<OnceLock<mpsc::Sender<UserCommand>>>,
```

### 5.3 `init()` 初始化

```rust
user_cmd_tx: Arc::new(OnceLock::new()),
```

### 5.4 `spawn_background` 存 clone（必须在 `run_headless()` 之前）

```rust
let user_cmd_tx = self.user_cmd_tx.clone();        // 与 node_handle.clone() 同处
// ...
let _ = user_cmd_tx.set(boot.user_cmd_tx.clone()); // 紧随 node_handle.set(...)
```

### 5.5 新增命令入口方法

```rust
/// 下发命令字符串（与 TUI 同语法），同步返回执行反馈
#[func]
fn send_user_command(&self, command_line: GString) -> GString {
    let Some(tx) = self.user_cmd_tx.get() else {
        return GString::from("ERR: 命令通道未就绪");
    };
    match parse_user_command(&command_line.to_string()) {
        Ok(Some(cmd)) => match tx.try_send(cmd) {
            Ok(()) => GString::from("OK"),
            Err(_) => GString::from("ERR: 通道已满/关闭"),
        },
        Ok(None) => GString::from("OK (本地命令)"),
        Err(msg) => GString::from(format!("ERR: {msg}")),
    }
}
```

---

## 六、Godot 侧（待讨论）

占位：`kernel_bridge.gd` 暴露 `send_user_command` 转发 + 文本框/启动流程 + `llm.gd` api_url 指向本地。待与人类讨论后补充。

## 七、备注

- `parse_user_command` 会把 `quit` / `exit` / `clear` 解析为 `Ok(None)`（本地命令）；Godot 场景下 `quit` 会触发 Core 优雅退出，人类已确认不会输入这些命令，无需特殊处理。
- 命令是 fire-and-forget；`try_send` 通道满则返回 false（"ERR: 通道已满/关闭"），Godot 侧可重试。
