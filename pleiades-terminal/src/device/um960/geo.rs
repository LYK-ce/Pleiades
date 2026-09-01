//Presented by KeJi
//Created Date ： 2026-08-31
//Modified Date ： 2026-08-31

//! UM960 基站侧 GNSS 纯函数（Task 24）
//!
//! 照搬 `uploads/lg290p_enu.py` L27-94 的 `parse_gga` / `ecef` / `enu`，Rust 化（f64 坐标）。
//! 与 ugv 侧 `device/lg290p/geo.rs` **内容相同但物理独立**（D3，不共享）。

/// WGS84 椭球长半轴（米）
pub const WGS84_A: f64 = 6_378_137.0;
/// WGS84 椭球扁率
pub const WGS84_F: f64 = 1.0 / 298.257_223_563;
/// WGS84 第一偏心率平方
pub const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);
/// 默认大地水准面差距（北京地区，米）
pub const DEFAULT_GEOID: f64 = -9.535;

/// 解析后的 GGA 定位结果
#[derive(Debug, Clone)]
pub struct GgaFix {
    /// UTC 时间（HHMMSS.sss）
    pub utc: String,
    /// 纬度（十进制度，S 为负）
    pub lat: f64,
    /// 经度（十进制度，W 为负）
    pub lon: f64,
    /// 质量码：0 无效 / 1 单点 / 2 DGPS / 4 RTK_FIXED / 5 RTK_FLOAT / 6 推算 / 7 基站原点
    pub quality: u8,
    /// 卫星数
    pub satellites: u8,
    /// 水平精度因子
    pub hdop: f64,
    /// 海拔（MSL，米）
    pub alt_msl: f64,
    /// 大地水准面差距（米）
    pub geoid: f64,
    /// 差分龄期（秒）
    pub age: f64,
}

/// 解析一行 NMEA GGA。
///
/// `$` 开头 + `*` 切分 → XOR 校验 → 仅接受 `GNGGA`/`GPGGA` → 度分转十进制度（S/W 取负）。
/// 校验失败 / 数值异常 / 下标越界均返回 `None`。
pub fn parse_gga(line: &str) -> Option<GgaFix> {
    let line = line.trim();
    if !line.starts_with('$') || !line.contains('*') {
        return None;
    }
    let (payload, supplied) = line[1..].split_once('*')?;
    // XOR 校验
    let checksum: u8 = payload.bytes().fold(0, |acc, b| acc ^ b);
    let supplied_val = u8::from_str_radix(supplied.get(0..2)?, 16).ok()?;
    if checksum != supplied_val {
        return None;
    }

    let fields: Vec<&str> = payload.split(',').collect();
    let msg_type = *fields.first()?;
    if (msg_type != "GNGGA" && msg_type != "GPGGA")
        || fields.get(2).map_or(true, |s| s.is_empty())
        || fields.get(4).map_or(true, |s| s.is_empty())
    {
        return None;
    }

    // 度分转度：dddmm.mmmm → ddd + mm.mmmm/60；S/W 半球取负
    let degrees = |value: &str, hemisphere: &str| -> Option<f64> {
        let raw: f64 = value.parse().ok()?;
        let whole = (raw / 100.0).floor();
        let result = whole + (raw - whole * 100.0) / 60.0;
        Some(if hemisphere == "S" || hemisphere == "W" {
            -result
        } else {
            result
        })
    };

    Some(GgaFix {
        utc: fields.get(1).copied().unwrap_or("").to_string(),
        lat: degrees(fields.get(2)?, fields.get(3).copied().unwrap_or(""))?,
        lon: degrees(fields.get(4)?, fields.get(5).copied().unwrap_or(""))?,
        quality: fields.get(6).and_then(|s| s.parse().ok()).unwrap_or(0),
        satellites: fields.get(7).and_then(|s| s.parse().ok()).unwrap_or(0),
        hdop: fields.get(8).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN),
        alt_msl: fields.get(9).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN),
        geoid: fields.get(11).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN),
        age: fields.get(13).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN),
    })
}

/// WGS84 经纬高 → ECEF（米）。`height` 为椭球高（= alt_msl + geoid）。
pub fn ecef(lat_deg: f64, lon_deg: f64, height: f64) -> (f64, f64, f64) {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let radius = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    (
        (radius + height) * cos_lat * lon.cos(),
        (radius + height) * cos_lat * lon.sin(),
        (radius * (1.0 - WGS84_E2) + height) * sin_lat,
    )
}

/// 以 base 为原点，返回 rover 的相对 `(east, north, up)`（米）。
pub fn enu(
    base_lat: f64,
    base_lon: f64,
    base_height: f64,
    lat: f64,
    lon: f64,
    height: f64,
) -> (f64, f64, f64) {
    let (x0, y0, z0) = ecef(base_lat, base_lon, base_height);
    let (x, y, z) = ecef(lat, lon, height);
    let dx = x - x0;
    let dy = y - y0;
    let dz = z - z0;
    let lat0 = base_lat.to_radians();
    let lon0 = base_lon.to_radians();
    let east = -lon0.sin() * dx + lon0.cos() * dy;
    let north = -lat0.sin() * lon0.cos() * dx - lat0.sin() * lon0.sin() * dy + lat0.cos() * dz;
    let up = lat0.cos() * lon0.cos() * dx + lat0.cos() * lon0.sin() * dy + lat0.sin() * dz;
    (east, north, up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gga_self_test_sample() {
        let sample = "$GNGGA,063450.200,4005.23407724,N,11613.24217273,E,5,32,0.75,41.438,M,-9.535,M,0.2,0000*70";
        let fix = parse_gga(sample).expect("应解析成功");
        assert_eq!(fix.quality, 5);
        assert!((fix.lat - 40.0872346207).abs() < 1e-9);
        assert_eq!(fix.satellites, 32);
    }

    #[test]
    fn test_parse_gga_bad_checksum() {
        // 篡改校验和 → 拒绝
        let sample = "$GNGGA,063450.200,4005.23407724,N,11613.24217273,E,5,32,0.75,41.438,M,-9.535,M,0.2,0000*71";
        assert!(parse_gga(sample).is_none());
    }

    #[test]
    fn test_enu_zero_displacement() {
        let (e, n, u) = enu(40.0, 116.0, 10.0, 40.0, 116.0, 10.0);
        assert!(e.abs() < 1e-6);
        assert!(n.abs() < 1e-6);
        assert!(u.abs() < 1e-6);
    }

    #[test]
    fn test_enu_north_1m() {
        // 纬度 +0.00001 度 ≈ 向北 1.11 米
        let (_, n, _) = enu(0.0, 0.0, 0.0, 0.00001, 0.0, 0.0);
        assert!(n > 1.10 && n < 1.12, "north = {n}");
    }
}
