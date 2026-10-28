//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! Goto 树（Task 36 §12.4.6）
//!
//! ```text
//! Timeout(60_000 ms)                          ← 业务超时
//! └─ Fallback
//!    ├─ GoTo(x, y)                            ← 主路
//!    └─ Sequence                              ← 恢复路（只重试一次）
//!       ├─ Wait(3_000 ms)
//!       └─ GoTo(x, y)
//! ```
//!
//! **上报不在树里**：失败码随 `Status` 回到驱动，由驱动记录/上报（§十一 #32）。

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{
    FailCode, Fallback, Frame, Node, Sequence, Status, Timeout, Wait,
};
use pleiades_base::robot::core::state::RobotState;

use crate::uav::nav::{BlockedCause, NavPort, NavStatus};

/// 业务超时（毫秒）：树内唯一的兜底时限，写在文件顶部便于后续调（§12.4.6）
const TASK_TIMEOUT_MS: u32 = 60_000;
/// 恢复路等待（毫秒）：主路失败后等这么久再试一次
const RETRY_WAIT_MS: u32 = 3_000;

/// 构造 Goto 树（**每次新建实例**：节点带 RUNNING 位置 / 计时，不复用，§十一 #12）
pub fn build<P: NavPort + 'static>(
    x: f32,
    y: f32,
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
) -> Box<dyn Node> {
    Box::new(Timeout::new(
        TASK_TIMEOUT_MS,
        Box::new(Fallback::new(vec![
            Box::new(GoToLeaf::new(x, y, nav.clone(), robot_state.clone())),
            Box::new(Sequence::new(vec![
                Box::new(Wait::new(RETRY_WAIT_MS)),
                Box::new(GoToLeaf::new(x, y, nav, robot_state)),
            ])),
        ])),
    ))
}

/// 叶子：判到达 → 算下一格 → 写 `Frame.out`（§十一 #25 / #32，算路在叶子里）
pub struct GoToLeaf<P: NavPort> {
    x: f32,
    y: f32,
    /// 是否已把目标递给导航器；`halt` 清掉 → 下次 tick 重新武装（§十一 #17 / #38）
    armed: bool,
    nav: Arc<Mutex<P>>,
    robot_state: Arc<RwLock<RobotState>>,
}

impl<P: NavPort> GoToLeaf<P> {
    pub fn new(x: f32, y: f32, nav: Arc<Mutex<P>>, robot_state: Arc<RwLock<RobotState>>) -> Self {
        Self { x, y, armed: false, nav, robot_state }
    }
}

#[async_trait]
impl<P: NavPort + 'static> Node for GoToLeaf<P> {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        // 位姿：先读完再放读锁（不在持读锁时等导航锁）
        let rs = self.robot_state.read().await.clone();
        let mut nav = self.nav.lock().await;

        // 重新武装：未武装，或目标被清过（抢占恢复 / reset，§十一 #17 / #38）
        if !self.armed || nav.status() == NavStatus::Idle {
            nav.set_goal(self.x, self.y);
            self.armed = true;
        }

        let cell = nav.step(&rs).await;
        let status = nav.status();
        drop(nav);

        match status {
            NavStatus::Arrived => {
                f.out.next_cell = None;
                Status::Success
            }
            NavStatus::Blocked(cause) => {
                f.out.next_cell = None;
                // 只用**通用码**（§十一 #31）：动态被堵 = Blocked / 地图无路 = Unreachable
                let code = match cause {
                    BlockedCause::Dynamic => FailCode::Blocked,
                    BlockedCause::NoPath => FailCode::Unreachable,
                };
                Status::Failure(code)
            }
            // Navigating（或刚清空的 Idle）：本拍仍给「下一格」；`None` = 这一拍不动
            _ => {
                f.out.next_cell = cell;
                Status::Running
            }
        }
    }

    /// 只清自身状态、不发 Stop（§十一 #23）；幂等
    fn halt(&mut self) {
        self.armed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    /// 假导航端口：按 `plan` 逐拍给出状态，用尽后重复最后一个
    struct FakeNav {
        plan: Vec<NavStatus>,
        at: usize,
        status: NavStatus,
        set_goal_calls: u32,
    }

    impl FakeNav {
        fn new(plan: Vec<NavStatus>) -> Self {
            Self { plan, at: 0, status: NavStatus::Idle, set_goal_calls: 0 }
        }
    }

    #[async_trait]
    impl NavPort for FakeNav {
        fn set_goal(&mut self, _x: f32, _y: f32) {
            self.set_goal_calls += 1;
            self.status = NavStatus::Navigating;
        }
        async fn step(&mut self, _rs: &RobotState) -> Option<(i32, i32)> {
            let st = *self
                .plan
                .get(self.at)
                .unwrap_or_else(|| self.plan.last().expect("plan 不得为空"));
            if self.at + 1 < self.plan.len() { self.at += 1; }
            self.status = st;
            match st { NavStatus::Navigating => Some((3, 4)), _ => None }
        }
        fn status(&self) -> NavStatus { self.status }
    }

    /// 假端口的 `set_goal` 调用计数（`Arc<Mutex<FakeNav>>` 外部读）
    async fn calls(nav: &Arc<Mutex<FakeNav>>) -> u32 {
        nav.lock().await.set_goal_calls
    }

    fn leaf(x: f32, y: f32, nav: &Arc<Mutex<FakeNav>>) -> GoToLeaf<FakeNav> {
        GoToLeaf::new(x, y, nav.clone(), Arc::new(RwLock::new(RobotState::default())))
    }

    fn frame(out: &mut MotionOut, dt_ms: u32) -> Frame<'_> {
        Frame { dt_ms, tick_no: 0, out }
    }

    /// #5a 叶子重新武装：`halt` 后再 tick → 重新 `set_goal`
    #[tokio::test]
    async fn test_leaf_rearm_after_halt() {
        let nav = Arc::new(Mutex::new(FakeNav::new(vec![NavStatus::Navigating])));
        let mut node = leaf(1.0, 2.0, &nav);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(calls(&nav).await, 1, "首拍应武装 set_goal");

        node.halt();
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(calls(&nav).await, 2, "halt 后应重新武装");
    }

    /// #5b 目标被清过（`status == Idle`）→ 下一拍自动重新 `set_goal`
    #[tokio::test]
    async fn test_leaf_rearm_when_nav_idle() {
        let nav = Arc::new(Mutex::new(FakeNav::new(vec![
            NavStatus::Navigating,
            NavStatus::Idle,
        ])));
        let mut node = leaf(1.0, 2.0, &nav);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(calls(&nav).await, 1);
        // 第 2 拍导航器报 Idle（目标被清）：本拍不重设（已武装），但状态已是 Idle
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(calls(&nav).await, 1);
        // 第 3 拍看到 Idle → 重新武装
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(calls(&nav).await, 2);
    }

    /// #6 主路：假端口第 3 拍 `Arrived` → 整棵树 `Success`，且输出清空
    #[tokio::test]
    async fn test_goto_tree_main_path() {
        let nav = Arc::new(Mutex::new(FakeNav::new(vec![
            NavStatus::Navigating,
            NavStatus::Navigating,
            NavStatus::Arrived,
        ])));
        let mut tree = build(1.0, 2.0, nav.clone(), Arc::new(RwLock::new(RobotState::default())));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(f.out.next_cell, Some((3, 4)));
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(tree.tick(&mut f).await, Status::Success);
        assert_eq!(f.out.next_cell, None, "成功那拍必须清输出");
        assert_eq!(calls(&nav).await, 1, "主路只武装一次");
    }

    /// #7 恢复路：主路 `Blocked` → 等 3 s → 第二次成功
    #[tokio::test]
    async fn test_goto_tree_recovery_path() {
        let nav = Arc::new(Mutex::new(FakeNav::new(vec![
            NavStatus::Blocked(BlockedCause::Dynamic),
            NavStatus::Navigating,
            NavStatus::Arrived,
        ])));
        let mut tree = build(1.0, 2.0, nav.clone(), Arc::new(RwLock::new(RobotState::default())));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        // 第 1 拍：主路失败 → 进恢复路的 Wait
        assert_eq!(tree.tick(&mut f).await, Status::Running);

        // 第 2..60 拍：Wait 未满（elapsed = 50*n ≤ 3000）
        for _ in 2..=60 {
            assert_eq!(tree.tick(&mut f).await, Status::Running);
        }
        // 第 61 拍：elapsed = 3050 > 3000 → Wait Success，同拍第二个叶子武装并 `Running`
        assert_eq!(tree.tick(&mut f).await, Status::Running);
        assert_eq!(calls(&nav).await, 2, "恢复路应再武装一次");
        // 第 62 拍：Arrived → Success
        assert_eq!(tree.tick(&mut f).await, Status::Success);
    }

    /// #8 超时：一直 `Navigating` → 第 1200 拍 `Failure(Timeout)`
    #[tokio::test]
    async fn test_goto_tree_timeout() {
        let nav = Arc::new(Mutex::new(FakeNav::new(vec![NavStatus::Navigating])));
        let mut tree = build(1.0, 2.0, nav.clone(), Arc::new(RwLock::new(RobotState::default())));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        for i in 1..=1199 {
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍不应超时");
        }
        assert_eq!(
            tree.tick(&mut f).await,
            Status::Failure(FailCode::Timeout),
            "第 1200 拍应超时（Timeout 先加 dt_ms 再判 >= 60000）"
        );
    }
}