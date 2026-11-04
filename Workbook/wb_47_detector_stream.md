# wb_47_detector_stream — detector 检测流（WS 推「图 + 框」） 工作记录

> Presented by KeJi
> Created Date ： 2026-11-04
> Modified Date ： 2026-11-04
> 任务：Task/task_47_detector_stream.md（detector 检测流（WS 推「图 + 框」））

## 执行引擎启动
- 开始时间：2026-11-04 10:35:55Z
- 引擎：tmp/task47_tools/execution_engine_task47.ts + .config.json（流程照搬 task_46）
- 运行标记：task_47 首轮
- 起始 step：1；计划：S1 → S2 → S3 → S4 → S5 → S6(mail)

## Step 1 — base：类型化发布口 + 幂等服务启动（task_47 首轮）
- 起止：2026-11-04 10:35:55Z → 2026-11-04 10:43:22Z
- 范围：pleiades-base 的 API 层（webui.rs + mod.rs）
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 2d570c9
- 规划要点：计划已写入 [plan.md](tmp/task47/plan.md)（294 行）。源码未改，git diff 为空，与基线一致。 **计划摘要（200 字内）** Step 1 只动两个文件。`API/webui.rs`：新增 `StreamDet`（7 字段，`&'static str` class_name）；拆 `build_header_json`、`pack_frame`（`[4B LE][JSON][JPEG]`）两个纯函数，`webui_publish` 改为复用 `pack_frame`（Lua 契约不变）；`publish_detection` 同步返回 `()`、`let _ = tx.send`、每帧都推、计数先行；`ensure_webui_server(SocketAddr)` 用 `ENSURE_LOCK` 串行 + `OnceLock<SocketAddr>` 缓存，bind 成功后才建广播通道并回填 `local_addr` 端口；`spawn_webui_server(Option<u16>)` 留 3 行薄包装供 `branch_user.rs:514` 编译。另加 `#[doc(hidden)] publish_count/is_server_started`（必须恒编译，下游测试才可见）。`mod.rs:15` 换 re-export。4 个单测覆盖 LE 布局、header 键名（`conf` 而非 Lua 的 `confidence`，查看器不读、口径 #7 成立）、幂等同端口、未启动不 panic。验收：`--lib` check 0 error/0 warning、`--lib webui` 4 passed、`git diff --name-only` 仅两文件。

## Step 2 — base：移除 TUI webui 命令（task_47 首轮）
- 起止：2026-11-04 10:43:22Z → 2026-11-04 10:51:26Z
- 范围：TUI 命令解析 + UserCommand + Core 分支 + Step 1 的临时包装
- 轮次：1 / 3
- 结果：PASS
- 提交：ok a022158
- 规划要点：计划已写入 [plan.md](tmp/task47/plan.md)（334 行，源码零改动，`git diff` 仍只有 workbook）。 **计划摘要**：涉及 5 个文件，删除 TUI `:561-574` 的解析块、`command.rs:69-72` 的 `Webui` 变体、`branch_user.rs:508-529` 的分支（保留 `:505-507` 的横幅）以及 `webui.rs` 中 `spawn_webui_server` 包装和 `L29` 的 `import`，并把 `mod.rs` 的 re-export 合并为一行。一次改完再编译：`--lib` 0 error、warning 行数不超过 25、三串 grep 均 0 命中、`webui` 只剩服务本体和 `caps` 注册链（4 个文件、21 行，`register_webui_caps` 按 #15 保留）；`--all-targets` 按 `step.md` 改用 `--lib`。

## Step 3 — 三份 detector：配置 + 启动 + 每帧推（task_47 首轮）
- 起止：2026-11-04 10:51:26Z → 2026-11-04 11:07:03Z
- 范围：ugv / uav / sim 的 config.rs + detector/mod.rs
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 7868a34
- 规划要点：计划已写入 [plan.md](tmp/task47/plan.md)（620 行），源码零改动（`git diff` 为空，与 `tmp/task47/git_baseline.txt` 一致）。 **计划摘要（200 字内）**：改 6 文件（三份 config.rs + 三份 detector/mod.rs）。config 加 3 常量/字段/访问器，ugv/uav `fill_detector` 补 3 键，修 sim `:680` 穷举字面量；detector 加 `Parse_Stream_Addr`（非法回退本机）与 `to_stream_dets`（类名搬指针），spawn 在 `Spawn_Tuned` 成功后才起流（失败只 warn），`Loop_Ctx.stream`，`jpeg` 统一 `Arc<[u8]>`（ugv 翻转分支 `Arc::from`），快照写入后用预存 `stream_dets` 推帧（无 `is_empty`、不 await）。4 个新用例覆盖映射、0 框仍推、`stream=false` 零 bind，`PUBLISH_LOCK` 串行防全局计数污染，不起真 WS。check 0 error、警告不超基线、`--bins detector` 全绿、两两 diff 只剩文件头/别名/`FRAME_TS_SOURCE`/`flip_vertical`。

## Step 4 — 前端：yolo_viewer 去掉 FPS（task_47 首轮）
- 起止：2026-11-04 11:07:03Z → 2026-11-04 11:16:33Z
- 范围：Tool/yolo_viewer.html（只删 FPS + 加连接状态）
- 轮次：1 / 3
- 结果：PASS
- 提交：ok bd3e037
- 规划要点：计划已写入 [plan.md](tmp/task47/plan.md)（319 行），源码零改动（`git status` 仍只有基线里的 workbook 与 `Tool/investigation/`）。 **计划摘要（约 175 字）** 只改 `Tool/yolo_viewer.html`（119→100 行，+9−28，10 hunk）。删 `.fps` CSS、总 FPS 文本、两计数器、`last`、计数两行、差分 FPS、整块 `setInterval`；`:44` 改 `class="status" id="st${idx}"` 初值「连接中…」，`:53` 取 `stEl`，`:60-62` 写 已连接/连接失败/断开（保留 `socks[idx]===ws` 守卫）；文件头加 3 行。拆包画框零改动、端口 9010 不变。已在 `tmp/task47/preview_yolo_viewer.html` 预演：fps grep 0 命中、`<script>` 1、`node --check` 通过、100 行、10 hunk。15 条静态判据，不跑 cargo/浏览器。 配套产物：`tmp/task47/preview_yolo_viewer.html`（已逐字节验证的目标文件，可 `cp` 落地）、`tmp/task47/make_preview_step4.py`（生成脚本，含行号断言）。计划里另登记了 Step 5 必改项：`Architecture/robot_arch.md:700` 仍写「浏览器算 FPS」——本步不得动。

## Step 5 — 文档 + 全仓静态终检（task_47 首轮）
- 起止：2026-11-04 11:16:33Z → 2026-11-04 11:28:43Z
- 范围：Architecture/robot_arch.md 的 detector 节 + 跨 crate 静态核对
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 916dd0f
- 规划要点：计划已写入 `tmp/task47/plan.md`（433 行）；全程只读源码、未改任何仓内文件（`git status` 仍只有进场时的 workbook 与 `Tool/investigation/`）。 **计划摘要（200 字内）** 只改 `Architecture/robot_arch.md`，6 处编辑：文件头更新日期 2026-11-04（旧的降为上一版/更早）、`:700` 去掉「端口自动分配 + 浏览器算 FPS」、`:712` 补 ugv 第三份 detector、`:716` 补三新键与 `fill_detector` 归属、`:717` 后新增「#### 画面流（task_47）」6 条要点 + toml 示例（三键默认值、仅 spawn 成功才起服务/`stream=false` 零 bind 零线程、`[4B LE][JSON][JPEG]` 与 header 六字段、每帧都推与 100–200KB@10Hz 带宽、静默丢弃不影响推理、查看器无 FPS 只显示连接状态）、常量速查表尾补 `stream_port` 9010 / `stream_bind` 127.0.0.1。终检 8 项只读：4 条 cargo check（base 仅 `--lib`）、4 条测试（base `--lib webui`、设备 `--bins detector`）、publish_detection 恰 5 文件、TUI webui 0 命中、viewer fps 0 命中、三份 detector diff 13/13/6 hunk 归四类、保护路径 `git diff --stat 16df4a5` 空、ugv/uav 无 axum/serde_json；问题只记 impl.md。上轮 issues：(无)，首轮。

## Step 6 — 邮件通知（引擎执行）（task_47 首轮）
- 起止：2026-11-04 11:28:43Z → 2026-11-04 11:28:45Z
- 结果：邮件已发送
- 摘要：状态 ALL_STEPS_DONE；无失败步

## 引擎结束
- 结束时间：2026-11-04 11:28:45Z
- 状态：ALL_STEPS_DONE
- 需人类亲验：浏览器 + 带相机的 detector 节点（画面持续刷新 / 框正确 / 无 FPS / 连接状态）
- 邮件：已发送
- git status（结束时）：
```
M Workbook/wb_47_detector_stream.md
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
```

## 主 agent 独立复核（2026-11-04，引擎结束后）

### 编译 / 测试（亲自跑，不采信评审转述）
- `cargo check -p pleiades-base --lib` + 三个设备 crate `--all-targets` → **全部 0 error**（exit 0）。
- 新增测试全绿：`base --lib webui` 4 passed；`ugv --bins stream` 4 passed；`uav --bins stream` 10 passed；`sim --bins stream` 3 passed。
- **存量红（非本任务引入，未修）**：`cargo test -p pleiades-ugv --bins detector` → `test_ensure_ugv_section_fills_detector_camera`（断言 `mount_z = 0.045` 与 `Some(0.135)` 自相矛盾，`config.rs:834`）与 `projection::tests::test_from_config_defaults_and_illegal`（`projection.rs:633`）。已在 `16df4a5` 版本核对：两处断言当时就在，`git diff 16df4a5..HEAD` 未触碰这两行 → 属全项目梳理里记的「ugv detector 默认值」组存量问题，本任务按口径不修。

### 关键实现抽查
- `publish_detection` 只在 5 个文件出现（base webui/mod + 三份 detector），device crate 无 axum/serde_json。
- 循环里 `if stream { publish_detection(&jpeg, ...) }`，**无 `!dets.is_empty()`**；ugv 用翻转后的局部 `jpeg`。
- `ensure_webui_server`：`ENSURE_LOCK` 串行 + `OnceLock<SocketAddr>`；已启动返回原端口；`port=0` 用 `local_addr()` 回填；bind 成功后才建广播通道。
- `Parse_Stream_Addr`：空串 / 非法 → `warn` + `127.0.0.1`。
- `Tool/yolo_viewer.html`：`grep -in 'fps\|totalFrames\|totalStart'` **0 命中**；状态文本四态齐；拆包/画框未动。
- `grep -rn 'UserCommand::Webui\|spawn_webui_server' pleiades-base/src` **0 命中**。

### 已知偏差（不影响功能）
- 评审称 Step 3「`--bins detector` 全绿」不准确：实际含 2 条上述存量红（filter 把 config/projection 用例也匹配进来了）。
- detector 循环里注释编号出现 ⑦→⑥→⑦ 的顺序错位（纯注释）。

### 待人类
浏览器 + 带相机的 detector 节点验收（画面持续刷新 / 框正确 / 框消失后仍在动 / 无 FPS / 连接状态）。
