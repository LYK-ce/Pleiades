//Presented by KeJi
//Date ： 2026-04-10

//! 节点身份管理模块
//!
//! 负责生成和持久化节点的密钥对（Ed25519）。
//! 密钥对决定了节点的 PeerId，持久化后确保每次启动使用相同的 PeerId。
//!
//! 存储位置：`.config/keypair.bin`
//! 格式：libp2p protobuf 编码的 Ed25519 密钥对
//!
//! ## 使用方式
//! ```ignore
//! let keypair = Ensure_Identity(config_dir)?;
//! // 传给 Network_Service::Init()
//! ```

#![allow(non_snake_case, non_camel_case_types)]

use libp2p::identity::Keypair;
use std::path::Path;
use tracing::info;

/// 密钥对文件名
const KEYPAIR_FILE: &str = "keypair.bin";

/// 确保节点身份密钥对存在并返回
///
/// - 如果 `.config/keypair.bin` 存在 → 读取并返回
/// - 如果不存在 → 生成新的 Ed25519 密钥对 → 保存到文件 → 返回
///
/// # 参数
/// - `config_dir`: 配置文件目录路径（通常为 `.config/`）
///
/// # 返回
/// `Keypair` — 节点的 Ed25519 密钥对
///
/// # 错误
/// 文件读写失败或密钥编解码失败时返回错误
pub fn Ensure_Identity(config_dir: &Path) -> Result<Keypair, Box<dyn std::error::Error>> {
    let keypair_path = config_dir.join(KEYPAIR_FILE);

    if keypair_path.exists() {
        // 读取已有密钥对
        let bytes = std::fs::read(&keypair_path)?;
        let keypair = Keypair::from_protobuf_encoding(&bytes)
            .map_err(|e| format!("密钥对解析失败: {}", e))?;
        info!("已加载节点密钥对: {}", keypair_path.display());
        Ok(keypair)
    } else {
        // 生成新的 Ed25519 密钥对
        let keypair = Keypair::generate_ed25519();
        // 确保目录存在
        std::fs::create_dir_all(config_dir)?;
        // 保存到文件
        let encoded = keypair
            .to_protobuf_encoding()
            .map_err(|e| format!("密钥对编码失败: {}", e))?;
        std::fs::write(&keypair_path, &encoded)?;
        info!("已生成并保存新的节点密钥对: {}", keypair_path.display());
        Ok(keypair)
    }
}
