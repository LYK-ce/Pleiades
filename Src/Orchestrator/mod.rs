// Presented by KeJi
// Date ： 2026-04-30

pub mod core;
pub mod job;
pub mod compiler;
pub mod executor;
pub mod command;
pub mod slot;
pub mod instruction;
pub mod tensor_io_broker;

use std::sync::Arc;
use crate::storage::StorageManager;
use crate::llm_io::LLM_IO_Broker;
use crate::network::Network_Capability;
use crate::ml_engine::capability::ML_Engine_Capability;
use crate::peer_management::Peer_Management_Capability;
use crate::event_bus::EventBus;
use tensor_io_broker::Tensor_IO_Broker;

// 统一的能力结构体，供整个 Orchestrator 层使用
pub struct Capabilities {
    pub storage: Arc<StorageManager>,
    pub ml_engine: Box<dyn ML_Engine_Capability>,
    pub network: Box<dyn Network_Capability>,
    pub peer_manager: Box<dyn Peer_Management_Capability>,
    pub event_bus: Arc<EventBus>,
    pub io_broker: Arc<LLM_IO_Broker>,
    pub tensor_io_broker: Tensor_IO_Broker,
}

/// 测试辅助模块（供所有 Orchestrator 子模块的测试共用）
#[cfg(test)]
pub(crate) mod test_utils {
    use async_trait::async_trait;
    use crate::network::{Network_Capability, Network_Error};
    use crate::peer_management::{Peer_Management_Capability, Peer_Management_Error};
    use crate::peer_management::{PeerInfo, PeerStatus, PeerCapability};

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
        async fn Get_All_Peer_Ids(&self) -> Result<Vec<libp2p::PeerId>, Peer_Management_Error> { Ok(vec![]) }
        async fn Add_Peer(&self, _peer_info: PeerInfo) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Remove_Peer(&self, _peer_id: &libp2p::PeerId) -> Result<PeerInfo, Peer_Management_Error> { Err(Peer_Management_Error::PeerNotFound("stub".to_string())) }
        async fn Update_Status(&self, _peer_id: &libp2p::PeerId, _status: PeerStatus) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Heartbeat(&self, _peer_id: &libp2p::PeerId, _latency_ms: Option<u64>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Capability(&self, _peer_id: &libp2p::PeerId, _capability: Option<PeerCapability>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Update_Bandwidth(&self, _peer_id: &libp2p::PeerId, _bandwidth_mbps: Option<u64>) -> Result<(), Peer_Management_Error> { Ok(()) }
        async fn Cleanup_Timeout_Peers(&self, _timeout_secs: u64) -> Result<usize, Peer_Management_Error> { Ok(0) }
        async fn Clear(&self) -> Result<(), Peer_Management_Error> { Ok(()) }
    }
}