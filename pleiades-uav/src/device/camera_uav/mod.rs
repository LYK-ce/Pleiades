//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-24

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! UAV 机载相机 sensor（SY011HD / UVC / V4L2；Task 34 Step 1）
//!
//! 采集路径照搬 `rust_uav_camera_test/src/camera.rs`（已在树莓派实测）：
//! 枚举 → 三级选格式 → `Exact` 设格式（失败先 `drop` 再 `Closest` 重开）→ **显式 `open_stream()`**
//! → 抓帧（MJPG 直通 / 其它格式解码重编码 JPEG）→ `stop_stream()` 关闭。
//!
//! 线程模型：`nokhwa::Camera` 内部是 `Box<dyn CaptureBackendTrait>`（无 `Send` 约束），
//! **不能跨线程搬运**，因此设备只在 worker 线程内创建与使用，主线程持有 `mpsc` 句柄；
//! 跨线程只传 [`Frame`] / [`Uav_Info`] 这类纯数据。顺带把「打开设备可能长时间阻塞」
//! 隔离在 worker 里，主线程用 `recv_timeout` 超时返回 `Err` 而不是卡死。
//!
//! 与另一个相机 sensor（开发机摄像头）的关系：**没有关系**（不 `use` 对方、不共享基类、
//! 重复代码是刻意接受的代价，Task 34 硬要点 1）。因此本文件里不出现对方的模块名
//! （验收 A5 要求两侧互不引用，连注释也不提）。
//!
//! 三条不可退让的硬要点：
//! 1. **MJPG 绝不 `decode_image`**：未启用 `decoding`/`mjpeg` feature，解码恒
//!    `NotImplementedError`（见 [`frame`] 模块注释）；
//! 2. **抓帧超时不 `join` worker**：worker 可能永久卡在驱动 `frame()` 里，`join` 会挂死进程；
//! 3. 所有失败返回 `Err(String)` + 排查提示，**不 panic / 不 unwrap / 不 expect**。
//!
//! Task 34 Step 2 增补：Lua 表多一个**同步** `info()`，返回
//! `{width, height, format, age_ms}`；数据来源三级回退
//! 「最近一次成功抓帧 → 打开时协商到的格式 → `[camera_uav].resolutions` 首项
//! （显式空列表则回落 640x480）」。

pub mod format;
pub mod frame;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nokhwa::utils::{
    self, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
};
use nokhwa::Camera as Nokhwa_Camera;
use tracing::warn;

use pleiades_base::orchestrator::DeviceCapability;

use crate::config::{
    CameraUavConfig, DEFAULT_UAV_CAPTURE_TIMEOUT_MS, DEFAULT_UAV_OPEN_TIMEOUT_MS,
    DEFAULT_UAV_RESOLUTIONS,
};

use format::{Choose_Format, Format_Line, Format_Table, Parse_Prefer, Prefer_Format};
use frame::{Capture_From_Buffer, Frame};

/// 关闭时等待 worker 退出的上限；超时则放弃 `join`（detach），保证进程一定能退出。
pub const SHUTDOWN_TIMEOUT_MS: u64 = 2_000;

/// `[camera_uav].prefer` 缺省值（与 `fill_camera_uav` 写入的默认一致）
const DEFAULT_PREFER: &str = "mjpg";
/// `[camera_uav].path` 缺省值
const DEFAULT_PATH: &str = "0";
/// `[camera_uav].fps` 缺省值：0 = 取该组合枚举到的最高帧率
const DEFAULT_FPS: u32 = 0;
/// 从未打开、也无任何可参考格式时 `info().format` 的取值（三处 sensor 逐字统一）
const FORMAT_NONE: &str = "none";
/// `[camera_uav].resolutions` 显式为空时 `info()` 的兜底宽（本 sensor 的常见最小档）
const FALLBACK_WIDTH: u32 = 640;
/// `[camera_uav].resolutions` 显式为空时 `info()` 的兜底高
const FALLBACK_HEIGHT: u32 = 480;

/// 发给 worker 的指令。
enum Uav_Cmd {
    Capture {
        reply: tokio::sync::oneshot::Sender<Result<Captured_Frame, String>>,
    },
    Shutdown,
}

/// 一次抓帧的完整结果（worker → 主线程）：[`Frame`] 给 JPEG，`at` 给 `info().age_ms`。
struct Captured_Frame {
    /// worker 内抓到的整帧（宽高/格式/JPEG 都在里面）
    frame: Frame,
    /// 帧被取到的时刻（`device.frame()` 返回后**立刻**打戳，不含解码/编码耗时）
    at: Instant,
}

/// 最近一次成功抓帧的元数据（`info()` 的首选数据源）。
///
/// [`Frame`] 里的 `jpeg`/`rgb` 会把整帧像素留在共享状态里，没必要；这里只留纯数据。
#[derive(Clone, Copy)]
struct Last_Frame {
    width: u32,
    height: u32,
    format: FrameFormat,
    /// 帧被取到的时刻（与 `Captured_Frame::at` 同一个戳）
    at: Instant,
}

/// `info()` 的返回值（字段与另一侧 sensor 逐字一致；**不要加第五个字段**）。
struct Sensor_Info {
    width: u32,
    height: u32,
    format: String,
    age_ms: i64,
}

/// 打开成功后回传的相机信息（含枚举表 —— 现场排障的唯一依据）。
#[derive(Clone, Debug)]
pub struct Uav_Info {
    /// 设备索引（`/dev/videoN` 的 N）
    pub index: u32,
    /// 设备 human_name（`v4l2-ctl --list-devices` 里的名字）
    pub name: String,
    /// 实际生效格式
    pub format: CameraFormat,
    /// 为什么选它（来自 [`Choose_Format`]）
    pub reason: String,
    /// 逐行格式枚举表（已去重 + 排序 + 截断）
    pub format_lines: Vec<String>,
}

/// 内部共享状态（`Arc<Uav_Inner>` 让 `register_lua_caps` 闭包能跨线程捕获）。
struct Uav_Inner {
    config: CameraUavConfig,
    cmd_tx: Mutex<Option<mpsc::Sender<Uav_Cmd>>>,
    /// worker 真正退出时发一个 `()`；`Close` 用它做有上限的等待。
    /// `mpsc::Receiver` 不是 `Sync`，必须包在 `Mutex` 里。
    done_rx: Mutex<Option<mpsc::Receiver<()>>>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 只有 worker 成功回传 `Ok(info)` 才置 true（`Drop` 是否 `join` 的判据）。
    opened: AtomicBool,
    info: Mutex<Option<Uav_Info>>,
    /// 最近一次**成功**抓帧的元数据（抓帧失败不写，`age_ms` 因此不会"变新鲜"）。
    /// `Close()` **不清空**：关掉后仍能回答「最后一帧是什么尺寸/格式、多久之前」。
    last_frame: Mutex<Option<Last_Frame>>,
}

impl Uav_Inner {
    /// 发 Capture 命令 + 等待 JPEG 字节（核心业务逻辑，独立于 Lua 胶水）。
    ///
    /// 关键：取 `cmd_tx` 克隆后**先释放锁再 `await`**（保持 future `Send`）；
    /// 抓帧超时**只返回 `Err`**，绝不 `join` worker。
    async fn Capture_Impl(&self) -> Result<Vec<u8>, String> {
        let tx = {
            let guard = Lock(&self.cmd_tx);
            guard
                .as_ref()
                .ok_or_else(|| {
                    "camera_uav 未打开（[camera_uav] enabled = false，或打开失败已被跳过）"
                        .to_string()
                })?
                .clone()
        };

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(Uav_Cmd::Capture { reply: reply_tx })
            .map_err(|_| {
                "camera_uav 工作线程已退出，无法抓帧（设备被拔出或被其他进程抢占？）\n  \
                 排查建议：重新插拔后重启 pleiades-uav；sudo fuser -v /dev/videoN 确认没有第二个进程占用"
                    .to_string()
            })?;

        let timeout_ms = self
            .config
            .capture_timeout_ms
            .unwrap_or(DEFAULT_UAV_CAPTURE_TIMEOUT_MS);
        let reply = tokio::time::timeout(Duration::from_millis(timeout_ms), reply_rx)
            .await
            .map_err(|_| {
                format!(
                    "camera_uav 抓帧超时（{timeout_ms}ms）：摄像头没有回帧\n  \
                     排查建议：\n  \
                     1. 降低 [camera_uav].resolutions / fps（1080p 在 USB2 上带宽吃紧）\n  \
                     2. 换 USB 口（优先主板直出，不用 HUB），确认供电充足\n  \
                     3. 确认镜头盖已打开、dmesg | tail -30 看 uvcvideo 报错\n  \
                     4. 注意：超时不会杀掉工作线程（可能仍卡在驱动 frame() 里），\
                     后续抓帧会排队；退出时由 SHUTDOWN_TIMEOUT_MS 兜底 detach"
                )
            })?
            .map_err(|_| {
                "camera_uav 工作线程在抓帧过程中退出（设备被拔出？）\n  \
                 排查建议：重新插拔摄像头后重启 pleiades-uav"
                    .to_string()
            })?;

        reply.and_then(|captured| {
            // `Jpeg()` 先转成 owned：`captured.frame` 之后还要借给 `Store_Last`。
            let jpeg = captured.frame.Jpeg()?.to_vec();
            self.Store_Last(&captured);
            Ok(jpeg)
        })
    }

    /// 记录最近一次成功抓帧（毒化安全：走 [`Lock`]）
    fn Store_Last(&self, captured: &Captured_Frame) {
        *Lock(&self.last_frame) = Some(Last_Frame {
            width: captured.frame.width,
            height: captured.frame.height,
            format: captured.frame.format,
            at: captured.at,
        });
    }

    /// `info()` 的业务逻辑：三级回退，**从不失败、从不返回 nil**。
    ///
    /// 1) `last_frame`：最近一次**成功**抓帧的宽高/格式 + 帧龄；
    /// 2) 打开时的 [`Uav_Info::format`]：设备真实生效的组合（首帧之前 `age_ms = 0`）；
    /// 3) `[camera_uav].resolutions` 首项（显式空列表 → `FALLBACK_*`），`format = "none"`。
    ///
    /// `age_ms` 语义（三处 sensor 逐字一致，脚本依赖）：
    /// - 单位毫秒，来自 `Instant::elapsed()`，**单调不减**（系统时钟回拨不影响）；
    /// - 表示「距最近一次成功抓帧过了多久」；抓帧失败/超时**不会**刷新它
    ///   （失败不写 `last_frame`，`age_ms` 会继续变大，正是想要的表现）；
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

        let opened = Lock(&self.info).clone();
        if let Some(opened) = opened {
            return Sensor_Info {
                width: opened.format.width(),
                height: opened.format.height(),
                format: Format_Token(opened.format.format()).to_string(),
                age_ms: 0,
            };
        }

        let (width, height) = Resolutions(&self.config)
            .first()
            .copied()
            .unwrap_or((FALLBACK_WIDTH, FALLBACK_HEIGHT));
        Sensor_Info {
            width,
            height,
            format: FORMAT_NONE.to_string(),
            age_ms: 0,
        }
    }
}

/// UAV 机载相机 sensor（`Send + Sync`，可直接 `Arc<dyn DeviceCapability>`）。
pub struct Uav_Camera {
    inner: Arc<Uav_Inner>,
}

impl Uav_Camera {
    /// 只存配置，不碰设备（未 `Open` 时 `Capture` 返回 `Err`）
    pub fn new(config: CameraUavConfig) -> Self {
        Self {
            inner: Arc::new(Uav_Inner {
                config,
                cmd_tx: Mutex::new(None),
                done_rx: Mutex::new(None),
                join: Mutex::new(None),
                opened: AtomicBool::new(false),
                info: Mutex::new(None),
                last_frame: Mutex::new(None),
            }),
        }
    }

    /// `path` → 设备索引（纯解析，不开设备，错误文案与 [`Open`](Self::Open) 完全一致）。
    ///
    /// 供 bootstrap 做「同一 `/dev/videoN` 只允许一个 sensor」的守卫。
    pub fn Device_Index(&self) -> Result<u32, String> {
        Parse_Camera_Index(self.inner.config.path.as_deref().unwrap_or(DEFAULT_PATH))
    }

    /// 打开时收集的信息；未打开 → `None`。
    pub fn Info(&self) -> Option<Uav_Info> {
        Lock(&self.inner.info).clone()
    }

    /// 打开相机：起 worker 线程 → 等它回传 `Ok(info)`（幂等：已打开 → `Ok(())`）。
    ///
    /// 失败语义（全部 `Err(String)`，worker 句柄一律 detach，不 `join`）：
    /// - worker 报错 → 透传（含格式枚举表等排障信息）；
    /// - 超时 → 提示驱动/设备无响应 + 处置建议；
    /// - worker 未回传任何状态就退出 → 提示设备节点/占用问题。
    pub fn Open(&self) -> Result<(), String> {
        if Lock(&self.inner.cmd_tx).is_some() {
            return Ok(());
        }

        let index = self.Device_Index()?;
        let config = self.inner.config.clone();
        let open_timeout_ms = config
            .open_timeout_ms
            .unwrap_or(DEFAULT_UAV_OPEN_TIMEOUT_MS);

        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<Uav_Info, String>>(1);
        let (cmd_tx, cmd_rx) = mpsc::channel::<Uav_Cmd>();
        let (done_tx, done_rx) = mpsc::channel::<()>();

        let handle = match std::thread::Builder::new()
            .name(format!("camera-uav-worker-{index}"))
            .spawn(move || Worker_Main(index, config, ready_tx, cmd_rx, done_tx))
        {
            Ok(handle) => handle,
            Err(error) => {
                return Err(format!(
                    "创建 camera_uav 工作线程失败：{error}\n  排查建议：确认进程线程/内存配额（ulimit -u、free -h）"
                ))
            }
        };

        match ready_rx.recv_timeout(Duration::from_millis(open_timeout_ms)) {
            Ok(Ok(info)) => {
                *Lock(&self.inner.info) = Some(info);
                self.inner.opened.store(true, Ordering::SeqCst);
                *Lock(&self.inner.cmd_tx) = Some(cmd_tx);
                *Lock(&self.inner.done_rx) = Some(done_rx);
                *Lock(&self.inner.join) = Some(handle);
                Ok(())
            }
            Ok(Err(message)) => {
                // worker 已自行退出（设备打不开 / 格式不支持 / 开流失败），句柄直接丢弃。
                drop(handle);
                Err(message)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                drop(handle);
                Err(format!(
                    "camera_uav 打开超时（{open_timeout_ms}ms）：驱动/设备无响应\n  \
                     排查建议：\n  \
                     1. 换一个 USB 口（优先主板直出，不用 HUB），确认供电充足\n  \
                     2. dmesg | tail -30 看 uvcvideo 是否报错\n  \
                     3. 重新插拔摄像头后重试\n  \
                     4. 确认没有别的进程占着设备：sudo fuser -v /dev/video{index}"
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                drop(handle);
                Err(format!(
                    "camera_uav 工作线程异常退出（未回传任何状态）\n  \
                     排查建议：确认 /dev/video{index} 存在、未被其他进程独占（sudo fuser -v /dev/video{index}）"
                ))
            }
        }
    }

    /// 取最新一帧的 JPEG 字节（`open_stream()` 后 `frame()` 即最新帧，不额外缓冲）。
    ///
    /// 预留 Rust 侧 API；Lua 侧走 [`DeviceCapability::register_lua_caps`] 的 `camera_uav.capture`。
    #[allow(dead_code)]
    pub async fn Capture(&self) -> Result<Vec<u8>, String> {
        self.inner.Capture_Impl().await
    }

    /// 发 Shutdown + 有上限等待 worker 退出；超时则 detach（保证进程能退出）。
    pub fn Close(&self) {
        if let Some(tx) = Lock(&self.inner.cmd_tx).take() {
            let _ = tx.send(Uav_Cmd::Shutdown);
        }

        let done = Lock(&self.inner.done_rx).take();
        let finished = match done {
            Some(done_rx) => match done_rx.recv_timeout(Duration::from_millis(SHUTDOWN_TIMEOUT_MS))
            {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
            },
            // 从未成功打开（没有 done_rx）：没什么可等的。
            None => true,
        };

        if finished {
            if let Some(handle) = Lock(&self.inner.join).take() {
                let _ = handle.join();
            }
        } else {
            // worker 还卡在设备调用里：丢掉句柄让它 detach。
            Lock(&self.inner.join).take();
            warn!(
                "[UAV] sensor=camera_uav 关闭超时（{SHUTDOWN_TIMEOUT_MS}ms），工作线程已 detach（进程可正常退出）"
            );
        }

        self.inner.opened.store(false, Ordering::SeqCst);
        *Lock(&self.inner.info) = None;
    }
}

impl Drop for Uav_Camera {
    fn drop(&mut self) {
        if !self.inner.opened.load(Ordering::SeqCst) {
            // 未成功打开过：直接丢句柄（detach），绝不 join 卡在 Camera::new 的线程。
            return;
        }
        self.Close();
    }
}

/// 设备能力注册：Lua 表 `camera_uav` 有两个成员——
/// `capture()`（异步，返回 JPEG 二进制字符串）与
/// `info()`（**同步**，返回 `{width, height, format, age_ms}`，永不抛错、永不返回 nil）。
/// 闭包只做薄胶水：`Capture_Impl()` + `lua.create_string(jpeg)`；`Info_Impl()` + `Info_Table()`。
impl DeviceCapability for Uav_Camera {
    fn register_lua_caps(&self, lua: &mlua::Lua) -> mlua::Result<()> {
        let inner = self.inner.clone();
        let table = lua.create_table()?;
        table.set(
            "capture",
            lua.create_async_function(move |lua, (): ()| {
                let inner = inner.clone();
                async move {
                    let jpeg = inner.Capture_Impl().await.map_err(mlua::Error::runtime)?;
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

        lua.globals().set("camera_uav", table)?;
        Ok(())
    }
}

/// worker 线程主体：建设备 → 枚举 → 选格式 → 设格式（失败 `drop` 后 `Closest` 重开）
/// → 开流 → 回传信息 → 服务抓帧指令 → `stop_stream()`。
fn Worker_Main(
    index: u32,
    config: CameraUavConfig,
    ready_tx: mpsc::SyncSender<Result<Uav_Info, String>>,
    cmd_rx: mpsc::Receiver<Uav_Cmd>,
    done_tx: mpsc::Sender<()>,
) {
    // 1. 建设备（格式先给 None，打开后再枚举、再设具体格式）
    let mut device = match Open_Nokhwa(index) {
        Ok(device) => device,
        Err(message) => {
            let _ = ready_tx.send(Err(message));
            return;
        }
    };

    // 2. 枚举格式（失败不致命，降级为当前格式）
    let (all_formats, enum_warning) = Enumerate_Formats(&mut device);
    let format_lines = Format_Table(&all_formats, enum_warning.as_deref());

    // 3. 选格式（prefer 非法时 warn + 回退默认）
    let prefer = match Parse_Prefer(config.prefer.as_deref().unwrap_or(DEFAULT_PREFER)) {
        Ok(prefer) => prefer,
        Err(error) => {
            warn!("[UAV] sensor=camera_uav {error}");
            Prefer_Format::Mjpg
        }
    };
    let resolutions = Resolutions(&config);
    let fps = config.fps.unwrap_or(DEFAULT_FPS);
    let (chosen, reason) = Choose_Format(&all_formats, prefer, &resolutions, fps);

    // 4. 设格式：Exact 走 fulfill()，wanted_decoder 给全 6 种格式因此必然命中 chosen 本身
    if chosen != device.camera_format() {
        let request = RequestedFormat::with_formats(
            RequestedFormatType::Exact(chosen),
            utils::frame_formats(),
        );
        if let Err(set_error) = device.set_camera_requset(request) {
            // 设格式失败：必须先 drop 掉当前设备（nokhwa 全局设备表对同 index 并存会 assert 崩溃），
            // 再按 Closest 重新打开。
            drop(device);
            match Open_Nokhwa_Closest(index, chosen) {
                Ok(reopened) => device = reopened,
                Err(reopen_error) => {
                    let mut message = String::from("camera_uav 不支持任何可用格式\n");
                    message.push_str(&format!(
                        "  设格式失败（{}）：{set_error}\n",
                        Format_Line(chosen)
                    ));
                    message.push_str(&format!("  Closest 重开失败：{reopen_error}\n"));
                    message.push_str("  实际枚举到的格式：\n");
                    for line in &format_lines {
                        message.push_str(&format!("{line}\n"));
                    }
                    let _ = ready_tx.send(Err(message));
                    return;
                }
            }
        }
    }

    // 5. 必须显式开流：V4L2 后端不会自动开流，否则 frame() 恒返回 "Stream Not Started"
    if let Err(error) = device.open_stream() {
        let _ = ready_tx.send(Err(format!(
            "camera_uav 启动视频流失败（{}）：{error}\n  \
             排查建议：\n  \
             1. 换 USB 口并确认供电充足（1080p MJPG 在 USB2 上带宽吃紧，程序会按 resolutions 降级）\n  \
             2. 确认没有别的进程正在读 /dev/video{index}（sudo fuser -v /dev/video{index}）\n  \
             3. dmesg | tail -30 看驱动报错",
            Format_Line(device.camera_format())
        )));
        return;
    }

    // 6. 回传打开结果；主线程若已放弃（超时退出），别继续占着设备
    let info = Uav_Info {
        index,
        name: device.info().human_name(),
        format: device.camera_format(),
        reason,
        format_lines,
    };
    if ready_tx.send(Ok(info)).is_err() {
        let _ = device.stop_stream();
        return;
    }

    // 7. 服务抓帧指令
    loop {
        match cmd_rx.recv() {
            Ok(Uav_Cmd::Capture { reply }) => {
                let result = Capture_One(&mut device);
                let _ = reply.send(result);
            }
            Ok(Uav_Cmd::Shutdown) | Err(_) => {
                let _ = device.stop_stream();
                break;
            }
        }
    }

    drop(device);
    let _ = done_tx.send(());
}

/// 建 nokhwa 设备，格式先给 `None`（打开后再枚举、再设具体格式）。
fn Open_Nokhwa(index: u32) -> Result<Nokhwa_Camera, String> {
    let requested =
        RequestedFormat::with_formats(RequestedFormatType::None, utils::frame_formats());

    Nokhwa_Camera::new(CameraIndex::Index(index), requested)
        .map_err(|error| Troubleshoot_Hint(index, &format!("打开摄像头失败：{error}")))
}

/// 用 `Closest(chosen)` 重新打开（`set_camera_requset(Exact)` 失败后的兜底）。
fn Open_Nokhwa_Closest(index: u32, target: CameraFormat) -> Result<Nokhwa_Camera, String> {
    let requested =
        RequestedFormat::with_formats(RequestedFormatType::Closest(target), utils::frame_formats());

    Nokhwa_Camera::new(CameraIndex::Index(index), requested).map_err(|error| {
        Troubleshoot_Hint(index, &format!("打开摄像头失败（Closest 重试）：{error}"))
    })
}

/// 枚举该设备支持的全部 `CameraFormat`；失败不致命，降级为当前格式并给出告警。
fn Enumerate_Formats(device: &mut Nokhwa_Camera) -> (Vec<CameraFormat>, Option<String>) {
    match device.compatible_camera_formats() {
        Ok(list) if !list.is_empty() => (list, None),
        Ok(_) => (
            vec![device.camera_format()],
            Some("枚举返回空列表，降级为当前格式（实际可用的组合可能更多）".to_string()),
        ),
        Err(error) => (
            vec![device.camera_format()],
            Some(format!("枚举失败：{error}，降级为当前格式（继续尝试抓帧）")),
        ),
    }
}

/// 抓一帧并转换成 [`Captured_Frame`]（在 worker 线程内调用）。
///
/// `at` 在 `device.frame()` 返回后**立刻**打戳（不是转码完才打），否则 `age_ms`
/// 会把解码/重编码耗时也算进去。
fn Capture_One(device: &mut Nokhwa_Camera) -> Result<Captured_Frame, String> {
    let buffer = device.frame().map_err(|error| {
        format!(
            "camera_uav 取帧失败：{error}（设备被拔出或被抢占？）\n  \
             排查建议：sudo fuser -v /dev/videoN 看占用；重新插拔后重启 pleiades-uav"
        )
    })?;
    let at = Instant::now();

    Ok(Captured_Frame {
        frame: Capture_From_Buffer(buffer)?,
        at,
    })
}

/// `[camera_uav].resolutions` → `(宽, 高)` 列表。
///
/// `None`（键缺失）→ 用 [`DEFAULT_UAV_RESOLUTIONS`]；`Some([])`（显式空列表）→ 空列表，
/// 表示「无分辨率偏好」，由 [`Choose_Format`] 直接走 Closest 兜底。
fn Resolutions(config: &CameraUavConfig) -> Vec<(u32, u32)> {
    match config.resolutions.as_ref() {
        Some(list) => list.iter().map(|pair| (pair[0], pair[1])).collect(),
        None => DEFAULT_UAV_RESOLUTIONS
            .iter()
            .map(|pair| (pair[0], pair[1]))
            .collect(),
    }
}

/// `path` → `u32` 设备索引的纯解析。
///
/// `"0"` / `" 3 "` → `Ok(0)` / `Ok(3)`；`"/dev/video12"` → `Ok(12)`；
/// `"/dev/video"`、`"abc"`、`"4k"`、`""`、溢出数字 → `Err(String)`。
///
/// 与另一个相机 sensor 里那份实现**刻意重复**（两个 sensor 互不引用，Task 34 硬要点 1/3）：
/// nokhwa 0.10 的 V4L2 后端只接受 `CameraIndex::Index(u32)`，字符串路径会在内部解析失败。
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
        "camera_uav.path=\"{path}\" 非法（{reason}）\n  \
         只接受两种写法：纯数字索引（如 \"0\" / \"1\"）或 Linux 设备节点（如 \"/dev/video0\"）\n  \
         原因：nokhwa 0.10 的 V4L2 后端只接受 CameraIndex::Index(u32)，字符串路径会在内部解析失败\n  \
         复核命令：ls -l /dev/video* 与 v4l2-ctl --list-devices"
    )
}

/// 打开设备失败的统一排障附录：把常见 errno 文案翻译成人话 + 给出处置命令。
pub fn Troubleshoot_Hint(index: u32, error: &str) -> String {
    let lowered = error.to_lowercase();
    let device_path = format!("/dev/video{index}");

    let head = if lowered.contains("busy") {
        format!(
            "camera_uav 打开摄像头失败：设备被占用（Device or resource busy）\n  原始错误：{error}"
        )
    } else if lowered.contains("permission")
        || lowered.contains("eacces")
        || lowered.contains("access denied")
    {
        format!("camera_uav 打开摄像头失败：权限不足\n  原始错误：{error}")
    } else if lowered.contains("no such file")
        || lowered.contains("not found")
        || lowered.contains("does not exist")
    {
        format!("camera_uav 打开摄像头失败：设备节点不存在（{device_path}）\n  原始错误：{error}")
    } else {
        format!("camera_uav 打开摄像头失败\n  原始错误：{error}")
    };

    format!(
        "{head}\n\
         排查建议：\n  \
         1. 设备独占：同一时刻只能有一个进程持有 {device_path}\n     \
            sudo fuser -v {device_path}      # 看谁占着\n     \
            pkill mjpg_streamer; pkill motion; pkill fswebcam\n  \
         2. 关掉另一个相机 sensor 的 enabled（同一个 /dev/videoN 不能被两个 sensor 同时使用）\n  \
         3. 权限：sudo usermod -aG video $USER 后重新登录（或临时 sudo 运行）\n  \
         4. 无设备：ls -l /dev/video* 与 v4l2-ctl --list-devices 复核，重插 USB、检查供电\n  \
         5. 驱动：dmesg | tail -30 看 uvcvideo 是否正常枚举"
    )
}

/// 取锁，毒化（某线程 panic 过）时恢复内部数据而不是二次 panic。
fn Lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 帧龄毫秒：`Instant::elapsed()` **单调不减**，饱和到 `i64` 上限，不 panic、不回绕、不为负。
fn Age_Ms(at: Instant) -> i64 {
    Duration_Ms(at.elapsed())
}

/// `Duration` → 毫秒 `i64`（超过 `i64::MAX` 时饱和，绝不回绕成负数）。
fn Duration_Ms(elapsed: Duration) -> i64 {
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

/// `FrameFormat` → 简短格式串（与 `prefer = "mjpg"`、日志用词一致的那套写法）。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_camera_index_accepts_numeric_and_dev_video() {
        assert_eq!(Parse_Camera_Index("0"), Ok(0));
        assert_eq!(Parse_Camera_Index("3"), Ok(3));
        assert_eq!(Parse_Camera_Index("  7  "), Ok(7));
        assert_eq!(Parse_Camera_Index("/dev/video0"), Ok(0));
        assert_eq!(Parse_Camera_Index("/dev/video12"), Ok(12));
        assert_eq!(Parse_Camera_Index("  /dev/video2  "), Ok(2));
    }

    #[test]
    fn parse_camera_index_rejects_others() {
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
                error.contains("camera_uav.path"),
                "错误文案应点名配置键：{error}"
            );
            assert!(
                error.contains("CameraIndex::Index"),
                "错误文案应说明 nokhwa 只接受索引：{error}"
            );
        }
        assert!(
            Parse_Camera_Index("4294967296").is_err(),
            "溢出 u32 必须拒绝"
        );
    }

    #[test]
    fn troubleshoot_hint_maps_busy_permission_missing() {
        let busy = Troubleshoot_Hint(0, "Device or resource busy");
        assert!(busy.contains("设备被占用"), "{busy}");
        assert!(busy.contains("/dev/video0"), "{busy}");
        assert!(busy.contains("sudo fuser -v /dev/video0"), "{busy}");

        let permission = Troubleshoot_Hint(2, "Permission denied (os error 13)");
        assert!(permission.contains("权限不足"), "{permission}");
        assert!(permission.contains("/dev/video2"), "{permission}");
        assert!(permission.contains("usermod -aG video"), "{permission}");

        let missing = Troubleshoot_Hint(4, "No such file or directory (os error 2)");
        assert!(missing.contains("设备节点不存在"), "{missing}");
        assert!(missing.contains("/dev/video4"), "{missing}");

        // 未知错误也要给通用分支 + 处置命令，不能空手返回
        let other = Troubleshoot_Hint(1, "some unknown driver failure");
        assert!(other.contains("打开摄像头失败"), "{other}");
        assert!(other.contains("dmesg | tail -30"), "{other}");
        assert!(other.contains("v4l2-ctl --list-devices"), "{other}");
    }

    #[test]
    fn device_index_uses_config_path() {
        let camera = Uav_Camera::new(UavCameraConfig_For_Test("/dev/video5"));
        assert_eq!(camera.Device_Index(), Ok(5));
        assert!(camera.Info().is_none(), "未打开时不应有 Info");
        // 未打开时抓帧必须直接 Err，不能 panic
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("测试用 runtime 应能创建");
        let error = runtime
            .block_on(camera.Capture())
            .expect_err("未打开时抓帧必须失败");
        assert!(error.contains("未打开"), "{error}");
    }

    #[test]
    fn resolutions_config_mapping() {
        // 键缺失 → 默认两档
        let config = UavCameraConfig_For_Test_With_Resolutions(None);
        assert_eq!(Resolutions(&config), vec![(640, 480), (1280, 720)]);
        // 显式空列表 → 空（无分辨率偏好，走 Closest）
        let empty = UavCameraConfig_For_Test_With_Resolutions(Some(Vec::new()));
        assert!(Resolutions(&empty).is_empty());
        // 显式单档 → 原样
        let single = UavCameraConfig_For_Test_With_Resolutions(Some(vec![[1920, 1080]]));
        assert_eq!(Resolutions(&single), vec![(1920, 1080)]);
    }

    #[test]
    fn format_token_maps_all_variants() {
        // Lua 侧只认这七个写法（六个格式 + "none"），任何一个跑偏都会让脚本打印出怪值
        assert_eq!(Format_Token(FrameFormat::MJPEG), "MJPG");
        assert_eq!(Format_Token(FrameFormat::YUYV), "YUYV");
        assert_eq!(Format_Token(FrameFormat::NV12), "NV12");
        assert_eq!(Format_Token(FrameFormat::GRAY), "GRAY");
        assert_eq!(Format_Token(FrameFormat::RAWRGB), "RGB");
        assert_eq!(Format_Token(FrameFormat::RAWBGR), "BGR");
        assert_eq!(FORMAT_NONE, "none");
    }

    #[test]
    fn info_before_open_uses_resolution_preference() {
        // 没打开：拿 [camera_uav].resolutions 首项 + format="none" + age_ms=0
        let camera = Uav_Camera::new(UavCameraConfig_For_Test_With_Resolutions(Some(vec![
            [800, 600],
            [1280, 720],
        ])));
        let info = camera.inner.Info_Impl();
        assert_eq!((info.width, info.height), (800, 600), "应取首选分辨率");
        assert_eq!(info.format, FORMAT_NONE, "没打开过就必须是 none");
        assert_eq!(info.age_ms, 0, "没有帧时 age_ms 取 0");
    }

    #[test]
    fn info_falls_back_when_resolutions_empty() {
        // 显式空列表 = 无分辨率偏好 → 兜底 640x480，而不是 panic / 0x0
        let camera = Uav_Camera::new(UavCameraConfig_For_Test_With_Resolutions(Some(Vec::new())));
        let info = camera.inner.Info_Impl();
        assert_eq!((info.width, info.height), (FALLBACK_WIDTH, FALLBACK_HEIGHT));
        assert_eq!((info.width, info.height), (640, 480));
        assert_eq!(info.format, FORMAT_NONE);
        assert_eq!(info.age_ms, 0);
    }

    #[test]
    fn info_after_frame_uses_frame_metadata() {
        // 有帧：帧元数据压过配置，帧龄来自打戳
        let camera = Uav_Camera::new(UavCameraConfig_For_Test_With_Resolutions(Some(vec![
            [640, 480],
        ])));
        camera.inner.Store_Last(&Captured_Frame {
            frame: Frame {
                format: FrameFormat::MJPEG,
                width: 1920,
                height: 1080,
                jpeg: Some(vec![0xFF, 0xD8, 0xFF, 0xD9]),
                rgb: None,
            },
            at: Instant::now() - Duration::from_millis(25),
        });
        let info = camera.inner.Info_Impl();
        assert_eq!((info.width, info.height), (1920, 1080), "应取最近帧的宽高");
        assert_eq!(info.format, "MJPG", "MJPEG 的对外写法是 MJPG");
        assert!(info.age_ms >= 25, "帧龄至少 25ms（单调测量）：{}", info.age_ms);
    }

    #[test]
    fn lua_info_table_matches_contract() {
        let lua = mlua::Lua::new();
        let info = Sensor_Info {
            width: 640,
            height: 480,
            format: "MJPG".to_string(),
            age_ms: 7,
        };
        let table = Info_Table(&lua, &info).expect("info 表应能构造");
        lua.globals().set("info", table).expect("info 应能挂到全局");

        // 字段数必须恰好 4（与另一侧 sensor 的契约逐字一致）
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
        assert_eq!((width, height), (640, 480));
        assert_eq!(format, "MJPG");
        assert_eq!(age_ms, 7);

        let age_kind: String = lua
            .load("return math.type(info.age_ms)")
            .eval()
            .expect("math.type 应可调用");
        assert_eq!(age_kind, "integer", "age_ms 必须是 Lua integer");
    }

    /// 构造一个只有 `path` 有意义的配置（其余字段全走缺省）
    fn UavCameraConfig_For_Test(path: &str) -> CameraUavConfig {
        CameraUavConfig {
            enabled: Some(true),
            path: Some(path.to_string()),
            prefer: None,
            resolutions: None,
            fps: None,
            open_timeout_ms: None,
            capture_timeout_ms: None,
        }
    }

    /// 构造一个指定 `resolutions` 的配置
    fn UavCameraConfig_For_Test_With_Resolutions(
        resolutions: Option<Vec<[u32; 2]>>,
    ) -> CameraUavConfig {
        CameraUavConfig {
            enabled: Some(true),
            path: Some("0".to_string()),
            prefer: None,
            resolutions,
            fps: None,
            open_timeout_ms: None,
            capture_timeout_ms: None,
        }
    }
}
