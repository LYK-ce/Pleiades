//Presented by KeJi
//Date ： 2026-04-24

//! 网络服务核心模块
//! 负责Swarm管理、连接管理、事件处理
//!
//! 架构设计：
//! - Network_Service: 网络服务实例，运行事件循环
//! - NodeHandle: 对外暴露的API句柄（定义在 node_handle.rs）
//! - NodeCommand: 外部命令枚举（定义在 node_handle.rs）
//! - Inbound_Manager: 入站请求与响应路由管理器（定义在 inbound_manager.rs）
//! - Outbound_Manager: 出站响应路由管理器（定义在 outbound_manager.rs）
//!
//! 事件分流：
//! - Response → 通过 oneshot 路由回 Send_Data 调用方
//! - 入站 Request → 通过 inbound_tx 转发给 Orchestrator
//! - 入站文件流 → 通过 orchestrator_event_tx 转发给 Orchestrator
//! - 入站张量流 → 通过 orchestrator_event_tx 转发给 Orchestrator
//! - 连接/发现/Ping 事件 → Network 内部处理（PeerManager）
//!
//! 注意：
//! 我们当前暂时先不考虑广域网的环境，只专注于当前的局域网环境。
//! 广域网放到未来支持。
#[allow(nonstandard_style)]
use libp2p::{
    identity::Keypair,
    kad::{self, store::MemoryStore, Mode},
    mdns,
    noise,
    ping,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use libp2p_stream as stream;
use std::error::Error;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use futures::StreamExt;

use super::data_protocol::{
    Network_Data, PleiadesCodec, DATA_PROTOCOL,
};
use super::inbound_manager::Inbound_Manager;
use super::outbound_manager::Outbound_Manager;
use super::node_handle::{NodeCommand, NodeHandle, InboundRequest};
use super::capability::{Network_Inbound_Event, Network_Service_Capability};
use super::stream_protocol::FILE_STREAM_PROTOCOL;
use super::tensor_stream_protocol::TENSOR_STREAM_PROTOCOL;

// 导入 PeerManagement 模块
use crate::peer_management::{PeerHandle, PeerInfo, PeerStatus};

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
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            lan_enabled: true,
            wan_enabled: false,
            transport_protocol: "TCP".to_string(),
            listen_port: 0,
            bootstrap_peers: Vec::new(),
            cleanup_interval: 300,    // 默认300秒
            timeout_interval: 300,    // 默认300秒
            heartbeat_interval: 60,   // 默认60秒
            heartbeat_timeout: 10,    // 默认10秒
        }
    }
}

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
}

/// 网络服务（内部实现）
#[allow(nonstandard_style)]
pub struct Network_Service {
    /// Swarm实例
    swarm: Swarm<PleiadesNetworkBehaviour>,
    /// 本地节点ID
    local_peer_id: PeerId,
    /// peer manager handler 持有的peer manager handle，我们通过它来管理节点信息表
    peer_handle: PeerHandle,
    /// 命令接收器（接收外部命令）
    cmd_rx: mpsc::Receiver<NodeCommand>,
    /// 配置
    config: NetworkConfig,

    // ===== 组件化管理器 =====

    /// 入站请求管理器（负责入站请求分发、回复管理）
    inbound_manager: Inbound_Manager,
    /// 出站响应路由管理器（负责出站 Response 路由回调用方）
    outbound_manager: Outbound_Manager,

    // ===== Orchestrator 事件转发 =====

    /// 转发复杂入站事件（文件流、张量流）给 Orchestrator
    orchestrator_event_tx: mpsc::Sender<Network_Inbound_Event>,

    // ===== 流控制句柄（用于 accept 入站流） =====

    /// 文件流控制句柄（用于 Start() 中 accept 入站文件流）
    file_accept_control: stream::Control,
    /// 张量流控制句柄（用于 Start() 中 accept 入站张量流）
    tensor_accept_control: stream::Control,
}

impl Network_Service {
    /// 初始化网络服务
    ///
    /// # Arguments
    /// * `config` - 网络配置
    /// * `keypair` - 密钥对
    /// * `peer_handle` - PeerManager 句柄，用于管理节点信息
    ///
    /// # Returns
    /// (Network_Service实例, NodeHandle句柄, inbound_rx 入站请求接收端,
    ///  Network_Service_Capability, orchestrator_event_rx 入站事件接收端)
    pub async fn Init(
        config: NetworkConfig,
        keypair: Keypair,
        peer_handle: PeerHandle,
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
                tcp::Config::default(),
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
                    .with_request_timeout(Duration::from_secs(30));
                let request_response =
                    request_response::Behaviour::<PleiadesCodec>::new(protocols, cfg);

                // 创建流式传输行为
                let stream = stream::Behaviour::new();

                // 创建Ping心跳行为，使用配置的heartbeat_interval和heartbeat_timeout
                let ping_config = ping::Config::new()
                    .with_interval(Duration::from_secs(config.heartbeat_interval))
                    .with_timeout(Duration::from_secs(config.heartbeat_timeout));
                let ping_behaviour = ping::Behaviour::new(ping_config);

                Ok(PleiadesNetworkBehaviour {
                    mdns,
                    kademlia,
                    request_response,
                    stream,
                    ping: ping_behaviour,
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

        // 6. 创建入站请求管理器和出站响应路由管理器
        let inbound_manager = Inbound_Manager::New(inbound_tx);
        let outbound_manager = Outbound_Manager::New();

        // 7. 创建 NodeHandle
        let handle = NodeHandle::New(cmd_tx, local_peer_id);

        // 8. 创建 Network_Service_Capability（用于 Orchestrator）
        let capability = Network_Service_Capability::New(
            handle.clone(),
            file_open_control,
            tensor_open_control,
        );

        // 9. 创建 Network_Service 实例
        let mut node = Self {
            swarm: node_swarm,
            local_peer_id,
            peer_handle,
            cmd_rx,
            config,
            inbound_manager,
            outbound_manager,
            orchestrator_event_tx,
            file_accept_control,
            tensor_accept_control,
        };

        // 添加引导节点
        for addr_str in &node.config.bootstrap_peers {
            if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                info!("添加引导节点: {}", addr);
                node.swarm.behaviour_mut().kademlia.add_address(
                    &PeerId::random(), // TODO: 从地址中提取PeerId
                    addr,
                );
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

        // 2. 启动Kademlia引导
        if self.config.wan_enabled {
            if let Err(e) = self.swarm.behaviour_mut().kademlia.bootstrap() {
                warn!("Kademlia引导失败: {}", e);
            }
        }

        // 3. 注册流式传输协议，接受入站流
        let mut incoming_file_streams = self.file_accept_control
            .accept(StreamProtocol::new(FILE_STREAM_PROTOCOL))
            .expect("文件流协议注册失败");

        let mut incoming_tensor_streams = self.tensor_accept_control
            .accept(StreamProtocol::new(TENSOR_STREAM_PROTOCOL))
            .expect("张量流协议注册失败");

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
                // 处理外部命令
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
                // 处理入站张量流 → 转发给 Orchestrator
                Some((peer_id, stream)) = incoming_tensor_streams.next() => {
                    info!("收到入站张量流 from {}, 转发给 Orchestrator", peer_id);
                    let _ = self.orchestrator_event_tx.send(
                        Network_Inbound_Event::TensorStreamArrived {
                            peer: peer_id,
                            stream,
                        }
                    ).await;
                }
            }
        }

        info!("网络事件循环结束");
        Ok(())
    }

    /// 处理Swarm事件
    /// 返回false表示应该退出循环
    async fn Handle_Swarm_Event(&mut self, event: SwarmEvent<PleiadesNetworkBehaviourEvent>) -> bool {
        match event {
            // mDNS事件
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Mdns(event)) => {
                self.Handle_Mdns_Event(event).await;
            }

            // Kademlia事件
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Kademlia(event)) => {
                self.Handle_Kademlia_Event(event).await;
            }

            // 请求响应事件
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::RequestResponse(event)) => {
                self.Handle_Request_Response_Event(event).await;
            }

            // 连接建立
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                ..
            } => {
                info!("连接建立: {} via {:?}", peer_id, endpoint);
                let peer_info = PeerInfo::new(
                    peer_id,
                    vec![endpoint.get_remote_address().clone()]
                );
                // 使用 peer_handle 添加节点
                if let Err(e) = self.peer_handle.add_peer(peer_info).await {
                    warn!("添加节点到 PeerManager 失败: {}", e);
                }
            }

            // 连接断开
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                info!("连接断开: {} 原因: {:?}", peer_id, cause);
                // 使用 peer_handle 移除节点
                if let Err(e) = self.peer_handle.remove_peer(&peer_id).await {
                    warn!("从 PeerManager 移除节点失败: {}", e);
                }
            }

            // 新监听地址
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("监听地址: {}/p2p/{}", address, self.local_peer_id);
            }

            // Ping心跳事件
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Ping(ping_event)) => {
                self.Handle_Ping_Event(ping_event).await;
            }

            // 其他事件
            event => {
                debug!("其他事件: {:?}", event);
            }
        }
        true
    }

    /// 处理外部命令
    /// 返回false表示应该退出循环
    async fn Handle_Command(&mut self, cmd: NodeCommand) -> bool {
        match cmd {
            NodeCommand::SendData { peer, data_type, payload, response_tx } => {
                info!("发送数据到 {} | type={:?} | size={} bytes", peer, data_type, payload.len());
                let request = Network_Data { data_type, payload };
                let outbound_id = self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, request);

                // 如果调用方需要等待 Response，注册到 outbound_manager
                if let Some(tx) = response_tx {
                    self.outbound_manager.Register_Outbound(outbound_id, tx);
                }
            }
            NodeCommand::PutRecord { key, value } => {
                let record_key = kad::RecordKey::new(&key);
                let record = kad::Record {
                    key: record_key,
                    value,
                    publisher: Some(self.local_peer_id),
                    expires: None,
                };
                if let Err(e) = self.swarm
                    .behaviour_mut()
                    .kademlia
                    .put_record(record, kad::Quorum::One) {
                    error!("DHT写入失败: {:?}", e);
                } else {
                    info!("DHT写入: {:?}", key);
                }
            }
            NodeCommand::GetRecord { key } => {
                let record_key = kad::RecordKey::new(&key);
                self.swarm.behaviour_mut().kademlia.get_record(record_key);
                info!("DHT查询: {:?}", key);
            }
            NodeCommand::Dial { addr } => {
                info!("尝试连接: {}", addr);
                if let Err(e) = self.swarm.dial(addr) {
                    error!("连接失败: {:?}", e);
                }
            }
            NodeCommand::Disconnect { peer } => {
                info!("断开连接: {}", peer);
                let _ = self.swarm.disconnect_peer_id(peer);
                // 使用 peer_handle 移除节点
                if let Err(e) = self.peer_handle.remove_peer(&peer).await {
                    warn!("从 PeerManager 移除节点失败: {}", e);
                }
            }
            NodeCommand::SendResponse { request_id, data_type, payload } => {
                // 从 inbound_manager 取出 ResponseChannel
                if let Some(channel) = self.inbound_manager.Take_Reply_Channel(request_id) {
                    let response = Network_Data { data_type, payload };
                    if let Err(e) = self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, response) {
                        error!("发送回复失败 (request_id={}): {:?}", request_id, e);
                    } else {
                        debug!("回复已发送 (request_id={})", request_id);
                    }
                }
            }
            NodeCommand::UpdateInfo { reply } => {
                info!("开始批量带宽测试");
                // 获取所有节点
                let peers = match self.peer_handle.list_peers().await {
                    Ok(peers) => peers,
                    Err(e) => {
                        let _ = reply.send(Err(format!("获取节点列表失败: {}", e).into()));
                        return true;
                    }
                };
                
                let total_count = peers.len();
                let mut success_count = 0;
                
                // 串行逐个测试，避免并发干扰
                for (index, peer_info) in peers.iter().enumerate() {
                    let peer_id = peer_info.peer_id;
                    info!("测试节点 {}/{}: {}", index + 1, total_count, peer_id);
                    
                    match self.Test_Bandwidth(&peer_id).await {
                        Ok(bandwidth) => {
                            info!("节点 {} 带宽测试成功: {} Mbps", peer_id, bandwidth);
                            success_count += 1;
                        }
                        Err(e) => {
                            info!("节点 {} 带宽测试失败: {}", peer_id, e);
                        }
                    }
                }
                
                info!("批量带宽测试完成: 成功{}/{}", success_count, total_count);
                let _ = reply.send(Ok((success_count, total_count)));
            }
            NodeCommand::Stop => {
                info!("收到停止命令，准备退出");
                // 关闭所有连接
                match self.peer_handle.list_peers().await {
                    Ok(peer_infos) => {
                        for peer_info in peer_infos {
                            let _ = self.swarm.disconnect_peer_id(peer_info.peer_id);
                        }
                    }
                    Err(e) => {
                        warn!("获取节点列表失败: {}", e);
                    }
                }
                // 清空 peer_handle
                if let Err(e) = self.peer_handle.clear().await {
                    warn!("清空 PeerManager 失败: {}", e);
                }
                // 清理所有 pending 状态
                self.inbound_manager.Clear_All();
                self.outbound_manager.Clear_All();
                return false;
            }
        }
        true
    }

    /// 处理mDNS事件
    ///
    /// mDNS 发现/离开仅更新 Kademlia 路由表，
    /// 不再通过 event_sender 通知上层（PeerManager 已通过 peer_handle 独立跟踪）。
    async fn Handle_Mdns_Event(&mut self, event: mdns::Event) {
        match event {
            mdns::Event::Discovered(peers) => {
                for (peer_id, addr) in peers {
                    if peer_id != self.local_peer_id {
                        info!("mDNS发现节点: {} at {}", peer_id, addr);
                        // 添加到Kademlia
                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .add_address(&peer_id, addr);
                    }
                }
            }
            mdns::Event::Expired(peers) => {
                for (peer_id, _addr) in peers {
                    info!("节点离开: {}", peer_id);
                }
            }
        }
    }

    /// 处理Kademlia事件
    ///
    /// DHT 结果不再通过 event_sender 通知上层。
    /// 未来通过 orchestrator_event_tx 或 Capability oneshot 模式投递结果。
    async fn Handle_Kademlia_Event(&mut self, event: kad::Event) {
        match event {
            kad::Event::OutboundQueryProgressed { result, .. } => match result {
                kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FoundRecord(peer_record))) => {
                    info!("DHT记录查询成功: {:?}", peer_record.record.key);
                }
                kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FinishedWithNoAdditionalRecord { .. })) => {
                    debug!("DHT记录查询完成，无更多记录");
                }
                kad::QueryResult::GetRecord(Err(e)) => {
                    warn!("DHT记录查询失败: {:?}", e);
                }
                kad::QueryResult::PutRecord(Ok(_)) => {
                    info!("DHT记录写入成功");
                }
                kad::QueryResult::PutRecord(Err(e)) => {
                    error!("DHT记录写入失败: {:?}", e);
                }
                kad::QueryResult::Bootstrap(Ok(_)) => {
                    info!("Kademlia引导成功");
                }
                kad::QueryResult::Bootstrap(Err(e)) => {
                    warn!("Kademlia引导失败: {:?}", e);
                }
                _ => {}
            },
            kad::Event::RoutingUpdated { peer, .. } => {
                debug!("路由表更新: {}", peer);
            }
            _ => {}
        }
    }

    /// 处理请求响应事件
    ///
    /// - Request（入站）：存储 ResponseChannel，通过 inbound_tx 转发给 Orchestrator
    /// - Response（出站回复）：通过 oneshot 路由回 Send_Data 调用方
    /// - OutboundFailure：通知等待方发送失败
    async fn Handle_Request_Response_Event(
        &mut self,
        event: request_response::Event<Network_Data, Network_Data>,
    ) {
        match event {
            request_response::Event::Message { peer, message, .. } => match message {
                // ===== 入站请求：通过 inbound_manager 转发给 Orchestrator =====
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    info!("收到数据请求 from {} | type={:?} | size={} bytes",
                          peer, request.data_type, request.payload.len());

                    self.inbound_manager.Register_Inbound(peer, request, channel).await;
                }
                // ===== 出站响应：通过 outbound_manager 路由回 Send_Data 调用方 =====
                request_response::Message::Response { request_id, response, .. } => {
                    debug!("收到响应 from {} | type={:?} | size={} bytes",
                           peer, response.data_type, response.payload.len());

                    self.outbound_manager.Route_Response(request_id, response);
                }
            },
            request_response::Event::OutboundFailure {
                peer,
                request_id,
                error,
                ..
            } => {
                error!("发送失败 to {}: {:?}", peer, error);
                self.outbound_manager.Route_Failure(request_id, format!("Outbound failure: {:?}", error));
            }
            request_response::Event::InboundFailure {
                peer,
                error,
                ..
            } => {
                error!("接收失败 from {}: {:?}", peer, error);
            }
            request_response::Event::ResponseSent { peer, .. } => {
                debug!("响应已发送 to {}", peer);
            }
        }
    }

    /// 处理Ping心跳事件
    ///
    /// 心跳事件完全在Network层内部处理，不向上层发送事件。
    /// - 成功收到Pong: 通过peer_handle更新延迟信息
    /// - 超时: 通过peer_handle将节点状态设置为Disconnected
    /// - 不支持/其他错误: 仅记录日志
    async fn Handle_Ping_Event(&mut self, event: ping::Event) {
        let peer_id = event.peer;

        match event.result {
            // 成功收到 Pong，包含 RTT
            Ok(rtt) => {
                let latency_ms = rtt.as_millis() as u64;
                info!("Ping成功: {} | RTT: {}ms", peer_id, latency_ms);

                // 使用 peer_handle 更新心跳延迟信息
                if let Err(e) = self.peer_handle.update_heartbeat(&peer_id, Some(latency_ms)).await {
                    info!("更新节点心跳失败 ({}): {}", peer_id, e);
                }
            }
            // Ping 超时
            Err(ping::Failure::Timeout) => {
                info!("Ping超时: {}", peer_id);

                // 将节点状态设置为 Disconnected
                if let Err(e) = self.peer_handle.update_status(&peer_id, PeerStatus::Disconnected).await {
                    info!("更新节点状态失败 ({}): {}", peer_id, e);
                }
            }
            // 对方不支持 Ping 协议
            Err(ping::Failure::Unsupported) => {
                info!("节点不支持Ping协议: {}", peer_id);
            }
            // 其他错误
            Err(ping::Failure::Other { error }) => {
                info!("Ping错误 ({}): {}", peer_id, error);
            }
        }
    }

    /// 测试指定节点的带宽
    /// 发送1M、10M、50M数据包，取最大值作为带宽结果
    pub async fn Test_Bandwidth(&mut self, peer_id: &PeerId) -> Result<u64, Box<dyn Error + Send + Sync>> {
        let test_sizes = [1_000_000, 10_000_000, 50_000_000]; // 1M, 10M, 50M
        let mut max_bandwidth = 0u64;
        let mut has_success = false;
        
        for &size in &test_sizes {
            match self.test_single_bandwidth(peer_id, size).await {
                Ok(bandwidth) => {
                    info!("带宽测试成功: peer={}, size={}B, bandwidth={}Mbps",
                          peer_id, size, bandwidth);
                    max_bandwidth = max_bandwidth.max(bandwidth);
                    has_success = true;
                }
                Err(e) => {
                    info!("带宽测试失败: peer={}, size={}B, error={}",
                          peer_id, size, e);
                }
            }
        }
        
        if has_success {
            // 更新PeerManager中的带宽信息
            if let Err(e) = self.peer_handle.update_bandwidth(peer_id, Some(max_bandwidth)).await {
                info!("更新带宽信息失败: peer={}, error={}", peer_id, e);
            }
            Ok(max_bandwidth)
        } else {
            Err("所有带宽测试均失败".into())
        }
    }

    /// 测试单个数据包大小的带宽
    async fn test_single_bandwidth(&mut self, peer_id: &PeerId, size_bytes: u64) -> Result<u64, Box<dyn Error + Send + Sync>> {
        use tokio::sync::oneshot;
        use std::time::Instant;
        
        // 1. 准备payload：size_bytes的小端字节序表示
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&size_bytes.to_le_bytes());
        
        // 2. 创建oneshot channel用于接收响应
        let (response_tx, response_rx) = oneshot::channel();
        
        // 3. 发送请求
        let request = super::data_protocol::Network_Data {
            data_type: super::data_protocol::DataType::BandwidthTest,
            payload,
        };
        
        let start_time = Instant::now();
        let outbound_id = self.swarm
            .behaviour_mut()
            .request_response
            .send_request(peer_id, request);
        
        // 4. 注册到outbound_manager等待响应
        self.outbound_manager.Register_Outbound(outbound_id, response_tx);
        
        // 5. 等待响应，设置30秒超时
        let response_result = match tokio::time::timeout(Duration::from_secs(30), response_rx).await {
            Ok(Ok(response_result)) => response_result,
            Ok(Err(_)) => return Err("响应通道已关闭".into()),
            Err(_) => return Err("带宽测试超时".into()),
        };
        
        let end_time = Instant::now();
        
        // 6. 检查响应结果
        let response = match response_result {
            Ok(network_data) => network_data,
            Err(e) => return Err(format!("带宽测试失败: {}", e).into()),
        };
        
        // 7. 验证响应大小
        if response.payload.len() != size_bytes as usize {
            return Err(format!("响应大小不匹配: 期望{}B, 实际{}B",
                size_bytes, response.payload.len()).into());
        }
        
        // 8. 计算带宽 (Mbps)
        let duration_secs = end_time.duration_since(start_time).as_secs_f64();
        let bandwidth_mbps = (size_bytes as f64 * 8.0) / (duration_secs * 1_000_000.0);
        
        Ok(bandwidth_mbps as u64)
    }

    /// 获取本地节点ID
    pub fn Get_Local_Peer_Id(&self) -> PeerId {
        self.local_peer_id
    }
}
