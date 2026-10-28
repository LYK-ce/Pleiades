//Presented by KeJi
//Created Date ： 2026-10-28
//Modified Date ： 2026-10-28

//! 直线跟随树（task_37 §六）：sim **机栈独有**，真机 / 车栈收到 `FollowStraight` 只建平凡树。
//!
//! 为什么不用 §五 的 `follow.rs`：那条路给导航器设目标 + D*，4 连通「下一格」在近距离会给
//! 侧向格，executor 原地转 90°、`Turning` 过冲又不反向修正（§六.2 实测）。本文件**完全不碰
//! 导航器**（不给目标、不取下一步、不清目标）：
//!
//! ```text
//! StraightLeaf（单叶，一拍算完）
//!   me = RobotState(x, y, attitude.yaw)
//!   s  = TargetSource::sample(now, me)      ← 复用 §五 的 detector 端口 + 本地 age 追踪
//!   out = decide(s, params, last)           ← 纯函数：Hold / Advance（距离带 + 迟滞）
//!   Frame.out.next_cell = match out { Hold => None, Advance => Some(front_cell(me, yaw)) }
//!   返回 Running                            ← 持续任务，**绝不终值**
//! ```
//!
//! **本文件不给导航器下达任何目标、也不索取它的下一格**（§六.3 硬要求）：目标恒在机头正前方
//! ⇒ executor 的 `delta ≈ 0` ⇒ 只发 `MoveForward`。代价：**不转向、不绕障、不修正横向偏差**
//! （§六.6 不做）。
//!
//! **`age` 的时钟口径**沿用 `follow.rs`：`Target.frame_ts_ms` 在 sim 是世界时间、没有「当前世界
//! 时间」可比，所以 `TargetSource` 用本地单调 `Instant` 记「首次见到该 `frame_seq`」的时刻。

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::RwLock;

use pleiades_base::robot::core::behavior_tree::{Frame, Node, Status};
use pleiades_base::robot::core::grid::{world_to_grid, CELL_RESOLUTION};
use pleiades_base::robot::core::state::RobotState;

use crate::sim::uav_sim::mission::follow::{Sample, TargetSource};

// 本文件**不引 `tracing`**：动作日志由 `sim_handler.rs` 的 `[Sim] 位置=… 动作=…` 覆盖，
// 10 Hz tick 再加日志只会刷屏。

// ---------------------------------------------------------------- 参数初值（§六.4）

/// `d > D_FAR_M` 才前进（米）
pub const D_FAR_M: f32 = 4.0;
/// `d <= D_NEAR_M` 停下（米）
pub const D_NEAR_M: f32 = 2.5;
/// 目标过期时间（毫秒）：`age > STALE_MS` → 视为丢失（§六.4）
pub const STALE_MS: u64 = 800;

/// 直线跟随参数（纯值 + `Default`；生产用默认值，单测可改）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StraightParams {
    pub d_far: f32,
    pub d_near: f32,
    pub stale_ms: u64,
}

impl Default for StraightParams {
    fn default() -> Self {
        Self { d_far: D_FAR_M, d_near: D_NEAR_M, stale_ms: STALE_MS }
    }
}

// ---------------------------------------------------------------- 纯函数

/// 本拍结论（[`decide`] 的返回值；**纯值**，单测主战场）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StraightOutcome {
    /// 无目标 / 过期 / `d <= d_near` → 不动（`next_cell = None`，不追旧点）
    Hold,
    /// `d > d_far`（或距离带内且上拍在走）→ 机头正前方一格
    Advance,
}

/// 本拍结论（**纯函数**：不碰时钟、不碰位姿、不碰导航器）
///
/// 三档 + 迟滞（§六.3 的两条阈值 + §六.4 的三个参数）：
/// - 无目标 / `age > stale` → `Hold`（丢掉，不追旧点）
/// - `d > d_far` → `Advance`（人走远了，往前飞）
/// - `d <= d_near` → `Hold`（到站，停）
/// - 带内（`d_near < d <= d_far`）→ **沿用上拍** `prev`（迟滞：不在 4.0 m 边界上抖）
pub fn decide(
    target: Option<&Sample>,
    params: &StraightParams,
    prev: StraightOutcome,
) -> StraightOutcome {
    let Some(s) = target else { return StraightOutcome::Hold };
    if s.age_ms > params.stale_ms {
        return StraightOutcome::Hold; // 严格大于：与 §五.5「超过即视为丢失」同口径
    }
    if s.d > params.d_far {
        return StraightOutcome::Advance;
    }
    if s.d <= params.d_near {
        return StraightOutcome::Hold;
    }
    prev // 距离带内：沿用上拍
}

/// 机头正前方一格的**格坐标**（§六.3：沿 `pose.yaw` 前进 `CELL_RESOLUTION` = 0.5 m）
///
/// 实现 = 「把机头前方 0.5 m 那个点落到的格」。因为 `grid::world_to_grid` 是 floor 语义、
/// 且 0.5 = `CELL_RESOLUTION`，**轴向 yaw 下它恒等于「当前格 +1 格」**
/// （`floor(x/0.5 + 1) = floor(x/0.5) + 1`）；格心口径与 `executor.rs::cell_center()`
/// （`(g + 0.5) × 0.5`）一致，故 `angle_to_target` 的 `delta ≈ 0`。
pub fn front_cell(me: (f32, f32), yaw: f32) -> (i32, i32) {
    let (c, s) = (yaw.cos(), yaw.sin());
    world_to_grid(me.0 + c * CELL_RESOLUTION, me.1 + s * CELL_RESOLUTION)
}

// ---------------------------------------------------------------- 节点

/// 单叶：一拍算完（没有分支 → 不需要 `ReactiveFallback`）
pub struct StraightLeaf {
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<TargetSource>,
    params: StraightParams,
    /// 上拍结论（距离带内沿用 → 迟滞状态）
    last: StraightOutcome,
}

impl StraightLeaf {
    fn new(
        robot_state: Arc<RwLock<RobotState>>,
        source: Arc<TargetSource>,
        params: StraightParams,
    ) -> Self {
        Self { robot_state, source, params, last: StraightOutcome::Hold }
    }
}

#[async_trait]
impl Node for StraightLeaf {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        // 位姿先读完再放锁（同 goto.rs / follow.rs 的纪律：持锁跨 await 会挤设备端 RX）
        let (x, y, yaw) = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y, rs.attitude.yaw)
        };
        let s = self.source.sample(Instant::now(), (x, y));
        let out = decide(s.as_ref(), &self.params, self.last);
        self.last = out;
        f.out.next_cell = match out {
            StraightOutcome::Hold => None,
            StraightOutcome::Advance => Some(front_cell((x, y), yaw)),
        };
        // 持续任务：**不终值**（`Success` 会让队列把这条弹走，人就再也跟不上了）
        Status::Running
    }

    /// 只清自身状态、不发 Stop（§十一 #23）；幂等
    fn halt(&mut self) {
        self.last = StraightOutcome::Hold;
    }
}

/// 建树（每次入队新建实例，§十一 #12）
pub fn build(
    robot_state: Arc<RwLock<RobotState>>,
    source: Arc<TargetSource>,
    params: StraightParams,
) -> Box<dyn Node> {
    Box::new(StraightLeaf::new(robot_state, source, params))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex as StdMutex;

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    use crate::sim::detector::Target;
    use crate::sim::uav_sim::mission::follow::TargetFeed;

    // ------------------------------------------------------------ 夹具

    /// 合成 detector 目标（`range_m` 与 `d` 同值）
    fn target(x: f32, y: f32, seq: u64) -> Target {
        Target { x, y, range_m: (x * x + y * y).sqrt(), bearing_deg: 0.0, frame_ts_ms: 0, frame_seq: seq }
    }

    /// 合成一拍快照（`decide` 只读 `d` 与 `age_ms`）
    fn sample(d: f32, age_ms: u64) -> Sample {
        Sample { x: 0.0, y: d, d, age_ms, frame_seq: 1, range_m: d }
    }

    fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
        match m.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(), // 锁中毒取回内层，不二次 panic
        }
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

    async fn set_pose(state: &Arc<RwLock<RobotState>>, x: f32, y: f32, yaw: f32) {
        let mut rs = state.write().await;
        rs.x = x;
        rs.y = y;
        rs.attitude.yaw = yaw;
    }

    fn frame(out: &mut MotionOut) -> Frame<'_> {
        Frame { dt_ms: 50, tick_no: 0, out }
    }

    /// 建树（`detector` 用 `FakeFeed` 顶替 → 不加载模型），同时返回位姿句柄给需要挪机的用例
    async fn tree(
        feed: Arc<FakeFeed>,
        x: f32,
        y: f32,
        yaw: f32,
    ) -> (Box<dyn Node>, Arc<RwLock<RobotState>>) {
        let state = Arc::new(RwLock::new(RobotState::default()));
        set_pose(&state, x, y, yaw).await;
        let source = Arc::new(TargetSource::new(Some(feed as Arc<dyn TargetFeed>)));
        (build(state.clone(), source, StraightParams::default()), state)
    }

    // ------------------------------------------------------------ 纯函数

    /// C3：`front_cell` 四个轴向（含 `yaw = 90° → +y`，§六.5-2 的第四条）
    #[test]
    fn test_front_cell_axis_aligned() {
        use std::f32::consts::{FRAC_PI_2, PI};
        let me = (0.25, 0.25); // 格 (0,0) 的格心
        assert_eq!(front_cell(me, 0.0), (1, 0), "yaw=0 → +x");
        assert_eq!(front_cell(me, FRAC_PI_2), (0, 1), "yaw=90° → +y");
        assert_eq!(front_cell(me, PI), (-1, 0), "yaw=180° → -x");
        assert_eq!(front_cell(me, -FRAC_PI_2), (0, -1), "yaw=-90° → -y");
    }

    /// C3：格内偏心 0.2 m 也要取到「当前格 +1 格」（floor 语义的推论，防有人改成 round）
    #[test]
    fn test_front_cell_off_center_is_next_cell() {
        for cx in 0..4 {
            for cy in 0..4 {
                let me = (cx as f32 * 0.5 + 0.1, cy as f32 * 0.5 + 0.4);
                assert_eq!(world_to_grid(me.0, me.1), (cx, cy), "me={me:?}");
                assert_eq!(front_cell(me, 0.0), (cx + 1, cy), "me={me:?} → +x 一格");
            }
        }
    }

    /// C2：`decide` 三档 + 两个边界 + 迟滞 + 丢失优先级
    #[test]
    fn test_decide_far_near_band_and_stale() {
        let p = StraightParams::default();
        // 无目标：即使上拍在走也必须停
        assert_eq!(decide(None, &p, StraightOutcome::Advance), StraightOutcome::Hold, "无目标 → Hold");
        // stale 边界：> 800 才算丢
        assert_eq!(
            decide(Some(&sample(5.0, STALE_MS)), &p, StraightOutcome::Hold),
            StraightOutcome::Advance,
            "age == stale 不过期"
        );
        assert_eq!(
            decide(Some(&sample(5.0, STALE_MS + 1)), &p, StraightOutcome::Advance),
            StraightOutcome::Hold,
            "age == stale+1 必须 Hold"
        );
        // d_far 边界：> 4.0 才走
        assert_eq!(
            decide(Some(&sample(D_FAR_M, 0)), &p, StraightOutcome::Hold),
            StraightOutcome::Hold,
            "d == d_far → 带内（沿用上拍 Hold）"
        );
        assert_eq!(
            decide(Some(&sample(D_FAR_M + 0.1, 0)), &p, StraightOutcome::Hold),
            StraightOutcome::Advance
        );
        // d_near 边界：<= 2.5 停（即使上拍在走）
        assert_eq!(
            decide(Some(&sample(D_NEAR_M, 0)), &p, StraightOutcome::Advance),
            StraightOutcome::Hold,
            "d == d_near → Hold"
        );
        assert_eq!(
            decide(Some(&sample(D_NEAR_M + 0.1, 0)), &p, StraightOutcome::Advance),
            StraightOutcome::Advance,
            "带内沿用上拍 Advance"
        );
        // 带内两向都验（迟滞是真的沿用，不是固定值）
        assert_eq!(decide(Some(&sample(3.0, 0)), &p, StraightOutcome::Hold), StraightOutcome::Hold);
        assert_eq!(decide(Some(&sample(3.0, 0)), &p, StraightOutcome::Advance), StraightOutcome::Advance);
    }

    // ------------------------------------------------------------ 树级

    /// C1：目标远（d=5）→ `next_cell` = 机头前方一格、`Running`
    #[tokio::test]
    async fn test_tree_far_gives_front_cell() {
        // 人 (0.25, 5.25)：正前方 5 m；机在格 (0,0) 格心、yaw=0
        let feed = FakeFeed::new(vec![target(0.25, 5.25, 1)]);
        let (mut t, _state) = tree(feed, 0.25, 0.25, 0.0).await;
        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running, "直线跟随不得终值");
        assert_eq!(out.next_cell, Some((1, 0)), "d=5 > 4 → 机头前方一格");
        // 第二拍：目标没动 → 仍是同一格（不抖）
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((1, 0)));
    }

    /// C1：`next_cell` 每拍按**当前位姿**重算（挪机后前进格跟着走）
    #[tokio::test]
    async fn test_tree_front_cell_recomputed_from_pose() {
        let feed = FakeFeed::new(vec![target(0.25, 9.25, 1)]);
        let (mut t, state) = tree(feed, 0.25, 0.25, 0.0).await;
        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((1, 0)));
        // 机已飞到格 (2,0) 的格心
        set_pose(&state, 1.25, 0.25, 0.0).await;
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((3, 0)), "应按新位姿给下一格");
    }

    /// C2：目标近（d == 2.5）→ `next_cell = None`
    #[tokio::test]
    async fn test_tree_near_holds() {
        let feed = FakeFeed::new(vec![target(0.25, 2.75, 1)]);
        let (mut t, _state) = tree(feed, 0.25, 0.25, 0.0).await;
        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, None, "d == 2.5 → Hold");
    }

    /// C2：无目标 → `None` 且连拍 `Running`；目标回来后立刻前进
    #[tokio::test]
    async fn test_tree_lost_holds_and_resumes() {
        let feed = FakeFeed::new(vec![]);
        let (mut t, _state) = tree(feed.clone(), 0.25, 0.25, 0.0).await;
        let mut out = MotionOut::default();
        for i in 0..3 {
            assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(out.next_cell, None, "无目标不得给下一格");
        }
        feed.set(vec![target(0.25, 9.25, 2)]);
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((1, 0)), "目标出现且远 → 立刻前进");
    }

    /// C2：`yaw = 90°` 时前方格方向正确（走树级，不只测纯函数）
    #[tokio::test]
    async fn test_tree_yaw_90_front_cell() {
        use std::f32::consts::FRAC_PI_2;
        let feed = FakeFeed::new(vec![target(0.25, 5.25, 1)]); // 机头方向（+y）前方 5 m
        let (mut t, _state) = tree(feed, 0.25, 0.25, FRAC_PI_2).await;
        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((0, 1)), "yaw=90° → 前方格在 +y");
    }

    /// `halt` 幂等；halt 后重新按位姿决策（不清队列、不发 Stop）
    #[tokio::test]
    async fn test_tree_halt_is_idempotent() {
        let feed = FakeFeed::new(vec![target(0.25, 5.25, 1)]);
        let (mut t, _state) = tree(feed, 0.25, 0.25, 0.0).await;
        let mut out = MotionOut::default();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        t.halt();
        t.halt();
        assert_eq!(t.tick(&mut frame(&mut out)).await, Status::Running);
        assert_eq!(out.next_cell, Some((1, 0)), "halt 后照样前进");
    }
}
