//Presented by KeJi
//Date : 2026-03-30

// gguf tensor 模块 - 负责GGUF格式的模型输入、输出tensor进行包装或者解析，便于网络传输
// 此类的作用包括
// - 定义TensorPacket结构体，包含tensor的名称、形状、数据类型、原始字节数据等信息，用于在网络传输之前包装GGUF模型输出的tensor数据
// - 序列化，序列化TensorPacket为字节流，便于网络传输
// - 反序列化，从字节流中解析出TensorPacket对象，恢复

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::fmt;

// ============================================================
// 错误类型
// ============================================================

/// TensorPacket 序列化/反序列化错误
#[derive(Debug)]
pub enum GGUF_Tensor_Error {
    /// 序列化时写入失败
    Serialize_Failed(String),
    /// 反序列化时数据不足
    Insufficient_Data { expected: usize, actual: usize },
    /// 反序列化时UTF-8解码失败
    Invalid_Utf8(String),
    /// 反序列化时dtype字符串无法识别
    Unknown_Dtype(String),
}

impl fmt::Display for GGUF_Tensor_Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GGUF_Tensor_Error::Serialize_Failed(msg) => write!(f, "Serialize failed: {}", msg),
            GGUF_Tensor_Error::Insufficient_Data { expected, actual } => {
                write!(f, "Insufficient data: expected {} bytes, got {}", expected, actual)
            }
            GGUF_Tensor_Error::Invalid_Utf8(msg) => write!(f, "Invalid UTF-8: {}", msg),
            GGUF_Tensor_Error::Unknown_Dtype(dtype) => write!(f, "Unknown dtype: {}", dtype),
        }
    }
}

impl std::error::Error for GGUF_Tensor_Error {}

// ============================================================
// 数据类型枚举
// ============================================================

/// GGUF 模型输出所使用的数据类型
#[derive(Debug, Clone, PartialEq)]
pub enum GGUF_Dtype {
    F32,
    F16,
    BF16,
    Q8_0,
    Q4_0,
    Q4_1,
    Q5_0,
    Q5_1,
    Q2_K,
    Q3_K,
    Q4_K,
    Q5_K,
    Q6_K,
}

impl GGUF_Dtype {
    /// 将 dtype 转为字符串标识符
    pub fn As_Str(&self) -> &'static str {
        match self {
            GGUF_Dtype::F32 => "F32",
            GGUF_Dtype::F16 => "F16",
            GGUF_Dtype::BF16 => "BF16",
            GGUF_Dtype::Q8_0 => "Q8_0",
            GGUF_Dtype::Q4_0 => "Q4_0",
            GGUF_Dtype::Q4_1 => "Q4_1",
            GGUF_Dtype::Q5_0 => "Q5_0",
            GGUF_Dtype::Q5_1 => "Q5_1",
            GGUF_Dtype::Q2_K => "Q2_K",
            GGUF_Dtype::Q3_K => "Q3_K",
            GGUF_Dtype::Q4_K => "Q4_K",
            GGUF_Dtype::Q5_K => "Q5_K",
            GGUF_Dtype::Q6_K => "Q6_K",
        }
    }

    /// 从字符串标识符解析 dtype
    pub fn From_Str(s: &str) -> Result<Self, GGUF_Tensor_Error> {
        match s {
            "F32" => Ok(GGUF_Dtype::F32),
            "F16" => Ok(GGUF_Dtype::F16),
            "BF16" => Ok(GGUF_Dtype::BF16),
            "Q8_0" => Ok(GGUF_Dtype::Q8_0),
            "Q4_0" => Ok(GGUF_Dtype::Q4_0),
            "Q4_1" => Ok(GGUF_Dtype::Q4_1),
            "Q5_0" => Ok(GGUF_Dtype::Q5_0),
            "Q5_1" => Ok(GGUF_Dtype::Q5_1),
            "Q2_K" => Ok(GGUF_Dtype::Q2_K),
            "Q3_K" => Ok(GGUF_Dtype::Q3_K),
            "Q4_K" => Ok(GGUF_Dtype::Q4_K),
            "Q5_K" => Ok(GGUF_Dtype::Q5_K),
            "Q6_K" => Ok(GGUF_Dtype::Q6_K),
            _ => Err(GGUF_Tensor_Error::Unknown_Dtype(s.to_string())),
        }
    }
}

impl fmt::Display for GGUF_Dtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.As_Str())
    }
}

// ============================================================
// TensorPacket 结构体
// ============================================================

/// 用于网络传输的 Tensor 数据包
///
/// 包含 tensor 的名称、形状、数据类型和原始字节数据，
/// 便于在网络传输之前包装 GGUF 模型输出的 tensor 数据。
#[derive(Debug, Clone)]
pub struct GGUF_Tensor_Packet {
    /// tensor 名称（如 "blk.0.attn_q.weight"）
    pub name: String,
    /// tensor 形状（如 [4096, 4096]）
    pub shape: Vec<usize>,
    /// 数据类型
    pub dtype: GGUF_Dtype,
    /// 原始字节数据
    pub data: Vec<u8>,
}

impl GGUF_Tensor_Packet {
    /// 创建新的 TensorPacket
    pub fn New(name: String, shape: Vec<usize>, dtype: GGUF_Dtype, data: Vec<u8>) -> Self {
        Self { name, shape, dtype, data }
    }
}

// ============================================================
// 序列化 / 反序列化
// ============================================================
//
// 二进制格式布局:
//   [4 bytes] name_len (u32 big-endian)
//   [name_len bytes] name (UTF-8)
//   [4 bytes] ndim (u32 big-endian)
//   [ndim * 8 bytes] shape values (each u64 big-endian)
//   [4 bytes] dtype_len (u32 big-endian)
//   [dtype_len bytes] dtype string (UTF-8)
//   [8 bytes] data_len (u64 big-endian)
//   [data_len bytes] raw tensor data

/// GGUF_Tensor_Serialize()
/// 序列化 GGUF_Tensor_Packet 为字节流，便于网络传输
pub fn GGUF_Tensor_Serialize(packet: &GGUF_Tensor_Packet) -> Result<Vec<u8>, GGUF_Tensor_Error> {
    let name_bytes = packet.name.as_bytes();
    let dtype_str = packet.dtype.As_Str();
    let dtype_bytes = dtype_str.as_bytes();

    // 预分配: 4 + name + 4 + ndim*8 + 4 + dtype + 8 + data
    let total_size = 4 + name_bytes.len()
        + 4 + packet.shape.len() * 8
        + 4 + dtype_bytes.len()
        + 8 + packet.data.len();

    let mut buf: Vec<u8> = Vec::with_capacity(total_size);

    // name
    buf.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(name_bytes);

    // shape
    buf.extend_from_slice(&(packet.shape.len() as u32).to_be_bytes());
    for &dim in &packet.shape {
        buf.extend_from_slice(&(dim as u64).to_be_bytes());
    }

    // dtype
    buf.extend_from_slice(&(dtype_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(dtype_bytes);

    // data
    buf.extend_from_slice(&(packet.data.len() as u64).to_be_bytes());
    buf.extend_from_slice(&packet.data);

    Ok(buf)
}

/// GGUF_Tensor_Deserialize()
/// 从字节流中解析出 GGUF_Tensor_Packet 对象，恢复 tensor 数据
pub fn GGUF_Tensor_Deserialize(bytes: &[u8]) -> Result<GGUF_Tensor_Packet, GGUF_Tensor_Error> {
    let mut offset: usize = 0;

    // --- 辅助: 读取指定字节数 ---
    let read_bytes = |offset: &mut usize, count: usize| -> Result<&[u8], GGUF_Tensor_Error> {
        if *offset + count > bytes.len() {
            return Err(GGUF_Tensor_Error::Insufficient_Data {
                expected: *offset + count,
                actual: bytes.len(),
            });
        }
        let slice = &bytes[*offset..*offset + count];
        *offset += count;
        Ok(slice)
    };

    // --- name ---
    let name_len_bytes = read_bytes(&mut offset, 4)?;
    let name_len = u32::from_be_bytes(name_len_bytes.try_into().unwrap()) as usize;

    let name_bytes = read_bytes(&mut offset, name_len)?;
    let name = std::str::from_utf8(name_bytes)
        .map_err(|e| GGUF_Tensor_Error::Invalid_Utf8(e.to_string()))?
        .to_string();

    // --- shape ---
    let ndim_bytes = read_bytes(&mut offset, 4)?;
    let ndim = u32::from_be_bytes(ndim_bytes.try_into().unwrap()) as usize;

    let mut shape: Vec<usize> = Vec::with_capacity(ndim);
    for _ in 0..ndim {
        let dim_bytes = read_bytes(&mut offset, 8)?;
        let dim = u64::from_be_bytes(dim_bytes.try_into().unwrap()) as usize;
        shape.push(dim);
    }

    // --- dtype ---
    let dtype_len_bytes = read_bytes(&mut offset, 4)?;
    let dtype_len = u32::from_be_bytes(dtype_len_bytes.try_into().unwrap()) as usize;

    let dtype_bytes = read_bytes(&mut offset, dtype_len)?;
    let dtype_str = std::str::from_utf8(dtype_bytes)
        .map_err(|e| GGUF_Tensor_Error::Invalid_Utf8(e.to_string()))?;
    let dtype = GGUF_Dtype::From_Str(dtype_str)?;

    // --- data ---
    let data_len_bytes = read_bytes(&mut offset, 8)?;
    let data_len = u64::from_be_bytes(data_len_bytes.try_into().unwrap()) as usize;

    let data_slice = read_bytes(&mut offset, data_len)?;
    let data = data_slice.to_vec();

    Ok(GGUF_Tensor_Packet {
        name,
        shape,
        dtype,
        data,
    })
}
