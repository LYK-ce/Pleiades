//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT KV 记录操作（put_record / get_record）

use libp2p::{
    kad::{self, store::MemoryStore},
    PeerId,
};

/// 写入 DHT KV 记录
pub fn put_record(
    kademlia: &mut kad::Behaviour<MemoryStore>,
    key: &[u8],
    value: Vec<u8>,
    publisher: PeerId,
) {
    let record = kad::Record {
        key: kad::RecordKey::from(key.to_vec()),
        value,
        publisher: Some(publisher),
        expires: None,
    };
    if let Err(e) = kademlia.put_record(record, kad::Quorum::One) {
        tracing::error!("DHT写入失败: {:?}", e);
    } else {
        tracing::info!("DHT写入: {:?}", key);
    }
}

/// 查询 DHT KV 记录（结果经事件异步返回，见 event::handle_event）
pub fn get_record(kademlia: &mut kad::Behaviour<MemoryStore>, key: &[u8]) {
    let record_key = kad::RecordKey::from(key.to_vec());
    kademlia.get_record(record_key);
    tracing::info!("DHT查询: {:?}", key);
}
