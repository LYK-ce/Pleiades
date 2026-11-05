//Presented by KeJi
//Created Date ： 2026-11-05
//Modified Date ： 2026-11-05

//! 值守树（task_48 §2.2）：**值守 → 出动 → 归位**，永不终值。
//!
//! ```text
//! ReactiveFallback                          ← 不是记忆式 Fallback（composite.rs:7-11 / follow.rs:28-32）
//! ├─ ReactiveSequence
//! │  ├─ ZoneGate         人在扇形警戒区内（3.0 进 / 3.5 出 / ±30°，带迟滞）→ 写本拍快照
//! │  └─ follow::build(…) 复用 Follow 子树：d > 2.2 前进 / d ≤ 1.2 停住朝人
//! └─ RepeatForever
//!    └─ Sequence [ HomeLeaf, face::build(…, 注视点) ]   ← 归位后转回注视方向
//! ```
//!
//! **本文件两份**（`pleiades-ugv` / `sim/ugv_sim`）：除 `use` 路径与文件头外**逐字相同**；机端
//! （`pleiades-uav` / `sim/uav_sim`）的 `Guard` 保持 naive（口径 #9），没有本文件。
//!
//! **警戒区绑在值守点、在世界系固定**（口径 #11）：圆心 = 值守点 `(x, y)`，朝向 = **值守点 → 注视点**
//! 的方位角（`build` 里算一次，`fan_yaw`），半角 ±30°，进半径 3.0 m / 出半径 3.5 m（口径 #13 迟滞）。
//! 它**不随车头转**：车追人时车头朝人，扇形仍在原处。
//!
//! **归位绝不冒泡 `Failure`**（口径 #15）：`HomeLeaf` 自己判到点 / 被堵，**不复用 `goto::build`**
//! （它自带 `Timeout(60 s)` → `Failure(Timeout)` → 守卫任务被队列弹走）。
//!
//! **与 Follow 子树共用同一个 `Arc<TargetSource>`**（`containment.rs:110-118` 的先例）：`TargetSource`
//! 内部是「首次见到该 `frame_seq`」的单调时钟（`follow.rs:158-203`），各建一份会出现两套 `age`。
//! 注意：`follow::build` 在内部**自建**它自己的快照槽位（`follow.rs:213`），不接受外部槽位；所以「共用」
//! 的具体形态是**共用同一个来源**，本文件的 `slot` 只是 `ZoneGate` 本拍的快照（唯一写点），
//! **不是** Follow 子树的输入。
//!
//! **Guard 固定 2.2 / 1.2**（口径 #12）：显式传 `FollowParams`，不用 `default()` —— Step 3 会把 Follow 的
//! 默认值改成 2.4 / 1.4，用了默认值就会被连带改掉。

use std::f32::consts::PI;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{
    FailCode, Frame, Node, ReactiveFallback, ReactiveSequence, RepeatForever, Sequence, Status,
};
use pleiades_base::robot::core::state::RobotState;

use crate::ugv::mission::{face, follow};
use crate::ugv::nav::{NavPort, NavStatus, NavStep};

// ---------------------------------------------------------------- 常量（全 `pub`，单测直接用）

/// 进警戒区半径（米，口径 #11）
pub const GUARD_R_IN_M: f32 = 3.0;
/// 出警戒区半径（米，口径 #13 迟滞）
pub const GUARD_R_OUT_M: f32 = 3.5;
/// 扇形半角（度，口径 #11）
pub const GUARD_HALF_ANGLE_DEG: f32 = 30.0;
/// 出动时 `d > GUARD_D_FAR_M` 才前进（米，口径 #12：固定 2.2，不跟 Follow / Containment）
pub const GUARD_D_FAR_M: f32 = 2.2;
/// 出动时 `d <= GUARD_D_NEAR_M` 停住朝人（米，口径 #12）
pub const GUARD_D_NEAR_M: f32 = 1.2;
/// 「已在值守点」判据（米）：`< 0.3` 直接 `Success`，免得每拍重算 D\*（task_48 §2.2 的 ①）
pub const GUARD_HOME_NEAR_M: f32 = 0.3;

// ---------------------------------------------------------------- 纯函数

/// 目标点是否落在「值守点的扇形」内
///
/// - `station`：值守点（世界坐标，米）= 扇形圆心
/// - `yaw`：扇形朝向（**弧度**，世界系固定 = 值守点 → 注视点的方位角；**不是**车头 yaw）
/// - `half_angle`：半角（**弧度**；`build` 里传 `GUARD_HALF_ANGLE_DEG.to_radians()`）
/// - `r_in` / `r_out`：进 / 出半径（米）；`last_in` = 上拍是否在区内（迟滞）
///
/// NaN 一律 `false`：判据写成**正向**的 `<=`（IEEE 下与 NaN 的比较恒 `false`）。
/// 反过来写成 `!(a > b)` 会把 NaN 判成「在区内」，**不要**这么写。
pub fn in_zone(
    px: f32,
    py: f32,
    station: (f32, f32),
    yaw: f32,
    half_angle: f32,
    r_in: f32,
    r_out: f32,
    last_in: bool,
) -> bool {
    let (sx, sy) = station;
    let (dx, dy) = (px - sx, py - sy);
    let d = (dx * dx + dy * dy).sqrt();
    let da = normalize_angle(dy.atan2(dx) - yaw).abs();
    // 迟滞：上拍在区内用出界半径，否则用进界半径（口径 #13：进 3.0 / 出 3.5）
    let within = if last_in { d <= r_out } else { d <= r_in };
    da <= half_angle && within
}

/// 取锁，毒化时恢复内部数据而不是二次 panic（与 `follow.rs:136-142` 同法）
fn lock<T>(m: &StdMutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// 角度归一化到 [−π, π]（与 `face.rs` / 四个 `executor.rs` 同式）
fn normalize_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

// ---------------------------------------------------------------- 建树

/// 建值守树（每次入队**新建实例**，§十一 #12）。`source` 无 feed 时 ⇒ 只值守不追（口径 #16）
pub fn build<P: NavPort + 'static>(
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<follow::TargetSource>,
    station: (f32, f32),
    gaze: (f32, f32),
) -> Box<dyn Node> {
    // 扇形朝向 = 值守点 → 注视点（世界系固定；**不是** `rs.attitude.yaw`，口径 #11）
    let fan_yaw = (gaze.1 - station.1).atan2(gaze.0 - station.0);
    // 口径 #12：显式传 2.2 / 1.2（Step 3 会把 Follow 的默认值改成 2.4 / 1.4）
    let params = follow::FollowParams {
        d_far: GUARD_D_FAR_M,
        d_near: GUARD_D_NEAR_M,
        stale_ms: follow::STALE_MS,
    };
    let slot: Arc<StdMutex<Option<follow::Sample>>> = Arc::new(StdMutex::new(None));
    let gate = Box::new(ZoneGate {
        robot_state: robot_state.clone(),
        source: source.clone(),
        slot,
        station,
        fan_yaw,
        half_angle: GUARD_HALF_ANGLE_DEG.to_radians(),
        r_in: GUARD_R_IN_M,
        r_out: GUARD_R_OUT_M,
        last_in: false,
    });
    // 与 `ZoneGate` 共用 `source`（同一份 age 追踪，两处快照不矛盾）
    let follow_tree = follow::build(robot_state.clone(), source, params);
    let home = Box::new(HomeLeaf {
        station,
        armed: false,
        nav,
        robot_state: robot_state.clone(),
        warned_blocked: false,
    });
    let gaze_leaf = face::build(robot_state, gaze);
    Box::new(ReactiveFallback::new(vec![
        Box::new(ReactiveSequence::new(vec![gate, follow_tree])),
        Box::new(RepeatForever::new(Box::new(Sequence::new(vec![home, gaze_leaf])))),
    ]))
}

// ---------------------------------------------------------------- 节点

/// 条件 0：人在扇形警戒区内（带迟滞）；**本拍唯一写快照的地方**
struct ZoneGate {
    robot_state: Arc<RwLock<RobotState>>,
    /// 与 Follow 子树**同一个**来源（同一份 age 追踪）
    source: Arc<follow::TargetSource>,
    /// 本拍快照（唯一写点；`halt` 清空）
    slot: Arc<StdMutex<Option<follow::Sample>>>,
    station: (f32, f32),
    /// 扇形朝向（弧度，世界系固定）
    fan_yaw: f32,
    half_angle: f32,
    r_in: f32,
    r_out: f32,
    /// 上拍是否在区内（迟滞状态；`halt` 归 `false`）
    last_in: bool,
}

#[async_trait]
impl Node for ZoneGate {
    async fn tick(&mut self, _f: &mut Frame<'_>) -> Status {
        // 位姿先读完再放锁（同 follow.rs / goto.rs 的纪律：不持锁跨 await）
        let me = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y)
        };
        let s = self.source.sample(Instant::now(), me);
        *lock(&self.slot) = s; // 无论有没有目标都写（本拍唯一写点）
        let snap = *lock(&self.slot);
        let inside = match snap {
            Some(sa) => in_zone(
                sa.x,
                sa.y,
                self.station,
                self.fan_yaw,
                self.half_angle,
                self.r_in,
                self.r_out,
                self.last_in,
            ),
            None => false,
        };
        self.last_in = inside;
        if inside {
            Status::Success
        } else {
            Status::Failure(FailCode::Unspecified)
        }
    }

    /// 清快照 + 迟滞（幂等）
    fn halt(&mut self) {
        *lock(&self.slot) = None;
        self.last_in = false;
    }
}

/// 归位叶子：回值守点；**永不 `Failure`**（口径 #15）
pub struct HomeLeaf<P: NavPort> {
    station: (f32, f32),
    /// 是否已把目标递给导航器；`halt` 清掉 → 下次 tick 重新武装（同 `goto.rs:76-81 / :115-117`）
    armed: bool,
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
    /// 已 warn 过「被堵」（10 Hz 不刷屏；恢复后清零）
    warned_blocked: bool,
}

#[async_trait]
impl<P: NavPort + 'static> Node for HomeLeaf<P> {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        // 位姿：先读完再放读锁（不在持读锁时等导航锁，同 goto.rs:73-75）
        let rs = self.robot_state.read().await.clone();
        // ① 已在值守点 → Success（不碰 nav：免得每拍重算 D\*）
        let (dx, dy) = (rs.x - self.station.0, rs.y - self.station.1);
        if (dx * dx + dy * dy).sqrt() < GUARD_HOME_NEAR_M {
            f.out.next_cell = None;
            self.warned_blocked = false;
            return Status::Success;
        }
        let mut nav = self.nav.lock().await;
        // ② 重新武装：未武装，或目标被清过（抢占恢复 / reset）
        if !self.armed || nav.status() == NavStatus::Idle {
            nav.set_goal(self.station.0, self.station.1);
            self.armed = true;
        }
        let step = nav.step_result(&rs).await;
        let status = nav.status();
        drop(nav);
        // ③-a 被堵：只等 / 重试，**绝不 Failure**（口径 #15）
        if let NavStatus::Blocked(cause) = status {
            if !self.warned_blocked {
                tracing::warn!(?cause, "[Guard] 归位被堵：原地等待重试（不终结任务）");
                self.warned_blocked = true;
            }
            f.out.next_cell = None;
            return Status::Running;
        }
        self.warned_blocked = false;
        // ③-b 到点
        if status == NavStatus::Arrived {
            f.out.next_cell = None;
            return Status::Success;
        }
        // ③-c 其余：这拍给下一格（`None` = 这一拍不动）
        f.out.next_cell = match step {
            NavStep::Step(cell) => Some(cell),
            NavStep::NoPath | NavStep::NotReady => None,
        };
        Status::Running
    }

    /// 只清自身状态、不发 Stop（§十一 #23）；幂等
    fn halt(&mut self) {
        self.armed = false;
        self.warned_blocked = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::f32::consts::FRAC_PI_2;

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    use crate::device::detector::Target;

    /// 值守点；注视点在正北 64 m 外 ⇒ `fan_yaw = atan2(64, 0) = +π/2`（扇形朝 +y）
    const STATION: (f32, f32) = (64.0, 64.0);
    const GAZE: (f32, f32) = (64.0, 128.0);

    // ------------------------------------------------------------ 夹具

    /// 从值守点按「世界系方位角（弧度）+ 距离（米）」取一点
    fn at_angle(station: (f32, f32), ang: f32, d: f32) -> (f32, f32) {
        (station.0 + d * ang.cos(), station.1 + d * ang.sin())
    }

    /// 合成 detector 目标（`range_m` 与「到值守点的距离」同值，选最近用）
    fn target(x: f32, y: f32, seq: u64) -> Target {
        let dx = x - STATION.0;
        let dy = y - STATION.1;
        Target {
            x,
            y,
            range_m: (dx * dx + dy * dy).sqrt(),
            bearing_deg: 0.0,
            frame_ts_ms: 0,
            frame_seq: seq,
        }
    }

    /// 假目标来源：外部改内容，`latest_targets` 同步读（同 `containment.rs` 的测试夹具）
    struct FakeFeed {
        targets: StdMutex<Vec<Target>>,
    }

    impl FakeFeed {
        fn new(targets: Vec<Target>) -> Arc<Self> {
            Arc::new(Self { targets: StdMutex::new(targets) })
        }

        fn set(&self, targets: Vec<Target>) {
            *lock(&self.targets) = targets;
        }
    }

    impl follow::TargetFeed for FakeFeed {
        fn latest_targets(&self) -> Vec<Target> {
            lock(&self.targets).clone()
        }
    }

    /// 假导航的行为：到点 / 恒被堵（`set_goal` 后锁存，同 `Navigator` 的 `Arrived` / `Blocked`）
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Arrived,
        Blocked,
    }

    struct FakeNav {
        goals: Vec<(f32, f32)>,
        mode: Mode,
        status: NavStatus,
        set_goal_calls: u32,
    }

    impl FakeNav {
        fn new(mode: Mode) -> Self {
            Self { goals: Vec::new(), mode, status: NavStatus::Idle, set_goal_calls: 0 }
        }
    }

    #[async_trait]
    impl NavPort for FakeNav {
        fn set_goal(&mut self, x: f32, y: f32) {
            self.goals.push((x, y));
            self.set_goal_calls += 1;
            self.status = match self.mode {
                Mode::Arrived => NavStatus::Navigating,
                Mode::Blocked => NavStatus::Blocked(crate::ugv::nav::BlockedCause::Dynamic),
            };
        }

        async fn step(&mut self, _rs: &RobotState) -> Option<(i32, i32)> {
            match self.mode {
                Mode::Arrived => {
                    self.status = NavStatus::Arrived;
                    None
                }
                Mode::Blocked => None,
            }
        }

        fn status(&self) -> NavStatus {
            self.status
        }
    }

    /// 一套值守树 + 位姿 / 导航句柄
    struct Rig {
        tree: Box<dyn Node>,
        state: Arc<RwLock<RobotState>>,
        nav: Arc<Mutex<FakeNav>>,
    }

    fn frame(out: &mut MotionOut) -> Frame<'_> {
        Frame { dt_ms: 50, tick_no: 0, out }
    }

    /// `feed = None` 就是「无 detector」路径（与 `mod.rs` 的 `TargetSource::new(None)` 同构）
    async fn rig(feed: Option<Arc<FakeFeed>>, me: (f32, f32), yaw_rad: f32, mode: Mode) -> Rig {
        let state = Arc::new(RwLock::new(RobotState::default()));
        {
            let mut rs = state.write().await;
            rs.x = me.0;
            rs.y = me.1;
            rs.attitude.yaw = yaw_rad;
        }
        let source =
            Arc::new(follow::TargetSource::new(feed.map(|f| f as Arc<dyn follow::TargetFeed>)));
        let nav = Arc::new(Mutex::new(FakeNav::new(mode)));
        let tree = build(nav.clone(), state.clone(), source, STATION, GAZE);
        Rig { tree, state, nav }
    }

    // ------------------------------------------------------------ 纯函数 `in_zone`

    /// 区内 / 区外 / 角度外（29.9° 进、31° 出；**不压 30.0 边界**，避 f32 误差）
    #[test]
    fn test_guard_in_zone_dist_and_angle() {
        let half = GUARD_HALF_ANGLE_DEG.to_radians();
        // 正北 2.5 m → 区内
        assert!(
            in_zone(64.0, 66.5, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, false),
            "正北 2.5 m（进半径内、方位角 0）必须在区内"
        );
        // 正北 6 m → 太远
        assert!(
            !in_zone(64.0, 70.0, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, false),
            "正北 6 m 超出进半径 → 区外"
        );
        // 正南 2.5 m → 角度外
        assert!(
            !in_zone(64.0, 61.5, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, false),
            "正南（方位角差 180°）→ 角度外"
        );
        // 正北偏 31° → 角度外（2.5 m，距离在区内）
        let out31 = at_angle(STATION, FRAC_PI_2 + 31f32.to_radians(), 2.5);
        assert!(
            !in_zone(out31.0, out31.1, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, false),
            "偏 31° 超出 ±30° 半角 → 区外"
        );
        // 正北偏 29.9° → 区内
        let in299 = at_angle(STATION, FRAC_PI_2 + 29.9f32.to_radians(), 2.5);
        assert!(
            in_zone(in299.0, in299.1, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, false),
            "偏 29.9° 仍在 ±30° 半角内 → 区内"
        );
    }

    /// 迟滞两向：出区用 3.5、进区用 3.0（口径 #13）
    #[test]
    fn test_guard_in_zone_hysteresis_two_ways() {
        let half = GUARD_HALF_ANGLE_DEG.to_radians();
        let p32 = (64.0, 67.2); // 3.2 m（正北）
        let p36 = (64.0, 67.6); // 3.6 m（正北）
        let p30 = (64.0, 67.0); // 3.0 m（正北）
        // 上拍在区内：3.2 m 仍留在区里，3.6 m 才掉出去
        assert!(
            in_zone(p32.0, p32.1, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, true),
            "last_in=true：3.2 m ≤ 出界半径 3.5 → 仍算区内"
        );
        assert!(
            !in_zone(p36.0, p36.1, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, true),
            "last_in=true：3.6 m > 出界半径 3.5 → 区外"
        );
        // 上拍在区外：3.2 m 进不来，3.0 m 才进来
        assert!(
            !in_zone(p32.0, p32.1, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, false),
            "last_in=false：3.2 m > 进界半径 3.0 → 仍算区外"
        );
        assert!(
            in_zone(p30.0, p30.1, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, false),
            "last_in=false：3.0 m ≤ 进界半径 3.0 → 进区"
        );
    }

    /// NaN 一律 `false`（判据写成正向 `<=`；`last_in` 两向都验）
    #[test]
    fn test_guard_in_zone_nan_is_false() {
        let half = GUARD_HALF_ANGLE_DEG.to_radians();
        let nan = f32::NAN;
        for last_in in [false, true] {
            assert!(
                !in_zone(nan, 66.5, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, last_in),
                "px = NaN（last_in={last_in}）必须 false"
            );
            assert!(
                !in_zone(64.0, nan, STATION, FRAC_PI_2, half, GUARD_R_IN_M, GUARD_R_OUT_M, last_in),
                "py = NaN（last_in={last_in}）必须 false"
            );
            assert!(
                !in_zone(64.0, 66.5, STATION, nan, half, GUARD_R_IN_M, GUARD_R_OUT_M, last_in),
                "yaw = NaN（last_in={last_in}）必须 false"
            );
        }
    }

    // ------------------------------------------------------------ 树级

    /// 值守：人在扇形**外**（正南 3 m）→ 只值守（`face_point` = 注视点），三拍 `Running`
    /// 车在值守点、车头 yaw = 0（与扇形朝向不同，扇形仍指正北）
    #[tokio::test]
    async fn test_guard_watch_writes_face_point() {
        let feed = FakeFeed::new(vec![target(64.0, 61.0, 1)]); // 正南 3 m：看得见但不在警戒区
        let mut r = rig(Some(feed.clone()), STATION, 0.0, Mode::Arrived).await;
        let mut out = MotionOut::default();
        for i in 0..3 {
            let mut f = frame(&mut out);
            assert_eq!(r.tree.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running（永不终值）");
            assert_eq!(out.face_point, Some(GAZE), "第 {i} 拍：值守 = 朝注视点");
            assert_eq!(out.target_point, None, "第 {i} 拍：区外的人不追");
            assert_eq!(out.next_cell, None, "第 {i} 拍：不巡逻、不走格");
        }
        // 人一直在区外 → 一直只值守
        feed.set(vec![target(64.0, 61.0, 2)]);
        let mut f = frame(&mut out);
        assert_eq!(r.tree.tick(&mut f).await, Status::Running);
        assert_eq!(out.face_point, Some(GAZE), "人换帧但仍在区外 → 仍只值守");
        assert_eq!(out.target_point, None);
    }

    /// 无 detector（`[detector].enabled=false`）→ **只值守不追**，三拍 `Running`（绝不一拍 `Success`）
    #[tokio::test]
    async fn test_guard_watch_without_detector() {
        let mut r = rig(None, STATION, 0.0, Mode::Arrived).await;
        let mut out = MotionOut::default();
        for i in 0..3 {
            let mut f = frame(&mut out);
            assert_eq!(
                r.tree.tick(&mut f).await,
                Status::Running,
                "第 {i} 拍必须 Running（一拍 Success 会把守卫任务弹走）"
            );
            assert_eq!(out.face_point, Some(GAZE), "第 {i} 拍：只值守（朝注视点）");
            assert_eq!(out.target_point, None);
            assert_eq!(out.next_cell, None);
        }
    }

    /// 出动：人在正北 2.5 m（扇形内、`d > 2.2`）→ `target_point` = 人的坐标；
    /// 把车头转 180°（−π/2 即朝南）连 2 拍仍进区 ⇒ **扇形不随车头转**（口径 #11）
    #[tokio::test]
    async fn test_guard_intruder_writes_target_point() {
        let mut r = rig(Some(FakeFeed::new(vec![target(64.0, 66.5, 1)])), STATION, 0.0, Mode::Arrived)
            .await;
        let mut out = MotionOut::default();
        let mut f = frame(&mut out);
        assert_eq!(r.tree.tick(&mut f).await, Status::Running, "出动也永不终值");
        assert_eq!(out.target_point, Some((64.0, 66.5)), "进区 → 朝人开（人的世界坐标）");
        assert_eq!(out.face_point, None, "前进这拍不给朝向（face > point 会顶掉点）");
        assert_eq!(out.next_cell, None, "不经 D*：不给格");
        // 车头转 180°：扇形绑值守点、世界系固定 → 仍判进区
        {
            let mut rs = r.state.write().await;
            rs.attitude.yaw = -FRAC_PI_2;
        }
        for i in 0..2 {
            let mut f = frame(&mut out);
            assert_eq!(r.tree.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(
                out.target_point,
                Some((64.0, 66.5)),
                "第 {i} 拍：车头朝南也仍进区（扇形不随车头转）"
            );
            assert_eq!(out.face_point, None);
        }
    }

    /// 归位：车在 (64, 59)（距值守点 5 m）、无人 → `nav.set_goal(值守点)` + 到点 → 转回注视方向
    #[tokio::test]
    async fn test_guard_home_writes_face_point() {
        let mut r = rig(None, (64.0, 59.0), 0.0, Mode::Arrived).await;
        let mut out = MotionOut::default();
        let mut f = frame(&mut out);
        assert_eq!(r.tree.tick(&mut f).await, Status::Running, "归位中必须 Running");
        assert_eq!(out.face_point, Some(GAZE), "到点后朝注视点");
        assert_eq!(out.next_cell, None, "归位到点这拍不给格");
        assert_eq!(out.target_point, None, "归位不给点（走的是 D* 的格）");
        let nav = r.nav.lock().await;
        assert_eq!(nav.goals, vec![(64.0, 64.0)], "必须回值守点");
        assert_eq!(nav.set_goal_calls, 1, "同一目标只武装一次");
    }

    /// 恒 `Blocked`：连续 10 拍全 `Running`（**绝不 `Failure`**，口径 #15）；
    /// `halt` 幂等并复位 `armed`（再 tick 一拍会重新 `set_goal`）
    #[tokio::test]
    async fn test_guard_blocked_never_fails_and_halt_idempotent() {
        let mut r = rig(None, (64.0, 59.0), 0.0, Mode::Blocked).await;
        let mut out = MotionOut::default();
        for i in 0..10 {
            let mut f = frame(&mut out);
            assert_eq!(
                r.tree.tick(&mut f).await,
                Status::Running,
                "第 {i} 拍被堵必须 Running（Failure 会让队列把守卫任务弹走）"
            );
            assert_eq!(out.next_cell, None, "第 {i} 拍：被堵这拍不动");
        }
        assert_eq!(r.nav.lock().await.set_goal_calls, 1, "被堵 10 拍只武装一次");
        // halt 幂等 + 复位 `armed`
        r.tree.halt();
        r.tree.halt();
        let mut f = frame(&mut out);
        assert_eq!(r.tree.tick(&mut f).await, Status::Running, "halt 后仍 Running（持久任务）");
        assert_eq!(
            r.nav.lock().await.set_goal_calls,
            2,
            "halt 复位 armed ⇒ 下一拍重新 set_goal"
        );
    }
}
