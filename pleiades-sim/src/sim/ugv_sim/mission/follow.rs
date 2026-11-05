//Presented by KeJi
//Created Date ： 2026-10-28
//Modified Date ： 2026-11-05

//! 跟随树（task_37 §六 / task_45 §二 Step 1）：sim 车栈 `Follow` 由此文件建真树；本文件由
//! `pleiades-ugv/src/ugv/mission/follow.rs` **逐字复制**（口径 #8），差异只有 `use` 路径与文件头。
//!
//! ```text
//! ReactiveFallback                          ← 不是记忆式 Fallback（见下）
//! ├─ ReactiveSequence
//! │  ├─ TargetGate         有目标（age ≤ stale）→ 写本拍快照
//! │  ├─ Condition          未太近（d > d_near）
//! │  └─ StepLeaf           d > d_far（或带内且上拍在走）→ 给人的世界坐标（点）
//! │                         带内停（上拍 Hold）→ 只给人的**朝向**（`face_point`，只转不走）
//! └─ HoldLeaf              太近 → 只给朝向；丢掉 / 过期 → 三个都 None（不追旧点、不乱转）
//! ```
//!
//! **本叶子不经 D\***（task_45 §一 #1/#6）：不调用 `nav.set_goal`、不调用 `nav.step`，也不再有
//! 「算一格」这一步 —— `Advance` 直接把人的**世界坐标**写进 `Frame.out.target_point`，执行器拿原值
//! 算方位（不经格心、不量化）。格与格心从此不进转向环：一个 0.5 m 的格最坏带来 35° 方位误差，
//! 会把人顶出 12° 弧线档、逼进「原地转」→ 车头左右摆、人从画面边缘出画（task_45 §一 的病根）。
//!
//! **本树不持 `nav`**（task_45 §一 #6）：mission 层那份「给 Follow 加一个清目标方法」的薄 trait、
//! 清目标调用、`build(nav, …)` 全部删除；旧口径「Hold ⇒ nav 目标被清」随 nav 一起消失
//! （本树从不 `set_goal`，nav 本就 `Idle`）。
//! `nav/` 一行不改：巡逻 / Goto 仍走格 + D\*（`containment.rs` 的 `goto::build(nav)`）。
//!
//! **为什么外层用 `ReactiveFallback` 而不是 §五.4 画字面的 `Fallback`**：本仓 `Fallback` 是
//! **记忆式**（`composite.rs`），子节点 `Running` 时记住下标、下拍不回头。`HoldLeaf` 必须返回
//! `Running`（`Follow` 是持续任务，一旦 `Success` 队列就弹走这条），于是记忆式 `Fallback` 会
//! **永久锁在 Hold 分支**——人回来也不再跟。`ReactiveFallback` 每拍从第 0 个子重判，并在切分支时
//! `halt` 上一拍的分支，`StepLeaf::halt` 借此把迟滞状态清回 `Hold`。
//!
//! **`age` 的时钟口径**：`Target.frame_ts_ms` 是**设备侧时钟**（真车 = 本机单调毫秒 / sim = 世界时间），
//! 都没有「当前时刻」可比，所以这里用本地单调 `Instant` 记「首次见到该 `frame_seq`」的时刻，之后算
//! `now - last_seen`。相机停帧 / 人出画（`latest_targets()` 空）→ 直接 `Lost`。

use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::RwLock;

use pleiades_base::robot::core::behavior_tree::{
    Condition, FailCode, Frame, Node, ReactiveFallback, ReactiveSequence, Status,
};
use pleiades_base::robot::core::state::RobotState;

use crate::sim::detector::{DetectorDevice, Target};

// 本文件**不引 `tracing`**：日志由驱动层 `robot_handler.rs` / `sim_handler.rs` 负责，10 Hz tick
// 再加日志只会刷屏。

// ---------------------------------------------------------------- 参数初值（§六.4）

/// Follow 的 `d > D_FAR_M` 才前进（米；task_48 §五 Step 3 **放宽** 2.2 → 2.4）
pub const D_FAR_M: f32 = 2.4;
/// Follow 的 `d <= D_NEAR_M` 停下（米；同上**放宽** 1.2 → 1.4）
pub const D_NEAR_M: f32 = 1.4;
/// Containment（围控）的 `d > CONTAINMENT_D_FAR_M` 才前进（米；同上**收紧** 2.2 → 2.0，围控车靠得更近）
pub const CONTAINMENT_D_FAR_M: f32 = 2.0;
/// Containment 的 `d <= CONTAINMENT_D_NEAR_M` 停下（米；同上**收紧** 1.2 → 1.0）
pub const CONTAINMENT_D_NEAR_M: f32 = 1.0;
/// 目标过期时间（毫秒）：`age > STALE_MS` → 视为丢失（§六.4）
pub const STALE_MS: u64 = 800;

/// 跟随参数（纯值 + `Default`）：`Default` = **Follow** 那一组（2.4 / 1.4）；Containment 用 `CONTAINMENT_*`、Guard 用 `guard::GUARD_D_*`，三处建树各自**显式传参**（口径 #19/#20），只有单测才走 `Default`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FollowParams {
    pub d_far: f32,
    pub d_near: f32,
    pub stale_ms: u64,
}

impl Default for FollowParams {
    fn default() -> Self {
        Self { d_far: D_FAR_M, d_near: D_NEAR_M, stale_ms: STALE_MS }
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
    /// `d > d_far`（或带内且上拍在走）→ 给人的世界坐标（点）
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

/// 「太近」：`d <= d_near`（含等号，与 §六.3 的 `d <= 1.4` 一致）。`Condition` 的唯一判据。
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
/// - `d > d_far` → `Advance`（人走远了，朝人开）
/// - `d <= d_near` → `Hold`（到站，停）
/// - 带内（`d_near < d <= d_far`）→ **沿用上拍** `prev`（迟滞：不在 2.4 m 边界上抖）
pub fn decide(target: Option<&Sample>, p: &FollowParams, prev: Outcome) -> Outcome {
    match target {
        None => Outcome::Hold,
        Some(s) if s.age_ms > p.stale_ms => Outcome::Hold,
        Some(s) if s.d > p.d_far => Outcome::Advance,
        Some(s) if s.d <= p.d_near => Outcome::Hold,
        Some(_) => prev,
    }
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

/// 建树（每次入队新建实例，§十一 #12）
pub fn build(
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<TargetSource>,
    params: FollowParams,
) -> Box<dyn Node> {
    let slot: Arc<StdMutex<Option<Sample>>> = Arc::new(StdMutex::new(None));
    let slot_for_cond = slot.clone();
    // `HoldLeaf` 也要读同一份快照（task_46 §一 #6）：必须在 `slot` 移进 `StepLeaf` 之前 clone
    let slot_for_hold = slot.clone();
    let p2 = params;
    Box::new(ReactiveFallback::new(vec![
        // 主路：有目标 → 未太近 → 直接给人的世界坐标（点）
        Box::new(ReactiveSequence::new(vec![
            Box::new(TargetGate::new(robot_state, source, slot.clone(), params)),
            Box::new(Condition::new(move || {
                // 与 `TargetGate` 共用同一份槽位快照 → 两个条件不会各算一次、不会互相矛盾
                let snap = *lock(&slot_for_cond);
                !lost(snap.as_ref(), &p2) && !too_close(snap.as_ref(), &p2)
            })),
            Box::new(StepLeaf::new(slot, params)),
        ])),
        // 兜底：太近 → 只给朝向（只转不走）；丢掉 / 过期 → 三个字段都 `None`
        Box::new(HoldLeaf::new(slot_for_hold, params)),
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

/// 叶子：**不经 D\***，`Advance` 直接把人的世界坐标写进 `target_point`
pub struct StepLeaf {
    slot: Arc<StdMutex<Option<Sample>>>,
    params: FollowParams,
    /// 上拍结论（距离带内沿用 → 迟滞状态；`halt` 归零）
    last: Outcome,
}

impl StepLeaf {
    fn new(slot: Arc<StdMutex<Option<Sample>>>, params: FollowParams) -> Self {
        Self { slot, params, last: Outcome::Hold }
    }
}

#[async_trait]
impl Node for StepLeaf {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        let snap = *lock(&self.slot);
        let out = decide(snap.as_ref(), &self.params, self.last);
        self.last = out; // 带内停（Hold）时下拍仍从 Hold 起算

        match snap {
            Some(s) if out == Outcome::Advance => {
                // 直接给**人的世界坐标**：转向环里没有格心（task_45 §一 #1）
                f.out.target_point = Some((s.x, s.y));
                f.out.next_cell = None;
                // 前进这拍不给朝向：`target()` 里 face > point，残留的朝向会顶掉点
                f.out.face_point = None;
            }
            _ => {
                // 带内停（`d_near < d <= d_far` 且上拍 `Hold`）：只给朝向，执行器原地转
                // （task_46 §一 #2/#5）。丢失 / 过期不算「带内停」→ 三个都 `None`
                f.out.face_point = match snap.as_ref() {
                    Some(s) if !lost(Some(s), &self.params) => Some((s.x, s.y)),
                    _ => None,
                };
                f.out.next_cell = None;
                f.out.target_point = None;
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

/// 兜底叶子：太近 → 只给朝向（只转不走）；丢掉 / 过期 → 本拍什么都不给
pub struct HoldLeaf {
    /// 与 `TargetGate` / `Condition` 共用的本拍快照（本叶子**只读**，本拍唯一写点仍在 `TargetGate`）
    slot: Arc<StdMutex<Option<Sample>>>,
    params: FollowParams,
}

impl HoldLeaf {
    fn new(slot: Arc<StdMutex<Option<Sample>>>, params: FollowParams) -> Self {
        Self { slot, params }
    }
}

#[async_trait]
impl Node for HoldLeaf {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        // 走到这里只有两种可能：`TargetGate` 失败（丢失 / 过期）或 `Condition` 为假（太近）。
        // 「快照存在且未过期」⇒ 必然是「太近」⇒ 把人的世界坐标作为**朝向**交出去（只转不走）。
        // 丢失 / 过期 ⇒ 三个字段都 `None`：机头不乱转（task_46 §一 #5）。
        // 返回 **`Running`**（不是 `Success`）：`Follow` 是持续任务，只有 TASK_SET 替换 /
        // `mission_count = 0` 才结束；`Success` 会让队列提前弹走这条任务。
        let snap = *lock(&self.slot);
        f.out.face_point = match snap.as_ref() {
            Some(s) if !lost(Some(s), &self.params) => Some((s.x, s.y)),
            _ => None,
        };
        f.out.next_cell = None;
        f.out.target_point = None;
        Status::Running
    }

    fn halt(&mut self) {} // 无状态（快照归 `TargetGate` 管）
}

#[cfg(test)]
mod tests {
    use super::*;

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    /// 带内中点（**由常量推导**，不写死具体数值）：车 1.9 m / 机 3.1 m。
    /// 四份 `follow.rs` 因此保持逐字一致（口径 #8），且与本 crate 的 `D_NEAR_M/D_FAR_M` 自洽
    const MID_BAND: f32 = (D_FAR_M + D_NEAR_M) / 2.0;

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

    async fn set_pose(state: &Arc<RwLock<RobotState>>, x: f32, y: f32) {
        let mut rs = state.write().await;
        rs.x = x;
        rs.y = y;
    }

    /// 一套树 + 位姿句柄（本树**不持 nav**，没有导航计数器可断言）
    struct Rig {
        tree: Box<dyn Node>,
        state: Arc<RwLock<RobotState>>,
    }

    async fn rig(feed: Arc<FakeFeed>, me: (f32, f32)) -> Rig {
        let state = Arc::new(RwLock::new(RobotState::default()));
        set_pose(&state, me.0, me.1).await;
        let source = Arc::new(TargetSource::new(Some(feed as Arc<dyn TargetFeed>)));
        let tree = build(state.clone(), source, FollowParams::default());
        Rig { tree, state }
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
            "age == stale 不过期，d=9 > 2.4 → Advance"
        );
        assert_eq!(
            decide(Some(&sample(9.0, STALE_MS + 1)), &p, Outcome::Advance),
            Outcome::Hold,
            "age == stale+1 → Hold（即使上拍在走）"
        );
        // d_far 边界：> 2.4 才走；== 2.4 落在带内 → 沿用上拍
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
        // d_near 边界：<= 1.4 停（与上拍无关）
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
        // 带内两向都验（迟滞是真的沿用，不是写死）；值由 `D_FAR_M`/`D_NEAR_M` 推导，
        // 车 1.9 m、机 3.1 m，四份用例与各自常量自洽（task_46 §一 #10）
        assert_eq!(decide(Some(&sample(MID_BAND, 0)), &p, Outcome::Hold), Outcome::Hold);
        assert_eq!(decide(Some(&sample(MID_BAND, 0)), &p, Outcome::Advance), Outcome::Advance);
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

    /// task_45 §一 #1：目标在远处 → `target_point` 是**人的世界坐标原值**、`next_cell = None`；连拍不抖
    #[tokio::test]
    async fn test_tree_far_gives_point() {
        let mut r = rig(FakeFeed::new(vec![target(0.25, 9.25, 1)]), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "跟随不得终值");
        assert_eq!(out.target_point, Some((0.25, 9.25)), "d=9 > 2.4 → 直接给人的世界坐标");
        assert_eq!(out.next_cell, None, "给点时不得同时给格");
        // 第二拍：位姿与人没变 → 同一个点（不抖）
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.target_point, Some((0.25, 9.25)));
        assert_eq!(out.next_cell, None);
    }

    /// task_45 §一 #1：目标在侧面 → 给的点随方位变化（同一台机、同一距离，只改方位）
    #[tokio::test]
    async fn test_tree_side_target_changes_direction() {
        let mut r = rig(FakeFeed::new(vec![target(9.25, 0.25, 1)]), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.target_point, Some((9.25, 0.25)), "人转到 +x → 目标点跟着转到 +x");
        assert_eq!(out.next_cell, None);
    }

    /// task_46 §一 #5：目标近（`d == d_near`）→ **只给朝向**（人的世界坐标），不给点、不给格
    #[tokio::test]
    async fn test_tree_near_gives_face() {
        let mut r = rig(FakeFeed::new(vec![target(0.25, 0.25 + D_NEAR_M, 1)]), (0.25, 0.25)).await; // d = d_near
        let mut out = MotionOut::default();
        for i in 0..2 {
            assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "第 {i} 拍不得终值");
            assert_eq!(out.target_point, None, "d <= d_near → 不给点");
            assert_eq!(out.next_cell, None, "d <= d_near → 也不给格");
            assert_eq!(
                out.face_point,
                Some((0.25, 0.25 + D_NEAR_M)),
                "第 {i} 拍：太近 ⇒ 给人的朝向（只转不走）"
            );
        }
    }

    /// 无目标 / 过期 → 三个字段都 `None` 且不终值；目标回来且远 → **立刻**给点
    /// （专门卡「记忆式 `Fallback` + `Running` 型 Hold = 死锁」那条坑）
    #[tokio::test]
    async fn test_tree_lost_holds_and_resumes() {
        let feed = FakeFeed::new(vec![]);
        let mut r = rig(feed.clone(), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        for i in 0..3 {
            assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(out.next_cell, None, "无目标不得给下一格");
            assert_eq!(out.target_point, None, "无目标不得给目标点");
            assert_eq!(out.face_point, None, "无目标不得给朝向（机头不乱转）");
        }
        feed.set(vec![target(0.25, 9.25, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.target_point, Some((0.25, 9.25)), "人回来必须重新跟");
        assert_eq!(out.next_cell, None);
        assert_eq!(out.face_point, None, "前进这拍不得同时给朝向（face > point 会顶掉点）");
    }

    /// task_45 §一 #1/#6 的核心断言：**给点不给格**（转向环里没有格心，也不再有 nav）
    #[tokio::test]
    async fn test_tree_writes_point_not_cell() {
        let mut r = rig(FakeFeed::new(vec![target(0.25, 9.25, 1)]), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        for i in 0..5 {
            assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
            assert_eq!(out.target_point, Some((0.25, 9.25)), "第 {i} 拍：给的是人的坐标原值");
            assert_eq!(out.next_cell, None, "第 {i} 拍：绝不写格（写了就会走格心）");
        }
    }

    /// 带内（`D_NEAR_M < d <= D_FAR_M`）沿用上拍；`halt` 把迟滞归零（切分支 / 抢占恢复后不继承旧结论）
    ///
    /// task_46 §一 #5：带内**停**（`halt` 后按 Hold 起算）⇒ 只给朝向，不给点、不给格
    #[tokio::test]
    async fn test_tree_band_hysteresis_and_halt() {
        let feed = FakeFeed::new(vec![target(0.25, 9.25, 1)]); // d = 9 → Advance
        let mut r = rig(feed.clone(), (0.25, 0.25)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.target_point, Some((0.25, 9.25)), "先跑起来");

        // 人走近到带内中点：沿用上拍 Advance → 仍给点，且**不给朝向**（face > point 会顶掉点）
        feed.set(vec![target(0.25, 0.25 + MID_BAND, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.target_point, Some((0.25, 0.25 + MID_BAND)), "带内沿用上拍 Advance");
        assert_eq!(out.face_point, None, "前进这拍不得给朝向");

        // halt（切分支 / 抢占恢复）：迟滞清零 → 同一份输入改判 Hold → 只给朝向
        r.tree.halt();
        r.tree.halt(); // 幂等
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.target_point, None, "halt 后带内按 Hold 起算：不再给点");
        assert_eq!(out.next_cell, None);
        assert_eq!(
            out.face_point,
            Some((0.25, 0.25 + MID_BAND)),
            "halt 后带内停 ⇒ 只给朝向（只转不走）"
        );
        // 位姿句柄仍在（`Rig` 的字段被读过 → 不留 dead_code）
        assert_eq!(r.state.read().await.x, 0.25);
    }

    /// task_46 §一 #5：过期（`age > STALE_MS`）时 `HoldLeaf` 三个字段全 `None`
    /// （不追旧点、也不追旧朝向 —— 机头不乱转）
    #[tokio::test]
    async fn test_hold_leaf_stale_writes_nothing() {
        let p = FollowParams::default();
        let stale = Sample { x: 7.0, y: 8.0, ..sample(1.0, STALE_MS + 1) };
        let mut leaf = HoldLeaf::new(Arc::new(StdMutex::new(Some(stale))), p);
        let mut out = MotionOut::default();
        assert_eq!(leaf.tick(&mut frame(&mut out)).await, Status::Running, "兜底叶子不得终值");
        assert_eq!(out.face_point, None, "过期不得给朝向");
        assert_eq!(out.target_point, None);
        assert_eq!(out.next_cell, None);
    }

    /// task_48 §五 Step 3（口径 #19/#20）：**三组距离分家** —— Follow 放宽到 2.4 / 1.4，
    /// Containment 收紧到 2.0 / 1.0（Guard 固定 2.2 / 1.2，走它自己的 `GUARD_D_*`，不在本文件）。
    /// 两组迟滞带都仍是 1.0 m；`Default` 只代表 Follow 那一组，另外两处建树都**显式传参**。
    #[test]
    fn test_distance_constants_split_follow_vs_containment() {
        assert_eq!((D_FAR_M, D_NEAR_M), (2.4, 1.4), "Follow 放宽 0.2：2.4 / 1.4（口径 #19）");
        assert_eq!(
            (CONTAINMENT_D_FAR_M, CONTAINMENT_D_NEAR_M),
            (2.0, 1.0),
            "Containment 收紧 0.2：2.0 / 1.0（口径 #19）"
        );
        // 迟滞带仍 1.0 m。f32 减法有舍入（2.4 - 1.4 不是精确的 1.0）→ 用 epsilon 比，别写 `== 1.0`
        assert!((D_FAR_M - D_NEAR_M - 1.0).abs() < 1e-6, "Follow 带 = {:?}", D_FAR_M - D_NEAR_M);
        assert!(
            (CONTAINMENT_D_FAR_M - CONTAINMENT_D_NEAR_M - 1.0).abs() < 1e-6,
            "Containment 带 = {:?}",
            CONTAINMENT_D_FAR_M - CONTAINMENT_D_NEAR_M
        );
        // `Default` 必须就是 Follow 那组（Containment / Guard 都不许走它）
        assert_eq!(
            FollowParams::default(),
            FollowParams { d_far: D_FAR_M, d_near: D_NEAR_M, stale_ms: STALE_MS },
            "`Default` = Follow 的显式那组"
        );
        let p_follow = FollowParams::default();
        let p_contain = FollowParams {
            d_far: CONTAINMENT_D_FAR_M,
            d_near: CONTAINMENT_D_NEAR_M,
            stale_ms: STALE_MS,
        };
        // 同一距离 2.1 m：Follow 落在 1.4~2.4 带内（沿用上拍 Hold）↔ Containment 已超 2.0（前进）
        assert_eq!(
            decide(Some(&sample(2.1, 0)), &p_follow, Outcome::Hold),
            Outcome::Hold,
            "Follow：2.1 m 在带内"
        );
        assert_eq!(
            decide(Some(&sample(2.1, 0)), &p_contain, Outcome::Hold),
            Outcome::Advance,
            "Containment：2.1 m > 2.0 → 前进（围控车贴得更近）"
        );
        // 同一距离 1.3 m：Follow 已到停距 ↔ Containment 还没到
        assert!(too_close(Some(&sample(1.3, 0)), &p_follow), "Follow：1.3 m <= 1.4 → 停");
        assert!(!too_close(Some(&sample(1.3, 0)), &p_contain), "Containment：1.3 m > 1.0 → 继续");
    }
}
