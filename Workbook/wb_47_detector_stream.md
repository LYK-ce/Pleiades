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
