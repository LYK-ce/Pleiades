//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! `pose` 消息 → `RobotState` + 位姿新鲜度（task_31 §五.4.6 / §六.5.6）
//!
//! 归属：**由底盘持有**（`ChassisBase`），`RobotState` 的**唯一世界写者**
//! （§六.2.5：真机也是「底盘 RX 回调」这一个写者）。`world_link` 不直接碰本模块，
//! 一律经 `Chassis::On_Pose` / `Chassis::On_Spawn` 进来。
//!
//! 世界底盘连接 10 Hz 下发 `pose`（§三：`world_time` / `x` / `y` / `z` / `yaw` / `vx` / `vy`），
//! 本模块是「世界位姿进入 `RobotState`」的唯一入口：
//!
//! - **只写世界能给的那几个字段**：`x` / `y` / `z` / `attitude.yaw` / `vx` / `vy`
//! - `rtk_fixed` **不写**（世界没有 RTK，与真机未启用 RTK 一致，恒 `false`）
//! - `battery` / `gyro` / `accel` / `mag` / `vz` 不写、不造数
//!
//! 计龄用**本地单调钟**（`OnceLock<Instant>` 基）：世界 `world_time` 在重启后归零、
//! 且与本地墙钟不同源（task_30 已记录该坑），拿它减墙钟必然算错。
//!
//! `last_ms == 0` 哨兵 = 「从未收到 pose」→ `Age_Ms()` 返回 `u64::MAX`（永不新鲜），
//! 建图 task 因此天然具备装配门闩：世界没起 / 没装配时每周期都会跳过。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use tokio::sync::RwLock;

use pleiades_base::robot::core::state::RobotState;

/// 位姿陈旧阈值（毫秒）：超过即认为「现在没有可用位姿」，本周期不建图
pub const POSE_STALE_MS: u64 = 1000;

/// `pose` 消息里世界能给的全部字段（world_link → PoseIngest 的载体）
#[derive(Debug, Clone, Copy, Default)]
pub struct PoseFields {
    /// 世界时间（秒）：**不参与计龄**（R8），但用来把「扫描帧」对齐到当时的位姿（见 [`PoseHistory`]）
    pub world_time: f64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub vx: f32,
    pub vy: f32,
}

/// `chassis_ready.spawn`：世界按 `avatar_id` 把车摆在错开的方位，需要用 spawn 覆盖 origin
#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnFields {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
}

/// 进程内单调毫秒（`OnceLock<Instant>` 基；不用世界 `world_time`、不用墙钟）
pub fn Now_Monotonic_Ms() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// `0`（从未收到）→ `u64::MAX`（= 永不新鲜）；否则 `now - last`（饱和减）
pub fn Age_Of(last_ms: &AtomicU64) -> u64 {
    let last = last_ms.load(Ordering::Relaxed);
    if last == 0 {
        return u64::MAX;
    }
    Now_Monotonic_Ms().saturating_sub(last)
}

/// 一个位姿样本（世界时间 + 水平位姿）——建图 task 用它取「扫描那一刻」的位姿
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseSample {
    pub world_time: f64,
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
}

/// 位姿短历史（环形，最近 [`PoseHistory::CAP`] 个样本）
///
/// **为什么要有它**（task_31 Step 7 实测）：世界 10 Hz 发 `pose` 与激光帧，但两者不同步；
/// 建图 task 200 ms 跑一次、拿到的是「最新的位姿 + 上一帧扫描」。车在转向时（本用例
/// `max_turn_rate=180`、`turn_speed=10` → 18 °/s）两者相差 ~80 ms ≈ **1.4°**，
/// 于是每个命中点被转偏 `r·cosθ·Δyaw`：r=4 m 的掠射点偏 **0.10 m**（实测），
/// 足以把 `wall_c` 的南面（y=92.0，正好是格边界）写进**墙前一格**（183），
/// 并使该格在 `Unknown↔Occupied` 间翻面 → D* 的目标在南北之间翻面 → 车原地打转、S3 失败。
///
/// 取「世界时间最接近扫描 `stamp` 的样本」后，偏差从 ~0.10 m 降到 ~0.01 m 量级
/// （10 Hz 采样间隔的一半 ≈ 50 ms × 18 °/s ≈ 0.9° → r=4 m 处 0.06 m…
/// 故 [`PoseHistory::At`] 用最近邻 + 只在跨度内取，配合建图侧 2 cm 外推即可稳定量化）。
pub struct PoseHistory {
    buf: Mutex<VecDeque<PoseSample>>,
}

impl PoseHistory {
    /// 环形容量：32 × 100 ms ≈ 3.2 s，足够覆盖一帧 200 ms 建图 + 抖动
    pub const CAP: usize = 32;

    pub fn New() -> Arc<Self> {
        Arc::new(Self { buf: Mutex::new(VecDeque::with_capacity(Self::CAP)) })
    }

    /// 记录一个样本（旧样本自动淘汰）
    pub fn Push(&self, s: PoseSample) {
        // 世界重启后 world_time 归零：整体丢弃，避免新旧时间轴混在一起
        let mut buf = match self.buf.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(), // 中毒不致命：历史只是加速器
        };
        if let Some(last) = buf.back() {
            if s.world_time + 0.5 < last.world_time {
                buf.clear();
            }
        }
        if buf.len() >= Self::CAP {
            buf.pop_front();
        }
        buf.push_back(s);
    }

    /// 取世界时间最接近 `world_time` 的样本；历史为空/时间差超过 `tolerance_s` → `None`
    ///
    /// `tolerance_s` 默认取 0.15 s（1.5 个 10 Hz 周期）：超出说明这一帧扫描根本没有对应
    /// 位姿（世界刚起 / 断线重连），调用方应回退到"当前位姿"。
    pub fn At(&self, world_time: f64, tolerance_s: f64) -> Option<PoseSample> {
        let buf = match self.buf.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        buf.iter()
            .min_by(|a, b| {
                (a.world_time - world_time)
                    .abs()
                    .partial_cmp(&(b.world_time - world_time).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .filter(|s| (s.world_time - world_time).abs() <= tolerance_s)
    }

    /// 历史样本数（单测/日志用）
    #[allow(dead_code)]
    pub fn Len(&self) -> usize {
        match self.buf.lock() {
            Ok(g) => g.len(),
            Err(e) => e.into_inner().len(),
        }
    }
}

/// 位姿注入器（`RobotState` 的唯一世界写者；与 `last_ms` 同一个 `Arc` 交给建图 task）
#[derive(Clone)]
pub struct PoseIngest {
    state: Arc<RwLock<RobotState>>,
    /// 上次 pose 的单调毫秒时间戳（**0 = 从未收到**）
    last_ms: Arc<AtomicU64>,
    /// 位姿短历史（建图 task 取「扫描时刻」的位姿用）
    history: Arc<PoseHistory>,
}

impl PoseIngest {
    pub fn New(state: Arc<RwLock<RobotState>>) -> Self {
        Self {
            state,
            last_ms: Arc::new(AtomicU64::new(0)),
            history: PoseHistory::New(),
        }
    }

    /// 收到 `pose`：写 `x` / `y` / `z` / `attitude.yaw` / `vx` / `vy`，刷新新鲜度。
    ///
    /// 锁内只做字段赋值，不做别的事（R3：锁绝不跨 await）。
    pub async fn On_Pose(&self, p: &PoseFields) {
        {
            let mut s = self.state.write().await;
            s.x = p.x;
            s.y = p.y;
            s.z = p.z;
            s.attitude.yaw = p.yaw;
            s.vx = p.vx;
            s.vy = p.vy;
        }
        // 位姿历史（扫描帧按世界时间对齐用；见 `PoseHistory`）
        self.history.Push(PoseSample { world_time: p.world_time, x: p.x, y: p.y, yaw: p.yaw });
        // R9：进程第一个毫秒内 `Now_Monotonic_Ms()` 就是 0，直接存会被当成"从未收到"
        self.last_ms.store(Now_Monotonic_Ms().max(1), Ordering::Relaxed);
    }

    /// 收到 `chassis_ready`：用 spawn 覆盖 origin（`x` / `y` / `z` / `attitude.yaw`）。
    ///
    /// **不动** `vx` / `vy`（速度是上一段运动的事实）、不动新鲜度（世界还没发过 pose，
    /// 此处不能假装位姿新鲜）、不动 `rtk_fixed`。
    pub async fn On_Spawn(&self, s: &SpawnFields) {
        let mut rs = self.state.write().await;
        rs.x = s.x;
        rs.y = s.y;
        rs.z = s.z;
        rs.attitude.yaw = s.yaw;
    }

    /// 距上次 `pose` 的毫秒数（未收到过 = `u64::MAX`）
    ///
    /// 建图 task 内部走 [`Age_Of`] + `Age_Handle()`（同一 `Arc`，不另建时钟）；
    /// 本方法给「单写者自己判新鲜度」的调用方（联调日志 / Step 4 底盘）。
    #[allow(dead_code)]
    pub fn Age_Ms(&self) -> u64 {
        Age_Of(&self.last_ms)
    }

    /// 给 `MappingCtx.pose_age` 用：与 `last_ms` 是**同一个** `Arc`（不要另建时钟）
    pub fn Age_Handle(&self) -> Arc<AtomicU64> {
        self.last_ms.clone()
    }

    /// 给 `MappingCtx.pose_history` 用：与 `On_Pose` 写入的是**同一个**历史
    pub fn History_Handle(&self) -> Arc<PoseHistory> {
        self.history.clone()
    }
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fresh_state() -> Arc<RwLock<RobotState>> {
        Arc::new(RwLock::new(RobotState::default()))
    }

    fn pose(x: f32, y: f32, yaw: f32, vx: f32, vy: f32) -> PoseFields {
        PoseFields { world_time: 1.0, x, y, z: 0.0, yaw, vx, vy }
    }

    // ─── PoseHistory：按扫描 world_time 取当时位姿 ─────────────────────

    fn sample(world_time: f64, x: f32, yaw: f32) -> PoseSample {
        PoseSample { world_time, x, y: 0.0, yaw }
    }

    #[test]
    fn test_pose_history_picks_nearest_sample() {
        let h = PoseHistory::New();
        h.Push(sample(1.00, 10.0, 0.0));
        h.Push(sample(1.10, 11.0, 0.9));
        h.Push(sample(1.20, 12.0, 1.8));
        assert_eq!(h.Len(), 3);
        // 扫描时间正好落在中途 → 取最近的样本（1.10）
        let got = h.At(1.11, 0.15).expect("应能在容差内取到样本");
        assert_eq!(got.x, 11.0);
        assert!((got.yaw - 0.9).abs() < 1e-6);
        assert_eq!(h.At(1.20, 0.15).map(|s| s.x), Some(12.0));
        // 超出容差 → 没有对应位姿（调用方回退到当前位姿）
        assert_eq!(h.At(2.00, 0.15), None);
    }

    #[test]
    fn test_pose_history_caps_len() {
        let h = PoseHistory::New();
        for i in 0..(PoseHistory::CAP + 8) {
            h.Push(sample(i as f64 * 0.1, i as f32, 0.0));
        }
        assert_eq!(h.Len(), PoseHistory::CAP, "历史必须按 CAP 淘汰旧样本");
        // 最新样本仍在
        let last = (PoseHistory::CAP + 7) as f64 * 0.1;
        assert_eq!(h.At(last, 0.05).map(|s| s.x), Some((PoseHistory::CAP + 7) as f32));
    }

    #[test]
    fn test_pose_history_clears_on_world_time_reset() {
        // 世界重启后 world_time 归零：旧时间轴必须整体丢弃，否则会取到上一轮会话的位姿
        let h = PoseHistory::New();
        for i in 0..5 {
            h.Push(sample(100.0 + i as f64, i as f32, 0.0));
        }
        h.Push(sample(0.2, 99.0, 0.0));
        assert_eq!(h.Len(), 1, "时间轴回退应清空历史");
        assert_eq!(h.At(0.2, 0.15).map(|s| s.x), Some(99.0));
    }

    #[tokio::test]
    async fn test_on_pose_feeds_history() {
        let state = fresh_state();
        let ingest = PoseIngest::New(state);
        let h = ingest.History_Handle();
        ingress_pose_world_time(&ingest, 2.0).await;
        assert_eq!(h.Len(), 1);
        assert_eq!(h.At(2.0, 0.05).map(|s| s.x), Some(70.0));
    }

    async fn ingress_pose_world_time(ingest: &PoseIngest, world_time: f64) {
        ingest
            .On_Pose(&PoseFields { world_time, x: 70.0, y: 71.0, z: 0.0, yaw: 0.5, vx: 0.0, vy: 0.0 })
            .await;
    }

    #[test]
    fn test_age_never_is_max() {
        let last = AtomicU64::new(0);
        assert_eq!(Age_Of(&last), u64::MAX, "从未收到 pose 必须永不新鲜");
    }

    #[test]
    fn test_age_of_stale_over_1000() {
        // 1.2 s 之前的时间戳（进程刚起时 now < 1200 → 饱和为 0 → MAX，同样 > 1000）
        let last = AtomicU64::new(Now_Monotonic_Ms().saturating_sub(1200));
        assert!(Age_Of(&last) > POSE_STALE_MS, "1.2 s 前的位姿必须判陈旧");
        // 刚刚的时间戳 → < 阈值
        let fresh = AtomicU64::new(Now_Monotonic_Ms().max(1));
        assert!(Age_Of(&fresh) < POSE_STALE_MS, "刚更新的位姿应新鲜");
    }

    #[tokio::test]
    async fn test_on_pose_refreshes_age() {
        let state = fresh_state();
        let ingest = PoseIngest::New(state.clone());
        assert_eq!(ingest.Age_Ms(), u64::MAX, "初始应永不新鲜");

        ingest.On_Pose(&pose(66.5, 63.25, 1.5, 0.4, -0.2)).await;
        assert!(ingest.Age_Ms() < POSE_STALE_MS, "写 pose 后应新鲜");

        // 位姿字段与真机同义
        {
            let s = state.read().await;
            assert!((s.x - 66.5).abs() < 1e-6);
            assert!((s.y - 63.25).abs() < 1e-6);
            assert!((s.attitude.yaw - 1.5).abs() < 1e-6);
            assert!((s.vx - 0.4).abs() < 1e-6);
            assert!((s.vy + 0.2).abs() < 1e-6);
        }

        // 计龄随时间递增
        let a1 = ingest.Age_Ms();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let a2 = ingest.Age_Ms();
        assert!(a2 > a1, "age 应随时间递增：{a1} -> {a2}");
    }

    #[tokio::test]
    async fn test_on_pose_first_millisecond_still_fresh() {
        // R9：最后写入的是 `max(1)`，绝不会把刚收到的 pose 当成"从未收到"
        let ingest = PoseIngest::New(fresh_state());
        ingest.On_Pose(&pose(1.0, 2.0, 0.0, 0.0, 0.0)).await;
        assert_ne!(ingest.Age_Ms(), u64::MAX);
    }

    #[tokio::test]
    async fn test_on_spawn_overrides_origin() {
        let state = fresh_state();
        // base 注入的 origin（默认 64,64）+ 一段旧速度
        {
            let mut s = state.write().await;
            s.x = 64.0;
            s.y = 64.0;
            s.z = 0.0;
            s.attitude.yaw = 0.0;
            s.vx = 1.0;
            s.vy = 2.0;
        }
        let ingest = PoseIngest::New(state.clone());

        ingest.On_Spawn(&SpawnFields { x: 66.0, y: 62.0, z: 0.5, yaw: 1.25 }).await;

        let s = state.read().await;
        assert!((s.x - 66.0).abs() < 1e-6, "x 应被 spawn 覆盖：{}", s.x);
        assert!((s.y - 62.0).abs() < 1e-6, "y 应被 spawn 覆盖：{}", s.y);
        assert!((s.z - 0.5).abs() < 1e-6, "z 应被 spawn 覆盖：{}", s.z);
        assert!((s.attitude.yaw - 1.25).abs() < 1e-6, "yaw 应被 spawn 覆盖");
        // 速度不是 spawn 能给的事实，不覆盖
        assert!((s.vx - 1.0).abs() < 1e-6);
        assert!((s.vy - 2.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_on_spawn_does_not_refresh_age() {
        // 收 spawn 不等于收到位姿：新鲜度不能被"假装"刷新（装配门闩靠这个）
        let ingest = PoseIngest::New(fresh_state());
        ingest.On_Spawn(&SpawnFields { x: 64.0, y: 64.0, z: 0.0, yaw: 0.0 }).await;
        assert_eq!(ingest.Age_Ms(), u64::MAX);
    }

    #[tokio::test]
    async fn test_rtk_fixed_stays_false() {
        let state = fresh_state();
        let ingest = PoseIngest::New(state.clone());
        ingest.On_Pose(&pose(64.0, 64.0, 0.0, 0.0, 0.0)).await;
        ingest.On_Spawn(&SpawnFields { x: 66.0, y: 64.0, z: 0.0, yaw: 0.0 }).await;
        assert!(!state.read().await.rtk_fixed, "世界没有 RTK，rtk_fixed 必须保持 false");
    }

    #[tokio::test]
    async fn test_age_handle_shares_clock() {
        // MappingCtx 与 PoseIngest 必须是同一个时钟（同一 Arc），否则建图永远不新鲜
        let ingest = PoseIngest::New(fresh_state());
        let handle = ingest.Age_Handle();
        assert_eq!(Age_Of(&handle), u64::MAX);
        ingest.On_Pose(&pose(64.0, 64.0, 0.0, 0.0, 0.0)).await;
        assert!(Age_Of(&handle) < POSE_STALE_MS);
    }
}
