//Presented by KeJi
//Created Date ： 2026-07-28
//Modified Date ： 2026-08-20

//! Executor — 自动任务执行器
//!
//! 三状态：Idle → Turning → Moving → Idle（循环）
//! auto_tick 每 50ms 调用 step()，内部根据状态执行对应逻辑。
//! 集成 D* Lite 路径规划器（Task 22 起 D* 移入世界模块 `World`），急停时标记障碍并重规划。

use std::f32::consts::PI;
use std::sync::Arc;
use tracing::{info, warn};

use crate::robot::device::MotionDevice;
use crate::robot::core::command::Mission;
use crate::robot::core::mission::MissionQueue;
use crate::robot::core::planning::assignment;
use crate::robot::slam::{OccupancyGrid, CELL_RESOLUTION};
use crate::robot::core::state::{ExecuteState, LidarState, RobotState};
use crate::robot::world::World;

/// 执行器状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecState {
    /// 停下思考：pop 任务 / 问 D* / 算角度
    Idle,
    /// 旋转中对准目标角度
    Turning,
    /// 直行中
    Moving,
}

/// 自动任务执行器
pub struct Executor {
    state: ExecState,
    /// 当前 Mission 的最终目标（世界坐标，米）
    goal: Option<(f32, f32)>,
    /// D* 给的当前要走的下一格（网格坐标）
    sub_target: Option<(i32, i32)>,
    /// 配置参数
    config: ExecutorConfig,
    /// 世界模块（Task 22：D* 移入 world，executor 只持有共享句柄）
    world: Arc<World>,
}

/// 执行器配置
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub auto_tick_ms: u64,
    pub sub_target_threshold_m: f32,
    pub obstacle_threshold_m: f32,
    pub turn_speed: i16,
    pub move_speed: i16,
    pub turn_align_threshold_deg: f32,
    pub arrival_threshold_m: f32,
    /// 直行连续化：到达 sub_target 后，与下一格方向偏差小于该角度（度）时不停车继续走
    pub straight_align_threshold_deg: f32,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            auto_tick_ms: 50,
            sub_target_threshold_m: 0.2,
            obstacle_threshold_m: 0.3,
            turn_speed: 10,
            move_speed: 30,
            turn_align_threshold_deg: 5.0,
            arrival_threshold_m: 0.3,
            straight_align_threshold_deg: 10.0,
        }
    }
}

impl Executor {
    /// 网格坐标 → 世界坐标（取格中心，与 world_to_grid 的 floor 语义一致）
    /// 格 (gx, gy) 覆盖世界 [gx·R, (gx+1)·R)，中心为 (gx + 0.5)·R
    fn cell_center_world(gx: i32) -> f32 {
        (gx as f32 + 0.5) * CELL_RESOLUTION
    }

    /// 问 D* 下一格：move_to + 动态障碍 + next_step（Task 22：委托给世界模块）
    async fn query_next_sub_target(
        &self,
        wx: f32,
        wy: f32,
        dynamic_obstacles: &[(i32, i32)],
    ) -> Option<(i32, i32)> {
        self.world.get_path(wx, wy, dynamic_obstacles).await
    }

    pub fn new(config: ExecutorConfig, world: Arc<World>) -> Self {
        Self {
            state: ExecState::Idle,
            goal: None,
            sub_target: None,
            config,
            world,
        }
    }

    /// 装载新目标：设置 goal / 清 sub_target / 建 D* 路径（Goto / Circle 共用）
    fn start_goal(&mut self, goal: (f32, f32), wx: f32, wy: f32) {
        self.goal = Some(goal);
        self.sub_target = None;
        let start_gx = (wx / CELL_RESOLUTION).floor() as i32;
        let start_gy = (wy / CELL_RESOLUTION).floor() as i32;
        let goal_gx = (goal.0 / CELL_RESOLUTION).floor() as i32;
        let goal_gy = (goal.1 / CELL_RESOLUTION).floor() as i32;
        self.world.set_goal((start_gx, start_gy), (goal_gx, goal_gy));
    }

    /// 每 tick 调用一次（仅 Auto 模式）
    ///
    /// Task 13_1：`execute_state` 为执行器意图（sub_target），
    /// 包装层在 step_impl 返回后统一同步——覆盖 step 内部所有 return 路径。
    pub async fn step(
        &mut self,
        stm32: &dyn MotionDevice,
        robot_state: &RobotState,
        lidar_state: &LidarState,
        grid: &OccupancyGrid,
        dynamic_obstacles: &[(i32, i32)],
        mission_queue: &mut MissionQueue,
        execute_state: &mut ExecuteState,
        own_peer_id: &[u8],
    ) {
        self.step_impl(stm32, robot_state, lidar_state, grid, dynamic_obstacles, mission_queue, own_peer_id).await;
        execute_state.sub_target = self.sub_target;
    }

    /// step 主体（私有实现，不含意图同步；Task 13_1 拆出以便包装同步）
    async fn step_impl(
        &mut self,
        stm32: &dyn MotionDevice,
        robot_state: &RobotState,
        lidar_state: &LidarState,
        grid: &OccupancyGrid,
        dynamic_obstacles: &[(i32, i32)],
        mission_queue: &mut MissionQueue,
        own_peer_id: &[u8],
    ) {
        // ① 感知：当前位置（世界坐标，直读 RobotState）+ 航向
        let (wx, wy) = (robot_state.x, robot_state.y);
        let yaw = robot_state.attitude.yaw;

        // ② 实时障碍急停：前方 LiDAR 距离 < 阈值 → 立即停车 + 标记障碍
        if let Some(ref scan) = lidar_state.scan {
            let closest = scan.points.iter()
                .filter(|p| {
                    let a = if p.angle < 0.0 { p.angle + 2.0 * PI } else { p.angle };
                    p.range >= 0.1 && (a < PI / 4.0 || a >= 7.0 * PI / 4.0)
                })
                .min_by(|a, b| a.range.partial_cmp(&b.range).unwrap_or(std::cmp::Ordering::Equal));

            if let Some(p) = closest {
                if p.range < self.config.obstacle_threshold_m {
                    warn!("[Executor] 前方障碍 {:.2}m < {:.2}m，急停", p.range, self.config.obstacle_threshold_m);
                    if let Err(e) = stm32.stop() { warn!("[Executor] 急停失败: {e}"); }

                    let ob_angle = yaw + p.angle;
                    let ob_wx = wx + p.range * ob_angle.cos();
                    let ob_wy = wy + p.range * ob_angle.sin();
                    let ob_gx = (ob_wx / CELL_RESOLUTION).floor() as i32;
                    let ob_gy = (ob_wy / CELL_RESOLUTION).floor() as i32;
                    self.world.mark_obstacle((ob_gx, ob_gy)).await;
                    self.sub_target = None;
                    self.state = ExecState::Idle;
                    return;
                }
            }
        }

        // ③ 状态机
        match self.state {
            ExecState::Idle => self.step_idle(stm32, wx, wy, yaw, grid, dynamic_obstacles, mission_queue, own_peer_id).await,
            ExecState::Turning => self.step_turning(stm32, wx, wy, yaw),
            ExecState::Moving => self.step_moving(stm32, wx, wy, yaw, dynamic_obstacles).await,
        }
    }

    // ─── Idle：思考 ────────

    async fn step_idle(
        &mut self,
        stm32: &dyn MotionDevice,
        wx: f32, wy: f32, yaw: f32,
        grid: &OccupancyGrid,
        dynamic_obstacles: &[(i32, i32)],
        mission_queue: &mut MissionQueue,
        own_peer_id: &[u8],
    ) {
        // 检查是否到达 goal
        if let Some((gx, gy)) = self.goal {
            let dist = ((wx - gx).powi(2) + (wy - gy).powi(2)).sqrt();
            if dist < self.config.arrival_threshold_m {
                info!("[Executor] 到达目标 ({:.2}, {:.2})", gx, gy);
                self.goal = None;
                self.sub_target = None;
                self.world.clear_goal();
                if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
                return;
            }
        }

        // 没有 goal → pop 下一个 Mission
        if self.goal.is_none() {
            match mission_queue.pop_next() {
                Some(Mission::Goto { x, y, members }) => {
                    info!(
                        "[Executor] 新任务: Goto({:.2}, {:.2}){}",
                        x, y,
                        if members.is_empty() { "" } else { "（群发）" }
                    );
                    // Task 14：群发任务（members 非空）经 assignment 自算本车散布位置；
                    // 单车（members 空）直接以目标点为 goal（老行为）。
                    let goal = match assignment::group_goto_mission((x, y), &members, own_peer_id, grid) {
                        Ok(g) => g,
                        Err(e) => {
                            warn!("[Executor] 群发任务分配失败: {e:?}，跳过此任务");
                            self.goal = None;
                            self.sub_target = None;
                            self.world.clear_goal();
                            return;
                        }
                    };
                    self.start_goal(goal, wx, wy);
                }
                Some(Mission::Circle { x, y, members }) => {
                    info!(
                        "[Executor] 新任务: Circle({:.2}, {:.2}){}",
                        x, y,
                        if members.is_empty() { "" } else { "（群发）" }
                    );
                    // Task 18：每车按 peer_id 排序序号在环上均匀铺开（半径 0.5m = 与圆心隔 1 格）
                    let goal = match assignment::group_circle_mission((x, y), &members, own_peer_id, grid) {
                        Ok(g) => g,
                        Err(e) => {
                            warn!("[Executor] 围圈任务分配失败: {e:?}，跳过此任务");
                            self.goal = None;
                            self.sub_target = None;
                            self.world.clear_goal();
                            return;
                        }
                    };
                    self.start_goal(goal, wx, wy);
                }
                None => return,
            }
        }

        // 需要 sub_target → 问 D* Lite
        if self.sub_target.is_none() {
            let current_gx = (wx / CELL_RESOLUTION).floor() as i32;
            let current_gy = (wy / CELL_RESOLUTION).floor() as i32;

            // 网格级到达检查：已在目标格但世界距离未达标时直接视为到达
            if let Some((gx, gy)) = self.goal {
                let goal_gx = (gx / CELL_RESOLUTION).floor() as i32;
                let goal_gy = (gy / CELL_RESOLUTION).floor() as i32;
                if current_gx == goal_gx && current_gy == goal_gy {
                    info!("[Executor] 已在目标格，到达");
                    self.goal = None;
                    self.world.clear_goal();
                    if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
                    return;
                }
            }

            let next = self.query_next_sub_target(wx, wy, dynamic_obstacles).await;
            match next {
                Some((sx, sy)) => {
                    info!("[Executor] sub_target=({sx}, {sy})");
                    self.sub_target = Some((sx, sy));
                }
                None => {
                    warn!("[Executor] D* Lite 不可达，跳过此任务");
                    self.goal = None;
                    self.world.clear_goal();
                    return;
                }
            }
        }

        // 计算角偏差（归一化到 [-π, π]）
        let (st_x, st_y) = self.sub_target.unwrap();
        let target_wx = Self::cell_center_world(st_x);
        let target_wy = Self::cell_center_world(st_y);
        let target_angle = (target_wy - wy).atan2(target_wx - wx);
        let mut delta = target_angle - yaw;
        delta = (delta + PI).rem_euclid(2.0 * PI) - PI;

        info!("[转向] turning: delta={:.1}° yaw={:.1}°", delta.to_degrees(), yaw.to_degrees());

        let threshold_rad = self.config.turn_align_threshold_deg.to_radians();

        info!("[转向] delta={:.1}° yaw={:.1}° target={:.1}° st=({},{})", delta.to_degrees(), yaw.to_degrees(), target_angle.to_degrees(), st_x, st_y);

        if delta.abs() > threshold_rad {
            if delta > 0.0 {
                if let Err(e) = stm32.turn_right(self.config.turn_speed) { warn!("[Executor] SpinRight 失败: {e}"); }
            } else {
                if let Err(e) = stm32.turn_left(self.config.turn_speed) { warn!("[Executor] SpinLeft 失败: {e}"); }
            }
            self.state = ExecState::Turning;
        } else {
            if let Err(e) = stm32.move_forward(self.config.move_speed) { warn!("[Executor] Forward 失败: {e}"); }
            self.state = ExecState::Moving;
        }
    }

    // ─── Turning：等待角度对齐 ────────

    fn step_turning(&mut self, stm32: &dyn MotionDevice, wx: f32, wy: f32, yaw: f32) {
        let (st_x, st_y) = match self.sub_target {
            Some(st) => st,
            None => {
                self.state = ExecState::Idle;
                return;
            }
        };

        let target_wx = Self::cell_center_world(st_x);
        let target_wy = Self::cell_center_world(st_y);
        let target_angle = (target_wy - wy).atan2(target_wx - wx);
        let mut delta = target_angle - yaw;
        delta = (delta + PI).rem_euclid(2.0 * PI) - PI;

        let threshold_rad = self.config.turn_align_threshold_deg.to_radians();

        if delta.abs() <= threshold_rad {
            self.state = ExecState::Idle;
            if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
        }
    }

    // ─── Moving：等到 sub_target ────────

    async fn step_moving(&mut self, stm32: &dyn MotionDevice, wx: f32, wy: f32, yaw: f32, dynamic_obstacles: &[(i32, i32)]) {
        let (st_x, st_y) = match self.sub_target {
            Some(st) => st,
            None => {
                self.state = ExecState::Idle;
                return;
            }
        };

        let target_wx = Self::cell_center_world(st_x);
        let target_wy = Self::cell_center_world(st_y);
        let dist = ((wx - target_wx).powi(2) + (wy - target_wy).powi(2)).sqrt();

        if dist < self.config.sub_target_threshold_m {
            // 到达当前 sub_target：先问 D* 下一格
            match self.query_next_sub_target(wx, wy, dynamic_obstacles).await {
                Some((nx, ny)) => {
                    let n_wx = Self::cell_center_world(nx);
                    let n_wy = Self::cell_center_world(ny);
                    let next_angle = (n_wy - wy).atan2(n_wx - wx);
                    let mut delta = next_angle - yaw;
                    delta = (delta + PI).rem_euclid(2.0 * PI) - PI;
                    if delta.abs() < self.config.straight_align_threshold_deg.to_radians() {
                        // 方向一致 → 直行连续化：不停车，直接换目标继续走
                        info!("[Executor] 直行连续化 → sub_target=({nx}, {ny})");
                        self.sub_target = Some((nx, ny));
                        return;
                    }
                    // 方向不一致 → 更新目标，停车交给 Idle 转向
                    info!("[Executor] 需转向，停车 → sub_target=({nx}, {ny})");
                    self.sub_target = Some((nx, ny));
                }
                None => {
                    // 无下一格（到达终点/不可达）→ 交给 Idle 收尾
                    self.sub_target = None;
                }
            }
            self.state = ExecState::Idle;
            if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
        }
    }

    /// 外部切换模式时重置
    pub fn reset(&mut self) {
        self.state = ExecState::Idle;
        self.goal = None;
        self.sub_target = None;
        self.world.clear_goal();
    }
}
