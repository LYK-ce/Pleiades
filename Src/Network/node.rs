//Presented by KeJi
//Date ： 2026-04-03

//! 网络节点核心模块
//! 负责Swarm管理、连接管理、事件处理
//!
//! 架构设计：
//! - Node: 内部网络节点，运行事件循环
//! - NodeHandle: 对外暴露的API句柄（定义在 node_handle.rs）
//! - NodeCommand: 外部命令枚举（定义在 node_handle.rs）
//!
//! 事件分流：
//! - Response → 通过 oneshot 路由回 Send_Bytes 调用方
//! - 入站 Request → 通过 inbound_tx 转发给 Control 层
//! - 连接/发现事件 → 通过 event_sender 上报
//! - 文件传输事件 → 通过 event_sender 上报
//!
//! 注意：
//! 我们当前暂时先不考虑广域网的环境，只专注于当前的局域网环境。
//! 广域网放到未来支持。

use libp2p::{
    identity::Keypair,
    kad::{self, store::MemoryStore, Mode},
    mdns,
    noise,
    request_response::{self, OutboundRequestId, ProtocolSupport, ResponseChannel},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use libp2p_stream as stream;
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};
use futures::StreamExt;

use super::data_protocol::{
    DataRequest, DataResponse, PleiadesCodec, DATA_PROTOCOL,
};
use super::stream_protocol::{
    FILE_STREAM_PROTOCOL, Send_File_Stream, Receive_File_Stream,
};
use super::node_handle::{NodeCommand, NodeHandle, InboundRequest};

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
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            lan_enabled: true,
            wan_enabled: false,
            transport_protocol: "TCP".to_string(),
            listen_port: 0,
            bootstrap_peers: Vec::new(),
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
}

/// 节点信息
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// 节点ID
    pub peer_id: PeerId,
    /// 地址列表
    pub addresses: Vec<Multiaddr>,
    /// 最后一次ping延迟(毫秒)
    pub latency_ms: Option<u64>,
    /// 连接时间
    pub connected_at: Instant,
}

/// 网络事件（发送给上层）
///
/// 注意：DataReceived 不再通过此通道发送，改走 inbound_tx。
/// Response 在内部通过 oneshot 路由回 Send_Bytes 调用方。
#[derive(Debug)]
pub enum NetworkEvent {
    /// 发现新节点
    PeerDiscovered(PeerId),
    /// 节点离开
    PeerLeft(PeerId),
    /// 连接建立
    ConnectionEstablished(PeerId),
    /// 连接断开
    ConnectionClosed(PeerId),
    /// 收到流式文件传输
    FileStreamReceived {
        peer: PeerId,
        file_path: PathBuf,
    },
    /// 流式文件发送失败
    FileStreamError {
        peer: PeerId,
        error: String,
    },
    /// DHT记录查询结果
    RecordFound { key: Vec<u8>, value: Vec<u8> },
    /// DHT记录未找到
    RecordNotFound { key: Vec<u8> },
}

/// 网络节点（内部实现）
pub struct Node {
    /// Swarm实例
    swarm: Swarm<PleiadesNetworkBehaviour>,
    /// 本地节点ID
    local_peer_id: PeerId,
    /// 已连接的节点信息
    connected_peers: HashMap<PeerId, PeerInfo>,
    /// 事件发送器（发送给上层，用于连接/文件/DHT等事件）
    event_sender: mpsc::Sender<NetworkEvent>,
    /// 命令接收器（接收外部命令）
    cmd_rx: mpsc::Receiver<NodeCommand>,
    /// 流式传输控制句柄
    stream_control: stream::Control,
    /// 配置
    config: NetworkConfig,
    /// 文件保存目录
    save_dir: PathBuf,

    // ===== 新增：Response 路由与入站请求管理 =====

    /// 出站 Response 路由：OutboundRequestId → oneshot Sender
    /// 当 Send_Bytes 发出请求后，Response 到达时通过此映射回传
    pending_responses: HashMap<OutboundRequestId, oneshot::Sender<Result<DataResponse, String>>>,
    /// 入站 ResponseChannel 存储：request_id → ResponseChannel
    /// 当 Control 层调用 Send_Reply(request_id) 时，从此映射取出 channel
    pending_replies: HashMap<u64, ResponseChannel<DataResponse>>,
    /// 入站请求发送器（转发给 Control 层）
    inbound_tx: mpsc::Sender<InboundRequest>,
    /// 入站请求 ID 自增计数器
    next_inbound_id: u64,
}

impl Node {
    /// 初始化网络节点
    ///
    /// # Arguments
    /// * `config` - 网络配置
    /// * `event_sender` - 事件发送通道（连接/文件/DHT 事件）
    ///
    /// # Returns
    /// (Node实例, NodeHandle句柄, inbound_rx 入站请求接收端)
    pub async fn Init(
        config: NetworkConfig,
        event_sender: mpsc::Sender<NetworkEvent>,
    ) -> Result<(Self, NodeHandle, mpsc::Receiver<InboundRequest>), Box<dyn Error>> {
        info!("初始化网络节点...");

        // 1. 生成节点身份（临时生成）
        let keypair = Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(keypair.public());
        info!("本地节点ID: {}", local_peer_id);

        // 2. 创建命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<NodeCommand>(100);

        // 3. 创建入站请求通道
        let (inbound_tx, inbound_rx) = mpsc::channel::<InboundRequest>(100);

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

                Ok(PleiadesNetworkBehaviour {
                    mdns,
                    kademlia,
                    request_response,
                    stream,
                })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(86400)))
            .build();

        // 5. 获取流式传输控制句柄
        let stream_control = node_swarm.behaviour().stream.new_control();

        // 6. 创建文件保存目录
        let save_dir = PathBuf::from("Pleiades_Workspace");

        // 7. 创建Node实例
        let mut node = Self {
            swarm: node_swarm,
            local_peer_id,
            connected_peers: HashMap::new(),
            event_sender,
            cmd_rx,
            stream_control,
            config,
            save_dir,
            // 新增字段
            pending_responses: HashMap::new(),
            pending_replies: HashMap::new(),
            inbound_tx,
            next_inbound_id: 1,
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

        // 8. 创建NodeHandle
        let handle = NodeHandle::New(cmd_tx, local_peer_id);

        info!("网络节点初始化完成");
        Ok((node, handle, inbound_rx))
    }

    /// 启动网络服务
    ///
    /// 使用tokio::select!同时监听Swarm事件、外部命令和流式传输入站流
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
        let mut incoming_streams = self
            .stream_control
            .accept(StreamProtocol::new(FILE_STREAM_PROTOCOL))
            .expect("流式传输协议注册失败");

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
                // 处理入站流式传输
                Some((peer_id, mut stream)) = incoming_streams.next() => {
                    info!("收到流式传输连接 from {}", peer_id);
                    let event_sender = self.event_sender.clone();
                    let save_dir = self.save_dir.clone();
                    // 在独立任务中处理流式接收，避免阻塞事件循环
                    tokio::spawn(async move {
                        match Receive_File_Stream(&mut stream, &save_dir).await {
                            Ok(file_path) => {
                                info!("流式文件接收完成: {} from {}", file_path.display(), peer_id);
                                let _ = event_sender
                                    .send(NetworkEvent::FileStreamReceived {
                                        peer: peer_id,
                                        file_path,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                error!("流式文件接收失败 from {}: {}", peer_id, e);
                                let _ = event_sender
                                    .send(NetworkEvent::FileStreamError {
                                        peer: peer_id,
                                        error: e.to_string(),
                                    })
                                    .await;
                            }
                        }
                    });
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
                let peer_info = PeerInfo {
                    peer_id,
                    addresses: vec![endpoint.get_remote_address().clone()],
                    latency_ms: None,
                    connected_at: Instant::now(),
                };
                self.connected_peers.insert(peer_id, peer_info);
                let _ = self
                    .event_sender
                    .send(NetworkEvent::ConnectionEstablished(peer_id))
                    .await;
            }

            // 连接断开
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                info!("连接断开: {} 原因: {:?}", peer_id, cause);
                self.connected_peers.remove(&peer_id);
                let _ = self
                    .event_sender
                    .send(NetworkEvent::ConnectionClosed(peer_id))
                    .await;
            }

            // 新监听地址
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("监听地址: {}/p2p/{}", address, self.local_peer_id);
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
                let request = DataRequest { data_type, payload };
                let outbound_id = self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, request);

                // 如果调用方需要等待 Response，存入 pending_responses
                if let Some(tx) = response_tx {
                    self.pending_responses.insert(outbound_id, tx);
                }
            }
            NodeCommand::SendFileStream { peer, file_path, completion_tx } => {
                info!("流式发送文件到 {} | path={}", peer, file_path.display());
                let mut control = self.stream_control.clone();
                let event_sender = self.event_sender.clone();
                let protocol = StreamProtocol::new(FILE_STREAM_PROTOCOL);
                // 在独立任务中处理流式发送，避免阻塞事件循环
                tokio::spawn(async move {
                    let result = match control.open_stream(peer, protocol).await {
                        Ok(mut stream) => {
                            match Send_File_Stream(&mut stream, &file_path).await {
                                Ok(()) => {
                                    info!("流式文件发送完成: {} -> {}", file_path.display(), peer);
                                    Ok(())
                                }
                                Err(e) => {
                                    error!("流式文件发送失败: {} -> {}: {}", file_path.display(), peer, e);
                                    let _ = event_sender
                                        .send(NetworkEvent::FileStreamError {
                                            peer,
                                            error: e.to_string(),
                                        })
                                        .await;
                                    Err(e.to_string())
                                }
                            }
                        }
                        Err(e) => {
                            error!("打开流式传输连接失败 -> {}: {}", peer, e);
                            let _ = event_sender
                                .send(NetworkEvent::FileStreamError {
                                    peer,
                                    error: e.to_string(),
                                })
                                .await;
                            Err(e.to_string())
                        }
                    };
                    // 通知调用方传输完成
                    if let Some(tx) = completion_tx {
                        let _ = tx.send(result);
                    }
                });
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
                self.connected_peers.remove(&peer);
            }
            NodeCommand::SendResponse { channel, data_type, payload } => {
                let response = DataResponse { data_type, payload };
                if let Err(e) = self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, response) {
                    error!("发送响应失败: {:?}", e);
                }
            }
            NodeCommand::SendReply { request_id, data_type, payload } => {
                // 从 pending_replies 取出 ResponseChannel
                if let Some(channel) = self.pending_replies.remove(&request_id) {
                    let response = DataResponse { data_type, payload };
                    if let Err(e) = self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, response) {
                        error!("发送回复失败 (request_id={}): {:?}", request_id, e);
                    } else {
                        debug!("回复已发送 (request_id={})", request_id);
                    }
                } else {
                    warn!("未找到入站请求 request_id={}", request_id);
                }
            }
            NodeCommand::GetPeers { reply } => {
                let peers = self.Get_Peers();
                debug!("查询已连接节点列表: {} 个", peers.len());
                let _ = reply.send(peers);
            }
            NodeCommand::GetPeerInfo { peer, reply } => {
                let info = self.Get_Peer_Info(&peer).cloned();
                debug!("查询节点信息: {} -> {:?}", peer, info.is_some());
                let _ = reply.send(info);
            }
            NodeCommand::Stop => {
                info!("收到停止命令，准备退出");
                // 关闭所有连接
                for peer_id in self.connected_peers.keys().cloned().collect::<Vec<_>>() {
                    let _ = self.swarm.disconnect_peer_id(peer_id);
                }
                self.connected_peers.clear();
                // 清理 pending_responses（通知等待方）
                for (_, tx) in self.pending_responses.drain() {
                    let _ = tx.send(Err("Node stopped".to_string()));
                }
                return false;
            }
        }
        true
    }

    /// 处理mDNS事件
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
                        let _ = self
                            .event_sender
                            .send(NetworkEvent::PeerDiscovered(peer_id))
                            .await;
                    }
                }
            }
            mdns::Event::Expired(peers) => {
                for (peer_id, _addr) in peers {
                    info!("节点离开: {}", peer_id);
                    let _ = self
                        .event_sender
                        .send(NetworkEvent::PeerLeft(peer_id))
                        .await;
                }
            }
        }
    }

    /// 处理Kademlia事件
    async fn Handle_Kademlia_Event(&mut self, event: kad::Event) {
        match event {
            kad::Event::OutboundQueryProgressed { result, .. } => match result {
                kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FoundRecord(peer_record))) => {
                    info!("DHT记录查询成功: {:?}", peer_record.record.key);
                    let _ = self
                        .event_sender
                        .send(NetworkEvent::RecordFound {
                            key: peer_record.record.key.to_vec(),
                            value: peer_record.record.value,
                        })
                        .await;
                }
                kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FinishedWithNoAdditionalRecord { .. })) => {
                    debug!("DHT记录查询完成，无更多记录");
                }
                kad::QueryResult::GetRecord(Err(e)) => {
                    warn!("DHT记录查询失败: {:?}", e);
                    let key = match e {
                        kad::GetRecordError::NotFound { key, .. } => key.to_vec(),
                        kad::GetRecordError::QuorumFailed { key, .. } => key.to_vec(),
                        kad::GetRecordError::Timeout { key, .. } => key.to_vec(),
                    };
                    let _ = self
                        .event_sender
                        .send(NetworkEvent::RecordNotFound { key })
                        .await;
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
    /// - Request（入站）：存储 ResponseChannel，通过 inbound_tx 转发给 Control 层
    /// - Response（出站回复）：通过 oneshot 路由回 Send_Bytes 调用方
    /// - OutboundFailure：通知等待方发送失败
    async fn Handle_Request_Response_Event(
        &mut self,
        event: request_response::Event<DataRequest, DataResponse>,
    ) {
        match event {
            request_response::Event::Message { peer, message, .. } => match message {
                // ===== 入站请求：转发给 Control 层 =====
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    info!("收到数据请求 from {} | type={:?} | size={} bytes",
                          peer, request.data_type, request.payload.len());

                    // 分配 request_id，存储 ResponseChannel
                    let request_id = self.next_inbound_id;
                    self.next_inbound_id += 1;
                    self.pending_replies.insert(request_id, channel);

                    // 转发给 Control 层（不含 libp2p 内部类型）
                    let inbound = InboundRequest {
                        request_id,
                        peer,
                        data_type: request.data_type,
                        payload: request.payload,
                    };
                    if let Err(e) = self.inbound_tx.send(inbound).await {
                        error!("转发入站请求失败: {}", e);
                        // 如果发送失败，清理 pending_replies
                        self.pending_replies.remove(&request_id);
                    }
                }
                // ===== 出站响应：路由回 Send_Bytes 调用方 =====
                request_response::Message::Response { request_id, response, .. } => {
                    debug!("收到响应 from {} | type={:?} | size={} bytes",
                           peer, response.data_type, response.payload.len());

                    // 用 OutboundRequestId 匹配 pending_responses
                    if let Some(tx) = self.pending_responses.remove(&request_id) {
                        let _ = tx.send(Ok(response));
                    } else {
                        debug!("收到未追踪的响应 (request_id={:?})", request_id);
                    }
                }
            },
            request_response::Event::OutboundFailure {
                peer,
                request_id,
                error,
                ..
            } => {
                error!("发送失败 to {}: {:?}", peer, error);
                // 通知等待方发送失败
                if let Some(tx) = self.pending_responses.remove(&request_id) {
                    let _ = tx.send(Err(format!("Outbound failure: {:?}", error)));
                }
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

    /// 获取当前已连接的所有节点列表
    pub fn Get_Peers(&self) -> Vec<PeerId> {
        self.connected_peers.keys().cloned().collect()
    }

    /// 获取特定节点的详细信息
    pub fn Get_Peer_Info(&self, peer_id: &PeerId) -> Option<&PeerInfo> {
        self.connected_peers.get(peer_id)
    }

    /// 获取本地节点ID
    pub fn Get_Local_Peer_Id(&self) -> PeerId {
        self.local_peer_id
    }
}
