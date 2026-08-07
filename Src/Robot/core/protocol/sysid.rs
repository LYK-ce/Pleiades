//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-08-07

//! sysid 派生（协议文档 §4.2）
//!
//! sysid = peer_id 的 multihash 字节数组最后一字节（0~255）
//! - 确定性：同一辆车恒得同一 sysid，无需配置
//! - 碰撞容忍：不参与路由，碰撞无影响
//! - 本模块不依赖 libp2p 类型，调用方传 `PeerId::as_bytes()` 即可

/// 从 multihash 字节派生 sysid（取末字节）
pub fn sysid_from_multihash(multihash: &[u8]) -> u8 {
    *multihash.last().unwrap_or(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sysid_from_multihash() {
        // 末字节即 sysid
        assert_eq!(sysid_from_multihash(&[1, 2, 3, 42]), 42);
        assert_eq!(sysid_from_multihash(&[0xFF]), 0xFF);
        assert_eq!(sysid_from_multihash(&[0x00]), 0x00);
        // 空输入回退 0
        assert_eq!(sysid_from_multihash(&[]), 0);
    }
}
