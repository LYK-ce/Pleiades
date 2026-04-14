//Presented by KeJi
//Date ： 2026-04-10

//! 张量流传输管理器 + IO 句柄
//!
//! ## Tensor_Stream_Manager
//! 通用的单方向流管理组件，只负责流的**建立/接收/关闭**。
//! 使用时创建两个实例（inbound + outbound），生命周期由 Network_Service 管理。
//!
//! ## Tensor_IO_Handle
//! 推理阶段由 ML Engine worker 线程持有，直接操作 stream 做张量收发。
//! stream 通过 `Take_Stream()` 从 Manager 移出，交给 Handle。
//!
//! ## 生命周期
//! 1. 建立阶段: Network_Service 持有 Manager → Open_Stream / Set_Stream
//! 2. 交接: Take_Stream() 移出 stream → 构建 Tensor_IO_Handle
//! 3. 推理阶段: ML Engine worker 通过 Handle 直接 read/write stream
//! 4. 结束: ML Engine drop Handle → stream 自动关闭

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use libp2p::{PeerId, StreamProtocol};
use libp2p_stream as stream;
use std::io;
use tracing::{info, warn};

use super::tensor_stream_protocol::{
    TENSOR_STREAM_PROTOCOL,
    Tensor_Buffer,
    Send_Tensor_Frame, Receive_Tensor_Frame, Send_EOF,
};

// ============================================================
// Tensor_Stream_Manager — 流生命周期管理（Network_Service 持有）
// ============================================================

/// 张量流管理器（通用，单方向）
///
/// 只负责流的建立、接收和关闭。不负责数据读写。
/// 数据读写由 `Tensor_IO_Handle` 在 ML Engine 线程中完成。
pub struct Tensor_Stream_Manager {
    /// 流式传输控制句柄（可 Clone）
    stream_control: stream::Control,
    /// 当前活跃的流（建立后通过 Take_Stream 移交给 Tensor_IO_Handle）
    stream: Option<libp2p::Stream>,
}

impl Tensor_Stream_Manager {
    /// 创建新的 Tensor_Stream_Manager
    ///
    /// # 参数
    /// - `stream_control`: libp2p-stream 的 Control 句柄
    pub fn New(stream_control: stream::Control) -> Self {
        Self {
            stream_control,
            stream: None,
        }
    }

    /// 保存外部传入的流（用于接收端）
    ///
    /// 由 Network_Service 的 select! 循环调用，将接收到的入站 tensor stream 保存。
    ///
    /// # 参数
    /// - `peer`: 对方节点 ID（用于日志）
    /// - `stream`: libp2p::Stream
    pub fn Set_Stream(&mut self, peer: PeerId, stream: libp2p::Stream) {
        if self.stream.is_some() {
            warn!("替换已有的张量流 (新来源/目标: {})", peer);
        }
        info!("保存张量流 (对端: {})", peer);
        self.stream = Some(stream);
    }

    /// 主动打开到指定节点的流（用于发送端）
    ///
    /// # 参数
    /// - `peer`: 目标节点 ID
    pub async fn Open_Stream(&mut self, peer: PeerId) -> io::Result<()> {
        if self.stream.is_some() {
            warn!("替换已有的张量流 (新目标: {})", peer);
        }
        info!("打开张量流 → {}", peer);
        let stream = self.stream_control
            .open_stream(peer, StreamProtocol::new(TENSOR_STREAM_PROTOCOL))
            .await
            .map_err(|e| io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("打开张量流失败: {}", e),
            ))?;
        self.stream = Some(stream);
        info!("张量流已建立 → {}", peer);
        Ok(())
    }

    /// 将 stream 所有权移出（交给 Tensor_IO_Handle）
    ///
    /// 调用后 Manager 不再持有 stream。
    /// 用于在流建立完成后，将 stream 交给 ML Engine 的 Tensor_IO_Handle。
    ///
    /// # Returns
    /// `Some(stream)` 如果流已建立，`None` 如果流不存在
    pub fn Take_Stream(&mut self) -> Option<libp2p::Stream> {
        let stream = self.stream.take();
        if stream.is_some() {
            info!("张量流所有权已移交");
        }
        stream
    }

    /// 检查流是否已建立
    pub fn Has_Stream(&self) -> bool {
        self.stream.is_some()
    }
}

// ============================================================
// Tensor_IO_Handle — 张量 I/O 句柄（ML Engine worker 线程持有）
// ============================================================

/// 张量 I/O 句柄
///
/// 由 ML Engine 的 worker 线程持有，提供同步的 `Receive` 和 `Send` 方法。
/// 内部直接操作 `libp2p::Stream`，不经过 Network_Service 事件循环。
///
/// ## 使用方式
/// ```ignore
/// loop {
///     let (offset, data) = tensor_io.Receive()?;    // 直接 stream.read
///     let result = model.inference(data, offset);     // 推理
///     tensor_io.Send(offset, &result)?;               // 直接 stream.write
/// }
/// ```
///
/// ## 生命周期
/// drop Handle 时，stream 自动关闭（TCP FIN）。
pub struct Tensor_IO_Handle {
    /// 入站流（从上游节点接收张量）
    inbound_stream: libp2p::Stream,
    /// 出站流（向下游节点发送张量）
    outbound_stream: libp2p::Stream,
    /// 私有缓冲区（ML Engine 线程独有，无需共享）
    buffer: Tensor_Buffer,
    /// tokio runtime 句柄（用于在阻塞线程中调用 async I/O）
    rt: tokio::runtime::Handle,
}

impl Tensor_IO_Handle {
    /// 创建 Tensor_IO_Handle
    ///
    /// # 参数
    /// - `inbound_stream`: 入站流（从 inbound manager 取出）
    /// - `outbound_stream`: 出站流（从 outbound manager 取出）
    /// - `rt`: tokio runtime 句柄
    pub fn New(
        inbound_stream: libp2p::Stream,
        outbound_stream: libp2p::Stream,
        rt: tokio::runtime::Handle,
    ) -> Self {
        Self {
            inbound_stream,
            outbound_stream,
            buffer: Tensor_Buffer::New(64 * 1024), // 64KB 初始容量
            rt,
        }
    }

    /// 接收张量（阻塞调用，适合 ML Engine worker 线程）
    ///
    /// 从入站流读取一帧张量数据。返回 (offset, data 引用)。
    /// 数据存储在内部缓冲区中，下次调用 Receive 会覆盖。
    ///
    /// # Returns
    /// - `Ok(offset)`: 当前帧偏移。通过 `Get_Buffer()` 获取数据。
    ///   若 offset == u64::MAX 表示 EOF（推理结束）。
    /// - `Err`: 流读取失败
    pub fn Receive(&mut self) -> io::Result<u64> {
        self.rt.block_on(
            Receive_Tensor_Frame(&mut self.inbound_stream, &mut self.buffer)
        )
    }

    /// 发送张量（阻塞调用，适合 ML Engine worker 线程）
    ///
    /// 将数据写入出站流。Fire-and-forget，无需等 ACK。
    ///
    /// # 参数
    /// - `offset`: 当前推理步骤的位置偏移
    /// - `data`: 张量原始字节数据
    pub fn Send(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.rt.block_on(
            Send_Tensor_Frame(&mut self.outbound_stream, offset, data)
        )
    }

    /// 发送 EOF 并关闭出站流
    ///
    /// 推理结束时由协调者调用。
    pub fn Send_EOF(&mut self) -> io::Result<()> {
        self.rt.block_on(Send_EOF(&mut self.outbound_stream))
    }

    /// 获取最近一次 Receive 读取的数据
    ///
    /// 返回内部缓冲区的只读引用。
    /// 调用 Receive 后有效，下次 Receive 会覆盖。
    pub fn Get_Buffer(&self) -> &[u8] {
        self.buffer.As_Slice()
    }

    /// 获取最近一次 Receive 读取的数据长度
    pub fn Get_Buffer_Len(&self) -> usize {
        self.buffer.Len()
    }
}
