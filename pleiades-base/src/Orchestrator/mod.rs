// Presented by KeJi
// Date ： 2026-05-16
// Modified Date ： 2026-08-18

pub mod core;
pub mod job;
pub mod command;
pub mod inference_id;
pub mod local_tensor_stream;

use std::sync::{Arc, RwLock};
use crate::network::Network_Capability;
use crate::peer_management::Peer_Management_Capability;
use crate::storage::StorageCapability;
use crate::event_bus::EventBus;
use crate::orchestrator::local_tensor_stream::LocalStreamHub;
use mlua::Lua;

// ============================================================
// Capabilities — 统一的组件能力容器
// ============================================================

/// 组件能力容器。
///
/// ML Engine 不在其中：
/// - 推理方法 (`load_model`/`forward`/`sample`/...) 是 `MlContext` 的方法 (`&mut self`)，
///   不适合 trait object。由未来 Lua 层通过 `mlua::UserData` 调用。
/// - 分析/切分 (`analyze_model`/`split_model`) 是独立 async 函数，
///   Orchestrator 直接 `use crate::ml_engine::{analyze_model, split_model}` 调用。
/// 设备能力接口：设备端（uav/ugv）实现此 trait，把自己的设备 cap（如 camera.capture）注册进 Lua。
///
/// 与 `Network_Capability` / `StorageCapability` 同为 trait object，风格统一。
pub trait DeviceCapability: Send + Sync {
    fn register_lua_caps(&self, lua: &Lua) -> mlua::Result<()>;
}

pub struct Capabilities {
    /// 网络通信
    pub network: Box<dyn Network_Capability>,
    /// 文件存储
    pub storage: Arc<dyn StorageCapability>,
    /// 节点管理
    pub peer_manager: Arc<dyn Peer_Management_Capability>,
    /// 事件总线
    pub event_bus: Arc<EventBus>,
    /// 本地张量流配对 Hub
    pub local_stream_hub: Arc<LocalStreamHub>,
    /// 设备端注入的设备能力列表（spawn_lua_script 遍历调用 register_lua_caps）
    pub device_caps: RwLock<Vec<Arc<dyn DeviceCapability>>>,
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
        async fn dial_by_peer_id(&self, _peer: libp2p::PeerId) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn disconnect(&self, _peer: libp2p::PeerId) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn open_file_stream(&self, _peer: libp2p::PeerId) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn send_file_data(&self, _s: &mut libp2p::Stream, _p: &std::path::Path) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn receive_file_data(&self, _s: &mut libp2p::Stream, _p: &std::path::Path, _sz: u64) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn send_file(&self, _peer: libp2p::PeerId, _path: &std::path::Path) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn open_tensor_stream(&self, _peer: libp2p::PeerId, _inference_id: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn accept_tensor_stream(&self, _inference_id: u64, _timeout_secs: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn open_session_stream(&self, _peer: &libp2p::PeerId, _session_id: u64) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn put_record(&self, _key: Vec<u8>, _value: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn get_record(&self, _key: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn discover_peers(&self) -> Result<Vec<libp2p::PeerId>, Network_Error> { unimplemented!("stub") }
        fn get_local_peer_id(&self) -> libp2p::PeerId { libp2p::PeerId::random() }
        async fn publish_gossipsub(&self, _topic: &str, _payload: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn test_bandwidth(&self, _peer: libp2p::PeerId) -> Result<u64, Network_Error> { Ok(0) }
    }

    // ─── Storage stub ──────────────────────────────────────

    pub struct StubStorage;

    #[async_trait]
    impl StorageCapability for StubStorage {
        async fn Acquire_Read(&self, _file_id: &str) -> Result<(std::path::PathBuf, ReadGuard), StorageError> { unimplemented!("stub") }
        async fn Acquire_Write(&self, _file_id: &str) -> Result<(std::path::PathBuf, WriteGuard), StorageError> { unimplemented!("stub") }
        async fn Remove(&self, _file_id: &str) -> Result<(), StorageError> { unimplemented!("stub") }
        async fn Exists(&self, _file_id: &str) -> Result<bool, StorageError> { unimplemented!("stub") }
        async fn List(&self) -> Result<Vec<FileEntry>, StorageError> { Ok(vec![]) }
        async fn Checksum(&self, _file_id: &str, _algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError> { unimplemented!("stub") }
        async fn Flush(&self) -> Result<(usize, usize), StorageError> { Ok((0, 0)) }
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
    if device.to_lowercase().starts_with("cuda") {
        query_cuda_memory_mb()
    } else {
        query_system_memory_mb()
    }
}
