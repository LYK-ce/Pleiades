// Presented by KeJi
// Date ： 2026-04-21

use std::collections::HashMap;

use super::job::{JobId, JobKind};
use super::instruction::{TaskInstruction, TaskProgram};

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
    /// 编译 Run 作业的 TaskProgram
    pub fn compile_run(
        &self,
        job_id: JobId,
        model_path: String,
        device_preference: Option<String>,
    ) -> Result<TaskProgram, CompilerError> {
        // 参数校验（占位符）
        if model_path.is_empty() {
            return Err(CompilerError::InvalidParameter("模型路径不能为空".to_string()));
        }
        // 占位符：实际编译逻辑
        todo!("实现 compile_run")
    }

    /// 编译 WorkerRelay 作业的 TaskProgram
    pub fn compile_worker_relay(
        &self,
        job_id: JobId,
        peer_id: String,
        optional_params: Option<String>,
    ) -> Result<TaskProgram, CompilerError> {
        // 参数校验
        if peer_id.is_empty() {
            return Err(CompilerError::InvalidParameter("peer_id 不能为空".to_string()));
        }
        // 占位符：实际编译逻辑
        todo!("实现 compile_worker_relay")
    }

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
            JobKind::WorkerRelay => {
                let peer_id = params.peer_id.ok_or_else(|| {
                    CompilerError::InvalidParameter("WorkerRelay 作业需要 peer_id".to_string())
                })?;
                self.compile_worker_relay(job_id, peer_id, params.optional_params)
            }
        }
    }
}

/// 编译参数集合
pub struct CompileParams {
    pub model_path: Option<String>,
    pub device_preference: Option<String>,
    pub peer_id: Option<String>,
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