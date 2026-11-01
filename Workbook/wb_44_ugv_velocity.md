# wb_44_ugv_velocity — task_44 工作记录

> Presented by KeJi
> Created Date ： 2026-11-01
> Modified Date ： 2026-11-01
> 任务：Task/task_44_ugv_velocity.md（四端统一连续运动控制 + 真车行为树复制）

## 执行引擎启动
- 开始时间：2026-11-01 04:52:39Z
- 引擎：tmp/task44_tools/execution_engine_task44.ts + .config.json（流程照搬 task_43）
- 运行标记：task_44 首轮（Step 1~8）
- 计划：Step 1~5 / 7 最多 3 轮（规划 → 实现 → 评审）；Step 6 / 8 **只跑 1 轮**，失败记录后继续
- 范围：协议（1）→ sim 两栈（2）→ Pictor（3）→ 真车 0x12+M1+导航订阅（4）→ 真机（5）→ 仿真验证（6）→ 真车行为树（7）→ 真车端到端（8）

## Step 1 执行记录（协议层：msgid 4 两轴 + msgid 8 模式）

**改动**：`pleiades-base/src/Robot/core/command.rs`、`protocol/{messages,mod,command_decode}.rs`、`docs/design_doc/orion_protocol.md`（仅这 5 个已跟踪文件；`git diff --stat` 无越界）。

**S0 基线**：`cargo test -p pleiades-base --lib protocol` → **39 passed / 0 failed**（与 task_41 记录值一致）。

**V1 编译（硬门禁）**
- `cargo check -p pleiades-base --lib` → `Finished`，**0 error**；`pleiades-base (lib) generated 24 warnings`（全部存量）。
- 新增代码 0 warning；唯一命中本步文件的 warning 是 `unused import: super::frame::MAGIC`（`messages.rs:9`，仅 `#[cfg(test)]` 使用，**改动前即有**，未顺手动它）。

**V2 单测（硬门禁）**
- `cargo test -p pleiades-base --lib protocol` → **49 passed / 0 failed**（39 − 1 条删除的 `test_manual_control_roundtrip` + 11 条新增）。
- 新增 11 条：`messages.rs` 5 条（金标 / 长度分流 / action 表 / 位模式往返 / 模式往返）、`command_decode.rs` 6 条（8 B→Motion / 5 个事件 / 未知 action 拒 / 非法长度拒 / msgid 8 变体断言 / 模式非法值拒）。

**V3 存档观测（不判 FAIL）**
- `cargo test -p pleiades-base --lib` → 224 passed / **2 failed**，仍是存量 `vm::engine::tests::test_load_and_execute_hello_lua` / `test_sandbox_os_blocked`。
- `cargo check -p pleiades-base --all-targets` → `grep -c "^error"` = **71**；**全部落在 `pleiades-base/tests/*.rs` 存量集成测试**（t01_storage / t04_peer / t07_network / t09_lua / t13_hf2gguf），`src/Robot` 与 `protocol` **零 error**。与 task_41 记录的 ≈30 差异只因计数方式（`^error` 含 `error: aborting` 收尾行）；文件集合未变，属存量。

**V4 变体删除（硬门禁）**：四条 grep（`ManualCmd::{Forward,Backward,SpinLeft,SpinRight}` / `ACTION_{FORWARD,BACKWARD,SPIN_LEFT,SPIN_RIGHT,SWITCH_TO_}` / `ACTION_STOP\b` / `ManualControl`）**全部无输出**。

**V5 新接口（硬门禁）**：`MSGID_MODE: u16 = 8`（mod.rs:41）；`ManualPayload` / `encode_manual_motion` / `encode_manual_event` / `decode_manual_control` / `encode_mode` / `decode_mode` 六项齐全；`command_decode.rs:30` 有 `MSGID_MODE =>` 分支。

**V6 金标（硬门禁）**：`3f00000000000000` / `0000000000000000` / `00000000bf000000` 在**文档 §3.4 与 messages.rs 单测两处都命中**；模式 `00`/`01` 在文档以 `| 0 | 手动 |`、`mode=manual -> 00` 形式出现。

**V7 文档三处（硬门禁）**：总览表有 `| 8 | ORION_MODE |`；`### 3.4`（新布局）/ `### 3.6`（MODE，msgid 8）/ `### 3.7`（RTCM，原 3.6 顺延）三者都在；`switch_to_manual` 只作为「旧值/已删除」说明出现（§3.4 旧值警告表 + §3.6 取代关系）。

**V8 改动面**：`git status --short` = 5 个已跟踪文件（command.rs + protocol 三个 + orion_protocol.md）+ 基线 3 个 untracked（`Tool/investigation/`、`Workbook/wb_44_ugv_velocity.md`）+ `tmp/task44/`；`git diff --stat` 不含 `Task/` / `Cargo.toml` / `Cargo.lock` / `Pictor/` / 设备 crate。

**V9 越界自检**：`git diff --name-only | grep -v <允许的 4 条路径>` → **无输出**。

**V10 Pictor 仓**：`git -C Pictor status --short` 与进场一致（**空**，本步不碰 Pictor）。

**跨 crate 已知红（预期，交 Step 2/4/5）**——`ManualCmd` 四个运动变体已删，下列旧引用在 Step 1 后必然编译失败，**本步不改**：
- `pleiades-sim/src/sim/control.rs:356-359`（`"forward"/"backward"/"spin_left"/"spin_right"` → 变体）、`:1103-1115`（单测）
- `pleiades-sim/src/sim/chassis/mod.rs:153-156`、`chassis/ugv.rs:161-173`、`chassis/uav.rs:168-176`
- `pleiades-ugv/src/ugv/stm32/mod.rs:151-154`
- `pleiades-uav/src/uav/mavlink/mod.rs:311-314`
> 已 grep 确认：设备 crate **不**引用本步删除的协议常量/函数（`ACTION_*` / `encode_manual_control` / `decode_manual_control`），断裂面只有上述 7 处 `ManualCmd` 变体引用。

**文档节序说明**：按任务文档 §二 Step 1 字面「新增 §3.6 模式切换」，MODE 落在 §3.6、原 RTCM 顺延为 §3.7（节序略偏离 msgid 升序，§3.6 首段已加一句指路）。

**未做（按计划 §2.4 守住）**：不做 `v`/`omega` 有限性校验、不做版本兼容/旧 action 映射、不改 `Frame`/`encode_frame`/compid、不碰设备 crate / Pictor / Task / 根 Cargo、不 `cargo fmt`、不 `git add`。

## Step 1 — 协议层：msgid 4 改两轴连续量 + 新增 msgid 8 模式切换（task_44 首轮（Step 1~8））
- 起止：2026-11-01 04:52:39Z → 2026-11-01 05:02:05Z
- 范围：约 base 协议（messages/mod/command_decode）+ command.rs 的 ManualCmd + 协议文档
- 轮次：1 / 3
- 结果：PASS
- 提交：ok c1424c4
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（420 行，未改任何源码）。 **摘要（≤200 字）** Step 1 只在 base 协议层实施：`messages.rs` 把 `ManualControl` 换成 `ManualPayload`（8 B 运动、3 B 事件，其它长度直接拒绝）、action 重排为 1 到 5、增加 `MODE_*` 与 `msgid 8` 编解码、五条金标逐字节写进单测；`mod.rs` 加 `MSGID_MODE=8` 并重排导出；`command_decode.rs` 按长度分流，`msgid 8 → Command::Mode`（用 `matches!` 断言变体，并反向断言不是 Manual）；`command.rs` 删四个运动变体、加 `Motion{v,omega}`、留 `Stop`；文档改 §3 总览表、整写 §3.4、新增 §3.6 模式（RTCM 顺延 §3.7）、§5.2/§5.4 注明 `d(yaw)/dt=−ω`。 验收：`cargo check -p pleiades-base --lib` 与 `cargo test -p pleiades-base --lib protocol`，11 条新用例；`--all-targets` 存量 30 errors、`--lib` 存量 2 failed 只对照基线记数（V1~V10 已列）。风险：sim/ugv/uav 旧变体引用必红，属预期，交 Step 2/4/5；Pictor 一律不碰。上轮 issues 为(无)，首轮已预防 6 个失分点。

## Step 2 — sim 车栈 + sim 机栈换 (v, omega)，删离散执行器与遗留文件（task_44 首轮（Step 1~8））
- 起止：2026-11-01 05:02:05Z → 2026-11-01 05:19:40Z
- 范围：pleiades-sim 两栈执行器/底盘/控制口/world_link/config + 世界 DiffDrive；删除 sim/executor.rs
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 5747085
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（408 行，未改任何源码；`git status` 仍只有基线两项）。 **摘要（≤200 字）** Step 2 计划要点：`VelocityCmd`→`{v,omega}`（车 0.45 / ±0.5，五档顺序与三阈值不变，两栈各一份）；`SpeedBinding`→`cruise_speed 0.45` / `cruise_yaw_rate 0.5`，删旧百分比键；`Actuator` 三态收两态，删 `Discrete_Actuator`、`Apply_Velocity_Pct`、`sim/executor.rs`；车机统一 `Apply_Velocity`；`motion_cmd` 删 `turn` 键（`speed` 留给 up/down/stop）、velocity 帧只带 v/omega；控制口 `manual` 改 `{v,omega}` 带 ±0.45/±0.5 校验；`DiffDrive.gd` 直收物理量。三个既定决策：`Apply_Action` 留默认空实现（`emergency_stop.rs` 越界）；不改 world 的 `src/tests/`，其离散用例登记为已知红；不碰 docs/Architecture。验收：`cargo check --all-targets` 0 error、`--bins` 全绿、6 组 grep 为空、越界自检。

## Step 3 — Pictor：WASD 两轴 + 模式切换走 msgid 8（task_44 首轮（Step 1~8））
- 起止：2026-11-01 05:19:40Z → 2026-11-01 05:36:14Z
- 范围：Pictor 独立仓（分支 godot-rust）：input_handler / protocol_def / message_builder / orion_messages / kernel_bridge / ws_command_server / 协议测试 / CLI
- 轮次：1 / 3
- 结果：PASS
- 提交：ok da48700
- 规划要点：计划已写入 `tmp/task44/plan.md`（420 行；未改任何源码，`git -C Pictor status` 仍为空）。 **摘要（≤200 字）** 计划写 tmp/task44/plan.md（420 行，未改源码）。10 个文件：protocol_def 删旧常量、action 重排 1..5、加 MSGID_MODE=8 与 V_MAX=0.45/OMEGA=0.5；orion_messages 改 8B 运动/3B 事件/1B 模式；builder 换 4 个新函数；input_handler 重写「按住集合→(v,ω)→变了才发」；ws_command_server 加 manual 门控、mode 走 msgid 8；测试入口必须改 `_initialize`（实测 `_init` 下 load 含 EventBus 的脚本必崩）→ 24 PASS；py 加 manual --dry-run。docs/ 与 check_ws 的 A9 在允许路径外，登记为已知陈旧。
