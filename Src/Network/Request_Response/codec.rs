//Presented by KeJi
//Date ： 2026-04-08

//! Request-Response 编解码模块
//!
//! TLV（Type-Length-Value）帧格式编解码器：网络层只负责发送和接收字节流，
//! 不负责序列化/反序列化。采用 TLV（Type-Length-Value）帧格式。
//!
//! 帧格式:
//! +----------+--------------+---------------------+
//! |  Type    |   Length     |      Payload        |
//! |  1 byte  |  8 bytes BE  |  Length bytes        |
//! +----------+--------------+---------------------+
#[allow(nonstandard_style)]
use libp2p::request_response::Codec;
use libp2p::StreamProtocol;
use futures::prelude::*;
use std::io;
use std::marker::PhantomData;

// ===== 协议标识符 =====
pub const DATA_PROTOCOL: &str = "/pleiades/data/1.0.0";

// ===== 最大帧大小限制 (2GB) =====
const MAX_FRAME_SIZE: u64 = 2 * 1024 * 1024 * 1024;

// ===== 数据类型枚举 =====

/// 数据帧类型标记
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DataType {
    /// 命令/控制消息
    Command = 0,
    /// 数据（张量、中间结果等）
    Data = 1,
    /// 文件通知（仅传输文件元数据：文件名、大小等，不传输文件内容）
    File = 2,
    /// 机器人数据（位姿/地图等，Task 9_1）
    Robot = 3,
    /// 信息通知（节点状态、能力描述等元信息交换）
    Info = 4,
}

impl DataType {
    /// 从 u8 转换为 DataType
    pub fn From_U8(v: u8) -> io::Result<Self> {
        match v {
            0 => Ok(DataType::Command),
            1 => Ok(DataType::Data),
            2 => Ok(DataType::File),
            3 => Ok(DataType::Robot),
            4 => Ok(DataType::Info),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown DataType: {}", v),
            )),
        }
    }
}

// ===== 统一网络数据结构 =====

/// 统一网络数据帧（同时用作 Request 和 Response）
///
/// 网络层只看字节流，不关心上层语义。
/// Request 和 Response 的线上格式完全相同，因此合并为一个结构体。
#[derive(Debug, Clone)]
#[allow(nonstandard_style)]
pub struct Network_Data {
    /// 数据类型标记
    pub data_type: DataType,
    /// 原始载荷字节流（上层负责序列化）
    pub payload: Vec<u8>,
}

// ===== 帧读写辅助函数 =====

/// 从异步读取流中读取一个 TLV 帧
///
/// 格式: [1 byte type] [8 bytes length BE u64] [payload bytes]
async fn Read_Frame<T: AsyncRead + Unpin + Send>(io: &mut T) -> io::Result<Network_Data> {
    // 1. 读取 1 字节 type
    let mut type_byte = [0u8; 1];
    io.read_exact(&mut type_byte).await?;
    let data_type = DataType::From_U8(type_byte[0])?;

    // 2. 读取 8 字节 length (big-endian u64)
    let mut len_bytes = [0u8; 8];
    io.read_exact(&mut len_bytes).await?;
    let len = u64::from_be_bytes(len_bytes);

    // 3. 检查大小限制 (2GB)
    if len > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {} bytes (max {})", len, MAX_FRAME_SIZE),
        ));
    }

    // 4. 读取 payload
    let mut payload = vec![0u8; len as usize];
    io.read_exact(&mut payload).await?;

    Ok(Network_Data { data_type, payload })
}

/// 向异步写入流中写入一个 TLV 帧
///
/// 格式: [1 byte type] [8 bytes length BE u64] [payload bytes]
async fn Write_Frame<T: AsyncWrite + Unpin + Send>(io: &mut T, data: Network_Data) -> io::Result<()> {
    // 1. 写入 1 字节 type
    io.write_all(&[data.data_type as u8]).await?;

    // 2. 写入 8 字节 length (big-endian u64)
    let len = data.payload.len() as u64;
    io.write_all(&len.to_be_bytes()).await?;

    // 3. 写入 payload
    io.write_all(&data.payload).await?;
    io.flush().await?;

    Ok(())
}

// ===== Codec 编解码器 =====

/// TLV 帧格式 Codec
///
/// 读写格式: [1 byte type] [8 bytes length BE u64] [payload bytes]
/// 不做任何业务序列化，只做字节搬运
///
/// Request 和 Response 使用相同的 Network_Data 类型，
/// 线上格式完全一致，通过 Read_Frame/Write_Frame 统一处理。
#[derive(Debug, Clone)]
pub struct PleiadesCodec {
    _phantom: PhantomData<()>,
}

impl Default for PleiadesCodec {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl Codec for PleiadesCodec {
    type Protocol = StreamProtocol;
    type Request = Network_Data;
    type Response = Network_Data;

    fn read_request<'life0, 'life1, 'life2, 'async_trait, T>(
        &'life0 mut self,
        _protocol: &'life1 Self::Protocol,
        io: &'life2 mut T,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<Self::Request>> + Send + 'async_trait>>
    where
        T: AsyncRead + Unpin + Send + 'async_trait,
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(Read_Frame(io))
    }

    fn read_response<'life0, 'life1, 'life2, 'async_trait, T>(
        &'life0 mut self,
        _protocol: &'life1 Self::Protocol,
        io: &'life2 mut T,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<Self::Response>> + Send + 'async_trait>>
    where
        T: AsyncRead + Unpin + Send + 'async_trait,
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(Read_Frame(io))
    }

    fn write_request<'life0, 'life1, 'life2, 'async_trait, T>(
        &'life0 mut self,
        _protocol: &'life1 Self::Protocol,
        io: &'life2 mut T,
        req: Self::Request,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + 'async_trait>>
    where
        T: AsyncWrite + Unpin + Send + 'async_trait,
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(Write_Frame(io, req))
    }

    fn write_response<'life0, 'life1, 'life2, 'async_trait, T>(
        &'life0 mut self,
        _protocol: &'life1 Self::Protocol,
        io: &'life2 mut T,
        res: Self::Response,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + 'async_trait>>
    where
        T: AsyncWrite + Unpin + Send + 'async_trait,
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(Write_Frame(io, res))
    }
}
