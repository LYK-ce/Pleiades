//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-30

//! Robot 主循环（Task 23 阶段 C：base 骨架 + DeviceHandler trait）
//!
//! - `Robot::new()` 创建共享状态 + 非设备 task（不含设备 spawn / goal_service / 主循环）
//! - `Robot::run(device)` 跑主循环骨架，设备操作走 `DeviceHandler` trait
//! - 设备端（ugv/uav）各自实现 `DeviceHandler`，装配后传入

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use async_trait::async_trait;

use super::command::{AutoCmd, Command, ManualCmd, ModeCmd};
use super::mission::MissionQueue;
use super::mode::OpMode;
use crate::robot::core::state::{ExecuteState, RobotState};
use crate::robot::core::grid::OccupancyGrid;
use crate::robot::core::map_delta::MapDelta;

use crate::event_bus::EventBus;
use crate::robot::core::cluster::{cluster_consumer, cluster_table_cleaner, ClusterInfoTable};
use crate::robot::core::command_consumer::command_consumer;
use crate::network::{NodeHandle, TOPIC_ROBOT_POSE};
use crate::robot::core::protocol::{
    encode_frame, encode_pose,
    COMPID_ROBOT, MSGID_POSE, PoseData,
};

/// 位姿广播消息
#[derive(Debug, Clone)]
pub struct Pose {
    pub time_boot_ms: u32,
    pub x: f32, pub y: f32, pub z: f32,
    pub yaw: f32,
    pub vx: f32, pub vy: f32,
    /// 本车意图：D* 寻路下一格（Task 13_1，WS 下行带）
    pub sub_target: Option<(i32, i32)>,
}

/// Robot — 机器人系统中枢（base：只含共享状态，不含设备）
pub struct Robot {
    pub robot_cmd_tx: mpsc::Sender<Command>,
    pub robot_state: Arc<RwLock<RobotState>>,
    /// 占据栅格地图（SLAM task 写，WebSocket / 外部读）
    pub grid: Arc<RwLock<OccupancyGrid>>,
    /// 位姿广播
    pub pose_tx: broadcast::Sender<Pose>,
    /// 地图增量广播
    pub map_tx: broadcast::Sender<Vec<MapDelta>>,
    pub op_mode: Arc<RwLock<OpMode>>,
    pub mission_queue: Arc<RwLock<MissionQueue>>,
    /// 集群信息表（Task 13_1：远端车状态，consumer 写 / 未来消费端读）
    pub cluster_table: Arc<ClusterInfoTable>,
    /// 执行器状态（main_loop 唯一写，state_notifier 读）
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
}

/// 设备命令处理器（Task 23 阶段 C：base 定义接口，设备端各自实现，
/// 避免 base 反向依赖 ugv/uav 的具体设备类型）
#[async_trait]
pub trait DeviceHandler: Send + Sync {
    /// 启动设备（内部 spawn 自己需要的设备：车 spawn stm32+lidar，机 spawn mavlink）
    async fn start(&self) -> Result<(), String>;
    /// 处理手动命令（设备自己解析支持的命令子集）
    async fn handle_manual_cmd(&self, cmd: &ManualCmd);
    /// 任务重置（停车 + 清 goal + 清意图；模式切换 / 任务替换时调用）
    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>);
    /// 50ms 决策循环（急停 + 寻路 + 决策 + 发动作，设备端完整实现）
    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>);
    /// 停车
    fn stop(&self);
    /// 关设备（停车 + 停雷达 + 关设备）
    async fn shutdown(&self);
}

impl Robot {
    /// 创建共享状态 + 非设备 task，返回 (Self, 命令接收通道)。
    ///
    /// 设备装配（stm32/lidar/mavlink + goal_service + 决策器）下沉到设备端，
    /// 由 ugv/uav 各自的 bootstrap 完成。
    pub async fn new(
        origin: (f32, f32, f32),
        node_handle: Option<Arc<NodeHandle>>,
        robot_bus: Option<Arc<EventBus>>,
        robot_cmd_frame_rx: Option<mpsc::Receiver<Vec<u8>>>,
        peer_name: String,
    ) -> Result<(Self, mpsc::Receiver<Command>), String> {
        let cancel = CancellationToken::new();

        // 1. 创建共享状态
        let robot_state = Arc::new(RwLock::new(RobotState::default()));
        // 立即注入初始世界坐标，关闭 (0,0) 初始化窗口（首帧 RX 覆盖前消费方可见）
        {
            let mut g = robot_state.write().await;
            g.x = origin.0;
            g.y = origin.1;
            g.z = origin.2;
        }

        // 2. 广播通道
        let (pose_tx, _) = broadcast::channel::<Pose>(32);
        let (map_tx, _) = broadcast::channel::<Vec<MapDelta>>(32);
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));

        // 3. 运行模式 + 任务队列
        let op_mode = Arc::new(RwLock::new(OpMode::default()));
        let mission_queue = Arc::new(RwLock::new(MissionQueue::default()));

        // 3.1 执行器状态（main_loop 唯一写，state_notifier 读）
        let execute_state = Arc::new(RwLock::new(ExecuteState::default()));

        // 3.2 集群信息表（Task 13_1：入站 POSE → 表）
        let cluster_table = Arc::new(ClusterInfoTable::new());

        // 5. spawn state_notifier
        let notifier_cancel = cancel.clone();
        let notifier_state = robot_state.clone();
        let notifier_tx = pose_tx.clone();
        let notifier_handle = node_handle.clone();
        let notifier_name = peer_name.clone();
        // Task 13 阶段一：peer_id 为常量，launch 时算一次传闭包（顺带修 P3#9 每拍重算）
        let notifier_peer_id = node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes());
        // Task 13_1：execute_state 供组帧带意图
        let notifier_exec = execute_state.clone();
        tokio::spawn(async move {
            state_notifier(notifier_state, notifier_tx, notifier_handle, notifier_name, notifier_peer_id, notifier_exec, notifier_cancel).await;
        });

        // 7. 集群入站消费者（Task 13_1：robot_bus → ClusterInfo 表，独立 task 数据面）
        let consumer_bus = robot_bus.clone();
        let consumer_peer_id = node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes()).unwrap_or_default();
        let consumer_table = cluster_table.clone();
        // Task 13_2：入站 MAP_DELTA 应用需要 grid（只写 merged）
        let consumer_grid = grid.clone();
        let consumer_cancel = cancel.clone();
        tokio::spawn(async move {
            cluster_consumer(consumer_bus, consumer_peer_id, consumer_table, consumer_grid, consumer_cancel).await;
        });

        // 7.1 集群表维护（周期清理失联车：2s 周期 / 2s 超时，Task 15）
        let cleaner_table = cluster_table.clone();
        let cleaner_cancel = cancel.clone();
        tokio::spawn(async move {
            cluster_table_cleaner(cleaner_table, cleaner_cancel).await;
        });

        // 8. 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(32);

        // 8.1 命令入站消费者（request-response 命令帧 → Command，Task 16）
        if let Some(cmd_frame_rx) = robot_cmd_frame_rx {
            let consumer_tx = cmd_tx.clone();
            let consumer_cancel = cancel.clone();
            tokio::spawn(async move {
                command_consumer(cmd_frame_rx, consumer_tx, consumer_cancel).await;
            });
        }

        Ok((Self {
            robot_cmd_tx: cmd_tx,
            robot_state,
            grid,
            pose_tx,
            map_tx,
            op_mode,
            mission_queue,
            cluster_table,
            execute_state,
            cancel,
        }, cmd_rx))
    }

    /// 跑主循环（阻塞直到退出）。`device` 由设备端装配后传入。
    pub async fn run(&self, device: Arc<dyn DeviceHandler>, cmd_rx: mpsc::Receiver<Command>) {
        main_loop(
            device,
            cmd_rx,
            self.op_mode.clone(),
            self.mission_queue.clone(),
            self.robot_state.clone(),
            self.execute_state.clone(),
            self.cancel.clone(),
        ).await;
    }

    /// 优雅退出
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

// ============================================================
// state_notifier — 位姿广播 (100ms)
// ============================================================

async fn state_notifier(
    state: Arc<RwLock<RobotState>>,
    pose_tx: broadcast::Sender<Pose>,
    node_handle: Option<Arc<NodeHandle>>,
    _peer_name: String,
    local_peer_id: Option<Vec<u8>>,
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        select! {
            _ = interval.tick() => {
                // 先取快照并立即 drop 读锁（Task 23 review 修复：持锁跨 await 会挤压设备端
                // RX 回调的 try_write，造成位姿抖动/滞后）
                let (x, y, z, yaw, vx, vy, sub_target) = {
                    let s = state.read().await;
                    let es = execute_state.read().await;
                    (s.x, s.y, s.z, s.attitude.yaw, s.vx, s.vy, es.sub_target)
                };
                let time_boot_ms = crate::robot::core::protocol::now_boot_ms();
                let _ = pose_tx.send(Pose {
                    time_boot_ms,
                    x, y, z, yaw, vx, vy,
                    sub_target,
                });

                // Task 9_2：位姿广播到集群（fire-and-forget，Network 只搬运字节）
                if let (Some(nh), Some(peer_id)) = (&node_handle, &local_peer_id) {
                    let pose = PoseData {
                        time_boot_ms,
                        x, y, z,
                        vx, vy,
                        yaw,
                        valid: sub_target.is_some(),
                        sub_gx: sub_target.map(|(gx, _)| gx).unwrap_or(0),
                        sub_gy: sub_target.map(|(_, gy)| gy).unwrap_or(0),
                    };
                    let frame = encode_frame(MSGID_POSE, peer_id, COMPID_ROBOT, &encode_pose(&pose));
                    if let Err(e) = nh.Gossipsub_Publish(TOPIC_ROBOT_POSE, frame).await {
                        warn!("[Robot] 位姿广播失败: {e}");
                    }
                }
            }
            _ = cancel.cancelled() => {
                info!("state_notifier 退出");
                return;
            }
        }
    }
}

// ============================================================
// 主 select! 循环（单点执行，纯骨架）
// ============================================================

async fn main_loop(
    device: Arc<dyn DeviceHandler>,
    mut cmd_rx: mpsc::Receiver<Command>,
    op_mode: Arc<RwLock<OpMode>>,
    mission_queue: Arc<RwLock<MissionQueue>>,
    robot_state: Arc<RwLock<RobotState>>,
    execute_state: Arc<RwLock<ExecuteState>>,
    cancel: CancellationToken,
) {
    // 统一启动契约：设备自己决定启动哪些硬件
    if let Err(e) = device.start().await {
        warn!("[Robot] 设备启动失败: {e}");
        // 兜底清理：设备 start 内部可能已起部分资源（Task 23 review 修复）
        device.stop();
        device.shutdown().await;
        return;
    }
    info!("Robot 主循环启动（base 骨架 + 设备 handler）");

    let auto_tick_ms = 50u64;
    let mut next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);

    loop {
        select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Command::Mode(m)) => {
                        match m {
                            ModeCmd::SwitchToManual => {
                                info!("[Robot] 切换到 Manual 模式");
                                *op_mode.write().await = OpMode::Manual;
                                mission_queue.write().await.clear();
                                device.reset(&execute_state).await;
                            }
                            ModeCmd::SwitchToAuto => {
                                info!("[Robot] 切换到 Auto 模式");
                                *op_mode.write().await = OpMode::Auto;
                                device.reset(&execute_state).await;
                                next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
                            }
                        }
                    }

                    Some(Command::Manual(m)) => {
                        if *op_mode.read().await == OpMode::Manual {
                            device.handle_manual_cmd(&m).await;
                        } else {
                            warn!("[Robot] 忽略 Manual 命令：当前为 Auto 模式");
                        }
                    }

                    Some(Command::Auto(a)) => {
                        if *op_mode.read().await == OpMode::Auto {
                            match a {
                                AutoCmd::Set(list) => {
                                    // ORION_TASK_SET 替换语义（2026-08-07）：立即中断当前任务 + 整体替换队列
                                    info!("[Robot] Auto 任务队列已替换");
                                    mission_queue.write().await.replace(list);
                                    device.reset(&execute_state).await;
                                }
                            }
                        } else {
                            warn!("[Robot] 忽略 Auto 命令：当前为 Manual 模式");
                        }
                    }

                    None => {
                        info!("命令通道关闭，Robot 退出");
                        break;
                    }
                }
            }

            // auto_tick：仅 Auto 模式激活（决策循环下沉到设备端）
            _ = tokio::time::sleep_until(next_tick.into()), if *op_mode.read().await == OpMode::Auto => {
                let rs = robot_state.read().await.clone();
                device.on_tick(&rs, &execute_state).await;

                next_tick += Duration::from_millis(auto_tick_ms);
                if next_tick <= Instant::now() {
                    next_tick = Instant::now() + Duration::from_millis(auto_tick_ms);
                }
            }

            _ = cancel.cancelled() => {
                info!("Robot 收到退出信号");
                break;
            }
        }
    }

    // 退出前：停车 → 关设备
    info!("正在停止机器人...");
    device.stop();
    device.shutdown().await;
    info!("Robot 主循环已退出");
}
