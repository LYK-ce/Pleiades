//Presented by KeJi
//Created Date ： 2026-10-31
//Modified Date ： 2026-10-31

//! 封控树 v2（task_43 §七）：sim **车栈**版——与 `uav_sim/mission/containment.rs` 同形（逐字照抄，仅 `uav_sim`→`ugv_sim` 与文件头），真机与机端各持一份、不跨栈 `use`（口径 #1）。
//!
//! ```text
//! ReactiveFallback                          ← 不是记忆式 Fallback（见下）
//! ├─ ReactiveSequence
//! │  ├─ InsideSquare        有目标 && 未过期 && 目标 (x,y) 在正方形内
//! │  └─ follow::build(...)  复用现有 Follow 子树（TargetGate / Step / Hold）
//! └─ RepeatForever
//!    └─ Sequence [ goto::build(角1), goto::build(角2), goto::build(角3), goto::build(角4) ]
//! ```
//!
//! **为什么外层必须 `ReactiveFallback` 而不是 `Fallback`**：与 `follow.rs` 同一条坑（task_37 已踩）——
//! 本仓 `Fallback` 是**记忆式**（`composite.rs`），而 Follow 子的 `StepLeaf` / `HoldLeaf` 恒返回
//! `Running`（持续任务，`Success` 会让队列把这条弹走）。记忆式 `Fallback` 会**永久锁在**第一分支：
//! 人离开后不再切巡逻，人回来也不再重判。`ReactiveFallback` 每拍从第 0 个子重判，并在切分支时 `halt`
//! 上一拍的分支，`InsideSquare` 这一拍才能真的当条件用。
//!
//! **为什么 `InsideSquare` 与 Follow 子树共享同一个 `Arc<TargetSource>`**：`TargetSource` 内部持有
//! 「首次见到该 `frame_seq`」的本地单调时钟。若各建一份，条件判「未过期」与叶子判「未过期」会用到
//! 两套 age，同一拍可能互相矛盾（条件说新鲜、叶子说丢了）。共用一份 → 一帧一算、两处一致。
//!
//! **巡逻循环的语义**（`RepeatForever` + 记忆式 `Sequence`）：`goto` 到角返回 `Success` → `Sequence`
//! 移到下一个角；四个角都 `Success` → `Sequence` 回到角 1 并 `Success` 一拍 → `RepeatForever` 计数 +1、
//! `halt` 子树（`GoToLeaf::armed` 清零）→ 返回 `Running`。所以下一拍**从角 1 重新开始**，永不终值。
//!
//! **唯一的失败路径**：巡逻某个角超时（`goto::build` 自带 `Timeout(60 s)`）→ `Failure(Timeout)` 上抛 →
//! 任务终值。人离开 / 丢失不算失败，只是切回巡逻。
//!
//! 本文件**不引 `tracing`**（10 Hz tick，避免刷屏；与 `follow.rs` 同纪律）。
//!
//! **v2（task_41 §5.3）：`InsideSquare` 内发协助请求**。条件成功时，若「首次」或目标相对
//! **上次发送点**移动 > [`ASSIST_MOVE_M`]（1 m）→ 调 [`AssistPublisher::publish`]；条件失败
//! （无目标 / 过期 / 出正方形）或 `halt()` → 清 `last_sent`，人再进界时立即重发。
//! 树的结构与巡逻 / 跟随决策**一行未改**：发送只是 `InsideSquare` 的副作用。
//! 传输细节（帧、topic、限流告警）全在发布者实现里（生产 = `sim_handler.rs`），本文件仍不引 `tracing`。
//! 注意：`ReactiveFallback` 切分支会 `halt` 跟随分支 → `last_sent` 被清 → 从巡逻切回跟随时会
//! **立即重发**一次协助请求（这是「人再进来立即发」的期望行为，Step 4 会看到多次 assist）。

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{
    FailCode, Frame, Node, ReactiveFallback, ReactiveSequence, RepeatForever, Sequence, Status,
};
use pleiades_base::robot::core::state::RobotState;

use crate::sim::ugv_sim::mission::{follow, goto};

// ---------------------------------------------------------------- 协助请求（task_41 §5.5.6）

/// 目标相对**上次发送点**移动超过该距离（米）才再发一次协助请求（§二.9）
pub const ASSIST_MOVE_M: f32 = 1.0;

/// 协助请求发布者（薄端口：树不关心是 gossipsub 还是记录器；生产实现在 `sim_handler.rs`）
pub trait AssistPublisher: Send + Sync {
    /// 发一条协助请求（非阻塞；实现方失败只自己 warn，不影响树）
    fn publish(&self, x: f32, y: f32);
}

// ---------------------------------------------------------------- 纯函数

/// 巡逻角点（顺序即初始航向，写死；task_41 §3.2）：
/// `(cx−h, cy−h) → (cx+h, cy−h) → (cx+h, cy+h) → (cx−h, cy+h)`
pub fn corners(cx: f32, cy: f32, half: f32) -> [(f32, f32); 4] {
    [
        (cx - half, cy - half),
        (cx + half, cy - half),
        (cx + half, cy + half),
        (cx - half, cy + half),
    ]
}

/// 正方形判定（轴对齐，边界含在界内）：`|px−cx| ≤ h && |py−cy| ≤ h`
pub fn inside_square(s: &follow::Sample, center: (f32, f32), half: f32) -> bool {
    (s.x - center.0).abs() <= half && (s.y - center.1).abs() <= half
}

/// 「本拍有入侵者」= 有目标 && 未过期（`!follow::lost`）&& 在正方形内
///
/// Step 1 不加边界迟滞（task_41 §二 口径）：出界即回巡逻，每拍独立判。
pub fn intruder(
    t: Option<&follow::Sample>,
    center: (f32, f32),
    half: f32,
    p: &follow::FollowParams,
) -> bool {
    match t {
        None => false,
        Some(s) => !follow::lost(Some(s), p) && inside_square(s, center, half),
    }
}

// ---------------------------------------------------------------- 建树

/// 建封控树（每次入队**新建实例**，不复用节点对象，§十一 #12）
///
/// `publisher = None` → 只判条件、不发协助请求（单测 / 无网络时）。
pub fn build<P: follow::FollowNav + 'static>(
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<follow::TargetSource>,
    center: (f32, f32),
    half: f32,
    publisher: Option<Arc<dyn AssistPublisher>>,
) -> Box<dyn Node> {
    let params = follow::FollowParams::default(); // stale_ms = follow::STALE_MS
    // 与 `InsideSquare` 共用 `source`（同一份 age 追踪，两处快照不矛盾）
    let follow_tree =
        follow::build(nav.clone(), robot_state.clone(), source.clone(), params);
    let inside = Box::new(InsideSquare {
        robot_state: robot_state.clone(),
        source,
        center,
        half,
        params,
        publisher,
        last_sent: None,
    });
    // 巡逻：四个角一圈（`goto` 自带 `Timeout(60 s)`），`RepeatForever` 让它永不终值
    let mut patrol: Vec<Box<dyn Node>> = Vec::with_capacity(4);
    for (x, y) in corners(center.0, center.1, half) {
        patrol.push(goto::build(x, y, nav.clone(), robot_state.clone()));
    }
    Box::new(ReactiveFallback::new(vec![
        Box::new(ReactiveSequence::new(vec![inside, follow_tree])),
        Box::new(RepeatForever::new(Box::new(Sequence::new(patrol)))),
    ]))
}

// ---------------------------------------------------------------- 节点

/// 条件：本拍目标是「入侵者」（有目标 && 未过期 && 在正方形内）
///
/// 除了判定，还负责**发协助请求**（v2，task_41 §5.3）：条件成功且（首次或目标相对上次发送点
/// 移动 > `ASSIST_MOVE_M`）→ `publisher.publish(x, y)` 并记下发送点；条件失败 → 清状态。
///
/// 不写任何**共享槽位**：Follow 子树里的 `TargetGate` 自己取快照（两处共用 `source`，
/// 取到的是同一帧、同一份 age）。
struct InsideSquare {
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<follow::TargetSource>,
    /// 正方形中心（世界坐标，米）
    center: (f32, f32),
    /// 半边长（米）
    half: f32,
    params: follow::FollowParams,
    /// 协助请求发布者（`None` = 不发，只判条件；单测可注入记录型假实现）
    publisher: Option<Arc<dyn AssistPublisher>>,
    /// 上次**已发出**的目标点（世界坐标）；`None` = 首次 / 已重置
    last_sent: Option<(f32, f32)>,
}

#[async_trait]
impl Node for InsideSquare {
    async fn tick(&mut self, _f: &mut Frame<'_>) -> Status {
        // 位姿先读完再放锁（同 `follow.rs` / `goto.rs` 的纪律：不持锁跨 await，也不持锁发帧）
        let me = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y)
        };
        let s = self.source.sample(Instant::now(), me);
        if !intruder(s.as_ref(), self.center, self.half, &self.params) {
            // 无目标 / 过期 / 出正方形 → 清状态：人再进来时**立即**发（§二.9）
            self.last_sent = None;
            return Status::Failure(FailCode::Unspecified);
        }
        // 条件成功：首次，或目标相对上次发送点移动 > ASSIST_MOVE_M 才发
        if let (Some(publisher), Some(sample)) = (self.publisher.as_ref(), s.as_ref()) {
            let need = match self.last_sent {
                None => true,
                Some((lx, ly)) => {
                    let (dx, dy) = (sample.x - lx, sample.y - ly);
                    dx * dx + dy * dy > ASSIST_MOVE_M * ASSIST_MOVE_M
                }
            };
            // 防御：坐标非有限不发、也不记账（下一拍重试；`InsideSquare` 的界内判定已先挡掉 NaN）
            if need && sample.x.is_finite() && sample.y.is_finite() {
                publisher.publish(sample.x, sample.y);
                self.last_sent = Some((sample.x, sample.y));
            }
        }
        Status::Success
    }

    fn halt(&mut self) {
        self.last_sent = None; // 幂等：树被 halt 后重进 → 立即发
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex as StdMutex;

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    use crate::sim::detector::Target;
    use crate::sim::ugv_sim::mission::follow::{
        FollowNav, TargetFeed, TargetSource, STALE_MS,
    };
    use crate::sim::ugv_sim::nav::{NavPort, NavStatus};

    /// 假导航端口给的哨兵格：巡逻分支若无路可走才会用到它
    const SENTINEL: (i32, i32) = (99, 99);

    /// 封控中心 / 半边长（与任务文档的金标同一组数：64,64 中心、8 m 半边长）
    const CENTER: (f32, f32) = (64.0, 64.0);
    const HALF: f32 = 8.0;

    // ------------------------------------------------------------ 夹具

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

    /// 合成一拍快照（纯函数只读 `x` / `y` / `age_ms`）
    fn sample(x: f32, y: f32, age_ms: u64) -> follow::Sample {
        follow::Sample { x, y, d: 0.0, age_ms, frame_seq: 1, range_m: 0.0 }
    }

    /// 假目标来源：外部改内容，`latest_targets` 同步读（`follow.rs` 的夹具是私有 test 项，
    /// 本文件自己写一份——`follow.rs` 本步冻结，不许改）
    ///
    /// 用 `std::sync::Mutex`（不是 `tokio` 的）：`TargetFeed::latest_targets` 是**同步**端口，
    /// 不能 `await`；与 `follow.rs` 的夹具同法。
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

    /// 假导航端口：记下每次 `set_goal` 的目标（巡逻角点的顺序断言靠它）
    struct FakeNav {
        /// 每次 `set_goal` 的目标（按调用顺序）
        goals: Vec<(f32, f32)>,
        /// `true` = 一到目标就 `Arrived`（巡逻循环）；`false` = 恒 `Navigating`（给哨兵格）
        arrived: bool,
        status: NavStatus,
        set_goal_calls: Arc<AtomicU32>,
        clear_goal_calls: Arc<AtomicU32>,
    }

    impl FakeNav {
        fn new(arrived: bool) -> Arc<Mutex<Self>> {
            Arc::new(Mutex::new(Self {
                goals: Vec::new(),
                arrived,
                status: NavStatus::Idle,
                set_goal_calls: Arc::new(AtomicU32::new(0)),
                clear_goal_calls: Arc::new(AtomicU32::new(0)),
            }))
        }
    }

    #[async_trait]
    impl NavPort for FakeNav {
        fn set_goal(&mut self, x: f32, y: f32) {
            self.goals.push((x, y));
            self.set_goal_calls.fetch_add(1, Ordering::SeqCst);
            self.status = NavStatus::Navigating;
        }

        async fn step(&mut self, _rs: &RobotState) -> Option<(i32, i32)> {
            if self.arrived {
                self.status = NavStatus::Arrived;
                None
            } else {
                Some(SENTINEL)
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

    /// 一套树 + 导航句柄（`goals` / 计数器都从它读）
    struct Rig {
        tree: Box<dyn Node>,
        nav: Arc<Mutex<FakeNav>>,
    }

    async fn rig(
        feed: Arc<FakeFeed>,
        me: (f32, f32),
        arrived: bool,
        publisher: Option<Arc<dyn AssistPublisher>>,
    ) -> Rig {
        let state = Arc::new(RwLock::new(RobotState::default()));
        {
            let mut rs = state.write().await;
            rs.x = me.0;
            rs.y = me.1;
        }
        let source = Arc::new(TargetSource::new(Some(feed as Arc<dyn TargetFeed>)));
        let nav = FakeNav::new(arrived);
        let tree = build(nav.clone(), state, source, CENTER, HALF, publisher);
        Rig { tree, nav }
    }

    fn frame(out: &mut MotionOut) -> Frame<'_> {
        Frame { dt_ms: 50, tick_no: 0, out }
    }

    // ------------------------------------------------------------ 纯函数

    /// §3.2：角点顺序写死（也是初始航向）
    #[test]
    fn test_corners_order() {
        assert_eq!(
            corners(64.0, 64.0, 8.0),
            [(56.0, 56.0), (72.0, 56.0), (72.0, 72.0), (56.0, 72.0)],
            "角点顺序：(cx−h,cy−h) → (cx+h,cy−h) → (cx+h,cy+h) → (cx−h,cy+h)"
        );
        // 半边长 0 退化为一点（不 panic）
        assert_eq!(corners(1.0, 2.0, 0.0), [(1.0, 2.0); 4]);
    }

    /// §3.2：正方形边界**含在界内**，超出 1 mm 即出界（四条边各验一次）
    #[test]
    fn test_inside_square_boundary() {
        let c = CENTER;
        // 边界上（|d| == half）→ 在界内
        assert!(inside_square(&sample(56.0, 64.0, 0), c, HALF), "左边在界内");
        assert!(inside_square(&sample(72.0, 64.0, 0), c, HALF), "右边在界内");
        assert!(inside_square(&sample(64.0, 56.0, 0), c, HALF), "下边在界内");
        assert!(inside_square(&sample(64.0, 72.0, 0), c, HALF), "上边在界内");
        assert!(inside_square(&sample(64.0, 64.0, 0), c, HALF), "中心在界内");
        // 角上（两个方向都恰好在边界）→ 在界内
        assert!(inside_square(&sample(56.0, 56.0, 0), c, HALF));
        // 出界 1 mm
        assert!(!inside_square(&sample(56.0 - 1e-3, 64.0, 0), c, HALF), "左出界");
        assert!(!inside_square(&sample(72.0 + 1e-3, 64.0, 0), c, HALF), "右出界");
        assert!(!inside_square(&sample(64.0, 56.0 - 1e-3, 0), c, HALF), "下出界");
        assert!(!inside_square(&sample(64.0, 72.0 + 1e-3, 0), c, HALF), "上出界");
    }

    /// 「入侵者」= 有目标 && 未过期 && 在正方形内（`age` 用合成值，**不 sleep**）
    #[test]
    fn test_intruder_requires_fresh_and_inside() {
        let p = follow::FollowParams::default();
        assert!(!intruder(None, CENTER, HALF, &p), "无目标 → 不是入侵者");
        assert!(
            intruder(Some(&sample(64.0, 64.0, STALE_MS)), CENTER, HALF, &p),
            "age == stale 不过期 → 是入侵者"
        );
        assert!(
            !intruder(Some(&sample(64.0, 64.0, STALE_MS + 1)), CENTER, HALF, &p),
            "age > stale → 过期 → 回巡逻"
        );
        assert!(
            !intruder(Some(&sample(64.0, 90.0, 0)), CENTER, HALF, &p),
            "在界外 → 不是入侵者"
        );
        assert!(!intruder(Some(&sample(64.0, 90.0, STALE_MS + 1)), CENTER, HALF, &p));
    }

    // ------------------------------------------------------------ 树级

    /// 目标在正方形内 → 走 **Follow**：`next_cell` 来自几何步，`set_goal` 一次都不许调
    #[tokio::test]
    async fn test_target_inside_runs_follow() {
        let me = (56.0, 56.0);
        let mut r = rig(FakeFeed::new(vec![target(64.0, 64.0, 1)]), me, false, None).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "封控不得终值");
        assert_eq!(
            out.next_cell,
            follow::step_cell(me, (64.0, 64.0)),
            "跟随时下一格由几何算出（me → 人方向一格）"
        );
        let nav = r.nav.lock().await;
        assert!(nav.goals.is_empty(), "跟随分支不得调用 nav.set_goal：{:?}", nav.goals);
        assert_eq!(nav.set_goal_calls.load(Ordering::SeqCst), 0, "跟随分支不得碰 D*");
    }

    /// 目标在正方形外 → 走 **巡逻**：第一个角点 `(56,56)`
    #[tokio::test]
    async fn test_target_outside_patrols() {
        let mut r = rig(FakeFeed::new(vec![target(64.0, 90.0, 1)]), (56.0, 56.0), false, None).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "巡逻不得终值");
        let nav = r.nav.lock().await;
        assert_eq!(nav.goals, vec![(56.0, 56.0)], "出界 → 从角 1 开始巡逻");
        assert!(out.next_cell.is_some(), "巡逻分支必须给出下一格");
    }

    /// 目标从内到外 → 下一拍切回巡逻（`Reactive*` 每拍重判，不被 Follow 的 `Running` 锁死）
    #[tokio::test]
    async fn test_target_inside_then_outside_switches_to_patrol() {
        let feed = FakeFeed::new(vec![target(64.0, 64.0, 1)]);
        let mut r = rig(feed.clone(), (56.0, 56.0), false, None).await;
        let mut out = MotionOut::default();

        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert!(r.nav.lock().await.goals.is_empty(), "第一拍在跟：不得有巡逻目标");

        // 人走到界外（换帧 → age 归零，仍「新鲜」，只是不在正方形内）
        feed.set(vec![target(64.0, 90.0, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(
            r.nav.lock().await.goals,
            vec![(56.0, 56.0)],
            "人出界 → 切回巡逻，从角 1 开始"
        );
    }

    /// 无目标 → 巡逻（第一拍就到角 1）
    #[tokio::test]
    async fn test_no_target_patrols() {
        let mut r = rig(FakeFeed::new(vec![]), (56.0, 56.0), false, None).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(r.nav.lock().await.goals, vec![(56.0, 56.0)]);
    }

    /// 巡逻到角返回 `Success` → 下一拍**从角 1 重新开始**（`RepeatForever` 永不终值）
    #[tokio::test]
    async fn test_patrol_cycle_restarts_from_corner1() {
        let mut r = rig(FakeFeed::new(vec![]), (56.0, 56.0), true, None).await; // 恒 Arrived
        let mut out = MotionOut::default();
        for i in 0..2 {
            assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "第 {i} 拍不得终值");
        }
        let goals = r.nav.lock().await.goals.clone();
        assert_eq!(
            goals[0..5],
            [(56.0, 56.0), (72.0, 56.0), (72.0, 72.0), (56.0, 72.0), (56.0, 56.0)],
            "一圈走完必须回角 1：{goals:?}"
        );
        assert_eq!(goals.len(), 8, "两拍各走完一圈：{goals:?}");
    }

    // ------------------------------------------------------------ 协助请求 v2（task_41 §5.3）

    /// 首次看到界内目标 → 立即发一条（x/y = 目标世界坐标）；阈值常量不得被改
    #[tokio::test]
    async fn test_assist_sends_on_first_seen() {
        assert_eq!(ASSIST_MOVE_M, 1.0, "重发阈值写死 1 m（task_41 §二.9）");
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let mut r = rig(
            FakeFeed::new(vec![target(64.0, 64.0, 1)]),
            (56.0, 56.0),
            false,
            Some(publisher),
        )
        .await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "跟随不得终值");
        assert_eq!(fake.count(), 1, "首次必须发一条");
        assert_eq!(fake.sent(), vec![(64.0, 64.0)], "发的是目标坐标");
    }

    /// 相对上次发送点 < 1 m（0.9 / 0.8 m）→ **不**重发
    #[tokio::test]
    async fn test_assist_silent_within_one_meter() {
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let feed = FakeFeed::new(vec![target(64.0, 64.0, 1)]);
        let mut r = rig(feed.clone(), (56.0, 56.0), false, Some(publisher)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 1, "首拍已发");

        feed.set(vec![target(64.9, 64.0, 2)]); // 0.9 m
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 1, "0.9 m 不重发");

        feed.set(vec![target(63.2, 64.0, 3)]); // 相对**上次发送点** 0.8 m
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 1, "0.8 m 不重发（阈值针对上次发送点，不是逐拍）");
    }

    /// 相对上次发送点 > 1 m（1.1 m）→ 重发，且每次刷新发送点
    #[tokio::test]
    async fn test_assist_sends_after_one_meter() {
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let feed = FakeFeed::new(vec![target(64.0, 64.0, 1)]);
        let mut r = rig(feed.clone(), (56.0, 56.0), false, Some(publisher)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 1);

        feed.set(vec![target(65.1, 64.0, 2)]); // 1.1 m
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 2, "1.1 m 必须重发");
        assert_eq!(fake.sent()[1], (65.1, 64.0));

        feed.set(vec![target(66.2, 64.0, 3)]); // 相对新发送点再 1.1 m
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 3, "发送点已被刷新");
    }

    /// 人出正方形 → 清状态；再回界内（**同一坐标**）→ 立即重发
    #[tokio::test]
    async fn test_assist_resets_after_leaving_square() {
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let feed = FakeFeed::new(vec![target(64.0, 64.0, 1)]);
        let mut r = rig(feed.clone(), (56.0, 56.0), false, Some(publisher)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 1);

        feed.set(vec![target(64.0, 90.0, 2)]); // 出界
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running, "出界 → 走巡逻");
        assert_eq!(fake.count(), 1, "出界那一拍不发");

        feed.set(vec![target(64.0, 64.0, 3)]); // 回到界内（与上次发送点同坐标）
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 2, "重返界内必须立即发（状态已清）");
        assert_eq!(fake.sent()[1], (64.0, 64.0));
    }

    /// `publisher = None` → 一条都不发，但条件照判（跟随分支仍 `Running`，不进 D\*）
    #[tokio::test]
    async fn test_assist_none_publisher_never_sends() {
        let fake = FakePublisher::new(); // 故意不交给树
        let feed = FakeFeed::new(vec![target(64.0, 64.0, 1)]);
        let mut r = rig(feed.clone(), (56.0, 56.0), false, None).await;
        let mut out = MotionOut::default();
        for i in 0..3u32 {
            if i > 0 {
                // 每拍移动 2 m（> 1 m 阈值）：有 publisher 时必发，None 时必不发
                feed.set(vec![target(64.0 + 2.0 * i as f32, 64.0, u64::from(i) + 1)]);
            }
            assert_eq!(
                r.tree.tick(&mut frame(&mut out)).await,
                Status::Running,
                "第 {i} 拍仍在跟随（不终值）"
            );
        }
        assert_eq!(fake.count(), 0, "无 publisher 不得发");
        assert_eq!(
            r.nav.lock().await.set_goal_calls.load(Ordering::SeqCst),
            0,
            "跟随分支不进 D*（条件照旧成功）"
        );
    }

    /// 目标坐标非有限（NaN）→ 不算入侵者、不发；恢复有限坐标 → 立即发
    #[tokio::test]
    async fn test_assist_skips_non_finite() {
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let feed = FakeFeed::new(vec![target(f32::NAN, 64.0, 1)]);
        let mut r = rig(feed.clone(), (56.0, 56.0), false, Some(publisher)).await;
        let mut out = MotionOut::default();
        assert_eq!(
            r.tree.tick(&mut frame(&mut out)).await,
            Status::Running,
            "NaN 目标 → 不是入侵者 → 走巡逻"
        );
        assert_eq!(fake.count(), 0, "坐标非有限值不发");

        feed.set(vec![target(64.0, 64.0, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 1, "恢复有限坐标 → 立即发（状态已清）");
    }

    /// `halt()` 清 `last_sent` → 树重进时同一坐标也重发
    #[tokio::test]
    async fn test_assist_halt_clears_state() {
        let fake = FakePublisher::new();
        let publisher: Arc<dyn AssistPublisher> = fake.clone();
        let feed = FakeFeed::new(vec![target(64.0, 64.0, 1)]);
        let mut r = rig(feed.clone(), (56.0, 56.0), false, Some(publisher)).await;
        let mut out = MotionOut::default();
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 1);

        r.tree.halt();
        feed.set(vec![target(64.0, 64.0, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(fake.count(), 2, "halt 清了状态 → 同一坐标也重发");
    }

    /// 树形（task_43 §七 实施步骤 4）：外层**必须是 `ReactiveFallback`**，不是记忆式 `Fallback`。
    ///
    /// `Node`（`pleiades-base/src/Robot/core/behavior_tree/node.rs:22`）只有 `tick` / `halt`，没有
    /// `as_any`；`build()` 返回 `Box<dyn Node>` ⇒ 无法 downcast，而 base 零改动（口径：不改 base）。
    /// 故用**只有 `ReactiveFallback` 才过得去**的行为判定：记忆式 `Fallback` 一旦锁在第 0 分支就
    /// 永不再重判，`ReactiveFallback` 每拍从第 0 子重判（`follow.rs` / `containment.rs` 文档同款论述）。
    #[tokio::test]
    async fn test_tree_shape_outer_is_reactive_fallback() {
        let me = (56.0, 56.0);
        let feed = FakeFeed::new(vec![target(64.0, 64.0, 1)]);
        let mut r = rig(feed.clone(), me, false, None).await;
        let mut out = MotionOut::default();

        // ① 界内 → 走跟随分支：几何给格、不进 D*
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, follow::step_cell(me, (64.0, 64.0)), "界内必须走 Follow");
        assert_eq!(r.nav.lock().await.set_goal_calls.load(Ordering::SeqCst), 0, "跟随分支不得进 D*");

        // ② 出界 → **同一拍**切到巡逻（记忆式 Fallback 会从这一拍起永远停在巡逻分支）
        feed.set(vec![target(64.0, 90.0, 2)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(r.nav.lock().await.goals, vec![(56.0, 56.0)], "出界 → 从角 1 起巡逻");
        assert_eq!(out.next_cell, Some(SENTINEL), "这一拍走的是巡逻分支（假 nav 的哨兵格）");

        // ③ 回界内 → **同一拍**切回跟随（记忆式会停在巡逻分支 → 本断言必红）
        feed.set(vec![target(64.0, 64.0, 3)]);
        assert_eq!(r.tree.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(
            out.next_cell,
            follow::step_cell(me, (64.0, 64.0)),
            "外层必须是 ReactiveFallback：每拍重判第 0 分支（记忆式 Fallback 会锁死在巡逻上）"
        );
        assert_eq!(
            r.nav.lock().await.set_goal_calls.load(Ordering::SeqCst),
            1,
            "切回跟随不得再调 set_goal（只有第 ② 拍角 1 那一次）"
        );
    }
}
