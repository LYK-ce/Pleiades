// Presented by KeJi
// Date ： 2026-05-16

//! B1: 用户命令路由。
//!
//! 所有业务逻辑用 todo!() 占位，仅验证 Capability 调用通路。

use super::Core;
use crate::orchestrator::job::JobId;
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
            UserCommand::Execute { command, params, reply } => {
                let entry = match self.program_registry.get_user(&command) {
                    Some(e) => e.clone(),
                    None => {
                        let _ = reply.send(Err(format!("未知命令: {}", command)));
                        return;
                    }
                };

                let caps = self.capabilities.clone();
                // 直接在 Core 的 async 上下文中执行，避免 Lua (非 Send) 跨线程
                let result = execute_lua_script(&entry.path, &params, &caps).await;
                let _ = reply.send(result.map(|_v| JobId(super::generate_id())));
            }

            // ─── 单机推理 (Lua 脚本 + 模型) ───
            UserCommand::Run { script, model_path, reply } => {
                // 1. analyze_model(model_path).await → Model_Arch_Info
                // 2. session.create_session(...).await → (session_id, IoHandle)
                // 3. 加载并执行 Lua script → ml.load_model(...) → sess:forward/sample (todo!())
                let _ = reply.send(Err("Run: not yet implemented".into()));
            }

            // ─── 取消作业 ──────────────────────────────────
            UserCommand::Cancel { job_id, reply } => {
                self.cancel_job(job_id);
                let _ = reply.send(Ok(()));
            }

            // ─── 优雅退出 ──────────────────────────────────
            UserCommand::Quit { reply } => {
                self.shutdown();
                let _ = reply.send(());
            }

            // ─── 查看节点 ──────────────────────────────────
            UserCommand::DisplayPeer { reply } => {
                let result = match self.capabilities.peer_manager.Get_Peers().await {
                    Ok(peers) => {
                        let list: Vec<String> = peers.iter()
                            .map(|p| format!("{} [mem={:?}MB latency={:?}ms]", p.peer_id, p.profile.memory_mb, p.profile.latency_ms))
                            .collect();
                        Ok(list)
                    }
                    Err(e) => Err(format!("{e}")),
                };
                let _ = reply.send(result);
            }

            // ─── 设置设备 ──────────────────────────────────
            UserCommand::SetDevice { device, reply } => {
                self.device_preference = device;
                let _ = reply.send(Ok(()));
            }

            // ─── 模型分发 ──────────────────────────────────
            UserCommand::DistributeModel { model_path, peers, reply } => {
                // 1. analyze_model(&Path::new(&model_path)).await
                // 2. split_model(...).await (per peer)
                // 3. storage.acquire_read → network.send_data → network.open_file_stream
                let _ = reply.send(Err("DistributeModel: not yet implemented".into()));
            }

            // ─── 列出模型 ──────────────────────────────────
            UserCommand::List { reply } => {
                let result = match self.capabilities.storage.list().await {
                    Ok(entries) => {
                        let list: Vec<String> = entries.iter()
                            .map(|e| format!("{} ({} bytes)", e.file_name, e.size))
                            .collect();
                        Ok(list)
                    }
                    Err(e) => Err(format!("{e}")),
                };
                let _ = reply.send(result);
            }

            // ─── 发送文件 ──────────────────────────────────
            UserCommand::Send { file_path, peer_id, reply } => {
                let entry = match self.program_registry.get("send") {
                    Some(e) => e.clone(),
                    None => {
                        let _ = reply.send(Err("send 脚本未找到".into()));
                        return;
                    }
                };
                let mut params = std::collections::HashMap::new();
                params.insert("file".into(), file_path);
                params.insert("peer".into(), peer_id);
                let caps = self.capabilities.clone();
                let result = execute_lua_script(&entry.path, &params, &caps).await;
                let _ = reply.send(result.map(|_v| JobId(super::generate_id())));
            }

            // ─── 性能测试 ──────────────────────────────────
            UserCommand::Profile { model_id: _, reply } => {
                let _ = reply.send(Err("Profile: 尚未实现".into()));
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
