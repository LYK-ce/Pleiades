//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 行为树引擎（Task 36 §12.2）
//!
//! **非泛型 + async + 零设备字段**（§十一 #20 / #22）：
//! - `trait Node` 无类型参数；上下文只有 `Frame { dt_ms, tick_no, out }`；
//! - `trait Node: Send`，可跨 `spawn` 持有（驱动在 `main_loop` 里 tick）；
//! - 不出现任何设备词汇（位姿 / 导航 / 检测 / 相机 / 底盘串口 / 飞控协议），§12.2.7 #3 用 grep 验。
//!
//! **没有 `Tree` 包装**：树就是一个 `Box<dyn Node>`；驱动每拍 `tick`、需要时 `halt`。
//! 也**没有兜底时限**（§十一 #33）：持续任务靠 `replace` 结束。
//!
//! 归属：本目录只放**引擎**；具体任务树与叶子（读状态、调导航、调 YOLO）在设备端各一份（#19）。

pub mod composite;
pub mod decorator;
pub mod frame;
pub mod leaf;
pub mod node;
pub mod status;

pub use composite::{Fallback, ReactiveFallback, ReactiveSequence, Sequence};
pub use decorator::{Guard, Inverter, Repeat, RepeatForever, Retry, Timeout};
pub use frame::{Frame, MotionOut};
pub use leaf::{Action, Condition, Wait};
pub use node::Node;
pub use status::{FailCode, Status};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;

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

    fn frame(out: &mut MotionOut, dt_ms: u32) -> Frame<'_> {
        Frame { dt_ms, tick_no: 0, out }
    }

    fn boxed(n: impl Node + 'static) -> Box<dyn Node> {
        Box::new(n)
    }

    fn fixed(st: Status) -> Box<dyn Node> {
        let (n, _) = Scripted::fixed(st);
        boxed(n)
    }

    /// #14 halt 幂等：每个节点都测；且 halt 后状态回到初始（计时 / 记忆 / 计数清零）
    #[tokio::test]
    async fn test_halt_idempotent_for_every_node() {
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        // （名字, 节点, 脏一次后的状态不影响；halt 两次后首拍应有的状态）
        let mut cases: Vec<(&str, Box<dyn Node>, Status)> = vec![
            (
                "Sequence",
                boxed(Sequence::new(vec![fixed(Status::Success), fixed(Status::Running)])),
                Status::Running,
            ),
            (
                "ReactiveSequence",
                boxed(ReactiveSequence::new(vec![fixed(Status::Success), fixed(Status::Running)])),
                Status::Running,
            ),
            (
                "Fallback",
                boxed(Fallback::new(vec![fixed(Status::Failure(FailCode::Blocked)), fixed(Status::Success)])),
                Status::Success,
            ),
            (
                "ReactiveFallback",
                boxed(ReactiveFallback::new(vec![
                    fixed(Status::Failure(FailCode::Blocked)),
                    fixed(Status::Success),
                ])),
                Status::Success,
            ),
            // 计时没清的话，第二次就会 Timeout
            ("Timeout", boxed(Timeout::new(100, fixed(Status::Running))), Status::Running),
            // 计数没清的话，第二次就会 RetryExhausted
            ("Retry", boxed(Retry::new(2, fixed(Status::Failure(FailCode::Blocked)))), Status::Running),
            ("Guard", boxed(Guard::new(|| true, fixed(Status::Success))), Status::Success),
            (
                "Inverter",
                boxed(Inverter::new(fixed(Status::Success))),
                Status::Failure(FailCode::Unspecified),
            ),
            // 计数没清的话，第二次就会 Success
            ("Repeat", boxed(Repeat::new(2, fixed(Status::Success))), Status::Running),
            ("RepeatForever", boxed(RepeatForever::new(fixed(Status::Success))), Status::Running),
            ("Wait", boxed(Wait::new(10)), Status::Success),
            ("Action", boxed(Action::new(|_f: &mut Frame<'_>| Status::Success)), Status::Success),
            ("Condition", boxed(Condition::new(|| true)), Status::Success),
        ];

        for (name, node, expect_after_halt) in cases.iter_mut() {
            // 弄脏：先 tick 一次（可能起计时 / 计数 / 记忆）
            let _dirty = node.tick(&mut f).await;
            // 连续 halt 两次：不 panic，且状态一致
            node.halt();
            node.halt();
            assert_eq!(node.tick(&mut f).await, *expect_after_halt, "节点 {name} 的 halt 未复位状态");
        }
    }

    /// #16 失败码传递：Sequence 原样传 / Fallback 丢旧留末 / Timeout 用自己的 / Retry 用 RetryExhausted
    #[tokio::test]
    async fn test_failcode_propagation() {
        let mut out = MotionOut::default();
        let mut f = frame(&mut out, 50);

        // Sequence：原样传失败子节点的码
        let mut seq = Sequence::new(vec![
            fixed(Status::Success),
            fixed(Status::Failure(FailCode::Unreachable)),
        ]);
        assert_eq!(seq.tick(&mut f).await, Status::Failure(FailCode::Unreachable));

        // Fallback：换分支丢弃前一个码，最后传末支的码
        let mut fb = Fallback::new(vec![
            fixed(Status::Failure(FailCode::Timeout)),
            fixed(Status::Failure(FailCode::Device(7))),
        ]);
        assert_eq!(fb.tick(&mut f).await, Status::Failure(FailCode::Device(7)));

        // Timeout：用自己的码（子报的是 Blocked）
        let mut to = Timeout::new(100, fixed(Status::Failure(FailCode::Blocked)));
        // 子失败直接透传（不是超时），先把计时耗掉再验超时
        assert_eq!(to.tick(&mut f).await, Status::Failure(FailCode::Blocked));
        to.halt();
        let mut to2 = Timeout::new(100, fixed(Status::Running));
        assert_eq!(to2.tick(&mut f).await, Status::Running);
        assert_eq!(to2.tick(&mut f).await, Status::Failure(FailCode::Timeout));

        // Retry：统一 RetryExhausted（子报的是 Blocked）
        let mut rt = Retry::new(2, fixed(Status::Failure(FailCode::Blocked)));
        assert_eq!(rt.tick(&mut f).await, Status::Running);
        assert_eq!(rt.tick(&mut f).await, Status::Failure(FailCode::RetryExhausted));

        // Device 码进 1000+，base 只透传不解释
        assert_eq!(FailCode::Device(7).code(), 1007);
        assert_eq!(FailCode::Unspecified.code(), 0);
    }
}
