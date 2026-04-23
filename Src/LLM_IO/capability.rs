// Presented by KeJi
// Date ： 2026-04-23

use crate::orchestrator::job::JobId;
use async_trait::async_trait;
use std::fmt;
use tokio::sync::mpsc;

// ─── 错误类型 ───────────────────────────────────────────────

/// LLM_IO 模块错误枚举
#[derive(Debug)]
pub enum LLM_IO_Error {
    /// 通道创建失败（如 job_id 已存在、内存不足等）
    AllocationFailed(String),
}

impl fmt::Display for LLM_IO_Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LLM_IO_Error::AllocationFailed(msg) => write!(f, "AllocationFailed: {}", msg),
        }
    }
}

impl std::error::Error for LLM_IO_Error {}

// ─── 通道结构 ───────────────────────────────────────────────

/// `Allocate` 的返回类型，包含前端侧和 ML 侧两个端点
#[derive(Debug)]
pub struct IoChannels {
    pub frontend: IoFrontend,
    pub ml_side: IoHandle,
}

/// 前端持有的外侧端点
#[derive(Debug)]
pub struct IoFrontend {
    /// 发送 Prompt
    pub input_tx: mpsc::Sender<String>,
    /// 接收 Completion
    pub output_rx: mpsc::Receiver<String>,
}

/// 注入 Job 的 ML 侧端点
#[derive(Debug)]
pub struct IoHandle {
    /// 接收 Prompt
    pub input_rx: mpsc::Receiver<String>,
    /// 发送 Completion
    pub output_tx: mpsc::Sender<String>,
}

// ─── Trait 定义 ─────────────────────────────────────────────

/// LLM_IO 能力 trait
#[async_trait]
pub trait LLM_IO_Capability: Send + Sync {
    /// 为指定 Job 创建一组双向文本通道。
    /// 返回前端侧端点（IoFrontend）和 ML 侧端点（IoHandle）。
    /// 若 job_id 已存在于内部索引中，返回 `AllocationFailed` 错误。
    async fn Allocate(&self, job_id: JobId) -> Result<IoChannels, LLM_IO_Error>;

    /// 从内部索引中移除指定 Job 的通道条目。
    /// **仅清理内部索引，通道的实际生命周期由两端句柄的 Drop 决定。**
    /// 若 job_id 不存在，幂等返回 Ok。
    async fn Deallocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error>;

    /// 查询指定 Job 的通道是否仍存在于内部索引中。
    async fn Is_Active(&self, job_id: JobId) -> bool;
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 验证 IoChannels 能正确构造，且 frontend/ml_side 字段可访问
    #[tokio::test]
    async fn test_io_channels_construct_and_destructure() {
        let (input_tx, input_rx) = mpsc::channel::<String>(64);
        let (output_tx, output_rx) = mpsc::channel::<String>(64);

        let channels = IoChannels {
            frontend: IoFrontend { input_tx, output_rx },
            ml_side: IoHandle { input_rx, output_tx },
        };

        // 验证字段可访问（解构）
        let _frontend = channels.frontend;
        let _ml_side = channels.ml_side;
    }

    // 验证 input_tx 可以 Clone（多前端场景预留）
    #[tokio::test]
    async fn test_io_frontend_clone_sender() {
        let (input_tx, _input_rx) = mpsc::channel::<String>(64);
        let (_output_tx, output_rx) = mpsc::channel::<String>(64);

        let frontend = IoFrontend { input_tx, output_rx };
        // mpsc::Sender 支持 Clone
        let _cloned_tx = frontend.input_tx.clone();
    }

    // 验证 LLM_IO_Error 的 Display 输出格式正确
    #[test]
    fn test_llm_io_error_display() {
        let err = LLM_IO_Error::AllocationFailed("out of memory".to_string());
        assert_eq!(format!("{}", err), "AllocationFailed: out of memory");
    }
}
