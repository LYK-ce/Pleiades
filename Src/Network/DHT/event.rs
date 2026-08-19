//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT Kademlia 事件处理

use libp2p::kad;

use super::provider::ProviderQueryTracker;

/// 处理 Kademlia 事件（含 GetProviders 结果累积与回传）
pub fn handle_event(event: kad::Event, queries: &mut ProviderQueryTracker) {
    match event {
        kad::Event::OutboundQueryProgressed { id, result, .. } => match result {
            kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FoundRecord(peer_record))) => {
                tracing::info!("DHT记录查询成功: {:?}", peer_record.record.key);
            }
            kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FinishedWithNoAdditionalRecord { .. })) => {
                tracing::debug!("DHT记录查询完成，无更多记录");
            }
            kad::QueryResult::GetRecord(Err(e)) => {
                tracing::warn!("DHT记录查询失败: {:?}", e);
            }
            kad::QueryResult::PutRecord(Ok(_)) => {
                tracing::info!("DHT记录写入成功");
            }
            kad::QueryResult::PutRecord(Err(e)) => {
                tracing::error!("DHT记录写入失败: {:?}", e);
            }
            kad::QueryResult::Bootstrap(Ok(_)) => {
                tracing::info!("Kademlia引导成功");
            }
            kad::QueryResult::Bootstrap(Err(e)) => {
                tracing::warn!("Kademlia引导失败: {:?}", e);
            }
            kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FoundProviders { providers, .. })) => {
                tracing::debug!("DHT 发现 {} 个新 provider", providers.len());
                queries.accumulate(&id, providers);
            }
            kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. })) => {
                tracing::info!("DHT provider 查询完成");
                queries.finish_ok(&id);
            }
            kad::QueryResult::GetProviders(Err(e)) => {
                tracing::warn!("DHT 查询 provider 失败: {:?}", e);
                queries.finish_err(&id, format!("{:?}", e));
            }
            kad::QueryResult::StartProviding(Ok(_)) => {
                tracing::info!("DHT provider 自注册成功");
            }
            kad::QueryResult::StartProviding(Err(e)) => {
                tracing::warn!("DHT provider 自注册失败: {:?}", e);
            }
            kad::QueryResult::RepublishProvider(Ok(_)) => {
                tracing::debug!("DHT provider 重新发布成功");
            }
            kad::QueryResult::RepublishProvider(Err(e)) => {
                tracing::warn!("DHT provider 重新发布失败: {:?}", e);
            }
            _ => {}
        },
        kad::Event::RoutingUpdated { peer, .. } => {
            tracing::debug!("路由表更新: {}", peer);
        }
        _ => {}
    }
}
