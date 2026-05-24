// Presented by KeJi
// Date ： 2026-05-16

pub mod core;
pub mod job;
pub mod command;
pub mod inference_id;
pub mod local_tensor_stream;

use std::sync::Arc;
use crate::network::Network_Capability;
use crate::peer_management::Peer_Management_Capability;
use crate::storage::StorageCapability;
use crate::event_bus::EventBus;
use crate::orchestrator::local_tensor_stream::LocalStreamHub;
use crate::session::SessionManagerHandle;

// ============================================================
// Capabilities — 统一的组件能力容器
// ============================================================

/// 组件能力容器。
///
/// ML Engine 不在其中：
/// - 推理方法 (`load_model`/`forward`/`sample`/...) 是 `MlSession` 的方法 (`&mut self`)，
///   不适合 trait object。由未来 Lua 层通过 `mlua::UserData` 调用。
/// - 分析/切分 (`analyze_model`/`split_model`) 是独立 async 函数，
///   Orchestrator 直接 `use crate::ml_engine::{analyze_model, split_model}` 调用。
pub struct Capabilities {
    /// 网络通信
    pub network: Box<dyn Network_Capability>,
    /// 文件存储
    pub storage: Arc<dyn StorageCapability>,
    /// 节点管理
    pub peer_manager: Box<dyn Peer_Management_Capability>,
    /// 事件总线
    pub event_bus: Arc<EventBus>,
    /// 本地张量流配对 Hub
    pub local_stream_hub: Arc<LocalStreamHub>,
    /// 会话管理器句柄
    pub session_manager: Arc<SessionManagerHandle>,
}

// ============================================================
// 测试辅助模块
// ============================================================

#[cfg(test)]
pub(crate) mod test_utils {
    use async_trait::async_trait;
    use crate::network::{Network_Capability, Network_Error, Network_Data, DataType};
    use crate::peer_management::{Peer_Management_Capability, Peer_Management_Error, PeerInfo};
    use crate::storage::{StorageCapability, StorageError, FileEntry, ChecksumAlgorithm, ReadGuard, WriteGuard};

    // ─── Network stub ──────────────────────────────────────

    pub struct StubNetwork;

    #[async_trait]
    impl Network_Capability for StubNetwork {
        async fn send_data(&self, _peer: libp2p::PeerId, _dt: DataType, _payload: Vec<u8>) -> Result<Network_Data, Network_Error> { unimplemented!("stub") }
        async fn send_response(&self, _id: u64, _dt: DataType, _payload: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn dial(&self, _addr: libp2p::Multiaddr) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn disconnect(&self, _peer: libp2p::PeerId) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn open_file_stream(&self, _peer: libp2p::PeerId) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn send_file_data(&self, _s: &mut libp2p::Stream, _p: &std::path::Path) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn receive_file_data(&self, _s: &mut libp2p::Stream, _p: &std::path::Path, _sz: u64) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn send_file(&self, _peer: libp2p::PeerId, _path: &std::path::Path) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn open_tensor_stream(&self, _peer: libp2p::PeerId, _inference_id: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn accept_tensor_stream(&self, _inference_id: u64, _timeout_secs: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn open_session_stream(&self, _peer: libp2p::PeerId, _session_id: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn put_record(&self, _key: Vec<u8>, _value: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn get_record(&self, _key: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        fn get_local_peer_id(&self) -> libp2p::PeerId { libp2p::PeerId::random() }
        async fn test_bandwidth(&self, _peer: libp2p::PeerId) -> Result<u64, Network_Error> { Ok(0) }
    }

    // ─── PeerManager stub ──────────────────────────────────

    pub struct StubPeerManager {
        inner: crate::peer_management::PeerManager,
    }

    impl StubPeerManager {
        pub fn new() -> Self {
            Self { inner: crate::peer_management::PeerManager::new(libp2p::PeerId::random(), String::new()) }
        }

        /// 注入 mock 节点数据（可选 peer_id，若为 None 则随机生成）
        pub async fn inject_peer(&self, peer_id: Option<libp2p::PeerId>, models: Vec<crate::peer_management::SupportedModel>) {
            let id = peer_id.unwrap_or_else(libp2p::PeerId::random);
            let mut info = crate::peer_management::PeerInfo::new(id, vec![]);
            info.update_supported_models(models);
            self.inner.upsert_peer(info).await;
        }
    }

    #[async_trait]
    impl Peer_Management_Capability for StubPeerManager {
        async fn Get_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> { Ok(self.inner.get_peers().await) }
        async fn Get_All_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> { Ok(self.inner.get_all_peers().await) }
        async fn Get_Local_Peer(&self) -> Result<PeerInfo, Peer_Management_Error> { self.inner.get_local_peer().await.ok_or_else(|| Peer_Management_Error::PeerNotFound("stub".to_string())) }
        async fn Get_Peer_By_Name(&self, name: &str) -> Result<PeerInfo, Peer_Management_Error> { self.inner.get_peer_by_name(name).await.ok_or_else(|| Peer_Management_Error::PeerNotFound(name.to_string())) }
        async fn Get_Peer(&self, peer_id: &libp2p::PeerId) -> Result<PeerInfo, Peer_Management_Error> { self.inner.get_peer(peer_id).await.ok_or_else(|| Peer_Management_Error::PeerNotFound(peer_id.to_string())) }
        async fn Contains_Peer(&self, peer_id: &libp2p::PeerId) -> Result<bool, Peer_Management_Error> { Ok(self.inner.contains_peer(peer_id).await) }
        async fn Count(&self) -> Result<usize, Peer_Management_Error> { Ok(self.inner.count().await) }
        async fn Is_Empty(&self) -> Result<bool, Peer_Management_Error> { Ok(self.inner.is_empty().await) }
        async fn Upsert_Peer(&self, peer_info: PeerInfo) -> Result<(), Peer_Management_Error> { self.inner.upsert_peer(peer_info).await; Ok(()) }
        async fn Set_Local_Name(&self, name: &str) -> Result<(), Peer_Management_Error> { self.inner.set_local_name(name.to_string()).await; Ok(()) }
        async fn Update_Peer_Name(&self, peer_id: &libp2p::PeerId, name: &str) -> Result<(), Peer_Management_Error> { self.inner.update_peer_name(peer_id, name).await; Ok(()) }
        async fn Remove_Peer(&self, peer_id: &libp2p::PeerId) -> Result<PeerInfo, Peer_Management_Error> { self.inner.remove_peer(peer_id).await.ok_or_else(|| Peer_Management_Error::PeerNotFound(peer_id.to_string())) }
        async fn Update_Profile(&self, peer_id: &libp2p::PeerId, profile: crate::peer_management::PeerProfile) -> Result<(), Peer_Management_Error> {
            if self.inner.update_profile(peer_id, profile).await { Ok(()) } else { Err(Peer_Management_Error::PeerNotFound(peer_id.to_string())) }
        }
        async fn Update_Supported_Models(&self, peer_id: &libp2p::PeerId, models: Vec<crate::peer_management::SupportedModel>) -> Result<(), Peer_Management_Error> {
            if self.inner.update_supported_models(peer_id, models).await { Ok(()) } else { Err(Peer_Management_Error::PeerNotFound(peer_id.to_string())) }
        }
        async fn Cleanup_Timeout_Peers(&self, timeout_secs: u64) -> Result<usize, Peer_Management_Error> { Ok(self.inner.cleanup_timeout_peers(timeout_secs).await) }
        async fn Clear(&self) -> Result<(), Peer_Management_Error> { self.inner.clear().await; Ok(()) }
    }

    // ─── Storage stub ──────────────────────────────────────

    pub struct StubStorage;

    #[async_trait]
    impl StorageCapability for StubStorage {
        async fn acquire_read(&self, _file_id: &str) -> Result<(std::path::PathBuf, ReadGuard), StorageError> { unimplemented!("stub") }
        async fn acquire_write(&self, _file_id: &str) -> Result<(std::path::PathBuf, WriteGuard), StorageError> { unimplemented!("stub") }
        async fn remove(&self, _file_id: &str) -> Result<(), StorageError> { unimplemented!("stub") }
        async fn exists(&self, _file_id: &str) -> Result<bool, StorageError> { unimplemented!("stub") }
        async fn list(&self) -> Result<Vec<FileEntry>, StorageError> { Ok(vec![]) }
        async fn checksum(&self, _file_id: &str, _algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError> { unimplemented!("stub") }
        async fn flush(&self) -> Result<(usize, usize), StorageError> { Ok((0, 0)) }
    }
}

// ============================================================
// Profile 内存查询
// ============================================================

/// 查询系统空闲内存（DRAM），返回 MB。
fn query_system_memory_mb() -> u64 {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_memory();
    sys.available_memory() / 1024 / 1024
}

/// 查询 CUDA 空闲显存（VRAM），返回 MB。
fn query_cuda_memory_mb() -> u64 {
    let nvml = match nvml_wrapper::Nvml::init() {
        Ok(nvml) => nvml,
        Err(_) => return 0,
    };
    let dev = match nvml.device_by_index(0) {
        Ok(dev) => dev,
        Err(_) => return 0,
    };
    match dev.memory_info() {
        Ok(info) => info.free / 1024 / 1024,
        Err(_) => 0,
    }
}

/// 按设备查询空闲内存，返回 MB。
pub(crate) fn query_free_memory_mb(device: &str) -> u64 {
    if device.to_lowercase() == "cuda" {
        query_cuda_memory_mb()
    } else {
        query_system_memory_mb()
    }
}
