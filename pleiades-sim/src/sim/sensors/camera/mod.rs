//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-24

#![allow(non_snake_case)]

//! Camera 传感器（task_34 §二 Step 4）：帧入口 / 最新帧快照 / 帧龄 / 生命周期
//!
//! 与 lidar 的**唯一实质差别**：相机不建图、不急停，**只保留最新一帧**给别人取
//! （`camera_sim.capture()` 拿 JPEG、`camera_sim.info()` 拿四个字段）。
//! 所以：
//! - `On_Binary` **同步**解析并覆盖 `Mutex<Option<Latest>>`（不 spawn task、不 await）
//! - 覆盖用**阻塞 `lock()`** 而不是 lidar 的 `try_write()`：lidar 抢不到锁可以丢这一圈
//!   （下一圈还会来，建图有 200 ms 节拍兜底）；相机要求"保留的必须是真最新"，
//!   `try_lock` 失败会让"最新"退化成"不一定最新"。临界区只有一次 move，且不 `await`，
//!   不会阻塞收包 task（`world_link/link.rs` 的纪律）
//! - `Shutdown` 是空实现（没有自持 task），只为满足 [`Sensor`]
//!
//! 帧龄用 `Instant`（进程内单调钟）：系统时钟回拨不影响 `Age_Ms()`。

pub mod frame;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use tracing::info;

use crate::sim::sensors::camera::frame::CameraFrame;
use crate::sim::sensors::{Sensor, Warn_Throttled};

/// `[sim_world.sensor.params].rate_hz` 缺键时的默认值（与世界 `SensorCatalog` 的 camera 默认 1 Hz 对齐）
///
/// **只用于帧龄告警阈值**，不参与任何控制/建图；sim 不复制世界的其它数值默认
/// （`fov_deg` / `quality` 一律不读，见 plan §9-R15）。
pub const DEFAULT_RATE_HZ: f32 = 1.0;

/// 每 N 帧一条 info（1 Hz → 1 分钟一条）
pub const FRAME_LOG_EVERY: u64 = 60;

/// 来自 `[sim_world.sensor.params]`；缺键/非法 → 回默认，不 panic
///
/// `width/height` **只用于"还没有收到帧"时的 `info()` 占位**（真帧的宽高一律以 JPEG SOF 为准）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraParams {
    pub width: u32,
    pub height: u32,
    pub rate_hz: f32,
}

impl Default for CameraParams {
    fn default() -> Self {
        // 宽高 0 = "未知"：不复制世界默认值，避免两处默认漂移（plan §9-R15）
        Self { width: 0, height: 0, rate_hz: DEFAULT_RATE_HZ }
    }
}

impl CameraParams {
    /// 来自 `[sim_world.sensor.params]`（缺键/非法 → 回默认，不 panic）
    ///
    /// 容忍 TOML 整数与浮点两种写法（`width = 640` 与 `width = 640.0`）；
    /// `<= 0` 与不可解析一律视为缺键。
    pub fn From_Json(v: &serde_json::Value) -> Self {
        let d = Self::default();
        let as_pos_u32 = |key: &str| -> Option<u32> {
            let n = v.get(key)?;
            let f = match n.as_u64() {
                Some(n) => n as f64,
                None => n.as_f64()?,
            };
            if f > 0.0 && f <= u32::MAX as f64 {
                Some(f as u32)
            } else {
                None
            }
        };
        let width = as_pos_u32("width").unwrap_or(d.width);
        let height = as_pos_u32("height").unwrap_or(d.height);
        let rate_hz = v
            .get("rate_hz")
            .and_then(|x| x.as_f64())
            .filter(|r| *r > 0.0)
            .map(|r| r as f32)
            .unwrap_or(d.rate_hz);
        Self { width, height, rate_hz }
    }
}

/// 最近一帧 + 它到达的时刻（`Age_Ms()` 的唯一来源；`Instant` 单调）
struct Latest {
    frame: CameraFrame,
    at: Instant,
}

/// sim 自己的相机传感器（世界 `camera` 零件的对端）
pub struct CameraSensor {
    slot: String,
    params: CameraParams,
    /// **同步** `Mutex`（不是 tokio `RwLock`）：`On_Binary` 不 await，覆盖即"最新帧胜出"
    latest: Mutex<Option<Latest>>,
    /// 成功解出的帧数（首帧 info + 每 [`FRAME_LOG_EVERY`] 条 info）
    frames: AtomicU64,
}

impl CameraSensor {
    pub fn New(slot: &str, params: CameraParams) -> Self {
        Self { slot: slot.to_string(), params, latest: Mutex::new(None), frames: AtomicU64::new(0) }
    }

    pub fn Slot(&self) -> &str {
        &self.slot
    }

    pub fn Params(&self) -> &CameraParams {
        &self.params
    }

    /// 毒化安全的小 helper：临界区只做一次 move，中毒也不 panic（与 `sensors::Should_Log` 同法）
    fn Lock(&self) -> MutexGuard<'_, Option<Latest>> {
        match self.latest.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// 一帧二进制进来：解析成功 → 覆盖最新帧；失败 → 限流告警后**不写 state、不加计数**
    ///
    /// 顺序固定：解析 → 计数/日志 → 覆盖。坏帧绝不覆盖好帧（否则 `capture()` 会拿到坏字节）。
    pub fn On_Binary(&self, bytes: &[u8]) {
        let frame = match frame::Parse_Camera_Frame(bytes) {
            Ok(f) => f,
            Err(e) => {
                Warn_Throttled("camera.bad_frame", 5000, || {
                    format!("[camera] slot={} 帧丢弃：{e}", self.slot)
                });
                return;
            }
        };

        let n = self.frames.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 {
            info!(
                "[camera] slot={} 首帧 {}x{} {} B seq={} world_time={:.3}",
                self.slot,
                frame.width,
                frame.height,
                frame.jpeg.len(),
                frame.seq,
                frame.world_time
            );
        } else if n % FRAME_LOG_EVERY == 0 {
            info!(
                "[camera] slot={} 帧={} {}x{} {} B seq={}",
                self.slot,
                n,
                frame.width,
                frame.height,
                frame.jpeg.len(),
                frame.seq
            );
        }

        // 阻塞 lock（见模块头：相机要的是"真最新"，不是"抢到锁就更新"）
        *self.Lock() = Some(Latest { frame, at: Instant::now() });
    }

    /// 最新一帧的**副本**（`capture()` 每次都要 owned 字节给 Lua）
    pub fn Latest(&self) -> Option<CameraFrame> {
        self.Lock().as_ref().map(|l| l.frame.clone())
    }

    /// 只取宽高（`info()` 用，避免为拿尺寸复制整段 JPEG）
    pub fn Latest_Size(&self) -> Option<(u32, u32)> {
        self.Lock().as_ref().map(|l| (l.frame.width, l.frame.height))
    }

    /// 距最新帧的毫秒数；**从未收到帧 → 0**（与真机两个相机 sensor 的 `age_ms` 语义逐字一致）
    pub fn Age_Ms(&self) -> i64 {
        match self.Lock().as_ref() {
            Some(l) => l.at.elapsed().as_millis() as i64,
            None => 0,
        }
    }

    /// 无自持 task（相机不建图、不急停）→ 空实现，只为满足 [`Sensor`]
    pub fn Shutdown(&self) {}
}

impl Sensor for CameraSensor {
    // 三行都转调上面的 inherent 方法（inherent 优先，不会递归）
    fn Slot(&self) -> &str {
        &self.slot
    }

    fn On_Binary(&self, bytes: &[u8]) {
        CameraSensor::On_Binary(self, bytes)
    }

    fn Shutdown(&self) {
        CameraSensor::Shutdown(self)
    }
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::sensors::camera::frame::{Test_Frame, Test_Jpeg};

    fn feed(sensor: &CameraSensor, seq: u32, jpeg: &[u8]) {
        sensor.On_Binary(&Test_Frame(3, seq, seq as f64 * 0.5, jpeg));
    }

    /// 取最新帧（不用 `expect`：新文件零容忍 unwrap/expect/panic，见 plan §6 A8）
    fn latest(sensor: &CameraSensor, why: &str) -> Option<CameraFrame> {
        let got = sensor.Latest();
        assert!(got.is_some(), "{why}");
        got
    }

    #[test]
    fn test_params_from_json() {
        let p = CameraParams::From_Json(&serde_json::json!({
            "width": 640, "height": 480, "rate_hz": 2
        }));
        assert_eq!(p, CameraParams { width: 640, height: 480, rate_hz: 2.0 });

        // 缺键 → 默认（宽高 0 = 未知，rate 1 Hz）
        let p = CameraParams::From_Json(&serde_json::json!({}));
        assert_eq!(p, CameraParams::default());
        assert_eq!(p.width, 0);
        assert_eq!(p.height, 0);
        assert_eq!(p.rate_hz, DEFAULT_RATE_HZ);

        // 非法值回默认，不 panic
        for bad in [serde_json::json!({"width": "x", "height": 480}),
                    serde_json::json!({"width": 0, "height": 0}),
                    serde_json::json!({"width": -1, "height": -1})] {
            let p = CameraParams::From_Json(&bad);
            assert_eq!(p.width, 0, "非法宽应回默认：{bad}");
        }
        // 浮点写法也能读（TOML 里写 640.0）
        let p = CameraParams::From_Json(&serde_json::json!({"width": 640.0, "height": 480.0}));
        assert_eq!((p.width, p.height), (640, 480));
    }

    #[test]
    fn test_camera_sensor_on_binary_writes_latest() {
        let s = CameraSensor::New("camera_nose", CameraParams::default());
        assert_eq!(s.Slot(), "camera_nose");
        assert!(s.Latest().is_none(), "初始必须没有帧");
        assert_eq!(s.Age_Ms(), 0, "从未收到帧时 age_ms = 0");

        feed(&s, 1, &Test_Jpeg(640, 480));
        assert_eq!(s.Latest_Size(), Some((640, 480)));
        let f = match latest(&s, "喂过帧后必须有最新帧") {
            Some(f) => f,
            None => return,
        };
        assert_eq!(f.seq, 1);
        assert_eq!((f.width, f.height), (640, 480));
    }

    #[test]
    fn test_camera_sensor_latest_wins() {
        let s = CameraSensor::New("camera_nose", CameraParams::default());
        let (j1, j2, j3) = (Test_Jpeg(320, 240), Test_Jpeg(640, 480), Test_Jpeg(1280, 720));
        feed(&s, 11, &j1);
        feed(&s, 22, &j2);
        feed(&s, 33, &j3);
        let f = match latest(&s, "必须有最新帧") {
            Some(f) => f,
            None => return,
        };
        assert_eq!(f.seq, 33, "必须是最后一帧");
        assert_eq!((f.width, f.height), (1280, 720), "宽高必须来自最后一帧");
        assert_eq!(f.jpeg, j3, "JPEG 字节必须来自最后一帧");
        assert_eq!(s.Latest_Size(), Some((1280, 720)));
    }

    #[test]
    fn test_camera_sensor_drops_bad_frame() {
        let s = CameraSensor::New("camera_nose", CameraParams::default());
        // 短包 / 非 type=2 / 坏 JPEG：都不写 state
        s.On_Binary(&[0u8; 3]);
        let mut wrong_type = Test_Frame(3, 1, 0.0, &Test_Jpeg(320, 240));
        wrong_type[0] = 1;
        s.On_Binary(&wrong_type);
        s.On_Binary(&Test_Frame(3, 2, 0.0, &[0, 1, 2, 3]));
        assert!(s.Latest().is_none(), "坏帧不得写 state");
        assert_eq!(s.Age_Ms(), 0);

        // 坏帧不覆盖好帧
        let good = Test_Jpeg(640, 480);
        feed(&s, 7, &good);
        s.On_Binary(&[0u8; 3]);
        s.On_Binary(&Test_Frame(3, 8, 0.0, &[0, 1, 2, 3]));
        let f = match latest(&s, "好帧必须还在") {
            Some(f) => f,
            None => return,
        };
        assert_eq!(f.seq, 7);
        assert_eq!(f.jpeg, good);
    }

    #[test]
    fn test_camera_sensor_age_ms_monotonic() {
        let s = CameraSensor::New("camera_nose", CameraParams::default());
        assert_eq!(s.Age_Ms(), 0);
        feed(&s, 1, &Test_Jpeg(320, 240));
        let age1 = s.Age_Ms();
        assert!(age1 >= 0, "刚喂完不该是负数：{age1}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let age2 = s.Age_Ms();
        assert!(age2 >= age1, "帧龄必须单调不减：{age1} → {age2}");
        assert!(age2 >= 15, "20 ms 后帧龄应到毫秒量级：{age2}");
    }

    #[test]
    fn test_camera_sensor_shutdown_is_idempotent() {
        let s = CameraSensor::New("camera_nose", CameraParams::default());
        s.Shutdown();
        s.Shutdown();
        // 通过 trait 再调一次（走 `impl Sensor`）
        Sensor::Shutdown(&s);
        // 停过之后仍可收帧（没有 task 可停，语义上是 no-op）
        feed(&s, 1, &Test_Jpeg(320, 240));
        assert!(s.Latest().is_some());
    }
}
