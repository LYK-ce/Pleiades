//Presented by KeJi
//Created Date ： 2026-10-28
//Modified Date ： 2026-10-28

//! 跟随树（task_37 §六）：sim **机栈独有**，真机 / 车栈收到 `Follow` 仍只建平凡树。
//!
//! ```text
//! ReactiveFallback                          ← 不是记忆式 Fallback（见下）
//! ├─ ReactiveSequence
//! │  ├─ TargetGate         有目标（age ≤ stale）→ 写本拍快照
//! │  ├─ Condition          未太近（d > d_near）
//! │  └─ StepLeaf           d > d_far（或带内且上拍在走）→ 沿「me → target」方向给一格
//! └─ HoldLeaf              丢掉 / 太近 → nav.clear_goal() + next_cell = None（不追旧点）
//! ```
//!
//! **本叶子不经 D\***（§六.2 / §六.3，相对 §五 的修正）：不调用 `nav.set_goal`、不调用 `nav.step`。
//! §五 走 `nav.set_goal → D*`，而 D* 是 **4 连通**：近距离时「下一格」常落在侧向格，`executor` 追格心
//! → 原地打转 → 人从画面边缘出画（§六.2 实测）。改为**直接朝目标方向给一格**：
//! `next_cell = world_to_grid(me + 0.5 m · unit(target − me))`（[`step_cell`]），一拍算完、无路径记忆。
//! 人类已明确**不改行动逻辑**（`executor.rs` / `nav/` 只读），所以修正只能落在叶子上。
//!
//! 仍持有 `nav` 的唯一用途：Hold 时 `clear_goal()`。本树从不 `set_goal`（nav 本就 `Idle`），但
//! 「Hold ⇒ nav 目标被清」这条观测口径要保住 —— `ugv_sim/mod.rs::test_follow_tree_holds_without_detector`
//! 按 `nav_status == idle` 断言。`nav/` 一行不改。
//!
//! **为什么外层用 `ReactiveFallback` 而不是 §五.4 画字面的 `Fallback`**：本仓 `Fallback` 是
//! **记忆式**（`composite.rs`），子节点 `Running` 时记住下标、下拍不回头。`HoldLeaf` 必须返回
//! `Running`（`Follow` 是持续任务，一旦 `Success` 队列就弹走这条），于是记忆式 `Fallback` 会
//! **永久锁在 Hold 分支**——人回来也不再跟。`ReactiveFallback` 每拍从第 0 个子重判，并在切分支时
//! `halt` 上一拍的分支，`StepLeaf::halt` 借此把迟滞状态清回 `Hold`。
//!
//! **`age` 的时钟口径**：`Target.frame_ts_ms` 在 sim 是**世界时间**，没有「当前世界时间」可比，
//! 所以这里用本地单调 `Instant` 记「首次见到该 `frame_seq`」的时刻，之后算 `now - last_seen`。
//! 相机停帧 / 人出画（`latest_targets()` 空）→ 直接 `Lost`。
//!
//! **人作为动态障碍**：机栈的 `Navigator` 是 `ObstacleMode::Ignore`（喂空栅格、`dynamic_obstacles`
//! 恒空），`mark_obstacle()` 的标记会被同一拍重算抹掉 → **无法真的注入 D\***。本文件复用
//! `nav/cluster_obstacles.rs` 的圆盘膨胀算出人的膨胀格，把「下一步落在人格里」的 `next_cell`
//! 否决掉（[`guard_cell`]）。`nav/` 一行不改。注意 `Advance` 只在 `d > d_far`（≥ 4 m）时给格，
//! 而 `next_cell` 只在 0.5 m 处 → 护栏实际不会触发；保留它是因为它是「人不能被撞」的不变量表达。

use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{
    Condition, FailCode, Frame, Node, ReactiveFallback, ReactiveSequence, Status,
};
use pleiades_base::robot::core::cluster::ClusterInfo;
use pleiades_base::robot::core::grid::{world_to_grid, CELL_RESOLUTION};
use pleiades_base::robot::core::state::RobotState;

use crate::sim::detector::{DetectorDevice, Target};
use crate::sim::ugv_sim::nav::{cluster_to_obstacle_cells, NavPort};

// 本文件**不引 `tracing`**：动作已有 `[Sim] 位置=… 动作=…` 日志（`sim_handler.rs`），10 Hz tick
// 再加日志只会刷屏；人类端到端验证够用。

// ---------------------------------------------------------------- 参数初值（§六.4）

/// `d > D_FAR_M` 才前进（米）
pub const D_FAR_M: f32 = 4.0;
/// `d <= D_NEAR_M` 停下（米）
pub const D_NEAR_M: f32 = 2.5;
/// 目标过期时间（毫秒）：`age > STALE_MS` → 视为丢失（§六.4）
pub const STALE_MS: u64 = 800;
/// 人作为障碍的圆盘膨胀半径（米）：1 格（0.5 m），与 `cluster_obstacle_cells` 同口径
pub const PERSON_INFLATE_M: f32 = 0.5;

/// 跟随参数（纯值 + `Default`；生产用默认值，单测可改小/改大）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FollowParams {
    pub d_far: f32,
    pub d_near: f32,
    pub stale_ms: u64,
    pub person_inflate_m: f32,
}

impl Default for FollowParams {
    fn default() -> Self {
        Self {
            d_far: D_FAR_M,
            d_near: D_NEAR_M,
            stale_ms: STALE_MS,
            person_inflate_m: PERSON_INFLATE_M,
        }
    }
}

/// 一拍的目标快照（`d` 与 detector 结果**同一次**算出）
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

/// 本拍结论（[`decide`] 的返回值；**纯值**，单测主战场）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 无目标 / 过期 / `d <= d_near`（或带内且上拍也没走）→ 不动
    Hold,
    /// `d > d_far`（或带内且上拍在走）→ 朝目标方向给一格
    Advance,
}

// ---------------------------------------------------------------- 纯函数

/// 「丢失」：无目标，或 `age` 严格超过 `stale`（§六.4「超过即视为丢失」）。`TargetGate` 的唯一判据。
pub fn lost(t: Option<&Sample>, p: &FollowParams) -> bool {
    match t {
        None => true,
        Some(s) => s.age_ms > p.stale_ms,
    }
}

/// 「太近」：`d <= d_near`（含等号，与 §六.3 的 `d <= 2.5` 一致）。`Condition` 的唯一判据。
pub fn too_close(t: Option<&Sample>, p: &FollowParams) -> bool {
    match t {
        None => false, // 无目标不谈远近（丢失由 `lost` 管）
        Some(s) => s.d <= p.d_near,
    }
}

/// 本拍结论（**纯函数**：不碰时钟、不碰位姿、不碰导航器）
///
/// 三档 + 迟滞（§六.3 的两条阈值 + §六.4 的三个参数）：
/// - 无目标 / `age > stale` → `Hold`（丢掉，不追旧点；优先级最高）
/// - `d > d_far` → `Advance`（人走远了，朝人飞）
/// - `d <= d_near` → `Hold`（到站，停）
/// - 带内（`d_near < d <= d_far`）→ **沿用上拍** `prev`（迟滞：不在 4.0 m 边界上抖）
pub fn decide(target: Option<&Sample>, p: &FollowParams, prev: Outcome) -> Outcome {
    match target {
        None => Outcome::Hold,
        Some(s) if s.age_ms > p.stale_ms => Outcome::Hold,
        Some(s) if s.d > p.d_far => Outcome::Advance,
        Some(s) if s.d <= p.d_near => Outcome::Hold,
        Some(_) => prev,
    }
}

/// 沿「me → target」单位向量前进 `CELL_RESOLUTION`（0.5 m）后所落的**格**（§六.3）
///
/// 两点重合（`d < 1e-3`）→ `None`：方向无定义，不给出下一格、不产生 `NaN`。
/// 取格用 `grid::world_to_grid`（floor 语义），与 `executor::cell_center`（`(g+0.5)×0.5`）同口径。
pub fn step_cell(me: (f32, f32), target: (f32, f32)) -> Option<(i32, i32)> {
    let (dx, dy) = (target.0 - me.0, target.1 - me.1);
    let d = (dx * dx + dy * dy).sqrt();
    if d < 1e-3 {
        return None;
    }
    Some(world_to_grid(
        me.0 + dx / d * CELL_RESOLUTION,
        me.1 + dy / d * CELL_RESOLUTION,
    ))
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
/// 而 `Hold` 必须把目标清掉（「Hold ⇒ nav 目标被清」的观测口径）；
/// `pleiades-sim/src/sim/ugv_sim/nav/` 不在允许改动路径，故在 mission 层加这个薄 trait。
pub trait FollowNav: NavPort {
    fn clear_goal(&mut self);
}

impl FollowNav for crate::sim::ugv_sim::nav::Navigator {
    fn clear_goal(&mut self) {
        crate::sim::ugv_sim::nav::Navigator::clear_goal(self)
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
        // 主路：有目标 → 未太近 → 朝人给一格
        Box::new(ReactiveSequence::new(vec![
            Box::new(TargetGate::new(robot_state.clone(), source, slot.clone(), params)),
            Box::new(Condition::new(move || {
                // 与 `TargetGate` 共用同一份槽位快照 → 两个条件不会各算一次、不会互相矛盾
                let snap = *lock(&slot_for_cond);
                !lost(snap.as_ref(), &p2) && !too_close(snap.as_ref(), &p2)
            })),
            Box::new(StepLeaf::new(nav.clone(), robot_state, slot, params)),
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
        let gone = lost(s.as_ref(), &self.params);
        *lock(&self.slot) = s; // 无论丢没丢都写；`Condition` / `StepLeaf` 读同一份
        if gone {
            Status::Failure(FailCode::Unspecified)
        } else {
            Status::Success
        }
    }

    fn halt(&mut self) {
        *lock(&self.slot) = None;
    }
}

/// 叶子：**不经 D\***，沿「me → target」方向给一格
pub struct StepLeaf<P: FollowNav> {
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
    slot: Arc<StdMutex<Option<Sample>>>,
    params: FollowParams,
    /// 上拍结论（距离带内沿用 → 迟滞状态；`halt` 归零）
    last: Outcome,
}

impl<P: FollowNav> StepLeaf<P> {
    fn new(
        nav: Arc<Mutex<P>>,
        robot_state: Arc<RwLock<RobotState>>,
        slot: Arc<StdMutex<Option<Sample>>>,
        params: FollowParams,
    ) -> Self {
        Self { nav, robot_state, slot, params, last: Outcome::Hold }
    }
}

#[async_trait]
impl<P: FollowNav + 'static> Node for StepLeaf<P> {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        // 位姿先读完再放锁（同 goto.rs 的纪律：持锁跨 await 会挤设备端 RX）
        let me = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y)
        };
        let snap = *lock(&self.slot);
        let out = decide(snap.as_ref(), &self.params, self.last);
        self.last = out; // 带内停（Hold）时下拍仍从 Hold 起算

        match snap {
            Some(s) if out == Outcome::Advance => {
                // 人作为动态障碍：下一步落在人的膨胀格里 → 本拍否决。
                // **不设目标、不取下一步**：格由几何算出（`step_cell`），nav 全程不参与。
                f.out.next_cell =
                    guard_cell(step_cell(me, (s.x, s.y)), (s.x, s.y), self.params.person_inflate_m);
            }
            _ => {
                self.nav.lock().await.clear_goal();
                f.out.next_cell = None;
            }
        }
        // 持续任务：**不终值**（`Success` 会让队列把这条弹走，人就再也跟不上了）
        Status::Running
    }

    /// 只清自身状态、不发 Stop（§十一 #23）；幂等
    fn halt(&mut self) {
        self.last = Outcome::Hold;
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
        // 返回 **`Running`**（不是 `Success`）：`Follow` 是持续任务，只有 TASK_SET 替换 /
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

    use crate::sim::ugv_sim::nav::NavStatus;

    /// 假导航端口给的哨兵格：若叶子敢用 `nav.step()` 的返回值，`next_cell` 会变成它 → 测试立刻红
    const SENTINEL: (i32, i32) = (99, 99);

    // ------------------------------------------------------------ 夹具

    /// 合成时刻：`t0 + ms`（不 sleep）
    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + std::time::Duration::from_millis(ms)
    }

    /// 合成 detector 目标（`range_m` 与 `d` 同值，除非另有说明）
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

    /// 合成一拍快照（`decide` 只读 `d` 与 `age_ms`）
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

    /// 假导航端口：**只用来验「有没有被打扰」**——本 step 要求 `set_goal` / `step` 一次都不许调
    struct FakeNav {
        status: NavStatus,
        set_goal_calls: Arc<AtomicU32>,
        step_calls: Arc<AtomicU32>,
        clear_goal_calls: Arc<AtomicU32>,
    }

    impl FakeNav {
        fn new() -> Arc<Mutex<Self>> {
            Arc::new(Mutex::new(Self {
                status: NavStatus::Idle,
                set_goal_calls: Arc::new(AtomicU32::new(0)),
                step_calls: Arc::new(AtomicU32::new(0)),
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
            self.step_calls.fetch_add(1, Ordering::SeqCst);
            match self.status {
                NavStatus::Navigating => Some(SENTINEL),
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

    /// 一套树 + 三个计数器 + 位姿句柄
    struct Rig {
        tree: Box<dyn Node>,
        state: Arc<RwLock<RobotState>>,
        set_goal: Arc<AtomicU32>,
        step: Arc<AtomicU32>,
        clear_goal: Arc<AtomicU32>,
    }

    async fn rig(feed: Arc<FakeFeed>, me: (f32, f32)) -> Rig {
        let state = Arc::new(RwLock::new(RobotState::default()));
        set_pose(&state, me.0, me.1).await;
        let source = Arc::new(TargetSource::new(Some(feed as Arc<dyn TargetFeed>)));
        let nav = FakeNav::new();
        let (set_goal, step, clear_goal) = {
            let n = nav.lock().await;
            (n.set_goal_calls.clone(), n.step_calls.clone(), n.clear_goal_calls.clone())
        };
        let tree = build(nav, state.clone(), source, FollowParams::default());
        Rig { tree, state, set_goal, step, clear_goal }
    }

    fn frame(out: &mut MotionOut) -> Frame<'_> {
        Frame { dt_ms: 50, tick_no: 0, out }
    }

    // ------------------------------------------------------------ 纯函数

    /// §六.5-3：`decide` 三档 + 两个边界 + 迟滞 + 丢失优先级
    #[test]
    fn test_decide_far_near_band_and_stale() {
        let p = FollowParams::default();
        // 无目标：即使上拍在走也必须停
        assert_eq!(decide(None, &p, Outcome::Advance), Outcome::Hold, "无目标 → Hold");
        // stale 边界：> 800 才算丢（与 §五.5「超过即视为丢失」同口径）
        assert_eq!(
            decide(Some(&sample(9.0, STALE_MS)), &p, Outcome::Hold),
            Outcome::Advance,
            "age == stale 不过期，d=9 > 4 → Advance"
        );
        assert_eq!(
            decide(Some(&sample(9.0, STALE_MS + 1)), &p, Outcome::Advance),
            Outcome::Hold,
            "age == stale+1 → Hold（即使上拍在走）"
        );
        // d_far 边界：> 4.0 才走；== 4.0 落在带内 → 沿用上拍
        assert_eq!(
            decide(Some(&sample(D_FAR_M, 0)), &p, Outcome::Hold),
            Outcome::Hold,
            "d == d_far → 带内，沿用上拍 Hold"
        );
        assert_eq!(
            decide(Some(&sample(D_FAR_M, 0)), &p, Outcome::Advance),
            Outcome::Advance,
            "d == d_far → 带内，沿用上拍 Advance"
        );
        assert_eq!(
            decide(Some(&sample(D_FAR_M + 0.1, 0)), &p, Outcome::Hold),
            Outcome::Advance,
            "d > d_far → Advance（与上拍无关）"
        );
        // d_near 边界：<= 2.5 停（与上拍无关）
        assert_eq!(
            decide(Some(&sample(D_NEAR_M, 0)), &p, Outcome::Advance),
            Outcome::Hold,
            "d == d_near → Hold"
        );
        assert_eq!(
            decide(Some(&sample(D_NEAR_M - 0.1, 0)), &p, Outcome::Advance),
            Outcome::Hold,
            "d < d_near（人已贴脸）→ Hold"
        );
        // 带内两向都验（迟滞是真的沿用，不是写死）
        assert_eq!(decide(Some(&sample(3.0, 0)), &p, Outcome::Hold), Outcome::Hold);
        assert_eq!(decide(Some(&sample(3.0, 0)), &p, Outcome::Advance), Outcome::Advance);
    }

    /// `lost` / `too_close` 两个谓词（`TargetGate` / `Condition` 的唯一判据）
    #[test]
    fn test_lost_and_too_close_predicates() {
        let p = FollowParams::default();
        assert!(lost(None, &p), "无目标 = 丢");
        assert!(!lost(Some(&sample(9.0, STALE_MS)), &p), "age == stale 不算丢");
        assert!(lost(Some(&sample(9.0, STALE_MS + 1)), &p), "age > stale = 丢");
        assert!(!too_close(None, &p), "无目标不谈远近");
        assert!(too_close(Some(&sample(D_NEAR_M, 0)), &p), "d == d_near 太近");
        assert!(too_close(Some(&sample(D_NEAR_M - 0.1, 0)), &p));
        assert!(!too_close(Some(&sample(D_NEAR_M + 0.1, 0)), &p));
    }

    /// §六.5-3：目标在前方 / 在侧面 → **方向随方位变化**（4 轴向 + 45°）
    #[test]
    fn test_step_cell_follows_bearing() {
        let me = (0.25, 0.25); // 格 (0,0) 的格心
        assert_eq!(step_cell(me, (0.25, 9.25)), Some((0, 1)), "正前方 +y → 上方一格");
        assert_eq!(step_cell(me, (9.25, 0.25)), Some((1, 0)), "侧面 +x → 右方一格");
        assert_eq!(step_cell(me, (0.25, -8.75)), Some((0, -1)), "-y → 下方一格");
        assert_eq!(step_cell(me, (-8.75, 0.25)), Some((-1, 0)), "-x → 左方一格");
        // 45°：unit = (0.7071, 0.7071) × 0.5 = 0.3536 → (0.6036, 0.6036) → 格 (1,1)
        assert_eq!(step_cell(me, (9.25, 9.25)), Some((1, 1)), "45° → 对角一格");
        // **方向真的随方位变**，不是恒给同一格
        assert_ne!(step_cell(me, (0.25, 9.25)), step_cell(me, (9.25, 0.25)));
    }

    /// `step_cell` 退化（两点重合）→ `None`，不 `NaN`、不 panic
    #[test]
    fn test_step_cell_degenerate_returns_none() {
        assert_eq!(step_cell((3.0, 3.0), (3.0, 3.0)), None, "两点重合 → 方向无定义");
        assert_eq!(step_cell((3.0, 3.0), (3.0, 3.0005)), None, "d < 1 mm 也按重合处理");
    }

    /// 人的膨胀格 + `guard_cell` 否决（护栏保留）
    #[test]
    fn test_person_cells_and_guard() {
        let cells = person_cells((2.0, 3.0), PERSON_INFLATE_M);
        assert!(cells.contains(&(4, 6)), "人所在格 (4,6) 必在集合里：{cells:?}");
        assert!(cells.windows(2).all(|w| w[0] < w[1]), "必须排序（确定性）");
        assert_eq!(guard_cell(Some((4, 6)), (2.0, 3.0), PERSON_INFLATE_M), None);
        assert_eq!(guard_cell(Some((20, 20)), (2.0, 3.0), PERSON_INFLATE_M), Some((20, 20)));
        assert_eq!(guard_cell(None, (2.0, 3.0), PERSON_INFLATE_M), None);
    }

    // ------------------------------------------------------------ 目标来源

    /// `TargetSource::sample` 的 age / 选最近 / 空帧（合成时刻，不 sleep）
    #[test]
    fn test_sample_age_and_nearest() {
        let t0 = Instant::now();
        let feed = FakeFeed::new(vec![target(0.0, 5.0, 7)]);
        let src = TargetSource::new(Some(feed.clone() as Arc<dyn TargetFeed>));

        let s = src.sample(t0, (0.0, 0.0)).expect("有目标");
        assert_eq!((s.age_ms, s.d), (0, 5.0));
        let s = src.sample(at(t0, 500), (0.0, 0.0)).expect("有目标");
        assert_eq!(s.age_ms, 500);
        feed.set(vec![target(0.0, 5.0, 8)]);
        let s = src.sample(at(t0, 900), (0.0, 0.0)).expect("有目标");
        assert_eq!(s.age_ms, 0, "换帧即归零");
        // age 超 stale → `decide` 必须 Stop（d=5 > d_far，若只看距离会误判 Advance）
        let s = src.sample(at(t0, 1701), (0.0, 0.0)).expect("有目标");
        assert_eq!(s.age_ms, 801);
        assert_eq!(decide(Some(&s), &FollowParams::default(), Outcome::Advance), Outcome::Hold);
        feed.set(vec![]);
        assert!(src.sample(at(t0, 1800), (0.0, 0.0)).is_none());
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

    /// §六.5-3：目标在前方 → `next_cell` 落在「me → target」方向的一格；连拍不抖
    #[tokio::test]
    async fn test_tree_far_gives_step_cell() {
        let mut r = rig(FakeFeed::new(vec![target(0.25, 9.25, 1)]), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "跟随不得终值");
        assert_eq!(out.next_cell, Some((0, 1)), "d=9 > 4 → 朝人方向一格");
        // 第二拍：位姿与人没变 → 同一格（不抖）
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((0, 1)));
    }

    /// §六.5-3：目标在侧面 → 方向随方位变化（同一台机、同一距离，只改方位）
    #[tokio::test]
    async fn test_tree_side_target_changes_direction() {
        let mut r = rig(FakeFeed::new(vec![target(9.25, 0.25, 1)]), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((1, 0)), "人转到 +x → 下一格跟着转到 +x");
    }

    /// §六.5-3：目标近（`d == d_near`）→ `next_cell = None`
    #[tokio::test]
    async fn test_tree_near_holds() {
        let mut r = rig(FakeFeed::new(vec![target(0.25, 2.75, 1)]), (0.25, 0.25)).await; // d = 2.5
        let mut out = MotionOut::default();
        for i in 0..2 {
            assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "第 {i} 拍不得终值");
            assert_eq!(out.next_cell, None, "d <= d_near → 停");
        }
        assert_eq!(r.set_goal.load(Ordering::SeqCst), 0, "太近不得设目标");
        assert!(r.clear_goal.load(Ordering::SeqCst) >= 1, "Hold 必须把 nav 目标清干净");
    }

    /// §六.5-3：无目标 / 过期 → `None` 且不终值；目标回来且远 → **立刻**给格
    /// （专门卡「记忆式 `Fallback` + `Running` 型 Hold = 死锁」那条坑）
    #[tokio::test]
    async fn test_tree_lost_holds_and_resumes() {
        let feed = FakeFeed::new(vec![]);
        let mut r = rig(feed.clone(), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        for i in 0..3 {
            assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(out.next_cell, None, "无目标不得给下一格");
        }
        assert_eq!(r.clear_goal.load(Ordering::SeqCst), 3, "每拍都清目标（不追旧点）");
        feed.set(vec![target(0.25, 9.25, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((0, 1)), "人回来必须重新跟");
    }

    /// §六.3 的核心断言：**跟随链路里没有 D\***（`set_goal` / `step` 一次都不许调）
    #[tokio::test]
    async fn test_tree_never_touches_nav() {
        let mut r = rig(FakeFeed::new(vec![target(0.25, 9.25, 1)]), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        for i in 0..5 {
            assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
            assert_eq!(out.next_cell, Some((0, 1)), "第 {i} 拍：格由几何算出，不是 nav 给的");
        }
        assert_eq!(r.set_goal.load(Ordering::SeqCst), 0, "不得 nav.set_goal");
        assert_eq!(r.step.load(Ordering::SeqCst), 0, "不得 nav.step");
        assert_ne!(out.next_cell, Some(SENTINEL), "绝不能用 nav.step() 的返回值");
    }

    /// 带内（`2.5 < d <= 4.0`）沿用上拍；`halt` 把迟滞归零（切分支 / 抢占恢复后不继承旧结论）
    #[tokio::test]
    async fn test_tree_band_hysteresis_and_halt() {
        let feed = FakeFeed::new(vec![target(0.25, 9.25, 1)]); // d = 9 → Advance
        let mut r = rig(feed.clone(), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((0, 1)), "先跑起来");

        // 人走近到 d = 3.0（带内）：沿用上拍 Advance → 仍给格
        feed.set(vec![target(0.25, 3.25, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((0, 1)), "带内沿用上拍 Advance");

        // halt（切分支 / 抢占恢复）：迟滞清零 → 同一份输入改判 Hold
        r.tree.halt();
        r.tree.halt(); // 幂等
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, None, "halt 后带内按 Hold 起算");
        // 位姿句柄仍在（`Rig` 的字段被读过 → 不留 dead_code）
        assert_eq!(r.state.read().await.x, 0.25);
    }
}
