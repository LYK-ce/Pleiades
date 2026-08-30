//! Safetensors loader from in-memory bytes — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01
//!
//! Key difference from MScanter: reads from `&[u8]` instead of mmap'ing a file,
//! because safetensors data comes from a PGGUF tensor blob.

use std::collections::HashMap;
use candle_core::{DType, Device, Result, Tensor};
use serde_json::Value;

use super::quant::{bf16_decode, fp4_weight_dequant, fp8_weight_dequant, FP4_BLOCK, FP8_BLOCK};

fn err(msg: String) -> candle_core::Error {
    candle_core::Error::Msg(msg)
}

#[derive(Debug, Clone)]
pub struct TensorInfo {
    pub dtype: String,
    pub shape: Vec<usize>,
    pub begin: usize,
    pub end: usize,
}

pub struct SafeTensors {
    data: Vec<u8>,
    tensors: HashMap<String, TensorInfo>,
}

impl SafeTensors {
    /// Parse safetensors from in-memory bytes (a single safetensors shard).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let data = bytes.to_vec();
        let tensors = parse_header(&data)?;
        Ok(Self { data, tensors })
    }

    pub fn tensor_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.tensors.keys().map(|s| s.as_str()).collect();
        v.sort_unstable();
        v
    }

    pub fn info(&self, name: &str) -> Option<&TensorInfo> {
        self.tensors.get(name)
    }

    pub fn raw(&self, name: &str) -> Result<&[u8]> {
        let info = self.tensors.get(name)
            .ok_or_else(|| err(format!("tensor not found: {name}")))?;
        Ok(&self.data[info.begin..info.end])
    }

    pub fn f32_tensor(&self, name: &str, shape: &[usize], dev: &Device) -> Result<Tensor> {
        let bytes = self.raw(name)?;
        let n: usize = shape.iter().product();
        if bytes.len() != n * 4 {
            return Err(err(format!(
                "{name}: expected {} F32 bytes for shape {shape:?}, found {}",
                n * 4, bytes.len()
            )));
        }
        let vals: Vec<f32> = bytes.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Tensor::from_vec(vals, shape.to_vec(), dev)?.to_dtype(DType::BF16)
    }

    pub fn bf16_tensor(&self, name: &str, shape: &[usize], dev: &Device) -> Result<Tensor> {
        let bytes = self.raw(name)?;
        let n: usize = shape.iter().product();
        if bytes.len() != n * 2 {
            return Err(err(format!(
                "{name}: expected {} BF16 bytes for shape {shape:?}, found {}",
                n * 2, bytes.len()
            )));
        }
        let vals: Vec<f32> = bytes.chunks_exact(2)
            .map(|c| bf16_decode(u16::from_le_bytes([c[0], c[1]])))
            .collect();
        Tensor::from_vec(vals, shape.to_vec(), dev)?.to_dtype(DType::BF16)
    }

    pub fn auto_tensor(&self, name: &str, shape: &[usize], dev: &Device) -> Result<Tensor> {
        let info = self.tensors.get(name)
            .ok_or_else(|| err(format!("tensor not found: {name}")))?;
        match info.dtype.as_str() {
            "F32" => self.f32_tensor(name, shape, dev),
            "BF16" => self.bf16_tensor(name, shape, dev),
            other => Err(err(format!(
                "{name}: auto_tensor handles F32/BF16, not {other}"
            ))),
        }
    }

    pub fn fp8_tensor(
        &self, prefix: &str, rows: usize, cols: usize, block: usize, dev: &Device,
    ) -> Result<Tensor> {
        let weight = self.raw(&format!("{prefix}.weight"))?;
        let scale = self.raw(&format!("{prefix}.scale"))?;
        let want_scale = rows.div_ceil(block) * cols.div_ceil(block);
        if weight.len() != rows * cols || scale.len() != want_scale {
            return Err(err(format!(
                "{prefix}: FP8 [{rows},{cols}] block {block} wants {} weight + {want_scale} scale bytes, found {} + {}",
                rows * cols, weight.len(), scale.len()
            )));
        }
        fp8_weight_dequant(weight, scale, rows, cols, block, dev)?.to_dtype(DType::BF16)
    }

    pub fn fp4_tensor(
        &self, prefix: &str, rows: usize, cols: usize, block: usize, dev: &Device,
    ) -> Result<Tensor> {
        let weight = self.raw(&format!("{prefix}.weight"))?;
        let scale = self.raw(&format!("{prefix}.scale"))?;
        let want_scale = rows * (cols / block);
        if weight.len() != rows * cols / 2 || scale.len() != want_scale {
            return Err(err(format!(
                "{prefix}: FP4 [{rows},{cols}] block {block} wants {} packed + {want_scale} scale bytes, found {} + {}",
                rows * cols / 2, weight.len(), scale.len()
            )));
        }
        fp4_weight_dequant(weight, scale, rows, cols, block, dev)?.to_dtype(DType::BF16)
    }

    pub fn linear(
        &self, prefix: &str, rows: usize, cols: usize, fp4: bool, dev: &Device,
    ) -> Result<Tensor> {
        if self.info(&format!("{prefix}.scale")).is_some() {
            if fp4 {
                self.fp4_tensor(prefix, rows, cols, FP4_BLOCK, dev)
            } else {
                self.fp8_tensor(prefix, rows, cols, FP8_BLOCK, dev)
            }
        } else {
            self.auto_tensor(&format!("{prefix}.weight"), &[rows, cols], dev)
        }
    }
}

/// Multi-shard safetensors: wraps multiple `SafeTensors` and delegates lookups.
pub struct MultiSafeTensors {
    shards: Vec<SafeTensors>,
}

impl MultiSafeTensors {
    /// Build from multiple safetensors shard bytes.
    pub fn from_shards(shard_bytes: &[Vec<u8>]) -> Result<Self> {
        let shards = shard_bytes.iter()
            .map(|b| SafeTensors::from_bytes(b))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { shards })
    }

    /// Find the shard that contains `name`.
    fn find(&self, name: &str) -> Result<&SafeTensors> {
        for shard in &self.shards {
            if shard.info(name).is_some() {
                return Ok(shard);
            }
        }
        Err(err(format!("tensor not found in any shard: {name}")))
    }

    pub fn info(&self, name: &str) -> Option<&TensorInfo> {
        self.shards.iter().find_map(|s| s.info(name))
    }

    pub fn auto_tensor(&self, name: &str, shape: &[usize], dev: &Device) -> Result<Tensor> {
        self.find(name)?.auto_tensor(name, shape, dev)
    }

    pub fn linear(&self, prefix: &str, rows: usize, cols: usize, fp4: bool, dev: &Device) -> Result<Tensor> {
        self.find(&format!("{prefix}.weight"))?.linear(prefix, rows, cols, fp4, dev)
    }
}

fn parse_header(buf: &[u8]) -> Result<HashMap<String, TensorInfo>> {
    if buf.len() < 8 {
        return Err(err("file too small for a safetensors header".into()));
    }
    let header_len = u64::from_le_bytes(buf[0..8].try_into().unwrap()) as usize;
    let data_start = 8 + header_len;
    if buf.len() < data_start {
        return Err(err("header length exceeds file size".into()));
    }
    let json: Value = serde_json::from_slice(&buf[8..data_start])
        .map_err(|e| err(e.to_string()))?;
    let obj = json.as_object()
        .ok_or_else(|| err("header is not a JSON object".into()))?;

    let mut tensors = HashMap::new();
    for (name, v) in obj {
        if name == "__metadata__" { continue; }
        let dtype = v.get("dtype").and_then(Value::as_str)
            .ok_or_else(|| err(format!("{name}: missing dtype")))?.to_string();
        let shape = v.get("shape").and_then(Value::as_array)
            .ok_or_else(|| err(format!("{name}: missing shape")))?
            .iter().map(|d| d.as_u64().map(|x| x as usize)
                .ok_or_else(|| err(format!("{name}: non-integer shape dim"))))
            .collect::<Result<Vec<usize>>>()?;
        let offs = v.get("data_offsets").and_then(Value::as_array)
            .ok_or_else(|| err(format!("{name}: missing data_offsets")))?;
        let [rb, re] = offs.as_slice() else {
            return Err(err(format!("{name}: data_offsets must have 2 entries")));
        };
        let begin = data_start + rb.as_u64()
            .ok_or_else(|| err(format!("{name}: bad data_offset")))? as usize;
        let end = data_start + re.as_u64()
            .ok_or_else(|| err(format!("{name}: bad data_offset")))? as usize;
        if begin > end || end > buf.len() {
            return Err(err(format!("{name}: data_offsets out of range")));
        }
        tensors.insert(name.clone(), TensorInfo { dtype, shape, begin, end });
    }
    Ok(tensors)
}
