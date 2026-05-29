// Presented by KeJi
// Date ： 2026-05-29

//! B2: Request-Response 入站请求路由。
//!
//! 处理来自 Network_Service 的 InboundRequest（Command / File DataType）。

use super::Core;
use crate::event_bus::{Bus_Event, NotifyLevel};
use crate::network::{InboundRequest, DataType};
use crate::orchestrator::command::{NetworkProtocol, Parse_Network_Command};

impl Core {
    /// 路由入站请求 (B2)。
    pub async fn route_inbound(&mut self, req: InboundRequest) {
        match req.data_type {
            DataType::Command => {
                match Parse_Network_Command(&req.payload) {
                    Ok(NetworkProtocol::ExecRemote { command, params_json }) => {
                        // 反序列化参数
                        let params: std::collections::HashMap<String, String> =
                            serde_json::from_str(&params_json).unwrap_or_default();

                        // 查找脚本
                        match self.program_registry.get_user(&command) {
                            Some(entry) => {
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                super::branch_user::spawn_lua_script_with_reply(
                                    entry.path.clone(),
                                    params,
                                    self.capabilities.clone(),
                                    command.clone(),
                                    tx,
                                );
                                // 等待脚本执行完成（60s 超时）
                                let result = tokio::time::timeout(
                                    std::time::Duration::from_secs(60),
                                    rx,
                                )
                                .await
                                .unwrap_or(Ok("TIMEOUT".to_string()))
                                .unwrap_or_else(|_| "ERR".to_string());
                                let _ = self.capabilities.network.send_response(
                                    req.request_id,
                                    DataType::Command,
                                    result.into_bytes(),
                                )
                                .await;
                            }
                            None => {
                                tracing::warn!(
                                    "[B2] 未知远程命令: '{}' (peer: {})",
                                    command, req.peer
                                );
                                let _ = self.capabilities.network.send_response(
                                    req.request_id,
                                    DataType::Command,
                                    format!("FAIL|未知命令: {}", command).into_bytes(),
                                )
                                .await;
                            }
                        }
                    }
                    Ok(other) => {
                        tracing::info!(
                            "[B2] 未处理的 NetworkProtocol: {:?} (peer: {})",
                            other, req.peer
                        );
                        let _ = self.capabilities.network.send_response(
                            req.request_id,
                            DataType::Command,
                            format!("FAIL|未实现的协议: {:?}", other).into_bytes(),
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[B2] 协议解析失败: {} (peer: {})",
                            e, req.peer
                        );
                        let _ = self.capabilities.network.send_response(
                            req.request_id,
                            DataType::Command,
                            format!("FAIL|协议解析: {}", e).into_bytes(),
                        )
                        .await;
                    }
                }
            }
            DataType::File => {
                // TODO: 未来实现文件元数据协商 → pending_file_receives
                self.capabilities.event_bus.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Info,
                    message: format!(
                        "[route_inbound] File 入站暂未实现 (peer: {})",
                        req.peer
                    ),
                });
                tracing::info!("[route_inbound] File from {} not yet implemented", req.peer);
            }
            _ => {
                // BandwidthTest / Data / Info 由 Network_Service 内部处理，不到达 Core
            }
        }
    }
}
