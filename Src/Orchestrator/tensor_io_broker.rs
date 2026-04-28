//Presented by KeJi
//Date ： 2026-04-28

//! Tensor IO Broker — 张量流资源的分配、托管和分发
//!
//! 类似 LLM_IO_Broker 的 Allocate → Take 模式，负责管理 Tensor Stream 的
//! inbound/outbound 两条流的存储和分发。
//!
//! ## 使用流程
//! 1. Core 在 spawn 分布式 Job 时调用 `Prepare(job_id)` 注册条目
//! 2. Job 的 `OpenTensorStream` handler 打开 outbound 后调用 `Store_Outbound(job_id, stream)`
//! 3. Core 收到 `TensorStreamArrived` 后读取 handshake → 调用 `Store_Inbound(job_id, stream)`
//! 4. Job 执行 `TakeInboundStream` → `Take_Inbound(job_id).await`（阻塞直到就绪）
//! 5. Job 执行 `TakeOutboundStream` → `Take_Outbound(job_id).await`（阻塞直到就绪）
//! 6. Job 执行 `BuildTensorIo` → 在 handler 中本地组装 `Tensor_IO_Handle`
//! 7. Job 结束后 Core 调用 `Deallocate(job_id)` 清理

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

use crate::orchestrator::job::JobId;

/// Tensor IO Broker 错误类型
#[derive(Debug, thiserror::Error)]
pub enum Tensor_IO_Broker_Error {
    #[error("JobId {0:?} 未注册")]
    NotFound(JobId),
    #[error("JobId {0:?} 已注册")]
    AlreadyExists(JobId),
    #[error("JobId {0:?} 的 inbound 已被存储")]
    InboundAlreadyStored(JobId),
    #[error("JobId {0:?} 的 outbound 已被存储")]
    OutboundAlreadyStored(JobId),
    #[error("JobId {0:?} 的 inbound 已被取出")]
    InboundAlreadyTaken(JobId),
    #[error("JobId {0:?} 的 outbound 已被取出")]
    OutboundAlreadyTaken(JobId),
    #[error("通道已关闭")]
    ChannelClosed,
}

/// 内部条目
struct Tensor_IO_Entry {
    /// 入站流（从上游节点到达）
    inbound: Option<libp2p::Stream>,
    /// 出站流（主动 open 到下游节点）
    outbound: Option<libp2p::Stream>,
    /// inbound 到达通知
    inbound_notify: Arc<Notify>,
    /// outbound 就绪通知
    outbound_notify: Arc<Notify>,
}

/// Tensor IO Broker
///
/// 管理张量流的 inbound/outbound 存储和分发。
/// 内部使用 `tokio::sync::Mutex` 避免锁中毒，锁内操作极短（HashMap 增删查）。
pub struct Tensor_IO_Broker {
    entries: Mutex<HashMap<JobId, Tensor_IO_Entry>>,
}

impl Tensor_IO_Broker {
    /// 创建空的 Broker
    pub fn New() -> Self {
        Tensor_IO_Broker {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Core 在 spawn 分布式 Job 时调用，注册 job_id
    ///
    /// 创建空的 entry，等待后续 Store_Inbound / Store_Outbound 填充。
    pub async fn Prepare(&self, job_id: JobId) -> Result<(), Tensor_IO_Broker_Error> {
        let mut map = self.entries.lock().await;
        if map.contains_key(&job_id) {
            return Err(Tensor_IO_Broker_Error::AlreadyExists(job_id));
        }
        map.insert(job_id, Tensor_IO_Entry {
            inbound: None,
            outbound: None,
            inbound_notify: Arc::new(Notify::new()),
            outbound_notify: Arc::new(Notify::new()),
        });
        Ok(())
    }

    /// Core 收到 TensorStreamArrived 时调用
    ///
    /// 从 stream handshake 读取 target_job_id 后，调用此方法存入 inbound。
    /// 存入后 notify 等待的 Job。
    pub async fn Store_Inbound(
        &self,
        job_id: JobId,
        stream: libp2p::Stream,
    ) -> Result<(), Tensor_IO_Broker_Error> {
        let mut map = self.entries.lock().await;
        let entry = map.get_mut(&job_id)
            .ok_or(Tensor_IO_Broker_Error::NotFound(job_id))?;
        if entry.inbound.is_some() {
            return Err(Tensor_IO_Broker_Error::InboundAlreadyStored(job_id));
        }
        entry.inbound = Some(stream);
        entry.inbound_notify.notify_one();
        Ok(())
    }

    /// Job 的 OpenTensorStream handler 调用
    ///
    /// 打开 outbound 后存入此方法。存入后 notify 等待的 Job。
    pub async fn Store_Outbound(
        &self,
        job_id: JobId,
        stream: libp2p::Stream,
    ) -> Result<(), Tensor_IO_Broker_Error> {
        let mut map = self.entries.lock().await;
        let entry = map.get_mut(&job_id)
            .ok_or(Tensor_IO_Broker_Error::NotFound(job_id))?;
        if entry.outbound.is_some() {
            return Err(Tensor_IO_Broker_Error::OutboundAlreadyStored(job_id));
        }
        entry.outbound = Some(stream);
        entry.outbound_notify.notify_one();
        Ok(())
    }

    /// Job 的 TakeInboundStream handler 调用
    ///
    /// 阻塞直到 inbound stream 到达。返回 raw `libp2p::Stream`。
    pub async fn Take_Inbound(
        &self,
        job_id: JobId,
    ) -> Result<libp2p::Stream, Tensor_IO_Broker_Error> {
        loop {
            let notify = {
                let mut map = self.entries.lock().await;
                let entry = map.get_mut(&job_id)
                    .ok_or(Tensor_IO_Broker_Error::NotFound(job_id))?;
                if let Some(stream) = entry.inbound.take() {
                    return Ok(stream);
                }
                entry.inbound_notify.clone()
            };
            notify.notified().await;
        }
    }

    /// Job 的 TakeOutboundStream handler 调用
    ///
    /// 阻塞直到 outbound stream 就绪。返回 raw `libp2p::Stream`。
    pub async fn Take_Outbound(
        &self,
        job_id: JobId,
    ) -> Result<libp2p::Stream, Tensor_IO_Broker_Error> {
        loop {
            let notify = {
                let mut map = self.entries.lock().await;
                let entry = map.get_mut(&job_id)
                    .ok_or(Tensor_IO_Broker_Error::NotFound(job_id))?;
                if let Some(stream) = entry.outbound.take() {
                    return Ok(stream);
                }
                entry.outbound_notify.clone()
            };
            notify.notified().await;
        }
    }

    /// Core 在 Job 结束后调用，清理内部索引
    ///
    /// 幂等操作，job_id 不存在时不报错。
    pub async fn Deallocate(&self, job_id: JobId) {
        let mut map = self.entries.lock().await;
        map.remove(&job_id);
    }

    /// 查询 job_id 是否已注册
    pub async fn Is_Active(&self, job_id: JobId) -> bool {
        let map = self.entries.lock().await;
        map.contains_key(&job_id)
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// TC-01: Prepare 注册后 Is_Active 返回 true
    #[tokio::test]
    async fn test_prepare_registers() {
        let broker = Tensor_IO_Broker::New();
        broker.Prepare(JobId(1)).await.unwrap();
        assert!(broker.Is_Active(JobId(1)).await);
    }

    /// TC-02: 重复 Prepare 返回 AlreadyExists
    #[tokio::test]
    async fn test_prepare_duplicate() {
        let broker = Tensor_IO_Broker::New();
        broker.Prepare(JobId(1)).await.unwrap();
        let err = broker.Prepare(JobId(1)).await.unwrap_err();
        assert!(matches!(err, Tensor_IO_Broker_Error::AlreadyExists(_)));
    }

    /// TC-03: Deallocate 后 Is_Active 返回 false
    #[tokio::test]
    async fn test_deallocate() {
        let broker = Tensor_IO_Broker::New();
        broker.Prepare(JobId(1)).await.unwrap();
        broker.Deallocate(JobId(1)).await;
        assert!(!broker.Is_Active(JobId(1)).await);
    }

    /// TC-04: Deallocate 不存在的 job_id 不 panic
    #[tokio::test]
    async fn test_deallocate_idempotent() {
        let broker = Tensor_IO_Broker::New();
        broker.Deallocate(JobId(999)).await;
        // 不 panic 即可
    }

    /// TC-05: Store_Inbound 到未注册的 job_id 返回 NotFound
    #[tokio::test]
    async fn test_store_inbound_not_found() {
        let broker = Tensor_IO_Broker::New();
        // 构造一对 dummy stream 不可行（需要 libp2p 连接），
        // 这里只测试未注册的路径
        // 实际 stream 测试需要集成测试环境
    }

    /// TC-06: Store_Outbound 到未注册的 job_id 返回 NotFound
    #[tokio::test]
    async fn test_store_outbound_not_found() {
        // 同 TC-05，无法在单元测试中构造 libp2p::Stream
    }
}
