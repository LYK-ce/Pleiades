//Presented by KeJi
//Created Date ： 2026-05-16
//Modified Date ： 2026-08-08

//! Swarm 事件处理器
//!
//! Network_Service 的 swarm 事件响应方法。

use libp2p::{gossipsub, identify, kad, mdns, ping, request_response, swarm::SwarmEvent};
use tracing::{debug, error, info, warn};

use super::network_service::{Network_Service, PleiadesNetworkBehaviourEvent};
use super::request_response::{DataType, Network_Data};
use crate::event_bus::Bus_Event;
use crate::peer_management::PeerInfo;

impl Network_Service {
    /// 处理Swarm事件
    /// 返回false表示应该退出循环
    pub(super) async fn Handle_Swarm_Event(
        &mut self,
        event: SwarmEvent<PleiadesNetworkBehaviourEvent>,
    ) -> bool {
        match event {
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Mdns(event)) => {
                self.Handle_Mdns_Event(event).await;
            }
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Kademlia(event)) => {
                self.Handle_Kademlia_Event(event).await;
            }
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::RequestResponse(event)) => {
                self.Handle_Request_Response_Event(event).await;
            }
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
                self.peer_handle.Upsert_Peer(peer_info).await.ok();


                self.event_bus.Publish(Bus_Event::State {
                    payload: serde_json::json!({
                        "type": "peer_connected",
                        "peer_id": peer_id.to_string(),
                    }).to_string(),
                });
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                info!("连接断开: {} 原因: {:?}", peer_id, cause);
                if let Err(e) = self.peer_handle.Remove_Peer(&peer_id).await {
                    warn!("从 PeerManager 移除节点失败: {}", e);
                }
                self.event_bus.Publish(Bus_Event::State {
                    payload: serde_json::json!({
                        "type": "peer_disconnected",
                        "peer_id": peer_id.to_string(),
                    }).to_string(),
                });
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("监听地址: {}/p2p/{}", address, self.local_peer_id);
            }
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Ping(ping_event)) => {
                self.Handle_Ping_Event(ping_event).await;
            }
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Identify(event)) => {
                self.Handle_Identify_Event(event).await;
            }
            SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Gossipsub(event)) => {
                self.Handle_Gossipsub_Event(event).await;
            }
            event => {
                debug!("其他事件: {:?}", event);
            }
        }
        true
    }

    /// 处理mDNS事件
    ///
    /// mDNS 发现/离开更新 Kademlia 路由表，
    /// 并通过 EventBus 发布节点发现/离开事件通知 TUI 等消费者。
    pub(super) async fn Handle_Mdns_Event(&mut self, event: mdns::Event) {
        match event {
            mdns::Event::Discovered(peers) => {
                for (peer_id, addr) in peers {
                    if peer_id != self.local_peer_id {
                        info!("mDNS发现节点: {} at {}", peer_id, addr);
                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .add_address(&peer_id, addr.clone());
                        // 主动 dial 以建立 TCP 连接，触发 ConnectionEstablished
                        // → PeerManager 注册 → Info 交换 (name + models)
                        if let Err(e) = self.swarm.dial(addr.clone()) {
                            debug!("mDNS dial {} 失败 (可能已连接): {:?}", peer_id, e);
                        }
                        self.event_bus.Publish(Bus_Event::State {
                            payload: serde_json::json!({
                                "type": "peer_discovered",
                                "peer_id": peer_id.to_string(),
                            }).to_string(),
                        });
                    }
                }
            }
            mdns::Event::Expired(peers) => {
                for (peer_id, _addr) in peers {
                    info!("节点离开: {}", peer_id);
                    self.event_bus.Publish(Bus_Event::State {
                        payload: serde_json::json!({
                            "type": "peer_left",
                            "peer_id": peer_id.to_string(),
                        }).to_string(),
                    });
                }
            }
        }
    }

    /// 处理Kademlia事件
    ///
    /// DHT 结果不再通过 event_sender 通知上层。
    /// 未来通过 orchestrator_event_tx 或 Capability oneshot 模式投递结果。
    pub(super) async fn Handle_Kademlia_Event(&mut self, event: kad::Event) {
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
    /// - Request（入站）：按 DataType 预筛选分流
    /// - Data / Info → Network 内部直接回复（不转发给 Orchestrator）
    ///   - Command / File → 通过 inbound_manager 转发给 Orchestrator Core B5
    /// - Response（出站回复）：通过 oneshot 路由回 Send_Data 调用方
    /// - OutboundFailure：通知等待方发送失败
    pub(super) async fn Handle_Request_Response_Event(
        &mut self,
        event: request_response::Event<Network_Data, Network_Data>,
    ) {
        match event {
            request_response::Event::Message { peer, message, .. } => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    info!("收到数据请求 from {} | type={:?} | size={} bytes",
                          peer, request.data_type, request.payload.len());

                    match request.data_type {
                        DataType::Data => {
                            let response = Network_Data {
                                data_type: DataType::Data,
                                payload: b"OK".to_vec(),
                            };
                            if let Err(e) = self.swarm.behaviour_mut()
                                .request_response.send_response(channel, response) {
                                error!("Data 入站回复失败: {:?}", e);
                            }
                        }
                        DataType::Info => {
                            // Info 类型已废弃（业务状态改走 GossipSub），仅记录日志
                            warn!("收到 Info 类型消息（已废弃，忽略） from {}", peer);
                        }
                        DataType::Command | DataType::File => {
                            self.inbound_manager.Register_Inbound(peer, request, channel).await;
                        }
                    }
                }
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
    pub(super) async fn Handle_Ping_Event(&mut self, event: ping::Event) {
        let peer_id = event.peer;

        match event.result {
            Ok(rtt) => {
                let latency_ms = rtt.as_millis() as u64;
                info!("Ping成功: {} | RTT: {}ms", peer_id, latency_ms);

                let profile = crate::peer_management::PeerProfile {
                    latency_ms: Some(latency_ms),
                    ..crate::peer_management::PeerProfile::default()
                };
                if let Err(e) = self.peer_handle.Update_Profile(&peer_id, profile).await {
                    info!("更新节点心跳失败 ({}): {}", peer_id, e);
                }
            }
            Err(ping::Failure::Timeout) => {
                info!("Ping超时: {}", peer_id);

                if let Err(e) = self.peer_handle.Remove_Peer(&peer_id).await {
                    info!("更新节点状态失败 ({}): {}", peer_id, e);
                }
            }
            Err(ping::Failure::Unsupported) => {
                info!("节点不支持Ping协议: {}", peer_id);
            }
            Err(ping::Failure::Other { error }) => {
                info!("Ping错误 ({}): {}", peer_id, error);
            }
        }
    }

    /// 处理 Identify 事件（连接建立后自动交换的协议级元信息）
    pub(super) async fn Handle_Identify_Event(&mut self, event: identify::Event) {
        match event {
            identify::Event::Received { peer_id, info, .. } => {
                info!("identify 收到: {} | agent={} | proto={:?} | listen={:?}",
                      peer_id, info.agent_version, info.protocols, info.listen_addrs);
                // 更新 PeerManager 地址列表（从连接端点升级为完整监听地址）
                let pi = PeerInfo::new(peer_id, info.listen_addrs);
                let _ = self.peer_handle.Upsert_Peer(pi).await;
            }
            identify::Event::Sent { peer_id, .. } => {
                debug!("identify 已发送: {}", peer_id);
            }
            identify::Event::Pushed { peer_id, .. } => {
                debug!("identify 已推送: {}", peer_id);
            }
            identify::Event::Error { peer_id, error, .. } => {
                debug!("identify 错误 {}: {:?}", peer_id, error);
            }
        }
    }

    /// 处理 GossipSub 事件（业务状态广播：peer-info / models / sessions）
    pub(super) async fn Handle_Gossipsub_Event(&mut self, event: gossipsub::Event) {
        use crate::peer_management::{SessionSummary, SupportedModel};
        match event {
            gossipsub::Event::Message { message, .. } => {
                let Some(author) = message.source else {
                    debug!("收到无作者的 gossipsub 消息，忽略");
                    return;
                };
                match message.topic.as_str() {
                    super::TOPIC_PEER_INFO => {
                        // 节点身份：{"name": "..."}
                        let name = serde_json::from_slice::<serde_json::Value>(&message.data)
                            .ok()
                            .and_then(|v| v["name"].as_str().map(|s| s.to_string()))
                            .unwrap_or_default();
                        if !name.is_empty() {
                            let mut updated = PeerInfo::new(author, vec![]);
                            updated.name = name.clone();
                            let _ = self.peer_handle.Upsert_Peer(updated).await;
                        }
                        // 通知 TUI
                        self.event_bus.Publish(Bus_Event::State {
                            payload: serde_json::json!({
                                "type": "peer_info_updated",
                                "peer_id": author.to_string(),
                                "peer_name": name,
                                "is_local": false,
                                "models": [],
                                "sessions": [],
                            }).to_string(),
                        });
                    }
                    super::TOPIC_MODELS => {
                        // 模型能力：Vec<SupportedModel>
                        let models: Vec<SupportedModel> = serde_json::from_slice(&message.data)
                            .unwrap_or_default();
                        if !models.is_empty() {
                            let _ = self.peer_handle
                                .Update_Supported_Models(&author, models.clone())
                                .await;
                        }
                        let models_display: Vec<serde_json::Value> = models.iter().map(|m| {
                            serde_json::json!({
                                "file_name": m.file_name,
                                "layer_range": m.layer_range(),
                            })
                        }).collect();
                        self.event_bus.Publish(Bus_Event::State {
                            payload: serde_json::json!({
                                "type": "peer_info_updated",
                                "peer_id": author.to_string(),
                                "peer_name": "",
                                "is_local": false,
                                "models": models_display,
                                "sessions": [],
                            }).to_string(),
                        });
                    }
                    super::TOPIC_SESSIONS => {
                        // 会话状态：Vec<SessionSummary>
                        let sessions: Vec<SessionSummary> = serde_json::from_slice(&message.data)
                            .unwrap_or_default();
                        if !sessions.is_empty() {
                            let _ = self.peer_handle
                                .Update_Sessions(&author, sessions.clone())
                                .await;
                        }
                        let sessions_display: Vec<serde_json::Value> = sessions.iter().map(|s| {
                            serde_json::json!({
                                "session_id": s.session_id,
                                "model_id": s.model_id,
                                "occupied_slots": s.occupied_slots,
                                "total_slots": s.total_slots,
                            })
                        }).collect();
                        self.event_bus.Publish(Bus_Event::State {
                            payload: serde_json::json!({
                                "type": "peer_info_updated",
                                "peer_id": author.to_string(),
                                "peer_name": "",
                                "is_local": false,
                                "models": [],
                                "sessions": sessions_display,
                            }).to_string(),
                        });
                    }
                    _ => { debug!("未知 gossipsub topic: {}", message.topic); }
                }
            }
            gossipsub::Event::Subscribed { peer_id, topic } => {
                // 对方订阅 topic：按 topic 精准重放快照（快照机制，类似 MQTT retained message）
                // gossipsub 只投递给已订阅者，订阅完成后发布必达；
                // 连接建立时不重放——对方可能尚未完成订阅。
                debug!("节点订阅 gossipsub topic: {} (peer: {})", topic, peer_id);
                if let Some(payload) = self.snapshot_cache.Get(topic.as_str()) {
                    let _ = self.swarm.behaviour_mut().gossipsub.publish(
                        gossipsub::TopicHash::from_raw(topic.as_str().to_string()),
                        payload,
                    );
                }
            }
            gossipsub::Event::Unsubscribed { peer_id, .. } => {
                debug!("节点退订 gossipsub topic: {}", peer_id);
            }
            gossipsub::Event::GossipsubNotSupported { peer_id } => {
                warn!("节点不支持 gossipsub: {}", peer_id);
            }
            gossipsub::Event::SlowPeer { peer_id, .. } => {
                debug!("gossipsub 慢节点: {}", peer_id);
            }
        }
    }
}