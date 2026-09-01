//Presented by KeJi
//Created Date ： 2026-08-15
//Modified Date ： 2026-08-16

//! Pictor Kernel — Pleiades × Godot GDExtension 桥（Task 16）
//!
//! Godot 通过 `.gdextension` 加载本 `.so`（`libpictor_kernel.so`），
//! 在进程内运行 Pleiades 无头逻辑层（libp2p 网络）。桥是**哑管道**，不解析、不合并业务数据：
//!
//! - 下行（Godot → 车）：`send_command`（Godot 拼好 ORION 帧，`Send_Data_Try` 转发）
//! - 上行（车 → Godot）：订阅 `robot_bus` 转发原始 ORION 帧 + 订阅 `event_bus` 转发 peer 事件
//! - 生命周期：`ready` 起后台线程（core_bootstrap + run_headless），`exit_tree` 停机并 join。

mod config;
mod device;

use godot::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use pleiades_base::event_bus::Bus_Event;
use pleiades_base::network::{DataType, NodeHandle};

/// 桥事件（后台线程 → 主线程 `poll()` 排空）
enum BridgeEvent {
    KernelReady,
    /// 原始 ORION 帧（POSE / MAP_DELTA / MAP_FULL，由 Godot 侧解码）
    RobotFrame {
        data: Vec<u8>,
    },
    PeerDiscovered {
        peer_id: String,
    },
    PeerLeft {
        peer_id: String,
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
        node_type: String,
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

    /// 原始 ORION 帧（Godot 侧用现有 `MessageParser.parse_orion_frame` 解码并分发）
    #[signal]
    fn robot_frame(data: PackedByteArray);

    #[signal]
    fn peer_discovered(peer_id: GString);

    #[signal]
    fn peer_left(peer_id: GString);

    #[signal]
    fn peer_connected(peer_id: GString);

    #[signal]
    fn peer_disconnected(peer_id: GString);

    #[signal]
    fn peer_info_updated(peer_id: GString, peer_name: GString, node_type: GString);

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
                BridgeEvent::RobotFrame { data } => {
                    let pba = PackedByteArray::from(data);
                    self.signals().robot_frame().emit(&pba);
                }
                BridgeEvent::PeerDiscovered { peer_id } => {
                    let peer = GString::from(peer_id.as_str());
                    self.signals().peer_discovered().emit(&peer);
                }
                BridgeEvent::PeerLeft { peer_id } => {
                    let peer = GString::from(peer_id.as_str());
                    self.signals().peer_left().emit(&peer);
                }
                BridgeEvent::PeerConnected { peer_id } => {
                    let peer = GString::from(peer_id.as_str());
                    self.signals().peer_connected().emit(&peer);
                }
                BridgeEvent::PeerDisconnected { peer_id } => {
                    let peer = GString::from(peer_id.as_str());
                    self.signals().peer_disconnected().emit(&peer);
                }
                BridgeEvent::PeerInfo { peer_id, name, node_type } => {
                    let peer = GString::from(peer_id.as_str());
                    let pname = GString::from(name.as_str());
                    let ntype = GString::from(node_type.as_str());
                    self.signals().peer_info_updated().emit(&peer, &pname, &ntype);
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
                let boot = match pleiades_base::bootstrap::core_bootstrap().await {
                    Ok(b) => b,
                    Err(e) => {
                        godot_error!("[Kernel] core_bootstrap 失败: {e}");
                        return;
                    }
                };

                let _ = node_handle.set(boot.node_handle.clone());

                // RTK 基站（UM960）：enabled 才 spawn，失败只告警不拖垮节点（Task 24）
                // 句柄存活到 run_headless 返回前（随后台线程生命周期自然释放）
                let _um960 = match config::Ensure_Terminal_Config() {
                    Ok(cfg) => {
                        if let Some(u) = cfg.um960.as_ref().filter(|u| u.enabled.unwrap_or(false)) {
                            let port = u.port.clone().unwrap_or_else(|| "/dev/ttyUSB0".into());
                            let baud = u.baudrate.unwrap_or(460800);
                            let secs = u.survey_seconds.unwrap_or(180);
                            match device::um960::Um960Device::spawn(
                                &port, baud, secs, Arc::new(boot.node_handle.clone()),
                            ) {
                                Ok(d) => Some(d),
                                Err(e) => {
                                    godot_warn!("[Kernel] UM960 启动失败: {e}");
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    }
                    Err(e) => {
                        godot_warn!("[Kernel] 读取 [um960] 配置失败: {e}");
                        None
                    }
                };

                // 上行 1：robot_bus 原始帧转发 → out_queue
                {
                    let (q, s) = (out_queue.clone(), shutdown.clone());
                    let rx = boot.robot_bus.Subscribe();
                    tokio::spawn(async move {
                        robot_forward_loop(q, s, rx).await;
                    });
                }
                // 上行 2：event_bus peer 事件 → out_queue
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

/// base58 字符串 → hex 字符串（统一 peer 事件的 peer_id 编码，Task 16）
fn base58_to_hex(s: &str) -> Option<String> {
    let peer: libp2p::PeerId = s.parse().ok()?;
    Some(peer_hex(&peer.to_bytes()))
}

/// robot_bus 原始帧转发 task：订阅 robot_bus → 转发原始 ORION 帧（不解析）
async fn robot_forward_loop(
    out_queue: Arc<Mutex<VecDeque<BridgeEvent>>>,
    shutdown: Arc<AtomicBool>,
    mut rx: tokio::sync::broadcast::Receiver<Bus_Event>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        match rx.recv().await {
            Ok(Bus_Event::StreamRaw { payload }) => {
                out_queue
                    .lock()
                    .unwrap()
                    .push_back(BridgeEvent::RobotFrame { data: payload });
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
        }
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
                    // 统一为 hex：event_bus 里的 peer_id 是 base58，转成 hex 与 send_command 一致
                    let Some(peer_id) = base58_to_hex(v["peer_id"].as_str().unwrap_or("")) else {
                        continue;
                    };
                    match ty {
                        "peer_discovered" => out_queue
                            .lock()
                            .unwrap()
                            .push_back(BridgeEvent::PeerDiscovered { peer_id }),
                        "peer_left" => out_queue
                            .lock()
                            .unwrap()
                            .push_back(BridgeEvent::PeerLeft { peer_id }),
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
                            let node_type = v["node_type"].as_str().unwrap_or("").to_string();
                            out_queue
                                .lock()
                                .unwrap()
                                .push_back(BridgeEvent::PeerInfo { peer_id, name, node_type });
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

/// GDExtension 入口点声明（生成 entry_symbol = `gdext_rust_init`，供 .gdextension 引用）
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
