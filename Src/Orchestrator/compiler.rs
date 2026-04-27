//Presented by KeJi
//Date ： 2026-04-27

use std::collections::HashMap;

use super::job::{JobId, JobKind};
use super::instruction::{TaskInstruction, TaskProgram};
use super::slot::SlotId;
use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Pipeline_Params};

// ─── 约定槽位常量 ─────────────────────────────────────────
// JobExecutor 初始化时写入的固定槽位，Compiler 编译指令时引用相同 ID。

/// IoHandle 约定槽位（由 JobExecutor::new 写入）
pub const SLOT_IO: SlotId = SlotId(0);

/// 编译器错误类型
#[derive(Debug, thiserror::Error)]
pub enum CompilerError {
    #[error("参数非法: {0}")]
    InvalidParameter(String),
    #[error("内部编译错误: {0}")]
    Internal(String),
}

/// 编译器结构体（无状态）
pub struct Compiler;

impl Compiler {
    /// 编译 Run 作业的 TaskProgram（单机推理）
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(model) → Const(device) → CreateSession → RunProgram
    /// 补偿：ShutdownSession
    /// ```
    pub fn compile_run(
        &self,
        _job_id: JobId,
        model_path: String,
        _device_preference: Option<String>,
    ) -> Result<TaskProgram, CompilerError> {
        if model_path.is_empty() {
            return Err(CompilerError::InvalidParameter("模型路径不能为空".to_string()));
        }
        // 占位符：实际编译逻辑
        todo!("实现 compile_run")
    }

    /// 编译 Coordinator 作业的 TaskProgram（分布式协调者）
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(model) → AnalyzeModel → SplitModel → SendFile(to workers)
    ///       → OpenTensorStream → Const(device) → CreateSession(with tensor_io)
    ///       → RunProgram
    /// 补偿：ShutdownSession
    /// ```
    pub fn compile_coordinator(
        &self,
        _job_id: JobId,
        model_path: String,
        _peers: Vec<String>,
        _device_preference: Option<String>,
    ) -> Result<TaskProgram, CompilerError> {
        if model_path.is_empty() {
            return Err(CompilerError::InvalidParameter("模型路径不能为空".to_string()));
        }
        if _peers.is_empty() {
            return Err(CompilerError::InvalidParameter("Coordinator 至少需要一个 Peer".to_string()));
        }
        // 占位符：实际编译逻辑
        todo!("实现 compile_coordinator")
    }

    /// 编译 Relay 作业的 TaskProgram（分布式中继 Worker）
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：ReceiveFile → OpenTensorStream(downstream) → Const(device)
    ///       → CreateSession(with tensor_io) → RunProgram
    /// 补偿：ShutdownSession
    /// ```
    pub fn compile_relay(
        &self,
        _job_id: JobId,
        peer_id: String,
        _optional_params: Option<String>,
    ) -> Result<TaskProgram, CompilerError> {
        if peer_id.is_empty() {
            return Err(CompilerError::InvalidParameter("peer_id 不能为空".to_string()));
        }
        // 占位符：实际编译逻辑
        todo!("实现 compile_relay")
    }

    // ─── ML Thread 程序构建 ────────────────────────────────

    /// 构建单机推理的 ML 指令序列
    ///
    /// 生成的 ML 指令序列：
    /// ```text
    /// Input → Encode → Set(max_tokens) → Prefill(TOKENID3) → CopyMeta
    /// → Sample → Decode → Output
    /// → Loop [ BreakIf, Inference(TOKENID2), Sample, Decode, Output ]
    /// → EndOutput
    /// ```
    pub fn build_run_ml_program(
        &self,
        _params: &Pipeline_Params,
    ) -> Vec<Instruction> {
        // 占位符：实际构建单机推理指令序列
        todo!("实现 build_run_ml_program")
    }

    /// 构建分布式协调者的 ML 指令序列
    ///
    /// 生成的 ML 指令序列：
    /// ```text
    /// Input → Encode → Set(max_tokens) → Inference(TOKENID3)
    /// → Send → Receive → Sample(TENSOR1) → Decode → Output
    /// → Loop [ BreakIf, Inference(TOKENID2), Send, Receive, Sample(TENSOR1), Decode, Output ]
    /// → SendEOF → EndOutput
    /// ```
    pub fn build_coordinator_ml_program(
        &self,
        _params: &Pipeline_Params,
    ) -> Vec<Instruction> {
        // 占位符：实际构建协调者指令序列
        todo!("实现 build_coordinator_ml_program")
    }

    /// 构建分布式中继 Worker 的 ML 指令序列
    ///
    /// 生成的 ML 指令序列：
    /// ```text
    /// Loop [ Receive, BreakIf, Inference(TENSOR1), Send ]
    /// ```
    pub fn build_relay_ml_program(&self) -> Vec<Instruction> {
        // 占位符：实际构建中继 Worker 指令序列
        todo!("实现 build_relay_ml_program")
    }

    // ─── 通用接口 ──────────────────────────────────────────

    /// 通用编译接口，根据 JobKind 分发
    pub fn compile(
        &self,
        kind: JobKind,
        job_id: JobId,
        params: CompileParams,
    ) -> Result<TaskProgram, CompilerError> {
        match kind {
            JobKind::Run => {
                let model_path = params.model_path.ok_or_else(|| {
                    CompilerError::InvalidParameter("Run 作业需要 model_path".to_string())
                })?;
                self.compile_run(job_id, model_path, params.device_preference)
            }
            JobKind::Coordinator => {
                let model_path = params.model_path.ok_or_else(|| {
                    CompilerError::InvalidParameter("Coordinator 作业需要 model_path".to_string())
                })?;
                let peers = params.peers.ok_or_else(|| {
                    CompilerError::InvalidParameter("Coordinator 作业需要 peers 列表".to_string())
                })?;
                self.compile_coordinator(job_id, model_path, peers, params.device_preference)
            }
            JobKind::Relay => {
                let peer_id = params.peer_id.ok_or_else(|| {
                    CompilerError::InvalidParameter("Relay 作业需要 peer_id".to_string())
                })?;
                self.compile_relay(job_id, peer_id, params.optional_params)
            }
        }
    }
}

/// 编译参数集合
pub struct CompileParams {
    pub model_path: Option<String>,
    pub device_preference: Option<String>,
    pub peer_id: Option<String>,
    pub peers: Option<Vec<String>>,
    pub optional_params: Option<String>,
}

/// 内部 TaskProgramBuilder（不暴露）
struct TaskProgramBuilder {
    instructions: Vec<TaskInstruction>,
    compensation: Vec<TaskInstruction>,
    labels: HashMap<String, usize>,
}

impl TaskProgramBuilder {
    fn new() -> Self {
        Self {
            instructions: Vec::new(),
            compensation: Vec::new(),
            labels: HashMap::new(),
        }
    }

    /// 添加一条正向指令，返回其索引。
    fn push_instruction(&mut self, instr: TaskInstruction) -> usize {
        let idx = self.instructions.len();
        self.instructions.push(instr);
        idx
    }

    /// 添加一条补偿指令。
    fn push_compensation(&mut self, instr: TaskInstruction) {
        self.compensation.push(instr);
    }

    /// 添加一个标签，指向当前正向指令序列的末尾（即下一条指令的索引）。
    fn label_here(&mut self, label: String) {
        self.labels.insert(label, self.instructions.len());
    }

    /// 根据标签名获取指令索引，若标签不存在则返回 `None`。
    fn get_label(&self, label: &str) -> Option<usize> {
        self.labels.get(label).copied()
    }

    /// 构建最终的 TaskProgram。
    fn build(self) -> TaskProgram {
        TaskProgram {
            instructions: self.instructions,
            compensation: self.compensation,
            labels: self.labels,
        }
    }
}
