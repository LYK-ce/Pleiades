//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-24

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! 开发机（电脑）USB 摄像头 sensor（Task 34 Step 1：由 2026-09-08 的 `device/camera/` 整体迁入，逻辑未改）
//!
//! 职责：抓一帧（NV12）→ 解码 RGB → 编码 JPEG，通过 `DeviceCapability` 把
//! `camera_pc.capture()` / `camera_pc.info()` 注册进 Lua。调用方是 base 的 Lua 脚本
//! （`exec program`），不是 uav 的 Rust 决策链。
//!
//! 线程模型（Task 28 D4，迁移前后**完全不变**）：`open()` 起一个 `std::thread` 常驻线程，
//! nokhwa 摄像头对象只在该线程内创建/使用（不跨线程）；`capture()` 用 `oneshot` 回传
//! 一帧的完整结果 [`Pc_Frame`]（JPEG 字节 + 宽高 + 格式 + 打戳）。
//!
//! 与另一个相机 sensor（机载相机）的关系：**没有关系**。两个 sensor 各自一份实现、
//! 各自一条打开流程、各自一个 Lua 表名，重复代码是刻意接受的代价（Task 34 硬要点 1）。
//! 因此本文件里不出现对方的模块名（验收 A5 要求两侧互不引用，连注释也不提）。
//!
//! Task 34 Step 1 相对迁移前的差异（逐项可对照 `tmp/task34/plan.md` §2.4）：
//! 1) 模块/类型/表名改为 `camera_pc` / `Pc_Camera`；
//! 2) `path` 解析从「非法路径原样交给 nokhwa（由它内部报错）」改为「`open()` 直接 `Err` + 提示」
//!    （nokhwa 0.10 的 V4L2 后端只接受 `CameraIndex::Index(u32)`，不接受字符串路径）；
//! 3) 设备打开成功后补一次显式 `open_stream()`（V4L2 后端不会自动开流，
//!    不补则 `frame()` 恒返回 `Stream Not Started`）。
//!
//! Task 34 Step 2 增补：Lua 表多一个**同步** `info()`，返回
//! `{width, height, format, age_ms}`；数据来源三级回退
//! 「最近一次成功抓帧 → 打开时协商到的格式 → 配置缺省」。

use std::io::Cursor;
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};

use pleiades_base::orchestrator::DeviceCapability;

use crate::config::CameraPcConfig;

/// 默认宽度（配置缺省时）
const DEFAULT_WIDTH: u32 = 1280;
/// 默认高度（配置缺省时）
const DEFAULT_HEIGHT: u32 = 720;
/// 默认打开/抓帧超时（毫秒，配置缺省时）
const DEFAULT_TIMEOUT_MS: u64 = 3000;
/// 从未打开、也无任何可参考格式时 `info().format` 的取值（三处 sensor 逐字统一）
const FORMAT_NONE: &str = "none";

/// 一次抓帧的完整结果（worker → 主线程）：`jpeg` 给 Lua，其余给 `info()`。
struct Pc_Frame {
    /// JPEG 字节（`capture()` 的返回值）
    jpeg: Vec<u8>,
    /// 本帧宽（解码后 RGB 的宽）
    width: u32,
    /// 本帧高（解码后 RGB 的高）
    height: u32,
    /// 本帧源像素格式（`Buffer::source_frame_format`，用于 `info().format`）
    format: FrameFormat,
    /// 帧被取到的时刻（worker 内 `camera.frame()` 返回后立刻打戳；`Instant` 单调）
    at: Instant,
}

/// 最近一次成功抓帧的元数据（`info()` 的首选数据源；`at` 只用于算 `age_ms`）。
///
/// `Pc_Frame` 里的 `jpeg` 会把整帧字节留在共享状态里，没必要；这里只留纯数据。
#[derive(Clone, Copy)]
struct Last_Frame {
    width: u32,
    height: u32,
    format: FrameFormat,
    /// 帧被取到的时刻（与 `Pc_Frame::at` 同一个戳）
    at: Instant,
}

/// 打开时协商到的格式（`open()` 成功后写入；第一帧之前 `info()` 用它）。
#[derive(Clone)]
struct Pc_Open_Info {
    width: u32,
    height: u32,
    format: FrameFormat,
}

/// `info()` 的返回值（字段名与另一侧 sensor / 仿真侧逐字一致；**不要加第五个字段**）。
struct Sensor_Info {
    width: u32,
    height: u32,
    format: String,
    age_ms: i64,
}

/// 常驻线程命令
enum Pc_Cmd {
    Capture {
        reply: tokio::sync::oneshot::Sender<Result<Pc_Frame, String>>,
    },
    Shutdown,
}

/// 内部共享状态（`Arc<Pc_Inner>` 使 `register_lua_caps` 闭包能跨线程捕获）
struct Pc_Inner {
    config: CameraPcConfig,
    cmd_tx: Mutex<Option<mpsc::Sender<Pc_Cmd>>>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 打开时协商到的格式（`open()` 成功才写；`Close`/`Drop` 不清空）
    opened_info: Mutex<Option<Pc_Open_Info>>,
    /// 最近一次**成功**抓帧的元数据（抓帧失败不写，`age_ms` 因此不会"变新鲜"）
    last_frame: Mutex<Option<Last_Frame>>,
}

impl Pc_Inner {
    /// 发 Capture 命令 + 等待一帧（核心业务逻辑，独立于 Lua 胶水）
    async fn capture_impl(&self) -> Result<Vec<u8>, String> {
        let tx = {
            let guard = self.cmd_tx.lock().unwrap();
            guard
                .as_ref()
                .ok_or_else(|| "摄像头未打开".to_string())?
                .clone()
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(Pc_Cmd::Capture { reply: reply_tx })
            .map_err(|_| "摄像头线程已退出".to_string())?;
        let timeout_ms = self.config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let frame = tokio::time::timeout(Duration::from_millis(timeout_ms), reply_rx)
            .await
            .map_err(|_| "拍照超时".to_string())?
            .map_err(|_| "摄像头线程已退出".to_string())??;
        // 先记元数据（此刻才真正"取到帧"），再移动出 JPEG 字节。
        self.Store_Last(&frame);
        Ok(frame.jpeg)
    }

    /// 记录最近一次成功抓帧（毒化安全：走 [`Lock`]，不加锁 panic 面）
    fn Store_Last(&self, frame: &Pc_Frame) {
        *Lock(&self.last_frame) = Some(Last_Frame {
            width: frame.width,
            height: frame.height,
            format: frame.format,
            at: frame.at,
        });
    }

    /// `info()` 的业务逻辑：三级回退，**从不失败、从不返回 nil**。
    ///
    /// 1) `last_frame`：最近一次**成功**抓帧的宽高/格式 + 帧龄；
    /// 2) `opened_info`：打开时协商到的格式（首帧之前 `age_ms = 0`）；
    /// 3) 配置缺省：`[camera_pc].width/height`（未配置则 1280x720），`format = "none"`。
    ///
    /// `age_ms` 语义（三处 sensor 逐字一致，脚本依赖）：
    /// - 单位毫秒，来自 `Instant::elapsed()`，**单调不减**（系统时钟回拨不影响）；
    /// - 表示「距最近一次成功抓帧过了多久」；抓帧失败**不会**刷新它（失败不写 `last_frame`）；
    /// - **从未抓到帧时取 `0`**（不是 -1、也不是 u64::MAX）：四字段契约里没有「是否已有帧」位，
    ///   `0` 配合 `format = "none"` 表示「没有帧可谈」，且不会因 `u64 → i64` 溢出在 Lua 侧报错。
    fn Info_Impl(&self) -> Sensor_Info {
        // 先拷出来（`Last_Frame: Copy`），守卫立刻释放：一次只持一把锁。
        let last = *Lock(&self.last_frame);
        if let Some(last) = last {
            return Sensor_Info {
                width: last.width,
                height: last.height,
                format: Format_Token(last.format).to_string(),
                age_ms: Age_Ms(last.at),
            };
        }

        let opened = Lock(&self.opened_info).clone();
        if let Some(opened) = opened {
            return Sensor_Info {
                width: opened.width,
                height: opened.height,
                format: Format_Token(opened.format).to_string(),
                age_ms: 0,
            };
        }

        Sensor_Info {
            width: self.config.width.unwrap_or(DEFAULT_WIDTH),
            height: self.config.height.unwrap_or(DEFAULT_HEIGHT),
            format: FORMAT_NONE.to_string(),
            age_ms: 0,
        }
    }
}

/// 开发机摄像头 sensor（nokhwa 后端，NV12 抓帧 → RGB → JPEG）
pub struct Pc_Camera {
    inner: Arc<Pc_Inner>,
}

impl Pc_Camera {
    /// 只存配置，不开设备（未 `open` 时 `capture` 返回 Err）
    pub fn new(config: CameraPcConfig) -> Self {
        Self {
            inner: Arc::new(Pc_Inner {
                config,
                cmd_tx: Mutex::new(None),
                join: Mutex::new(None),
                opened_info: Mutex::new(None),
                last_frame: Mutex::new(None),
            }),
        }
    }

    /// `path` → 设备索引（纯解析，不开设备）。
    ///
    /// 供 bootstrap 做「同一 `/dev/videoN` 只允许一个 sensor」的守卫。
    pub fn Device_Index(&self) -> Result<u32, String> {
        Parse_Camera_Index(self.inner.config.path.as_deref().unwrap_or("0"))
    }

    /// 一行人类可读描述（bootstrap 成功日志用）
    pub fn Describe(&self) -> String {
        format!(
            "NV12 请求 {}x{}（未协商实际格式），timeout {}ms",
            self.inner.config.width.unwrap_or(DEFAULT_WIDTH),
            self.inner.config.height.unwrap_or(DEFAULT_HEIGHT),
            self.inner.config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
        )
    }

    /// 起常驻线程 + 打开摄像头（幂等：已 open 则 no-op）
    pub fn open(&self) -> Result<(), String> {
        if self.inner.cmd_tx.lock().unwrap().is_some() {
            return Ok(());
        }

        let (cmd_tx, cmd_rx) = mpsc::channel::<Pc_Cmd>();
        let (open_tx, open_rx) = mpsc::sync_channel::<Result<Pc_Open_Info, String>>(1);
        let width = self.inner.config.width.unwrap_or(DEFAULT_WIDTH);
        let height = self.inner.config.height.unwrap_or(DEFAULT_HEIGHT);
        let index_number = self.Device_Index()?;
        let index = CameraIndex::Index(index_number);
        let timeout_ms = self.inner.config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);

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
            // Task 34 Step 1 唯一补充：V4L2 后端不会自动开流（nokhwa-bindings-linux 的
            // `frame()` 在 `stream_handle.is_none()` 时恒返回 "Stream Not Started"），
            // 必须显式开流，否则迁移前的抓帧在 Linux 上必然失败。
            if let Err(e) = camera.open_stream() {
                let _ = open_tx.send(Err(format!(
                    "启动视频流失败（NV12 {}x{}）: {e}\n  排查建议：确认 /dev/video{} 未被其他进程占用（sudo fuser -v /dev/video{}）、换 USB 口、dmesg | tail -30 看 uvcvideo 报错",
                    width,
                    height,
                    index_number,
                    index_number
                )));
                return;
            }
            // Step 2：把协商到的实际格式回传主线程（`info()` 在首帧之前用它）。
            // RequestedFormatType::Closest 可能给出非 NV12 的组合，这里记的就是设备真实生效的那份。
            let negotiated = camera.camera_format();
            let _ = open_tx.send(Ok(Pc_Open_Info {
                width: negotiated.width(),
                height: negotiated.height(),
                format: negotiated.format(),
            }));
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    Pc_Cmd::Capture { reply } => {
                        let _ = reply.send(Capture_Frame(&mut camera));
                    }
                    Pc_Cmd::Shutdown => break,
                }
            }
        });

        match open_rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(Ok(open_info)) => {
                *Lock(&self.inner.opened_info) = Some(open_info);
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
            let _ = tx.send(Pc_Cmd::Shutdown);
        }
        if let Some(handle) = self.inner.join.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Pc_Camera {
    fn drop(&mut self) {
        self.close();
    }
}

/// 设备能力注册（uav 侧，实现 base 的 `DeviceCapability` trait）。
///
/// Lua 表 `camera_pc` 有两个成员：
/// - `capture()`：异步，返回 JPEG 二进制字符串；
/// - `info()`：**同步**，返回 `{width, height, format, age_ms}`（永不抛错、永不返回 nil）。
///
/// 闭包只做薄胶水：`capture_impl()` + `lua.create_string(jpeg)`；`Info_Impl()` + `Info_Table()`。
impl DeviceCapability for Pc_Camera {
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

        let info_inner = self.inner.clone();
        table.set(
            "info",
            lua.create_function(move |lua, (): ()| {
                let info = info_inner.Info_Impl();
                Info_Table(lua, &info)
            })?,
        )?;

        lua.globals().set("camera_pc", table)?;
        Ok(())
    }
}

/// 一帧 NV12 → RGB → JPEG，并带上 `info()` 需要的宽高/格式/打戳（跑在常驻线程内）。
///
/// `at` 在 `camera.frame()` 返回后**立刻**打戳（不是编码完才打），否则 `age_ms`
/// 会把 JPEG 编码耗时也算进去。
fn Capture_Frame(camera: &mut nokhwa::Camera) -> Result<Pc_Frame, String> {
    let frame = camera.frame().map_err(|e| format!("取帧失败: {e}"))?;
    let at = Instant::now();
    let format = frame.source_frame_format();
    let rgb = frame
        .decode_image::<RgbFormat>()
        .map_err(|e| format!("NV12→RGB 解码失败: {e}"))?;
    let mut jpeg = Vec::new();
    rgb.write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
        .map_err(|e| format!("JPEG 编码失败: {e}"))?;
    Ok(Pc_Frame {
        jpeg,
        width: rgb.width(),
        height: rgb.height(),
        format,
        at,
    })
}

/// 帧龄毫秒：`Instant::elapsed()` **单调不减**，饱和到 `i64` 上限，不 panic、不回绕、不为负。
fn Age_Ms(at: Instant) -> i64 {
    Duration_Ms(at.elapsed())
}

/// `Duration` → 毫秒 `i64`（超过 `i64::MAX` 时饱和，绝不回绕成负数）。
fn Duration_Ms(elapsed: Duration) -> i64 {
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

/// `FrameFormat` → 简短格式串（与配置里 `prefer = "mjpg"`、日志用词一致的那套写法）。
///
/// MJPEG 写 `"MJPG"`（不是 Display 的 `"MJPEG"`），RAWRGB/RAWBGR 写 `"RGB"`/`"BGR"`；
/// YUYV / NV12 / GRAY 原样。Lua 侧只认这六个值 + `"none"`。
fn Format_Token(format: FrameFormat) -> &'static str {
    match format {
        FrameFormat::MJPEG => "MJPG",
        FrameFormat::YUYV => "YUYV",
        FrameFormat::NV12 => "NV12",
        FrameFormat::GRAY => "GRAY",
        FrameFormat::RAWRGB => "RGB",
        FrameFormat::RAWBGR => "BGR",
    }
}

/// `Sensor_Info` → Lua table：**只做类型转换**（`as i64` + `set`），字段名逐字固定。
///
/// 数值一律转 `i64`：Lua 5.4 的 `mlua` 对 `u64` 超出 `i64::MAX` 会报错，
/// 而 `age_ms` 的量级足以触发这个问题。
fn Info_Table(lua: &mlua::Lua, info: &Sensor_Info) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    table.set("width", info.width as i64)?;
    table.set("height", info.height as i64)?;
    table.set("format", info.format.clone())?;
    table.set("age_ms", info.age_ms)?;
    Ok(table)
}

/// 取锁，毒化（某线程 panic 过）时恢复内部数据而不是二次 panic（只用于 Step 2 新增状态）。
fn Lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 设备路径解析：`"0"` / `" 3 "` → `Ok(0)` / `Ok(3)`；`"/dev/video12"` → `Ok(12)`。
///
/// 其余一律 `Err`：nokhwa 0.10 的 V4L2 后端只接受 `CameraIndex::Index(u32)`，
/// 传字符串路径会在内部 `parse::<u32>()` 失败（Task 34 硬要点 3），因此这里提前拦下并给提示。
pub fn Parse_Camera_Index(path: &str) -> Result<u32, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(Path_Hint(path, "路径为空"));
    }
    let digits = match trimmed.strip_prefix("/dev/video") {
        Some(rest) => rest,
        None => trimmed,
    };
    if digits.is_empty() {
        return Err(Path_Hint(path, "缺少设备编号"));
    }
    match digits.parse::<u32>() {
        Ok(number) => Ok(number),
        Err(error) => Err(Path_Hint(path, &format!("无法解析为设备编号：{error}"))),
    }
}

/// `path` 解析失败的统一文案（列出合法写法 + 为什么不能写字符串路径）
fn Path_Hint(path: &str, reason: &str) -> String {
    format!(
        "camera_pc.path=\"{path}\" 非法（{reason}）\n  \
         只接受两种写法：纯数字索引（如 \"0\" / \"1\"）或 Linux 设备节点（如 \"/dev/video0\"）\n  \
         原因：nokhwa 0.10 的 V4L2 后端只接受 CameraIndex::Index(u32)，字符串路径会在内部解析失败\n  \
         复核命令：ls -l /dev/video* 与 v4l2-ctl --list-devices"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个只关心宽高的配置（其余字段空着，等于走缺省）
    fn Pc_Config_For_Test(width: Option<u32>, height: Option<u32>) -> CameraPcConfig {
        CameraPcConfig {
            enabled: Some(true),
            path: Some("0".to_string()),
            width,
            height,
            timeout_ms: None,
        }
    }

    #[test]
    fn parse_camera_index_numeric() {
        assert_eq!(Parse_Camera_Index("0"), Ok(0));
        assert_eq!(Parse_Camera_Index("3"), Ok(3));
        assert_eq!(Parse_Camera_Index("  7  "), Ok(7));
        assert_eq!(Parse_Camera_Index("12"), Ok(12));
    }

    #[test]
    fn parse_camera_index_dev_video() {
        assert_eq!(Parse_Camera_Index("/dev/video0"), Ok(0));
        assert_eq!(Parse_Camera_Index("/dev/video12"), Ok(12));
        assert_eq!(Parse_Camera_Index("  /dev/video2  "), Ok(2));
    }

    #[test]
    fn parse_camera_index_rejects_string_paths() {
        for bad in [
            "",
            "   ",
            "/dev/video",
            "/dev/videoX",
            "/dev/video0x",
            "abc",
            "4k",
            "-1",
            "/dev/video 0",
        ] {
            let error = Parse_Camera_Index(bad).expect_err(&format!("应拒绝 {bad:?}"));
            assert!(
                error.contains("camera_pc.path"),
                "错误文案应点名配置键：{error}"
            );
            assert!(
                error.contains("CameraIndex::Index"),
                "错误文案应说明 nokhwa 只接受索引：{error}"
            );
        }
        // 溢出 u32 的数字同样要拒绝，不能静默截断
        assert!(Parse_Camera_Index("4294967296").is_err());
    }

    #[test]
    fn describe_reports_nv12_request() {
        let camera = Pc_Camera::new(CameraPcConfig {
            enabled: Some(true),
            path: Some("/dev/video1".to_string()),
            width: Some(640),
            height: Some(480),
            timeout_ms: Some(1500),
        });
        assert_eq!(camera.Device_Index(), Ok(1));
        let text = camera.Describe();
        assert!(text.contains("NV12"), "{text}");
        assert!(text.contains("640x480"), "{text}");
        assert!(text.contains("1500ms"), "{text}");
    }

    #[test]
    fn info_before_open_uses_config_defaults() {
        // 未 open：既没有帧、也没有协商格式 → 配置缺省 + format="none" + age_ms=0
        let camera = Pc_Camera::new(Pc_Config_For_Test(Some(800), Some(600)));
        let info = camera.inner.Info_Impl();
        assert_eq!((info.width, info.height), (800, 600), "应回落到配置宽高");
        assert_eq!(info.format, FORMAT_NONE, "没打开过就必须是 none");
        assert_eq!(info.age_ms, 0, "没有帧时 age_ms 取 0");

        // 配置也缺省 → 与 Describe() 一致的 1280x720
        let default = Pc_Camera::new(Pc_Config_For_Test(None, None));
        let info = default.inner.Info_Impl();
        assert_eq!((info.width, info.height), (DEFAULT_WIDTH, DEFAULT_HEIGHT));
        assert_eq!(info.format, FORMAT_NONE);
        assert_eq!(info.age_ms, 0);
    }

    #[test]
    fn info_uses_negotiated_format_before_first_frame() {
        // 打开成功但还没抓到帧：用开放时协商到的格式，帧龄 0
        let camera = Pc_Camera::new(Pc_Config_For_Test(Some(640), Some(480)));
        *Lock(&camera.inner.opened_info) = Some(Pc_Open_Info {
            width: 1280,
            height: 720,
            format: FrameFormat::MJPEG,
        });
        let info = camera.inner.Info_Impl();
        assert_eq!((info.width, info.height), (1280, 720), "应优先用协商宽高");
        assert_eq!(info.format, "MJPG", "MJPEG 的对外写法是 MJPG");
        assert_eq!(info.age_ms, 0, "首帧之前没有帧龄可谈");
    }

    #[test]
    fn info_after_frame_uses_frame_metadata() {
        // 有帧：帧元数据压过配置与协商值，帧龄来自打戳
        let camera = Pc_Camera::new(Pc_Config_For_Test(Some(640), Some(480)));
        *Lock(&camera.inner.opened_info) = Some(Pc_Open_Info {
            width: 640,
            height: 480,
            format: FrameFormat::MJPEG,
        });
        camera.inner.Store_Last(&Pc_Frame {
            jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9],
            width: 1280,
            height: 720,
            format: FrameFormat::NV12,
            at: Instant::now() - Duration::from_millis(25),
        });
        let info = camera.inner.Info_Impl();
        assert_eq!((info.width, info.height), (1280, 720), "应取最近帧的宽高");
        assert_eq!(info.format, "NV12", "应取最近帧的格式");
        assert!(info.age_ms >= 25, "帧龄至少 25ms（单调测量）：{}", info.age_ms);
    }

    #[test]
    fn age_ms_is_monotonic_and_saturating() {
        assert_eq!(Duration_Ms(Duration::from_millis(0)), 0);
        assert_eq!(Duration_Ms(Duration::from_millis(7)), 7);
        assert_eq!(
            Duration_Ms(Duration::from_secs(u64::MAX)),
            i64::MAX,
            "超过 i64 上限必须饱和，绝不回绕成负数"
        );

        let past = Instant::now() - Duration::from_millis(40);
        let first = Age_Ms(past);
        let second = Age_Ms(past);
        assert!(first >= 40, "同一起点测得至少 40ms：{first}");
        assert!(second >= first, "同一起点两次测量必须单调不减");
        assert!(Age_Ms(Instant::now()) >= 0, "刚打的戳不该是负数");
    }

    #[test]
    fn lua_info_table_matches_contract() {
        let lua = mlua::Lua::new();
        let info = Sensor_Info {
            width: 1280,
            height: 720,
            format: "NV12".to_string(),
            age_ms: 42,
        };
        let table = Info_Table(&lua, &info).expect("info 表应能构造");
        lua.globals().set("info", table).expect("info 应能挂到全局");

        // 字段数必须恰好 4（多一个少一个都算破坏契约）
        let count: i64 = lua
            .load("local n = 0 for _ in pairs(info) do n = n + 1 end return n")
            .eval()
            .expect("统计字段数应成功");
        assert_eq!(count, 4, "契约恰好四个字段");

        let (width, height): (i64, i64) = lua
            .load("return info.width, info.height")
            .eval()
            .expect("宽高应是整数");
        let format: String = lua
            .load("return info.format")
            .eval()
            .expect("format 应是字符串");
        let age_ms: i64 = lua
            .load("return info.age_ms")
            .eval()
            .expect("age_ms 应是整数");
        assert_eq!((width, height), (1280, 720));
        assert_eq!(format, "NV12");
        assert_eq!(age_ms, 42);

        // 数值字段必须是 Lua integer（用 u64 会溢出 / 变成 float，脚本拼字符串就能看出差别）
        let (width_kind, height_kind, age_kind): (String, String, String) = lua
            .load("return math.type(info.width), math.type(info.height), math.type(info.age_ms)")
            .eval()
            .expect("math.type 应可调用");
        assert_eq!(width_kind, "integer", "width 必须是 Lua integer");
        assert_eq!(height_kind, "integer", "height 必须是 Lua integer");
        assert_eq!(age_kind, "integer", "age_ms 必须是 Lua integer");
    }

    #[test]
    fn camera_photo_script_parses() {
        // 本机没有 lua / luac 二进制（step.md 验收允许），用仓库自带的 mlua 编译脚本兜底：
        // 与 base `ProgramRegistry::parse_script` 的加载方式一致（load + eval::<()>）。
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../programs/user/camera_photo.lua");
        let script = std::fs::read_to_string(path).expect("camera_photo.lua 应存在且可读");
        let lua = mlua::Lua::new();
        lua.load(&script)
            .eval::<()>()
            .expect("camera_photo.lua 必须是合法的 Lua 5.4");

        let globals = lua.globals();
        let command: String = globals.get("COMMAND").expect("COMMAND 应存在");
        assert_eq!(command, "camera_photo");
        let description: String = globals.get("DESCRIPTION").expect("DESCRIPTION 应存在");
        assert!(!description.is_empty(), "DESCRIPTION 不应为空");
        let execute_kind: String = lua
            .load("return type(execute)")
            .eval()
            .expect("type(execute) 应可调用");
        assert_eq!(execute_kind, "function", "execute 必须是函数");
    }

    /// 桩 `caps`：只实现脚本用到的两个成员，并把结果记在全局表里供断言。
    const STUB_CAPS: &str = r#"
        PRINTS = {}
        WROTE = { size = 0, released = false, path = "" }
        caps = {
            print = function(text) PRINTS[#PRINTS + 1] = tostring(text) end,
            storage_acquire_write = function(name)
                return {
                    write = function(self, bytes) WROTE.size = #bytes end,
                    path = function(self) return "/tmp/orion-stub/" .. name end,
                    release = function(self) WROTE.released = true end,
                }
            end,
        }
    "#;

    /// 一个只含脚本 `COMMAND`/`execute` 定义（不执行）+ 桩 caps 的干净 Lua 状态
    fn Lua_With_Script_And_Stub_Caps() -> mlua::Lua {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../programs/user/camera_photo.lua");
        let script = std::fs::read_to_string(path).expect("camera_photo.lua 应存在且可读");
        let lua = mlua::Lua::new();
        lua.load(&script)
            .eval::<()>()
            .expect("camera_photo.lua 必须是合法的 Lua 5.4");
        lua.load(STUB_CAPS).eval::<()>().expect("桩 caps 应能建立");
        lua
    }

    #[test]
    fn photo_script_runtime_with_stub_caps() {
        // 场景 1：只有 camera_pc 存在，capture() 返回 10 字节 → "ok" + 落盘 + 已 release
        let lua = Lua_With_Script_And_Stub_Caps();
        lua.load(
            r#"
            camera_pc = {
                info = function()
                    return { width = 640, height = 480, format = "NV12", age_ms = 0 }
                end,
                capture = function() return string.rep("J", 10) end,
            }
            "#,
        )
        .eval::<()>()
        .expect("桩 sensor 应能建立");
        let (result, size, released, prints): (String, i64, bool, i64) = lua
            .load(
                r#"
                local result = execute({})
                return result, WROTE.size, WROTE.released, #PRINTS
                "#,
            )
            .eval()
            .expect("execute 应能跑完");
        assert_eq!(result, "ok");
        assert_eq!(size, 10, "写盘字节数应与 capture() 返回一致");
        assert!(released, "handle:release() 必须被调用");
        assert!(prints >= 3, "至少打印 sensor/info/字节数 三行：{prints}");

        // 场景 2：三个表都不存在 → 明确报错 + 返回 "no-camera"（不是抛错）
        let lua = Lua_With_Script_And_Stub_Caps();
        let result: String = lua.load("return execute({})").eval().expect("execute 应能跑完");
        assert_eq!(result, "no-camera");

        // 场景 3：仿真 sensor 与开发机 sensor 同时存在 → 按内置顺序取前者；
        // 它 capture() 返回 nil 时必须归一化成 "no-jpeg"，不能因为 #nil 把脚本炸掉。
        let lua = Lua_With_Script_And_Stub_Caps();
        lua.load(
            r#"
            camera_sim = {
                info = function()
                    return { width = 1280, height = 720, format = "MJPG", age_ms = 33 }
                end,
                capture = function() return nil end,
            }
            camera_pc = {
                info = function()
                    return { width = 640, height = 480, format = "NV12", age_ms = 0 }
                end,
                capture = function() return string.rep("J", 10) end,
            }
            "#,
        )
        .eval::<()>()
        .expect("桩 sensor 应能建立");
        let result: String = lua.load("return execute({})").eval().expect("execute 应能跑完");
        assert_eq!(result, "no-jpeg", "nil 帧必须归一化成可读错误，不能 #nil");
        // 探测顺序：前面那个 sensor 优先，且日志里点明了 sensor 名
        let joined: String = lua
            .load(r"return table.concat(PRINTS, '\n')")
            .eval()
            .expect("PRINTS 应能拼接");
        assert!(joined.contains("sensor=camera_sim"), "{joined}");
        assert!(
            joined.contains("info: width=1280 height=720 format=MJPG age_ms=33"),
            "{joined}"
        );
        assert!(
            !joined.contains("sensor=camera_pc"),
            "探测顺序错了，竟然选了后一个 sensor：{joined}"
        );

        // 场景 4：capture() 返回 0 字节 → "empty-jpeg"
        let lua = Lua_With_Script_And_Stub_Caps();
        lua.load(
            r#"
            camera_pc = {
                info = function() return { width = 4, height = 4, format = "none", age_ms = 0 } end,
                capture = function() return "" end,
            }
            "#,
        )
        .eval::<()>()
        .expect("桩 sensor 应能建立");
        let result: String = lua.load("return execute({})").eval().expect("execute 应能跑完");
        assert_eq!(result, "empty-jpeg");
    }
}
