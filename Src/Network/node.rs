//Presented by KeJi
//Date: 2026-03-12

//! 网络节点核心模块
//! 负责Swarm管理、连接管理、事件处理以及提供对外接口
//!
//! 架构设计：
//! - Node: 内部网络节点，运行事件循环
//! - NodeHandle: 对外暴露的API句柄，可Clone可Send
//! - NodeCommand: 外部命令枚举，通过通道发送给Node执行

use libp2p::{
    identity::Keypair,
    kad::{self, store::MemoryStore, Mode},
    mdns,
    noise,
    request_response::{self, ProtocolSupport, ResponseChannel},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use futures::StreamExt;

use super::protocol::{
    CommandRequest, FileRequest, PleiadesCodec, PleiadesRequest,
    PleiadesResponse, TensorRequest, COMMAND_PROTOCOL,
};

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
    /// 收到命令请求
    CommandReceived {
        peer: PeerId,
        request: CommandRequest,
        channel: ResponseChannel<PleiadesResponse>,
    },
    /// 收到文件请求
    FileReceived {
        peer: PeerId,
        request: FileRequest,
        channel: ResponseChannel<PleiadesResponse>,
    },
    /// 收到张量请求
    TensorReceived {
        peer: PeerId,
        request: TensorRequest,
        channel: ResponseChannel<PleiadesResponse>,
    },
    /// DHT记录查询结果
    RecordFound { key: Vec<u8>, value: Vec<u8> },
    /// DHT记录未找到
    RecordNotFound { key: Vec<u8> },
}

/// 节点命令（外部通过NodeHandle发送给Node执行）
#[derive(Debug)]
pub enum NodeCommand {
    /// 发送命令
    SendCommand { peer: PeerId, cmd: CommandRequest },
    /// 发送文件
    SendFile { peer: PeerId, path: PathBuf },
    /// 发送张量
    SendTensor { peer: PeerId, tensor: TensorRequest },
    /// DHT写入
    PutRecord { key: Vec<u8>, value: Vec<u8> },
    /// DHT读取
    GetRecord { key: Vec<u8> },
    /// 主动连接
    Dial { addr: Multiaddr },
    /// 断开连接
    Disconnect { peer: PeerId },
    /// 发送响应
    SendResponse { 
        channel: ResponseChannel<PleiadesResponse>, 
        response: PleiadesResponse 
    },
    /// 停止节点
    Stop,
}

/// 节点句柄（对外API，可Clone可Send）
#[derive(Clone)]
pub struct NodeHandle {
    /// 命令发送器
    cmd_tx: mpsc::Sender<NodeCommand>,
    /// 本地节点ID
    local_peer_id: PeerId,
}

impl NodeHandle {
    /// 获取本地节点ID
    pub fn Get_Local_Peer_Id(&self) -> PeerId {
        self.local_peer_id
    }

    /// 发送命令给指定节点
    pub async fn Send_Command(&self, peer: &PeerId, cmd: CommandRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::SendCommand {
            peer: *peer,
            cmd,
        }).await?;
        Ok(())
    }

    /// 发送文件给指定节点
    pub async fn Send_File(&self, peer: &PeerId, path: &Path) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::SendFile {
            peer: *peer,
            path: path.to_path_buf(),
        }).await?;
        Ok(())
    }

    /// 发送张量给指定节点
    pub async fn Send_Tensor(&self, peer: &PeerId, tensor: TensorRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::SendTensor {
            peer: *peer,
            tensor,
        }).await?;
        Ok(())
    }

    /// DHT写入
    pub async fn Put_Record(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::PutRecord { key, value }).await?;
        Ok(())
    }

    /// DHT读取
    pub async fn Get_Record(&self, key: Vec<u8>) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::GetRecord { key }).await?;
        Ok(())
    }

    /// 主动连接到指定地址
    pub async fn Dial(&self, addr: Multiaddr) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::Dial { addr }).await?;
        Ok(())
    }

    /// 断开与指定节点的连接
    pub async fn Disconnect(&self, peer: &PeerId) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::Disconnect { peer: *peer }).await?;
        Ok(())
    }

    /// 发送响应
    pub async fn Send_Response(
        &self,
        channel: ResponseChannel<PleiadesResponse>,
        response: PleiadesResponse,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::SendResponse { channel, response }).await?;
        Ok(())
    }

    /// 停止节点
    pub async fn Stop(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::Stop).await?;
        Ok(())
    }
}

/// 网络节点（内部实现）
pub struct Node {
    /// Swarm实例
    swarm: Swarm<PleiadesNetworkBehaviour>,
    /// 本地节点ID
    local_peer_id: PeerId,
    /// 已连接的节点信息
    connected_peers: HashMap<PeerId, PeerInfo>,
    /// 事件发送器（发送给上层）
    event_sender: mpsc::Sender<NetworkEvent>,
    /// 命令接收器（接收外部命令）
    cmd_rx: mpsc::Receiver<NodeCommand>,
    /// 配置
    config: NetworkConfig,
}

impl Node {
    /// 初始化网络节点
    ///
    /// # Arguments
    /// * `config` - 网络配置
    /// * `event_sender` - 事件发送通道
    ///
    /// # Returns
    /// (Node实例, NodeHandle句柄)
    pub async fn Init(
        config: NetworkConfig,
        event_sender: mpsc::Sender<NetworkEvent>,
    ) -> Result<(Self, NodeHandle), Box<dyn Error>> {
        info!("初始化网络节点...");

        // 1. 生成节点身份（临时生成）
        let keypair = Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(keypair.public());
        info!("本地节点ID: {}", local_peer_id);

        // 2. 创建命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel::<NodeCommand>(100);

        // 3. 创建Swarm
        let swarm = SwarmBuilder::with_existing_identity(keypair)
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

                // 创建请求响应行为
                let protocols = [(
                    StreamProtocol::new(COMMAND_PROTOCOL),
                    ProtocolSupport::Full,
                )];
                let cfg = request_response::Config::default()
                    .with_request_timeout(Duration::from_secs(30));
                let request_response =
                    request_response::Behaviour::<PleiadesCodec>::new(protocols, cfg);

                Ok(PleiadesNetworkBehaviour {
                    mdns,
                    kademlia,
                    request_response,
                })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        // 4. 创建Node实例
        let mut node = Self {
            swarm,
            local_peer_id,
            connected_peers: HashMap::new(),
            event_sender,
            cmd_rx,
            config,
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

        // 5. 创建NodeHandle
        let handle = NodeHandle {
            cmd_tx,
            local_peer_id,
        };

        info!("网络节点初始化完成");
        Ok((node, handle))
    }

    /// 启动网络服务
    ///
    /// 使用tokio::select!同时监听Swarm事件和外部命令
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

        // 3. 进入事件循环（使用select!同时监听网络事件和命令）
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
            NodeCommand::SendCommand { peer, cmd } => {
                info!("发送命令到 {}: {:?}", peer, cmd);
                let request = PleiadesRequest::Command(cmd);
                self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, request);
            }
            NodeCommand::SendFile { peer, path } => {
                // 读取文件内容
                match std::fs::read(&path) {
                    Ok(data) => {
                        // 获取文件名
                        let filename = path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        
                        info!("发送文件 {} ({} bytes) to {}", filename, data.len(), peer);
                        
                        // 使用SendFile变体发送完整文件
                        let request = PleiadesRequest::File(FileRequest::SendFile {
                            filename,
                            data,
                        });
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&peer, request);
                    }
                    Err(e) => {
                        error!("读取文件失败 {}: {:?}", path.display(), e);
                    }
                }
            }
            NodeCommand::SendTensor { peer, tensor } => {
                info!("发送张量 {} to {}", tensor.request_id, peer);
                let request = PleiadesRequest::Tensor(tensor);
                self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, request);
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
            NodeCommand::SendResponse { channel, response } => {
                if let Err(e) = self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, response) {
                    error!("发送响应失败: {:?}", e);
                }
            }
            NodeCommand::Stop => {
                info!("收到停止命令，准备退出");
                // 关闭所有连接
                for peer_id in self.connected_peers.keys().cloned().collect::<Vec<_>>() {
                    let _ = self.swarm.disconnect_peer_id(peer_id);
                }
                self.connected_peers.clear();
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
    async fn Handle_Request_Response_Event(
        &mut self,
        event: request_response::Event<PleiadesRequest, PleiadesResponse>,
    ) {
        match event {
            request_response::Event::Message { peer, message } => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    info!("收到请求 from {}", peer);
                    match request {
                        PleiadesRequest::Command(cmd) => {
                            let _ = self
                                .event_sender
                                .send(NetworkEvent::CommandReceived {
                                    peer,
                                    request: cmd,
                                    channel,
                                })
                                .await;
                        }
                        PleiadesRequest::File(file_req) => {
                            let _ = self
                                .event_sender
                                .send(NetworkEvent::FileReceived {
                                    peer,
                                    request: file_req,
                                    channel,
                                })
                                .await;
                        }
                        PleiadesRequest::Tensor(tensor_req) => {
                            let _ = self
                                .event_sender
                                .send(NetworkEvent::TensorReceived {
                                    peer,
                                    request: tensor_req,
                                    channel,
                                })
                                .await;
                        }
                    }
                }
                request_response::Message::Response { response, .. } => {
                    debug!("收到响应 from {}: {:?}", peer, response);
                }
            },
            request_response::Event::OutboundFailure {
                peer,
                error,
                ..
            } => {
                error!("发送失败 to {}: {:?}", peer, error);
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
