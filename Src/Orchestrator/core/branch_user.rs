// Presented by KeJi
// Date ： 2026-05-19

//! B1: 用户命令路由。
//!
//! 所有业务逻辑用 todo!() 占位，仅验证 Capability 调用通路。

use super::Core;
use crate::orchestrator::command::{UserCommand, NetworkProtocol, Serialize_Network_Command};
use crate::network::DataType;
use crate::vm::engine::LuaContext;
use crate::vm::capability_binding::{
    register_caps, register_logging_caps, register_network_caps,
    register_storage_caps, register_ml_caps, register_robot_caps,
};
use crate::event_bus::{Bus_Event, NotifyLevel};
use std::path::Path;

// ============================================================
// 本地 HelpEntry（已从 EventBus 中解耦）
// ============================================================

struct HelpEntry {
    usage: String,
    description: String,
}

// ============================================================
// Output 构造辅助
// ============================================================

pub(super) fn cmd_output(text: impl Into<String>, completed: bool) -> String {
    serde_json::json!({"type":"cmd_result","text":text.into(),"completed":completed}).to_string()
}

// ============================================================
// 内置命令描述符（与命令实现同文件，就近管理）
// ============================================================

const BUILTIN_COMMANDS: &[(&str, &str)] = &[
    ("api <session_id>",       "启动 OpenAI 兼容 API Server"),
    ("run <model>",           "启动本地推理"),
    ("pipeline <model>",      "启动分布式流水线推理"),
    ("cancel <job_id>",       "取消指定作业"),
    ("dp / display-peer",     "查看节点列表"),
    ("set-device cpu|cuda|cuda:N", "切换计算设备"),
    ("ls",                    "列出存储文件"),
    ("flush",                 "刷新存储索引"),
    ("reload",                "重新加载用户 Lua 脚本"),
    ("set-name <name>",       "设置本地节点名称"),
    ("distribute <model> <peer:0-15> ...", "分发模型分片"),
    ("send <file> <name>",    "向节点发送文件（支持 name 或 peer_id）"),
    ("profile <model>",       "启动 Profile"),
    ("exec <cmd> [k=v ...]",  "执行用户 Lua 脚本"),
    ("clear",                 "清空日志"),
    ("quit / exit",           "退出程序"),
    ("help",                  "显示此帮助"),
];

impl Core {
    /// 路由用户命令 (B1)。
    ///
    /// 每个分支组合调用 Capability 方法，推理相关业务留给未来 Lua 层。
    pub async fn route_user(&mut self, cmd: UserCommand) {
        match cmd {
            // ─── 通用 Lua 脚本执行 (fire-and-forget) ─────
            UserCommand::Execute { command, mut params } => {
                let Some(entry) = self.program_registry.get_user(&command).cloned() else {
                    let available: Vec<_> = self.program_registry.command_names()
                        .iter().map(|s| s.as_str()).collect();
                    tracing::warn!(
                        "[Execute] 未知命令: '{}', 可用: {:?}",
                        command, available
                    );
                    self.capabilities.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(format!("未知命令: {}", command), true),
                    });
                    return;
                };
                tracing::info!(
                    "[Execute] 执行脚本: {} (命令: {})",
                    entry.path.display(),
                    command
                );
                // 解析 peer 参数：name → peer_id
                if let Some(peer_val) = params.get("peer") {
                    if let Ok(info) = self.capabilities.peer_manager.Get_Peer_By_Name(peer_val).await {
                        params.insert("peer".to_string(), info.peer_id.to_string());
                    }
                }
                spawn_lua_script(entry.path, params, self.capabilities.clone(), command);
            }

            // ─── 单机推理 (Lua 脚本 + 模型) ───
            UserCommand::Run { script: _, model_path: _ } => {
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output("Run: not yet implemented", true),
                });
            }

            // ─── 取消作业 ──────────────────────────────────
            UserCommand::Cancel { job_id } => {
                self.cancel_job(job_id);
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("Job #{} 取消信号已发送", job_id.0),
                        true,
                    ),
                });
            }

            // ─── 优雅退出 ──────────────────────────────────
            UserCommand::Quit { reply } => {
                self.shutdown();
                let _ = reply.send(());
            }

            // ─── 查看节点 ──────────────────────────────────
            UserCommand::DisplayPeer => {
                let text = match self.capabilities.peer_manager.Get_All_Peers().await {
                    Ok(peers) => {
                        if peers.is_empty() {
                            "当前无已知节点".to_string()
                        } else {
                            peers.iter()
                                .map(|p| {
                                    let name = p.display_name();
                                    let mut line = format!("{} [latency={:?}ms]", name, p.profile.latency_ms);
                                    if !p.supported_models.is_empty() {
                                        line.push_str(&format!("\n  模型 ({}):", p.supported_models.len()));
                                        for m in &p.supported_models {
                                            line.push_str(&format!("\n    {} (id={:08x}) [layers: {}]", m.file_name, m.id, m.layer_range()));
                                        }
                                    }
                                    line
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                    Err(e) => format!("错误: {}", e),
                };
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(text, true),
                });
            }

            // ─── 设置本地节点名称 ──────────────────────────
            UserCommand::SetName { name } => {
                let text = match self.capabilities.peer_manager.Set_Local_Name(&name).await {
                    Ok(()) => {
                        // 同时持久化到 config
                        let config_path = Path::new(".config").join("config.toml");
                        if let Err(e) = crate::config::Set_Peer_Name(&config_path, &name) {
                            format!("名称已设置为 {}，但持久化失败: {}", name, e)
                        } else {
                            // 广播新的节点名称（GossipSub peer-info topic）
                            crate::network::publish_peer_info(
                                &*self.capabilities.peer_manager,
                                &*self.capabilities.network,
                                &self.capabilities.event_bus,
                            ).await;
                            format!("节点名称已设置为: {}", name)
                        }
                    }
                    Err(e) => format!("设置名称失败: {}", e),
                };
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(text, true),
                });
            }

            // ─── 重新加载 Lua 脚本 ──────────────────────────
            UserCommand::Reload => {
                let before = self.program_registry.command_names().len();
                self.program_registry.reload_user();
                let after = self.program_registry.command_names().len();
                let text = format!(
                    "reload 完成: 重新扫描 user 脚本 (总命令数: {} → {})",
                    before, after
                );
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(text, true),
                });
            }

            // ─── 设置设备 ──────────────────────────────────
            UserCommand::SetDevice { device } => {
                self.device_preference = device.clone();
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("设备已切换为: {}", device.to_uppercase()),
                        true,
                    ),
                });
            }

            // ─── 模型分发 ──────────────────────────────────
            UserCommand::DistributeModel { model_path: _, peers: _ } => {
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output("DistributeModel: not yet implemented", true),
                });
            }

            // ─── 列出模型 ──────────────────────────────────
            UserCommand::List => {
                let (text, json_entries) = match self.capabilities.storage.List().await {
                    Ok(entries) => {
                        if entries.is_empty() {
                            ("存储为空（无文件）".to_string(), vec![])
                        } else {
                            let mut output = format!("共 {} 个文件:\n", entries.len());
                            let mut items: Vec<serde_json::Value> = Vec::new();
                            for e in &entries {
                                let size = if e.size >= 1_073_741_824 {
                                    format!("{:.1} GB", e.size as f64 / 1_073_741_824.0)
                                } else if e.size >= 1_048_576 {
                                    format!("{:.1} MB", e.size as f64 / 1_048_576.0)
                                } else if e.size >= 1024 {
                                    format!("{:.1} KB", e.size as f64 / 1024.0)
                                } else {
                                    format!("{} B", e.size)
                                };
                                let bitmap_hex = e.layer_bitmap.map(|bm| {
                                    let mut s = String::with_capacity(64);
                                    for b in &bm { s.push_str(&format!("{:02x}", b)); }
                                    s
                                });
                                if let (Some(arch), Some(layers), Some(id)) =
                                    (e.architecture.as_ref(), e.num_layers, e.model_id)
                                {
                                    output.push_str(&format!(
                                        "  {:<40} {:>8}  [{} {} layers, id={:08x}]\n",
                                        e.file_name, size, arch, layers, id
                                    ));
                                } else {
                                    output.push_str(&format!(
                                        "  {:<40} {:>8}\n",
                                        e.file_name, size
                                    ));
                                }
                                items.push(serde_json::json!({
                                    "file_name": e.file_name,
                                    "size": e.size,
                                    "architecture": e.architecture,
                                    "num_layers": e.num_layers,
                                    "model_id": e.model_id,
                                    "layer_bitmap": bitmap_hex,
                                }));
                            }
                            (output, items)
                        }
                    }
                    Err(e) => (format!("错误: {}", e), vec![]),
                };
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: serde_json::json!({
                        "type": "cmd_result",
                        "text": text,
                        "completed": true,
                        "entries": json_entries,
                    })
                    .to_string(),
                });
            }

            // ─── 刷新存储索引 (fire-and-forget) ──────────
            UserCommand::Flush => {
                let caps = self.capabilities.clone();
                tokio::spawn(async move {
                    let text = Core::do_flush(&caps).await;
                    caps.event_bus.Publish(Bus_Event::Output { payload: cmd_output(text, true) });
                });
            }

            // ─── 手动 dial 节点 ─────────────────────────
            UserCommand::Dial { addr } => {
                let caps = self.capabilities.clone();
                tokio::spawn(async move {
                    match addr.parse::<libp2p::Multiaddr>() {
                        Ok(ma) => match caps.network.dial(ma).await {
                            Ok(()) => caps.event_bus.Publish(Bus_Event::Notify {
                                level: NotifyLevel::Info,
                                message: format!("dial {} 成功", addr),
                            }),
                            Err(e) => caps.event_bus.Publish(Bus_Event::Notify {
                                level: NotifyLevel::Error,
                                message: format!("dial {} 失败: {}", addr, e),
                            }),
                        },
                        Err(e) => caps.event_bus.Publish(Bus_Event::Notify {
                            level: NotifyLevel::Error,
                            message: format!("无效 Multiaddr: {}", e),
                        }),
                    }
                });
            }

            // ─── 发送文件 (fire-and-forget) ──────────────
            UserCommand::Send { file_path, peer_id } => {
                let Some(entry) = self.program_registry.get("send").cloned() else {
                    self.capabilities.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output("send 脚本未找到", true),
                    });
                    return;
                };
                // 解析目标：优先按 name 查找，否则当作裸 peer_id
                let target_id = match self.capabilities.peer_manager.Get_Peer_By_Name(&peer_id).await {
                    Ok(info) => info.peer_id.to_string(),
                    Err(_) => peer_id.clone(),
                };
                let mut params = std::collections::HashMap::new();
                params.insert("file".into(), file_path.clone());
                params.insert("peer".into(), target_id);
                spawn_lua_script(
                    entry.path,
                    params,
                    self.capabilities.clone(),
                    format!("send {} -> {}", file_path, peer_id),
                );
            }

            // ─── 性能测试 ──────────────────────────────────
            UserCommand::Profile { model_id: _ } => {
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output("Profile: 尚未实现", true),
                });
            }

            // ─── 帮助信息 ──────────────────────────────────
            UserCommand::Help => {
                let mut builtin: Vec<HelpEntry> = BUILTIN_COMMANDS
                    .iter()
                    .map(|(usage, desc)| HelpEntry {
                        usage: usage.to_string(),
                        description: desc.to_string(),
                    })
                    .collect();

                for e in self.program_registry.builtin_entries() {
                    builtin.push(HelpEntry {
                        usage: e.command.clone(),
                        description: e.description.clone(),
                    });
                }

                let user: Vec<HelpEntry> = self.program_registry
                    .user_entries()
                    .into_iter()
                    .map(|e| HelpEntry {
                        usage: e.command.clone(),
                        description: e.description.clone(),
                    })
                    .collect();

                let mut lines = vec!["[内置命令]".to_string()];
                for e in &builtin {
                    lines.push(format!("  {:<38} {}", e.usage, e.description));
                }
                lines.push(String::new());
                lines.push("[用户命令]".to_string());
                if user.is_empty() {
                    lines.push("  (无)".to_string());
                } else {
                    for e in &user {
                        lines.push(format!("  {:<38} {}", e.usage, e.description));
                    }
                }
                let text = lines.join("\n");

                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: serde_json::json!({"type":"help","text":text}).to_string(),
                });
            }
            // ════════════════════════════════════════════════
            // Session / Chat (v2)
            // ════════════════════════════════════════════════
            UserCommand::Session { model_id } => {
                let session_mgr = self.session_mgr.clone();
                let caps = self.capabilities.clone();
                tokio::spawn(async move {
                    let id = session_mgr.lock().unwrap().create_session(&model_id);

                    // 更新 PeerManager 本地 sessions
                    if let Ok(local) = caps.peer_manager.Get_Local_Peer().await {
                        let sessions: Vec<crate::peer_management::SessionSummary> =
                            session_mgr.lock().unwrap().list_sessions().iter().map(|s| {
                                crate::peer_management::SessionSummary {
                                    session_id: s.session_id,
                                    model_id: s.model_id.clone(),
                                    occupied_slots: s.occupied_slots,
                                    total_slots: s.total_slots,
                                }
                            }).collect();
                        let _ = caps.peer_manager.Update_Sessions(&local.peer_id, sessions).await;
                    }

                    // 广播本地会话状态（GossipSub sessions topic + EventBus → TUI）
                    crate::network::publish_sessions(&*caps.peer_manager, &*caps.network, &caps.event_bus).await;

                    caps.event_bus.Publish(crate::event_bus::Bus_Event::Notify {
                        level: crate::event_bus::NotifyLevel::Info,
                        message: format!("Session {} created (model: {})", id, model_id),
                    });
                });
            }
            UserCommand::SessionInference { command, session_id, model_path } => {
                let Some(entry) = self.program_registry.get(&command).cloned() else {
                    tracing::error!("SessionInference: script '{}' not found", command);
                    self.capabilities.event_bus.Publish(Bus_Event::Notify {
                        level: NotifyLevel::Error,
                        message: format!("ML Thread: 脚本 '{}' 未找到", command),
                    });
                    return;
                };
                let mut params = std::collections::HashMap::new();
                params.insert("session_id".into(), session_id.to_string());
                params.insert("model_path".into(), model_path);
                spawn_lua_script(
                    entry.path,
                    params,
                    self.capabilities.clone(),
                    format!("inference:{} session {}", command, session_id),
                );
            }
            // ════════════════════════════════════════════════
            // API Server (OpenAI 兼容)
            // ════════════════════════════════════════════════
            UserCommand::Api { session_id } => {
                let session_mgr = self.session_mgr.clone();
                let event_bus = self.capabilities.event_bus.clone();

                let model_id = {
                    let mgr = session_mgr.lock().unwrap();
                    let sessions = mgr.list_sessions();
                    sessions
                        .iter()
                        .find(|s| s.session_id == session_id)
                        .map(|s| s.model_id.clone())
                        .unwrap_or_else(|| "unknown".to_string())
                };

                tokio::spawn(async move {
                    match crate::api::spawn_api_server(
                        session_mgr,
                        session_id,
                        model_id.clone(),
                    ).await {
                        Ok(port) => {
                            event_bus.Publish(Bus_Event::Notify {
                                level: NotifyLevel::Info,
                                message: format!(
                                    "API server started for session {} on http://127.0.0.1:{} (model: {})",
                                    session_id, port, model_id
                                ),
                            });
                        }
                        Err(e) => {
                            event_bus.Publish(Bus_Event::Notify {
                                level: NotifyLevel::Error,
                                message: format!("API server failed for session {}: {}", session_id, e),
                            });
                        }
                    }
                });
            }
            // ════════════════════════════════════════════════
            // 远程 Lua 脚本执行 (请求-响应)
            // ════════════════════════════════════════════════
            UserCommand::ExecRemote { peer, command, params } => {
                // 1. 解析 peer 名称 → peer_id
                let Ok(info) = self.capabilities.peer_manager.Get_Peer_By_Name(&peer).await else {
                    self.capabilities.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(format!("未知节点: {}", peer), true),
                    });
                    return;
                };
                // 2. JSON 序列化参数
                let params_json = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
                // 3. 构造协议消息
                let proto = NetworkProtocol::ExecRemote {
                    command: command.clone(),
                    params_json,
                };
                let payload = Serialize_Network_Command(&proto);
                let payload_str = String::from_utf8_lossy(&payload);
                tracing::info!(
                    "[B1] rexec → peer={} ({}) payload={}",
                    peer, info.peer_id, payload_str
                );
                // 4. 发送到远程节点
                match self.capabilities.network.send_data(
                    info.peer_id.clone(),
                    DataType::Command,
                    payload,
                ).await {
                    Ok(response) => {
                        let text = String::from_utf8_lossy(&response.payload);
                        self.capabilities.event_bus.Publish(Bus_Event::Output {
                            payload: cmd_output(
                                format!("[{}] 远程执行完成: {}", peer, text),
                                true,
                            ),
                        });
                    }
                    Err(e) => {
                        self.capabilities.event_bus.Publish(Bus_Event::Output {
                            payload: cmd_output(
                                format!("[{}] 远程执行失败: {}", peer, e),
                                true,
                            ),
                        });
                    }
                }
            }
        }
    }
}

/// Fire-and-forget: 在独立线程中加载并执行 Lua 脚本。
///
/// 线程内创建 tokio runtime 驱动异步能力函数，执行结果通过 EventBus 推送。
/// Core 调用后立即返回，不等待脚本完成。
pub(super) fn spawn_lua_script(
    path: std::path::PathBuf,
    params: std::collections::HashMap<String, String>,
    caps: std::sync::Arc<crate::orchestrator::Capabilities>,
    label: String,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                caps.event_bus.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Error,
                    message: format!("[{}] 创建 tokio runtime 失败: {}", label, e),
                });
                return;
            }
        };

        rt.block_on(async {
            // 1. 读取脚本
            let script = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    caps.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(
                            format!("[{}] 读取脚本失败: {}", label, e),
                            true,
                        ),
                    });
                    return;
                }
            };

            // 2. 创建沙箱 Lua 实例
            let lua = match LuaContext::new() {
                Ok(l) => l,
                Err(e) => {
                    caps.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(
                            format!("[{}] 创建 Lua 实例失败: {}", label, e),
                            true,
                        ),
                    });
                    return;
                }
            };

            // 3. 注册能力函数
            if let Err(e) = register_caps(&lua) {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 注册基础能力失败: {}", label, e),
                        true,
                    ),
                });
                return;
            }
            if let Err(e) = register_logging_caps(&lua, caps.event_bus.clone()) {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 注册日志能力失败: {}", label, e),
                        true,
                    ),
                });
                return;
            }
            if let Err(e) = register_network_caps(&lua, caps.clone()) {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 注册 Network 能力失败: {}", label, e),
                        true,
                    ),
                });
                return;
            }
            if let Err(e) = register_storage_caps(&lua, caps.storage.clone()) {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 注册 Storage 能力失败: {}", label, e),
                        true,
                    ),
                });
                return;
            }
            if let Err(e) = register_ml_caps(&lua) {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 注册 ML 能力失败: {}", label, e),
                        true,
                    ),
                });
                return;
            }

            // 3.5 注册本地张量流能力
            if let Err(e) = crate::vm::local_stream::register_local_stream_caps(
                &lua,
                caps.local_stream_hub.clone(),
            ) {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 注册 LocalStream 能力失败: {}", label, e),
                        true,
                    ),
                });
                return;
            }

            // 3.6 注册 Robot 能力
            if let Err(e) = register_robot_caps(&lua) {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 注册 Robot 能力失败: {}", label, e),
                        true,
                    ),
                });
                return;
            }

            // 4. 编译脚本
            if let Err(e) = lua.load(&script).eval::<()>() {
                caps.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(
                        format!("[{}] 脚本语法错误: {}", label, e),
                        true,
                    ),
                });
                return;
            }

            // 5. 构造参数 table
            let params_table = match lua.create_table() {
                Ok(t) => t,
                Err(e) => {
                    caps.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(
                            format!("[{}] 创建参数表失败: {}", label, e),
                            true,
                        ),
                    });
                    return;
                }
            };
            for (k, v) in &params {
                if let Err(e) = params_table.set(k.as_str(), v.as_str()) {
                    caps.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(
                            format!("[{}] 设置参数 {} 失败: {}", label, k, e),
                            true,
                        ),
                    });
                    return;
                }
            }

            // 6. 调用 execute 函数
            let execute: mlua::Function = match lua.globals().get("execute") {
                Ok(f) => f,
                Err(_) => {
                    caps.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(
                            format!("[{}] 脚本缺少 execute 函数", label),
                            true,
                        ),
                    });
                    return;
                }
            };

            match execute.call_async::<mlua::Value>(params_table).await {
                Ok(_) => {
                    caps.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(
                            format!("[{}] 执行完成", label),
                            true,
                        ),
                    });
                }
                Err(e) => {
                    tracing::error!("[{}] 执行失败: {}", label, e);
                    caps.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(
                            format!("[{}] 执行失败: {}", label, e),
                            true,
                        ),
                    });
                }
            }
        });
    });
}
