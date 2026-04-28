//Presented by KeJi
//Date ： 2026-04-27

use crate::orchestrator::job::JobId;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use super::capability::{IoFrontend, IoHandle, LLM_IO_Capability, LLM_IO_Error};

// ─── 常量 ───────────────────────────────────────────────────

/// 通道缓冲大小
const CHANNEL_BUFFER_SIZE: usize = 64;

// ─── 内部结构 ───────────────────────────────────────────────

/// 内部通道条目。
///
/// 保留 Sender 克隆用于 Deallocate 时加速通道关闭；
/// 托管 `IoHandle` 和 `IoFrontend` 供 `Take_ML_Side` / `Take_Frontend` 取出。
///
/// **生命周期语义**：Deallocate 仅清理内部索引，通道的实际生命周期
/// 由两端句柄（`IoFrontend` / `IoHandle`）的 Drop 决定。
/// Take 后对应 Option 变为 None，Broker 不再持有该端的 Receiver。
struct ChannelEntry {
    /// 保留前端 input 侧的 Sender 引用
    _frontend_input_tx: mpsc::Sender<String>,
    /// 保留 ML output 侧的 Sender 引用
    _ml_output_tx: mpsc::Sender<String>,
    /// ML 侧端点，Take_ML_Side 后变为 None
    ml_side: Option<IoHandle>,
    /// 前端侧端点，Take_Frontend 后变为 None
    frontend: Option<IoFrontend>,
}

// ─── LLM_IO_Broker ─────────────────────────────────────────

/// LLM_IO 代理，负责通道的分配、托管、分发和回收。
///
/// ## 使用流程
/// 1. 编排层调用 `Allocate(job_id)` 创建通道对，两端由 Broker 内部托管
/// 2. 编排层调用 `Take_ML_Side(job_id)` 取出 ML 侧端点，注入 JobExecutor
/// 3. 前端/测试调用 `Take_Frontend(job_id)` 取出前端侧端点，进行文本交互
/// 4. Job 结束后调用 `Deallocate(job_id)` 清理内部索引
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
    async fn Allocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error> {
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

        // 4. 在索引中插入 ChannelEntry（两端托管 + Sender 克隆）
        let entry = ChannelEntry {
            _frontend_input_tx: input_tx,
            _ml_output_tx: output_tx,
            ml_side: Some(ml_side),
            frontend: Some(frontend),
        };
        map.insert(job_id, entry);

        Ok(())
    }

    async fn Take_ML_Side(&self, job_id: JobId) -> Result<IoHandle, LLM_IO_Error> {
        let mut map = self.channels.lock().await;
        let entry = map.get_mut(&job_id).ok_or_else(|| {
            LLM_IO_Error::NotFound(format!("job_id {:?} not found", job_id))
        })?;
        entry.ml_side.take().ok_or_else(|| {
            LLM_IO_Error::NotFound(format!("job_id {:?} ML side already taken", job_id))
        })
    }

    async fn Take_Frontend(&self, job_id: JobId) -> Result<IoFrontend, LLM_IO_Error> {
        let mut map = self.channels.lock().await;
        let entry = map.get_mut(&job_id).ok_or_else(|| {
            LLM_IO_Error::NotFound(format!("job_id {:?} not found", job_id))
        })?;
        entry.frontend.take().ok_or_else(|| {
            LLM_IO_Error::NotFound(format!("job_id {:?} frontend already taken", job_id))
        })
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

    // 验证 Allocate + Take 两端后可正常收发
    #[tokio::test]
    async fn test_allocate_and_take_channels() {
        let broker = LLM_IO_Broker::New();
        broker.Allocate(JobId(1)).await.unwrap();

        let mut frontend = broker.Take_Frontend(JobId(1)).await.unwrap();
        let mut ml_side = broker.Take_ML_Side(JobId(1)).await.unwrap();

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

    // 验证 Allocate 后 Is_Active 返回 true
    #[tokio::test]
    async fn test_allocate_registers_in_index() {
        let broker = LLM_IO_Broker::New();
        broker.Allocate(JobId(10)).await.unwrap();
        assert!(broker.Is_Active(JobId(10)).await);
    }

    // 验证 Deallocate 后 Is_Active 返回 false
    #[tokio::test]
    async fn test_deallocate_removes_from_index() {
        let broker = LLM_IO_Broker::New();
        broker.Allocate(JobId(20)).await.unwrap();
        assert!(broker.Is_Active(JobId(20)).await);

        broker.Deallocate(JobId(20)).await.unwrap();
        assert!(!broker.Is_Active(JobId(20)).await);
    }

    // 验证对不存在的 job_id 调用 Deallocate 不 panic，返回 Ok
    #[tokio::test]
    async fn test_deallocate_idempotent() {
        let broker = LLM_IO_Broker::New();
        let result = broker.Deallocate(JobId(999)).await;
        assert!(result.is_ok());
    }

    // 验证前端 IoFrontend Drop + Deallocate 后，ML 侧的 input_rx.recv() 返回 None
    #[tokio::test]
    async fn test_frontend_drop_closes_ml_input_rx() {
        let broker = LLM_IO_Broker::New();
        broker.Allocate(JobId(30)).await.unwrap();

        let frontend = broker.Take_Frontend(JobId(30)).await.unwrap();
        let mut ml_side = broker.Take_ML_Side(JobId(30)).await.unwrap();

        // 先 Deallocate 移除 ChannelEntry 中的 Sender 克隆
        broker.Deallocate(JobId(30)).await.unwrap();
        // 再 drop 前端（移除最后一个 input_tx）
        drop(frontend);

        // ML 侧接收应返回 None
        let result = ml_side.input_rx.recv().await;
        assert!(result.is_none());
    }

    // 验证 IoHandle Drop + Deallocate 后，前端的 output_rx.recv() 返回 None
    #[tokio::test]
    async fn test_ml_side_drop_closes_frontend_output_rx() {
        let broker = LLM_IO_Broker::New();
        broker.Allocate(JobId(40)).await.unwrap();

        let mut frontend = broker.Take_Frontend(JobId(40)).await.unwrap();
        let ml_side = broker.Take_ML_Side(JobId(40)).await.unwrap();

        // 先 Deallocate 移除 ChannelEntry 中的 Sender 克隆
        broker.Deallocate(JobId(40)).await.unwrap();
        // 再 drop ML 侧（移除最后一个 output_tx）
        drop(ml_side);

        // 前端接收应返回 None
        let result = frontend.output_rx.recv().await;
        assert!(result.is_none());
    }

    // 验证对同一 job_id 重复 Allocate 时返回 AllocationFailed 错误
    #[tokio::test]
    async fn test_duplicate_allocate_returns_error() {
        let broker = LLM_IO_Broker::New();

        broker.Allocate(JobId(50)).await.unwrap();
        assert!(broker.Is_Active(JobId(50)).await);

        let result = broker.Allocate(JobId(50)).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            LLM_IO_Error::AllocationFailed(msg) => {
                assert!(msg.contains("already has an active channel"));
            }
            other => panic!("Expected AllocationFailed, got {:?}", other),
        }
    }

    // 验证 Deallocate 后可重新 Allocate 同一 job_id
    #[tokio::test]
    async fn test_reallocate_after_deallocate() {
        let broker = LLM_IO_Broker::New();

        broker.Allocate(JobId(60)).await.unwrap();
        broker.Deallocate(JobId(60)).await.unwrap();

        // 重新分配（应成功）
        broker.Allocate(JobId(60)).await.unwrap();
        assert!(broker.Is_Active(JobId(60)).await);

        // 验证新通道可用
        let frontend = broker.Take_Frontend(JobId(60)).await.unwrap();
        let mut ml_side = broker.Take_ML_Side(JobId(60)).await.unwrap();

        frontend.input_tx.send("re-allocated".to_string()).await.unwrap();
        let prompt = ml_side.input_rx.recv().await.unwrap();
        assert_eq!(prompt, "re-allocated");
    }

    // 验证 Take_ML_Side 对不存在的 job_id 返回 NotFound
    #[tokio::test]
    async fn test_take_ml_side_not_found() {
        let broker = LLM_IO_Broker::New();
        let result = broker.Take_ML_Side(JobId(999)).await;
        assert!(matches!(result, Err(LLM_IO_Error::NotFound(_))));
    }

    // 验证 Take_Frontend 对不存在的 job_id 返回 NotFound
    #[tokio::test]
    async fn test_take_frontend_not_found() {
        let broker = LLM_IO_Broker::New();
        let result = broker.Take_Frontend(JobId(999)).await;
        assert!(matches!(result, Err(LLM_IO_Error::NotFound(_))));
    }

    // 验证 Take_ML_Side 重复调用返回 NotFound（已被取走）
    #[tokio::test]
    async fn test_take_ml_side_already_taken() {
        let broker = LLM_IO_Broker::New();
        broker.Allocate(JobId(70)).await.unwrap();

        let _ml_side = broker.Take_ML_Side(JobId(70)).await.unwrap();
        let result = broker.Take_ML_Side(JobId(70)).await;
        assert!(matches!(result, Err(LLM_IO_Error::NotFound(ref msg)) if msg.contains("already taken")));
    }

    // 验证 Take_Frontend 重复调用返回 NotFound（已被取走）
    #[tokio::test]
    async fn test_take_frontend_already_taken() {
        let broker = LLM_IO_Broker::New();
        broker.Allocate(JobId(80)).await.unwrap();

        let _frontend = broker.Take_Frontend(JobId(80)).await.unwrap();
        let result = broker.Take_Frontend(JobId(80)).await;
        assert!(matches!(result, Err(LLM_IO_Error::NotFound(ref msg)) if msg.contains("already taken")));
    }
}
