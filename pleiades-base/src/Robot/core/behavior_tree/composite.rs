//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 组合节点（Task 36 §12.2.4）
//!
//! **记忆式与反应式必须分开用**（§十一 #24）：
//! - `Sequence` / `Fallback`：记住停在哪个子节点，下拍从那里继续；
//! - `ReactiveSequence` / `ReactiveFallback`：每拍从第 0 个子重判。
//!
//! 规则：树里"带条件的循环"一律用反应式，否则条件不再重判（"看到人走了还在跟"）。

use async_trait::async_trait;

use super::{FailCode, Frame, Node, Status};

/// 记忆式顺序：子 `Success` → 下一个；`Running` → 记住位置；`Failure` → 清记忆并原样传码
#[derive(Default)]
pub struct Sequence {
    children: Vec<Box<dyn Node>>,
    /// 下一个要 tick 的子下标（= 记忆位置）
    idx: usize,
}

impl Sequence {
    pub fn new(children: Vec<Box<dyn Node>>) -> Self {
        Self { children, idx: 0 }
    }
}

#[async_trait]
impl Node for Sequence {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        while self.idx < self.children.len() {
            let i = self.idx;
            match self.children[i].tick(f).await {
                Status::Success => self.idx += 1,
                Status::Running => return Status::Running,
                // 失败码原样传（§12.2.4 失败码传递表）
                Status::Failure(code) => {
                    self.idx = 0;
                    return Status::Failure(code);
                }
            }
        }
        self.idx = 0;
        Status::Success
    }

    fn halt(&mut self) {
        self.idx = 0;
        for c in &mut self.children {
            c.halt();
        }
    }
}

/// 反应式顺序：每拍从第 0 个子重判；条件类节点必须用它
#[derive(Default)]
pub struct ReactiveSequence {
    children: Vec<Box<dyn Node>>,
    /// 上一拍停在哪个子（只用于"本拍前序失败 → 停掉它"，不用于跳过重判）
    running: Option<usize>,
}

impl ReactiveSequence {
    pub fn new(children: Vec<Box<dyn Node>>) -> Self {
        Self { children, running: None }
    }
}

#[async_trait]
impl Node for ReactiveSequence {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        for i in 0..self.children.len() {
            let st = self.children[i].tick(f).await;
            match st {
                Status::Success => {}
                Status::Running => {
                    self.running = Some(i);
                    return Status::Running;
                }
                // 失败码原样传；同时停掉上一拍还挂在后面兄弟上的进度，避免状态残留
                Status::Failure(code) => {
                    if let Some(r) = self.running.take() {
                        if r != i {
                            self.children[r].halt();
                        }
                    }
                    return Status::Failure(code);
                }
            }
        }
        self.running = None;
        Status::Success
    }

    fn halt(&mut self) {
        self.running = None;
        for c in &mut self.children {
            c.halt();
        }
    }
}

/// 记忆式回退：子 `Failure` → 下一个（丢该分支的码）；`Running` → 记住位置；`Success` → 清记忆
#[derive(Default)]
pub struct Fallback {
    children: Vec<Box<dyn Node>>,
    idx: usize,
}

impl Fallback {
    pub fn new(children: Vec<Box<dyn Node>>) -> Self {
        Self { children, idx: 0 }
    }
}

#[async_trait]
impl Node for Fallback {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        let mut last = FailCode::Unspecified;
        while self.idx < self.children.len() {
            let i = self.idx;
            match self.children[i].tick(f).await {
                // 换分支：丢弃该分支的码，只留最后一个
                Status::Failure(code) => {
                    last = code;
                    self.idx += 1;
                }
                Status::Running => return Status::Running,
                Status::Success => {
                    self.idx = 0;
                    return Status::Success;
                }
            }
        }
        self.idx = 0;
        Status::Failure(last)
    }

    fn halt(&mut self) {
        self.idx = 0;
        for c in &mut self.children {
            c.halt();
        }
    }
}

/// 反应式回退：每拍从第 0 个子重判；高优先级分支转 `Success` / `Running` 时先 `halt`
/// 上一拍在跑的低优先级分支
#[derive(Default)]
pub struct ReactiveFallback {
    children: Vec<Box<dyn Node>>,
    /// 上一拍停在哪个子（低优先级分支）
    running: Option<usize>,
}

impl ReactiveFallback {
    pub fn new(children: Vec<Box<dyn Node>>) -> Self {
        Self { children, running: None }
    }

    /// 切分支：上一拍跑的分支不是 `keep` 就停掉
    fn halt_previous(&mut self, keep: Option<usize>) {
        if let Some(r) = self.running {
            if Some(r) != keep {
                self.children[r].halt();
            }
        }
    }
}

#[async_trait]
impl Node for ReactiveFallback {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        let mut last = FailCode::Unspecified;
        for i in 0..self.children.len() {
            let st = self.children[i].tick(f).await;
            match st {
                // 换分支：丢弃该分支的码
                Status::Failure(code) => last = code,
                Status::Running => {
                    self.halt_previous(Some(i));
                    self.running = Some(i);
                    return Status::Running;
                }
                Status::Success => {
                    self.halt_previous(Some(i));
                    self.running = None;
                    return Status::Success;
                }
            }
        }
        // 全失败：停掉上一拍的分支，传**最后一个分支**的码
        self.halt_previous(None);
        self.running = None;
        Status::Failure(last)
    }

    fn halt(&mut self) {
        self.running = None;
        for c in &mut self.children {
            c.halt();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::super::MotionOut;

    /// 脚本化节点：按 `plan` 顺序返回状态（用尽后固定返回最后一个），记录 tick / halt 次数
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

        /// 每拍都返回同一状态
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

    /// #1 Sequence 记忆：子 1 Running 后，下一拍不再 tick 子 0
    #[tokio::test]
    async fn test_sequence_memory() {
        let (c0, t0, _) = Scripted::fixed(Status::Success);
        let (c1, t1, _) = Scripted::fixed(Status::Running);
        let mut seq = Sequence::new(vec![boxed(c0), boxed(c1)]);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(seq.tick(&mut f).await, Status::Running);
        assert_eq!((cnt(&t0), cnt(&t1)), (1, 1));

        assert_eq!(seq.tick(&mut f).await, Status::Running);
        // 子 0 不再被 tick，子 1 继续
        assert_eq!((cnt(&t0), cnt(&t1)), (1, 2));
    }

    /// #2 Sequence 失败清记忆：子 1 失败后下一拍从子 0 重来，且码原样传
    #[tokio::test]
    async fn test_sequence_failure_clears_memory() {
        let (c0, t0, _) = Scripted::fixed(Status::Success);
        let (c1, _, _) = Scripted::new(vec![Status::Running, Status::Failure(FailCode::Blocked)]);
        let mut seq = Sequence::new(vec![boxed(c0), boxed(c1)]);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(seq.tick(&mut f).await, Status::Running);
        assert_eq!(seq.tick(&mut f).await, Status::Failure(FailCode::Blocked));
        assert_eq!(cnt(&t0), 1);

        // 记忆已清：从子 0 重来
        assert_eq!(seq.tick(&mut f).await, Status::Failure(FailCode::Blocked));
        assert_eq!(cnt(&t0), 2);
    }

    /// #3 ReactiveSequence：子 1 已 Running、本拍子 0 转 Failure → 整个 Failure（条件每拍重判），
    /// 并 halt 掉还挂在子 1 上的进度
    #[tokio::test]
    async fn test_reactive_sequence_reevaluates_conditions() {
        let (c0, t0, _) = Scripted::new(vec![Status::Success, Status::Failure(FailCode::Unspecified)]);
        let (c1, _, h1) = Scripted::fixed(Status::Running);
        let mut seq = ReactiveSequence::new(vec![boxed(c0), boxed(c1)]);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(seq.tick(&mut f).await, Status::Running);
        assert_eq!(seq.tick(&mut f).await, Status::Failure(FailCode::Unspecified));
        assert_eq!(cnt(&t0), 2);
        assert_eq!(cnt(&h1), 1);
    }

    /// #4 ReactiveFallback 抢回：低优先级 Running 时高优先级转 Success → 低优先级被 halt
    #[tokio::test]
    async fn test_reactive_fallback_preempts_low_priority() {
        let (c0, _, _) = Scripted::new(vec![Status::Failure(FailCode::Blocked), Status::Success]);
        let (c1, t1, h1) = Scripted::fixed(Status::Running);
        let mut fb = ReactiveFallback::new(vec![boxed(c0), boxed(c1)]);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(fb.tick(&mut f).await, Status::Running);
        assert_eq!((cnt(&t1), cnt(&h1)), (1, 0));

        assert_eq!(fb.tick(&mut f).await, Status::Success);
        assert_eq!(cnt(&h1), 1);
        assert_eq!(cnt(&t1), 1);
    }

    /// #5 Fallback 记忆：子 0 Running 时不去 tick 子 1
    #[tokio::test]
    async fn test_fallback_memory() {
        let (c0, t0, _) = Scripted::fixed(Status::Running);
        let (c1, t1, _) = Scripted::fixed(Status::Success);
        let mut fb = Fallback::new(vec![boxed(c0), boxed(c1)]);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(fb.tick(&mut f).await, Status::Running);
        assert_eq!(fb.tick(&mut f).await, Status::Running);
        assert_eq!((cnt(&t0), cnt(&t1)), (2, 0));
    }

    /// #6 halt 传播：父 halt 后，正在 Running 的子收到 halt
    #[tokio::test]
    async fn test_halt_propagates_to_running_child() {
        let (c0, t0, _) = Scripted::fixed(Status::Success);
        let (c1, _, h1) = Scripted::fixed(Status::Running);
        let mut seq = Sequence::new(vec![boxed(c0), boxed(c1)]);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        assert_eq!(seq.tick(&mut f).await, Status::Running);
        assert_eq!(cnt(&t0), 1);

        seq.halt();
        // 正在 Running 的子收到 halt
        assert_eq!(cnt(&h1), 1);

        // halt 后记忆清零：下一拍从子 0 重来
        assert_eq!(seq.tick(&mut f).await, Status::Running);
        assert_eq!(cnt(&t0), 2);
        assert_eq!(cnt(&h1), 1);
    }
}
