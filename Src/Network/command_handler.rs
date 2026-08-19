//Presented by KeJi
//Created Date ： 2026-05-16
//Modified Date ： 2026-08-18

//! 命令处理器
//!
//! Network_Service 的命令通道事件处理方法。

use libp2p::gossipsub;
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
                crate::network::DHT::put_record(
                    &mut self.swarm.behaviour_mut().kademlia,
                    &key,
                    value,
                    self.local_peer_id,
                );
            }
            NodeCommand::GetRecord { key } => {
                crate::network::DHT::get_record(
                    &mut self.swarm.behaviour_mut().kademlia,
                    &key,
                );
            }
            NodeCommand::GetProviders { reply } => {
                let qid = crate::network::DHT::get_providers(
                    &mut self.swarm.behaviour_mut().kademlia,
                    &self.config.dht_namespace,
                );
                self.provider_queries.register(qid, reply);
                info!("DHT 查询 provider 列表: query_id={:?}", qid);
            }
            NodeCommand::GossipsubPublish { topic, payload } => {
                let topic_hash = gossipsub::TopicHash::from_raw(topic.clone());
                match self.swarm.behaviour_mut().gossipsub.publish(topic_hash, payload.clone()) {
                    Ok(msg_id) => debug!("gossipsub 发布成功: topic={} id={:?}", topic, msg_id),
                    Err(e) => {
                        // 无订阅者属正常情况（PublishError::NoPeersSubscribedToTopic）
                        debug!("gossipsub 发布失败 ({}): {:?}", topic, e);
                    }
                }
                // 快照更新：无论是否有订阅者都更新——快照是"最近状态"，
                // 新节点订阅 topic 时按 topic 精准重放即可
                self.snapshot_cache.Update(&topic, payload);
            }
            NodeCommand::Dial { addr } => {
                info!("尝试连接: {}", addr);
                if let Err(e) = self.swarm.dial(addr) {
                    error!("连接失败: {:?}", e);
                }
            }
            NodeCommand::DialPeer { peer } => {
                info!("按 PeerId 连接: {}", peer);
                if let Err(e) = self.swarm.dial(peer) {
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
                match self.peer_handle.Get_All_Peers().await {
                    Ok(peer_infos) => {
                        for peer_info in peer_infos {
                            if !peer_info.local {
                                let _ = self.swarm.disconnect_peer_id(peer_info.peer_id);
                            }
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
