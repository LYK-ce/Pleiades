// Presented by KeJi
// Date ： 2026-04-23

use crate::orchestrator::job::JobId;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use super::capability::{IoChannels, IoFrontend, IoHandle, LLM_IO_Capability, LLM_IO_Error};

// ─── 常量 ───────────────────────────────────────────────────

/// 通道缓冲大小
const CHANNEL_BUFFER_SIZE: usize = 64;

// ─── 内部结构 ───────────────────────────────────────────────

/// 内部通道条目，保留 Sender 引用用于加速通道关闭。
/// 当 `Deallocate` 移除条目时，这些 Sender 随之 Drop，
/// 减少通道的引用计数，配合两端句柄的 Drop 最终关闭通道。
struct ChannelEntry {
    /// 保留前端 input 侧的 Sender 引用
    _frontend_input_tx: mpsc::Sender<String>,
    /// 保留 ML output 侧的 Sender 引用
    _ml_output_tx: mpsc::Sender<String>,
}

// ─── LLM_IO_Broker ─────────────────────────────────────────

/// LLM_IO 代理，负责通道的分配、回收和状态查询。
///
/// 内部使用 `tokio::sync::Mutex` 以避免锁中毒问题。
/// 锁内操作极短（HashMap 增删查），不会长期阻塞异步运行时。
///
/// **deallocate 语义**：仅清理内部索引，通道的实际生命周期
/// 由两端句柄（`IoFrontend` / `IoHandle`）的 Drop 决定。
pub struct LLM_IO_Broker {
    channels: Mutex<HashMap<JobId, ChannelEntry>>,
}

impl LLM_IO_Broker {
    /// 创建空索引表
    pub fn New() -> Self {
        LLM_IO_Broker {
            channels: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl LLM_IO_Capability for LLM_IO_Broker {
    async fn Allocate(&self, job_id: JobId) -> Result<IoChannels, LLM_IO_Error> {
        let mut map = self.channels.lock().await;

        // 若 job_id 已存在，拒绝分配
        if map.contains_key(&job_id) {
            return Err(LLM_IO_Error::AllocationFailed(
                format!("job_id {:?} already has an active channel", job_id),
            ));
        }

        // 1. 创建 input 通道 (前端 → ML)
        let (input_tx, input_rx) = mpsc::channel::<String>(CHANNEL_BUFFER_SIZE);
        // 2. 创建 output 通道 (ML → 前端)
        let (output_tx, output_rx) = mpsc::channel::<String>(CHANNEL_BUFFER_SIZE);

        // 3. 组装前端侧和 ML 侧端点
        let frontend = IoFrontend {
            input_tx: input_tx.clone(),
            output_rx,
        };
        let ml_side = IoHandle {
            input_rx,
            output_tx: output_tx.clone(),
        };

        // 4. 在索引中插入 ChannelEntry（保留两份 Sender 用于后续清理）
        let entry = ChannelEntry {
            _frontend_input_tx: input_tx,
            _ml_output_tx: output_tx,
        };
        map.insert(job_id, entry);

        // 5. 返回 IoChannels
        Ok(IoChannels { frontend, ml_side })
    }

    async fn Deallocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error> {
        let mut map = self.channels.lock().await;
        // 移除对应条目（若存在），幂等操作。
        // 仅清理内部索引，通道的实际生命周期由两端句柄的 Drop 决定。
        map.remove(&job_id);
        Ok(())
    }

    async fn Is_Active(&self, job_id: JobId) -> bool {
        let map = self.channels.lock().await;
        map.contains_key(&job_id)
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 验证 allocate 返回的 IoChannels 两端非空，Sender/Receiver 可正常收发
    #[tokio::test]
    async fn test_allocate_returns_valid_channels() {
        let broker = LLM_IO_Broker::New();
        let channels = broker.Allocate(JobId(1)).await.unwrap();

        let mut frontend = channels.frontend;
        let mut ml_side = channels.ml_side;

        // 前端发送 Prompt
        frontend.input_tx.send("hello".to_string()).await.unwrap();
        // ML 侧接收 Prompt
        let prompt = ml_side.input_rx.recv().await.unwrap();
        assert_eq!(prompt, "hello");

        // ML 侧发送 Completion
        ml_side.output_tx.send("world".to_string()).await.unwrap();
        // 前端接收 Completion
        let completion = frontend.output_rx.recv().await.unwrap();
        assert_eq!(completion, "world");
    }

    // 验证 allocate 后 is_active(job_id) 返回 true
    #[tokio::test]
    async fn test_allocate_registers_in_index() {
        let broker = LLM_IO_Broker::New();
        let _channels = broker.Allocate(JobId(10)).await.unwrap();
        assert!(broker.Is_Active(JobId(10)).await);
    }

    // 验证 deallocate 后 is_active(job_id) 返回 false
    #[tokio::test]
    async fn test_deallocate_removes_from_index() {
        let broker = LLM_IO_Broker::New();
        let _channels = broker.Allocate(JobId(20)).await.unwrap();
        assert!(broker.Is_Active(JobId(20)).await);

        broker.Deallocate(JobId(20)).await.unwrap();
        assert!(!broker.Is_Active(JobId(20)).await);
    }

    // 验证对不存在的 job_id 调用 deallocate 不 panic，返回 Ok
    #[tokio::test]
    async fn test_deallocate_idempotent() {
        let broker = LLM_IO_Broker::New();
        // 对不存在的 job_id deallocate
        let result = broker.Deallocate(JobId(999)).await;
        assert!(result.is_ok());
    }

    // 验证前端 IoFrontend Drop + deallocate 后，ML 侧的 input_rx.recv() 返回 None
    #[tokio::test]
    async fn test_frontend_drop_closes_ml_input_rx() {
        let broker = LLM_IO_Broker::New();
        let channels = broker.Allocate(JobId(30)).await.unwrap();

        let frontend = channels.frontend;
        let mut ml_side = channels.ml_side;

        // 先 deallocate 移除 ChannelEntry 中的 Sender 克隆
        broker.Deallocate(JobId(30)).await.unwrap();
        // 再 drop 前端（移除最后一个 input_tx）
        drop(frontend);

        // ML 侧接收应返回 None
        let result = ml_side.input_rx.recv().await;
        assert!(result.is_none());
    }

    // 验证 IoHandle Drop + deallocate 后，前端的 output_rx.recv() 返回 None
    #[tokio::test]
    async fn test_ml_side_drop_closes_frontend_output_rx() {
        let broker = LLM_IO_Broker::New();
        let channels = broker.Allocate(JobId(40)).await.unwrap();

        let mut frontend = channels.frontend;
        let ml_side = channels.ml_side;

        // 先 deallocate 移除 ChannelEntry 中的 Sender 克隆
        broker.Deallocate(JobId(40)).await.unwrap();
        // 再 drop ML 侧（移除最后一个 output_tx）
        drop(ml_side);

        // 前端接收应返回 None
        let result = frontend.output_rx.recv().await;
        assert!(result.is_none());
    }

    // 验证对同一 job_id 重复 allocate 时返回 AllocationFailed 错误
    #[tokio::test]
    async fn test_duplicate_allocate_returns_error() {
        let broker = LLM_IO_Broker::New();

        // 第一次分配成功
        let _channels = broker.Allocate(JobId(50)).await.unwrap();
        assert!(broker.Is_Active(JobId(50)).await);

        // 第二次分配同一 job_id，应返回错误
        let result = broker.Allocate(JobId(50)).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            LLM_IO_Error::AllocationFailed(msg) => {
                assert!(msg.contains("already has an active channel"));
            }
        }
    }

    // 验证 deallocate 后可重新 allocate 同一 job_id
    #[tokio::test]
    async fn test_reallocate_after_deallocate() {
        let broker = LLM_IO_Broker::New();

        // 第一次分配
        let _channels_1 = broker.Allocate(JobId(60)).await.unwrap();
        // 释放
        broker.Deallocate(JobId(60)).await.unwrap();
        // 重新分配（应成功）
        let channels_2 = broker.Allocate(JobId(60)).await.unwrap();
        assert!(broker.Is_Active(JobId(60)).await);

        // 验证新通道可用
        let frontend = channels_2.frontend;
        let mut ml_side = channels_2.ml_side;

        frontend.input_tx.send("re-allocated".to_string()).await.unwrap();
        let prompt = ml_side.input_rx.recv().await.unwrap();
        assert_eq!(prompt, "re-allocated");
    }
}
