//Presented by KeJi
//Created Date ： 2026-11-05
//Modified Date ： 2026-11-05

//! Face 叶子（task_48 §2.1）：**原地转到面向某个世界点**，不移动。
//!
//! ```text
//! ① 读 yaw / x / y        ② delta = normalize(atan2(fy−y, fx−x) − yaw)
//! ③ |delta| ≤ 8° → Success ④ ticks ≥ 60 也 Success（口径 #6：转不到位**不得** Failure）
//! ⑤ 写 face_point，清 next_cell / target_point   ⑥ Running
//! ```
//!
//! 叶子只写 `Frame.out`：不碰 `nav`、不走 D\*、不做避障（口径 #7）。
//! `MotionOut::target()` 的优先级是 **face > point > cell**（`behavior_tree/frame.rs:42-50`），
//! 所以写 `face_point` 的同一拍必须把另两个字段清掉，否则上一拍的残留会顶掉朝向。
//!
//! **四份逐字**（`pleiades-ugv` / `pleiades-uav` / `sim/ugv_sim` / `sim/uav_sim`）：本文件刻意**不引任何
//! crate 内路径**（`use` 只有 `pleiades_base` 与标准库），四份连 `use` 都相同，`cp` 即可；
//! `FACE_DEADBAND_DEG` 在本文件定义而不是从执行器 import ——
//! 机栈执行器死区是 5°（`uav/executor.rs:46`）、车栈是 8°（`ugv/executor.rs:60`），import 会让四份行为不一致，
//! 也会让「刚出死区 → Running」的用例在机栈失效（口径 #5：四端统一 8°）。
//!
//! **为什么 8° 安全**：`MotionTarget::Face` 在 `|Δθ| ≤ 执行器死区` 时发 `(0, 0)` **不再转**
//! （`ugv/executor.rs:114-121`、`uav/executor.rs:89-95`），叶子阈值必须 **≥** 执行器死区才可能 `Success`；
//! 8 ≥ 8（车）与 8 ≥ 5（机）都成立。

use std::f32::consts::PI;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use pleiades_base::robot::core::behavior_tree::{Frame, Node, Status};
use pleiades_base::robot::core::state::RobotState;

/// 对准死区（度）：与车栈 `executor::DEADBAND_DEG` 同值（口径 #5）
pub const FACE_DEADBAND_DEG: f32 = 8.0;
/// 放弃拍数（60 拍 @ 50 ms = 3 s）：转不到位也 `Success`（口径 #6）
pub const FACE_GIVE_UP_TICKS: u32 = 60;

/// 建树（每次入队新建实例，§十一 #12）
pub fn build(robot_state: Arc<RwLock<RobotState>>, target: (f32, f32)) -> Box<dyn Node> {
    Box::new(FaceLeaf::new(robot_state, target))
}

/// 叶子：读位姿 → 算方位差 → 只写 `face_point`
pub struct FaceLeaf {
    /// 要朝向的**世界点**（米）
    target: (f32, f32),
    robot_state: Arc<RwLock<RobotState>>,
    /// 已 `Running` 的拍数（`halt` 归零）
    ticks: u32,
}

impl FaceLeaf {
    pub fn new(robot_state: Arc<RwLock<RobotState>>, target: (f32, f32)) -> Self {
        Self { target, robot_state, ticks: 0 }
    }
}

#[async_trait]
impl Node for FaceLeaf {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status {
        // ① 读位姿（读锁用完即放）
        let (x, y, yaw) = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y, rs.attitude.yaw)
        };
        // ② 期望朝向 − 当前朝向，归一化到 [−π, π]
        let (fx, fy) = self.target;
        let delta = normalize_angle((fy - y).atan2(fx - x) - yaw);
        // ③ 已对准 → 终值。这一拍**不写** face_point：执行器无目标即 (0, 0) 停车
        if delta.to_degrees().abs() <= FACE_DEADBAND_DEG {
            return Status::Success;
        }
        // ④ 限时到点：转不到位也 Success（口径 #6）。先加再判 ⇒ 第 60 拍收尾（60×50 ms = 3.0 s）
        self.ticks += 1;
        if self.ticks >= FACE_GIVE_UP_TICKS {
            return Status::Success;
        }
        // ⑤ 只转不走：写朝向 + 清另两个字段（face > point > cell）
        f.out.face_point = Some(self.target);
        f.out.next_cell = None;
        f.out.target_point = None;
        // ⑥
        Status::Running
    }

    /// 只清自身计数、不发 Stop（§十一 #23）；幂等
    fn halt(&mut self) {
        self.ticks = 0;
    }
}

/// 角度归一化到 [−π, π]（与四个 `executor.rs` 同式；本仓无跨 crate 公共工具，四份各带一份）
fn normalize_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    use pleiades_base::robot::core::behavior_tree::MotionOut;

    // ------------------------------------------------------------ 夹具

    /// 车在原点、yaw 由度数给（写成弧度）；目标固定 (10, 0) ⇒ 期望朝向 = +x
    async fn pose(yaw_deg: f32) -> Arc<RwLock<RobotState>> {
        let state = Arc::new(RwLock::new(RobotState::default()));
        {
            let mut rs = state.write().await;
            rs.x = 0.0;
            rs.y = 0.0;
            rs.attitude.yaw = yaw_deg.to_radians();
        }
        state
    }

    fn frame(out: &mut MotionOut) -> Frame<'_> {
        Frame { dt_ms: 50, tick_no: 0, out }
    }

    /// 目标（+x 方向 10 m）
    const TARGET: (f32, f32) = (10.0, 0.0);

    // ------------------------------------------------------------ 用例

    /// `Δθ = 0`（车头已朝 +x）⇒ 第 1 拍就 `Success`（口径 #5：`|Δθ| ≤ 8°`）
    #[tokio::test]
    async fn test_face_aligned_success_on_first_tick() {
        let state = pose(0.0).await;
        let mut tree = build(state, TARGET);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out);
        assert_eq!(tree.tick(&mut f).await, Status::Success, "已对准必须一拍终值");
        // `Success` 那拍不写 face_point：执行器拿 `None` ⇒ (0, 0) 停车（已是期望行为）
        assert_eq!(out.face_point, None, "已对准这拍不再给朝向");
    }

    /// 偏 10°（`FACE_DEADBAND_DEG + 2`）⇒ `Running`，且**三个字段一起**正确
    /// （`face_point` 写目标、另两个清空；`MotionOut::target()` 是 face > point > cell）
    #[tokio::test]
    async fn test_face_off_deadband_is_running_and_writes_three_fields() {
        let state = pose(FACE_DEADBAND_DEG + 2.0).await;
        let mut leaf = FaceLeaf::new(state, TARGET);
        let mut out = MotionOut::default();
        let mut f = frame(&mut out);
        assert_eq!(leaf.tick(&mut f).await, Status::Running, "出死区必须 Running（继续转）");
        assert_eq!(out.face_point, Some(TARGET), "朝向 = 目标点");
        assert_eq!(out.next_cell, None, "只转不走：不得给格");
        assert_eq!(out.target_point, None, "只转不走：不得给点");
    }

    /// 偏 90°（车头朝 +y）⇒ 连 2 拍 `Running`，`face_point` 一直是目标点（不抖）
    #[tokio::test]
    async fn test_face_off_90deg_is_running() {
        let state = pose(90.0).await;
        let mut leaf = FaceLeaf::new(state, TARGET);
        let mut out = MotionOut::default();
        for i in 0..2 {
            let mut f = frame(&mut out);
            assert_eq!(leaf.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(out.face_point, Some(TARGET), "第 {i} 拍：朝向不抖");
            assert_eq!(out.next_cell, None);
            assert_eq!(out.target_point, None);
        }
    }

    /// 偏 90° 且永不进死区（位姿每拍都一样）：前 **59** 拍 `Running`，第 **60** 拍 `Success`
    /// （口径 #6：60 拍 = 3 s 兜底，**不得** `Failure`）
    #[tokio::test]
    async fn test_face_gives_up_after_60_ticks_success() {
        let state = pose(90.0).await;
        let mut leaf = FaceLeaf::new(state, TARGET);
        let mut out = MotionOut::default();
        for i in 1..FACE_GIVE_UP_TICKS {
            let mut f = frame(&mut out);
            assert_eq!(leaf.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running");
            assert_eq!(out.face_point, Some(TARGET));
        }
        let mut f = frame(&mut out);
        assert_eq!(
            leaf.tick(&mut f).await,
            Status::Success,
            "第 {FACE_GIVE_UP_TICKS} 拍必须 Success（转不到位也不得 Failure）"
        );
    }

    /// `halt` 幂等：`ticks` 归零后从 0 重新计数（59 拍 `Running`、第 60 拍 `Success`）
    #[tokio::test]
    async fn test_face_halt_resets_ticks() {
        let state = pose(90.0).await;
        let mut leaf = FaceLeaf::new(state, TARGET);
        let mut out = MotionOut::default();
        for _ in 0..3 {
            let mut f = frame(&mut out);
            assert_eq!(leaf.tick(&mut f).await, Status::Running);
        }
        assert_eq!(leaf.ticks, 3, "3 拍后计数 = 3");
        leaf.halt();
        leaf.halt(); // 幂等
        assert_eq!(leaf.ticks, 0, "halt 必须把计数归零");
        for i in 1..FACE_GIVE_UP_TICKS {
            let mut f = frame(&mut out);
            assert_eq!(leaf.tick(&mut f).await, Status::Running, "halt 后第 {i} 拍必须 Running");
        }
        let mut f = frame(&mut out);
        assert_eq!(leaf.tick(&mut f).await, Status::Success, "halt 后第 60 拍才 Success");
    }
}
