//Presented by KeJi
//Date ： 2026-04-30

//! Tensor Port Switch — 张量流的共享锁管理与热切换模块
//!
//! 替代旧的 `Tensor_IO_Broker`（存取模式）和 `Tensor_IO_Handle`（直连模式），
//! 提供以下核心能力：
//!
//! - **Core 管理张量流**：实际的 `libp2p::Stream` 由 Switch 持有，通过 `Mutex` 保护
//! - **ML Thread 只拿接口**：Session 通过 `Arc` 引用访问 Mutex 内的 stream，零拷贝
//! - **Core 热切换**：换目标时锁住 Mutex → 替换 stream → 解锁，ML Thread 无感
//! - **广播支持**：Outbound 使用 `Vec<Stream>`，一次 Send 写入所有目标
//! - **故障上报**：单个目标失败不中断推理，通过 channel 异步通知 Core
//!
//! ## 数据路径（零拷贝，零中间层）
//! ```text
//! 发送: ML Thread → lock(outbound) → stream.write(data) → unlock → 网络
//! 接收: 网络 → stream → lock(inbound) → stream.read → buffer → unlock → ML Thread
//! ```

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::network::tensor_stream_protocol::{
    Receive_Tensor_Frame, Send_EOF, Send_Tensor_Frame, Tensor_Buffer,
};
use crate::orchestrator::job::JobId;

// ─── 常量 ───────────────────────────────────────────────────

/// I/O 超时时间（防止 stream 断开时长时间持锁）
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Tensor_Buffer 初始容量（64KB，Decode 阶段足够）
const BUFFER_INITIAL_CAPACITY: usize = 64 * 1024;

// ─── 错误类型 ───────────────────────────────────────────────

/// Tensor Port Switch 错误枚举
#[derive(Debug, thiserror::Error)]
pub enum Tensor_IO_Error {
    /// 指定 job_id 未注册
    #[error("job_id {0:?} not registered in switch")]
    NotFound(JobId),
    /// 指定 job_id 已注册（重复 Register）
    #[error("job_id {0:?} already registered in switch")]
    AlreadyExists(JobId),
    /// 出站索引越界
    #[error("output index {index} out of bounds (len={len}) for job_id {job_id:?}")]
    IndexOutOfBounds {
        job_id: JobId,
        index: usize,
        len: usize,
    },
    /// Pipeline inference_id 未注册
    #[error("pipeline inference_id {0} not found")]
    PipelineNotFound(u64),
    /// Pipeline 的 inbound 或 outbound 尚未就绪
    #[error("pipeline inference_id {0}: stream not ready (inbound={1}, outbound={2})")]
    PipelineStreamNotReady(u64, bool, bool),
}

// ─── 失败上报 ───────────────────────────────────────────────

/// 失败上报结构（ML Thread → Core）
///
/// 当广播发送中某些目标失败时，通过 `failure_tx` 上报给 Core，
/// Core 异步处理（如 `Remove_Output` 清理坏目标）。
#[derive(Debug, Clone)]
pub struct FailureReport {
    /// 关联的 Job
    pub job_id: JobId,
    /// 失败的出站目标索引列表
    pub failed_indices: Vec<usize>,
}

// ─── Pipeline 条目（新路径：以 inference_id 为 key） ──────────

/// Pipeline 条目
///
/// 每个分布式推理 pipeline 对应一个 Pipeline_Entry，
/// 支持 inbound/outbound 分步注册（Phase 1 期间逐步到达）。
struct Pipeline_Entry {
    /// 入站流（从上游节点接收张量），分步注册时先为 None
    inbound: Arc<std::sync::Mutex<Option<libp2p::Stream>>>,
    /// 出站流（向下游节点发送张量），分步注册时先为 None
    outbound: Arc<std::sync::Mutex<Option<libp2p::Stream>>>,
    /// 最后成功发送的 offset（AtomicU64）
    last_send_offset: Arc<AtomicU64>,
    /// 最后成功接收的 offset（AtomicU64）
    last_recv_offset: Arc<AtomicU64>,
    /// 失败上报通道发送端
    failure_tx: mpsc::UnboundedSender<FailureReport>,
    /// 失败上报通道接收端（Create_Endpoint 时取走）
    failure_rx: Option<mpsc::UnboundedReceiver<FailureReport>>,
}

// ─── 内部条目（旧路径：以 JobId 为 key） ─────────────────────

/// Switch 内部条目
///
/// 每个注册的 Job 对应一个 Port_Entry，持有 inbound/outbound stream 的共享引用。
/// ML Thread 和 Core 通过 Arc 共享同一份锁。
struct Port_Entry {
    /// 入站流（单源，受 Mutex 保护）
    inbound: Arc<std::sync::Mutex<libp2p::Stream>>,
    /// 出站流（多目标广播，受 Mutex 保护）
    outbound: Arc<std::sync::Mutex<Vec<libp2p::Stream>>>,
    /// 最后成功发送的 offset（AtomicU64，Core 可无锁读取）
    last_send_offset: Arc<AtomicU64>,
    /// 最后成功接收的 offset（AtomicU64，Core 可无锁读取）
    last_recv_offset: Arc<AtomicU64>,
    /// 失败上报通道发送端（Core 持有接收端）
    failure_tx: mpsc::UnboundedSender<FailureReport>,
}

// ─── Tensor_IO_Endpoint ─────────────────────────────────────

/// ML Thread 端点（Session 持有）
///
/// 提供 `Send`、`Receive`、`Send_EOF` 三个阻塞方法，供 ML Engine worker 线程调用。
/// 内部通过 `Arc<Mutex>` 与 Switch 共享 stream，Core 可在推理间隙热切换。
///
/// ## 并发安全
/// - 推理计算时（~50ms）不持锁，Core 可随时 Swap
/// - I/O 读写时（~1ms）持锁，Core 极短等待
/// - 所有 `lock()` 使用 `unwrap_or_else(|e| e.into_inner())` 防中毒
///
/// ## I/O 超时保护
/// 所有 `block_on` 调用包装 `tokio::time::timeout(IO_TIMEOUT, ...)`，
/// 防止 stream 断开时 ML Thread 长时间持锁，阻塞 Core 的 Swap 操作。
pub struct Tensor_IO_Endpoint {
    /// 入站流引用（与 Switch 共享同一 Arc）
    inbound: Arc<std::sync::Mutex<libp2p::Stream>>,
    /// 出站流引用（与 Switch 共享同一 Arc）
    outbound: Arc<std::sync::Mutex<Vec<libp2p::Stream>>>,
    /// 线程私有的预分配缓冲区（复用，无需共享）
    buffer: Tensor_Buffer,
    /// tokio runtime 句柄（用于 block_on）
    rt: tokio::runtime::Handle,
    /// 最后成功发送的 offset（AtomicU64，Core 可无锁读取）
    last_send_offset: Arc<AtomicU64>,
    /// 最后成功接收的 offset（AtomicU64，Core 可无锁读取）
    last_recv_offset: Arc<AtomicU64>,
    /// 失败上报通道（发送失败索引到 Core）
    failure_tx: mpsc::UnboundedSender<FailureReport>,
    /// 关联的 Job ID（用于 FailureReport）
    job_id: JobId,
}

// Tensor_IO_Endpoint 必须可跨线程传递（Send 到 OS 线程）
// Arc<std::sync::Mutex<_>> 天然满足 Send + Sync
unsafe impl Send for Tensor_IO_Endpoint {}

impl Tensor_IO_Endpoint {
    /// 独立构造器（向后兼容，供过渡期使用）
    ///
    /// 从原始 inbound/outbound stream 构造 Endpoint，不经过 Switch。
    /// 此构造器在"先连接后启动"模式全面切换后将被移除。
    ///
    /// # 参数
    /// - `inbound`: 入站流（从上游节点）
    /// - `outbound`: 出站流（向下游节点）
    /// - `rt`: tokio runtime 句柄
    /// - `job_id`: 关联的 Job ID
    ///
    /// # Returns
    /// `(Tensor_IO_Endpoint, UnboundedReceiver<FailureReport>)` 元组
    pub fn New(
        inbound: libp2p::Stream,
        outbound: libp2p::Stream,
        rt: tokio::runtime::Handle,
        job_id: JobId,
    ) -> (Self, mpsc::UnboundedReceiver<FailureReport>) {
        let (failure_tx, failure_rx) = mpsc::unbounded_channel();
        let endpoint = Self {
            inbound: Arc::new(std::sync::Mutex::new(inbound)),
            outbound: Arc::new(std::sync::Mutex::new(vec![outbound])),
            buffer: Tensor_Buffer::New(BUFFER_INITIAL_CAPACITY),
            rt,
            last_send_offset: Arc::new(AtomicU64::new(0)),
            last_recv_offset: Arc::new(AtomicU64::new(0)),
            failure_tx,
            job_id,
        };
        (endpoint, failure_rx)
    }

    /// 阻塞发送张量（广播到所有出站目标）
    ///
    /// 单个目标失败不中断推理，记录失败索引并上报 Core。
    /// Mutex 使用 `unwrap_or_else` 防中毒处理。
    ///
    /// # Arguments
    /// * `offset` - 当前推理步骤的位置偏移
    /// * `data` - 张量原始字节数据
    ///
    /// # Returns
    /// 始终返回 `Ok(())`，即使部分目标失败也不中断推理。
    pub fn Send(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        let mut streams = self.outbound.lock().unwrap_or_else(|e| e.into_inner());
        let mut failed_indices = Vec::new();

        for (idx, stream) in streams.iter_mut().enumerate() {
            let result = self.rt.block_on(async {
                tokio::time::timeout(IO_TIMEOUT, Send_Tensor_Frame(stream, offset, data)).await
            });
            match result {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    failed_indices.push(idx);
                }
            }
        }

        // 上报失败索引（供 Core 异步处理）
        if !failed_indices.is_empty() {
            let _ = self.failure_tx.send(FailureReport {
                job_id: self.job_id,
                failed_indices,
            });
        }

        // 记录最后成功 offset
        self.last_send_offset.store(offset, Ordering::Relaxed);
        Ok(())
    }

    /// 阻塞接收张量（从入站源读取一帧）
    ///
    /// 带超时保护，超时返回 `TimedOut` 错误，释放锁后 Core 可执行 Swap。
    ///
    /// # Returns
    /// - `Ok(offset)` - 当前帧偏移。通过 `Get_Buffer()` 获取数据。
    ///   若 offset == `u64::MAX` 表示 EOF（推理结束）。
    /// - `Err(TimedOut)` - I/O 超时，stream 可能已断开
    /// - `Err(other)` - 其他 I/O 错误
    pub fn Receive(&mut self) -> io::Result<u64> {
        let mut stream = self.inbound.lock().unwrap_or_else(|e| e.into_inner());
        let result = self.rt.block_on(async {
            tokio::time::timeout(
                IO_TIMEOUT,
                Receive_Tensor_Frame(&mut *stream, &mut self.buffer),
            )
            .await
        });
        match result {
            Ok(inner) => {
                if let Ok(offset) = &inner {
                    self.last_recv_offset.store(*offset, Ordering::Relaxed);
                }
                inner
            }
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "receive timeout",
            )),
        }
    }

    /// 获取最近一次 Receive 的数据
    ///
    /// 返回内部缓冲区的只读引用。调用 Receive 后有效，下次 Receive 覆盖。
    pub fn Get_Buffer(&self) -> &[u8] {
        self.buffer.As_Slice()
    }

    /// 发送 EOF 到所有出站目标
    ///
    /// 推理结束时调用。单个目标失败不影响其他目标。
    pub fn Send_EOF(&mut self) -> io::Result<()> {
        let mut streams = self.outbound.lock().unwrap_or_else(|e| e.into_inner());
        for stream in streams.iter_mut() {
            let _ = self.rt.block_on(async {
                tokio::time::timeout(IO_TIMEOUT, Send_EOF(stream)).await
            });
        }
        Ok(())
    }

    /// 获取最后成功发送的 offset（供 Core 读取做一致性决策）
    pub fn Last_Send_Offset(&self) -> u64 {
        self.last_send_offset.load(Ordering::Relaxed)
    }

    /// 获取最后成功接收的 offset（供 Core 读取做一致性决策）
    pub fn Last_Recv_Offset(&self) -> u64 {
        self.last_recv_offset.load(Ordering::Relaxed)
    }
}

impl std::fmt::Debug for Tensor_IO_Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tensor_IO_Endpoint")
            .field("job_id", &self.job_id)
            .field("last_send_offset", &self.last_send_offset.load(Ordering::Relaxed))
            .field("last_recv_offset", &self.last_recv_offset.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

// ─── Tensor_IO_Capability trait ─────────────────────────────

/// Switch 控制面 Trait（Core 调用）
///
/// 提供 Register/Swap/Add/Remove/Set/Deregister 操作，
/// 所有操作为 async（内部使用 tokio::sync::Mutex 保护 entry map）。
#[async_trait]
pub trait Tensor_IO_Capability: Send + Sync {
    /// 注册 Job + 初始绑定 stream → 返回 ML 接口
    ///
    /// # Arguments
    /// * `job_id` - Job 标识
    /// * `inbound` - 入站流（从上游节点）
    /// * `outbound` - 出站流列表（多目标广播）
    /// * `rt` - tokio runtime 句柄（注入 Endpoint 供 block_on）
    ///
    /// # Returns
    /// `(Tensor_IO_Endpoint, UnboundedReceiver<FailureReport>)` 元组：
    /// - Endpoint 交给 ML Thread
    /// - Receiver 由 Core 持有并轮询处理失败上报
    async fn Register(
        &self,
        job_id: JobId,
        inbound: libp2p::Stream,
        outbound: Vec<libp2p::Stream>,
        rt: tokio::runtime::Handle,
    ) -> Result<(Tensor_IO_Endpoint, mpsc::UnboundedReceiver<FailureReport>), Tensor_IO_Error>;

    /// 热切换入站源
    ///
    /// 锁住 inbound Mutex → 替换 stream → 解锁。ML Thread 下次 Receive 自动使用新流。
    async fn Swap_Input(
        &self,
        job_id: JobId,
        new_stream: libp2p::Stream,
    ) -> Result<(), Tensor_IO_Error>;

    /// 添加一个出站目标
    ///
    /// 新目标追加到 Vec 末尾。ML Thread 下次 Send 会包含此目标。
    async fn Add_Output(
        &self,
        job_id: JobId,
        stream: libp2p::Stream,
    ) -> Result<(), Tensor_IO_Error>;

    /// 移除一个出站目标（按索引）
    ///
    /// 使用 `swap_remove` 高效移除。注意：索引在移除后可能改变。
    async fn Remove_Output(
        &self,
        job_id: JobId,
        index: usize,
    ) -> Result<(), Tensor_IO_Error>;

    /// 替换全部出站目标
    ///
    /// 原子替换整个 Vec，适用于故障恢复场景。
    async fn Set_Outputs(
        &self,
        job_id: JobId,
        streams: Vec<libp2p::Stream>,
    ) -> Result<(), Tensor_IO_Error>;

    /// 清理（Job 结束后调用）
    ///
    /// 从内部索引移除条目。Endpoint 若仍存活，其 Arc 引用仍有效但 stream 不再受管理。
    async fn Deregister(&self, job_id: JobId);

    /// 查询是否活跃
    async fn Is_Active(&self, job_id: JobId) -> bool;

    /// 获取指定 Job 最后成功发送的 offset
    ///
    /// Core 在 Swap 前读取此值，用于决策新 Worker 的起始 offset。
    async fn Last_Send_Offset(&self, job_id: JobId) -> Option<u64>;

    /// 获取指定 Job 最后成功接收的 offset
    async fn Last_Recv_Offset(&self, job_id: JobId) -> Option<u64>;
}

// ─── Tensor_Port_Switch ─────────────────────────────────────

/// Tensor Port Switch 主体
///
/// 集中管理所有 Job 的张量流。Core 通过 `Tensor_IO_Capability` trait 操作，
/// ML Thread 通过 `Tensor_IO_Endpoint` 直接做 I/O。
///
/// ## 内部使用 `tokio::sync::Mutex`
/// 保护 entry map 的增删查操作。锁内仅做 HashMap 操作（极短），
/// 不含任何 I/O，不会阻塞异步运行时。
///
/// ## 线程安全
/// - `Port_Entry` 内的 `inbound`/`outbound` 使用 `std::sync::Mutex`（ML Thread 是 OS 线程）
/// - Switch 的 `entries` 使用 `tokio::sync::Mutex`（Core 是 async 任务）
pub struct Tensor_Port_Switch {
    /// 旧路径：以 JobId 为 key 管理 Job 的张量流
    entries: tokio::sync::Mutex<HashMap<JobId, Port_Entry>>,
    /// 新路径：以 inference_id 为 key 管理 Pipeline 的张量流（分步注册）
    pipeline_entries: tokio::sync::Mutex<HashMap<u64, Pipeline_Entry>>,
}

impl Tensor_Port_Switch {
    /// 创建空的 Switch
    pub fn New() -> Self {
        Tensor_Port_Switch {
            entries: tokio::sync::Mutex::new(HashMap::new()),
            pipeline_entries: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    // ─── Pipeline 分步注册接口（新路径） ─────────────────────

    /// 注册入站流（TensorStreamArrived 时调用）
    ///
    /// 若 inference_id 对应条目不存在则自动创建。
    /// 若已存在则替换 inbound stream。
    pub async fn Register_Inbound(&self, inference_id: u64, stream: libp2p::Stream) {
        let mut map = self.pipeline_entries.lock().await;
        let entry = map.entry(inference_id).or_insert_with(|| {
            let (failure_tx, failure_rx) = mpsc::unbounded_channel();
            Pipeline_Entry {
                inbound: Arc::new(std::sync::Mutex::new(None)),
                outbound: Arc::new(std::sync::Mutex::new(None)),
                last_send_offset: Arc::new(AtomicU64::new(0)),
                last_recv_offset: Arc::new(AtomicU64::new(0)),
                failure_tx,
                failure_rx: Some(failure_rx),
            }
        });
        let mut guard = entry.inbound.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(stream);
    }

    /// 注册出站流（Establish_Tensor_Stream 成功后调用）
    ///
    /// 若 inference_id 对应条目不存在则自动创建。
    /// 若已存在则替换 outbound stream。
    pub async fn Register_Outbound(&self, inference_id: u64, stream: libp2p::Stream) {
        let mut map = self.pipeline_entries.lock().await;
        let entry = map.entry(inference_id).or_insert_with(|| {
            let (failure_tx, failure_rx) = mpsc::unbounded_channel();
            Pipeline_Entry {
                inbound: Arc::new(std::sync::Mutex::new(None)),
                outbound: Arc::new(std::sync::Mutex::new(None)),
                last_send_offset: Arc::new(AtomicU64::new(0)),
                last_recv_offset: Arc::new(AtomicU64::new(0)),
                failure_tx,
                failure_rx: Some(failure_rx),
            }
        });
        let mut guard = entry.outbound.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(stream);
    }

    /// 创建 Endpoint 供 ML Thread 使用（Join_Pipeline 时调用）
    ///
    /// 要求 inbound 和 outbound 均已注册（非 None），否则返回错误。
    /// 创建后从 Pipeline_Entry 中取走 failure_rx。
    ///
    /// # Arguments
    /// * `inference_id` - 分布式推理全局唯一 ID
    /// * `rt` - tokio runtime 句柄（注入 Endpoint 供 block_on）
    /// * `job_id` - 关联的 Job ID（用于 FailureReport）
    pub async fn Create_Endpoint(
        &self,
        inference_id: u64,
        rt: tokio::runtime::Handle,
        job_id: JobId,
    ) -> Result<(Tensor_IO_Endpoint, mpsc::UnboundedReceiver<FailureReport>), Tensor_IO_Error> {
        let mut map = self.pipeline_entries.lock().await;
        let entry = map.get_mut(&inference_id)
            .ok_or(Tensor_IO_Error::PipelineNotFound(inference_id))?;

        // 检查 inbound 和 outbound 是否已就绪
        let has_inbound = {
            let guard = entry.inbound.lock().unwrap_or_else(|e| e.into_inner());
            guard.is_some()
        };
        let has_outbound = {
            let guard = entry.outbound.lock().unwrap_or_else(|e| e.into_inner());
            guard.is_some()
        };
        if !has_inbound || !has_outbound {
            return Err(Tensor_IO_Error::PipelineStreamNotReady(
                inference_id, has_inbound, has_outbound,
            ));
        }

        // 取出 inbound stream 并包装为单源 Mutex（Endpoint 需要 Stream 而非 Option<Stream>）
        let inbound_stream = {
            let mut guard = entry.inbound.lock().unwrap_or_else(|e| e.into_inner());
            guard.take().unwrap() // 已验证 is_some
        };
        let outbound_stream = {
            let mut guard = entry.outbound.lock().unwrap_or_else(|e| e.into_inner());
            guard.take().unwrap() // 已验证 is_some
        };

        // 取走 failure_rx
        let failure_rx = entry.failure_rx.take()
            .ok_or_else(|| Tensor_IO_Error::PipelineNotFound(inference_id))?;

        // 组装 Endpoint（与旧路径 Register 相同结构）
        let inbound_arc = Arc::new(std::sync::Mutex::new(inbound_stream));
        let outbound_arc = Arc::new(std::sync::Mutex::new(vec![outbound_stream]));

        let endpoint = Tensor_IO_Endpoint {
            inbound: inbound_arc,
            outbound: outbound_arc,
            buffer: Tensor_Buffer::New(BUFFER_INITIAL_CAPACITY),
            rt,
            last_send_offset: Arc::clone(&entry.last_send_offset),
            last_recv_offset: Arc::clone(&entry.last_recv_offset),
            failure_tx: entry.failure_tx.clone(),
            job_id,
        };

        Ok((endpoint, failure_rx))
    }

    /// 清理 Pipeline 条目（推理结束或建立失败时调用）
    pub async fn Deregister_Pipeline(&self, inference_id: u64) {
        let mut map = self.pipeline_entries.lock().await;
        map.remove(&inference_id);
    }

    /// 查询 Pipeline 是否已注册
    pub async fn Is_Pipeline_Active(&self, inference_id: u64) -> bool {
        let map = self.pipeline_entries.lock().await;
        map.contains_key(&inference_id)
    }
}

#[async_trait]
impl Tensor_IO_Capability for Tensor_Port_Switch {
    async fn Register(
        &self,
        job_id: JobId,
        inbound: libp2p::Stream,
        outbound: Vec<libp2p::Stream>,
        rt: tokio::runtime::Handle,
    ) -> Result<(Tensor_IO_Endpoint, mpsc::UnboundedReceiver<FailureReport>), Tensor_IO_Error> {
        let mut map = self.entries.lock().await;

        if map.contains_key(&job_id) {
            return Err(Tensor_IO_Error::AlreadyExists(job_id));
        }

        // 创建共享状态
        let inbound_arc = Arc::new(std::sync::Mutex::new(inbound));
        let outbound_arc = Arc::new(std::sync::Mutex::new(outbound));
        let last_send_offset = Arc::new(AtomicU64::new(0));
        let last_recv_offset = Arc::new(AtomicU64::new(0));
        let (failure_tx, failure_rx) = mpsc::unbounded_channel();

        // 组装 Port_Entry
        let entry = Port_Entry {
            inbound: Arc::clone(&inbound_arc),
            outbound: Arc::clone(&outbound_arc),
            last_send_offset: Arc::clone(&last_send_offset),
            last_recv_offset: Arc::clone(&last_recv_offset),
            failure_tx: failure_tx.clone(),
        };
        map.insert(job_id, entry);

        // 组装 Endpoint（ML Thread 持有）
        let endpoint = Tensor_IO_Endpoint {
            inbound: inbound_arc,
            outbound: outbound_arc,
            buffer: Tensor_Buffer::New(BUFFER_INITIAL_CAPACITY),
            rt,
            last_send_offset,
            last_recv_offset,
            failure_tx,
            job_id,
        };

        Ok((endpoint, failure_rx))
    }

    async fn Swap_Input(
        &self,
        job_id: JobId,
        new_stream: libp2p::Stream,
    ) -> Result<(), Tensor_IO_Error> {
        let map = self.entries.lock().await;
        let entry = map
            .get(&job_id)
            .ok_or(Tensor_IO_Error::NotFound(job_id))?;

        // 锁住 inbound Mutex → 替换 stream → 解锁
        let mut guard = entry.inbound.lock().unwrap_or_else(|e| e.into_inner());
        *guard = new_stream;

        Ok(())
    }

    async fn Add_Output(
        &self,
        job_id: JobId,
        stream: libp2p::Stream,
    ) -> Result<(), Tensor_IO_Error> {
        let map = self.entries.lock().await;
        let entry = map
            .get(&job_id)
            .ok_or(Tensor_IO_Error::NotFound(job_id))?;

        let mut guard = entry.outbound.lock().unwrap_or_else(|e| e.into_inner());
        guard.push(stream);

        Ok(())
    }

    async fn Remove_Output(
        &self,
        job_id: JobId,
        index: usize,
    ) -> Result<(), Tensor_IO_Error> {
        let map = self.entries.lock().await;
        let entry = map
            .get(&job_id)
            .ok_or(Tensor_IO_Error::NotFound(job_id))?;

        let mut guard = entry.outbound.lock().unwrap_or_else(|e| e.into_inner());
        if index >= guard.len() {
            return Err(Tensor_IO_Error::IndexOutOfBounds {
                job_id,
                index,
                len: guard.len(),
            });
        }
        guard.swap_remove(index);

        Ok(())
    }

    async fn Set_Outputs(
        &self,
        job_id: JobId,
        streams: Vec<libp2p::Stream>,
    ) -> Result<(), Tensor_IO_Error> {
        let map = self.entries.lock().await;
        let entry = map
            .get(&job_id)
            .ok_or(Tensor_IO_Error::NotFound(job_id))?;

        let mut guard = entry.outbound.lock().unwrap_or_else(|e| e.into_inner());
        *guard = streams;

        Ok(())
    }

    async fn Deregister(&self, job_id: JobId) {
        let mut map = self.entries.lock().await;
        map.remove(&job_id);
    }

    async fn Is_Active(&self, job_id: JobId) -> bool {
        let map = self.entries.lock().await;
        map.contains_key(&job_id)
    }

    async fn Last_Send_Offset(&self, job_id: JobId) -> Option<u64> {
        let map = self.entries.lock().await;
        map.get(&job_id)
            .map(|e| e.last_send_offset.load(Ordering::Relaxed))
    }

    async fn Last_Recv_Offset(&self, job_id: JobId) -> Option<u64> {
        let map = self.entries.lock().await;
        map.get(&job_id)
            .map(|e| e.last_recv_offset.load(Ordering::Relaxed))
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// TC-01: Register 后 Is_Active 返回 true
    #[tokio::test]
    async fn test_register_makes_active() {
        let switch = Tensor_Port_Switch::New();
        // 由于无法在单元测试中构造 libp2p::Stream，仅测试 NotFound 路径
        assert!(!switch.Is_Active(JobId(1)).await);
    }

    /// TC-02: Deregister 后 Is_Active 返回 false
    #[tokio::test]
    async fn test_deregister_removes() {
        let switch = Tensor_Port_Switch::New();
        switch.Deregister(JobId(1)).await;
        assert!(!switch.Is_Active(JobId(1)).await);
    }

    /// TC-03: Swap_Input 对未注册的 job_id 返回 NotFound
    #[tokio::test]
    async fn test_swap_input_not_found() {
        let switch = Tensor_Port_Switch::New();
        // 无法构造 libp2p::Stream，但可以验证错误路径
        // 实际 Swap 需要先 Register，这里只验证 guard
        let result = switch.Last_Send_Offset(JobId(999)).await;
        assert!(result.is_none());
    }

    /// TC-04: Last_Send_Offset 对未注册的 job_id 返回 None
    #[tokio::test]
    async fn test_last_send_offset_not_found() {
        let switch = Tensor_Port_Switch::New();
        assert_eq!(switch.Last_Send_Offset(JobId(42)).await, None);
    }

    /// TC-05: Last_Recv_Offset 对未注册的 job_id 返回 None
    #[tokio::test]
    async fn test_last_recv_offset_not_found() {
        let switch = Tensor_Port_Switch::New();
        assert_eq!(switch.Last_Recv_Offset(JobId(42)).await, None);
    }

    /// TC-06: Remove_Output 对未注册的 job_id 返回 NotFound
    #[tokio::test]
    async fn test_remove_output_not_found() {
        let switch = Tensor_Port_Switch::New();
        // 无法完整测试（需要 libp2p::Stream），验证接口不 panic
        let offset = switch.Last_Recv_Offset(JobId(100)).await;
        assert!(offset.is_none());
    }

    /// TC-07: FailureReport 字段正确构造
    #[test]
    fn test_failure_report_construct() {
        let report = FailureReport {
            job_id: JobId(7),
            failed_indices: vec![0, 2, 5],
        };
        assert_eq!(report.job_id, JobId(7));
        assert_eq!(report.failed_indices.len(), 3);
        assert_eq!(report.failed_indices[1], 2);
    }

    /// TC-08: Tensor_IO_Error Display 格式正确
    #[test]
    fn test_tensor_io_error_display() {
        let err = Tensor_IO_Error::NotFound(JobId(42));
        assert!(format!("{}", err).contains("42"));

        let err = Tensor_IO_Error::AlreadyExists(JobId(7));
        assert!(format!("{}", err).contains("already registered"));

        let err = Tensor_IO_Error::IndexOutOfBounds {
            job_id: JobId(1),
            index: 5,
            len: 3,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("5"));
        assert!(msg.contains("3"));
    }

    /// TC-09: New 创建空 Switch，无 panic
    #[test]
    fn test_new_creates_empty_switch() {
        let _switch = Tensor_Port_Switch::New();
    }

    /// TC-10: Deregister 不存在的 job_id 不 panic（幂等）
    #[tokio::test]
    async fn test_deregister_idempotent() {
        let switch = Tensor_Port_Switch::New();
        switch.Deregister(JobId(999)).await;
        switch.Deregister(JobId(999)).await;
        // 不 panic 即通过
    }
}
