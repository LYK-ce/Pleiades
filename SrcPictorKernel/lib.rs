//Presented by KeJi
//Created Date ： 2026-08-15
//Modified Date ： 2026-08-15

//! Pictor Kernel — Pleiades × Godot GDExtension 桥（Task 16）
//!
//! Godot 通过 `.gdextension` 加载本 `.so`（`libpictor_kernel.so`），
//! 在进程内运行 Pleiades 无头逻辑层（libp2p 网络 / ORION 协议 / 地图合并），
//! 通过信号（上行）与 handle（下行）跟 Godot 表现层交互。
//!
//! - 出站（Godot → 车）：`send_command`（Godot 拼好 ORION 帧，`Send_Data_Try` 转发）
//! - 入站（车 → Godot）：后台同步 task → `out_queue` → `poll()` 排空并 emit 信号（方案 B）
//! - 生命周期：`ready` 起后台线程（core_bootstrap + GroundStation + run_headless），
//!   `exit_tree` 置停机标志并 join。

use godot::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use pleiades::event_bus::Bus_Event;
use pleiades::network::{DataType, NodeHandle};
use pleiades::robot::core::cluster::ClusterInfoTable;
use pleiades::robot::core::ground_station::GroundStation;
use pleiades::robot::slam::OccupancyGrid;

/// 桥事件（后台线程 → 主线程 `poll()` 排空）
enum BridgeEvent {
    KernelReady,
    Pose {
        peer_id: String,
        x: f32,
        y: f32,
        yaw: f32,
        vx: f32,
        vy: f32,
    },
    Map {
        data: Vec<u8>,
    },
    PeerConnected {
        peer_id: String,
    },
    PeerDisconnected {
        peer_id: String,
    },
    PeerInfo {
        peer_id: String,
        name: String,
    },
}

/// Godot 里的唯一 Rust 类（extends Node）
#[derive(GodotClass)]
#[class(base = Node)]
struct PleiadesKernel {
    base: Base<Node>,
    /// 出站发送句柄（core_bootstrap 就绪后填充）
    node_handle: Arc<OnceLock<NodeHandle>>,
    /// 同步队列（后台 → 主线程 poll）
    out_queue: Arc<Mutex<VecDeque<BridgeEvent>>>,
    /// 优雅停机标志
    shutdown: Arc<AtomicBool>,
    /// 后台线程句柄
    worker: Option<JoinHandle<()>>,
}

#[godot_api]
impl INode for PleiadesKernel {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            node_handle: Arc::new(OnceLock::new()),
            out_queue: Arc::new(Mutex::new(VecDeque::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }

    fn ready(&mut self) {
        self.spawn_background();
    }

    fn exit_tree(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() {
            // 后台线程在 select! 里监听停机标志，join 会及时返回。
            let _ = handle.join();
        }
    }
}

#[godot_api]
impl PleiadesKernel {
    #[signal]
    fn kernel_ready();

    #[signal]
    fn pose_received(peer_id: GString, x: f32, y: f32, yaw: f32, vx: f32, vy: f32);

    #[signal]
    fn map_updated(data: PackedByteArray);

    #[signal]
    fn peer_connected(peer_id: GString);

    #[signal]
    fn peer_disconnected(peer_id: GString);

    #[signal]
    fn peer_info_updated(peer_id: GString, peer_name: GString);

    /// 下发命令帧（Godot 拼好的完整 ORION 帧，fire-and-forget，同步返回 bool）
    #[func]
    fn send_command(&self, peer_id: GString, frame: PackedByteArray) -> bool {
        let Some(node) = self.node_handle.get() else {
            godot_warn!("[Kernel] send_command 失败：节点未就绪");
            return false;
        };
        let Some(peer) = parse_peer_hex(&peer_id.to_string()) else {
            godot_warn!("[Kernel] send_command 失败：peer_id 非法");
            return false;
        };
        node.Send_Data_Try(&peer, DataType::Robot, frame.to_vec())
            .is_ok()
    }

    /// Godot 主线程每帧调用：排空 `out_queue` 并 emit 信号（方案 B：零跨线程发信号）
    #[func]
    fn poll(&mut self) {
        let events: Vec<BridgeEvent> = {
            let mut queue = self.out_queue.lock().unwrap();
            queue.drain(..).collect()
        };
        for ev in events {
            match ev {
                BridgeEvent::KernelReady => {
                    self.signals().kernel_ready().emit();
                }
                BridgeEvent::Pose {
                    peer_id,
                    x,
                    y,
                    yaw,
                    vx,
                    vy,
                } => {
                    let peer = GString::from(peer_id.as_str());
                    self.signals().pose_received().emit(&peer, x, y, yaw, vx, vy);
                }
                BridgeEvent::Map { data } => {
                    let pba = PackedByteArray::from(data);
                    self.signals().map_updated().emit(&pba);
                }
                BridgeEvent::PeerConnected { peer_id } => {
                    let peer = GString::from(peer_id.as_str());
                    self.signals().peer_connected().emit(&peer);
                }
                BridgeEvent::PeerDisconnected { peer_id } => {
                    let peer = GString::from(peer_id.as_str());
                    self.signals().peer_disconnected().emit(&peer);
                }
                BridgeEvent::PeerInfo { peer_id, name } => {
                    let peer = GString::from(peer_id.as_str());
                    let pname = GString::from(name.as_str());
                    self.signals().peer_info_updated().emit(&peer, &pname);
                }
            }
        }
    }
}

impl PleiadesKernel {
    fn spawn_background(&mut self) {
        let node_handle = self.node_handle.clone();
        let out_queue = self.out_queue.clone();
        let shutdown = self.shutdown.clone();

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let boot = match pleiades::bootstrap::core_bootstrap().await {
                    Ok(b) => b,
                    Err(e) => {
                        godot_error!("[Kernel] core_bootstrap 失败: {e}");
                        return;
                    }
                };

                let local_peer_id = boot.node_handle.Get_Local_Peer_Id().to_bytes();
                let _ = node_handle.set(boot.node_handle.clone());

                // GroundStation 消费侧（遥测 → 表 + 地图）
                let gs = GroundStation::launch(boot.robot_bus.clone(), local_peer_id);

                // 遥测/地图 同步 task → out_queue
                {
                    let (q, s, t, g) = (
                        out_queue.clone(),
                        shutdown.clone(),
                        gs.table.clone(),
                        gs.grid.clone(),
                    );
                    tokio::spawn(async move {
                        sync_loop(q, s, t, g).await;
                    });
                }
                // 节点事件 task（event_bus）→ out_queue
                {
                    let (q, s) = (out_queue.clone(), shutdown.clone());
                    let rx = boot.event_bus.Subscribe();
                    tokio::spawn(async move {
                        event_loop(q, s, rx).await;
                    });
                }

                // 就绪
                out_queue.lock().unwrap().push_back(BridgeEvent::KernelReady);

                // 无头主循环（阻塞）；停机标志触发时 select! 提前返回
                let sh = shutdown.clone();
                tokio::select! {
                    _ = boot.run_headless() => {
                        godot_print!("[Kernel] 无头节点正常退出");
                    }
                    _ = async move {
                        while !sh.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    } => {
                        godot_print!("[Kernel] 收到停机信号，退出");
                    }
                }
            });
        });
        self.worker = Some(handle);
    }
}

// ===== helpers =====

/// hex 字符串 → libp2p PeerId（与 WS hello 的 hex-of-bytes 一致）
fn parse_peer_hex(s: &str) -> Option<libp2p::PeerId> {
    let bytes = hex::decode(s).ok()?;
    libp2p::PeerId::from_bytes(&bytes).ok()
}

/// peer_id 字节 → hex 字符串
fn peer_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 遥测/地图同步 task（后台 runtime）：周期读 table/grid → out_queue
async fn sync_loop(
    out_queue: Arc<Mutex<VecDeque<BridgeEvent>>>,
    shutdown: Arc<AtomicBool>,
    table: Arc<ClusterInfoTable>,
    grid: Arc<tokio::sync::RwLock<OccupancyGrid>>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        interval.tick().await;
        if shutdown.load(Ordering::SeqCst) {
            return;
        }

        // 位姿（按车发）
        for info in table.snapshot().await {
            out_queue.lock().unwrap().push_back(BridgeEvent::Pose {
                peer_id: peer_hex(&info.peer_id),
                x: info.x,
                y: info.y,
                yaw: info.yaw,
                vx: info.vx,
                vy: info.vy,
            });
        }

        // 地图（合并全量 log-odds，i8 → u8 位模式）
        let data: Vec<u8> = grid
            .read()
            .await
            .log_odds_bytes()
            .iter()
            .map(|&v| v as u8)
            .collect();
        out_queue.lock().unwrap().push_back(BridgeEvent::Map { data });
    }
}

/// 节点事件 task（后台 runtime）：订阅 event_bus → out_queue
async fn event_loop(
    out_queue: Arc<Mutex<VecDeque<BridgeEvent>>>,
    shutdown: Arc<AtomicBool>,
    mut rx: tokio::sync::broadcast::Receiver<Bus_Event>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        match rx.recv().await {
            Ok(Bus_Event::State { payload }) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) {
                    let ty = v["type"].as_str().unwrap_or("");
                    let peer_id = v["peer_id"].as_str().unwrap_or("").to_string();
                    match ty {
                        "peer_connected" => out_queue
                            .lock()
                            .unwrap()
                            .push_back(BridgeEvent::PeerConnected { peer_id }),
                        "peer_disconnected" => out_queue
                            .lock()
                            .unwrap()
                            .push_back(BridgeEvent::PeerDisconnected { peer_id }),
                        "peer_info_updated" => {
                            let name = v["peer_name"].as_str().unwrap_or("").to_string();
                            out_queue
                                .lock()
                                .unwrap()
                                .push_back(BridgeEvent::PeerInfo { peer_id, name });
                        }
                        _ => {}
                    }
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
        }
    }
}


/// GDExtension 入口点声明（生成 entry_symbol = `pictor_kernel_init`，供 .gdextension 引用）
struct PictorKernelExtension;

#[gdextension]
unsafe impl ExtensionLibrary for PictorKernelExtension {
    fn on_stage_init(stage: godot::init::InitStage) {
        // PleiadesKernel 类已通过 #[derive(GodotClass)] 自动注册，无需手动注册
        if stage == godot::init::InitStage::Scene {
            godot_print!("[Kernel] GDExtension 初始化完成 (Scene)");
        }
    }
}
