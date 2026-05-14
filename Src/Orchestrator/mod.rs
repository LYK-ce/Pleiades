// Presented by KeJi
// Date ： 2026-05-04

pub mod core;
pub mod job;
pub mod program_selector;
pub mod command;
pub mod inference_id;
pub mod orchestrator_vm;

use std::sync::Arc;
use crate::storage::StorageManager;
use crate::llm_io::LLM_IO_Broker;
use crate::network::Network_Capability;
use crate::ml_engine::capability::ML_Engine_Capability;
use crate::peer_management::Peer_Management_Capability;
use crate::scheduler::Scheduler_Capability;
use crate::event_bus::EventBus;
use crate::tensor_io::Tensor_Port_Switch;

// 统一的能力结构体，供整个 Orchestrator 层使用
pub struct Capabilities {
    pub storage: Arc<StorageManager>,
    pub ml_engine: Box<dyn ML_Engine_Capability>,
    pub network: Box<dyn Network_Capability>,
    pub peer_manager: Box<dyn Peer_Management_Capability>,
    pub scheduler: Box<dyn Scheduler_Capability>,
    pub event_bus: Arc<EventBus>,
    pub io_broker: Arc<LLM_IO_Broker>,
    /// 张量流热切换管理器（"先连接后启动"模式）
    pub tensor_switch: Arc<Tensor_Port_Switch>,
}

/// 测试辅助模块（供所有 Orchestrator 子模块的测试共用）
#[cfg(test)]
pub(crate) mod test_utils {
    use async_trait::async_trait;
    use crate::network::{Network_Capability, Network_Error};
    use crate::peer_management::{Peer_Management_Capability, Peer_Management_Error};
    use crate::peer_management::{PeerInfo, PeerStatus, PeerCapability};
    use crate::scheduler::{Scheduler_Capability, Scheduler_Error, Scheduler_Input, Pipeline_Plan};

    /// Network_Capability 的空桩实现（用于不实际调用网络的单元测试）
    pub struct StubNetwork;

    #[async_trait]
    impl Network_Capability for StubNetwork {
        async fn send_data(&self, _peer: libp2p::PeerId, _dt: crate::network::DataType, _payload: Vec<u8>) -> Result<crate::network::Network_Data, Network_Error> { unimplemented!("stub") }
        async fn send_response(&self, _id: u64, _dt: crate::network::DataType, _payload: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn dial(&self, _addr: libp2p::Multiaddr) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn disconnect(&self, _peer: libp2p::PeerId) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn open_file_stream(&self, _peer: libp2p::PeerId) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn send_file_data(&self, _s: &mut libp2p::Stream, _p: &std::path::Path) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn receive_file_data(&self, _s: &mut libp2p::Stream, _p: &std::path::Path, _sz: u64) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn open_tensor_stream(&self, _peer: libp2p::PeerId) -> Result<libp2p::Stream, Network_Error> { unimplemented!("stub") }
        async fn put_record(&self, _key: Vec<u8>, _value: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        async fn get_record(&self, _key: Vec<u8>) -> Result<(), Network_Error> { unimplemented!("stub") }
        fn get_local_peer_id(&self) -> libp2p::PeerId { libp2p::PeerId::random() }
        async fn test_bandwidth(&self, _peer: libp2p::PeerId) -> Result<u64, Network_Error> { Ok(0) }
    }

    /// Peer_Management_Capability 的空桩实现（用于不实际调用节点管理的单元测试）
    pub struct StubPeerManager;

    #[async_trait]
    impl Peer_Management_Capability for StubPeerManager {
        async fn List_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> { Ok(vec![]) }
        async fn Get_Peer(&self, _peer_id: &libp2p::PeerId) -> Result<PeerInfo, Peer_Management_Error> { Err(Peer_Management_Error::PeerNotFound("stub".to_string())) }
        async fn Contains_Peer(&self, _peer_id: &libp2p::PeerId) -> Result<bool, Peer_Management_Error> { Ok(false) }
        async fn Get_Idle_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> { Ok(vec![]) }
        async fn Count(&self) -> Result<usize, Peer_Management_Error> { Ok(0) }
        async fn Add_Peer(&self, _peer_info: PeerInfo) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Remove_Peer(&self, _peer_id: &libp2p::PeerId) -> Result<PeerInfo, Peer_Management_Error> { Err(Peer_Management_Error::PeerNotFound("stub".to_string())) }
        async fn Update_Status(&self, _peer_id: &libp2p::PeerId, _status: PeerStatus) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Heartbeat(&self, _peer_id: &libp2p::PeerId, _latency_ms: Option<u64>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Capability(&self, _peer_id: &libp2p::PeerId, _capability: Option<PeerCapability>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Bandwidth(&self, _peer_id: &libp2p::PeerId, _bandwidth_mbps: Option<u64>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Cleanup_Timeout_Peers(&self, _timeout_secs: u64) -> Result<usize, Peer_Management_Error> { Ok(0) }
        async fn Clear(&self) -> Result<(), Peer_Management_Error> { Ok(()) }
    }

    /// Scheduler_Capability 的空桩实现（返回空 Plan，无 Worker）
    pub struct StubScheduler;

    #[async_trait]
    impl Scheduler_Capability for StubScheduler {
        async fn Plan_Pipeline(&self, input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error> {
            // 默认退化为单机模式（无 Worker）
            Ok(Pipeline_Plan {
                inference_id: input.inference_id,
                coord_layer_start: 0,
                coord_layer_end: input.model_info.num_layers,
                coord_outbound_target: None,
                workers: vec![],
            })
        }
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
