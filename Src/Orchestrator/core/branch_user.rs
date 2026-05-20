// Presented by KeJi
// Date ： 2026-05-19

//! B1: 用户命令路由。
//!
//! 所有业务逻辑用 todo!() 占位，仅验证 Capability 调用通路。

use super::Core;
use crate::orchestrator::command::UserCommand;
use crate::vm::engine::LuaContext;
use crate::vm::capability_binding::{
    register_caps, register_logging_caps, register_network_caps,
    register_storage_caps, register_ml_caps,
};
use crate::event_bus::{Bus_Event, NotifyLevel};

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

fn cmd_output(text: impl Into<String>, completed: bool) -> String {
    serde_json::json!({"type":"cmd_result","text":text.into(),"completed":completed}).to_string()
}

// ============================================================
// 内置命令描述符（与命令实现同文件，就近管理）
// ============================================================

const BUILTIN_COMMANDS: &[(&str, &str)] = &[
    ("run <model>",           "启动本地推理"),
    ("pipeline <model>",      "启动分布式流水线推理"),
    ("cancel <job_id>",       "取消指定作业"),
    ("dp / display-peer",     "查看节点列表"),
    ("set-device cpu|cuda",   "切换计算设备"),
    ("ls",                    "列出存储文件"),
    ("distribute <model> <peer:0-15> ...", "分发模型分片"),
    ("send <file> <peer>",    "向节点发送文件"),
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
            UserCommand::Execute { command, params } => {
                let Some(entry) = self.program_registry.get_user(&command).cloned() else {
                    self.capabilities.event_bus.Publish(Bus_Event::Output {
                        payload: cmd_output(format!("未知命令: {}", command), true),
                    });
                    return;
                };
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
                let text = match self.capabilities.peer_manager.Get_Peers().await {
                    Ok(peers) => {
                        if peers.is_empty() {
                            "当前无已知节点".to_string()
                        } else {
                            peers.iter()
                                .map(|p| format!("{} [mem={:?}MB latency={:?}ms]", p.peer_id, p.profile.memory_mb, p.profile.latency_ms))
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
                let text = match self.capabilities.storage.list().await {
                    Ok(entries) => {
                        if entries.is_empty() {
                            "存储为空（无文件）".to_string()
                        } else {
                            let mut output = format!("共 {} 个文件:\n", entries.len());
                            for e in &entries {
                                output.push_str(&format!("  {} ({} bytes)\n", e.file_name, e.size));
                            }
                            output
                        }
                    }
                    Err(e) => format!("错误: {}", e),
                };
                self.capabilities.event_bus.Publish(Bus_Event::Output {
                    payload: cmd_output(text, true),
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
                let mut params = std::collections::HashMap::new();
                params.insert("file".into(), file_path.clone());
                params.insert("peer".into(), peer_id.clone());
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
        }
    }
}

/// Fire-and-forget: 在独立线程中加载并执行 Lua 脚本。
///
/// 线程内创建 tokio runtime 驱动异步能力函数，执行结果通过 EventBus 推送。
/// Core 调用后立即返回，不等待脚本完成。
fn spawn_lua_script(
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
