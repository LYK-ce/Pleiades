//Presented by KeJi
//Created Date ： 2026-07-28
//Modified Date ： 2026-07-30

//! Executor — 自动任务执行器
//!
//! 三状态：Idle → Turning → Moving → Idle（循环）
//! auto_tick 每 50ms 调用 step()，内部根据状态执行对应逻辑。

use std::f32::consts::PI;
use tracing::{info, warn};

use crate::robot::control::device::stm32::STM32Device;
use crate::robot::core::command::Mission;
use crate::robot::core::mission::MissionQueue;
use crate::robot::slam::OccupancyGrid;
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
        }
    }
}

impl Executor {
    pub fn new(config: ExecutorConfig) -> Self {
        Self {
            state: ExecState::Idle,
            goal: None,
            sub_target: None,
            config,
        }
    }

    /// 每 tick 调用一次（仅 Auto 模式）
    pub fn step(
        &mut self,
        stm32: &STM32Device,
        robot_state: &RobotState,
        lidar_state: &LidarState,
        _grid: &OccupancyGrid,
        mission_queue: &mut MissionQueue,
    ) {
        // ① 感知：当前位置 + 航向
        let (wx, wy) = (64.0 + robot_state.odom_x, 64.0 + robot_state.odom_y);
        let yaw = robot_state.attitude.yaw;

        // ② 实时障碍急停：前方 LiDAR 距离 < 阈值 → 立即停车
        if let Some(ref scan) = lidar_state.scan {
            let front_min = scan.points.iter()
                .filter(|p| {
                    let a = if p.angle < 0.0 { p.angle + 2.0 * PI } else { p.angle };
                    p.range >= 0.1 && (a < PI / 4.0 || a >= 7.0 * PI / 4.0)
                })
                .map(|p| p.range)
                .fold(f32::MAX, f32::min);

            if front_min < self.config.obstacle_threshold_m {
                warn!("[Executor] 前方障碍 {:.2}m < {:.2}m，急停", front_min, self.config.obstacle_threshold_m);
                if let Err(e) = stm32.stop() { warn!("[Executor] 急停失败: {e}"); }
                self.state = ExecState::Idle;
                return;
            }
        }

        // ③ 状态机
        match self.state {
            ExecState::Idle => self.step_idle(stm32, wx, wy, yaw, mission_queue),
            ExecState::Turning => self.step_turning(stm32, wx, wy, yaw),
            ExecState::Moving => self.step_moving(stm32, wx, wy),
        }
    }

    // ─── Idle：思考 ────────

    fn step_idle(
        &mut self,
        stm32: &STM32Device,
        wx: f32, wy: f32, yaw: f32,
        mission_queue: &mut MissionQueue,
    ) {
        // 检查是否到达 goal
        if let Some((gx, gy)) = self.goal {
            let dist = ((wx - gx).powi(2) + (wy - gy).powi(2)).sqrt();
            if dist < self.config.arrival_threshold_m {
                info!("[Executor] 到达目标 ({:.2}, {:.2})", gx, gy);
                self.goal = None;
                self.sub_target = None;
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
                }
                None => return,
            }
        }

        // 需要 sub_target → 计算
        if self.sub_target.is_none() {
            let (gx, gy) = self.goal.unwrap();
            let grid_x = (gx / 0.5).round() as i32;
            let grid_y = (gy / 0.5).round() as i32;
            let current_gx = (wx / 0.5).round() as i32;
            let current_gy = (wy / 0.5).round() as i32;

            if (grid_x, grid_y) == (current_gx, current_gy) {
                info!("[Executor] 已在目标格，到达");
                self.goal = None;
                self.sub_target = None;
                if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
                return;
            }

            self.sub_target = Some((grid_x, grid_y));
        }

        // 计算角偏差（归一化到 [-π, π]）
        let (st_x, st_y) = self.sub_target.unwrap();
        let target_wx = st_x as f32 * 0.5;
        let target_wy = st_y as f32 * 0.5;
        let target_angle = (target_wy - wy).atan2(target_wx - wx);
        let mut delta = target_angle - yaw;
        delta = (delta + PI).rem_euclid(2.0 * PI) - PI;

        let threshold_rad = self.config.turn_align_threshold_deg.to_radians();

        if delta.abs() > threshold_rad {
            if delta > 0.0 {
                if let Err(e) = stm32.spin_left(self.config.turn_speed) { warn!("[Executor] SpinLeft 失败: {e}"); }
            } else {
                if let Err(e) = stm32.spin_right(self.config.turn_speed) { warn!("[Executor] SpinRight 失败: {e}"); }
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

        // 实时计算角偏差
        let target_wx = st_x as f32 * 0.5;
        let target_wy = st_y as f32 * 0.5;
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

    fn step_moving(&mut self, stm32: &STM32Device, wx: f32, wy: f32) {
        let (st_x, st_y) = match self.sub_target {
            Some(st) => st,
            None => {
                self.state = ExecState::Idle;
                return;
            }
        };

        let target_wx = st_x as f32 * 0.5;
        let target_wy = st_y as f32 * 0.5;
        let dist = ((wx - target_wx).powi(2) + (wy - target_wy).powi(2)).sqrt();

        if dist < self.config.sub_target_threshold_m {
            self.sub_target = None;
            self.state = ExecState::Idle;
            if let Err(e) = stm32.stop() { warn!("[Executor] Stop 失败: {e}"); }
        }
    }

    /// 外部切换模式时重置
    pub fn reset(&mut self) {
        self.state = ExecState::Idle;
        self.goal = None;
        self.sub_target = None;
    }
}
