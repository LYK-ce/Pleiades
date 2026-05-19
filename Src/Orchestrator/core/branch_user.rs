// Presented by KeJi
// Date ： 2026-05-16

//! B1: 用户命令路由。
//!
//! 所有业务逻辑用 todo!() 占位，仅验证 Capability 调用通路。

use super::Core;
use crate::orchestrator::command::UserCommand;
use crate::lua::engine::LuaContext;
use crate::lua::capability_binding::{
    register_caps, register_logging_caps, register_network_caps,
    register_storage_caps, register_ml_caps,
};
use crate::event_bus::event::{Bus_Event, HelpEntry};

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
            // ─── 通用 Lua 脚本执行 ─────────────────────────
            UserCommand::Execute { command, params } => {
                let entry = match self.program_registry.get_user(&command) {
                    Some(e) => e.clone(),
                    None => {
                        self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                            text: format!("未知命令: {}", command),
                            completed: true,
                        });
                        return;
                    }
                };

                let caps = self.capabilities.clone();
                let result = execute_lua_script(&entry.path, &params, &caps).await;
                match result {
                    Ok(_v) => {
                        let job_id = super::generate_id();
                        self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                            text: format!("脚本 '{}' 执行完成 (Job #{})", command, job_id),
                            completed: true,
                        });
                    }
                    Err(e) => {
                        self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                            text: format!("执行失败: {}", e),
                            completed: true,
                        });
                    }
                }
            }

            // ─── 单机推理 (Lua 脚本 + 模型) ───
            UserCommand::Run { script: _, model_path: _ } => {
                self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                    text: "Run: not yet implemented".to_string(),
                    completed: true,
                });
            }

            // ─── 取消作业 ──────────────────────────────────
            UserCommand::Cancel { job_id } => {
                self.cancel_job(job_id);
                self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                    text: format!("Job #{} 取消信号已发送", job_id.0),
                    completed: true,
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
                self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                    text,
                    completed: true,
                });
            }

            // ─── 设置设备 ──────────────────────────────────
            UserCommand::SetDevice { device } => {
                self.device_preference = device.clone();
                self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                    text: format!("设备已切换为: {}", device.to_uppercase()),
                    completed: true,
                });
            }

            // ─── 模型分发 ──────────────────────────────────
            UserCommand::DistributeModel { model_path: _, peers: _ } => {
                self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                    text: "DistributeModel: not yet implemented".to_string(),
                    completed: true,
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
                self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                    text,
                    completed: true,
                });
            }

            // ─── 发送文件 ──────────────────────────────────
            UserCommand::Send { file_path, peer_id } => {
                let entry = match self.program_registry.get("send") {
                    Some(e) => e.clone(),
                    None => {
                        self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                            text: "send 脚本未找到".to_string(),
                            completed: true,
                        });
                        return;
                    }
                };
                let mut params = std::collections::HashMap::new();
                params.insert("file".into(), file_path.clone());
                params.insert("peer".into(), peer_id.clone());
                let caps = self.capabilities.clone();
                let result = execute_lua_script(&entry.path, &params, &caps).await;
                match result {
                    Ok(_v) => {
                        let job_id = super::generate_id();
                        self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                            text: format!("发送 Job #{} 已创建 ({} → {})", job_id, file_path, peer_id),
                            completed: true,
                        });
                    }
                    Err(e) => {
                        self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                            text: format!("错误: {}", e),
                            completed: true,
                        });
                    }
                }
            }

            // ─── 性能测试 ──────────────────────────────────
            UserCommand::Profile { model_id: _ } => {
                self.capabilities.event_bus.Publish(Bus_Event::CommandResult {
                    text: "Profile: 尚未实现".to_string(),
                    completed: true,
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

                // 合并 Lua 内置脚本的元数据
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

                self.capabilities.event_bus.Publish(Bus_Event::HelpInfo { builtin, user });
            }
        }
    }
}

/// 加载并执行 Lua 脚本。
///
/// 1. 创建沙箱 Lua 实例
/// 2. 注册 caps 函数表
/// 3. 加载脚本 → 构造 params table → 调用 execute(params, caps)
async fn execute_lua_script(
    path: &std::path::Path,
    params: &std::collections::HashMap<String, String>,
    caps: &std::sync::Arc<crate::orchestrator::Capabilities>,
) -> Result<mlua::Value, String> {
    let script = std::fs::read_to_string(path)
        .map_err(|e| format!("读取脚本失败: {}", e))?;

    let lua = LuaContext::new()
        .map_err(|e| format!("创建 Lua 实例失败: {}", e))?;

    register_caps(&lua)
        .map_err(|e| format!("注册能力函数失败: {}", e))?;

    register_logging_caps(&lua, caps.event_bus.clone())
        .map_err(|e| format!("注册日志能力失败: {}", e))?;

    register_network_caps(&lua, caps.clone())
        .map_err(|e| format!("注册 Network 能力失败: {}", e))?;

    register_storage_caps(&lua, caps.storage.clone())
        .map_err(|e| format!("注册 Storage 能力失败: {}", e))?;

    register_ml_caps(&lua)
        .map_err(|e| format!("注册 ML 能力失败: {}", e))?;

    lua.load(&script).eval::<()>()
        .map_err(|e| format!("脚本语法错误: {}", e))?;

    let params_table = lua.create_table()
        .map_err(|e| format!("创建 params 表失败: {}", e))?;
    for (k, v) in params {
        params_table.set(k.as_str(), v.as_str())
            .map_err(|e| format!("设置参数 {} 失败: {}", k, e))?;
    }

    let execute: mlua::Function = lua.globals().get("execute")
        .map_err(|_| "脚本缺少 execute 函数".to_string())?;

    execute.call_async::<mlua::Value>(params_table).await
        .map_err(|e| format!("脚本执行失败: {}", e))
}
