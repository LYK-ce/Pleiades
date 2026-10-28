//Presented by KeJi
//Created Date ： 2026-10-28
//Modified Date ： 2026-10-28

//! 跟随树（task_37 §五）：sim **机栈独有**，真机 / 车栈收到 Follow 只建平凡树。
//!
//! ```text
//! ReactiveFallback                          ← 不是记忆式 Fallback（见下）
//! ├─ ReactiveSequence
//! │  ├─ TargetGate         有目标（age ≤ stale）→ 写本拍快照
//! │  ├─ Condition          未太近（d > d_min）
//! │  └─ FollowLeaf         太远（d > d_max）才（重）设 goal → nav.step → Frame.out.next_cell
//! └─ HoldLeaf              丢掉 / 太近 → nav.clear_goal() + next_cell = None（不追旧点）
//! ```
//!
//! **为什么外层用 `ReactiveFallback` 而不是 §五.4 画字面的 `Fallback`**：本仓 `Fallback` 是
//! **记忆式**（`composite.rs`），子节点 `Running` 时记住下标、下拍不回头。`HoldLeaf` 必须返回
//! `Running`（Follow 是持续任务，一旦 `Success` 队列就弹走这条），于是记忆式 `Fallback` 会
//! **永久锁在 Hold 分支**——人回来也不再跟。`ReactiveFallback` 每拍从第 0 个子重判，符合
//! `composite.rs` 的仓内规则「带条件的循环一律用反应式」。
//!
//! **`age` 的时钟口径**：`Target.frame_ts_ms` 在 sim 是**世界时间**，没有「当前世界时间」可比，
//! 所以这里用本地单调 `Instant` 记「首次见到该 `frame_seq` 的时刻」，之后算 `now - last_seen`。
//! 相机停帧 / 人出画（`latest_targets()` 空）→ 直接 `Lost`。
//!
//! **人作为动态障碍**：机栈的 `Navigator` 是 `ObstacleMode::Ignore`（喂空栅格、`dynamic_obstacles`
//! 恒空），`mark_obstacle()` 的标记会被同一拍重算抹掉 → **无法真的注入 D\***。本文件复用
//! `nav/cluster_obstacles.rs` 的圆盘膨胀算出人的膨胀格，把「下一步落在人格里」的 `next_cell`
//! 否决掉（[`guard_cell`]）。`nav/` 一行不改。

use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{
    Condition, FailCode, Frame, Node, ReactiveFallback, ReactiveSequence, Status,
};
use pleiades_base::robot::core::cluster::ClusterInfo;
use pleiades_base::robot::core::state::RobotState;

use crate::sim::detector::{DetectorDevice, Target};
use crate::sim::uav_sim::nav::{cluster_to_obstacle_cells, NavPort, NavStatus};

// 本文件**不引 `tracing`**：设目标已有 `[Nav] 新目标` 日志（`nav/navigator.rs`），
// 动作已有 `[Sim] 位置=… 动作=…` 日志（`sim_handler.rs`），人类端到端验证够用；
// 再加日志只会刷屏（10 Hz tick）。

// ---------------------------------------------------------------- 参数初值

/// 停在人身前多远（米）
pub const FOLLOW_DIST_M: f32 = 2.5;
/// 距离带下沿（米）：`d <= d_min` → 停（太近，不追）
pub const D_MIN_M: f32 = 1.5;
/// 距离带上沿（米）：`d > d_max` 才重设目标（带内只沿用，迟滞）
pub const D_MAX_M: f32 = 3.0;
/// 目标过期时间（毫秒）：`age > STALE_MS` → 视为丢失（§五.5）
pub const STALE_MS: u64 = 800;
/// 目标点重设阈值（米）：目标点挪不超过它就不重建 D*（§五.5「避免每拍重建」）
pub const GOAL_UPDATE_M: f32 = 0.5;
/// 人作为障碍的圆盘膨胀半径（米）：1 格（0.5 m），与 `cluster_obstacle_cells` 同口径
pub const PERSON_INFLATE_M: f32 = 0.5;

/// 跟随参数（纯值 + `Default`；生产用默认值，单测可改小/改大）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FollowParams {
    pub follow_dist: f32,
    pub d_min: f32,
    pub d_max: f32,
    pub stale_ms: u64,
    pub goal_update_m: f32,
    pub person_inflate_m: f32,
}

impl Default for FollowParams {
    fn default() -> Self {
        Self {
            follow_dist: FOLLOW_DIST_M,
            d_min: D_MIN_M,
            d_max: D_MAX_M,
            stale_ms: STALE_MS,
            goal_update_m: GOAL_UPDATE_M,
            person_inflate_m: PERSON_INFLATE_M,
        }
    }
}

/// 一拍的目标快照（`d` 与 detector 结果**同一次**算出，两个条件不各自再算）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
    /// 与机体原点的水平距离（米）
    pub d: f32,
    /// 距「首次见到该 `frame_seq`」的本地单调毫秒
    pub age_ms: u64,
    /// detector 帧号（换帧 → `age` 归零）
    pub frame_seq: u64,
    /// detector 给的距离（选最近目标用；与 `d` 口径不同：`range_m` 来自投影）
    pub range_m: f32,
}

/// 判定结论（[`judge`] 的返回值；**纯值**，单测主战场）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 无目标 / age > stale → 停，且不追旧点
    Lost,
    /// `d <= d_min` → 停（太近）
    TooClose,
    /// `d_min < d <= d_max` → 距离带内：**不重设目标**，只沿用已武装的
    InBand,
    /// `d > d_max` → （重）设目标
    Chase,
}

// ---------------------------------------------------------------- 纯函数

/// 本拍结论（**纯函数**，不碰时钟 / 不碰导航）
pub fn judge(t: Option<&Sample>, p: &FollowParams) -> Outcome {
    let Some(s) = t else { return Outcome::Lost };
    if s.age_ms > p.stale_ms {
        return Outcome::Lost; // 严格大于：与 §五.5「超过即视为丢失」一致
    }
    if s.d <= p.d_min {
        return Outcome::TooClose; // 「未太近」= d > d_min（严格大于）
    }
    if s.d > p.d_max {
        return Outcome::Chase; // 太远才追；带内只沿用已武装目标（迟滞）
    }
    Outcome::InBand
}

/// `goal = P - dir * follow_dist`（`dir` = normalize(P - me)）；`me == P`（d≈0）时原样返回 P
pub fn goal_on_line(me: (f32, f32), target: (f32, f32), follow_dist: f32) -> (f32, f32) {
    let (dx, dy) = (target.0 - me.0, target.1 - me.1);
    let d = (dx * dx + dy * dy).sqrt();
    if d < 1e-3 {
        return target;
    }
    (target.0 - dx / d * follow_dist, target.1 - dy / d * follow_dist)
}

/// 人 → 动态障碍格：**复用** `nav/cluster_obstacles.rs` 的圆盘膨胀（不改 `nav/`）
pub fn person_cells(person: (f32, f32), radius_m: f32) -> Vec<(i32, i32)> {
    let info = ClusterInfo {
        peer_id: Vec::new(),
        x: person.0,
        y: person.1,
        z: 0.0,
        yaw: 0.0,
        vx: 0.0,
        vy: 0.0,
        time_boot_ms: 0,
        last_seen: Instant::now(),
        sub_target: None,
    };
    let mut cells: Vec<(i32, i32)> =
        cluster_to_obstacle_cells(std::slice::from_ref(&info), radius_m).into_iter().collect();
    cells.sort_unstable(); // 确定性（`HashSet` 迭代序不定 → 单测断言要排序）
    cells
}

/// 下一步落在人的膨胀格里 → 本拍不动（`None`）；否则原样返回
pub fn guard_cell(
    cell: Option<(i32, i32)>,
    person: (f32, f32),
    radius_m: f32,
) -> Option<(i32, i32)> {
    let cell = cell?;
    if person_cells(person, radius_m).contains(&cell) {
        return None;
    }
    Some(cell)
}

/// 取锁，毒化时恢复内部数据而不是二次 panic（与 `detector/mod.rs` 同法）
fn lock<T>(m: &StdMutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// ---------------------------------------------------------------- 目标来源

/// detector 的薄端口（与 `detector::FrameSource` 同款思路：让单测不需要真模型）
pub trait TargetFeed: Send + Sync {
    /// 最新一帧通过保护的目标（同步、非阻塞；无 → 空 `Vec`）
    fn latest_targets(&self) -> Vec<Target>;
}

impl TargetFeed for DetectorDevice {
    fn latest_targets(&self) -> Vec<Target> {
        DetectorDevice::latest_targets(self) // 显式指名，避免与自身无限递归
    }
}

/// 年龄追踪（`frame_seq` 变化 → 归零；`frame_seq` 不变 → 单调累积）
#[derive(Debug, Default)]
struct AgeTrack {
    last_seq: Option<u64>,
    last_seen: Option<Instant>,
}

/// 目标来源：detector 最新目标 + **本地单调** age 追踪（不碰 `frame_ts_ms`）
pub struct TargetSource {
    feed: Option<Arc<dyn TargetFeed>>,
    age: StdMutex<AgeTrack>,
}

impl TargetSource {
    pub fn new(feed: Option<Arc<dyn TargetFeed>>) -> Self {
        Self { feed, age: StdMutex::new(AgeTrack::default()) }
    }

    /// 取最新目标（最近的一个）；`now` 由调用方给（单测可传合成时刻，**不 sleep**）
    pub fn sample(&self, now: Instant, me: (f32, f32)) -> Option<Sample> {
        let feed = self.feed.as_ref()?;
        let targets = feed.latest_targets();
        // 多目标 → 取最近（§五.7「不做多目标」；`range_m` 相同则取先出现的，稳定）
        let t = targets
            .iter()
            .min_by(|a, b| a.range_m.partial_cmp(&b.range_m).unwrap_or(std::cmp::Ordering::Equal))?;
        let mut track = lock(&self.age);
        if track.last_seq != Some(t.frame_seq) {
            track.last_seq = Some(t.frame_seq);
            track.last_seen = Some(now);
        }
        let age_ms = track
            .last_seen
            .map(|at| now.saturating_duration_since(at).as_millis() as u64)
            .unwrap_or(0);
        let (dx, dy) = (t.x - me.0, t.y - me.1);
        Some(Sample {
            x: t.x,
            y: t.y,
            d: (dx * dx + dy * dy).sqrt(),
            age_ms,
            frame_seq: t.frame_seq,
            range_m: t.range_m,
        })
    }
}

// ---------------------------------------------------------------- 节点

/// 跟随需要的导航端口：`NavPort` + 一个「清目标」
///
/// 为什么不直接用 `NavPort`：`nav/mod.rs` 的 `NavPort` 只有 `set_goal` / `step` / `status`，
/// 而 `Hold` 必须把目标清掉（否则 `status` 一直 `Navigating`、`nav_status` 观测骗人）；
/// `pleiades-sim/src/sim/uav_sim/nav/` 不在本 step 允许改动路径，故在 mission 层加这个薄 trait。
pub trait FollowNav: NavPort {
    fn clear_goal(&mut self);
}

impl FollowNav for crate::sim::uav_sim::nav::Navigator {
    fn clear_goal(&mut self) {
        crate::sim::uav_sim::nav::Navigator::clear_goal(self)
    }
}

/// 建树（每次入队新建实例，§十一 #12）
pub fn build<P: FollowNav + 'static>(
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<TargetSource>,
    params: FollowParams,
) -> Box<dyn Node> {
    let slot: Arc<StdMutex<Option<Sample>>> = Arc::new(StdMutex::new(None));
    let slot_for_cond = slot.clone();
    let p2 = params;
    Box::new(ReactiveFallback::new(vec![
        // 主路：有目标 → 未太近 → 跟
        Box::new(ReactiveSequence::new(vec![
            Box::new(TargetGate::new(robot_state.clone(), source, slot.clone(), params)),
            Box::new(Condition::new(move || {
                // 与 `TargetGate` 共用同一份槽位快照 → 两个条件不会各算一次、不会互相矛盾
                matches!(
                    judge(lock(&slot_for_cond).as_ref(), &p2),
                    Outcome::InBand | Outcome::Chase
                )
            })),
            Box::new(FollowLeaf::new(nav.clone(), robot_state, slot, params)),
        ])),
        // 兜底：丢掉 / 太近 → 停（清 nav 目标，`next_cell=None`）
        Box::new(HoldLeaf::new(nav)),
    ]))
}

/// 条件 0：取本拍目标快照并判定「有目标且未过期」；**本拍唯一写快照的地方**
struct TargetGate {
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<TargetSource>,
    slot: Arc<StdMutex<Option<Sample>>>,
    params: FollowParams,
}

impl TargetGate {
    fn new(
        robot_state: Arc<RwLock<RobotState>>,
        source: Arc<TargetSource>,
        slot: Arc<StdMutex<Option<Sample>>>,
        params: FollowParams,
    ) -> Self {
        Self { robot_state, source, slot, params }
    }
}

#[async_trait]
impl Node for TargetGate {
    async fn tick(&mut self, _f: &mut Frame<'_>) -> Status {
        let me = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y)
        };
        let s = self.source.sample(Instant::now(), me);
        let out = judge(s.as_ref(), &self.params);
        *lock(&self.slot) = s; // 无论 Lost 与否都写；`Condition` / `FollowLeaf` 读同一份
        match out {
            Outcome::Lost => Status::Failure(FailCode::Unspecified),
            _ => Status::Success,
        }
    }

    fn halt(&mut self) {
        *lock(&self.slot) = None;
    }
}

/// 叶子：太远才（重）设目标 → 算下一格 → 写 `Frame.out`
pub struct FollowLeaf<P: FollowNav> {
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
    slot: Arc<StdMutex<Option<Sample>>>,
    params: FollowParams,
    /// 是否已把目标递给导航器；`halt` 清掉 → 下次 tick 重新武装
    armed: bool,
    /// 上一次**真正**递给导航器的目标点（0.5 m 节流用）
    last_goal: Option<(f32, f32)>,
}

impl<P: FollowNav> FollowLeaf<P> {
    fn new(
        nav: Arc<Mutex<P>>,
        robot_state: Arc<RwLock<RobotState>>,
        slot: Arc<StdMutex<Option<Sample>>>,
        params: FollowParams,
    ) -> Self {
        Self { nav, robot_state, slot, params, armed: false, last_goal: None }
    }
}

#[async_trait]
impl<P: FollowNav + 'static> Node for FollowLeaf<P> {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        let rs = self.robot_state.read().await.clone(); // 位姿先读完再放锁（同 goto.rs）
        let snap = *lock(&self.slot);
        let Some(s) = snap else {
            f.out.next_cell = None;
            return Status::Running;
        };
        let mut nav = self.nav.lock().await;

        // 迟滞（§五.5「距离带 + 迟滞，太近停、太远追」）：只有 `Chase` 才（重）设目标
        if judge(Some(&s), &self.params) == Outcome::Chase {
            let goal = goal_on_line((rs.x, rs.y), (s.x, s.y), self.params.follow_dist);
            let moved = self.last_goal.map_or(true, |g| {
                ((g.0 - goal.0).powi(2) + (g.1 - goal.1).powi(2)).sqrt()
                    > self.params.goal_update_m
            });
            // 未武装 / 目标被清过（抢占恢复、Hold 清过）/ 目标点挪超阈值 → 重设
            if !self.armed || nav.status() == NavStatus::Idle || moved {
                nav.set_goal(goal.0, goal.1);
                self.armed = true;
                self.last_goal = Some(goal);
            }
        }
        let cell = nav.step(&rs).await;
        // 人作为动态障碍：下一步落在人的膨胀格里 → 本拍否决
        f.out.next_cell = guard_cell(cell, (s.x, s.y), self.params.person_inflate_m);
        Status::Running
    }

    /// 只清自身状态、不发 Stop（§十一 #23）；幂等
    fn halt(&mut self) {
        self.armed = false;
        self.last_goal = None;
    }
}

/// 兜底叶子：丢掉 / 太近 → 清导航目标 + 本拍不动
pub struct HoldLeaf<P: FollowNav> {
    nav: Arc<Mutex<P>>,
}

impl<P: FollowNav> HoldLeaf<P> {
    fn new(nav: Arc<Mutex<P>>) -> Self {
        Self { nav }
    }
}

#[async_trait]
impl<P: FollowNav + 'static> Node for HoldLeaf<P> {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        // 丢掉/太近：清 nav 目标（**不追最后一个点**），本拍不动。
        // 返回 **`Running`**（不是 `Success`）：Follow 是持续任务，只有 TASK_SET 替换 /
        // `mission_count = 0` 才结束；`Success` 会让队列提前弹走这条任务。
        self.nav.lock().await.clear_goal();
        f.out.next_cell = None;
        Status::Running
    }

    fn halt(&mut self) {} // 无状态
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU32, Ordering};

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    // ------------------------------------------------------------ 夹具

    /// 合成时刻：`t0 + ms`（不 sleep）
    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + std::time::Duration::from_millis(ms)
    }

    /// 合成 detector 帧（`range_m` 与 `d` 同值，除非另有说明）
    fn target(x: f32, y: f32, seq: u64) -> Target {
        Target {
            x,
            y,
            range_m: (x * x + y * y).sqrt(),
            bearing_deg: 0.0,
            frame_ts_ms: 0,
            frame_seq: seq,
        }
    }

    fn sample(d: f32, age_ms: u64) -> Sample {
        Sample { x: 0.0, y: d, d, age_ms, frame_seq: 1, range_m: d }
    }

    /// 假目标来源：外部改内容，`latest_targets` 同步读
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

    impl TargetFeed for FakeFeed {
        fn latest_targets(&self) -> Vec<Target> {
            lock(&self.targets).clone()
        }
    }

    /// 假导航端口：`set_goal` / `clear_goal` 计数 + 可控 `next_cell`
    struct FakeNav {
        status: NavStatus,
        next_cell: Option<(i32, i32)>,
        set_goal_calls: Arc<AtomicU32>,
        clear_goal_calls: Arc<AtomicU32>,
    }

    impl FakeNav {
        fn new(next_cell: Option<(i32, i32)>) -> Arc<Mutex<Self>> {
            Arc::new(Mutex::new(Self {
                status: NavStatus::Idle,
                next_cell,
                set_goal_calls: Arc::new(AtomicU32::new(0)),
                clear_goal_calls: Arc::new(AtomicU32::new(0)),
            }))
        }
    }

    #[async_trait]
    impl NavPort for FakeNav {
        fn set_goal(&mut self, _x: f32, _y: f32) {
            self.set_goal_calls.fetch_add(1, Ordering::SeqCst);
            self.status = NavStatus::Navigating;
        }

        async fn step(&mut self, _rs: &RobotState) -> Option<(i32, i32)> {
            // 真 `Navigator` 在 Idle 下不给路：假端口照抄，才能验「带内不重设 → 不动」
            match self.status {
                NavStatus::Navigating => self.next_cell,
                _ => None,
            }
        }

        fn status(&self) -> NavStatus {
            self.status
        }
    }

    impl FollowNav for FakeNav {
        fn clear_goal(&mut self) {
            self.clear_goal_calls.fetch_add(1, Ordering::SeqCst);
            self.status = NavStatus::Idle;
        }
    }

    async fn set_pose(state: &Arc<RwLock<RobotState>>, x: f32, y: f32) {
        let mut rs = state.write().await;
        rs.x = x;
        rs.y = y;
    }

    /// 建一棵树 + 返回它的两个计数器（`Arc` 已 clone 出来，不受 `Box<dyn Node>` 影响）
    async fn tree(
        feed: Arc<FakeFeed>,
        nav: Arc<Mutex<FakeNav>>,
        params: FollowParams,
    ) -> (Box<dyn Node>, Arc<AtomicU32>, Arc<AtomicU32>) {
        let state = Arc::new(RwLock::new(RobotState::default()));
        set_pose(&state, 0.0, 0.0).await;
        let (set_calls, clear_calls) = {
            let n = nav.lock().await;
            (n.set_goal_calls.clone(), n.clear_goal_calls.clone())
        };
        let source = Arc::new(TargetSource::new(Some(feed as Arc<dyn TargetFeed>)));
        (build(nav, state, source, params), set_calls, clear_calls)
    }

    fn frame(out: &mut MotionOut) -> Frame<'_> {
        Frame { dt_ms: 50, tick_no: 0, out }
    }

    // ------------------------------------------------------------ 纯函数

    /// C6：`judge` 四种结论 + 边界（含 stale 严格大于、`d_min`/`d_max` 两端）
    #[test]
    fn test_judge_all_outcomes() {
        let p = FollowParams::default();
        assert_eq!(judge(None, &p), Outcome::Lost, "无目标 → Lost");
        // stale 边界：> 800 才丢
        assert_eq!(judge(Some(&sample(5.0, 800)), &p), Outcome::Chase, "age=800 不过期");
        assert_eq!(judge(Some(&sample(5.0, 801)), &p), Outcome::Lost, "age=801 过期");
        // d_min 边界：<= 1.5 太近
        assert_eq!(judge(Some(&sample(1.4, 0)), &p), Outcome::TooClose);
        assert_eq!(judge(Some(&sample(1.5, 0)), &p), Outcome::TooClose, "d == d_min → 太近");
        assert_eq!(judge(Some(&sample(1.6, 0)), &p), Outcome::InBand);
        // d_max 边界：> 3.0 才追
        assert_eq!(judge(Some(&sample(3.0, 0)), &p), Outcome::InBand, "d == d_max → 带内");
        assert_eq!(judge(Some(&sample(3.1, 0)), &p), Outcome::Chase);
    }

    /// C7：`goal_on_line` = P - dir * follow_dist
    #[test]
    fn test_goal_on_line() {
        let (gx, gy) = goal_on_line((0.0, 0.0), (0.0, 5.0), FOLLOW_DIST_M);
        assert!((gx - 0.0).abs() < 1e-5 && (gy - 2.5).abs() < 1e-5, "实际 ({gx}, {gy})");
        // me=(1,1) P=(4,5)：d=5、dir=(0.6,0.8) → (2.5, 3.0)
        let (gx, gy) = goal_on_line((1.0, 1.0), (4.0, 5.0), FOLLOW_DIST_M);
        assert!((gx - 2.5).abs() < 1e-5, "x 应为 2.5，实际 {gx}");
        assert!((gy - 3.0).abs() < 1e-5, "y 应为 3.0，实际 {gy}");
        // me == P（d≈0）：原样返回 P，不产生 NaN
        assert_eq!(goal_on_line((3.0, 3.0), (3.0, 3.0), FOLLOW_DIST_M), (3.0, 3.0));
    }

    /// 人的膨胀格 + `guard_cell` 否决
    #[test]
    fn test_person_cells_and_guard() {
        let cells = person_cells((2.0, 3.0), PERSON_INFLATE_M);
        assert!(cells.contains(&(4, 6)), "人所在格 (4,6) 必在集合里：{cells:?}");
        assert!(cells.windows(2).all(|w| w[0] < w[1]), "必须排序（确定性）");
        // 下一步落在人格 → 否决
        assert_eq!(guard_cell(Some((4, 6)), (2.0, 3.0), PERSON_INFLATE_M), None);
        // 远离人格 → 原样
        assert_eq!(guard_cell(Some((20, 20)), (2.0, 3.0), PERSON_INFLATE_M), Some((20, 20)));
        // 本来就没路 → 仍是 None
        assert_eq!(guard_cell(None, (2.0, 3.0), PERSON_INFLATE_M), None);
    }

    // ------------------------------------------------------------ 目标来源

    /// C8：`TargetSource::sample` 的 age / 选最近 / 空帧（合成时刻，不 sleep）
    #[test]
    fn test_sample_age_and_nearest() {
        let t0 = Instant::now();
        let feed = FakeFeed::new(vec![target(0.0, 5.0, 7)]);
        let src = TargetSource::new(Some(feed.clone() as Arc<dyn TargetFeed>));

        // 首次见到 seq=7 → age 0；d 由 me 算
        let s = src.sample(t0, (0.0, 0.0)).expect("有目标");
        assert_eq!((s.age_ms, s.d), (0, 5.0));
        // 同一 frame_seq 过 500 ms → age 500（不 sleep）
        let s = src.sample(at(t0, 500), (0.0, 0.0)).expect("有目标");
        assert_eq!(s.age_ms, 500);
        // 换帧 seq=8 → age 归零
        feed.set(vec![target(0.0, 5.0, 8)]);
        let s = src.sample(at(t0, 900), (0.0, 0.0)).expect("有目标");
        assert_eq!(s.age_ms, 0, "换帧即归零");
        // age 超 stale → judge 判 Lost
        let s = src.sample(at(t0, 1701), (0.0, 0.0)).expect("有目标");
        assert_eq!(s.age_ms, 801);
        assert_eq!(judge(Some(&s), &FollowParams::default()), Outcome::Lost);
        // 空帧 → None
        feed.set(vec![]);
        assert!(src.sample(at(t0, 1800), (0.0, 0.0)).is_none());
        // 无 feed（未启用 detector）→ None
        assert!(TargetSource::new(None).sample(t0, (0.0, 0.0)).is_none());
    }

    /// 多目标取最近（`range_m` 小的那个）
    #[test]
    fn test_sample_picks_nearest() {
        let t0 = Instant::now();
        let mut far = target(0.0, 9.0, 1);
        far.range_m = 9.0;
        let mut near = target(0.0, 4.0, 1);
        near.range_m = 4.0;
        let feed = FakeFeed::new(vec![far, near]);
        let src = TargetSource::new(Some(feed as Arc<dyn TargetFeed>));
        let s = src.sample(t0, (0.0, 0.0)).expect("有目标");
        assert_eq!(s.y, 4.0, "应取 range_m 最小的目标");
    }

    // ------------------------------------------------------------ 树级

    /// C9-1：`d=5`（Chase）→ `set_goal` 恰一次、本拍有下一格、持续 `Running`
    #[tokio::test]
    async fn test_tree_chase_sets_goal() {
        let feed = FakeFeed::new(vec![target(0.0, 5.0, 1)]);
        let nav = FakeNav::new(Some((3, 4)));
        let (mut t, set_calls, _) = tree(feed, nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 1, "Chase 首拍应武装一次");
        assert_eq!(out.next_cell, Some((3, 4)));
        // 第二拍：目标没挪 → 不重设
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 1, "目标没挪不得重设");
    }

    /// C9-2：无目标 → 不追、`clear_goal`、连拍 3 次仍 `Running`（**不终值**）
    #[tokio::test]
    async fn test_tree_lost_holds_without_terminating() {
        let feed = FakeFeed::new(vec![]);
        let nav = FakeNav::new(Some((3, 4)));
        let (mut t, set_calls, clear_calls) = tree(feed, nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        for i in 0..3 {
            assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(out.next_cell, None, "丢失时不得给出下一格");
        }
        assert_eq!(set_calls.load(Ordering::SeqCst), 0, "丢失不得设目标");
        assert_eq!(clear_calls.load(Ordering::SeqCst), 3, "每拍都清目标（不追旧点）");
    }

    /// C9-3：**丢失 3 拍后目标回来 → 再 `set_goal`**（专门卡记忆式 `Fallback` 的死锁）
    #[tokio::test]
    async fn test_tree_resumes_after_lost() {
        let feed = FakeFeed::new(vec![target(0.0, 5.0, 1)]);
        let nav = FakeNav::new(Some((3, 4)));
        let (mut t, set_calls, clear_calls) = tree(feed.clone(), nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 1);

        feed.set(vec![]); // 人出画
        for _ in 0..3 {
            assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
            assert_eq!(out.next_cell, None);
        }
        assert_eq!(clear_calls.load(Ordering::SeqCst), 3);

        // 人回来：必须重新跟（若外层误用记忆式 `Fallback`，这里会永远停在 Hold）
        feed.set(vec![target(0.0, 5.0, 2)]);
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 2, "目标回来后必须重新武装");
        assert_eq!(out.next_cell, Some((3, 4)));
    }

    /// C9-4：`d=1.0`（TooClose）→ 不追、不动
    #[tokio::test]
    async fn test_tree_too_close_holds() {
        let feed = FakeFeed::new(vec![target(0.0, 1.0, 1)]);
        let nav = FakeNav::new(Some((3, 4)));
        let (mut t, set_calls, clear_calls) = tree(feed, nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, None, "太近必须停");
        assert_eq!(set_calls.load(Ordering::SeqCst), 0, "太近不得设目标");
        assert_eq!(clear_calls.load(Ordering::SeqCst), 1);
    }

    /// C9-5：`d=2.0`（InBand）→ **不重设**目标（`next_cell` 为 `None`：假端口 `Idle` 不给路）
    #[tokio::test]
    async fn test_tree_in_band_keeps_goal() {
        let feed = FakeFeed::new(vec![target(0.0, 2.0, 1)]);
        let nav = FakeNav::new(Some((3, 4)));
        let (mut t, set_calls, clear_calls) = tree(feed, nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 0, "带内不得重设目标（迟滞）");
        assert_eq!(clear_calls.load(Ordering::SeqCst), 0, "带内不得清目标");
        assert_eq!(out.next_cell, None, "未武装的假端口不给路");
    }

    /// C9-6：Chase 中目标径向挪 0.2 m → 不重设；再挪 0.6 m（累计 0.8 m）→ 重设（0.5 m 节流）
    #[tokio::test]
    async fn test_tree_goal_update_throttled() {
        let feed = FakeFeed::new(vec![target(0.0, 5.0, 1)]);
        let nav = FakeNav::new(Some((3, 4)));
        let (mut t, set_calls, _) = tree(feed.clone(), nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 1);
        // 目标点从 (0,2.5) 挪到 (0,2.7)：0.2 m < 0.5 m → 不重设
        feed.set(vec![target(0.0, 5.2, 2)]);
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 1, "0.2 m 不得重设");
        // 目标点挪到 (0,3.3)：距上次**设过**的 (0,2.5) 是 0.8 m > 0.5 m → 重设
        feed.set(vec![target(0.0, 5.8, 3)]);
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 2, "累计 0.8 m 应重设");
    }

    /// C9-7：假端口给的下一格落在人的膨胀格 → 本拍否决
    #[tokio::test]
    async fn test_tree_guards_person_cell() {
        // 人在 (0,5)（世界），其膨胀格含 (0,10)
        let feed = FakeFeed::new(vec![target(0.0, 5.0, 1)]);
        let nav = FakeNav::new(Some((0, 10)));
        let (mut t, set_calls, _) = tree(feed, nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 1, "仍是 Chase（只是本拍让行）");
        assert_eq!(out.next_cell, None, "落在人格里必须否决");
    }

    /// `halt` 幂等：连清两次不 panic，且之后重新武装
    #[tokio::test]
    async fn test_tree_halt_is_idempotent() {
        let feed = FakeFeed::new(vec![target(0.0, 5.0, 1)]);
        let nav = FakeNav::new(Some((3, 4)));
        let (mut t, set_calls, _) = tree(feed, nav, FollowParams::default()).await;

        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        t.halt();
        t.halt();
        // 树被 halt 后再 tick：FollowLeaf 重新武装
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(set_calls.load(Ordering::SeqCst), 2);
    }
}
