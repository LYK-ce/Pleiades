// Presented by KeJi
// Date ： 2026-04-29

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use libp2p::PeerId;
use super::job::{JobId, JobKind, JobResult, LifecycleEvent};
use super::command::{UserCommand, NetworkProtocol, Parse_Network_Command};
use super::executor::JobExecutor;
use super::compiler::Compiler;
use super::Capabilities;
use crate::llm_io::{IoHandle, LLM_IO_Capability};
use crate::network::{Network_Inbound_Event, InboundRequest, DataType};
use crate::network::tensor_stream_protocol::Read_Tensor_Stream_Handshake;
use crate::config::Update_Config;
use crate::event_bus::Bus_Event;
use crate::storage::StorageCapability;

/// 生命周期通道缓冲大小
const LIFECYCLE_CHANNEL_BUFFER: usize = 64;

/// Portal 错误类型
#[derive(Debug, thiserror::Error)]
pub enum PortalError {
    #[error("路由错误: {0}")]
    Routing(String),
    #[error("编译失败: {0}")]
    Compilation(String),
    #[error("内部错误: {0}")]
    Internal(String),
}

/// Core 内部使用的 Job 句柄
struct JobHandle {
    kind: JobKind,
    cancel: CancellationToken,
}

/// 文件接收元数据（B2 DataType::File 入站存入，B3 FileStreamArrived 消费）
///
/// 当前仅记录文件名和大小。
/// 文件校验由独立的 VERIFY_FILE 命令（Phase 3）处理，不在此阶段传入 checksum。
struct FileMetadata {
    file_name: String,
    file_size: u64,
}

/// Orchestrator Core 结构体
///
/// ## select! 分支总览
/// | # | 通道 | 来源 | 处理方法 |
/// |---|------|------|---------|
/// | B1 | `user_cmd_rx` | TUI / CLI | `route_user()` |
/// | B2 | `inbound_rx` | Network Request-Response（仅 Command + File） | `handle_inbound_request()` |
/// | B3 | `network_inbound_rx` | Network Stream（文件流 + 张量流） | `handle_network_inbound()` |
/// | B4 | `lifecycle_rx` | JobExecutor | `handle_lifecycle_event()` |
pub struct Core {
    // --- 内核状态 ---
    registry: HashMap<JobId, JobHandle>,
    shutting_down: bool,

    // --- 路由工具 ---
    compiler: Arc<Compiler>,
    capabilities: Arc<Capabilities>,
    config_path: PathBuf,

    // --- B1: 用户命令 ---
    user_cmd_rx: mpsc::Receiver<UserCommand>,

    // --- B2: Request-Response 入站（仅 Command + File） ---
    inbound_rx: mpsc::Receiver<InboundRequest>,

    // --- B3: Stream 入站 ---
    network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,

    // --- B4: 生命周期 ---
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    lifecycle_rx: mpsc::Receiver<LifecycleEvent>,

    // --- B2/B3 共享状态 ---
    pending_file_receives: HashMap<PeerId, Vec<FileMetadata>>,

    // --- SetDevice ---
    device_preference: String,
}

impl Core {
    /// 创建新的 Core 实例
    ///
    /// lifecycle 通道由 Core 内部创建，不需要外部传入。
    /// JobExecutor 通过 lifecycle_tx 的克隆向 Core 报告生命周期事件。
    pub fn new(
        compiler: Arc<Compiler>,
        capabilities: Arc<Capabilities>,
        config_path: PathBuf,
        user_cmd_rx: mpsc::Receiver<UserCommand>,
        inbound_rx: mpsc::Receiver<InboundRequest>,
        network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,
    ) -> Self {
        let (lifecycle_tx, lifecycle_rx) = mpsc::channel(LIFECYCLE_CHANNEL_BUFFER);
        Core {
            registry: HashMap::new(),
            shutting_down: false,
            compiler,
            capabilities,
            config_path,
            user_cmd_rx,
            inbound_rx,
            network_inbound_rx,
            lifecycle_tx,
            lifecycle_rx,
            pending_file_receives: HashMap::new(),
            device_preference: String::new(),
        }
    }

    /// 主循环
    ///
    /// 4 个 select! 分支，每轮只处理一个事件。
    /// 各分支具体跳转操作使用 TODO 占位，后续逐个接入。
    pub async fn run(mut self) {
        loop {
            if self.shutting_down && self.registry.is_empty() {
                break;  // 所有作业完成，安全退出
            }
            tokio::select! {
                // B1: 用户命令
                Some(cmd) = self.user_cmd_rx.recv(), if !self.shutting_down => {
                    self.route_user(cmd).await;
                }
                // B2: Request-Response 入站请求（仅 Command + File）
                Some(req) = self.inbound_rx.recv(), if !self.shutting_down => {
                    self.handle_inbound_request(req).await;
                }
                // B3: Network 转发的 Stream 入站事件（文件流 + 张量流）
                Some(event) = self.network_inbound_rx.recv(), if !self.shutting_down => {
                    // TODO: self.handle_network_inbound(event).await;
                    let _ = event;  // 临时消耗，防止未使用变量警告
                }
                // B4: 生命周期事件（始终活跃）
                Some(event) = self.lifecycle_rx.recv() => {
                    self.handle_lifecycle_event(event);
                }
            }
        }
    }

    /// 处理 B2 入站请求（Request-Response 协议，仅 Command + File）
    ///
    /// Network_Service 按 DataType 预筛选后，仅将 Command 和 File 类型转发到此方法。
    /// 每个请求携带 request_id，处理完后必须通过 network.send_response(request_id, ...) 回复。
    async fn handle_inbound_request(&mut self, req: InboundRequest) {
        match req.data_type {
            DataType::File => {
                self.handle_network_file(req).await;
            }
            DataType::Command => {
                self.handle_network_command(req).await;
            }
            _ => {
                // BandwidthTest / Data / Info 不应到达此处（已被 Network 内部处理）
                warn!("B2: 收到非预期的 DataType {:?} from {}, request_id={}", req.data_type, req.peer, req.request_id);
            }
        }
    }

    /// 处理 DataType::File 入站请求（文件元数据协商，阶段1 接收侧）
    ///
    /// 发送方 handle_send_file() 阶段1 发送 `file_name|file_size`，
    /// 接收方解析后检查存储空间，存入 `pending_file_receives`，回复 ACCEPT 或 REJECT。
    ///
    /// ## 处理流程
    /// 1. 解析 payload 为 `file_name|file_size`
    /// 2. 查询 `storage.quota_info()` 检查可用空间是否 >= file_size
    /// 3. 空间充足 → 存入 `pending_file_receives[peer]` → 回复 ACCEPT
    /// 4. 空间不足 → 回复 `REJECT|insufficient storage space`
    ///
    /// ## 注意
    /// - 此阶段仅做空间检查，不预留空间（`reserve()` 在 B3 FileStreamArrived 真正接收时调用）
    /// - 文件校验由独立的 VERIFY_FILE 命令（Phase 3）处理
    /// - 未来可增加防重复接收检查
    async fn handle_network_file(&mut self, req: InboundRequest) {
        // 1. 解析 payload: "file_name|file_size"
        let payload_str = match std::str::from_utf8(&req.payload) {
            Ok(s) => s,
            Err(e) => {
                warn!("B2/File: payload 非 UTF-8 from {}: {}", req.peer, e);
                let _ = self.capabilities.network.send_response(
                    req.request_id, DataType::Command, b"REJECT|payload not UTF-8".to_vec()
                ).await;
                return;
            }
        };

        let parts: Vec<&str> = payload_str.split('|').collect();
        if parts.len() != 2 {
            warn!("B2/File: payload 格式错误 from {}: 需要 2 字段(file_name|file_size), 实际 {}", req.peer, parts.len());
            let _ = self.capabilities.network.send_response(
                req.request_id, DataType::Command, b"REJECT|invalid format".to_vec()
            ).await;
            return;
        }

        let file_name = parts[0].to_string();
        let file_size = match parts[1].parse::<u64>() {
            Ok(s) => s,
            Err(e) => {
                warn!("B2/File: file_size 解析失败 from {}: {}", req.peer, e);
                let _ = self.capabilities.network.send_response(
                    req.request_id, DataType::Command, b"REJECT|invalid file_size".to_vec()
                ).await;
                return;
            }
        };

        // 2. 检查存储空间是否充足
        let quota = self.capabilities.storage.quota_info().await;
        if quota.total > 0 && file_size > quota.available {
            warn!(
                "B2/File: 存储空间不足 from {}: 请求 {} bytes, 可用 {} bytes",
                req.peer, file_size, quota.available
            );
            let reject_msg = format!("REJECT|insufficient storage space: need {} bytes, available {} bytes", file_size, quota.available);
            let _ = self.capabilities.network.send_response(
                req.request_id, DataType::Command, reject_msg.into_bytes()
            ).await;
            return;
        }

        // 3. 存入 pending_file_receives（供 B3 FileStreamArrived 消费）
        let metadata = FileMetadata { file_name: file_name.clone(), file_size };
        self.pending_file_receives
            .entry(req.peer)
            .or_insert_with(Vec::new)
            .push(metadata);

        info!("B2/File: 接受文件元数据 from {}: name={}, size={}, request_id={}", req.peer, file_name, file_size, req.request_id);

        // 4. 回复 ACCEPT
        if let Err(e) = self.capabilities.network.send_response(
            req.request_id, DataType::Command, b"ACCEPT".to_vec()
        ).await {
            warn!("B2/File: send_response(ACCEPT) 失败: {}", e);
        }
    }

    /// 处理 DataType::Command 入站请求（命令分发）
    ///
    /// 通过 `Parse_Network_Command` 解析为 `NetworkProtocol` 枚举，
    /// 再按变体分发到对应处理方法。
    async fn handle_network_command(&mut self, req: InboundRequest) {
        let protocol = match Parse_Network_Command(&req.payload) {
            Ok(p) => p,
            Err(e) => {
                warn!("B2/Command: 协议解析失败 from {}, request_id={}: {}", req.peer, req.request_id, e);
                // 回复解析错误
                if let Err(send_err) = self.capabilities.network.send_response(
                    req.request_id, DataType::Command, format!("REJECT|{}", e).into_bytes()
                ).await {
                    warn!("send_response 失败: {}", send_err);
                }
                return;
            }
        };

        match protocol {
            NetworkProtocol::Request_Pipeline {
                coordinator_job_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            } => {
                // TODO: 获取 coordinator_peer_id（当前 req.peer 即为 coordinator）
                //       调用 route_pipeline_flow → 回复 OK|job_id 或 REJECT|reason
                info!(
                    "B2/Command: REQUEST_PIPELINE from {}, coordinator_job_id={}, model={}, device={}, layers={}-{}, request_id={}, 待实现",
                    req.peer, coordinator_job_id, model_file_id, device, layer_start, layer_end, req.request_id
                );
            }
            NetworkProtocol::Verify_File { file_name } => {
                // TODO: 查 StorageManager 确认文件存在
                //       → send_response(confirmed / failed|reason)
                info!(
                    "B2/Command: VERIFY_FILE from {}, file={}, request_id={}, 待实现",
                    req.peer, file_name, req.request_id
                );
            }
        }
    }

    /// 路由用户命令（异步，因需要调用 io_broker.Allocate）
    ///
    /// 每个分支处理完后通过 `reply` 通道回传结果给前端。
    async fn route_user(&mut self, cmd: UserCommand) {
        match cmd {
            UserCommand::Run { model_path, reply } => {
                let job_id = JobId(generate_id());
                // 使用 Core 的 device_preference（空字符串时传 None，由 Compiler 默认 "cpu"）
                let device = if self.device_preference.is_empty() { None } else { Some(self.device_preference.clone()) };
                // 先编译，失败则回传错误
                let program = match self.compiler.compile_run(job_id, model_path, device) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = reply.send(Err(format!("编译失败: {:?}", e)));
                        return;
                    }
                };
                // 编译成功后分配 IO 通道
                if let Err(e) = self.capabilities.io_broker.Allocate(job_id).await {
                    let _ = reply.send(Err(format!("IO 分配失败: {}", e)));
                    return;
                }
                match self.capabilities.io_broker.Take_ML_Side(job_id).await {
                    Ok(io) => {
                        self.spawn_job(job_id, JobKind::Run, program, io);
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
                            format!("{} [{:?}] lat={:?}ms bw={:?}Mbps",
                                p.peer_id, p.status, p.latency_ms, p.bandwidth_mbps)
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
            UserCommand::DistributeModel { model_path, peers, reply } => {
                let job_id = JobId(generate_id());
                // 编译分发作业
                let program = match self.compiler.compile_distribute(job_id, model_path, peers) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = reply.send(Err(format!("编译失败: {:?}", e)));
                        return;
                    }
                };
                // 分配 IO 通道（Distribute Job 不使用 ML 推理，但保持接口一致）
                if let Err(e) = self.capabilities.io_broker.Allocate(job_id).await {
                    let _ = reply.send(Err(format!("IO 分配失败: {}", e)));
                    return;
                }
                match self.capabilities.io_broker.Take_ML_Side(job_id).await {
                    Ok(io) => {
                        self.spawn_job(job_id, JobKind::Distribute, program, io);
                        let _ = reply.send(Ok(job_id));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("IO Take_ML_Side 失败: {}", e)));
                    }
                }
            }
        }
    }

    /// 路由 Pipeline Flow 请求（远端 Coordinator 请求本节点作为 Worker 加入流水线）
    ///
    /// 编译 Relay Job → 分配 IO → 预注册 Tensor_IO_Broker → spawn Job。
    /// 由 handle_network_command() 中的 REQUEST_PIPELINE 分支调用。
    async fn route_pipeline_flow(
        &mut self,
        coordinator_peer_id: String,
        coordinator_job_id: u64,
        model_file_id: String,
        device: String,
        layer_start: usize,
        layer_end: usize,
    ) -> Result<JobId, String> {
        let job_id = JobId(generate_id());
        // 先编译
        let program = self.compiler.compile_relay(
            job_id,
            coordinator_peer_id,
            coordinator_job_id,
            model_file_id,
            Some(device),
            layer_start,
            layer_end,
        ).map_err(|e| format!("编译失败: {:?}", e))?;

        // 分配 IO 通道
        self.capabilities.io_broker.Allocate(job_id).await
            .map_err(|e| format!("IO 分配失败: {}", e))?;

        let io = self.capabilities.io_broker.Take_ML_Side(job_id).await
            .map_err(|e| format!("IO Take_ML_Side 失败: {}", e))?;

        // 分布式 Job 需要预注册 Tensor_IO_Broker
        if let Err(e) = self.capabilities.tensor_io_broker.Prepare(job_id).await {
            warn!("Tensor_IO_Broker Prepare 失败: {}", e);
        }

        self.spawn_job(job_id, JobKind::Relay, program, io);
        Ok(job_id)
    }

    /// 将已编译的程序 spawn 为独立的 Job 任务。
    /// 职责单一：仅负责创建 Executor 并注册到 registry。
    fn spawn_job(&mut self, job_id: JobId, kind: JobKind, program: super::instruction::TaskProgram, io: IoHandle) {
        // 发布 Job 创建事件
        self.capabilities.event_bus.Publish(Bus_Event::Job_Created {
            job_id: job_id.0,
            kind: format!("{:?}", kind),
            model_name: String::new(),
        });

        let cancel = CancellationToken::new();
        
        // 创建 Executor，clone lifecycle_tx 供 Executor 回报生命周期事件
        let executor = JobExecutor::new(
            job_id,
            kind,
            program,
            cancel.clone(),
            self.capabilities.clone(),
            io,
            self.lifecycle_tx.clone(),
        );
        tokio::spawn(executor.run());  // 同步 spawn，立即返回
        
        self.registry.insert(job_id, JobHandle { kind, cancel });
    }

    fn cancel_job(&mut self, job_id: JobId) {
        if let Some(handle) = self.registry.get(&job_id) {
            handle.cancel.cancel();  // 同步发信号
        }
    }

    fn shutdown(&mut self) {
        self.shutting_down = true;
        for handle in self.registry.values() {
            handle.cancel.cancel();  // 同步发信号
        }
    }

    /// 处理 Network 转发的入站事件
    ///
    /// - FileStreamArrived: compile 接收作业 → spawn Job（stream 存入 SlotFile）
    /// - TensorStreamArrived: 读取 handshake → 通过 Tensor_IO_Broker 路由到目标 Job
    async fn handle_network_inbound(&mut self, event: Network_Inbound_Event) {
        match event {
            Network_Inbound_Event::FileStreamArrived { peer, stream: _ } => {
                // TODO: 解析 pending_file_receives 中的元数据，compile 接收作业并 spawn Job
                info!("收到入站文件流 from {}, 待实现接收作业 spawn", peer);
            }
            Network_Inbound_Event::TensorStreamArrived { peer, mut stream } => {
                // 1. 从 stream 读取 handshake 帧（target_job_id）
                match Read_Tensor_Stream_Handshake(&mut stream).await {
                    Ok(target_job_id) => {
                        let job_id = super::job::JobId(target_job_id);
                        info!("收到入站张量流 from {}, target_job_id={}, 路由到 Broker", peer, target_job_id);
                        // 2. 通过 Broker 路由到目标 Job
                        if let Err(e) = self.capabilities.tensor_io_broker.Store_Inbound(job_id, stream).await {
                            warn!("Tensor_IO_Broker Store_Inbound 失败: {}", e);
                        }
                    }
                    Err(e) => {
                        warn!("读取张量流 handshake 失败 from {}: {}", peer, e);
                    }
                }
            }
        }
    }

    fn handle_lifecycle_event(&mut self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::Done { job_id, result } => {
                // 发布 Job 完成事件
                let result_str = match &result {
                    JobResult::Success => "Success".to_string(),
                    JobResult::Cancelled => "Cancelled".to_string(),
                    JobResult::Failed(e) => format!("Failed: {}", e),
                };
                self.capabilities.event_bus.Publish(Bus_Event::Job_Completed {
                    job_id: job_id.0,
                    result: result_str,
                });

                // 清理 Tensor_IO_Broker 条目（幂等，对非分布式 Job 无影响）
                let caps = self.capabilities.clone();
                let job = job_id;
                // 使用 tokio::spawn 异步清理，避免在同步方法中 .await
                tokio::spawn(async move {
                    caps.tensor_io_broker.Deallocate(job).await;
                });
                self.registry.remove(&job_id);  // 同步 HashMap 操作
            }
        }
    }
}

/// 生成唯一 ID
static JOB_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_id() -> u64 {
    JOB_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod core_tests {
    use super::*;
    use crate::orchestrator::job::{JobId, JobKind, JobResult};
    use crate::orchestrator::instruction::{TaskInstruction, TaskProgram};
    use crate::orchestrator::slot::{SlotId, ConstValue};
    use crate::orchestrator::Capabilities;
    use crate::orchestrator::test_utils::{StubNetwork, StubPeerManager};
    use crate::network::{Network_Inbound_Event, InboundRequest};
    use crate::storage::StorageManager;
    use crate::llm_io::{LLM_IO_Broker, LLM_IO_Capability};
    use crate::orchestrator::tensor_io_broker::Tensor_IO_Broker;
    use crate::event_bus::EventBus;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Pipeline_Params, Pipeline_Result, Model_Info};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;
    use tokio::time::{timeout, Duration};

    // ---- Stub 实现 ----

    struct StubMLEngine;
    #[async_trait]
    impl ML_Engine_Capability for StubMLEngine {
        async fn Create_Session(&self, _config: ML_Session_Config, _io_handle: crate::llm_io::IoHandle) -> Result<Model_Info, ML_Engine_Error> { unimplemented!("stub") }
        async fn Shutdown_Session(&self, _session_id: &str) -> Result<(), ML_Engine_Error> { unimplemented!("stub") }
        async fn Run_Program(&self, _session_id: &str, _program: Vec<Instruction>, _params: Pipeline_Params, _cancel_flag: Arc<AtomicBool>) -> Result<Pipeline_Result, ML_Engine_Error> { unimplemented!("stub") }
        async fn Analyze_Model(&self, _model_file_id: &str) -> Result<Model_Info, ML_Engine_Error> { unimplemented!("stub") }
        async fn Split_Model(&self, _source_file_id: &str, _start: usize, _end: usize, _output_file_id: &str) -> Result<(), ML_Engine_Error> { unimplemented!("stub") }
    }

    /// 创建 Stub Capabilities + TempDir（TempDir 必须保持存活以维持临时目录）
    async fn stub_capabilities() -> (Arc<Capabilities>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::New(temp_dir.path()).await.unwrap();
        let caps = Arc::new(Capabilities {
            storage,
            ml_engine: Box::new(StubMLEngine),
            network: Box::new(StubNetwork),
            peer_manager: Box::new(StubPeerManager),
            event_bus: Arc::new(EventBus::New(16)),
            io_broker: LLM_IO_Broker::New(),
            tensor_io_broker: Tensor_IO_Broker::New(),
        });
        (caps, temp_dir)
    }

    /// 创建一个用于测试的 Core 实例
    async fn create_test_core() -> (Core, TempDir) {
        let (caps, temp_dir) = stub_capabilities().await;
        let (_user_tx, user_rx) = mpsc::channel(16);
        let (_inbound_tx, inbound_rx) = mpsc::channel::<InboundRequest>(16);
        let (_net_inbound_tx, net_inbound_rx) = mpsc::channel::<Network_Inbound_Event>(16);
        // 测试用 config_path 指向 temp_dir 内的虚拟路径（SetDevice 测试不实际写盘）
        let config_path = temp_dir.path().join("config.toml");
        let core = Core::new(Arc::new(Compiler), caps, config_path, user_rx, inbound_rx, net_inbound_rx);
        (core, temp_dir)
    }

    /// 通过 LLM_IO_Broker 分配一个 IoHandle（ML 侧端点）
    async fn stub_io_handle(broker: &LLM_IO_Broker, job_id: JobId) -> IoHandle {
        broker.Allocate(job_id).await.unwrap();
        broker.Take_ML_Side(job_id).await.unwrap()
    }

    // ---- TC-01: spawn 简单 Const Job → Done/Success ----

    #[tokio::test]
    async fn test_spawn_simple_job_success() {
        let (mut core, _temp_dir) = create_test_core().await;
        let job_id = JobId(1001);
        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Const {
                    value: ConstValue::U64(42),
                    dst: SlotId(0),
                },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        };
        let io = stub_io_handle(&core.capabilities.io_broker, job_id).await;

        // spawn job
        core.spawn_job(job_id, JobKind::Run, program, io);

        // 验证 registry 已注册
        assert!(core.registry.contains_key(&job_id));

        // 等待 LifecycleEvent::Done
        let event = timeout(Duration::from_secs(2), core.lifecycle_rx.recv())
            .await
            .expect("timeout waiting for lifecycle event")
            .expect("lifecycle channel closed");

        match event {
            LifecycleEvent::Done { job_id: recv_id, result } => {
                assert_eq!(recv_id, job_id);
                assert!(matches!(result, JobResult::Success));
            }
        }

        // 处理 lifecycle 事件后，registry 应清空
        core.handle_lifecycle_event(LifecycleEvent::Done {
            job_id,
            result: JobResult::Success,
        });
        assert!(!core.registry.contains_key(&job_id));
    }

    // ---- TC-02: spawn Abort Job → Done/Failed ----

    #[tokio::test]
    async fn test_spawn_abort_job_failed() {
        let (mut core, _temp_dir) = create_test_core().await;
        let job_id = JobId(1002);
        let program = TaskProgram {
            instructions: vec![
                TaskInstruction::Abort {
                    reason: "test abort reason".to_string(),
                },
            ],
            compensation: vec![],
            labels: HashMap::new(),
        };
        let io = stub_io_handle(&core.capabilities.io_broker, job_id).await;

        core.spawn_job(job_id, JobKind::Run, program, io);

        let event = timeout(Duration::from_secs(2), core.lifecycle_rx.recv())
            .await
            .expect("timeout waiting for lifecycle event")
            .expect("lifecycle channel closed");

        match event {
            LifecycleEvent::Done { job_id: recv_id, result } => {
                assert_eq!(recv_id, job_id);
                match result {
                    JobResult::Failed(reason) => {
                        assert!(reason.contains("test abort reason"));
                    }
                    _ => panic!("expected Failed, got {:?}", result),
                }
            }
        }
    }

    // TC-03（编译失败测试）已移除：编译错误现在在 route_user/route_network 中处理，
    // spawn_job 仅接受已编译的 TaskProgram。编译失败测试将在 Compiler 实现后通过端到端测试覆盖。

    // ---- TC-03: spawn 多个 Job → 全部收到 Done 事件 ----

    #[tokio::test]
    async fn test_spawn_multiple_jobs_all_done() {
        let (mut core, _temp_dir) = create_test_core().await;

        let job_ids = vec![JobId(1004), JobId(1005), JobId(1006)];

        for &jid in &job_ids {
            let program = TaskProgram {
                instructions: vec![
                    TaskInstruction::Const {
                        value: ConstValue::String(format!("job-{}", jid.0)),
                        dst: SlotId(0),
                    },
                ],
                compensation: vec![],
                labels: HashMap::new(),
            };
            let io = stub_io_handle(&core.capabilities.io_broker, jid).await;
            core.spawn_job(jid, JobKind::Run, program, io);
        }

        // 验证全部注册
        assert_eq!(core.registry.len(), 3);

        // 收集所有 Done 事件
        let mut received_ids = Vec::new();
        for _ in 0..3 {
            let event = timeout(Duration::from_secs(2), core.lifecycle_rx.recv())
                .await
                .expect("timeout waiting for lifecycle event")
                .expect("lifecycle channel closed");

            match event {
                LifecycleEvent::Done { job_id, result } => {
                    assert!(matches!(result, JobResult::Success));
                    // 处理事件，从 registry 移除
                    core.registry.remove(&job_id);
                    received_ids.push(job_id);
                }
            }
        }

        // 验证收到了所有 3 个 Job 的 Done 事件
        assert_eq!(received_ids.len(), 3);
        for jid in &job_ids {
            assert!(received_ids.contains(jid));
        }

        // registry 应清空
        assert!(core.registry.is_empty());
    }
}
