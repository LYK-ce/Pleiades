//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 节点接口（Task 36 §12.2.4）
//!
//! 非泛型 + async + 零设备字段。**没有 `Tree` 包装**：树就是一个 `Box<dyn Node>`，
//! 驱动持有它，每拍 `tick`、需要时 `halt`（§十一 #23 / §12.2.4 注）。

use async_trait::async_trait;

use super::{Frame, Status};

/// 行为树节点
///
/// 约定（实现与评审都按这条）：
/// - `tick` 可 `await`，但**只能拿现成的**（`lidar.get_scan().await` 这类），
///   长过程必须拆成"发意图 → 下拍取结果"，不得拖慢 50 ms 环（§十一 #22 / §12.2.8 #4）。
/// - `halt` **只清自身状态**，不发 `Stop`；驱动在 `halt` 之后统一发一次 `Stop` + 清意图（#23）。
/// - `halt` **必须幂等**：连续调用两次不 panic、状态一致（§12.2.6 #14）。
#[async_trait]
pub trait Node: Send {
    /// 走一拍
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status;

    /// 停：清自身状态（记忆位置 / 计时 / 重试计数），并向下传播；幂等
    fn halt(&mut self);
}
