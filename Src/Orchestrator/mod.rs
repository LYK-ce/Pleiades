// Presented by KeJi
// Date ： 2026-05-16

pub mod core;
pub mod job;
pub mod command;
pub mod inference_id;

use std::sync::Arc;
use crate::network::Network_Capability;
use crate::peer_management::Peer_Management_Capability;
use crate::storage::StorageCapability;
use crate::session::Session_Capability;
use crate::event_bus::EventBus;

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
    /// 会话管理 (IO 通道)
    pub session: Box<dyn Session_Capability>,
    /// 事件总线
    pub event_bus: Arc<EventBus>,
}

// ============================================================
// 测试辅助模块
// ============================================================

#[cfg(test)]
pub(crate) mod test_utils {
    use async_trait::async_trait;
    use crate::network::{Network_Capability, Network_Error, Network_Data, DataType};
    use crate::peer_management::{Peer_Management_Capability, Peer_Management_Error, PeerInfo, PeerStatus};
    use crate::storage::{StorageCapability, StorageError, FileEntry, ChecksumAlgorithm, ReadGuard, WriteGuard};
    use crate::session::{Session_Capability, Session_Error, IoHandle, IoFrontend, SessionInfo};

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
        async fn open_tensor_stream(&self, _peer: libp2p::PeerId, _inference_id: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn accept_tensor_stream(&self, _inference_id: u64, _timeout_secs: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn put_record(&self, _key: Vec<u8>, _value: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn get_record(&self, _key: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        fn get_local_peer_id(&self) -> libp2p::PeerId { libp2p::PeerId::random() }
        async fn test_bandwidth(&self, _peer: libp2p::PeerId) -> Result<u64, Network_Error> { Ok(0) }
    }

    // ─── PeerManager stub ──────────────────────────────────

    pub struct StubPeerManager;

    #[async_trait]
    impl Peer_Management_Capability for StubPeerManager {
        async fn Get_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> { Ok(vec![]) }
        async fn Get_Peer(&self, _peer_id: &libp2p::PeerId) -> Result<PeerInfo, Peer_Management_Error> { Err(Peer_Management_Error::PeerNotFound("stub".to_string())) }
        async fn Contains_Peer(&self, _peer_id: &libp2p::PeerId) -> Result<bool, Peer_Management_Error> { Ok(false) }
        async fn Count(&self) -> Result<usize, Peer_Management_Error> { Ok(0) }
        async fn Is_Empty(&self) -> Result<bool, Peer_Management_Error> { Ok(true) }
        async fn Upsert_Peer(&self, _peer_info: PeerInfo) {}
        async fn Remove_Peer(&self, _peer_id: &libp2p::PeerId) -> Result<PeerInfo, Peer_Management_Error> { Err(Peer_Management_Error::PeerNotFound("stub".to_string())) }
        async fn Update_Profile(&self, _peer_id: &libp2p::PeerId, _profile: crate::peer_management::PeerProfile) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Supported_Models(&self, _peer_id: &libp2p::PeerId, _models: Vec<crate::peer_management::SupportedModel>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Cleanup_Timeout_Peers(&self, _timeout_secs: u64) -> Result<usize, Peer_Management_Error> { Ok(0) }
        async fn Clear(&self) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Heartbeat(&self, _peer_id: &libp2p::PeerId, _latency_ms: Option<u64>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Status(&self, _peer_id: &libp2p::PeerId, _status: PeerStatus) -> Result<(), Peer_Management_Error> { Ok(()) }
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

    // ─── Session stub ──────────────────────────────────────

    pub struct StubSession;

    #[async_trait]
    impl Session_Capability for StubSession {
        async fn create_session(&self, _model_id: String) -> Result<(String, IoHandle), Session_Error> { unimplemented!("stub") }
        async fn destroy_session(&self, _session_id: &str) -> Result<(), Session_Error> { unimplemented!("stub") }
        async fn connect(&self, _session_id: &str) -> Result<(u32, IoFrontend), Session_Error> { unimplemented!("stub") }
        fn list_sessions(&self) -> Vec<SessionInfo> { vec![] }
        async fn release_slot(&self, _session_id: &str, _slot_id: u32) -> Result<(), Session_Error> { unimplemented!("stub") }
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
