//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! LiDAR 传感器（task_31 §五.4.3）：帧入口 / `LaserScan` 快照 / 生命周期 / 拉起建图 task
//!
//! 与真机同构：**建图 task 归传感器所有**（`Spawn_Mapping` 内部 spawn 200 ms task），
//! 不在 `SimDeviceHandler::on_tick` 里（§五.1：写 `robot.grid` 的是传感器，不是决策链）。
//!
//! 角度与无回波语义（§五.4.3，**逐条对齐**）：
//! - 角度**不上线**：`angle = TAU * i / n`，`n` 取**帧实际点数**（与配置 `rays` 不符 → 限流 warning，
//!   并以实际为准；否则整圈角度会压缩）
//! - `range == 0.0`（无回波）/ `< 0.01` **原样保留**，不剔除——过滤交给 mapper（`0.1~5.0 m`），
//!   这样 `LaserScan` 与真机同义
//! - `intensity` 世界不上线 → 恒 `0.0`
//! - `stamp = (world_time * 1e9) as u64`；`scan_freq = rate_hz`；`scan_time = 1 / rate_hz`

pub mod frame;
pub mod mapper;
pub mod mapping_task;
pub mod types;

pub use types::{LaserPoint, LaserScan, LidarState};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::sim::sensors::lidar::mapping_task::MappingCtx;
use crate::sim::sensors::{Sensor, Warn_Throttled};
use crate::world_link::protocol::MountOut;

// ================================================================ 参数与默认值

/// 缺省线数（世界 `SensorCatalog` 的 lidar_2d 默认值）
pub const DEFAULT_RAYS: usize = 360;
/// 缺省量程（米）
pub const DEFAULT_RANGE: f32 = 12.0;
/// 缺省扫描频率（Hz）
pub const DEFAULT_RATE_HZ: f32 = 10.0;
/// 每 N 帧打一条 info（10 Hz → 5 s 一条，联调时用来核对「points 与实际一致」）
pub const FRAME_LOG_EVERY: u64 = 50;

/// 来自 `[sim_world.sensor.params]`；缺键/非法 → 回默认，不 panic
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LidarParams {
    /// 期望线数（只用于日志与配置对账；角度按**帧实际点数**算）
    pub rays: usize,
    /// 量程（米）
    pub range: f32,
    /// 扫描频率（Hz）
    pub rate_hz: f32,
}

impl Default for LidarParams {
    fn default() -> Self {
        Self { rays: DEFAULT_RAYS, range: DEFAULT_RANGE, rate_hz: DEFAULT_RATE_HZ }
    }
}

impl LidarParams {
    /// 来自 `[sim_world.sensor.params]`（缺键/非法 → 回默认，不 panic）
    pub fn From_Json(v: &serde_json::Value) -> Self {
        let d = Self::default();
        let rays = v
            .get("rays")
            .and_then(|x| x.as_u64())
            .filter(|n| *n > 0)
            .map(|n| n as usize)
            .unwrap_or(d.rays);
        let range = v
            .get("range")
            .and_then(|x| x.as_f64())
            .filter(|r| *r > 0.0)
            .map(|r| r as f32)
            .unwrap_or(d.range);
        let rate_hz = v
            .get("rate_hz")
            .and_then(|x| x.as_f64())
            .filter(|r| *r > 0.0)
            .map(|r| r as f32)
            .unwrap_or(d.rate_hz);
        Self { rays, range, rate_hz }
    }
}

// ================================================================ LidarSensor

/// sim 自己的激光雷达传感器（世界 `lidar_2d` 的对端）
///
/// 一帧进来：`frame::Parse_Lidar_Frame` → [`Build_Scan`] → 写 `LidarState`（`try_write`，
/// 抢不到就丢这一帧，与真机 `lidar/mod.rs` 同语义）。
pub struct LidarSensor {
    slot: String,
    params: LidarParams,
    /// 最新一圈（建图 task / Step 6 急停读）
    state: Arc<RwLock<LidarState>>,
    /// 建图 task 的取消令牌（`Shutdown` 只 cancel，不 join）
    cancel: CancellationToken,
    /// 已解析帧数（首帧 & 每 [`FRAME_LOG_EVERY`] 帧 info）
    frames: AtomicU64,
    /// 本传感器的外参（`[sim_world.sensor.mount]`）
    ///
    /// 只给**急停**用：世界侧 `Lidar2D.ScanOnce()` 从**挂载点**起算，而 `RobotState` 只有车体原点，
    /// 把回波换算成障碍格时必须补这份外参（否则每个障碍格沿车头偏 `mount.x`，约 20% 格宽）。
    /// 建图走的是 [`MappingCtx::mount`]（同一个值，但由建图 task 自己持有）。
    mount: MountOut,
}

impl LidarSensor {
    pub fn New(slot: &str, params: LidarParams) -> Self {
        Self {
            slot: slot.to_string(),
            params,
            state: Arc::new(RwLock::new(LidarState::default())),
            cancel: CancellationToken::new(),
            frames: AtomicU64::new(0),
            mount: MountOut::default(),
        }
    }

    /// 带上外参（builder；[`LidarSensor::New`] 签名保持不变，缺省外参全 0）
    pub fn With_Mount(mut self, mount: MountOut) -> Self {
        self.mount = mount;
        self
    }

    /// 本传感器的外参（急停换算障碍格用；语义见结构体 `mount` 字段的说明）
    pub fn Mount(&self) -> &MountOut {
        &self.mount
    }

    pub fn Slot(&self) -> &str {
        &self.slot
    }

    pub fn Params(&self) -> &LidarParams {
        &self.params
    }

    /// 一帧二进制进来：解析 → `Build_Scan` → 写 `state`（`try_write`，抢不到就丢这一帧）
    pub fn On_Binary(&self, bytes: &[u8]) {
        let f = match frame::Parse_Lidar_Frame(bytes) {
            Ok(f) => f,
            Err(e) => {
                Warn_Throttled("lidar.bad_frame", 5000, || {
                    format!("[lidar] slot={} 帧丢弃：{e}", self.slot)
                });
                return;
            }
        };

        let scan = Build_Scan(&f, &self.params);
        let n = self.frames.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 {
            info!(
                "[lidar] slot={} 首个扫描帧 points={} seq={} world_time={:.3} scan_freq={}Hz",
                self.slot,
                scan.points.len(),
                f.seq,
                f.world_time,
                scan.scan_freq
            );
        } else if n % FRAME_LOG_EVERY == 0 {
            info!(
                "[lidar] slot={} 帧={} points={} seq={} world_time={:.3}",
                self.slot,
                n,
                scan.points.len(),
                f.seq,
                f.world_time
            );
        }

        match self.state.try_write() {
            Ok(mut g) => g.scan = Some(scan),
            // 与真机同语义：抢不到就丢这一帧（建图用的是"最新一圈"，不补帧）
            Err(_) => warn!("[lidar] slot={} try_write 失败，扫描结果丢弃", self.slot),
        }
    }

    /// 最新一圈（`None` = 还没收到过合法帧）
    pub async fn Get_Scan(&self) -> Option<LaserScan> {
        self.state.read().await.scan.clone()
    }

    /// 拉起本传感器的建图 task（200 ms）。需要 `Arc` 是因为 task 要持有 `self`
    pub fn Spawn_Mapping(self: &Arc<Self>, ctx: MappingCtx) {
        let p = self.Params();
        debug!(
            "[lidar] slot={} 建图 task 启动 interval={}ms rays={} range={}m rate={}Hz",
            self.slot,
            mapping_task::MAPPING_INTERVAL_MS,
            p.rays,
            p.range,
            p.rate_hz
        );
        mapping_task::Spawn_Mapping(ctx, self.clone(), self.cancel.clone());
    }

    /// 停止本传感器持有的 task（建图 task 在下一个 tick 醒来时退出）
    pub fn Shutdown(&self) {
        self.cancel.cancel();
    }
}

impl Sensor for LidarSensor {
    // 三行都转调上面的 inherent 方法（inherent 优先，不会递归）
    fn Slot(&self) -> &str {
        &self.slot
    }

    fn On_Binary(&self, bytes: &[u8]) {
        LidarSensor::On_Binary(self, bytes)
    }

    fn Shutdown(&self) {
        LidarSensor::Shutdown(self)
    }
}

/// 帧 → `LaserScan`（纯函数，单测入口）
///
/// `angle = TAU * i / n`，`n` = **帧实际点数**；`n != params.rays` 时限流 warning 并以实际为准。
pub fn Build_Scan(f: &frame::LidarFrame, params: &LidarParams) -> LaserScan {
    let n = f.ranges.len();
    if n != params.rays {
        Warn_Throttled("lidar.rays_mismatch", 5000, || {
            format!(
                "[lidar] 帧点数 {n} 与配置 rays={} 不符：按实际点数算角度（angle = TAU*i/n），请核对世界 sensor.params",
                params.rays
            )
        });
    }

    let mut points = Vec::with_capacity(n);
    for (i, &range) in f.ranges.iter().enumerate() {
        let angle = if n == 0 { 0.0 } else { std::f32::consts::TAU * i as f32 / n as f32 };
        points.push(LaserPoint { angle, range, intensity: 0.0 });
    }

    let stamp = (f.world_time * 1e9) as u64;
    let scan_time = if params.rate_hz > 0.0 { 1.0 / params.rate_hz } else { 0.0 };

    LaserScan { points, stamp, scan_freq: params.rate_hz, scan_time }
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::sensors::lidar::frame::Test_Frame;

    fn frame_of(ranges: &[f32]) -> frame::LidarFrame {
        frame::LidarFrame { sensor_id: 1, seq: 1, world_time: 0.0, ranges: ranges.to_vec() }
    }

    #[test]
    fn test_build_scan_angles() {
        // n = 4：逐点比对 angle = TAU*i/n
        let params = LidarParams { rays: 4, ..Default::default() };
        let scan = Build_Scan(&frame_of(&[1.0, 2.0, 3.0, 4.0]), &params);
        assert_eq!(scan.points.len(), 4);
        for (i, p) in scan.points.iter().enumerate() {
            let want = std::f32::consts::TAU * i as f32 / 4.0;
            assert!((p.angle - want).abs() < 1e-6, "i={i} angle={} 期望 {want}", p.angle);
        }

        // n = 360：抽查 i = 0 / 1 / 90 / 359（角度按实际点数算，不按配置）
        let ranges = vec![0.5f32; 360];
        let scan = Build_Scan(&frame_of(&ranges), &LidarParams::default());
        assert_eq!(scan.points.len(), 360);
        for i in [0usize, 1, 90, 359] {
            let want = std::f32::consts::TAU * i as f32 / 360.0;
            assert!((scan.points[i].angle - want).abs() < 1e-6, "i={i} 角度不对");
        }
        // i=90 恰好 90°（正左，yaw=0 时 +y 方向）
        assert!((scan.points[90].angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn test_build_scan_angles_use_actual_point_count() {
        // 配置 rays=360，帧只有 4 点 → 仍按 4 点铺满一圈（否则整圈角度都错）
        let params = LidarParams { rays: 360, ..Default::default() };
        let scan = Build_Scan(&frame_of(&[1.0, 1.0, 1.0, 1.0]), &params);
        assert_eq!(scan.points.len(), 4);
        assert!((scan.points[1].angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((scan.points[2].angle - std::f32::consts::PI).abs() < 1e-6);
    }

    #[test]
    fn test_build_scan_keeps_zero_range() {
        // range == 0.0（无回波）与超近值原样保留，过滤是 mapper 的事
        let params = LidarParams { rays: 3, ..Default::default() };
        let scan = Build_Scan(&frame_of(&[0.0, 0.005, 3.0]), &params);
        assert_eq!(scan.points[0].range, 0.0);
        assert_eq!(scan.points[1].range, 0.005);
        assert_eq!(scan.points[2].range, 3.0);
        // intensity 世界不上线 → 恒 0.0
        assert!(scan.points.iter().all(|p| p.intensity == 0.0));
    }

    #[test]
    fn test_build_scan_stamp_and_scan_time() {
        let params = LidarParams { rays: 1, rate_hz: 10.0, ..Default::default() };
        let mut f = frame_of(&[1.0]);
        f.world_time = 1.234;
        let scan = Build_Scan(&f, &params);
        assert_eq!(scan.stamp, (1.234 * 1e9) as u64);
        assert_eq!(scan.scan_freq, 10.0);
        assert!((scan.scan_time - 0.1).abs() < 1e-6, "scan_time={}", scan.scan_time);
        // rate_hz <= 0 时防除零（From_Json 已挡非法值，这里是兜底）
        let scan = Build_Scan(&f, &LidarParams { rays: 1, rate_hz: 0.0, ..Default::default() });
        assert_eq!(scan.scan_time, 0.0);
    }

    #[test]
    fn test_params_from_json() {
        let p = LidarParams::From_Json(&serde_json::json!({
            "rays": 90, "range": 8.0, "rate_hz": 20
        }));
        assert_eq!(p, LidarParams { rays: 90, range: 8.0, rate_hz: 20.0 });

        // 缺键 → 默认
        assert_eq!(LidarParams::From_Json(&serde_json::json!({})), LidarParams::default());
        // 非法值（0 / 负数 / 类型不对）→ 默认，不 panic
        let p = LidarParams::From_Json(&serde_json::json!({
            "rays": 0, "range": -1.0, "rate_hz": "fast"
        }));
        assert_eq!(p, LidarParams::default());
    }

    #[tokio::test]
    async fn test_lidar_sensor_on_binary_writes_state() {
        let sensor = LidarSensor::New("lidar_front", LidarParams { rays: 3, ..Default::default() });
        assert!(sensor.Get_Scan().await.is_none(), "初始无扫描");
        assert_eq!(sensor.Slot(), "lidar_front");

        let bytes = Test_Frame(1, 9, 2.5, &[1.0, 0.0, 4.0]);
        sensor.On_Binary(&bytes);

        let scan = sensor.Get_Scan().await.expect("合法帧应写入 state");
        assert_eq!(scan.points.len(), 3);
        assert_eq!(scan.stamp, (2.5 * 1e9) as u64);
        assert_eq!(scan.points[1].range, 0.0);
        // 帧计数（首帧 + 每 50 帧日志用）
        assert_eq!(sensor.frames.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_lidar_sensor_drops_bad_frame() {
        let sensor = LidarSensor::New("lidar_front", LidarParams::default());
        // 短包：state 保持 None，不 panic
        sensor.On_Binary(&[1u8, 0, 1]);
        assert!(sensor.Get_Scan().await.is_none());
        // 非 lidar 类型
        let mut bytes = Test_Frame(1, 1, 0.0, &[1.0]);
        bytes[0] = 2;
        sensor.On_Binary(&bytes);
        assert!(sensor.Get_Scan().await.is_none());
        assert_eq!(sensor.frames.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_lidar_sensor_latest_scan_wins() {
        let sensor = LidarSensor::New("lidar_front", LidarParams { rays: 1, ..Default::default() });
        sensor.On_Binary(&Test_Frame(1, 1, 1.0, &[1.0]));
        sensor.On_Binary(&Test_Frame(1, 2, 2.0, &[2.0]));
        let scan = sensor.Get_Scan().await.expect("应有一圈");
        assert_eq!(scan.points.len(), 1);
        assert_eq!(scan.points[0].range, 2.0);
        assert_eq!(scan.stamp, (2.0 * 1e9) as u64);
    }

    #[tokio::test]
    async fn test_lidar_sensor_shutdown_is_idempotent() {
        let sensor = Arc::new(LidarSensor::New("lidar_front", LidarParams::default()));
        sensor.Shutdown();
        sensor.Shutdown();
        // 停掉后仍可读（没有 join，也不 panic）
        assert!(sensor.Get_Scan().await.is_none());
    }
}
