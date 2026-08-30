//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT（Kademlia）节点发现模块
//!
//! 集中存放 DHT 相关逻辑，按职责拆分：
//! - mod.rs：模块声明 + 命名空间常量与 key 构造
//! - record.rs：KV 记录操作（put_record / get_record）
//! - provider.rs：provider 记录操作 + 查询跟踪
//! - event.rs：Kademlia 事件处理
//!
//! 注意：真正操作 swarm 的代码仍由 Network_Service 持有，本模块只提供
//! 以 `kad::Behaviour` 为参数的封装函数。

mod event;
mod provider;
mod record;

pub use event::handle_event;
pub use provider::{get_providers, start_providing, ProviderQueryTracker};
pub use record::{get_record, put_record};

use libp2p::kad;

/// 默认节点发现命名空间：所有 Pleiades 节点都注册为该 key 的 provider
/// 实际值由 config.toml 的 [Network].dht_namespace 决定，此常量仅作默认值。
pub const DEFAULT_NODE_NAMESPACE: &str = "pleiades-nodes";

/// 构造节点发现命名空间对应的 DHT RecordKey
pub fn node_namespace_key(namespace: &str) -> kad::RecordKey {
    kad::RecordKey::from(namespace.as_bytes().to_vec())
}
