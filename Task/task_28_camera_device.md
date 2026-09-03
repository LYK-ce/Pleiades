# task_28_camera_device — 无人机 USB 摄像头设备驱动

> Created Date ： 2026-09-03
> Modified Date ： 2026-09-03
> 状态：方案已定，待实施
> 关联文档：`Architecture/robot_arch.md`、`docs/design_doc/UAV.md`（DRF450 摄像头：200 万像素 1080P USB）

---

## 一、目标

把无人机（DRF450）的 USB 摄像头做成一个**子设备 `CameraDevice`**（与 `mavlink` / `lg290p` 同级），第一版只实现「**拍照返回 JPEG 字节**」，为后续 YOLO / 推流 / 地面站显示打基础。

- 设备：USB UVC 摄像头，Linux 下 `/dev/video0`，MJPEG 输出。
- 第一版**只做驱动本身**；发图给小车、地面站显示、YOLO、Lua caps 均为后续扩展（见第八节）。

## 二、设计决策

| # | 决策 |
|---|------|
| D1 | 子设备，放 `pleiades-uav/src/device/camera/`，与 `mavlink`/`lg290p` 同级 |
| D2 | 底层用 **`nokhwa`**（`default-features = false, features = ["input-v4l"]`）——std::thread 常驻模式下 `Camera` 非 Sync 无碍，`frame()` 一行取帧 |
| D3 | API 四方法：`new` / `open` / `capture` / `close`（显式生命周期） |
| D4 | 线程模型：`open` 起 `std::thread` 常驻线程（持摄像头 + loop 等命令）；`capture` 用 `oneshot` 回传（async）；`close` 关线程 |
| D5 | fourcc **固定 MJPG**（代码写死，不暴露 config），读一帧 = 一张 JPEG 字节，可存图/显示 |
| D6 | config `[camera]`：`enabled`（默认 false）/ `path` / `width` / `height` / `timeout_ms` |
| D7 | MJPG 协商：请求 MJPG，设备不支持则报错（第一版不做 YUYV→JPEG 转换） |
| D8 | 接入 `UavDeviceHandler`：`start()` 装配 + `UavInner` 加字段 + `shutdown()` 关闭 |
| D9 | `timeout_ms` 是 `capture` 等待结果的总超时（包在 `oneshot` await 上）；nokhwa 的 `frame()` 本身阻塞无超时（摄像头正常时无碍，已知可接受） |

## 三、线程模型

```
UavDeviceHandler::start()
   └─> CameraDevice::open()   （若 [camera].enabled = true）
          └─> std::thread::spawn 常驻线程
                 ├─ nokhwa Camera::new(path, MJPG, w×h)   ← 打开摄像头（常驻）
                 └─ loop: recv 命令
                       ├─ Capture{reply} → camera.frame() → JPEG → reply.send
                       └─ Shutdown     → 关摄像头 → 退出

CameraDevice::capture()（async，供 Lua / 上层调用）
   └─> 发 Capture 命令 + oneshot → await（timeout 包超时）→ 拿 JPEG
```

- 摄像头对象只在常驻线程内创建/使用，**不跨线程**，故非 Send/Sync 无碍（D2 依据）。
- `capture` 是 `async`（oneshot await），不阻塞调用方，未来可直接包装成 Lua async cap。

## 四、涉及文件

| 文件 | 改动 |
|---|---|
| `pleiades-uav/src/device/camera/mod.rs` | ✏️ 新增（`CameraDevice` + `CameraCmd` + 常驻线程） |
| `pleiades-uav/src/device/mod.rs` | ✏️ 新增（`pub mod camera;`） |
| `pleiades-uav/src/main.rs` | ✏️ 加 `mod device;` |
| `pleiades-uav/src/config.rs` | ✏️ 加 `CameraConfig` + `[camera]` 段 + `fill_camera`（幂等补默认） |
| `pleiades-uav/src/uav/robot_handler.rs` | ✏️ `UavInner` 加字段 + `start()` 装配 + `shutdown()` 关闭 |
| `pleiades-uav/Cargo.toml` | ✏️ 加 `nokhwa` 依赖 |

> 参考：`pleiades-ugv/src/device/lg290p/`（device 目录结构）、`pleiades-ugv/src/config.rs` 的 `Lg290pConfig` + `fill_lg290p`（config 模式）。

## 五、API 设计

```rust
// pleiades-uav/src/device/camera/mod.rs
pub struct CameraDevice {
    config: CameraConfig,
    cmd_tx: Option<std::sync::mpsc::Sender<CameraCmd>>,
    join: Option<std::thread::JoinHandle<()>>,
    cancel: CancellationToken,
}

enum CameraCmd {
    Capture { reply: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>> },
    Shutdown,
}

impl CameraDevice {
    /// 只存配置、不开设备（未 open 时 capture 返回 Err("摄像头未打开")）
    pub fn new(config: CameraConfig) -> Self;

    /// spawn 常驻线程 + 打开摄像头（幂等：已 open 则 no-op）
    pub fn open(&mut self) -> Result<(), String>;

    /// 拍一张：发 Capture + await oneshot（timeout_ms 包超时），返回 JPEG 字节
    pub async fn capture(&self) -> Result<Vec<u8>, String>;

    /// 发 Shutdown + join + 释放设备（幂等）
    pub fn close(&mut self);
}

impl Drop for CameraDevice {
    fn drop(&mut self) { self.close(); }   // 兜底：忘 close 自动关
}
```

常驻线程核心（示意）：

```rust
let handle = std::thread::spawn(move || {
    let mut camera = match nokhwa::Camera::new(
        nokhwa::utils::CameraIndex::String(path.clone()),
        RequestedFormat::with_formats(
            RequestedFormatType::Closest(CameraFormat::new(
                Resolution::new(width, height), FrameFormat::MJPEG, 30)),
            &[FrameFormat::MJPEG],
        ),
    ) {
        Ok(c) => c,
        Err(e) => { /* 通过错误通道回报 open 失败 */ return; }
    };
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            CameraCmd::Capture { reply } => {
                let r = camera.frame()
                    .map(|b| b.buffer().to_vec())      // MJPEG 帧 = JPEG 字节
                    .map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            CameraCmd::Shutdown => break,
        }
    }
    // 线程结束，摄像头随 camera 释放
});
```

`capture()` 实现（async）：

```rust
pub async fn capture(&self) -> Result<Vec<u8>, String> {
    let tx = self.cmd_tx.as_ref().ok_or("摄像头未打开")?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(CameraCmd::Capture { reply: reply_tx }).map_err(|_| "摄像头线程已退出")?;
    tokio::time::timeout(Duration::from_millis(self.config.timeout_ms), reply_rx)
        .await
        .map_err(|_| "拍照超时")?
        .map_err(|_| "摄像头线程已退出")?
}
```

## 六、config

`pleiades-uav/src/config.rs` 加 `CameraConfig`，`[camera]` 段默认：

```toml
[camera]
enabled = false        # 默认关（与 lg290p 一致）
path = "/dev/video0"   # 摄像头设备（也可填 /dev/v4l/by-id/... 持久化路径）
width = 1280
height = 720
timeout_ms = 3000      # capture 等待结果总超时
```

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct CameraConfig {
    pub enabled: Option<bool>,
    pub path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub timeout_ms: Option<u64>,
}
```

- `fill_camera(doc)` 幂等补默认值（参照 `fill_lg290p`）。
- `UavConfig` 加 `pub camera: Option<CameraConfig>`。

## 七、验证

- `cargo check -p pleiades-uav`（0 error）。
- 有摄像头的机器（笔记本/树莓派）：`open → capture → 写 /tmp/test.jpg → 打开确认出图`。
- 设备不支持 MJPG 时：`open`/`capture` 报错而非 panic（D7）。

## 八、后续扩展（记录，不实现）

| 扩展 | 方案 | 备注 |
|---|---|---|
| 地面站显示 | 图片包 ORION 帧（新 `MSGID_IMAGE`）→ gossipsub `TOPIC_ROBOT_IMAGE` → terminal `StreamRaw` 透传 → `robot_frame` 信号 → Godot 解码 | 与 task_24 加 `TOPIC_RTK_RTCM` 同模式；大图带宽需评估，或 `send_data` 单播给 terminal |
| 发图给小车推理 | Lua 调 `network.send_data(peer, DataType, jpeg)` | 「发」现成；小车侧「收图→YOLO→回传」无 handler，且踩 ML_Engine/分布式推理边界，需单独评估 |
| Lua caps | `register_camera_caps`：`camera.capture_image()`（`create_async_function` + 薄胶水） | 符合 instructions Lua 绑定规范 |
| YOLO 推理 | 本地 `yolov8n.pt`（树莓派 0.1 TOPS，需跳帧+低分辨率） | 后续 task |

## 九、讨论记录

- 2026-09-03 与李永康讨论定稿：摄像头做成子设备（D1）；选型从 v4l 改为 **nokhwa**——因确定用 std::thread 常驻线程后，`Camera` 非 Sync 短板消除、`frame()` 一行取帧更省（D2）；显式 `new/open/capture/close` 生命周期（D3/D4）；fourcc 固定 MJPG 不暴露 config（D5）；config 精简为 `enabled/path/width/height/timeout_ms`（D6）；第一版只做驱动，发图/显示/YOLO/Lua caps 后续扩展（D8/D9）。
