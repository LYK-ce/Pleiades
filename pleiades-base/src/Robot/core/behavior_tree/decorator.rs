//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 装饰节点（Task 36 §12.2.4）
//!
//! 计时 / 计数全部按 `Frame.dt_ms` 累积，**不看墙钟**（§十一 #17：挂起期间时间冻结）。
//! `halt` 一律"清自己的计数 + 向下 halt"，且幂等（#23）。

use async_trait::async_trait;

use super::{FailCode, Frame, Node, Status};

/// 超时：`dt_ms` 累加，**先判超时再 tick 子**；超限 → `child.halt()` + `Failure(Timeout)`
pub struct Timeout {
    limit_ms: u32,
    elapsed_ms: u32,
    child: Box<dyn Node>,
}

impl Timeout {
    pub fn new(limit_ms: u32, child: Box<dyn Node>) -> Self {
        Self { limit_ms, elapsed_ms: 0, child }
    }
}

#[async_trait]
impl Node for Timeout {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        self.elapsed_ms = self.elapsed_ms.saturating_add(f.dt_ms);
        if self.elapsed_ms >= self.limit_ms {
            // 超时是它造成的，所以用自己的码（§12.2.4 失败码传递表）
            self.child.halt();
            return Status::Failure(FailCode::Timeout);
        }
        self.child.tick(f).await
    }

    fn halt(&mut self) {
        self.elapsed_ms = 0;
        self.child.halt();
    }
}

/// 重试：子 `Failure` → 计数 +1、`child.halt()`、下拍重试；计满 → `Failure(RetryExhausted)`
pub struct Retry {
    /// 允许的尝试次数上限（`0` 按 `1` 处理）
    limit: u32,
    attempts: u32,
    child: Box<dyn Node>,
}

impl Retry {
    pub fn new(limit: u32, child: Box<dyn Node>) -> Self {
        Self { limit: limit.max(1), attempts: 0, child }
    }
}

#[async_trait]
impl Node for Retry {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        match self.child.tick(f).await {
            Status::Success => {
                self.attempts = 0;
                Status::Success
            }
            Status::Running => Status::Running,
            Status::Failure(code) => {
                // 各次尝试的码只进日志；对外统一 RetryExhausted
                tracing::debug!(?code, attempts = self.attempts + 1, "behavior_tree: retry attempt failed");
                self.child.halt();
                self.attempts += 1;
                if self.attempts >= self.limit {
                    self.attempts = 0;
                    Status::Failure(FailCode::RetryExhausted)
                } else {
                    Status::Running
                }
            }
        }
    }

    fn halt(&mut self) {
        self.attempts = 0;
        self.child.halt();
    }
}

/// 门闩：`cond == false` → `Failure(Unspecified)`，**不 tick 子**；否则 tick 子
///
/// 关闭时**不** halt 子（§12.2.4 表未要求）：子停在原处，再开门时从暂停点继续。
pub struct Guard {
    cond: Box<dyn FnMut() -> bool + Send>,
    child: Box<dyn Node>,
}

impl Guard {
    pub fn new<F>(cond: F, child: Box<dyn Node>) -> Self
    where
        F: FnMut() -> bool + Send + 'static,
    {
        Self { cond: Box::new(cond), child }
    }
}

#[async_trait]
impl Node for Guard {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        if !(self.cond)() {
            return Status::Failure(FailCode::Unspecified);
        }
        self.child.tick(f).await
    }

    fn halt(&mut self) {
        self.child.halt();
    }
}

/// 取反：`Success ↔ Failure`，`Running` 原样
pub struct Inverter {
    child: Box<dyn Node>,
}

impl Inverter {
    pub fn new(child: Box<dyn Node>) -> Self {
        Self { child }
    }
}

#[async_trait]
impl Node for Inverter {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        match self.child.tick(f).await {
            Status::Success => Status::Failure(FailCode::Unspecified),
            Status::Failure(_) => Status::Success,
            Status::Running => Status::Running,
        }
    }

    fn halt(&mut self) {
        self.child.halt();
    }
}

/// 重复 n 次：子 `Success` → 计数 +1、`child.halt()`、下拍重 tick；计满 → `Success`
pub struct Repeat {
    /// 重复次数（`0` 按 `1` 处理）
    limit: u32,
    count: u32,
    child: Box<dyn Node>,
}

impl Repeat {
    pub fn new(limit: u32, child: Box<dyn Node>) -> Self {
        Self { limit: limit.max(1), count: 0, child }
    }
}

#[async_trait]
impl Node for Repeat {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        match self.child.tick(f).await {
            Status::Success => {
                self.count += 1;
                self.child.halt();
                if self.count >= self.limit {
                    self.count = 0;
                    Status::Success
                } else {
                    Status::Running
                }
            }
            Status::Running => Status::Running,
            Status::Failure(code) => {
                self.count = 0;
                Status::Failure(code)
            }
        }
    }

    fn halt(&mut self) {
        self.count = 0;
        self.child.halt();
    }
}

/// 永远重复：**不会自己结束**，只能被父 / 驱动 `halt`（§十二 Step 2 单测 #10）
pub struct RepeatForever {
    /// 已完成的轮数（仅供测试与日志；`halt` 清零）
    count: u64,
    child: Box<dyn Node>,
}

impl RepeatForever {
    pub fn new(child: Box<dyn Node>) -> Self {
        Self { count: 0, child }
    }
}

#[async_trait]
impl Node for RepeatForever {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        match self.child.tick(f).await {
            Status::Success => {
                self.count += 1;
                self.child.halt();
                Status::Running
            }
            Status::Running => Status::Running,
            Status::Failure(code) => {
                self.count = 0;
                Status::Failure(code)
            }
        }
    }

    fn halt(&mut self) {
        self.count = 0;
        self.child.halt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::super::{MotionOut, Sequence};

    struct Scripted {
        ticks: Arc<AtomicU32>,
        halts: Arc<AtomicU32>,
        plan: Vec<Status>,
        at: usize,
    }

    impl Scripted {
        fn new(plan: Vec<Status>) -> (Self, Arc<AtomicU32>, Arc<AtomicU32>) {
            let ticks = Arc::new(AtomicU32::new(0));
            let halts = Arc::new(AtomicU32::new(0));
            (
                Self { ticks: ticks.clone(), halts: halts.clone(), plan, at: 0 },
                ticks,
                halts,
            )
        }

        fn fixed(st: Status) -> (Self, Arc<AtomicU32>, Arc<AtomicU32>) {
            Self::new(vec![st])
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

        fn halt(&mut self) {
            self.halts.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn cnt(a: &Arc<AtomicU32>) -> u32 {
        a.load(Ordering::SeqCst)
    }

    fn frame(out: &mut MotionOut, dt_ms: u32) -> Frame<'_> {
        Frame { dt_ms, tick_no: 0, out }
    }

    fn boxed(n: impl Node + 'static) -> Box<dyn Node> {
        Box::new(n)
    }

    /// #7 Timeout：dt=50、limit=100 → 第二拍 Failure(Timeout)，且子被 halt
    #[tokio::test]
    async fn test_timeout_fires_and_halts_child() {
        let (c, t, h) = Scripted::fixed(Status::Running);
        let mut node = Timeout::new(100, boxed(c));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!((cnt(&t), cnt(&h)), (1, 0));

        assert_eq!(node.tick(&mut f).await, Status::Failure(FailCode::Timeout));
        assert_eq!(cnt(&h), 1);
        // 超时那一拍不再 tick 子
        assert_eq!(cnt(&t), 1);
    }

    /// #8 Retry：子连续失败，第 n 次后 Failure(RetryExhausted)；计数正确；子成功清计数
    #[tokio::test]
    async fn test_retry_exhausts_and_resets() {
        let (c, _, _) = Scripted::fixed(Status::Failure(FailCode::Blocked));
        let mut node = Retry::new(3, boxed(c));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(node.attempts, 1);
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(node.attempts, 2);
        assert_eq!(node.tick(&mut f).await, Status::Failure(FailCode::RetryExhausted));
        assert_eq!(node.attempts, 0);

        // 子成功 → Success 且清计数
        let (c2, _, _) = Scripted::new(vec![Status::Failure(FailCode::Blocked), Status::Success]);
        let mut node2 = Retry::new(3, boxed(c2));
        assert_eq!(node2.tick(&mut f).await, Status::Running);
        assert_eq!(node2.attempts, 1);
        assert_eq!(node2.tick(&mut f).await, Status::Success);
        assert_eq!(node2.attempts, 0);
    }

    /// #9 Repeat(3)：子 3 次 Success 后 Success
    #[tokio::test]
    async fn test_repeat_three_times() {
        let (c, t, _) = Scripted::fixed(Status::Success);
        let mut node = Repeat::new(3, boxed(c));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(node.count, 1);
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(node.count, 2);
        assert_eq!(node.tick(&mut f).await, Status::Success);
        assert_eq!(node.count, 0);
        assert_eq!(cnt(&t), 3);
    }

    /// #10 RepeatForever：一直 Running、不会自己结束；外部 halt 后状态清零
    #[tokio::test]
    async fn test_repeat_forever_only_ends_by_halt() {
        let (c, _, _) = Scripted::fixed(Status::Success);
        let mut node = RepeatForever::new(boxed(c));
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        for _ in 0..5 {
            assert_eq!(node.tick(&mut f).await, Status::Running);
        }
        assert_eq!(node.count, 5);

        node.halt();
        assert_eq!(node.count, 0);
        assert_eq!(node.tick(&mut f).await, Status::Running);
        assert_eq!(node.count, 1);
    }

    /// #13 Inverter：Success ↔ Failure，Running 不变
    #[tokio::test]
    async fn test_inverter_flips_terminals() {
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        let (c, _, _) = Scripted::new(vec![
            Status::Success,
            Status::Failure(FailCode::Blocked),
            Status::Running,
        ]);
        let mut node = Inverter::new(boxed(c));
        assert_eq!(node.tick(&mut f).await, Status::Failure(FailCode::Unspecified));
        assert_eq!(node.tick(&mut f).await, Status::Success);
        assert_eq!(node.tick(&mut f).await, Status::Running);

        // Sequence 的失败码被 Inverter 吃掉 → 换成 Unspecified
        let (c2, _, _) = Scripted::fixed(Status::Success);
        let (c3, _, _) = Scripted::fixed(Status::Failure(FailCode::Unreachable));
        let seq = Sequence::new(vec![boxed(c2), boxed(c3)]);
        let mut inv = Inverter::new(boxed(seq));
        assert_eq!(inv.tick(&mut f).await, Status::Success);
    }
}
