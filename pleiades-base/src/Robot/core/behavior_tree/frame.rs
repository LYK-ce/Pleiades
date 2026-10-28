//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 每拍上下文（Task 36 §12.2.4）
//!
//! `Frame` 只有时间与输出，**不含任何设备字段**（§十一 #20：引擎非泛型、无 `Obs` / 无黑板）。
//! 节点需要观测就直连自己持有的能力端口或设备状态快照（#21），不是从 `Frame` 里拿。

/// 本拍输出：叶子算出的"下一格"；驱动每拍读完就丢弃
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MotionOut {
    /// 本拍叶子算出的"下一格"；`None` = 这一拍不动（执行器会自然停车）
    pub next_cell: Option<(i32, i32)>,
}

impl MotionOut {
    /// 每拍 `tick` 之前调用：清空上一拍的输出（没有新输出 = 停车，§十一 #25）
    pub fn clear(&mut self) {
        self.next_cell = None;
    }
}

/// 一拍上下文
#[derive(Debug)]
pub struct Frame<'a> {
    /// 本拍时长（驱动传入）；挂起期间不推进，时间按 tick 累积（§十一 #17）
    pub dt_ms: u32,
    /// 拍号（驱动维护，节点只读）
    pub tick_no: u64,
    /// 本拍输出（叶子写，驱动读）
    pub out: &'a mut MotionOut,
}
