//Presented by KeJi
//Date ： 2026-04-24

//! 张量流式传输协议模块
//!
//! 定义张量流式传输协议，用于 pipeline 推理期间的高频张量传输。
//! 与 data_protocol.rs 中的 request-response 协议不同，
//! 本模块使用 libp2p::stream 提供的持久化双向流，
//! 张量以 fire-and-forget 方式发送，无需等待 ACK。
//!
//! ## 帧格式
//! +-------------------+--------------------+---------------------+
//! |   Offset          |   Tensor Length    |   Raw Tensor Data   |
//! |   8 bytes u64 LE  |   8 bytes u64 LE  |   Length bytes      |
//! +-------------------+--------------------+---------------------+
//!
//! ## EOF 哨兵帧（标记推理会话结束）
//! +-------------------+--------------------+
//! |   u64::MAX        |   0u64             |
//! |   8 bytes         |   8 bytes          |
//! +-------------------+--------------------+
//!
//! 发送方写入 [offset][length][data]，接收方读取 header 后精确读取 data。
//! 使用长度前缀保证帧边界，支持 Prefill (~8MB) 和 Decode (~16KB) 不同大小的张量。
//!
//! ## 已迁移
//! `Tensor_IO_Handle` 已被 `Tensor_IO_Endpoint`（`Src/Tensor_IO` 模块）替代。
//! 本模块保留帧格式函数供 `Tensor_IO_Endpoint` 内部调用。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use futures::prelude::*;
use std::io;

// ===== 协议标识符 =====
pub const TENSOR_STREAM_PROTOCOL: &str = "/pleiades/tensor/1.0.0";

// ===== EOF 标记 =====
/// EOF 帧的 offset 值，表示推理会话结束
pub const TENSOR_EOF_OFFSET: u64 = u64::MAX;

// ===== 最大张量大小限制 (512MB) =====
const MAX_TENSOR_SIZE: u64 = 512 * 1024 * 1024;

// ============================================================
// Tensor_Buffer — 预分配可复用缓冲区
// ============================================================

/// 预分配的张量缓冲区
///
/// 在整个 pipeline 推理生命周期内复用，避免每步推理都分配新内存。
/// Decode 阶段每步张量大小固定（hidden_size * 4 bytes），
/// Vec 分配一次即可。Prefill 阶段更大但只有一次，
/// Vec::resize 自动扩容后 Decode 阶段不再分配。
pub struct Tensor_Buffer {
    /// 原始字节缓冲区
    buf: Vec<u8>,
    /// 当前有效数据长度（可能小于 buf 容量）
    valid_len: usize,
}

impl Tensor_Buffer {
    /// 创建指定初始容量的缓冲区
    ///
    /// # Arguments
    /// * `capacity` - 初始容量（字节数），建议设为预期的最大张量大小
    pub fn New(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
            valid_len: 0,
        }
    }

    /// 获取可写切片（供 stream read_exact 写入）
    ///
    /// 如果 `len` 超过当前容量，Vec 会自动扩容。
    ///
    /// # Arguments
    /// * `len` - 需要写入的字节数
    ///
    /// # Returns
    /// 长度恰好为 `len` 的可变切片
    pub fn As_Mut_Slice(&mut self, len: usize) -> &mut [u8] {
        self.buf.resize(len, 0);
        self.valid_len = len;
        &mut self.buf[..len]
    }

    /// 获取有效数据的只读切片
    ///
    /// 返回最后一次 `As_Mut_Slice` 或 `Set_Valid_Len` 设定的有效范围。
    pub fn As_Slice(&self) -> &[u8] {
        &self.buf[..self.valid_len]
    }

    /// 获取当前有效数据长度
    pub fn Len(&self) -> usize {
        self.valid_len
    }

    /// 当前是否为空（无有效数据）
    pub fn Is_Empty(&self) -> bool {
        self.valid_len == 0
    }

    /// 清空有效数据标记（不释放内存）
    pub fn Clear(&mut self) {
        self.valid_len = 0;
    }
}

// ============================================================
// 帧读写函数
// ============================================================

/// 发送张量帧
///
/// 写入格式: [8B offset LE][8B length LE][data bytes]
/// Fire-and-forget，无需等待 ACK。
///
/// # Arguments
/// * `stream` - 已打开的双向流
/// * `offset` - 当前推理步骤的位置偏移（用于 RoPE 等位置编码）
/// * `data` - 张量原始字节数据
pub async fn Send_Tensor_Frame(
    stream: &mut libp2p::Stream,
    offset: u64,
    data: &[u8],
) -> io::Result<()> {
    // 1. 写入 offset (8 bytes, u64 LE)
    stream.write_all(&offset.to_le_bytes()).await?;

    // 2. 写入 data length (8 bytes, u64 LE)
    let length = data.len() as u64;
    stream.write_all(&length.to_le_bytes()).await?;

    // 3. 写入 data
    stream.write_all(data).await?;

    // 4. flush 确保数据发出
    stream.flush().await?;

    Ok(())
}

/// 接收张量帧到预分配缓冲区
///
/// 读取格式: [8B offset LE][8B length LE][data bytes]
/// 数据直接写入外部提供的 Tensor_Buffer，避免内存分配。
///
/// # Arguments
/// * `stream` - 已打开的双向流
/// * `buffer` - 预分配的缓冲区（数据将被写入此缓冲区）
///
/// # Returns
/// 当前帧的 offset 值。若 offset == `TENSOR_EOF_OFFSET`(u64::MAX)，
/// 表示 EOF 哨兵帧，此时 buffer 内容为空。
pub async fn Receive_Tensor_Frame(
    stream: &mut libp2p::Stream,
    buffer: &mut Tensor_Buffer,
) -> io::Result<u64> {
    // 1. 读取 16 字节 header: [8B offset][8B length]
    let mut header = [0u8; 16];
    stream.read_exact(&mut header).await?;

    let offset = u64::from_le_bytes(header[0..8].try_into().unwrap());
    let length = u64::from_le_bytes(header[8..16].try_into().unwrap());

    // 2. 检查 EOF 哨兵帧
    if offset == TENSOR_EOF_OFFSET && length == 0 {
        buffer.Clear();
        return Ok(TENSOR_EOF_OFFSET);
    }

    // 3. 检查大小限制
    if length > MAX_TENSOR_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "张量帧过大: {} bytes (max {} bytes)",
                length, MAX_TENSOR_SIZE
            ),
        ));
    }

    // 4. 读取数据到预分配缓冲区
    let len = length as usize;
    let slice = buffer.As_Mut_Slice(len);
    stream.read_exact(slice).await?;

    Ok(offset)
}

/// 发送 EOF 哨兵帧
///
/// 标记推理会话结束。接收方收到 offset=u64::MAX, length=0 后
/// 应停止推理循环并关闭流。
///
/// # Arguments
/// * `stream` - 已打开的双向流
pub async fn Send_EOF(stream: &mut libp2p::Stream) -> io::Result<()> {
    // 写入 EOF: offset=u64::MAX, length=0
    stream.write_all(&TENSOR_EOF_OFFSET.to_le_bytes()).await?;
    stream.write_all(&0u64.to_le_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

// ============================================================
// Tensor Stream Handshake — 流建立时的 JobId 交换
// ============================================================

/// 写入 Tensor Stream Handshake 帧（8 字节）
///
/// 发起方在 open_tensor_stream 后调用，写入 target_job_id
/// 供接收方 Core 读取并路由到正确的 Job。
///
/// # 参数
/// - `stream`: 已打开的出站流
/// - `target_job_id`: 接收方的 JobId（u64）
pub async fn Write_Tensor_Stream_Handshake(
    stream: &mut libp2p::Stream,
    target_job_id: u64,
) -> io::Result<()> {
    stream.write_all(&target_job_id.to_le_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// 读取 Tensor Stream Handshake 帧（8 字节）
///
/// 接收方 Core 从入站流读取 target_job_id，用于路由到正确的 Job。
///
/// # 参数
/// - `stream`: 入站流
///
/// # 返回
/// - target_job_id（u64）
pub async fn Read_Tensor_Stream_Handshake(
    stream: &mut libp2p::Stream,
) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    stream.read_exact(&mut buf).await?;
    Ok(u64::from_le_bytes(buf))
}
