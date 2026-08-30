//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-08-30

//! 目标服务（Task 22 步骤 5，Task 22_3 改造：get_path 移入 main_loop，不再维护 sub_target）
//!
//! `get_path()` = 完整目标服务：内部封装「到达检测 + 任务切换（pop + 群发分配 + 设 goal 进 D*）+ 寻路」。
//! 只返回「下一步格」或 None（无任务/到达/不可达）；sub_target 维护移入 `ExecuteState`（main_loop 单点写）。
//! 急停（前方障碍强制 stop）在 Rust 侧 main_loop，不在此处。
//!
//! Task 23 决策 D1：寻路（D* Lite）下沉设备端，由本服务直接持有（不再经 world）。

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use pleiades_base::robot::core::cluster::ClusterInfoTable;
use pleiades_base::robot::core::command::Mission;
use pleiades_base::robot::core::mission::MissionQueue;
use crate::uav::planning::pathfinder::DStarLite;
use crate::uav::planning::{assignment, cluster_to_obstacle_cells};
use pleiades_base::robot::core::state::RobotState;
use pleiades_base::robot::core::grid::{OccupancyGrid, CELL_RESOLUTION};

/// 到达判定阈值（米）
pub const ARRIVAL_THRESHOLD_M: f32 = 0.3;

/// 目标服务：任务管理 + 寻路（Rust 侧「通用任务执行框架」）
pub struct GoalService {
    robot_state: Arc<RwLock<RobotState>>,
    mission_queue: Arc<RwLock<MissionQueue>>,
    own_peer_id: Vec<u8>,
    grid: Arc<RwLock<OccupancyGrid>>,
    cluster_table: Arc<ClusterInfoTable>,
    obstacle_inflation_radius: f32,
    arrival_threshold_m: f32,
    /// 当前 Mission 最终目标（世界坐标，米）
    goal: Option<(f32, f32)>,
    /// D* Lite 路径规划器（决策 D1：下沉设备端，GoalService 直接持有）
    pathfinder: Option<DStarLite>,
}

impl GoalService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        robot_state: Arc<RwLock<RobotState>>,
        mission_queue: Arc<RwLock<MissionQueue>>,
        own_peer_id: Vec<u8>,
        grid: Arc<RwLock<OccupancyGrid>>,
        cluster_table: Arc<ClusterInfoTable>,
        obstacle_inflation_radius: f32,
        arrival_threshold_m: f32,
    ) -> Self {
        Self {
            robot_state,
            mission_queue,
            own_peer_id,
            grid,
            cluster_table,
            obstacle_inflation_radius,
            arrival_threshold_m,
            goal: None,
            pathfinder: None,
        }
    }

    /// 完整目标服务：到达检测 + 任务切换 + 寻路，返回下一步格或 None（无任务/到达/不可达）
    pub async fn get_path(&mut self) -> Option<(i32, i32)> {
        let (wx, wy) = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y)
        };

        // ① 到达检测（世界距离阈值）
        if let Some((gx, gy)) = self.goal {
            let dist = ((wx - gx).powi(2) + (wy - gy).powi(2)).sqrt();
            if dist < self.arrival_threshold_m {
                info!("[Goal] 到达目标 ({:.2}, {:.2})", gx, gy);
                self.goal = None;
                self.pathfinder = None;
                return None;
            }
        }

        // ② 任务切换：无 goal → pop 下一个 Mission + 群发分配 + 设 goal 进 D*
        if self.goal.is_none() {
            let mission = self.mission_queue.write().await.pop_next();
            match mission {
                Some(Mission::Goto { x, y, members }) => {
                    info!("[Goal] 新任务 Goto({:.2}, {:.2})", x, y);
                    let goal = {
                        let g = self.grid.read().await;
                        assignment::group_goto_mission((x, y), &members, &self.own_peer_id, &g)
                    };
                    match goal {
                        Ok(goal) => self.start_goal(goal, wx, wy),
                        Err(e) => {
                            warn!("[Goal] 群发任务分配失败: {e:?}，跳过此任务");
                            return None;
                        }
                    }
                }
                Some(Mission::Circle { x, y, members }) => {
                    info!("[Goal] 新任务 Circle({:.2}, {:.2})", x, y);
                    let goal = {
                        let g = self.grid.read().await;
                        assignment::group_circle_mission((x, y), &members, &self.own_peer_id, &g)
                    };
                    match goal {
                        Ok(goal) => self.start_goal(goal, wx, wy),
                        Err(e) => {
                            warn!("[Goal] 围圈任务分配失败: {e:?}，跳过此任务");
                            return None;
                        }
                    }
                }
                None => {
                    // 无任务 → 无下一步
                    return None;
                }
            }
        }

        // ③ 网格级到达检查：已在目标格但世界距离未达标 → 视为到达
        if let Some((gx, gy)) = self.goal {
            let goal_gx = (gx / CELL_RESOLUTION).floor() as i32;
            let goal_gy = (gy / CELL_RESOLUTION).floor() as i32;
            let cur_gx = (wx / CELL_RESOLUTION).floor() as i32;
            let cur_gy = (wy / CELL_RESOLUTION).floor() as i32;
            if cur_gx == goal_gx && cur_gy == goal_gy {
                info!("[Goal] 已在目标格，到达");
                self.goal = None;
                self.pathfinder = None;
                return None;
            }
        }

        // ④ 寻路：move_to 当前位置 → 注入动态障碍 → next_step
        let dynamic_obstacles: Vec<(i32, i32)> = {
            let others = self.cluster_table.snapshot().await;
            cluster_to_obstacle_cells(&others, self.obstacle_inflation_radius).into_iter().collect()
        };
        let current_gx = (wx / CELL_RESOLUTION).floor() as i32;
        let current_gy = (wy / CELL_RESOLUTION).floor() as i32;
        let grid = self.grid.read().await;
        let next = match self.pathfinder.as_mut() {
            Some(pf) => {
                pf.move_to((current_gx, current_gy));
                pf.set_dynamic_obstacles(&dynamic_obstacles, &grid);
                pf.next_step(&grid)
            }
            None => None,
        };
        match next {
            Some(cell) => Some(cell),
            None => {
                warn!("[Goal] D* 不可达，跳过此任务");
                self.goal = None;
                self.pathfinder = None;
                None
            }
        }
    }

    /// 外部切换模式时重置
    pub fn reset(&mut self) {
        self.goal = None;
        self.pathfinder = None;
    }

    /// 急停标记障碍（触发 D* 局部修补；决策 D1：寻路下沉后由 GoalService 提供）
    pub async fn mark_obstacle(&mut self, cell: (i32, i32)) {
        let grid = self.grid.read().await;
        if let Some(pf) = self.pathfinder.as_mut() {
            pf.mark_obstacle(cell, &grid);
        }
    }

    /// 装载新目标：设 goal + 建 D* 路径（Goto / Circle 共用）
    fn start_goal(&mut self, goal: (f32, f32), wx: f32, wy: f32) {
        self.goal = Some(goal);
        let start_gx = (wx / CELL_RESOLUTION).floor() as i32;
        let start_gy = (wy / CELL_RESOLUTION).floor() as i32;
        let goal_gx = (goal.0 / CELL_RESOLUTION).floor() as i32;
        let goal_gy = (goal.1 / CELL_RESOLUTION).floor() as i32;
        self.pathfinder = Some(DStarLite::new((start_gx, start_gy), (goal_gx, goal_gy)));
    }
}
