//Presented by KeJi
//Date: 2026-03-12

//! 网络协议定义模块
//! 包含命令协议、文件传输协议、张量传输协议的消息结构定义

use serde::{Deserialize, Serialize};
use libp2p::request_response::Codec;
use libp2p::StreamProtocol;
use futures::prelude::*;
use std::io;
use std::marker::PhantomData;

// ===== 协议标识符 =====
pub const COMMAND_PROTOCOL: &str = "/pleiades/cmd/1.0.0";
pub const FILE_PROTOCOL: &str = "/pleiades/file/1.0.0";
pub const TENSOR_PROTOCOL: &str = "/pleiades/tensor/1.0.0";

// ===== 张量数据类型 =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TensorDtype {
    F32,
    F64,
    I32,
    I64,
    U8,
    BF16,
    F16,
}

// ===== 命令协议 =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandRequest {
    /// 心跳检测
    Ping,
    /// 获取节点状态
    GetStatus,
    /// 自定义命令
    Custom { cmd: String, args: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandResponse {
    /// 心跳响应
    Pong { latency_ms: u64 },
    /// 状态响应
    Status { load: f32, memory_free: u64 },
    /// 结果响应
    Result { success: bool, data: String },
    /// 错误响应
    Error { msg: String },
}

// ===== 文件传输协议 =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileRequest {
    /// 请求文件信息
    Info { file_id: String },
    /// 请求文件块
    Chunk { file_id: String, offset: u64, length: u32 },
    /// 发送完整文件（MVP简化版，适用于小文件）
    SendFile {
        /// 文件名
        filename: String,
        /// 文件内容
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileResponse {
    /// 文件信息响应
    Info { file_id: String, size: u64, hash: Vec<u8> },
    /// 文件块响应
    Chunk { offset: u64, data: Vec<u8> },
    /// 文件接收确认
    Received { filename: String, success: bool },
    /// 错误响应
    Error { msg: String },
}

// ===== 张量传输协议（推理核心）=====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorRequest {
    /// 请求追踪ID
    pub request_id: String,
    /// 序列化后的张量数据
    pub tensor_data: Vec<u8>,
    /// 张量形状
    pub shape: Vec<usize>,
    /// 数据类型
    pub dtype: TensorDtype,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TensorResponse {
    /// 确认响应
    Ack { request_id: String },
    /// 错误响应
    Error { request_id: String, msg: String },
}

// ===== 统一请求/响应类型 =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PleiadesRequest {
    Command(CommandRequest),
    File(FileRequest),
    Tensor(TensorRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PleiadesResponse {
    Command(CommandResponse),
    File(FileResponse),
    Tensor(TensorResponse),
}

// ===== Codec编解码器 =====
/// 使用bincode进行序列化的Codec
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
    type Request = PleiadesRequest;
    type Response = PleiadesResponse;

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
        Box::pin(async move {
            // 读取长度前缀 (4字节)
            let mut len_bytes = [0u8; 4];
            io.read_exact(&mut len_bytes).await?;
            let len = u32::from_be_bytes(len_bytes) as usize;

            // 限制最大消息大小 (16MB)
            if len > 16 * 1024 * 1024 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
            }

            // 读取数据
            let mut buf = vec![0u8; len];
            io.read_exact(&mut buf).await?;

            // 反序列化
            bincode::deserialize(&buf)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        })
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
        Box::pin(async move {
            // 读取长度前缀 (4字节)
            let mut len_bytes = [0u8; 4];
            io.read_exact(&mut len_bytes).await?;
            let len = u32::from_be_bytes(len_bytes) as usize;

            // 限制最大消息大小 (16MB)
            if len > 16 * 1024 * 1024 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
            }

            // 读取数据
            let mut buf = vec![0u8; len];
            io.read_exact(&mut buf).await?;

            // 反序列化
            bincode::deserialize(&buf)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        })
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
        Box::pin(async move {
            // 序列化
            let data = bincode::serialize(&req)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            // 写入长度前缀
            let len = data.len() as u32;
            io.write_all(&len.to_be_bytes()).await?;

            // 写入数据
            io.write_all(&data).await?;
            io.flush().await?;

            Ok(())
        })
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
        Box::pin(async move {
            // 序列化
            let data = bincode::serialize(&res)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            // 写入长度前缀
            let len = data.len() as u32;
            io.write_all(&len.to_be_bytes()).await?;

            // 写入数据
            io.write_all(&data).await?;
            io.flush().await?;

            Ok(())
        })
    }
}

// ===== 分块传输常量 =====
/// 文件分块大小 (64KB)
pub const FILE_CHUNK_SIZE: u32 = 65536;

/// 张量分块大小 (1MB)
pub const TENSOR_CHUNK_SIZE: usize = 1024 * 1024;
