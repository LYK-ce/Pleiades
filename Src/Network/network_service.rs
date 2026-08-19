//Presented by KeJi
//Created Date ： 2026-04-29
//Modified Date ： 2026-08-18

//! 网络服务核心模块
//! 负责Swarm管理、连接管理、事件处理
//!
//! 架构设计：
//! - Network_Service: 网络服务实例，运行事件循环
//! - NodeHandle: 对外暴露的API句柄（定义在 node_handle.rs）
//! - NodeCommand: 外部命令枚举（定义在 node_handle.rs）
//! - Inbound_Manager: 入站请求与响应路由管理器（定义在 request_response/inbound.rs）
//! - Outbound_Manager: 出站响应路由管理器（定义在 request_response/outbound.rs）
//!
//! 事件分流：
//! - Response → 通过 oneshot 路由回 Send_Data 调用方
//! - 入站 Request（DataType 预筛选）：
//!   - Command / File → 通过 inbound_tx 转发给 Orchestrator Core B5
//!   - Data / Info → Network 内部直接回复，不转发
//! - 入站带宽流 → Network 内部 Receive_And_Count + 回传结果
//! - 入站文件流 → 通过 orchestrator_event_tx 转发给 Orchestrator
//! - 入站张量流 → 通过 orchestrator_event_tx 转发给 Orchestrator
//! - 连接/发现/Ping 事件 → Network 内部处理（PeerManager）
//!
//! 注意：
//! 我们当前暂时先不考虑广域网的环境，只专注于当前的局域网环境。
//! 广域网放到未来支持。
#[allow(nonstandard_style)]
use libp2p::{
    gossipsub,
    identify,
    identity::Keypair,
    kad::{self, store::MemoryStore, Mode},
    mdns,
    noise,
    ping,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour},
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use libp2p_stream as stream;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};
use futures::StreamExt;

use crate::event_bus::EventBus;

use super::request_response::{
    PleiadesCodec, DATA_PROTOCOL,
    Inbound_Manager, Outbound_Manager,
};
use super::node_handle::{NodeCommand, NodeHandle, InboundRequest};
use super::capability::{Network_Inbound_Event, Network_Service_Capability};
use super::file_stream::protocol::FILE_STREAM_PROTOCOL;
use super::tensor_stream::protocol::TENSOR_STREAM_PROTOCOL;
use super::tensor_stream::rendezvous::RendezvousMap;
use super::Gossipsub::SnapshotCache;
use super::bandwidth_stream::protocol::{
    BANDWIDTH_STREAM_PROTOCOL,
    Receive_And_Count, Write_Bandwidth_Result,
};

// 导入 PeerManagement 模块
use crate::peer_management::Peer_Management_Capability;

/// 网络配置
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// 是否启用局域网发现
    pub lan_enabled: bool,
    /// 是否启用广域网发现
    pub wan_enabled: bool,
    /// 传输协议 ("TCP" 或 "QUIC")
    pub transport_protocol: String,
    /// 监听端口 (0表示随机)
    pub listen_port: u16,
    /// DHT 节点发现命名空间
    pub dht_namespace: String,
    /// 引导节点地址列表
    pub bootstrap_peers: Vec<String>,
    /// 清理间隔（秒）
    pub cleanup_interval: u64,
    /// 超时间隔（秒）
    pub timeout_interval: u64,
    /// 心跳间隔（秒）
    pub heartbeat_interval: u64,
    /// 心跳超时（秒）
    pub heartbeat_timeout: u64,
    /// Request-Response 协议超时（秒），默认300
    pub request_response_timeout: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            lan_enabled: true,
            wan_enabled: false,
            transport_protocol: "TCP".to_string(),
            listen_port: 0,
            dht_namespace: crate::network::DHT::DEFAULT_NODE_NAMESPACE.to_string(),
            bootstrap_peers: Vec::new(),
            cleanup_interval: 300,    // 默认300秒
            timeout_interval: 300,    // 默认300秒
            heartbeat_interval: 60,   // 默认60秒
            heartbeat_timeout: 10,    // 默认10秒
            request_response_timeout: 300, // 默认300秒
        }
    }
}

/// 网络行为组合
/// 网络行为组合
#[derive(NetworkBehaviour)]
pub struct PleiadesNetworkBehaviour {
    /// mDNS局域网发现
    pub mdns: mdns::tokio::Behaviour,
    /// Kademlia DHT
    pub kademlia: kad::Behaviour<MemoryStore>,
    /// 请求响应协议
    pub request_response: request_response::Behaviour<PleiadesCodec>,
    /// 流式传输协议
    pub stream: stream::Behaviour,
    /// Ping心跳协议
    pub ping: ping::Behaviour,
    /// Identify 节点识别（连接建立后自动交换协议级元信息）
    pub identify: identify::Behaviour,
    /// GossipSub 主题广播（业务状态：peer-info / models / sessions）
    pub gossipsub: gossipsub::Behaviour,
}

/// 网络服务（内部实现）
#[allow(nonstandard_style)]
pub struct Network_Service {
    /// Swarm实例
    pub(crate) swarm: Swarm<PleiadesNetworkBehaviour>,
    /// 本地节点ID
    pub(crate) local_peer_id: PeerId,
    /// peer manager capability 持有的节点管理能力 trait object，我们通过它来管理节点信息表
    pub(crate) peer_handle: Arc<dyn Peer_Management_Capability>,
    /// 命令接收器（接收外部命令）
    pub(crate) cmd_rx: mpsc::Receiver<NodeCommand>,
    /// 配置
    pub(crate) config: NetworkConfig,
    /// 全局事件总线，用于发布网络层事件（节点发现/离开、连接建立/断开等）
    pub(crate) event_bus: Arc<EventBus>,
    /// Robot 数据专用事件总线（Task 9_1：位姿/地图高频数据与全局总线隔离）
    pub(crate) robot_bus: Arc<EventBus>,

    // ===== 组件化管理器 =====

    /// 入站请求管理器（负责入站请求分发、回复管理）
    pub(crate) inbound_manager: Inbound_Manager,
    /// 出站响应路由管理器（负责出站 Response 路由回调用方）
    pub(crate) outbound_manager: Outbound_Manager,

    // ===== Orchestrator 事件转发 =====

    /// 转发复杂入站事件（文件流、张量流）给 Orchestrator
    pub(crate) orchestrator_event_tx: mpsc::Sender<Network_Inbound_Event>,

    // ===== 流控制句柄（用于 accept 入站流） =====

    /// 文件流控制句柄（用于 Start() 中 accept 入站文件流）
    pub(crate) file_accept_control: stream::Control,
    /// 张量流控制句柄（用于 Start() 中 accept 入站张量流）
    pub(crate) tensor_accept_control: stream::Control,

    /// 带宽测试流控制 — accept 入站测试流
    pub(crate) bandwidth_accept_control: stream::Control,
    /// Session 流控制 — accept 入站 session 流
    pub(crate) session_accept_control: stream::Control,

    // ===== Tensor Stream Rendezvous =====

    /// 张量流双向匹配器（与 Network_Service_Capability 共享）
    pub(crate) rendezvous: Arc<RendezvousMap>,
    /// GossipSub 快照缓存（最近发布状态，对方订阅 topic 时按 topic 精准重放）
    pub(crate) snapshot_cache: SnapshotCache,
    /// 进行中的 DHT get_providers 查询跟踪（QueryId → 结果回传通道）
    pub(crate) provider_queries: crate::network::DHT::ProviderQueryTracker,
}

impl Network_Service {
    /// 初始化网络服务
    ///
    /// # Arguments
    /// * `config` - 网络配置
    /// * `keypair` - 密钥对
    /// * `peer_handle` - PeerManager Capability，用于管理节点信息
    /// * `event_bus` - 全局事件总线，用于发布网络事件
    ///
    /// # Returns
    /// (Network_Service实例, NodeHandle句柄, inbound_rx 入站请求接收端,
    ///  Network_Service_Capability, orchestrator_event_rx 入站事件接收端)
    pub async fn Init(
        config: NetworkConfig,
        keypair: Keypair,
        peer_handle: Arc<dyn Peer_Management_Capability>,
        event_bus: Arc<EventBus>,
        robot_bus: Arc<EventBus>,
    ) -> Result<(
        Self,
        NodeHandle,
        mpsc::Receiver<InboundRequest>,
        Network_Service_Capability,
        mpsc::Receiver<Network_Inbound_Event>,
    ), Box<dyn Error>> {
        info!("初始化网络服务...");

        // 1. 使用传入的持久化密钥对
        let local_peer_id = PeerId::from(keypair.public());
        info!("本地节点ID: {}", local_peer_id);

        // 2. 创建命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<NodeCommand>(100);

        // 3. 创建入站请求通道
        let (inbound_tx, inbound_rx) = mpsc::channel::<InboundRequest>(100);

        // 3.1 创建 Orchestrator 事件通道
        let (orchestrator_event_tx, orchestrator_event_rx) =
            mpsc::channel::<Network_Inbound_Event>(100);

        // 4. 创建Swarm
        let node_swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default().nodelay(true),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                // 创建mDNS行为
                let mdns = mdns::tokio::Behaviour::new(
                    mdns::Config::default(),
                    key.public().to_peer_id(),
                )?;

                // 创建Kademlia行为
                let store = MemoryStore::new(key.public().to_peer_id());
                let mut kademlia = kad::Behaviour::new(key.public().to_peer_id(), store);

                // 设置为服务器模式（可被发现）
                kademlia.set_mode(Some(Mode::Server));

                // 创建请求响应行为 — 统一 DATA_PROTOCOL
                let protocols = [(
                    StreamProtocol::new(DATA_PROTOCOL),
                    ProtocolSupport::Full,
                )];
                let cfg = request_response::Config::default()
                    .with_request_timeout(Duration::from_secs(config.request_response_timeout));
                let request_response =
                    request_response::Behaviour::<PleiadesCodec>::new(protocols, cfg);

                // 创建流式传输行为
                let stream = stream::Behaviour::new();

                // 创建Ping心跳行为，使用配置的heartbeat_interval和heartbeat_timeout
                let ping_config = ping::Config::new()
                    .with_interval(Duration::from_secs(config.heartbeat_interval))
                    .with_timeout(Duration::from_secs(config.heartbeat_timeout));
                let ping_behaviour = ping::Behaviour::new(ping_config);

                // 创建 Identify 行为 — 连接建立后自动交换节点元信息（protocols/地址/版本）
                let identify = identify::Behaviour::new(
                    identify::Config::new(
                        "/pleiades/1.0.0".to_string(),
                        key.public().clone(),
                    )
                    .with_agent_version(format!("pleiades/{}", env!("CARGO_PKG_VERSION")))
                    .with_interval(Duration::from_secs(60))
                    .with_push_listen_addr_updates(true),
                );

                // 创建 GossipSub 行为 — 业务状态主题广播（Signed 消息认证）
                let gossipsub_config = gossipsub::ConfigBuilder::default().build()?;
                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )?;

                Ok(PleiadesNetworkBehaviour {
                    mdns,
                    kademlia,
                    request_response,
                    stream,
                    ping: ping_behaviour,
                    identify,
                    gossipsub,
                })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(86400)))
            .build();

        // 5. 创建流控制句柄
        //    - file_accept_control: Network_Service 用于 accept 入站文件流
        //    - file_open_control: Capability 用于 open 出站文件流
        //    - tensor_accept_control: Network_Service 用于 accept 入站张量流
        //    - tensor_open_control: Capability 用于 open 出站张量流
        let file_accept_control = node_swarm.behaviour().stream.new_control();
        let file_open_control = node_swarm.behaviour().stream.new_control();
        let tensor_accept_control = node_swarm.behaviour().stream.new_control();
        let tensor_open_control = node_swarm.behaviour().stream.new_control();
        let bandwidth_accept_control = node_swarm.behaviour().stream.new_control();
        let bandwidth_stream_control = node_swarm.behaviour().stream.new_control();
        let session_accept_control = node_swarm.behaviour().stream.new_control();
        let session_stream_control = node_swarm.behaviour().stream.new_control();

        // 6. 创建入站请求管理器和出站响应路由管理器
        let inbound_manager = Inbound_Manager::New(inbound_tx);
        let outbound_manager = Outbound_Manager::New();

        // 7. 创建 NodeHandle（传入超时配置）
        let handle = NodeHandle::New(cmd_tx, local_peer_id, config.request_response_timeout);

        // 7.1 创建 RendezvousMap（Network_Service 和 Capability 共享）
        let rendezvous = Arc::new(RendezvousMap::new());

        // 8. 创建 Network_Service_Capability（用于 Orchestrator）
        let capability = Network_Service_Capability::New(
            handle.clone(),
            file_open_control,
            tensor_open_control,
            bandwidth_stream_control,
            session_stream_control,
            rendezvous.clone(),
            event_bus.clone(),
        );

        // 9. 创建 Network_Service 实例
        let mut node = Self {
            swarm: node_swarm,
            local_peer_id,
            peer_handle,
            cmd_rx,
            config,
            event_bus,
            robot_bus,
            inbound_manager,
            outbound_manager,
            orchestrator_event_tx,
            file_accept_control,
            tensor_accept_control,
            bandwidth_accept_control,
            session_accept_control,
            rendezvous,
            snapshot_cache: SnapshotCache::New(),
            provider_queries: crate::network::DHT::ProviderQueryTracker::new(),
        };

        // 添加引导节点
        for addr_str in &node.config.bootstrap_peers {
            if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                // 从 Multiaddr 末尾 /p2p/<PeerId> 提取真实 PeerId（替代 PeerId::random）
                let peer_id = match addr.iter().last() {
                    Some(libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
                    _ => {
                        warn!("bootstrap 地址缺少 /p2p/<PeerId> 后缀，跳过: {}", addr);
                        continue;
                    }
                };
                info!("添加引导节点: {}", addr);
                node.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
            }
        }

        info!("网络节点初始化完成");
        Ok((node, handle, inbound_rx, capability, orchestrator_event_rx))
    }

    /// 启动网络服务
    ///
    /// 使用tokio::select!同时监听Swarm事件、外部命令和入站流
    pub async fn Start(&mut self) -> Result<(), Box<dyn Error>> {
        // 1. 启动监听
        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", self.config.listen_port).parse()?;
        self.swarm.listen_on(listen_addr)?;
        info!("开始监听网络连接");

        // 2. 启动Kademlia引导（仅当配置了种子节点时）
        if !self.config.bootstrap_peers.is_empty() {
            if let Err(e) = self.swarm.behaviour_mut().kademlia.bootstrap() {
                warn!("Kademlia引导失败: {}", e);
            }
        }

        // 2.4 自注册为 Pleiades 节点 provider（供 get_providers 发现）
        crate::network::DHT::start_providing(
            &mut self.swarm.behaviour_mut().kademlia,
            &self.config.dht_namespace,
        );
        // 2.5 订阅 GossipSub 业务状态 topic（IdentTopic = 原始字符串 topic，与默认配置一致）
        let topics = [
            gossipsub::IdentTopic::new(super::TOPIC_PEER_INFO),
            gossipsub::IdentTopic::new(super::TOPIC_MODELS),
            gossipsub::IdentTopic::new(super::TOPIC_SESSIONS),
            gossipsub::IdentTopic::new(super::TOPIC_ROBOT_POSE),
            gossipsub::IdentTopic::new(super::TOPIC_ROBOT_MAP),
        ];
        for t in &topics {
            if let Err(e) = self.swarm.behaviour_mut().gossipsub.subscribe(t) {
                warn!("GossipSub 订阅失败 ({}): {}", t, e);
            }
        }
        info!("GossipSub 已订阅 {} 个 topic", topics.len());

        // 3. 注册流式传输协议，接受入站流
        let mut incoming_file_streams = self.file_accept_control
            .accept(StreamProtocol::new(FILE_STREAM_PROTOCOL))
            .expect("文件流协议注册失败");

        let mut incoming_tensor_streams = self.tensor_accept_control
            .accept(StreamProtocol::new(TENSOR_STREAM_PROTOCOL))
            .expect("张量流协议注册失败");

        let mut incoming_bandwidth_streams = self.bandwidth_accept_control
            .accept(StreamProtocol::new(BANDWIDTH_STREAM_PROTOCOL))
            .expect("带宽测试流协议注册失败");

        let mut incoming_session_streams = self.session_accept_control
            .accept(StreamProtocol::new(super::session_stream::protocol::SESSION_STREAM_PROTOCOL))
            .expect("Session 流协议注册失败");

        // 4. 进入事件循环（使用select!同时监听网络事件、命令和入站流）
        info!("进入网络事件循环");
        loop {
            tokio::select! {
                // 处理Swarm网络事件
                event = self.swarm.select_next_some() => {
                    if !self.Handle_Swarm_Event(event).await {
                        break;
                    }
                }
                // 处理来自本系统内部其他组件的命令，主要是来自core的命令。
                Some(cmd) = self.cmd_rx.recv() => {
                    if !self.Handle_Command(cmd).await {
                        break;
                    }
                }
                // 处理入站文件流 → 转发给 Orchestrator
                Some((peer_id, stream)) = incoming_file_streams.next() => {
                    info!("收到入站文件流 from {}, 转发给 Orchestrator", peer_id);
                    let _ = self.orchestrator_event_tx.send(
                        Network_Inbound_Event::FileStreamArrived {
                            peer: peer_id,
                            stream,
                        }
                    ).await;
                }
                // 处理入站张量流 → 读 handshake → rendezvous 匹配
                Some((peer_id, mut stream)) = incoming_tensor_streams.next() => {
                    info!("收到入站张量流 from {}", peer_id);
                    match super::tensor_stream::protocol::Read_Tensor_Stream_Handshake(&mut stream).await {
                        Ok(inference_id) => {
                            info!("张量流 handshake: inference_id={}", inference_id);
                            self.rendezvous.insert_inbound(inference_id, stream);
                        }
                        Err(e) => {
                            warn!("张量流 handshake 读取失败 from {}: {}", peer_id, e);
                        }
                    }
                }
                // 处理入站带宽测试流 — 接收方：仅计数，回传结果
                Some((peer_id, mut stream)) = incoming_bandwidth_streams.next() => {
                    info!("收到入站带宽测试流 from {}", peer_id);
                    match Receive_And_Count(&mut stream).await {
                        Ok(total_bytes) => {
                            info!("带宽测试接收完成: from={}, total={} bytes", peer_id, total_bytes);
                            if let Err(e) = Write_Bandwidth_Result(&mut stream, total_bytes).await {
                                warn!("带宽测试结果回传失败 from {}: {}", peer_id, e);
                            }
                        }
                        Err(e) => {
                            warn!("带宽测试入站失败 from {}: {}", peer_id, e);
                        }
                    }
                }
                // 处理入站 Session 流 → 读 handshake → 转发给 Orchestrator
                Some((peer_id, mut stream)) = incoming_session_streams.next() => {
                    info!("收到入站 Session 流 from {}", peer_id);
                    match super::session_stream::protocol::Read_Session_Handshake(&mut stream).await {
                        Ok(session_id) => {
                            info!("Session 流 handshake: session_id={}", session_id);
                            let _ = self.orchestrator_event_tx.send(
                                Network_Inbound_Event::SessionStreamArrived {
                                    peer: peer_id,
                                    session_id,
                                    stream,
                                }
                            ).await;
                        }
                        Err(e) => {
                            warn!("Session 流 handshake 读取失败 from {}: {}", peer_id, e);
                        }
                    }
                }
            }
        }

        info!("网络事件循环结束");
        Ok(())
    }

    /// 获取本地节点ID
    pub fn Get_Local_Peer_Id(&self) -> PeerId {
        self.local_peer_id
    }
}
