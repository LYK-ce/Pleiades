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
}

/// 将 Tensor 序列化为字节数组（含 shape header）。
///
/// 格式: `[ndim: u64 LE][d0: u64 LE]...[dn: u64 LE][f32 LE raw data]`
pub fn tensor_to_bytes(t: &Tensor) -> Result<Vec<u8>, String> {
    let shape = t.dims().to_vec();
    if shape.is_empty() || shape.iter().any(|&d| d == 0) {
        return Err("tensor_to_bytes: empty shape".into());
    }
    let total: usize = shape.iter().product();
    let t_flat = t.reshape(&[total]).map_err(|e| format!("reshape: {e}"))?;
    let flat: Vec<f32> = t_flat.to_vec1().map_err(|e| format!("to_vec1: {e}"))?;

    let header_size = 8 + shape.len() * 8;
    let mut buf = Vec::with_capacity(header_size + flat.len() * 4);
    buf.extend_from_slice(&(shape.len() as u64).to_le_bytes());
    for &d in &shape {
        buf.extend_from_slice(&(d as u64).to_le_bytes());
    }
    // SAFETY: f32 array reinterpreted as [u8], well-aligned and properly sized
    let f32_bytes =
        unsafe { std::slice::from_raw_parts(flat.as_ptr() as *const u8, flat.len() * 4) };
    buf.extend_from_slice(f32_bytes);
    Ok(buf)
}

/// 从字节数组反序列化为 Tensor。
///
/// 读取 `tensor_to_bytes` 的格式，在指定设备上重建 Tensor。
pub fn bytes_to_tensor(data: &[u8], device: &Device) -> Result<Tensor, String> {
    if data.len() < 8 {
        return Err("bytes_to_tensor: data too short for header".into());
    }
    let ndim = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
    let header_size = 8 + ndim * 8;
    if data.len() < header_size {
        return Err("bytes_to_tensor: data too short for dims".into());
    }
    let mut shape: Vec<usize> = Vec::with_capacity(ndim);
    for i in 0..ndim {
        let start = 8 + i * 8;
        let d = u64::from_le_bytes(data[start..start + 8].try_into().unwrap());
        shape.push(d as usize);
    }
    let f32_data = &data[header_size..];
    let elem_count: usize = shape.iter().product();
    let expected_bytes = elem_count * 4;
    if f32_data.len() != expected_bytes {
        return Err(format!(
            "bytes_to_tensor: data size mismatch: expected {expected_bytes}, got {}",
            f32_data.len()
        ));
    }
    // SAFETY: f32 byte slice aligned and properly sized
    let f32_slice: &[f32] =
        unsafe { std::slice::from_raw_parts(f32_data.as_ptr() as *const f32, elem_count) };
    Tensor::from_vec(f32_slice.to_vec(), &shape[..], device)
        .map_err(|e| format!("tensor from_vec: {e}"))
}

impl mlua::UserData for LuaTensor {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("dims", |lua, this, (): ()| {
            let dims = this.dims();
            let t = lua.create_table()?;
            for (i, &d) in dims.iter().enumerate() {
                t.set(i + 1, d)?;
            }
            Ok(t)
        });

        methods.add_method("to_bytes", |_, this, (): ()| {
            let bytes = this.to_bytes().map_err(|e| mlua::Error::runtime(e))?;
            Ok(bytes)
        });

        methods.add_method("to_device", |_, this, device_str: String| {
            let device = match device_str.to_lowercase().as_str() {
                "cpu" => Device::Cpu,
                "cuda" => match Device::new_cuda(0) {
                    Ok(d) => d,
                    Err(e) => return Err(mlua::Error::runtime(format!("cuda unavailable: {e}"))),
                },
                other => return Err(mlua::Error::runtime(format!("unknown device: {other}"))),
            };
            let moved = this.to_device(&device)
                .map_err(|e| mlua::Error::runtime(format!("to_device: {e}")))?;
            Ok(LuaTensor(moved))
        });
    }
}
