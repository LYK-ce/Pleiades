// Presented by KeJi
// Date ： 2026-05-16

//! B1: 用户命令路由。
//!
//! 所有业务逻辑用 todo!() 占位，仅验证 Capability 调用通路。

use super::Core;
use super::job::{JobId, JobKind};
use super::command::UserCommand;

impl Core {
    /// 路由用户命令 (B1)。
    ///
    /// 每个分支组合调用 Capability 方法，推理相关业务留给未来 Lua 层。
    pub async fn route_user(&mut self, cmd: UserCommand) {
        match cmd {
            // ─── 通用 Lua 脚本执行 ─────────────────────────
            UserCommand::Execute { command, params, reply } => {
                // 1. ProgramRegistry::get(&command) → 查找 scripts/programs/*.lua
                // 2. 加载 Lua 脚本 → 注册 caps 函数 → 调用 execute(params, caps)
                // 3. todo!() — 等待 Lua capability_binding 完成
                let _ = reply.send(Err(format!("Execute '{}': not yet implemented", command)));
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
                todo!("Send file to peer")
            }

            // ─── 性能测试 ──────────────────────────────────
            UserCommand::Profile { model_id, reply } => {
                todo!("Profile model")
            }
        }
    }
}
