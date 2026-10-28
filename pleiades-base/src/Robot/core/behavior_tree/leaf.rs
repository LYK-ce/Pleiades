//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 通用叶子（Task 36 §12.2.4）
//!
//! 这里只放**不需要 I/O** 的叶子：`Wait`（等待）、`Action`（同步闭包）、`Condition`（同步判据）。
//! 需要 `await` 设备 API 的叶子（读状态、调导航、调 YOLO）由设备端 `impl Node` 自己写（§十一 #19）。

use async_trait::async_trait;

use super::{FailCode, Frame, Node, Status};

/// 等待：`dt_ms` 累加，未到 `Running`，跨过 `wait_ms` 才 `Success`
///
/// 边界：`elapsed > wait_ms`（严格大于）。`dt=50`、`wait=100` 的拍序是
/// `Running`（50）→ `Running`（100）→ `Success`（150），与 §12.2.6 单测 #11 一致。
pub struct Wait {
    wait_ms: u32,
    elapsed_ms: u32,
}

impl Wait {
    pub fn new(wait_ms: u32) -> Self {
        Self { wait_ms, elapsed_ms: 0 }
    }
}

#[async_trait]
impl Node for Wait {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        self.elapsed_ms = self.elapsed_ms.saturating_add(f.dt_ms);
        if self.elapsed_ms > self.wait_ms {
            self.elapsed_ms = 0;
            Status::Success
        } else {
            Status::Running
        }
    }

    fn halt(&mut self) {
        self.elapsed_ms = 0;
    }
}

/// 同步动作叶子：闭包直接拿到 `Frame`（可写 `out.next_cell`）
///
/// 只放**不 await** 的动作；需要 I/O 的叶子请直接 `impl Node`（§12.2.4）。
pub struct Action {
    f: Box<dyn for<'a, 'b> FnMut(&'a mut Frame<'b>) -> Status + Send>,
}

impl Action {
    pub fn new<F>(f: F) -> Self
    where
        F: for<'a, 'b> FnMut(&'a mut Frame<'b>) -> Status + Send + 'static,
    {
        Self { f: Box::new(f) }
    }
}

#[async_trait]
impl Node for Action {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        (self.f)(f)
    }

    /// 无状态
    fn halt(&mut self) {}
}

/// 同步判据叶子：瞬时返回 `Success` / `Failure(Unspecified)`，**从不 Running**
pub struct Condition {
    cond: Box<dyn FnMut() -> bool + Send>,
}

impl Condition {
    pub fn new<F>(cond: F) -> Self
    where
        F: FnMut() -> bool + Send + 'static,
    {
        Self { cond: Box::new(cond) }
    }
}

#[async_trait]
impl Node for Condition {
    async fn tick(&mut self, _f: &mut Frame<'_>) -> Status {
        if (self.cond)() {
            Status::Success
        } else {
            Status::Failure(FailCode::Unspecified)
        }
    }

    /// 无状态
    fn halt(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    use super::super::MotionOut;

    struct Scripted {
        ticks: Arc<AtomicU32>,
        plan: Vec<Status>,
        at: usize,
    }

    impl Scripted {
        fn fixed(st: Status) -> (Self, Arc<AtomicU32>) {
            let ticks = Arc::new(AtomicU32::new(0));
            (Self { ticks: ticks.clone(), plan: vec![st], at: 0 }, ticks)
        }
    }

    #[async_trait]
    impl Node for Scripted {
        async fn tick(&mut self, _f: &mut Frame<'_>) -> Status {
            self.ticks.fetch_add(1, Ordering::SeqCst);
            let st = *self
                .plan
                .get(self.at)
                .unwrap_or_else(|| self.plan.last().expect("plan 不得为空"));
            if self.at + 1 < self.plan.len() {
                self.at += 1;
            }
            st
        }

        fn halt(&mut self) {}
    }

    fn cnt(a: &Arc<AtomicU32>) -> u32 {
        a.load(Ordering::SeqCst)
    }

    fn frame(out: &mut MotionOut, dt_ms: u32) -> Frame<'_> {
        Frame { dt_ms, tick_no: 0, out }
    }

    /// #11 Wait：dt=50、wait=100 → Running, Running, Success；halt 后清零
    #[tokio::test]
    async fn test_wait_ticks() {
        let mut node = Wait::new(100);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(node.tick(&mut f).await, Status::Success);
        // 成功后自动归零：下一轮重新计时
        assert_eq!(node.tick(&mut f).await, Status::Running);

        // halt 清零
        assert_eq!(node.tick(&mut f).await, Status::Running);
        node.halt();
        assert_eq!(node.elapsed_ms, 0);
    }

    /// #12 Guard：cond=false 时子**未被** tick；cond=true 才 tick 子
    #[tokio::test]
    async fn test_guard_blocks_child() {
        let open = Arc::new(AtomicBool::new(false));
        let gate = open.clone();
        let (child, ticks) = Scripted::fixed(Status::Running);
        let mut node = super::super::Guard::new(move || gate.load(Ordering::SeqCst), Box::new(child));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(node.tick(&mut f).await, Status::Failure(FailCode::Unspecified));
        assert_eq!(cnt(&ticks), 0);

        open.store(true, Ordering::SeqCst);
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(cnt(&ticks), 1);
    }

    /// #15 Frame.out：叶子写入的 next_cell 驱动读得到；没写时是 None
    #[tokio::test]
    async fn test_frame_out_roundtrip() {
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        let mut act = Action::new(|f: &mut Frame<'_>| {
            f.out.next_cell = Some((3, 4));
            Status::Success
        });
        f.out.clear();
        assert_eq!(act.tick(&mut f).await, Status::Success);
        assert_eq!(f.out.next_cell, Some((3, 4)));

        // 没写输出的叶子：驱动 clear 之后仍是 None（None = 停车）
        let mut wait = Wait::new(0);
        f.out.clear();
        assert_eq!(wait.tick(&mut f).await, Status::Success);
        assert_eq!(f.out.next_cell, None);
    }
}
