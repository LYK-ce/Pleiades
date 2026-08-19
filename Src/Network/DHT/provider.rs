//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT provider 记录操作（start_providing / get_providers）+ 查询跟踪

use libp2p::{
    kad::{self, store::MemoryStore},
    PeerId,
};
use std::collections::{HashMap, HashSet};
use tokio::sync::oneshot;

use super::node_namespace_key;

/// 自注册为 namespace 的 provider（供 get_providers 发现）
pub fn start_providing(kademlia: &mut kad::Behaviour<MemoryStore>, namespace: &str) {
    match kademlia.start_providing(node_namespace_key(namespace)) {
        Ok(_query_id) => tracing::info!("DHT 自注册成功: {}", namespace),
        Err(e) => tracing::warn!("DHT 自注册失败: {:?}", e),
    }
}

/// 查询 namespace 的 provider 列表，返回 QueryId（结果经事件回传）
pub fn get_providers(kademlia: &mut kad::Behaviour<MemoryStore>, namespace: &str) -> kad::QueryId {
    kademlia.get_providers(node_namespace_key(namespace))
}

/// 进行中的 get_providers 查询跟踪表
///
/// `get_providers` 返回 QueryId，结果分多批经 swarm 事件异步返回
/// （`FoundProviders` 累积 + `FinishedWithNoAdditionalRecord` 终结），
/// 这里用 QueryId 关联 oneshot 通道，累积完成后把结果回传给调用方。
pub struct ProviderQueryTracker {
    pending: HashMap<kad::QueryId, PendingQuery>,
}

/// 单次查询的中间状态
struct PendingQuery {
    /// 已发现的 provider 集合（跨多个 FoundProviders 事件累积）
    providers: HashSet<PeerId>,
    /// 结果回传通道
    reply: Option<oneshot::Sender<Result<Vec<PeerId>, String>>>,
}

impl ProviderQueryTracker {
    pub fn new() -> Self {
        Self { pending: HashMap::new() }
    }

    /// 注册一次查询（发起 get_providers 时调用）
    pub fn register(&mut self, qid: kad::QueryId, tx: oneshot::Sender<Result<Vec<PeerId>, String>>) {
        self.pending.insert(qid, PendingQuery {
            providers: HashSet::new(),
            reply: Some(tx),
        });
    }

    /// 累积一批 provider（FoundProviders 事件时调用）
    pub fn accumulate(&mut self, qid: &kad::QueryId, providers: impl IntoIterator<Item = PeerId>) {
        if let Some(pq) = self.pending.get_mut(qid) {
            pq.providers.extend(providers);
        }
    }

    /// 查询完成，回传累积结果（FinishedWithNoAdditionalRecord 事件时调用）
    pub fn finish_ok(&mut self, qid: &kad::QueryId) {
        if let Some(pq) = self.pending.remove(qid) {
            if let Some(tx) = pq.reply {
                let list: Vec<PeerId> = pq.providers.into_iter().collect();
                let _ = tx.send(Ok(list));
            }
        }
    }

    /// 查询失败，回传错误（GetProviders(Err) 事件时调用）
    pub fn finish_err(&mut self, qid: &kad::QueryId, err: String) {
        if let Some(pq) = self.pending.remove(qid) {
            if let Some(tx) = pq.reply {
                let _ = tx.send(Err(err));
            }
        }
    }
}
