// Presented by KeJi
// Date ： 2026-05-05

//! B1 分支：用户命令路由
//!
//! 处理来自 TUI / CLI 的 `UserCommand`，编译并 spawn 对应的 Job。

use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{Core, generate_id, JobHandle};
use crate::orchestrator::job::{JobId, JobKind};
use crate::orchestrator::command::UserCommand;
use crate::orchestrator::core::job_executor::JobExecutor;
use crate::orchestrator::program_selector::ProgramSelector;
use crate::orchestrator::inference_id::Generate_Inference_Id;
use crate::config::Update_Config;
use crate::event_bus::Bus_Event;
use crate::llm_io::LLM_IO_Capability;
use crate::storage::StorageCapability;

impl Core {
    /// 路由用户命令（异步，因需要调用 io_broker.Allocate）
    ///
    /// 每个分支处理完后通过 `reply` 通道回传结果给前端。
    pub(super) async fn route_user(&mut self, cmd: UserCommand) {
        match cmd {
            UserCommand::Run { model_path, reply } => {
                let job_id = JobId(generate_id());
                // 使用 Core 的 device_preference（空字符串时传 None，由 Compiler 默认 "cpu"）
                let device = if self.device_preference.is_empty() { "cpu".to_string() } else { self.device_preference.clone() };
                let mut vars = HashMap::new();
                vars.insert("model_path".to_string(), model_path.clone());
                vars.insert("device".to_string(), device.clone());
                let program = match ProgramSelector::select(JobKind::Run, job_id, vars) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = reply.send(Err(format!("编译失败: {:?}", e)));
                        return;
                    }
                };
                // 编译成功后分配 IO 通道（推理类 Job 需要 IoHandle）
                if let Err(e) = self.capabilities.io_broker.Allocate(job_id).await {
                    let _ = reply.send(Err(format!("IO 分配失败: {}", e)));
                    return;
                }
                match self.capabilities.io_broker.Take_ML_Side(job_id).await {
                    Ok(io) => {
                        self.spawn_job(job_id, JobKind::Run, program, Some(io));
                        let _ = reply.send(Ok(job_id));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("IO Take_ML_Side 失败: {}", e)));
                    }
                }
            }
            UserCommand::Cancel { job_id, reply } => {
                if self.registry.contains_key(&job_id) {
                    self.cancel_job(job_id);
                    let _ = reply.send(Ok(()));
                } else {
                    let _ = reply.send(Err(format!("Job {:?} 不存在", job_id)));
                }
            }
            UserCommand::Quit { reply } => {
                self.shutdown();
                let _ = reply.send(());
            }
            UserCommand::DisplayPeer { reply } => {
                match self.capabilities.peer_manager.List_Peers().await {
                    Ok(peers) => {
                        let peer_strs: Vec<String> = peers.iter().map(|p| {
                            let mut s = format!("{} [{:?}] lat={:?}ms bw={:?}Mbps",
                                p.peer_id, p.status, p.latency_ms, p.bandwidth_mbps);
                            if let Some(ref cap) = p.capability {
                                if cap.memory_mb > 0 {
                                    s.push_str(&format!(" mem={}MB", cap.memory_mb));
                                }
                                if !cap.layer_time.is_empty() {
                                    s.push_str(" layer:");
                                    for (model, d) in &cap.layer_time {
                                        s.push_str(&format!(" {}:{:.2}ms", model, d.as_secs_f64() * 1000.0));
                                    }
                                }
                            }
                            s
                        }).collect();
                        let _ = reply.send(Ok(peer_strs));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("查询节点列表失败: {}", e)));
                    }
                }
            }
            UserCommand::SetDevice { device, reply } => {
                let old_device = self.device_preference.clone();
                // 1. 更新内存
                self.device_preference = device.clone();
                info!("设备已切换: {} → {}", if old_device.is_empty() { "default(cpu)" } else { &old_device }, &device);

                // 2. 持久化到 config.toml
                if let Err(e) = Update_Config(&self.config_path, "Runtime", "device", &device) {
                    warn!("配置文件写入失败: {} (设备已切换但未持久化)", e);
                } else {
                    info!("配置文件已更新: device = {}", device);
                }

                // 3. 通知 TUI 更新设备显示
                self.capabilities.event_bus.Publish(Bus_Event::Device_Changed {
                    device: device.clone(),
                });

                // 4. 回复成功
                let _ = reply.send(Ok(()));
            }
            UserCommand::DistributeModel { model_path: _, peers: _, reply } => {
                let _ = reply.send(Err("DistributeModel not yet implemented".to_string()));
            }
            UserCommand::Send { file_path, peer_id, reply } => {
                let job_id = JobId(generate_id());
                // 编译发送作业
                let mut vars = HashMap::new();
                vars.insert("file_path".to_string(), file_path.clone());
                vars.insert("peer_id".to_string(), peer_id.clone());
                let program = match ProgramSelector::select(JobKind::Send, job_id, vars) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = reply.send(Err(format!("编译失败: {:?}", e)));
                        return;
                    }
                };
                // Send Job 不使用 ML 推理，无需 IO 通道
                self.spawn_job(job_id, JobKind::Send, program, None);
                let _ = reply.send(Ok(job_id));
            }
            UserCommand::List { reply } => {
                // 1. flush — 同步磁盘，确保索引与磁盘一致
                match self.capabilities.storage.flush().await {
                    Ok((discovered, cleaned)) => {
                        info!("Storage flush 完成: 新发现 {} 个文件, 清理 {} 个僵尸条目", discovered, cleaned);
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("flush 失败: {}", e)));
                        return;
                    }
                }
                // 2. list — 获取文件列表
                match self.capabilities.storage.list().await {
                    Ok(files) => {
                        let _ = reply.send(Ok(files));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("list 失败: {}", e)));
                    }
                }
            }
            UserCommand::Pipeline { model_path, reply } => {
                // Phase 0: 生成全局唯一 inference_id + 规划拓扑
                let local_peer_id = self.capabilities.network.get_local_peer_id();
                let inference_id = Generate_Inference_Id(&local_peer_id);
                let job_id = JobId(generate_id());
                let device = if self.device_preference.is_empty() { "cpu".to_string() } else { self.device_preference.clone() };

                info!(
                    "UserCommand::Pipeline: inference_id={}, job_id={:?}, model={}, device={}",
                    inference_id, job_id, model_path, device
                );

                // 编译 Pipeline TaskProgram
                let mut vars = HashMap::new();
                vars.insert("model_path".to_string(), model_path.clone());
                vars.insert("inference_id".to_string(), inference_id.to_string());
                vars.insert("device".to_string(), device.clone());
                let program = match ProgramSelector::select(JobKind::Pipeline, job_id, vars) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = reply.send(Err(format!("编译失败: {:?}", e)));
                        return;
                    }
                };

                // Pipeline Job 需要 IO 通道（Coordinator 推理需要与前端交互）
                if let Err(e) = self.capabilities.io_broker.Allocate(job_id).await {
                    let _ = reply.send(Err(format!("IO 分配失败: {}", e)));
                    return;
                }
                match self.capabilities.io_broker.Take_ML_Side(job_id).await {
                    Ok(io) => {
                        // 发布 Job 创建事件
                        self.capabilities.event_bus.Publish(Bus_Event::Job_Created {
                            job_id: job_id.0,
                            kind: format!("{:?}", JobKind::Pipeline),
                            model_name: String::new(),
                        });

                        let cancel = CancellationToken::new();
                        let executor = JobExecutor::new(
                            job_id,
                            JobKind::Pipeline,
                            program,
                            cancel.clone(),
                            self.capabilities.clone(),
                            Some(io),
                            self.lifecycle_tx.clone(),
                        );
                        tokio::spawn(executor.run());
                        self.registry.insert(job_id, JobHandle {
                            kind: JobKind::Pipeline,
                            cancel,
                            inference_id: Some(inference_id),
                        });
                        let _ = reply.send(Ok(job_id));
                    }
                Err(e) => {
                        let _ = reply.send(Err(format!("IO Take_ML_Side 失败: {}", e)));
                    }
                }
            }
            UserCommand::Profile { model_id, reply } => {
                let job_id = JobId(generate_id());
                let device = if self.device_preference.is_empty() { "cpu".to_string() } else { self.device_preference.clone() };

                let mut vars = HashMap::new();
                vars.insert("model_path".to_string(), model_id.clone());
                vars.insert("device".to_string(), device);
                vars.insert("layer_start".to_string(), "1".to_string());
                vars.insert("layer_end".to_string(), "5".to_string());
                let program = match ProgramSelector::select(JobKind::Profile, job_id, vars) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = reply.send(Err(format!("编译失败: {:?}", e)));
                        return;
                    }
                };

                self.capabilities.event_bus.Publish(Bus_Event::Job_Created {
                    job_id: job_id.0,
                    kind: format!("{:?}", JobKind::Profile),
                    model_name: model_id.clone(),
                });

                // Profile 不需要文本 IO，但 CreateSession 要求 IO 槽位存在
                if let Err(e) = self.capabilities.io_broker.Allocate(job_id).await {
                    let _ = reply.send(Err(format!("IO 分配失败: {}", e)));
                    return;
                }
                match self.capabilities.io_broker.Take_ML_Side(job_id).await {
                    Ok(io) => {
                        let cancel = CancellationToken::new();
                        let executor = JobExecutor::new(
                            job_id,
                            JobKind::Profile,
                            program,
                            cancel.clone(),
                            self.capabilities.clone(),
                            Some(io),
                            self.lifecycle_tx.clone(),
                        );
                        tokio::spawn(executor.run());
                        self.registry.insert(job_id, JobHandle {
                            kind: JobKind::Profile,
                            cancel,
                            inference_id: None,
                        });
                        let _ = reply.send(Ok(job_id));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("IO Take_ML_Side 失败: {}", e)));
                    }
                }
            }
        }
    }
}
