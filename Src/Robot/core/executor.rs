//Presented by KeJi
//Created Date ： 2026-07-28
//Modified Date ： 2026-08-03

//! Executor — 自动任务执行器
//!
//! 三状态：Idle → Turning → Moving → Idle（循环）
//! auto_tick 每 50ms 调用 step()，内部根据状态执行对应逻辑。
//! 集成 D* Lite 路径规划器，急停时标记障碍并重规划。

use std::f32::consts::PI;
use tracing::{info, warn};

use crate::robot::control::device::stm32::STM32Device;
use crate::robot::core::command::Mission;
use crate::robot::core::mission::MissionQueue;
use crate::robot::slam::pathfinder::DStarLite;
use crate::robot::slam::{OccupancyGrid, CELL_RESOLUTION};
use crate::robot::core::state::{LidarState, RobotState};

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
    /// D* Lite 路径规划器（pop 新 Mission 时创建）
    pathfinder: Option<DStarLite>,
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

    /// 问 D* 下一格：更新 start 到当前位置，返回下一格网格坐标
    fn query_next_sub_target(&mut self, wx: f32, wy: f32, grid: &OccupancyGrid) -> Option<(i32, i32)> {
        let current_gx = (wx / CELL_RESOLUTION).floor() as i32;
        let current_gy = (wy / CELL_RESOLUTION).floor() as i32;
        self.pathfinder.as_mut().and_then(|pf| {
            pf.move_to((current_gx, current_gy));
            pf.next_step(grid)
        })
    }

    pub fn new(config: ExecutorConfig) -> Self {
        Self {
            state: ExecState::Idle,
            goal: None,
            sub_target: None,
            config,
            pathfinder: None,
        }
    }

    /// 每 tick 调用一次（仅 Auto 模式）
    pub fn step(
        &mut self,
        stm32: &STM32Device,
        robot_state: &RobotState,
        lidar_state: &LidarState,
        grid: &OccupancyGrid,
        mission_queue: &mut MissionQueue,
    ) {
        // ① 感知：当前位置 + 航向
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
                    if let Some(ref mut pf) = self.pathfinder {
                        pf.mark_obstacle((ob_gx, ob_gy), grid);
                    }
                    self.sub_target = None;
                    self.state = ExecState::Idle;
                    return;
                }
            }
        }

        // ③ 状态机
        match self.state {
            ExecState::Idle => self.step_idle(stm32, wx, wy, yaw, grid, mission_queue),
            ExecState::Turning => self.step_turning(stm32, wx, wy, yaw),
            ExecState::Moving => self.step_moving(stm32, wx, wy, yaw, grid),
        }
    }

    // ─── Idle：思考 ────────

    fn step_idle(
        &mut self,
        stm32: &STM32Device,
        wx: f32, wy: f32, yaw: f32,
        grid: &OccupancyGrid,
        mission_queue: &mut MissionQueue,
    ) {
        // 检查是否到达 goal
        if let Some((gx, gy)) = self.goal {
            let dist = ((wx - gx).powi(2) + (wy - gy).powi(2)).sqrt();
            if dist < self.config.arrival_threshold_m {
                info!("[Executor] 到达目标 ({:.2}, {:.2})", gx, gy);
                self.goal = None;
                self.sub_target = None;
                self.pathfinder = None;
                if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
                return;
            }
        }

        // 没有 goal → pop 下一个 Mission
        if self.goal.is_none() {
            match mission_queue.pop_next() {
                Some(Mission::Goto(x, y)) => {
                    info!("[Executor] 新任务: Goto({:.2}, {:.2})", x, y);
                    self.goal = Some((x, y));
                    self.sub_target = None;
                    let start_gx = (wx / CELL_RESOLUTION).floor() as i32;
                    let start_gy = (wy / CELL_RESOLUTION).floor() as i32;
                    let goal_gx = (x / CELL_RESOLUTION).floor() as i32;
                    let goal_gy = (y / CELL_RESOLUTION).floor() as i32;
                    self.pathfinder = Some(DStarLite::new(
                        (start_gx, start_gy),
                        (goal_gx, goal_gy),
                    ));
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
                    self.pathfinder = None;
                    if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
                    return;
                }
            }

            let next = self.query_next_sub_target(wx, wy, grid);
            match next {
                Some((sx, sy)) => {
                    info!("[Executor] sub_target=({sx}, {sy})");
                    self.sub_target = Some((sx, sy));
                }
                None => {
                    warn!("[Executor] D* Lite 不可达，跳过此任务");
                    self.goal = None;
                    self.pathfinder = None;
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
                if let Err(e) = stm32.spin_right(self.config.turn_speed) { warn!("[Executor] SpinRight 失败: {e}"); }
            } else {
                if let Err(e) = stm32.spin_left(self.config.turn_speed) { warn!("[Executor] SpinLeft 失败: {e}"); }
            }
            self.state = ExecState::Turning;
        } else {
            if let Err(e) = stm32.forward(self.config.move_speed) { warn!("[Executor] Forward 失败: {e}"); }
            self.state = ExecState::Moving;
        }
    }

    // ─── Turning：等待角度对齐 ────────

    fn step_turning(&mut self, stm32: &STM32Device, wx: f32, wy: f32, yaw: f32) {
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

    fn step_moving(&mut self, stm32: &STM32Device, wx: f32, wy: f32, yaw: f32, grid: &OccupancyGrid) {
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
            match self.query_next_sub_target(wx, wy, grid) {
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
        self.pathfinder = None;
    }
}
