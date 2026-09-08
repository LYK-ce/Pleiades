//Presented by KeJi
//Date ： 2026-05-18

//! LuaTensor — candle_core::Tensor 的 mlua UserData 包装
//!
//! 使 Tensor 可以安全地跨 Lua 边界传递，Lua 脚本中表现为 userdata 对象。
//! 通过 Deref 自动解引用，Rust 侧使用时零开销。

use candle_core::{Device, Tensor};
use std::ops::Deref;

use super::device::Parse_Device_Str;

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
        let device = Parse_Device_Str(s)?;
        self.0
            .to_device(&device)
            .map(LuaTensor)
            .map_err(|e| format!("to_device: {e}"))
    }
}

/// 将 Tensor 序列化为字节数组（含 shape header + dtype）。
///
/// 格式: `[dtype: u8][ndim: u64 LE][d0: u64 LE]...[dn: u64 LE][raw data]`
/// dtype: 0=F32, 1=U32, 2=U8
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
        candle_core::DType::U8 => 2,
        other => return Err(format!("tensor_to_bytes: unsupported dtype {:?}", other)),
    };

    let header_size = 1 + 8 + shape.len() * 8;
    let elem_size: usize = match dtype {
        0 | 1 => 4,
        2 => 1,
        _ => unreachable!(),
    };
    let mut buf = Vec::with_capacity(header_size + total * elem_size);
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
        2 => {
            let flat: Vec<u8> = t_flat.to_vec1().map_err(|e| format!("to_vec1: {e}"))?;
            buf.extend_from_slice(&flat);
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
    let elem_size: usize = match dtype {
        0 | 1 => 4,
        2 => 1,
        _ => return Err(format!("bytes_to_tensor: unknown dtype {}", dtype)),
    };
    let expected_bytes = elem_count * elem_size;
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
        2 => Tensor::from_vec(raw_data.to_vec(), &shape[..], device)
            .map_err(|e| format!("tensor from_vec (u8): {e}")),
        _ => Err(format!("bytes_to_tensor: unknown dtype {}", dtype)),
    }
}

// ─── Device 解析 ──────────────────────────────────────────
//
// Parse_Device_Str() 已移至 Src/ML_Engine/device.rs，
// 此处通过 use super::device::Parse_Device_Str 引用。

/// bytes_to_tensor 的字符串入口 — 供绑定层使用。
pub fn bytes_to_tensor_str(data: &[u8], device_str: &str) -> Result<Tensor, String> {
    let device = Parse_Device_Str(device_str)?;
    bytes_to_tensor(data, &device)
}

// ─── 裸字节 ↔ U8 Tensor（与上面的序列化/反序列化是两套不同语义）───
//
// 区别对照（务必区分，名字相近、方向相反）：
// - tensor_to_bytes / tensor_from_bytes(=bytes_to_tensor_str) 是「序列化对」：
//   带 [dtype][ndim][dims] header，用于跨网络恢复任意 dtype 张量。
// - tensor_from_u8_bytes_fn / tensor_to_u8_bytes_fn 是「裸字节 wrap/unwrap 对」：
//   无 header、不解析，仅把任意 u8 字节（如 JPEG 图片）包成 1D U8 Tensor 再原样取回，
//   用于把非张量的字节数据塞进 tensor stream 通道（tensor stream 只接受 LuaTensor）。

/// 裸字节 → 1D U8 Tensor（无 header，不解析）。
///
/// 与 `bytes_to_tensor_str`（反序列化带 header 的字节）方向相反、语义不同：
/// 这里输入是任意原始 u8 字节（如 JPEG），直接包成 shape=[N] 的 U8 Tensor。
/// 与 `tensor_to_u8_bytes_fn` 成对（wrap/unwrap）。
pub fn tensor_from_u8_bytes_fn(bytes: &[u8], device_str: &str) -> Result<LuaTensor, String> {
    let device = Parse_Device_Str(device_str)?;
    let t = Tensor::from_vec(bytes.to_vec(), &[bytes.len()], &device)
        .map_err(|e| format!("tensor_from_u8_bytes: {e}"))?;
    Ok(LuaTensor(t))
}

/// U8 Tensor → 纯字节（无 header，与 `tensor_from_u8_bytes_fn` 成对）。
///
/// 与 `LuaTensor::to_bytes`（带 header 的序列化）不同：这里用 `to_vec1::<u8>`
/// 直接取回原始字节，不附加任何 header。仅接受 U8 dtype，其余 dtype 报错。
pub fn tensor_to_u8_bytes_fn(t: &Tensor) -> Result<Vec<u8>, String> {
    if t.dtype() != candle_core::DType::U8 {
        return Err(format!(
            "tensor_to_u8_bytes: expected U8 tensor, got {:?}",
            t.dtype()
        ));
    }
    t.flatten_all()
        .map_err(|e| format!("tensor_to_u8_bytes: {e}"))?
        .to_vec1::<u8>()
        .map_err(|e| format!("tensor_to_u8_bytes: {e}"))
}
