// Presented by KeJi
// Date ： 2026-04-24

pub mod core;
pub mod job;
pub mod compiler;
pub mod executor;
pub mod command;
pub mod slot;
pub mod instruction;

use crate::storage::StorageManager;
use crate::llm_io::LLM_IO_Broker;
use crate::network::Network_Capability;
use crate::ml_engine::capability::ML_Engine_Capability;

// 用户界面能力占位符
pub struct UiCapability;

impl UiCapability {
    /// 显示节点列表（占位符）
    pub fn display(&self, _peers: Vec<String>) {
        todo!("实现 display")
    }
}

// 统一的能力结构体，供整个 Orchestrator 层使用
pub struct Capabilities {
    pub storage: StorageManager,
    pub ml_engine: Box<dyn ML_Engine_Capability>,
    pub network: Box<dyn Network_Capability>,
    pub ui: UiCapability,
    pub io_broker: LLM_IO_Broker,
}

impl Capabilities {
    /// 设置计算设备偏好（占位符）
    pub fn set_compute_preference(&self, _device: String) {
        todo!("实现 set_compute_preference")
    }
}

/// 测试辅助模块（供所有 Orchestrator 子模块的测试共用）
#[cfg(test)]
pub(crate) mod test_utils {
    use async_trait::async_trait;
    use crate::network::{Network_Capability, Network_Error};

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
}