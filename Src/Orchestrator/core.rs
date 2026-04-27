// Presented by KeJi
// Date ： 2026-04-24

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use super::job::{JobId, JobKind, LifecycleEvent};
use super::command::{UserCommand, NetworkCommand};
use super::executor::JobExecutor;
use super::compiler::Compiler;
use super::Capabilities;
use crate::llm_io::{IoHandle, LLM_IO_Capability};
use crate::network::Network_Inbound_Event;

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

/// Orchestrator Core 结构体（简化方案）
pub struct Core {
    // 内核状态
    registry: HashMap<JobId, JobHandle>,
    shutting_down: bool,
    
    // 路由工具（原 Portal 的职责）
    compiler: Arc<Compiler>,
    capabilities: Arc<Capabilities>,
    
    // 通道
    user_cmd_rx: mpsc::Receiver<UserCommand>,
    network_cmd_rx: mpsc::Receiver<NetworkCommand>,
    network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    lifecycle_rx: mpsc::Receiver<LifecycleEvent>,
}

impl Core {
    /// 创建新的 Core 实例
    ///
    /// lifecycle 通道由 Core 内部创建，不需要外部传入。
    /// JobExecutor 通过 lifecycle_tx 的克隆向 Core 报告生命周期事件。
    pub fn new(
        compiler: Arc<Compiler>,
        capabilities: Arc<Capabilities>,
        user_cmd_rx: mpsc::Receiver<UserCommand>,
        network_cmd_rx: mpsc::Receiver<NetworkCommand>,
        network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,
    ) -> Self {
        let (lifecycle_tx, lifecycle_rx) = mpsc::channel(LIFECYCLE_CHANNEL_BUFFER);
        Core {
            registry: HashMap::new(),
            shutting_down: false,
            compiler,
            capabilities,
            user_cmd_rx,
            network_cmd_rx,
            network_inbound_rx,
            lifecycle_tx,
            lifecycle_rx,
        }
    }

    /// 主循环
    pub async fn run(mut self) {
        loop {
            if self.shutting_down && self.registry.is_empty() {
                break;  // 所有作业完成，安全退出
            }
            tokio::select! {
                // 用户命令
                Some(cmd) = self.user_cmd_rx.recv(), if !self.shutting_down => {
                    self.route_user(cmd).await;
                }
                // 网络命令
                Some(cmd) = self.network_cmd_rx.recv(), if !self.shutting_down => {
                    self.route_network(cmd).await;
                }
                // Network 转发的入站事件（文件流、张量流）
                Some(event) = self.network_inbound_rx.recv(), if !self.shutting_down => {
                    self.handle_network_inbound(event).await;
                }
                // 生命周期事件
                Some(event) = self.lifecycle_rx.recv() => {
                    self.handle_lifecycle_event(event);  // 同步调用
                }
            }
        }
    }

    /// 路由用户命令（异步，因需要调用 io_broker.Allocate）
    async fn route_user(&mut self, cmd: UserCommand) {
        match cmd {
            UserCommand::Run { model_path } => {
                let job_id = JobId(generate_id());
                // 先编译，失败则提前返回（不分配 IO 通道）
                let program = match self.compiler.compile_run(job_id, model_path, None) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("编译失败: {:?}", e);
                        return;
                    }
                };
                // 编译成功后再分配 IO 通道
                match self.capabilities.io_broker.Allocate(job_id).await {
                    Ok(channels) => {
                        self.spawn_job(job_id, JobKind::Run, program, channels.ml_side);
                    }
                    Err(e) => {
                        eprintln!("IO 分配失败: {}", e);
                    }
                }
            }
            UserCommand::Cancel { job_id } => {
                self.cancel_job(job_id);
            }
            UserCommand::Quit => {
                self.shutdown();
            }
            UserCommand::DisplayPeer => {
                // TODO: 通过 PeerManager Capability 查询节点列表
                warn!("DisplayPeer 尚未对接 PeerManager Capability");
            }
            UserCommand::SetDevice { device } => {
                self.capabilities.set_compute_preference(device);
            }
        }
    }

    /// 路由网络命令（异步，因需要调用 io_broker.Allocate）
    async fn route_network(&mut self, cmd: NetworkCommand) {
        match cmd {
            NetworkCommand::PipelineFlow { peer_id } => {
                let job_id = JobId(generate_id());
                // 先编译，失败则提前返回
                let program = match self.compiler.compile_relay(job_id, peer_id, None) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("编译失败: {:?}", e);
                        return;
                    }
                };
                // 编译成功后再分配 IO 通道
                match self.capabilities.io_broker.Allocate(job_id).await {
                    Ok(channels) => {
                        self.spawn_job(job_id, JobKind::Relay, program, channels.ml_side);
                    }
                    Err(e) => {
                        eprintln!("IO 分配失败: {}", e);
                    }
                }
            }
        }
    }

    /// 将已编译的程序 spawn 为独立的 Job 任务。
    /// 职责单一：仅负责创建 Executor 并注册到 registry。
    fn spawn_job(&mut self, job_id: JobId, kind: JobKind, program: super::instruction::TaskProgram, io: IoHandle) {
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
    /// - TensorStreamArrived: 保存入站 tensor stream 供后续 Job 使用
    async fn handle_network_inbound(&mut self, event: Network_Inbound_Event) {
        match event {
            Network_Inbound_Event::FileStreamArrived { peer, stream: _ } => {
                // TODO: 解析 pending_file_receives 中的元数据，compile 接收作业并 spawn Job
                info!("收到入站文件流 from {}, 待实现接收作业 spawn", peer);
            }
            Network_Inbound_Event::TensorStreamArrived { peer, stream: _ } => {
                // TODO: 保存入站 tensor stream，供后续 pipeline Job 使用
                info!("收到入站张量流 from {}, 待实现 stream 注入", peer);
            }
        }
    }

    fn handle_lifecycle_event(&mut self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::Done { job_id, .. } => {
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
    use crate::orchestrator::{UiCapability, Capabilities};
    use crate::orchestrator::test_utils::StubNetwork;
    use crate::network::Network_Inbound_Event;
    use crate::storage::StorageManager;
    use crate::llm_io::{LLM_IO_Broker, LLM_IO_Capability};
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
            ui: UiCapability,
            io_broker: LLM_IO_Broker::New(),
        });
        (caps, temp_dir)
    }

    /// 创建一个用于测试的 Core 实例
    async fn create_test_core() -> (Core, TempDir) {
        let (caps, temp_dir) = stub_capabilities().await;
        let (_user_tx, user_rx) = mpsc::channel(16);
        let (_net_tx, net_rx) = mpsc::channel(16);
        let (_inbound_tx, inbound_rx) = mpsc::channel::<Network_Inbound_Event>(16);
        let core = Core::new(Arc::new(Compiler), caps, user_rx, net_rx, inbound_rx);
        (core, temp_dir)
    }

    /// 通过 LLM_IO_Broker 分配一个 IoHandle（ML 侧端点）
    async fn stub_io_handle(broker: &LLM_IO_Broker, job_id: JobId) -> IoHandle {
        broker.Allocate(job_id).await.unwrap().ml_side
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
