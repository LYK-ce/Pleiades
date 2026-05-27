//Presented by KeJi
//Date ： 2026-05-18

//! LuaTensor — candle_core::Tensor 的 mlua UserData 包装
//!
//! 使 Tensor 可以安全地跨 Lua 边界传递，Lua 脚本中表现为 userdata 对象。
//! 通过 Deref 自动解引用，Rust 侧使用时零开销。

use candle_core::{Device, Tensor};
use std::ops::Deref;

/// Tensor 的 Lua 包装。
///
/// 内存开销：Tensor 内部是 `Arc<Storage>`，LuaTensor 仅多一层 newtype，
/// 构造和传递均为指针复制，无数据拷贝。
pub struct LuaTensor(pub Tensor);

impl Deref for LuaTensor {
    type Target = Tensor;

    fn deref(&self) -> &Tensor {
        &self.0
    }
}

impl LuaTensor {
    /// Tensor → 字节序列化。
    ///
    /// 格式: `[ndim: u64 LE][d0: u64 LE]...[dn: u64 LE][f32 LE raw data]`
    /// 可在任意设备上运行（CPU/CUDA），candle 会处理数据传输。
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        tensor_to_bytes(&self.0)
    }

    /// 将 Tensor 迁移到指定设备（接受字符串）。
    pub fn to_device_str(&self, s: &str) -> Result<LuaTensor, String> {
        let device = parse_device_str(s)?;
        self.0
            .to_device(&device)
            .map(LuaTensor)
            .map_err(|e| format!("to_device: {e}"))
    }
}

/// 将 Tensor 序列化为字节数组（含 shape header + dtype）。
///
/// 格式: `[dtype: u8][ndim: u64 LE][d0: u64 LE]...[dn: u64 LE][raw data]`
/// dtype: 0=F32, 1=U32
pub fn tensor_to_bytes(t: &Tensor) -> Result<Vec<u8>, String> {
    let shape = t.dims().to_vec();
    if shape.is_empty() || shape.iter().any(|&d| d == 0) {
        return Err("tensor_to_bytes: empty shape".into());
    }
    let total: usize = shape.iter().product();
    let t_flat = t.reshape(&[total]).map_err(|e| format!("reshape: {e}"))?;

    let dtype: u8 = match t.dtype() {
        candle_core::DType::F32 => 0,
        candle_core::DType::U32 => 1,
        other => return Err(format!("tensor_to_bytes: unsupported dtype {:?}", other)),
    };

    let header_size = 1 + 8 + shape.len() * 8;
    let mut buf = Vec::with_capacity(header_size + total * 4);
    buf.push(dtype);
    buf.extend_from_slice(&(shape.len() as u64).to_le_bytes());
    for &d in &shape {
        buf.extend_from_slice(&(d as u64).to_le_bytes());
    }

    match dtype {
        0 => {
            let flat: Vec<f32> = t_flat.to_vec1().map_err(|e| format!("to_vec1: {e}"))?;
            let f32_bytes =
                unsafe { std::slice::from_raw_parts(flat.as_ptr() as *const u8, flat.len() * 4) };
            buf.extend_from_slice(f32_bytes);
        }
        1 => {
            let flat: Vec<u32> = t_flat.to_vec1().map_err(|e| format!("to_vec1: {e}"))?;
            let u32_bytes =
                unsafe { std::slice::from_raw_parts(flat.as_ptr() as *const u8, flat.len() * 4) };
            buf.extend_from_slice(u32_bytes);
        }
        _ => unreachable!(),
    }
    Ok(buf)
}

/// 从字节数组反序列化为 Tensor。
///
/// 读取 `tensor_to_bytes` 的格式，在指定设备上重建 Tensor。
pub fn bytes_to_tensor(data: &[u8], device: &Device) -> Result<Tensor, String> {
    if data.len() < 9 {
        return Err("bytes_to_tensor: data too short for header".into());
    }
    let dtype = data[0];
    let ndim = u64::from_le_bytes(data[1..9].try_into().unwrap()) as usize;
    let header_size = 9 + ndim * 8;
    if data.len() < header_size {
        return Err("bytes_to_tensor: data too short for dims".into());
    }
    let mut shape: Vec<usize> = Vec::with_capacity(ndim);
    for i in 0..ndim {
        let start = 9 + i * 8;
        let d = u64::from_le_bytes(data[start..start + 8].try_into().unwrap());
        shape.push(d as usize);
    }
    let raw_data = &data[header_size..];
    let elem_count: usize = shape.iter().product();
    let expected_bytes = elem_count * 4;
    if raw_data.len() != expected_bytes {
        return Err(format!(
            "bytes_to_tensor: data size mismatch: expected {expected_bytes}, got {}",
            raw_data.len()
        ));
    }

    match dtype {
        0 => {
            let f32_slice: &[f32] =
                unsafe { std::slice::from_raw_parts(raw_data.as_ptr() as *const f32, elem_count) };
            Tensor::from_vec(f32_slice.to_vec(), &shape[..], device)
                .map_err(|e| format!("tensor from_vec: {e}"))
        }
        1 => {
            let u32_slice: &[u32] =
                unsafe { std::slice::from_raw_parts(raw_data.as_ptr() as *const u32, elem_count) };
            Tensor::from_vec(u32_slice.to_vec(), &shape[..], device)
                .map_err(|e| format!("tensor from_vec: {e}"))
        }
        _ => Err(format!("bytes_to_tensor: unknown dtype {}", dtype)),
    }
}

// ─── Device 解析 ──────────────────────────────────────────

/// 将 Lua 设备字符串解析为 candle Device。
pub(crate) fn parse_device_str(s: &str) -> Result<Device, String> {
    match s.to_lowercase().as_str() {
        "cpu" => Ok(Device::Cpu),
        "cuda" => Device::new_cuda(0).map_err(|e| format!("cuda unavailable: {e}")),
        other => Err(format!("unknown device: {other}")),
    }
}

/// bytes_to_tensor 的字符串入口 — 供绑定层使用。
pub fn bytes_to_tensor_str(data: &[u8], device_str: &str) -> Result<Tensor, String> {
    let device = parse_device_str(device_str)?;
    bytes_to_tensor(data, &device)
}
