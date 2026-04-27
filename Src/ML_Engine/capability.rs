//Presented by KeJi
//Date ： 2026-04-23

//! ML Engine Capability 层
//!
//! 定义 ML Engine 对外暴露的唯一 trait `ML_Engine_Capability`，
//! 以及相关的错误类型 `ML_Engine_Error` 和配置类型 `ML_Session_Config`。
//!
//! 所有内部类型（Session_Handle、Session_Command 等）对外不可见，
//! 上层（Orchestrator）仅通过此 trait 与 ML Engine 交互。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use async_trait::async_trait;

use crate::llm_io::IoHandle;
use crate::network::tensor_stream_protocol::Tensor_IO_Handle;
use super::ml_thread_engine_instruction::{
    Instruction, Pipeline_Params, Pipeline_Result, Model_Info,
};

// ─── 错误类型 ───────────────────────────────────────────────

/// ML Engine 能力层错误枚举
#[derive(Debug)]
pub enum ML_Engine_Error {
    /// Session 创建失败（模型加载错误、设备不支持等）
    SessionCreationFailed(String),
    /// 指定 session_id 不存在
    SessionNotFound(String),
    /// 指令序列执行失败
    ProgramFailed(String),
    /// 模型分析失败
    ModelAnalysisFailed(String),
    /// 模型切分失败
    ModelSplitFailed(String),
}

impl fmt::Display for ML_Engine_Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ML_Engine_Error::SessionCreationFailed(msg) => {
                write!(f, "SessionCreationFailed: {}", msg)
            }
            ML_Engine_Error::SessionNotFound(msg) => {
                write!(f, "SessionNotFound: {}", msg)
            }
            ML_Engine_Error::ProgramFailed(msg) => {
                write!(f, "ProgramFailed: {}", msg)
            }
            ML_Engine_Error::ModelAnalysisFailed(msg) => {
                write!(f, "ModelAnalysisFailed: {}", msg)
            }
            ML_Engine_Error::ModelSplitFailed(msg) => {
                write!(f, "ModelSplitFailed: {}", msg)
            }
        }
    }
}

impl std::error::Error for ML_Engine_Error {}

// ─── 配置类型 ───────────────────────────────────────────────

/// Session 创建配置
///
/// 由上层（Orchestrator handler_inference）构造，传入 `Create_Session`。
/// `model_file_id` 通过 Storage 解析为物理路径，替代旧设计中的 `model_path`。
///
/// ## 字段说明
/// - `session_id`: 全局唯一标识，由 Orchestrator 分配
/// - `model_file_id`: Storage 的 file_id，由 Storage 负责路径解析和锁管理
/// - `layer_start` / `layer_end`: 加载的层范围，用于模型切片（分布式推理）
/// - `device`: 推理设备选择
/// - `tensor_io`: 网络张量 IO 句柄，单机推理时为 None
pub struct ML_Session_Config {
    /// Session 唯一标识
    pub session_id: String,
    /// Storage 的 file_id（替代 model_path）
    pub model_file_id: String,
    /// 起始加载层编号（0 = embedding 输入层）
    pub layer_start: usize,
    /// 结束加载层编号（N+1 = 输出层）
    pub layer_end: usize,
    /// 使用的设备（"cpu" 或 "cuda"）
    pub device: String,
    /// 网络张量 IO 句柄（分布式推理用，单机为 None）
    pub tensor_io: Option<Tensor_IO_Handle>,
}

// ─── Trait 定义 ─────────────────────────────────────────────

/// ML Engine 能力 trait
///
/// ML Engine 对外仅暴露此 trait，所有内部类型（Session_Handle、Session_Command 等）对外不可见。
/// Orchestrator 通过 `Arc<Capabilities>` 中的 `ml_engine: Box<dyn ML_Engine_Capability>` 调用。
///
/// ## 方法分类
/// - **Session 生命周期**: `Create_Session`, `Shutdown_Session`
/// - **指令执行**: `Run_Program`
/// - **独立操作**: `Analyze_Model`, `Split_Model`（无需 Session）
#[async_trait]
pub trait ML_Engine_Capability: Send + Sync {
    /// 创建推理 Session
    ///
    /// 内部流程：
    /// 1. 通过 Storage 获取模型文件路径 + 读锁
    /// 2. 创建 cmd 通道
    /// 3. 启动 OS 线程，将物理路径和 io_handle 注入 Session_Thread
    /// 4. 等待 ready 信号，获取 Model_Info
    /// 5. 将 SessionEntry(Handle + ReadGuard) 注册到内部 sessions 表
    ///
    /// # 参数
    /// - `config`: Session 配置（model_file_id, layer_start, layer_end, device, tensor_io）
    /// - `io_handle`: LLM_IO 提供的 ML 侧文本通道端点
    ///
    /// # 返回
    /// - `Model_Info`: 模型信息（架构名称、层数、是否有 tokenizer 等）
    async fn Create_Session(
        &self,
        config: ML_Session_Config,
        io_handle: IoHandle,
    ) -> Result<Model_Info, ML_Engine_Error>;

    /// 关闭并移除指定 Session
    ///
    /// 内部流程：
    /// 1. 从 sessions 表中移除 SessionEntry
    /// 2. 通过 Handle 发送 Shutdown 命令
    /// 3. Session 线程退出，卸载模型
    /// 4. SessionEntry drop → ReadGuard drop → Storage 读锁释放
    ///
    /// 若 session_id 不存在，返回 `SessionNotFound` 错误。
    async fn Shutdown_Session(
        &self,
        session_id: &str,
    ) -> Result<(), ML_Engine_Error>;

    /// 提交指令序列到指定 Session 执行
    ///
    /// 内部流程：
    /// 1. 按 session_id 查 sessions 表，clone Handle（释放锁）
    /// 2. 通过 Handle 发送 Run_Program 命令
    /// 3. 等待 oneshot reply 获取 Pipeline_Result
    ///
    /// 执行期间，Session 线程会通过 io_handle 与前端交换文本。
    async fn Run_Program(
        &self,
        session_id: &str,
        program: Vec<Instruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<Pipeline_Result, ML_Engine_Error>;

    /// 分析模型文件结构（无需 Session，独立操作）
    ///
    /// 内部通过 Storage 获取临时读锁 + 路径，分析完自动释放。
    async fn Analyze_Model(
        &self,
        model_file_id: &str,
    ) -> Result<Model_Info, ML_Engine_Error>;

    /// 切分模型文件（无需 Session，独立操作）
    ///
    /// 内部通过 Storage 获取源文件读锁 + 输出文件写锁。
    async fn Split_Model(
        &self,
        source_file_id: &str,
        start: usize,
        end: usize,
        output_file_id: &str,
    ) -> Result<(), ML_Engine_Error>;
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 ML_Engine_Error 的 Display 实现
    #[test]
    fn test_error_display() {
        let error = ML_Engine_Error::SessionCreationFailed("test error".to_string());
        assert_eq!(format!("{}", error), "SessionCreationFailed: test error");

        let error = ML_Engine_Error::SessionNotFound("sess-001".to_string());
        assert_eq!(format!("{}", error), "SessionNotFound: sess-001");

        let error = ML_Engine_Error::ProgramFailed("timeout".to_string());
        assert_eq!(format!("{}", error), "ProgramFailed: timeout");

        let error = ML_Engine_Error::ModelAnalysisFailed("bad format".to_string());
        assert_eq!(format!("{}", error), "ModelAnalysisFailed: bad format");

        let error = ML_Engine_Error::ModelSplitFailed("io error".to_string());
        assert_eq!(format!("{}", error), "ModelSplitFailed: io error");
    }

    /// 验证 ML_Engine_Error 实现了 std::error::Error
    #[test]
    fn test_error_is_std_error() {
        let error: Box<dyn std::error::Error> = Box::new(
            ML_Engine_Error::SessionNotFound("test".to_string()),
        );
        assert!(error.source().is_none());
    }

    /// 验证 ML_Engine_Error 的 Debug 实现
    #[test]
    fn test_error_debug() {
        let error = ML_Engine_Error::SessionCreationFailed("debug test".to_string());
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("SessionCreationFailed"));
        assert!(debug_str.contains("debug test"));
    }

    /// 验证 ML_Session_Config 可正确构造
    #[test]
    fn test_session_config_construction() {
        let config = ML_Session_Config {
            session_id: "sess-001".to_string(),
            model_file_id: "qwen3-0.6b.gguf".to_string(),
            layer_start: 0,
            layer_end: 29,
            device: "cpu".to_string(),
            tensor_io: None,
        };
        assert_eq!(config.session_id, "sess-001");
        assert_eq!(config.model_file_id, "qwen3-0.6b.gguf");
        assert_eq!(config.layer_start, 0);
        assert_eq!(config.layer_end, 29);
        assert_eq!(config.device, "cpu");
        assert!(config.tensor_io.is_none());
    }
}
