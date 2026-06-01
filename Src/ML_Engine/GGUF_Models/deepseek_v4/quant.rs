//! FP4 / FP8 weight dequantization — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01

use candle_core::{Device, Result, Tensor};

pub const FP8_BLOCK: usize = 128;
pub const FP4_BLOCK: usize = 32;

pub fn e2m1_decode(nibble: u8) -> f32 {
    const MAG: [f32; 8] = [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0];
    let sign = if nibble & 0x08 != 0 { -1.0 } else { 1.0 };
    sign * MAG[(nibble & 0x07) as usize]
}

pub fn e4m3_decode(byte: u8) -> f32 {
    let sign = if byte & 0x80 != 0 { -1.0 } else { 1.0 };
    let exp = ((byte >> 3) & 0x0F) as i32;
    let mant = (byte & 0x07) as f32;
    if exp == 0 {
        sign * (mant / 8.0) * 2f32.powi(-6)
    } else if exp == 0x0F && mant == 7.0 {
        f32::NAN
    } else {
        sign * (1.0 + mant / 8.0) * 2f32.powi(exp - 7)
    }
}

pub fn e8m0_decode(byte: u8) -> f32 {
    if byte == 0xFF { f32::NAN } else { 2f32.powi(byte as i32 - 127) }
}

pub fn bf16_decode(bits: u16) -> f32 {
    f32::from_bits((bits as u32) << 16)
}

pub fn fp8_weight_dequant(
    weight: &[u8], scale: &[u8],
    rows: usize, cols: usize, block: usize,
    dev: &Device,
) -> Result<Tensor> {
    let sc_cols = cols.div_ceil(block);
    let mut out = Vec::with_capacity(rows * cols);
    for i in 0..rows {
        for j in 0..cols {
            let w = e4m3_decode(weight[i * cols + j]);
            let s = e8m0_decode(scale[(i / block) * sc_cols + (j / block)]);
            out.push(w * s);
        }
    }
    Tensor::from_vec(out, (rows, cols), dev)
}

pub fn fp4_weight_dequant(
    packed: &[u8], scale: &[u8],
    rows: usize, cols: usize, block: usize,
    dev: &Device,
) -> Result<Tensor> {
    let sc_cols = cols / block;
    let packed_cols = cols / 2;
    let mut out = Vec::with_capacity(rows * cols);
    for i in 0..rows {
        for j in 0..cols {
            let byte = packed[i * packed_cols + j / 2];
            let nibble = if j % 2 == 0 { byte & 0x0F } else { byte >> 4 };
            let w = e2m1_decode(nibble);
            let s = e8m0_decode(scale[i * sc_cols + j / block]);
            out.push(w * s);
        }
    }
    Tensor::from_vec(out, (rows, cols), dev)
}
