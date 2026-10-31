# wb_33_uav_camera_test — rust_uav_camera_test 工作记录

## 执行引擎启动
- 引擎：Tool/execution_engine_uav_camera.ts + Tool/execution_engine_uav_camera.config.json
- 目标：`rust_uav_camera_test/`（自包含，可压缩包上树莓派）
- 计划：Step 1~2 串行，每 step 最多 3 轮（规划 → 实现 → 评审）

## Step 1 — 摄像头拍照（cargo run step 1）
- 范围：`rust_uav_camera_test/`
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/uav_camera/plan.md)（187 行，≤200）。关键结论都来自 registry 源码实证： **必须避开的三处问题** - `pleiades-uav/src/device/camera/mod.rs:100-116` 从未调用 `open_stream()`，而 V4L2 后端的 `frame()` 在未开流时直接返回 `ReadFrameError("Stream Not Started")`（`nokhwa-bindings-linux-0.1.4/src/lib.rs:878-897`）。计划中已把 `open_stream()` 固定为打开序列中的一步，导出的函数 `Open_Camera/Capture_Frame` 也不使用旧的 `Camera` 类型，而改用自定义句柄。 - `nokhwa::Camera` 内部的 `Box<dyn CaptureBackendTrait>` 没有 `Send` 约束（`nokhwa-core-0.1.9/src/traits.rs:41`），因此不能跨线程传递，只能在 worker 线程内创建并持有摄像头对象；打开超时通过在 main 线程用 `recv_timeout(8s)` 等待 ready 实现，超时后 Drop 不 join，仅丢弃句柄。 - 未启用 `decoding` 时，MJPEG 解码返回 `NotImplementedError`（`nokhwa-core-0.1.9/src/types.rs:1508-1513`），所以 MJPEG 分支只能直通 `buffer().to_vec()`。注意枚举名是 `FrameFormat::MJPEG`，不是 MJPG。 **规划要点** - 4 个新文件（`Cargo.toml`、`src/main.rs

## Step 2 — YOLO 吞吐（cargo run step 2，10 秒报 FPS）
- 范围：`rust_uav_camera_test/`
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/uav_camera/plan.md)（恰好 200 行，未改动任何代码/配置/文档）。 **核对到的关键代码事实（决定了计划的取舍）：** - `model.rs` 虽只 `use candle_core/candle_nn`，但**用到了 `tracing::Span` / `span!`**（`pleiades-base/src/ML_Engine/Yolo/model.rs:5-6,81,105`）——"原样保留"就必须额外加 `tracing 0.1`，这是最容易漏掉的一处编译错误。 - `detector.rs` 要摘掉的四处依赖：`candle_transformers`（`:13`）、`Parse_Device_Str`（`:17`，实现可照抄 `ML_Engine/device.rs:17-34`）、`config`（`:78-80`）、`mlua`（`:214-243`）。 - `nms.rs` 只需内联 `object_detection.rs:8-58`（`Bbox`/`KeyPoint`/`iou`/`non_maximum_suppression`），soft-NMS（`:61-116`）不要，否则死代码。 - 实测确认 `cargo run --offline step 2 --secs 3` 的 `--secs 3` **会原样传给二进制**（落到现有"未知参数"分支），故 CLI 表可直接写这种形式，无需强制 `--`。 - candle 两 crate 在 0.10.2 的 `default = []` 本已为空，仍显式 `default-features = false` 以防版本漂移；registry cache 与 index cache 齐备（`cargo metadata --offlin
