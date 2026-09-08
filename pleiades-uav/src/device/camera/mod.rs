//Presented by KeJi
//Created Date ： 2026-09-08
//Modified Date ： 2026-09-08

//! 无人机 USB 摄像头子设备（Task 28 / Task 29 阶段 1）
//!
//! 职责：抓一帧（NV12）→ 解码 RGB → 编码 JPEG，通过 `DeviceCapability` 把 `camera.capture()` 注册进 Lua。
//! 调用方是 base 的 Lua 脚本（`exec program`），不是 uav 的 Rust 决策链。
//!
//! 线程模型（Task 28 D4）：`open()` 起一个 `std::thread` 常驻线程，nokhwa 摄像头对象只在
//! 该线程内创建/使用（不跨线程）；`capture()` 用 `oneshot` 回传 JPEG 字节。

use std::io::Cursor;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};

use pleiades_base::orchestrator::DeviceCapability;

use crate::config::CameraConfig;

/// 常驻线程命令
enum CameraCmd {
    Capture { reply: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>> },
    Shutdown,
}

/// 内部共享状态（`Arc<CameraInner>` 使 `register_lua_caps` 闭包能跨线程捕获）
struct CameraInner {
    config: CameraConfig,
    cmd_tx: Mutex<Option<mpsc::Sender<CameraCmd>>>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl CameraInner {
    /// 发 Capture 命令 + 等待 JPEG 字节（核心业务逻辑，独立于 Lua 胶水）
    async fn capture_impl(&self) -> Result<Vec<u8>, String> {
        let tx = {
            let guard = self.cmd_tx.lock().unwrap();
            guard
                .as_ref()
                .ok_or_else(|| "摄像头未打开".to_string())?
                .clone()
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(CameraCmd::Capture { reply: reply_tx })
            .map_err(|_| "摄像头线程已退出".to_string())?;
        let timeout_ms = self.config.timeout_ms.unwrap_or(3000);
        tokio::time::timeout(Duration::from_millis(timeout_ms), reply_rx)
            .await
            .map_err(|_| "拍照超时".to_string())?
            .map_err(|_| "摄像头线程已退出".to_string())?
    }
}

/// 摄像头子设备（nokhwa 后端，NV12 抓帧 → RGB → JPEG）
pub struct CameraDevice {
    inner: Arc<CameraInner>,
}

impl CameraDevice {
    /// 只存配置，不开设备（未 `open` 时 `capture` 返回 Err）
    pub fn new(config: CameraConfig) -> Self {
        Self {
            inner: Arc::new(CameraInner {
                config,
                cmd_tx: Mutex::new(None),
                join: Mutex::new(None),
            }),
        }
    }

    /// 起常驻线程 + 打开摄像头（幂等：已 open 则 no-op）
    pub fn open(&self) -> Result<(), String> {
        if self.inner.cmd_tx.lock().unwrap().is_some() {
            return Ok(());
        }

        let (cmd_tx, cmd_rx) = mpsc::channel::<CameraCmd>();
        let (open_tx, open_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let width = self.inner.config.width.unwrap_or(1280);
        let height = self.inner.config.height.unwrap_or(720);
        let index = parse_index(self.inner.config.path.as_deref().unwrap_or("0"));
        let timeout_ms = self.inner.config.timeout_ms.unwrap_or(3000);

        let handle = std::thread::spawn(move || {
            // 请求 NV12（Task 29：调试机摄像头只出 NV12，默认 NV12 不做 fallback）
            let req = RequestedFormat::with_formats(
                RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(width, height),
                    FrameFormat::NV12,
                    30,
                )),
                &[FrameFormat::NV12],
            );
            let mut camera = match nokhwa::Camera::new(index, req) {
                Ok(c) => c,
                Err(e) => {
                    let _ = open_tx.send(Err(format!("打开摄像头失败: {e}")));
                    return;
                }
            };
            let _ = open_tx.send(Ok(()));
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    CameraCmd::Capture { reply } => {
                        let _ = reply.send(capture_jpeg(&mut camera));
                    }
                    CameraCmd::Shutdown => break,
                }
            }
        });

        match open_rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(Ok(())) => {
                *self.inner.cmd_tx.lock().unwrap() = Some(cmd_tx);
                *self.inner.join.lock().unwrap() = Some(handle);
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err("打开摄像头超时".to_string())
            }
        }
    }

    /// 拍一张：发 Capture + await oneshot，返回 JPEG 字节。
    /// 预留 Rust 侧 API（task_28 D3）；Lua 侧走 `register_lua_caps` → `capture_impl`。
    #[allow(dead_code)]
    pub async fn capture(&self) -> Result<Vec<u8>, String> {
        self.inner.capture_impl().await
    }

    /// 发 Shutdown + join + 释放设备（幂等）
    pub fn close(&self) {
        if let Some(tx) = self.inner.cmd_tx.lock().unwrap().take() {
            let _ = tx.send(CameraCmd::Shutdown);
        }
        if let Some(handle) = self.inner.join.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for CameraDevice {
    fn drop(&mut self) {
        self.close();
    }
}

/// 设备能力注册（uav 侧，实现 base 的 `DeviceCapability` trait）。
/// 闭包只做薄胶水：`inner.capture_impl()` + `lua.create_string(jpeg)`。
impl DeviceCapability for CameraDevice {
    fn register_lua_caps(&self, lua: &mlua::Lua) -> mlua::Result<()> {
        let inner = self.inner.clone();
        let table = lua.create_table()?;
        table.set(
            "capture",
            lua.create_async_function(move |lua, (): ()| {
                let inner = inner.clone();
                async move {
                    let jpeg = inner.capture_impl().await.map_err(mlua::Error::runtime)?;
                    lua.create_string(jpeg)
                }
            })?,
        )?;
        lua.globals().set("camera", table)?;
        Ok(())
    }
}

/// 一帧 NV12 → RGB → JPEG（跑在常驻线程内）
fn capture_jpeg(camera: &mut nokhwa::Camera) -> Result<Vec<u8>, String> {
    let frame = camera.frame().map_err(|e| format!("取帧失败: {e}"))?;
    let rgb = frame
        .decode_image::<RgbFormat>()
        .map_err(|e| format!("NV12→RGB 解码失败: {e}"))?;
    let mut jpeg = Vec::new();
    rgb.write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
        .map_err(|e| format!("JPEG 编码失败: {e}"))?;
    Ok(jpeg)
}

/// 设备路径解析：纯数字 = 索引（Windows 常用 "0"），否则 = 路径（Linux "/dev/video0"）
fn parse_index(path: &str) -> CameraIndex {
    if let Ok(n) = path.parse::<u32>() {
        CameraIndex::Index(n)
    } else {
        CameraIndex::String(path.to_string())
    }
}
