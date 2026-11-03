//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-11-03

//! 每拍上下文（Task 36 §12.2.4）
//!
//! `Frame` 只有时间与输出，**不含任何设备字段**（§十一 #20：引擎非泛型、无 `Obs` / 无黑板）。
//! 节点需要观测就直连自己持有的能力端口或设备状态快照（#21），不是从 `Frame` 里拿。

/// 本拍目标：**格**（Goto / 巡逻，取格心）或**点**（Follow，人的世界坐标，不量化）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionTarget {
    /// 网格坐标（执行器侧再取格心：`cell_center`）
    Cell((i32, i32)),
    /// 世界坐标（米）：原值直接用，不经格心
    Point((f32, f32)),
}

/// 本拍输出：叶子算出的目标；驱动每拍读完就丢弃
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MotionOut {
    /// 下一格（Goto / 巡逻）；`None` = 这一拍不给格
    pub next_cell: Option<(i32, i32)>,
    /// 目标点（Follow）；`None` = 这一拍不给点（task_45 §一 #1）
    pub target_point: Option<(f32, f32)>,
}

impl MotionOut {
    /// 每拍 `tick` 之前调用：清空上一拍的输出（两个字段都清；没有新输出 = 停车，§十一 #25）
    pub fn clear(&mut self) {
        self.next_cell = None;
        self.target_point = None;
    }

    /// 本拍目标：**点优先 → 其次格 → 都没有 = `None`**（task_45 §一 #1，唯一入口，不加第二个函数）
    pub fn target(&self) -> Option<MotionTarget> {
        if let Some((px, py)) = self.target_point {
            return Some(MotionTarget::Point((px, py)));
        }
        self.next_cell.map(MotionTarget::Cell)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// task_45 §一 #1：点优先、其次格、都没有 = `None`；`clear()` 两个字段都清
    #[test]
    fn test_target_prefers_point_then_cell() {
        // 都没有 → None（执行器会自然停车）
        let out = MotionOut::default();
        assert_eq!(out.target(), None, "两个字段都空 → 无目标");

        // 只有格 → Cell
        let out = MotionOut { next_cell: Some((3, -1)), target_point: None };
        assert_eq!(out.target(), Some(MotionTarget::Cell((3, -1))), "只有格 → 取格");

        // 只有点 → Point
        let out = MotionOut { next_cell: None, target_point: Some((1.25, -2.5)) };
        assert_eq!(out.target(), Some(MotionTarget::Point((1.25, -2.5))), "只有点 → 取点");

        // 两者都有 → **点优先**（Follow 的坐标不得被格心顶替）
        let out = MotionOut { next_cell: Some((0, 1)), target_point: Some((0.25, 9.25)) };
        assert_eq!(
            out.target(),
            Some(MotionTarget::Point((0.25, 9.25))),
            "点优先于格：转向环里不得出现格心"
        );
    }

    /// `clear()` 必须两个字段一起清（只清一个会跨拍残留）
    #[test]
    fn test_clear_drops_both_fields() {
        let mut out = MotionOut { next_cell: Some((2, 2)), target_point: Some((5.0, 5.0)) };
        out.clear();
        assert_eq!(out.next_cell, None);
        assert_eq!(out.target_point, None);
        assert_eq!(out.target(), None);
    }
}
