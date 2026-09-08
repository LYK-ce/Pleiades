# Workbook — Task 26: Godot 命令通道（Pictor 通过 terminal 下发 UserCommand）

> 对应任务：`Task/task_26_godot_command_channel.md`
> 分支：`robot_yolo`
> 创建日期：2026-09-08

## 状态

Orion 侧（Rust 侧，`pleiades-terminal`）全部改动已实施 + `cargo check` 通过。Godot 侧（`kernel_bridge.gd` 转发 + 文本框/启动流程 + `llm.gd` api_url）待实施（方案 §六为占位，待与人类讨论后补充）。

## 实施前核实（方案 §二 D1/D6 前提全部成立）

- `CoreBootstrap.user_cmd_tx: mpsc::Sender<UserCommand>` pub —— `pleiades-base/src/bootstrap.rs:43`
- `parse_user_command(&str) -> Result<Option<UserCommand>, String>` pub 纯函数（不依赖 TUI App 状态）—— `TUI/mod.rs:523`
- `UserCommand` pub —— `Orchestrator/command.rs:15`
- 导出路径 `pleiades_base::tui`（`lib.rs:49`）/ `pleiades_base::orchestrator::command`（`Orchestrator/mod.rs:7`）均 pub
- 接口自 2026-09-03 方案定稿以来未变，方案 §五直接适用

## 改动文件（仅 `pleiades-terminal/src/lib.rs`，未碰 base）

1. import 追加：`UserCommand`、`parse_user_command`、`tokio::sync::mpsc`
2. `PleiadesKernel` 加字段 `user_cmd_tx: Arc<OnceLock<mpsc::Sender<UserCommand>>>` + `init()` 初始化
3. `spawn_background`：clone `self.user_cmd_tx` + 在 `run_headless()` 之前 `user_cmd_tx.set(boot.user_cmd_tx.clone())`
   - **关键**：`run_headless(self)` 按值消耗 self，必须在调用前 clone 出独立 sender
4. 新增 `#[func] fn send_user_command(&self, command_line: GString) -> GString`：同步、`try_send`（非阻塞）、`parse_user_command` 复用

## 验证

- `cargo check -p pleiades-terminal` → 0 error（base 24 warning + terminal 7 warning 均为改动前已存在，本次改动未引入新 warning）

## 关键坑（后续 agent 注意）

- godot `GString` **没有** `From<String>`，只有 `From<&str>` / `From<&String>`；拼错误消息须 `GString::from(format!("ERR: {msg}").as_str())`，直接 `GString::from(format!(...))` 报 E0277
- `parse_user_command` 对 `quit`/`exit`/`clear`/空串 返回 `Ok(None)`（本地命令）→ `send_user_command` 返回 `"OK (本地命令)"`，不真正退出 Core（`UserCommand::Quit` 需 oneshot，解析器刻意不构造）
- `#[func]` 是 Godot 主线程同步调用，用 `try_send`（非阻塞）而非 `send().await`，避免阻塞渲染线程

## 代码审查（2026-09-08 子 agent）

结论：**有条件通过**，无🔴严重 bug、无🟠正确性风险。

核验通过：
- 生命周期：`run_headless(self)` 按值消耗 self，先 clone 存 OnceLock、后 run_headless 是唯一正确时机（set 在 `lib.rs:223`，run_headless 在 `lib.rs:275`）
- 线程安全：`OnceLock` 后台线程 set / Godot 主线程 get 有 happens-before 保证；`try_send` 非阻塞（buffer 64），不卡渲染线程
- headless 模式下 Core B1 分支（`user_cmd_rx.recv()`，`core.rs:137`）不依赖 TUI，命令真的会被消费
- 方案 §五 5 个改动点全部落实

审查后修复：
- P1：文件头 Modified Date bump `2026-08-16 → 2026-09-08`（规范必改）
- P3：`try_send` 失败区分 `Full`（可重试）/ `Closed`（不可恢复），返回语义更清晰

保留不修（提示性）：
- P2：`quit`/`exit` 返回 `"OK (本地命令)"` 字面可能误导，但方案 §七 已声明 Godot 不下发，可接受
- P4/P5：`.as_str()` 冗余、import 顺序与方案措辞差异，无实质影响
- P6：`help` 结果无法回传（D4 既定不做回传），需回传时另行设计

## 结束时间

（待 Godot 侧实施后填写）
