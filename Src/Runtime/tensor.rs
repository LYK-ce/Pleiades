//Presented by KeJi
//Date: 2026-03-21

//! Tensor Packet 模块 - 用于网络传输的序列化/反序列化
//!
//! 仅用于传输层，不负责推理计算

use std::collections::HashMap;

/// 数据类型标识
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DataType {
    Float32 = 0,
    Float16 = 1,
    Int64 = 2,
    Int32 = 3,
    UInt8 = 4,
    Bool = 5,
}

impl DataType {
    /// 从 u8 解析数据类型
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(DataType::Float32),
            1 => Some(DataType::Float16),
            2 => Some(DataType::Int64),
            3 => Some(DataType::Int32),
            4 => Some(DataType::UInt8),
            5 => Some(DataType::Bool),
            _ => None,
        }
    }

    /// 获取数据类型对应的字节大小
    pub fn Element_Size(&self) -> usize {
        match self {
            DataType::Float32 => 4,
            DataType::Float16 => 2,
            DataType::Int64 => 8,
            DataType::Int32 => 4,
            DataType::UInt8 => 1,
            DataType::Bool => 1,
        }
    }
}

/// Tensor Packet - 用于网络传输的张量封装
#[derive(Debug, Clone)]
pub struct TensorPacket {
    /// 张量名称
    pub name: String,
    /// 数据类型
    pub dtype: DataType,
    /// 形状
    pub shape: Vec<usize>,
    /// 原始字节数据
    pub data: Vec<u8>,
}

impl TensorPacket {
    /// 创建新的 Tensor Packet
    pub fn new(name: String, dtype: DataType, shape: Vec<usize>, data: Vec<u8>) -> Self {
        Self {
            name,
            dtype,
            shape,
            data,
        }
    }

    /// 计算元素总数
    pub fn len(&self) -> usize {
        self.shape.iter().product()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 计算数据大小（字节）
    pub fn Data_Size(&self) -> usize {
        self.len() * self.dtype.Element_Size()
    }

    /// 序列化为字节流
    /// 
    /// 格式：[版本(1B)] + [名称长度(4B)] + [名称(NB)] + [数据类型(1B)] + [维度数(4B)] + [形状(8B*ndim)] + [数据长度(8B)] + [数据(NB)]
    pub fn Serialize(&self) -> Vec<u8> {
        let mut result = Vec::new();

        // 版本号 (1字节)
        result.push(1u8);

        // 名称长度 + 名称
        let name_bytes = self.name.as_bytes();
        result.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        result.extend_from_slice(name_bytes);

        // 数据类型 (1字节)
        result.push(self.dtype as u8);

        // 维度数
        result.extend_from_slice(&(self.shape.len() as u32).to_le_bytes());

        // 形状 (每个维度8字节)
        for &dim in &self.shape {
            result.extend_from_slice(&dim.to_le_bytes());
        }

        // 数据长度 + 数据
        result.extend_from_slice(&(self.data.len() as u64).to_le_bytes());
        result.extend_from_slice(&self.data);

        result
    }

    /// 从字节流反序列化
    pub fn Deserialize(bytes: &[u8]) -> Result<Self, TensorPacketError> {
        if bytes.is_empty() {
            return Err(TensorPacketError::InvalidData("空数据".to_string()));
        }

        let mut offset = 0usize;

        // 读取版本号
        let version = bytes[offset];
        offset += 1;
        if version != 1 {
            return Err(TensorPacketError::InvalidData(format!("不支持的版本: {}", version)));
        }

        // 读取名称长度
        if offset + 4 > bytes.len() {
            return Err(TensorPacketError::InvalidData("数据不足: 名称长度".to_string()));
        }
        let name_len = u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]) as usize;
        offset += 4;

        // 读取名称
        if offset + name_len > bytes.len() {
            return Err(TensorPacketError::InvalidData("数据不足: 名称".to_string()));
        }
        let name = String::from_utf8(bytes[offset..offset + name_len].to_vec())
            .map_err(|e| TensorPacketError::InvalidData(format!("名称解码失败: {}", e)))?;
        offset += name_len;

        // 读取数据类型
        if offset + 1 > bytes.len() {
            return Err(TensorPacketError::InvalidData("数据不足: 数据类型".to_string()));
        }
        let dtype = DataType::from_u8(bytes[offset])
            .ok_or_else(|| TensorPacketError::InvalidData(format!("未知数据类型: {}", bytes[offset])))?;
        offset += 1;

        // 读取维度数
        if offset + 4 > bytes.len() {
            return Err(TensorPacketError::InvalidData("数据不足: 维度数".to_string()));
        }
        let ndim = u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]) as usize;
        offset += 4;

        // 读取形状
        if offset + ndim * 8 > bytes.len() {
            return Err(TensorPacketError::InvalidData("数据不足: 形状".to_string()));
        }
        let mut shape = Vec::with_capacity(ndim);
        for _ in 0..ndim {
            let dim = u64::from_le_bytes([
                bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3],
                bytes[offset + 4], bytes[offset + 5], bytes[offset + 6], bytes[offset + 7],
            ]) as usize;
            shape.push(dim);
            offset += 8;
        }

        // 读取数据长度
        if offset + 8 > bytes.len() {
            return Err(TensorPacketError::InvalidData("数据不足: 数据长度".to_string()));
        }
        let data_len = u64::from_le_bytes([
            bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3],
            bytes[offset + 4], bytes[offset + 5], bytes[offset + 6], bytes[offset + 7],
        ]) as usize;
        offset += 8;

        // 读取数据
        if offset + data_len > bytes.len() {
            return Err(TensorPacketError::InvalidData("数据不足: 数据内容".to_string()));
        }
        let data = bytes[offset..offset + data_len].to_vec();

        // 验证数据大小
        let expected_size: usize = shape.iter().product::<usize>() * dtype.Element_Size();
        if data.len() != expected_size {
            return Err(TensorPacketError::InvalidData(format!(
                "数据大小不匹配: 期望 {} 字节, 实际 {} 字节",
                expected_size, data.len()
            )));
        }

        Ok(Self {
            name,
            dtype,
            shape,
            data,
        })
    }

    /// 从 f32 切片创建
    pub fn From_F32_Slice(name: String, shape: Vec<usize>, data: &[f32]) -> Result<Self, TensorPacketError> {
        let expected_len: usize = shape.iter().product();
        if data.len() != expected_len {
            return Err(TensorPacketError::InvalidData(format!(
                "数据长度不匹配: 期望 {} 个元素, 实际 {}",
                expected_len, data.len()
            )));
        }

        let data_bytes: Vec<u8> = data.iter()
            .flat_map(|&f| f.to_le_bytes())
            .collect();

        Ok(Self::new(name, DataType::Float32, shape, data_bytes))
    }

    /// 提取为 f32 向量
    pub fn To_F32_Vec(&self) -> Result<Vec<f32>, TensorPacketError> {
        if self.dtype != DataType::Float32 {
            return Err(TensorPacketError::InvalidData(format!(
                "数据类型不匹配: 期望 Float32, 实际 {:?}",
                self.dtype
            )));
        }

        let element_count = self.len();
        if self.data.len() != element_count * 4 {
            return Err(TensorPacketError::InvalidData("数据大小不匹配".to_string()));
        }

        let mut result = Vec::with_capacity(element_count);
        for i in 0..element_count {
            let offset = i * 4;
            let bytes = [
                self.data[offset],
                self.data[offset + 1],
                self.data[offset + 2],
                self.data[offset + 3],
            ];
            result.push(f32::from_le_bytes(bytes));
        }

        Ok(result)
    }
}

/// Tensor Packet 错误类型
#[derive(Debug)]
pub enum TensorPacketError {
    InvalidData(String),
    IoError(std::io::Error),
}

impl std::fmt::Display for TensorPacketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TensorPacketError::InvalidData(msg) => write!(f, "数据错误: {}", msg),
            TensorPacketError::IoError(e) => write!(f, "IO错误: {}", e),
        }
    }
}

impl std::error::Error for TensorPacketError {}

impl From<std::io::Error> for TensorPacketError {
    fn from(e: std::io::Error) -> Self {
        TensorPacketError::IoError(e)
    }
}

/// Tensor Packet 集合 - 用于多输入/多输出场景
#[derive(Debug, Clone)]
pub struct TensorPacketSet {
    pub tensors: HashMap<String, TensorPacket>,
}

impl TensorPacketSet {
    pub fn new() -> Self {
        Self {
            tensors: HashMap::new(),
        }
    }

    pub fn Insert(&mut self, packet: TensorPacket) {
        self.tensors.insert(packet.name.clone(), packet);
    }

    pub fn Get(&self, name: &str) -> Option<&TensorPacket> {
        self.tensors.get(name)
    }

    /// 序列化整个集合
    /// 格式: [张量数量(4B)] + [每个张量的序列化数据]
    pub fn Serialize(&self) -> Vec<u8> {
        let mut result = Vec::new();

        // 张量数量
        result.extend_from_slice(&(self.tensors.len() as u32).to_le_bytes());

        // 每个张量的序列化数据
        for packet in self.tensors.values() {
            let serialized = packet.Serialize();
            result.extend_from_slice(&(serialized.len() as u64).to_le_bytes());
            result.extend_from_slice(&serialized);
        }

        result
    }

    /// 反序列化整个集合
    pub fn Deserialize(bytes: &[u8]) -> Result<Self, TensorPacketError> {
        if bytes.len() < 4 {
            return Err(TensorPacketError::InvalidData("数据不足".to_string()));
        }

        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let mut offset = 4usize;
        let mut tensors = HashMap::new();

        for _ in 0..count {
            if offset + 8 > bytes.len() {
                return Err(TensorPacketError::InvalidData("数据不足: 张量长度".to_string()));
            }

            let packet_len = u64::from_le_bytes([
                bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3],
                bytes[offset + 4], bytes[offset + 5], bytes[offset + 6], bytes[offset + 7],
            ]) as usize;
            offset += 8;

            if offset + packet_len > bytes.len() {
                return Err(TensorPacketError::InvalidData("数据不足: 张量数据".to_string()));
            }

            let packet = TensorPacket::Deserialize(&bytes[offset..offset + packet_len])?;
            tensors.insert(packet.name.clone(), packet);
            offset += packet_len;
        }

        Ok(Self { tensors })
    }
}

impl Default for TensorPacketSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tensor_packet_serialize_deserialize() {
        let packet = TensorPacket::From_F32_Slice(
            "input".to_string(),
            vec![1, 3, 224, 224],
            &vec![1.0f32; 3 * 224 * 224],
        ).unwrap();

        let serialized = packet.Serialize();
        let deserialized = TensorPacket::Deserialize(&serialized).unwrap();

        assert_eq!(packet.name, deserialized.name);
        assert_eq!(packet.dtype, deserialized.dtype);
        assert_eq!(packet.shape, deserialized.shape);
        assert_eq!(packet.data, deserialized.data);
    }

    #[test]
    fn test_tensor_packet_f32_conversion() {
        let original_data = vec![1.0f32, 2.0, 3.0, 4.0];
        let packet = TensorPacket::From_F32_Slice(
            "test".to_string(),
            vec![2, 2],
            &original_data,
        ).unwrap();

        let recovered = packet.To_F32_Vec().unwrap();
        assert_eq!(original_data, recovered);
    }

    #[test]
    fn test_tensor_packet_set() {
        let mut set = TensorPacketSet::new();

        let packet1 = TensorPacket::From_F32_Slice(
            "input1".to_string(),
            vec![1, 3],
            &vec![1.0f32, 2.0, 3.0],
        ).unwrap();

        let packet2 = TensorPacket::From_F32_Slice(
            "input2".to_string(),
            vec![2, 2],
            &vec![4.0f32, 5.0, 6.0, 7.0],
        ).unwrap();

        set.Insert(packet1);
        set.Insert(packet2);

        let serialized = set.Serialize();
        let deserialized = TensorPacketSet::Deserialize(&serialized).unwrap();

        assert_eq!(deserialized.tensors.len(), 2);
        assert!(deserialized.Get("input1").is_some());
        assert!(deserialized.Get("input2").is_some());
    }
}
