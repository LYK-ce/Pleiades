//Presented by KeJi
//Created Date ： 2026-10-31
//Modified Date ： 2026-10-31

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! UGV 车载相机 sensor（HP60C / Angstrong 厂商 SDK；task_43 Step 1）
//!
//! **接口与 `pleiades-uav/src/device/camera_uav/mod.rs` 同形**（同样的
//! `new` / `Open` / `Close` / `latest_frame` / `Info` / `Drop`，同样的槽位语义），
//! 差异只在取帧后端：
//! - 机载相机是 UVC/V4L2 拉帧（`nokhwa`），**自己起 worker 线程**轮询；
//! - 本设备是厂商 SDK **推帧**：`Open` 在调用线程同步完成
//!   `dlopen → Init → GetCameraList → OpenCamera → SetStreamParam → RegisterCallback →
//!   StartStream(0x83)`，**回调跑在 SDK 自己的采集线程上**，直接写 [`slot::FrameSlot`]。
//!
//! 因此本模块**不创建任何线程**：`enabled = false` 时零线程，`enabled = true` 时
//! 跑着的线程也全是 SDK 自己的（生产代码里没有任何线程创建点）。
//!
//! 三条不可退让（对应任务文档 §二 #2/#3 与 `RESULTS.md` §4）：
//! 1. **只读 `mjpegImg` 那一路**（`pstData + 288`，已是完整 JPEG），不做深度；
//! 2. **不做 0x83 → 0x03 降级**：降级后拿不到 JPEG，等于白开流；
//! 3. **不实现 `DeviceCapability`**：Lua 侧 camera 已全部移除（§二 #5），
//!    取帧只走 Rust 侧 [`Ugv_Camera::latest_frame`]，留给将来的车端 detector。

pub mod sdk;
pub mod slot;

use std::os::raw::c_void;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::config::{
    CameraUgvConfig, DEFAULT_CAMERA_UGV_CONFIG_JSON, DEFAULT_CAMERA_UGV_HEIGHT,
    DEFAULT_CAMERA_UGV_SDK_LIB, DEFAULT_CAMERA_UGV_STREAM_FPS, DEFAULT_CAMERA_UGV_TIMEOUT_MS,
    DEFAULT_CAMERA_UGV_WIDTH,
};

use sdk::{Cb_Ctx, Sdk};
use slot::{Frame, FrameSlot};

/// 首帧诊断的轮询周期（毫秒）：只影响"打开后等多久打日志"，不影响取帧
pub const FIRST_FRAME_POLL_MS: u64 = 10;

/// 打开成功后回传的设备信息（bootstrap 日志用；不含任何裸指针）
#[derive(Debug, Clone)]
pub struct Ugv_Info {
    /// 实际加载的 SDK 库路径
    pub sdk_lib: String,
    /// `AS_SDK_GetSwVersion`（失败为 None）
    pub sdk_version: Option<String>,
    /// 实际使用的流掩码（恒为 0x83）
    pub stream_flag: i32,
    /// 请求的流宽（`SetStreamParam` 用；真实帧宽以槽位帧为准）
    pub width: u32,
    /// 请求的流高
    pub height: u32,
    /// 请求的流帧率
    pub stream_fps: u32,
}

impl Ugv_Info {
    /// 一句话描述（bootstrap 成功日志用：现场看一行就知道库/版本/flag/分辨率）
    pub fn Describe(&self) -> String {
        let version = self.sdk_version.as_deref().unwrap_or("未知");
        format!(
            "HP60C sdk={} 版本={} stream_flag=0x{:x} 请求 {}x{}@{}",
            self.sdk_lib, version, self.stream_flag, self.width, self.height, self.stream_fps
        )
    }
}

/// UGV 车载相机（`Send + Sync`，可直接 `Arc` 共享给 detector）
pub struct Ugv_Camera {
    config: CameraUgvConfig,
    /// 最新帧槽位（SDK 采集线程写、`latest_frame()` 同步读）
    slot: Arc<FrameSlot>,
    // ⚠ 字段声明顺序是**有意**的：Rust 按声明顺序析构 ⇒ `sdk` 先析构（走完
    //    Shutdown + dlclose），此时 `cb` 这个 Arc 仍存活，任何残余回调都不会踩到
    //    已释放的 Cb_Ctx。反过来写会留下一个"dlclose 之后回调还在用 ctx"的窗口。
    sdk: Mutex<Option<Sdk>>,
    /// 回调共享状态（交给 SDK 的 privateData 是 `Arc::as_ptr` 的**借用**指针）
    cb: Arc<Cb_Ctx>,
}

impl Ugv_Camera {
    /// 只存配置 + 建槽位，**不碰设备、不 spawn 线程**（未 `Open` 时 `latest_frame()` 恒 `None`）
    pub fn new(config: CameraUgvConfig) -> Self {
        let slot = Arc::new(FrameSlot::New());
        let width = config.width.unwrap_or(DEFAULT_CAMERA_UGV_WIDTH);
        let height = config.height.unwrap_or(DEFAULT_CAMERA_UGV_HEIGHT);
        Self {
            cb: Arc::new(Cb_Ctx::New(slot.clone(), width, height)),
            slot,
            sdk: Mutex::new(None),
            config,
        }
    }

    /// 打开相机（幂等：已打开 → `Ok(())`）。
    ///
    /// 同步走完 SDK 的 6 步（dlopen → Init → GetCameraList → OpenCamera →
    /// SetStreamParam → RegisterCallback → StartStream），然后按
    /// `[camera_ugv].timeout_ms` 做**首帧诊断**（只记日志，失败不回滚、不返回 Err）。
    ///
    /// 失败一律 `Err(String)`（含函数名 + 返回码 + 排查提示），不 panic。
    pub fn Open(&self) -> Result<(), String> {
        let mut guard = Lock(&self.sdk);
        if guard.is_some() {
            return Ok(()); // 幂等
        }
        let lib = self
            .config
            .sdk_lib
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_CAMERA_UGV_SDK_LIB);
        let json = self
            .config
            .config_json
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_CAMERA_UGV_CONFIG_JSON);
        let width = self.config.width.unwrap_or(DEFAULT_CAMERA_UGV_WIDTH);
        let height = self.config.height.unwrap_or(DEFAULT_CAMERA_UGV_HEIGHT);
        let fps = self.config.stream_fps.unwrap_or(DEFAULT_CAMERA_UGV_STREAM_FPS);
        let timeout_ms = self.config.timeout_ms.unwrap_or(DEFAULT_CAMERA_UGV_TIMEOUT_MS);

        // privateData 是**借用**的 Arc 指针，不转移所有权（见 Cb_Ctx 的注释）
        let user = Arc::as_ptr(&self.cb) as *mut Cb_Ctx as *mut c_void;
        // SAFETY: 同一时刻至多一个线程在这把锁内构造/销毁 Sdk（`self.sdk`），
        //         且 `self.cb` 的生命周期覆盖整个 Sdk（字段声明顺序保证 sdk 先析构）。
        let sdk = unsafe { Sdk::Open(lib, json, width, height, fps, user)? };
        *guard = Some(sdk);
        drop(guard); // ⚠ 必须在等待之前放锁：Wait 里会调 latest_frame() / Info() 再取这把锁

        self.Wait_First_Frame_Or_Warn(timeout_ms);
        Ok(())
    }

    /// 关闭相机（幂等）；关掉之后不该再答出旧帧
    pub fn Close(&self) {
        if let Some(sdk) = Lock(&self.sdk).take() {
            // drop 会走 Sdk::Drop → Shutdown（StopStream 只一次 → … → dlclose）
            drop(sdk);
            self.slot.Clear();
        }
    }

    /// 最新一帧（**同步、非阻塞**；无帧 → `None`）。
    ///
    /// 只做「锁槽位 → `Arc` 克隆」两件事，不碰设备、不 `await`，可在任何上下文调用。
    pub fn latest_frame(&self) -> Option<Frame> {
        self.slot.Latest()
    }

    /// 打开时收集的信息；未打开 → `None`。
    pub fn Info(&self) -> Option<Ugv_Info> {
        let guard = Lock(&self.sdk);
        let sdk = guard.as_ref()?;
        let sdk_lib = self
            .config
            .sdk_lib
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CAMERA_UGV_SDK_LIB.to_string());
        Some(Ugv_Info {
            sdk_lib,
            sdk_version: sdk.Version(),
            stream_flag: sdk.Used_Flag(),
            width: self.config.width.unwrap_or(DEFAULT_CAMERA_UGV_WIDTH),
            height: self.config.height.unwrap_or(DEFAULT_CAMERA_UGV_HEIGHT),
            stream_fps: self.config.stream_fps.unwrap_or(DEFAULT_CAMERA_UGV_STREAM_FPS),
        })
    }

    /// 打开后的首帧诊断：轮询最多 `timeout_ms`，**只记日志**（不返回 Err）。
    ///
    /// - `timeout_ms == 0` → 直接返回（逃生开关：bootstrap 不等首帧，立刻往下走）；
    /// - 命中 → `info!` 一行（seq / 宽×高 / 字节数 / 帧龄 / 回调统计）；
    /// - 超时 → `warn!` 一行并带上回调次数（`no_mjpeg` 是 0x83 没生效的第一现场证据）。
    ///
    /// 为什么不返回 Err：回调链已经注册好了，慢一帧不算打开失败；而且失败回滚会把
    /// 唯一的诊断信息（回调计数）一起丢掉。
    fn Wait_First_Frame_Or_Warn(&self, timeout_ms: u64) {
        if timeout_ms == 0 {
            info!("[UGV] camera_ugv 已开流（timeout_ms=0，跳过首帧等待）");
            return;
        }

        let started = Instant::now();
        loop {
            if let Some(frame) = self.latest_frame() {
                let stats = self.cb.Stats();
                let age_ms = slot::Monotonic_Ms().saturating_sub(frame.captured_at_ms);
                info!(
                    "[UGV] camera_ugv 首帧 seq={} {}x{} {} B 帧龄 {}ms；回调统计 cb_total={} frames={} dropped={} bad_size={} no_mjpeg={} panics={}",
                    frame.seq,
                    frame.width,
                    frame.height,
                    frame.jpeg.len(),
                    age_ms,
                    stats.cb_total,
                    stats.frames,
                    stats.dropped,
                    stats.bad_size,
                    stats.no_mjpeg,
                    stats.panics
                );
                return;
            }
            if started.elapsed() >= Duration::from_millis(timeout_ms) {
                let stats = self.cb.Stats();
                warn!(
                    "[UGV] camera_ugv 已开流但 {timeout_ms}ms 内没收到帧（回调 {} 次，no_mjpeg={}）：检查 LD_LIBRARY_PATH / 加密 json 路径 / 是否被 ROS ascamera 节点占用",
                    stats.cb_total, stats.no_mjpeg
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(FIRST_FRAME_POLL_MS));
        }
    }
}

impl Drop for Ugv_Camera {
    fn drop(&mut self) {
        self.Close();
    }
}

/// 取锁，毒化（某线程 panic 过）时恢复内部数据而不是二次 panic
fn Lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全缺省的配置（`enabled = false`，其余走 `DEFAULT_CAMERA_UGV_*`）
    fn Config_For_Test() -> CameraUgvConfig {
        CameraUgvConfig {
            enabled: Some(false),
            sdk_lib: None,
            config_json: None,
            width: None,
            height: None,
            stream_fps: None,
            timeout_ms: None,
        }
    }

    #[test]
    fn new_does_not_open_and_latest_frame_is_none() {
        let camera = Ugv_Camera::new(Config_For_Test());
        assert!(
            camera.latest_frame().is_none(),
            "只构造不 Open：任何情况下都不该有帧"
        );
        assert!(camera.Info().is_none(), "未打开时 Info 必须是 None");
    }

    #[test]
    fn info_is_none_before_open() {
        // 把 timeout_ms 配成 0 也不会改变"未 Open ⇒ 无信息"的语义
        let mut config = Config_For_Test();
        config.timeout_ms = Some(0);
        let camera = Ugv_Camera::new(config);
        assert!(camera.Info().is_none());
        assert!(camera.latest_frame().is_none());
    }

    #[test]
    fn close_without_open_is_noop_and_idempotent() {
        let camera = Ugv_Camera::new(Config_For_Test());
        camera.Close();
        camera.Close();
        assert!(camera.latest_frame().is_none(), "重复 Close 仍不得 panic / 产帧");
        assert!(camera.Info().is_none());
    }

    // 本机没有 HP60C，只能验"失败路径"：dlopen 失败必须给出可读提示并保持状态干净
    #[cfg(unix)]
    #[test]
    fn open_with_missing_library_fails_with_hint() {
        let mut config = Config_For_Test();
        config.sdk_lib = Some("/nonexistent/libAngstrongCameraSdk.so".to_string());
        config.timeout_ms = Some(0); // 即便走到成功路径也不会等首帧
        let camera = Ugv_Camera::new(config);

        let error = camera.Open().expect_err("库不存在时 Open 必须失败");
        assert!(error.contains("dlopen"), "错误文案要点名 dlopen：{error}");
        assert!(
            error.contains("LD_LIBRARY_PATH"),
            "错误文案要给排查提示：{error}"
        );
        assert!(camera.Info().is_none(), "失败的 Open 不得留下半个 Sdk");
        assert!(camera.latest_frame().is_none(), "失败的 Open 不得产帧");
        // 失败后还能再试一次（幂等位没被写脏），仍应是同样的错误
        let again = camera.Open().expect_err("再次 Open 仍应失败");
        assert!(again.contains("dlopen"), "{again}");
        camera.Close(); // 不得 panic
    }

    #[test]
    fn ugv_camera_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Ugv_Camera>();
        assert_send_sync::<Ugv_Info>();
    }

    #[test]
    fn new_does_not_touch_sdk() {
        // new 只做内存动作：不 dlopen、不起线程 ⇒ Open 之前的三个接口全部"空转"
        let camera = Ugv_Camera::new(Config_For_Test());
        assert!(camera.Info().is_none(), "new 不得让 Info 有值");
        assert!(camera.latest_frame().is_none(), "new 不得产帧");
        camera.Close(); // new 之后立刻 Close 必须安全
        let describe = Ugv_Info {
            sdk_lib: "lib.so".to_string(),
            sdk_version: None,
            stream_flag: sdk::STREAM_FLAG,
            width: 640,
            height: 480,
            stream_fps: 25,
        }
        .Describe();
        assert!(describe.contains("0x83"), "Describe 要写出流掩码：{describe}");
        assert!(describe.contains("640x480@25"), "{describe}");
    }
}
