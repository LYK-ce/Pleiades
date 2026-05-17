//Presented by KeJi
//Date ： 2026-05-16

//! 命令处理器
//!
//! Network_Service 的命令通道事件处理方法。

use libp2p::{kad, PeerId};
use tracing::{debug, error, info, warn};

use super::network_service::Network_Service;
use super::request_response::Network_Data;
use super::node_handle::NodeCommand;

impl Network_Service {
    /// 处理外部命令
    /// 返回false表示应该退出循环
    pub(super) async fn Handle_Command(&mut self, cmd: NodeCommand) -> bool {
        match cmd {
            NodeCommand::SendData { peer, data_type, payload, response_tx } => {
                info!("发送数据到 {} | type={:?} | size={} bytes", peer, data_type, payload.len());
                let request = Network_Data { data_type, payload };
                let outbound_id = self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, request);

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
                if let Err(e) = self.peer_handle.Remove_Peer(&peer).await {
                    warn!("从 PeerManager 移除节点失败: {}", e);
                }
            }
            NodeCommand::SendResponse { request_id, data_type, payload } => {
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
            NodeCommand::Stop => {
                info!("收到停止命令，准备退出");
                match self.peer_handle.Get_Peers().await {
                    Ok(peer_infos) => {
                        for peer_info in peer_infos {
                            let _ = self.swarm.disconnect_peer_id(peer_info.peer_id);
                        }
                    }
                    Err(e) => {
                        warn!("获取节点列表失败: {}", e);
                    }
                }
                if let Err(e) = self.peer_handle.Clear().await {
                    warn!("清空 PeerManager 失败: {}", e);
                }
                self.inbound_manager.Clear_All();
                self.outbound_manager.Clear_All();
                return false;
            }
        }
        true
    }
}
