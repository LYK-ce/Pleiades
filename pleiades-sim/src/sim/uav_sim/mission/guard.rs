//Presented by KeJi
//Created Date ： 2026-11-08
//Modified Date ： 2026-11-08

//! 值守树（sim 机栈）：**原地值守 → 人进扇区即出动 → 一次触发后锁存跟随**，整棵树永不终值。
//!
//! ```text
//! ReactiveFallback                                  ← 反应式：条件每拍重判（记忆式 Fallback 会锁死分支）
//! ├─ ReactiveSequence
//! │  ├─ ZoneGate          目标在扇形警戒区内（心 = 值守点，朝向 = 值守点→注视点，±15°，≤ 10 m）
//! │  │                    条件成功那拍发协助请求（首次 / 目标相对上次发送点移动 > 1 m）
//! │  └─ follow::build(…)  复用 Follow 子树（TargetGate / Step / Hold，`FollowParams::default()`）
//! └─ HoldStation          无人进区：**原地朝注视点**（只写 `face_point`），`Running`
//! ```
//!
//! **锁存（2026-11-08 人类确认）**：`ZoneGate` 首次满足后 `latched = true` 恒真 —— 人走出扇区
//! 也**不回值守点、不切回 `HoldStation`**，交给 Follow 子树继续跟（这个演示就是要一直跟着）。
//! 目标丢失时 Follow 的 `HoldLeaf` 三个字段都 `None` ⇒ 原地停住，同样**不回位、不终值**。
//! **整棵树永不 `Success`**：`Guard` 是持续任务，`Success` 会让任务队列把这条弹走。
//! 也不返回 `Err` / `Failure`：那等于清队列 + 停车（`ReactiveFallback` 的两个分支都返 `Running`）。
//!
//! **与真机的关系**：`pleiades-uav` 的 `Guard` 仍是 naive 分支（一拍 `Success` + warn），本文件
//! 只服务 sim；协议 `type 5` 不变（`x, y` = 值守点，`fx, fy` = 注视点）。
//!
//! **为什么外层必须 `ReactiveFallback`**：本仓 `Fallback` 是**记忆式**（`composite.rs`），而
//! `follow::build` 的叶子恒返回 `Running`（持续任务）。记忆式会永久锁在 `HoldStation` 分支，
//! 人进区也不再重判；`ReactiveFallback` 每拍从第 0 个子重判，切分支时 `halt` 上一拍的分支。
//!
//! **为什么扇形绑值守点、绑世界系**：扇形朝向是 **值守点 → 注视点** 的世界方位角（`build` 里算一次
//! `fan_yaw`），**不随** `rs.attitude.yaw` 转 —— 无人机自己转头（或 Follow 里写 `face_point` 朝人）
//! 都不影响警戒区。因此判定用世界 `atan2(dy, dx)` 与 `fan_yaw` 比，而不是读机头 yaw。
//!
//! **协助请求**（assist）：只由 `ZoneGate` 发（与 `containment.rs` 的 `InsideSquare` 同款副作用）。
//! 进区第一拍发一条；锁存后目标每移动 > [`ASSIST_MOVE_M`]（1 m，相对**上次发送点**）再发一条，
//! 让 4 台车拿到新的环点。目标丢失/过期**不重置锁存**，也不重发（不追旧点）；`publisher = None`
//! 时只判条件、一条都不发（单测 / 无网络）。
//!
//! **本文件不引 `tracing`**：10 Hz tick 加日志会刷屏（与 `follow.rs` / `containment.rs` 同纪律）。
//! 本树**不持 `nav`**：`HoldStation` 与 Follow 都不经 D\*，没有 `set_goal`，也不给 `next_cell`。

use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::RwLock;

use pleiades_base::robot::core::behavior_tree::{
    FailCode, Frame, Node, ReactiveFallback, ReactiveSequence, Status,
};
use pleiades_base::robot::core::state::RobotState;

use crate::sim::uav_sim::mission::containment::AssistPublisher;
use crate::sim::uav_sim::mission::follow;

// ---------------------------------------------------------------- 常量（全 `pub`，单测直接用）

/// 进警戒区半径（米）：目标到**值守点**的距离 ≤ 该值才算「进区」（无出区迟滞：锁存后本来就不再判）
pub const GUARD_R_IN_M: f32 = 10.0;
/// 扇形半角（度）：目标方位与「值守点 → 注视点」方位之差 ≤ 该值（±15°）才算「进区」
pub const GUARD_HALF_ANGLE_DEG: f32 = 15.0;
/// 协助请求重发阈值（米）：目标相对**上次发送点**移动超过该值才再发一次
/// （与 `containment::ASSIST_MOVE_M` 同值 1.0；两棵树各自持有自己的门槛，语义独立）
pub const ASSIST_MOVE_M: f32 = 1.0;

// ---------------------------------------------------------------- 纯函数

/// 角度归一化到 [−π, π]（与 `face.rs` / `follow.rs` / 四个 `executor.rs` 同式）
fn normalize_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

/// 目标点是否落在「值守点的扇形」内（纯几何，**不含**新鲜度判定）
///
/// - `station`：值守点（世界坐标，米）= 扇形圆心
/// - `fan_yaw`：扇形朝向（**弧度**，世界系固定 = 值守点 → 注视点的方位角；**不是**机头 yaw）
/// - `half_angle`：半角（**弧度**；`build` 里传 `GUARD_HALF_ANGLE_DEG.to_radians()`）
/// - `r_in`：进区半径（米）
///
/// NaN 一律 `false`：判据写成**正向**的 `<=`（IEEE 下与 NaN 的比较恒 `false`）。
/// 反过来写成 `!(a > b)` 会把 NaN 判成「在区内」，**不要**这么写。
/// 目标与值守点重合（`d = 0`）时 `atan2(0, 0) = 0`，通常不等于 `fan_yaw` ⇒ 算区外
/// （扇形在圆心处没有方向可言，这是可接受的退化）。
pub fn in_zone(px: f32, py: f32, station: (f32, f32), fan_yaw: f32, half_angle: f32, r_in: f32) -> bool {
    let (sx, sy) = station;
    let (dx, dy) = (px - sx, py - sy);
    let d = (dx * dx + dy * dy).sqrt();
    let da = normalize_angle(dy.atan2(dx) - fan_yaw).abs();
    d <= r_in && da <= half_angle
}

/// 「本拍有入侵者」= 有目标 && 未过期（`!follow::lost`）&& 在扇形内
///
/// 与 `ZoneGate` 的判据**同一个函数**（单测直接测它，不碰树）。
pub fn intruder(
    t: Option<&follow::Sample>,
    station: (f32, f32),
    fan_yaw: f32,
    half_angle: f32,
    r_in: f32,
    p: &follow::FollowParams,
) -> bool {
    match t {
        None => false,
        Some(s) => !follow::lost(Some(s), p) && in_zone(s.x, s.y, station, fan_yaw, half_angle, r_in),
    }
}

// ---------------------------------------------------------------- 建树

/// 建值守树（每次入队**新建实例**，不复用节点对象，§十一 #12）
///
/// `publisher = None` → 只判条件、不发协助请求（单测 / 无网络时）。
/// `source` 无 feed（无 detector）时 ⇒ `sample` 恒 `None` ⇒ 门恒失败 ⇒ **只值守不追**，
/// 但树仍是同一棵（永不 `Success`）；`mod.rs` 的无 detector 路径另走 [`hold_station`]（等价叶子）。
pub fn build(
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<follow::TargetSource>,
    station: (f32, f32),
    gaze: (f32, f32),
    publisher: Option<Arc<dyn AssistPublisher>>,
) -> Box<dyn Node> {
    // 扇形朝向 = 值守点 → 注视点（世界系固定；**不是** `rs.attitude.yaw`）
    let fan_yaw = (gaze.1 - station.1).atan2(gaze.0 - station.0);
    let params = follow::FollowParams::default();
    let gate = Box::new(ZoneGate {
        robot_state: robot_state.clone(),
        source: source.clone(),
        station,
        fan_yaw,
        half_angle: GUARD_HALF_ANGLE_DEG.to_radians(),
        r_in: GUARD_R_IN_M,
        params,
        latched: false,
        publisher,
        last_sent: None,
    });
    // 与 `ZoneGate` 共用 `source`（同一份 age 追踪，两处快照不矛盾）
    let follow_tree = follow::build(robot_state, source, params);
    Box::new(ReactiveFallback::new(vec![
        Box::new(ReactiveSequence::new(vec![gate, follow_tree])),
        hold_station(gaze),
    ]))
}

/// 「只值守」的叶子树：每拍只写 `face_point = gaze`，恒 `Running`（**不得** `Success`）
///
/// 无 detector 时由 `mod.rs` 直接使用（不必为一棵恒失败的门建整棵树）。
pub fn hold_station(gaze: (f32, f32)) -> Box<dyn Node> {
    Box::new(HoldStation { gaze })
}

// ---------------------------------------------------------------- 节点

/// 条件 0：本拍目标是「入侵者」；首次满足后**锁存**，之后恒 `Success`
///
/// 副作用：条件成功且（首次或目标相对上次发送点移动 > [`ASSIST_MOVE_M`]）→ `publisher.publish(x, y)`
/// 并记下发送点。锁存后即使目标走出扇形，只要目标仍新鲜就继续按 1 m 阈值续发（4 台车要新环点）。
///
/// 不写任何**共享槽位**：Follow 子树里的 `TargetGate` 自己取快照（两处共用 `source`，
/// 取到的是同一帧、同一份 age 口径）。
struct ZoneGate {
    robot_state: Arc<RwLock<RobotState>>,
    /// 与 Follow 子树**同一个**来源（同一份 age 追踪）
    source: Arc<follow::TargetSource>,
    /// 值守点（扇形圆心，世界坐标，米）
    station: (f32, f32),
    /// 扇形朝向（弧度，世界系固定）
    fan_yaw: f32,
    /// 半角（弧度）
    half_angle: f32,
    /// 进区半径（米）
    r_in: f32,
    params: follow::FollowParams,
    /// 一次触发后锁存（口径：人离开扇区也不回值守点、不停 Follow）
    latched: bool,
    /// 协助请求发布者（`None` = 不发，只判条件；单测可注入记录型假实现）
    publisher: Option<Arc<dyn AssistPublisher>>,
    /// 上次**已发出**的目标点（世界坐标）；`None` = 还没发过
    last_sent: Option<(f32, f32)>,
}

impl ZoneGate {
    /// 按 1 m 阈值发协助请求（同步、非阻塞；坐标非有限不发也不记账）
    fn assist(&mut self, s: &follow::Sample) {
        if !s.x.is_finite() || !s.y.is_finite() {
            return;
        }
        // 阈值针对**上次发送点**（不是逐拍），与 `containment::InsideSquare` 同口径
        let need = match self.last_sent {
            None => true,
            Some((lx, ly)) => {
                let (dx, dy) = (s.x - lx, s.y - ly);
                dx * dx + dy * dy > ASSIST_MOVE_M * ASSIST_MOVE_M
            }
        };
        if !need {
            return;
        }
        // 先 clone 出 `Arc`：`publish` 期间不借 `self` 的字段（避免 `&mut self` 冲突）
        let Some(publisher) = self.publisher.clone() else { return };
        publisher.publish(s.x, s.y);
        self.last_sent = Some((s.x, s.y));
    }
}

#[async_trait]
impl Node for ZoneGate {
    async fn tick(&mut self, _f: &mut Frame<'_>) -> Status {
        // 位姿先读完再放锁（同 `follow.rs` / `goto.rs` 的纪律：不持锁跨 await）
        let me = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y)
        };
        let s = self.source.sample(Instant::now(), me);
        // 已锁存：条件恒成功；目标仍新鲜就按 1 m 阈值续发协助（丢失不重置锁存、也不追旧点）
        if self.latched {
            if let Some(sample) = s.as_ref() {
                if !follow::lost(Some(sample), &self.params) {
                    self.assist(sample);
                }
            }
            return Status::Success;
        }
        if !intruder(
            s.as_ref(),
            self.station,
            self.fan_yaw,
            self.half_angle,
            self.r_in,
            &self.params,
        ) {
            return Status::Failure(FailCode::Unspecified);
        }
        // 首次进区：锁存 + 立即发一条（`last_sent == None` ⇒ `assist` 必发）
        self.latched = true;
        if let Some(sample) = s.as_ref() {
            self.assist(sample);
        }
        Status::Success
    }

    /// **不**复位锁存（口径：一次触发后恒跟随）；`halt` 幂等
    fn halt(&mut self) {}
}

/// 值守叶子：原地朝注视点（**只转不走**），恒 `Running`
///
/// 不写 `target_point` / `next_cell`：无人机原地不动、不走 D\*、不巡逻。
/// 写 `face_point` 的同一拍必须把另两个字段清掉 —— `MotionOut::target()` 的优先级是
/// **face > point > cell**（`behavior_tree/frame.rs:42-50`），上一拍的残留会顶掉朝向。
pub struct HoldStation {
    /// 注视点（世界坐标，米）
    gaze: (f32, f32),
}

#[async_trait]
impl Node for HoldStation {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        f.out.face_point = Some(self.gaze);
        f.out.target_point = None;
        f.out.next_cell = None;
        // 持续任务：**不终值**（`Success` 会让队列把值守弹走）
        Status::Running
    }

    fn halt(&mut self) {} // 无状态
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::f32::consts::FRAC_PI_2;
    use std::sync::Mutex as StdMutex;

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    use crate::sim::detector::Target;
    use crate::sim::uav_sim::mission::follow::{FollowParams, TargetFeed, TargetSource, STALE_MS};

    /// 值守点；注视点在正北 64 m 外 ⇒ `fan_yaw = atan2(64, 0) = +π/2`（扇形朝 +y）
    const STATION: (f32, f32) = (64.0, 64.0);
    const GAZE: (f32, f32) = (64.0, 128.0);
    /// 扇形朝向（世界系固定，正北）
    const FAN_YAW: f32 = FRAC_PI_2;

    // ------------------------------------------------------------ 夹具

    /// 从值守点按「世界系方位角（弧度）+ 距离（米）」取一点
    fn at_angle(station: (f32, f32), ang: f32, d: f32) -> (f32, f32) {
        (station.0 + d * ang.cos(), station.1 + d * ang.sin())
    }

    /// 合成 detector 目标（`range_m` = 到值守点的距离，选最近用）
    fn target(x: f32, y: f32, seq: u64) -> Target {
        let (dx, dy) = (x - STATION.0, y - STATION.1);
        Target {
            x,
            y,
            range_m: (dx * dx + dy * dy).sqrt(),
            bearing_deg: 0.0,
            frame_ts_ms: 0,
            frame_seq: seq,
        }
    }

    /// 合成一拍快照（纯函数只读 `x` / `y` / `age_ms`）
    fn sample(x: f32, y: f32, age_ms: u64) -> follow::Sample {
        follow::Sample { x, y, d: 0.0, age_ms, frame_seq: 1, range_m: 0.0 }
    }

    /// 假目标来源：外部改内容，`latest_targets` 同步读（`follow.rs` 的夹具是私有 test 项，
    /// 本文件自己写一份 —— `follow.rs` 冻结，不许改）
    ///
    /// 用 `std::sync::Mutex`（不是 `tokio` 的）：`TargetFeed::latest_targets` 是**同步**端口，不能 `await`。
    struct FakeFeed {
        targets: StdMutex<Vec<Target>>,
    }

    impl FakeFeed {
        fn new(targets: Vec<Target>) -> Arc<Self> {
            Arc::new(Self { targets: StdMutex::new(targets) })
        }

        /// 换目标集（**同步**：tick 之外调用，不与 tick 竞争）
        fn set(&self, targets: Vec<Target>) {
            *lock(&self.targets) = targets;
        }
    }

    impl TargetFeed for FakeFeed {
        fn latest_targets(&self) -> Vec<Target> {
            lock(&self.targets).clone()
        }
    }

    /// 取锁，毒化时恢复内部数据而不是二次 panic（与 `follow.rs` 同法）
    fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
        match m.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// 记录型假发布者（`publish` 只 append；`Mutex` 以便跨 await / 外部读）
    #[derive(Default)]
    struct FakePublisher {
        sent: StdMutex<Vec<(f32, f32)>>,
    }

    impl FakePublisher {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn sent(&self) -> Vec<(f32, f32)> {
            lock(&self.sent).clone()
        }

        fn count(&self) -> usize {
            lock(&self.sent).len()
        }
    }

    impl AssistPublisher for FakePublisher {
        fn publish(&self, x: f32, y: f32) {
            lock(&self.sent).push((x, y));
        }
    }

    /// `feed = None` 就是「无 detector」路径（与 `mod.rs` 的 `TargetSource::new(None)` 同构）
    async fn make_tree(
        feed: Option<Arc<FakeFeed>>,
        me: (f32, f32),
        publisher: Option<Arc<dyn AssistPublisher>>,
    ) -> Box<dyn Node> {
        let state = Arc::new(RwLock::new(RobotState::default()));
        {
            let mut rs = state.write().await;
            rs.x = me.0;
            rs.y = me.1;
        }
        let source =
            Arc::new(TargetSource::new(feed.map(|f| f as Arc<dyn TargetFeed>)));
        build(state, source, STATION, GAZE, publisher)
    }

    fn frame(out: &mut MotionOut) -> Frame<'_> {
        Frame { dt_ms: 50, tick_no: 0, out }
    }

    // ------------------------------------------------------------ 常量 / 纯函数

    /// 阈值写死：进区 10 m、±15°、assist 重发 1 m（2026-11-08 口径，单测钉死防误改）
    #[test]
    fn test_guard_constants() {
        assert_eq!(GUARD_R_IN_M, 10.0, "进区半径写死 10 m");
        assert_eq!(GUARD_HALF_ANGLE_DEG, 15.0, "扇形半角写死 ±15°");
        assert_eq!(ASSIST_MOVE_M, 1.0, "assist 重发阈值写死 1 m");
    }

    /// 距离 / 角度两个边界 + 圆心退化 + NaN（不压 15.0 边界，避 f32 误差）
    #[test]
    fn test_in_zone_distance_angle_and_nan() {
        let half = GUARD_HALF_ANGLE_DEG.to_radians();
        // 正北 10 m（恰在进界上）→ 区内；10.001 m → 区外
        assert!(
            in_zone(64.0, 74.0, STATION, FAN_YAW, half, GUARD_R_IN_M),
            "正北 10 m（d == r_in）必须在区内"
        );
        assert!(
            !in_zone(64.0, 74.001, STATION, FAN_YAW, half, GUARD_R_IN_M),
            "正北 10.001 m 超出进区半径 → 区外"
        );
        // 正东 5 m：距离在区内但方位差 90° → 区外
        assert!(
            !in_zone(69.0, 64.0, STATION, FAN_YAW, half, GUARD_R_IN_M),
            "正东（方位差 90°）→ 角度外"
        );
        // 正北偏 14.9° → 区内；偏 15.1° → 区外
        let in149 = at_angle(STATION, FAN_YAW + 14.9f32.to_radians(), 5.0);
        assert!(
            in_zone(in149.0, in149.1, STATION, FAN_YAW, half, GUARD_R_IN_M),
            "偏 14.9° 仍在 ±15° 半角内 → 区内"
        );
        let out151 = at_angle(STATION, FAN_YAW + 15.1f32.to_radians(), 5.0);
        assert!(
            !in_zone(out151.0, out151.1, STATION, FAN_YAW, half, GUARD_R_IN_M),
            "偏 15.1° 超出 ±15° 半角 → 区外"
        );
        // 目标与值守点重合：atan2(0, 0) = 0 ≠ fan_yaw ⇒ 区外（圆心处无方向可言）
        assert!(
            !in_zone(64.0, 64.0, STATION, FAN_YAW, half, GUARD_R_IN_M),
            "与该点重合 → 区外"
        );
        // NaN 一律 false（每个参数各验一次）
        let nan = f32::NAN;
        assert!(!in_zone(nan, 72.0, STATION, FAN_YAW, half, GUARD_R_IN_M), "px=NaN");
        assert!(!in_zone(64.0, nan, STATION, FAN_YAW, half, GUARD_R_IN_M), "py=NaN");
        assert!(!in_zone(64.0, 72.0, STATION, nan, half, GUARD_R_IN_M), "fan_yaw=NaN");
        assert!(!in_zone(64.0, 72.0, STATION, FAN_YAW, nan, GUARD_R_IN_M), "half=NaN");
        assert!(!in_zone(64.0, 72.0, STATION, FAN_YAW, half, nan), "r_in=NaN");
    }

    /// 「入侵者」= 有目标 && 未过期 && 在扇形内（`age` 用合成值，**不 sleep**）
    #[test]
    fn test_intruder_requires_fresh_and_in_zone() {
        let p = FollowParams::default();
        let half = GUARD_HALF_ANGLE_DEG.to_radians();
        assert!(!intruder(None, STATION, FAN_YAW, half, GUARD_R_IN_M, &p), "无目标 → 不是入侵者");
        assert!(
            intruder(
                Some(&sample(64.0, 72.0, STALE_MS)),
                STATION,
                FAN_YAW,
                half,
                GUARD_R_IN_M,
                &p
            ),
            "age == stale 不过期 → 是入侵者"
        );
        assert!(
            !intruder(
                Some(&sample(64.0, 72.0, STALE_MS + 1)),
                STATION,
                FAN_YAW,
                half,
                GUARD_R_IN_M,
                &p
            ),
            "age > stale → 过期 → 不是入侵者"
        );
        assert!(
            !intruder(Some(&sample(64.0, 56.0, 0)), STATION, FAN_YAW, half, GUARD_R_IN_M, &p),
            "正南（扇形外）→ 不是入侵者"
        );
        assert!(
            !intruder(Some(&sample(64.0, 80.0, 0)), STATION, FAN_YAW, half, GUARD_R_IN_M, &p),
            "16 m（超出进区半径）→ 不是入侵者"
        );
    }

    // ------------------------------------------------------------ 树级：值守 / HoldStation

    /// 无人（空目标集）→ 原地值守：`face_point = 注视点`、不追、**不发 assist**，恒 `Running`
    /// 有人但**没进扇区**（正南 8 m）→ 同样只值守
    #[tokio::test]
    async fn test_guard_watch_holds_without_intruder() {
        let feed = FakeFeed::new(vec![]);
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let mut tree = make_tree(Some(feed.clone()), STATION, Some(publisher)).await;
        let mut out = MotionOut::default();

        // ① 真正无人：只值守、一条 assist 都不发
        for i in 0..2 {
            let mut f = frame(&mut out);
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍（无人）必须 Running");
            assert_eq!(out.face_point, Some(GAZE), "第 {i} 拍（无人）：原地朝注视点");
            assert_eq!(out.target_point, None, "第 {i} 拍（无人）：不追");
            assert_eq!(out.next_cell, None, "第 {i} 拍（无人）：不巡逻、不走格");
        }
        assert_eq!(fake.count(), 0, "无人 → 一条 assist 都不发");

        // ② 有人但人在扇形外（正南 8 m）：仍只值守
        feed.set(vec![target(64.0, 56.0, 1)]);
        for i in 0..3 {
            let mut f = frame(&mut out);
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍（区外）必须 Running");
            assert_eq!(out.face_point, Some(GAZE), "第 {i} 拍（区外）：原地朝注视点");
            assert_eq!(out.target_point, None, "第 {i} 拍（区外）：不追");
            assert_eq!(out.next_cell, None, "第 {i} 拍（区外）：不巡逻、不走格");
        }
        // 人换帧但仍在区外 → 仍只值守
        feed.set(vec![target(64.0, 56.0, 2)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(out.face_point, Some(GAZE), "换帧但仍在区外 → 仍只值守");
        assert_eq!(out.target_point, None);
        assert_eq!(fake.count(), 0, "人没进扇区 → 一条 assist 都不发");
    }

    /// 无 detector（`TargetSource` 无 feed）→ 只值守：朝注视点、恒 `Running`（绝不一拍 `Success`）
    #[tokio::test]
    async fn test_guard_watch_without_detector() {
        let mut tree = make_tree(None, STATION, None).await;
        let mut out = MotionOut::default();
        for i in 0..3 {
            let mut f = frame(&mut out);
            assert_eq!(
                tree.tick(&mut f).await,
                Status::Running,
                "第 {i} 拍必须 Running（一拍 Success 会把守卫任务弹走）"
            );
            assert_eq!(out.face_point, Some(GAZE), "第 {i} 拍：只值守（朝注视点）");
            assert_eq!(out.target_point, None);
            assert_eq!(out.next_cell, None);
        }
    }

    /// `hold_station`（`mod.rs` 无 detector 路径用的叶子）：只写 `face_point`，恒 `Running`
    #[tokio::test]
    async fn test_hold_station_leaf_writes_only_face_point() {
        let mut leaf = hold_station(GAZE);
        let mut out = MotionOut::default();
        for i in 0..3 {
            let mut f = frame(&mut out);
            assert_eq!(leaf.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(out.face_point, Some(GAZE), "第 {i} 拍：朝注视点");
            assert_eq!(out.target_point, None, "第 {i} 拍：不给点");
            assert_eq!(out.next_cell, None, "第 {i} 拍：不给格");
        }
    }

    // ------------------------------------------------------------ 树级：出动 / assist / 锁存

    /// 人进扇区 → 立刻切 Follow（`target_point` = 人的世界坐标）+ assist 发 1 次；人不动不重发
    #[tokio::test]
    async fn test_guard_intruder_starts_follow_and_assist_once() {
        let feed = FakeFeed::new(vec![target(64.0, 72.0, 1)]);
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let mut tree = make_tree(Some(feed.clone()), STATION, Some(publisher)).await;
        let mut out = MotionOut::default();

        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running, "出动也永不终值");
        assert_eq!(out.target_point, Some((64.0, 72.0)), "进区 → 追上人（人的世界坐标）");
        assert_eq!(out.face_point, None, "前进这拍不给朝向（face > point 会顶掉点）");
        assert_eq!(out.next_cell, None, "不经 D*：不给格");
        assert_eq!(fake.sent(), vec![(64.0, 72.0)], "进区第一拍必须发一条 assist");

        // 人换帧但位置没变 → 不再发
        feed.set(vec![target(64.0, 72.0, 2)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(out.target_point, Some((64.0, 72.0)));
        assert_eq!(fake.count(), 1, "人没动 → 不重发");
    }

    /// assist 重发：相对上次发送点 ≤ 1 m 不发，> 1 m 发；每次发送都刷新发送点
    #[tokio::test]
    async fn test_guard_assist_resends_after_one_meter() {
        let feed = FakeFeed::new(vec![target(64.0, 72.0, 1)]);
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let mut tree = make_tree(Some(feed.clone()), STATION, Some(publisher)).await;
        let mut out = MotionOut::default();

        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(fake.count(), 1, "首拍已发");

        // 相对上次发送点 0.9 m → 不发
        feed.set(vec![target(64.9, 72.0, 2)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(fake.count(), 1, "0.9 m 不重发");

        // 相对上次发送点恰 1.0 m（阈值是**严格** > 1 m）→ 不发
        feed.set(vec![target(64.0, 73.0, 3)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(fake.count(), 1, "1.0 m 不重发（严格大于）");

        // 相对上次发送点 1.1 m → 发，且刷新发送点
        feed.set(vec![target(64.0, 73.1, 4)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(fake.count(), 2, "1.1 m 必须重发");
        assert_eq!(fake.sent()[1], (64.0, 73.1), "发的是目标当前坐标");

        // 相对**新**发送点再 1.1 m → 再发
        feed.set(vec![target(64.0, 74.2, 5)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(fake.count(), 3, "发送点已被刷新");
    }

    /// 锁存：人走出扇区仍 Follow（不给回位朝向、不 Success）；再丢失 → 三字段全 `None`；
    /// 人回来（仍在扇区外）→ 继续 Follow（锁存不因丢失 / 出区复位）
    #[tokio::test]
    async fn test_guard_latches_after_person_leaves_sector() {
        let feed = FakeFeed::new(vec![target(64.0, 70.5, 1)]); // 正北 6.5 m：扇形内且 d > 3.6
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let mut tree = make_tree(Some(feed.clone()), STATION, Some(publisher)).await;
        let mut out = MotionOut::default();

        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(out.target_point, Some((64.0, 70.5)), "进区 → 跟");
        assert_eq!(fake.count(), 1, "进区第一拍发一条");

        // 人绕到正南 6.5 m（扇形外）：锁存 ⇒ 仍 Follow，不回值守点
        feed.set(vec![target(64.0, 57.5, 2)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running, "锁存后仍不得终值");
        assert_eq!(out.target_point, Some((64.0, 57.5)), "人出扇区仍 Follow（锁存）");
        assert_eq!(out.face_point, None, "不得切回值守朝向（不回值守点）");
        assert_ne!(out.target_point, Some(STATION), "目标点不得是值守点");
        assert_eq!(fake.count(), 2, "移动 13 m > 1 m → 续发一条");

        // 目标彻底丢失：Follow 的 Hold 叶子 ⇒ 三个字段都 None，仍 Running（不回位、不 Success）
        feed.set(vec![]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running, "丢失也不得终值");
        assert_eq!(out.target_point, None, "丢失不追旧点");
        assert_eq!(out.face_point, None, "丢失也不回值守点（保持原地）");
        assert_eq!(out.next_cell, None);
        assert_eq!(fake.count(), 2, "丢失那拍不发");
        let mut f = frame(&mut out);
        assert_ne!(tree.tick(&mut f).await, Status::Success, "整棵树永不 Success");

        // 人回来（仍在扇区外，同一坐标）：锁存未复位 → 继续 Follow
        feed.set(vec![target(64.0, 57.5, 3)]);
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(out.target_point, Some((64.0, 57.5)), "人回来仍直接 Follow（锁存保持）");
        assert_eq!(out.face_point, None);
        assert_eq!(fake.count(), 2, "位置与上次发送点相同 → 不重发");
    }

    /// 无 publisher：进区照走 Follow，但一条都不发（`publisher = None` 只判条件）
    #[tokio::test]
    async fn test_guard_assist_none_publisher_never_sends() {
        let feed = FakeFeed::new(vec![target(64.0, 72.0, 1)]);
        let mut tree = make_tree(Some(feed.clone()), STATION, None).await;
        let mut out = MotionOut::default();
        for i in 0..3u32 {
            if i > 0 {
                // 每拍移动 2 m（> 1 m 阈值）：有 publisher 必发，None 时必不发
                feed.set(vec![target(64.0 + 2.0 * i as f32, 72.0, u64::from(i) + 1)]);
            }
            let mut f = frame(&mut out);
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍仍在跟随（不终值）");
        }
        assert_eq!(out.target_point, Some((68.0, 72.0)), "第 3 拍仍在 Follow");
    }

    /// 整棵树**永不 Success**：无人 40 拍（值守）+ 人进区 30 拍（跟随）+ 丢失 30 拍（原地）全 `Running`
    #[tokio::test]
    async fn test_guard_never_succeeds_for_100_ticks() {
        let feed = FakeFeed::new(vec![]);
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let mut tree = make_tree(Some(feed.clone()), STATION, Some(publisher)).await;
        let mut out = MotionOut::default();

        // ① 无人 40 拍：原地值守（朝注视点）
        for i in 0..40 {
            let mut f = frame(&mut out);
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍（无人）不得终值");
            assert_eq!(out.face_point, Some(GAZE), "第 {i} 拍：值守朝注视点");
            assert_eq!(out.target_point, None, "第 {i} 拍：无人不追");
        }
        // ② 人进区 30 拍：跟随（累计移动 0.29 m < 1 m，只发首条）
        for i in 0..30 {
            feed.set(vec![target(64.0, 72.0 + 0.01 * i as f32, i as u64 + 1)]);
            let mut f = frame(&mut out);
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍（跟随）不得终值");
            assert!(out.target_point.is_some(), "第 {i} 拍：跟随给人坐标");
        }
        // ③ 人丢失 30 拍：Follow 的 Hold（不回值守点、不终值）
        feed.set(vec![]);
        for i in 0..30 {
            let mut f = frame(&mut out);
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍（丢失）不得终值");
            assert_eq!(out.target_point, None, "第 {i} 拍：丢失不追旧点");
            assert_eq!(out.face_point, None, "第 {i} 拍：丢失也不回值守点");
        }
        assert_eq!(fake.count(), 1, "只在进区第一拍发一次（30 拍累计移动 0.29 m < 1 m）");
    }
}
