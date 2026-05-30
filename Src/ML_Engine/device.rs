//Presented by KeJi
//Date : 2026-05-30

//! 设备解析 — 将字符串转换为 candle Device
//!
//! ML Engine 对外暴露的唯一设备解析入口。
//! Lua VM 层、TUI 层、Orchestrator 层不应自行解析设备字符串，
//! 统一通过此模块进行转换。

use candle_core::Device;

/// 解析设备字符串，支持：
/// - "cpu"              → Device::Cpu
/// - "cuda" / "cuda:0"  → Device::new_cuda(0)
/// - "cuda:N"           → Device::new_cuda(N)
pub fn parse_device_str(s: &str) -> Result<Device, String> {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "cpu" => Ok(Device::Cpu),
        _ if s == "cuda" || s == "cuda:0" => {
            Device::new_cuda(0).map_err(|e| format!("cuda:0 unavailable: {e}"))
        }
        _ if s.starts_with("cuda:") => {
            let idx: usize = s[5..]
                .parse()
                .map_err(|_| format!("invalid cuda device index: '{s}'"))?;
            Device::new_cuda(idx).map_err(|e| format!("cuda:{idx} unavailable: {e}"))
        }
        _ => Err(format!(
            "unknown device: '{s}'. Use 'cpu', 'cuda', or 'cuda:N'"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu() {
        let d = parse_device_str("cpu").unwrap();
        assert!(d.is_cpu());
    }

    #[test]
    fn test_invalid() {
        assert!(parse_device_str("tpu").is_err());
        assert!(parse_device_str("cuda:abc").is_err());
    }

    #[test]
    #[cfg(feature = "cuda")]
    fn test_cuda_default() {
        let d = parse_device_str("cuda").unwrap();
        assert!(!d.is_cpu());
    }

    #[test]
    #[cfg(feature = "cuda")]
    fn test_cuda_0() {
        let d = parse_device_str("cuda:0").unwrap();
        assert!(!d.is_cpu());
    }

    #[test]
    #[cfg(feature = "cuda")]
    fn test_cuda_named() {
        let d = parse_device_str("cuda:1").unwrap();
        assert!(!d.is_cpu());
    }
}
