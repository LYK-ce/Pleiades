# Workbook — Task 28: 无人机 USB 摄像头子设备

> 对应任务：`Task/task_28_camera_device.md`（也作为 `task_29_yolo.md` 阶段 1 落地）
> 分支：`robot_yolo`
> 创建日期：2026-09-08

## 状态

已实施 + Windows 实机验证通过（`exec test_camera` 成功抓图存盘）。

## 关键决策（与 task_28 原始方案 D1-D9 的差异）

- **nokhwa `0.10.11`**（非 0.11，0.11 不存在），平台条件依赖：Linux `input-v4l` / Windows `input-msmf`
- **fourcc NV12**（非 MJPG）：实测 Chicony 笔记本摄像头 30 种格式全 NV12，无 MJPG
- **NV12 → `decode_image::<RgbFormat>()` → `image` 编码 JPEG**（不做 fallback，默认 NV12）
- **装配位置 `uav_bootstrap`**（非 `UavDeviceHandler::start()`）：camera 被 base 的 Lua 调用（非 uav 决策链），且需在 `exec` 前注入 `device_caps`
- `CameraDevice` 实现 `DeviceCapability`，Lua 侧 `camera.capture()` 返回 JPEG 字节（二进制安全字符串）

## 改动文件

- **uav 新增**：`device/mod.rs`、`device/camera/mod.rs`（CameraDevice ~200 行：常驻线程 + NV12→RGB→JPEG + DeviceCapability）
- **uav 改**：`Cargo.toml`（nokhwa/image/mlua）、`config.rs`（CameraConfig + fill_camera + 测试）、`main.rs`（mod device + 传 capabilities）、`bootstrap.rs`（创建/open camera + push device_caps）
- **base 改**：`bootstrap.rs`（CoreBootstrap 暴露 capabilities）、`Orchestrator/mod.rs`（device_caps Vec→RwLock）、`branch_user.rs`（遍历加 read()）、`storage_handle.rs`（StorageWriteHandle 补 write 方法）
- **新增脚本**：`programs/user/test_camera.lua`

## 验证

- `cargo check -p pleiades-base --no-default-features` / `-p pleiades-uav` → 0 error
- `cargo test -p pleiades-uav config::` → 2 passed（fill_camera 补全）
- Windows 实机：`[camera] enabled=true` + `exec test_camera` → 抓图存 `Pleiades_Workspace/test_camera.jpg` 成功

## 关键坑（后续 agent 注意）

- `device_caps` 在 `Arc<Capabilities>` 里无法 `push`，必须改 `RwLock<Vec<...>>`
- uav 的 mlua 版本必须与 base 严格一致（`0.12.0-rc.1`），否则 `impl DeviceCapability` 类型对不上
- nokhwa 最新稳定版是 `0.10.11`（无 0.11）
- Lua `io` 被禁用（`engine.rs`），写文件走 `storage_acquire_write` + `handle:write`（不打开 io）
- `camera.capture()` 是 `create_async_function`，在 async 上下文（`execute` 被 `call_async` 调用）直接调用即得结果

## 结束时间

2026-09-08
