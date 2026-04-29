//Presented by KeJi
//Date ： 2026-04-28

use std::collections::HashMap;

use super::job::{JobId, JobKind};
use super::instruction::{TaskInstruction, TaskProgram};
use super::slot::{SlotId, ConstValue};
use crate::ml_engine::ml_thread_engine_instruction::{
    Instruction, Inference_Input, Set_Target, Pipeline_Params,
};
use crate::ml_engine::ml_thread_register::{
    TOKENID2, TOKENID3, TENSOR1, TENSOR2, FLAG1, META1, META2, META5,
};

// ─── 约定槽位常量 ─────────────────────────────────────────
// JobExecutor 初始化时写入的固定槽位，Compiler 编译指令时引用相同 ID。

/// IoHandle 约定槽位（由 JobExecutor::new 写入）
pub const SLOT_IO: SlotId = SlotId(0);
/// 模型路径槽位
pub const SLOT_MODEL: SlotId = SlotId(1);
/// 设备字符串槽位
pub const SLOT_DEVICE: SlotId = SlotId(2);
/// Session ID 槽位
pub const SLOT_SESSION: SlotId = SlotId(3);
/// 推理结果槽位
pub const SLOT_RESULT: SlotId = SlotId(4);
/// 层起始编号槽位
pub const SLOT_LAYER_START: SlotId = SlotId(5);
/// 层结束编号槽位
pub const SLOT_LAYER_END: SlotId = SlotId(6);
/// 下游 Peer ID 槽位
pub const SLOT_PEER: SlotId = SlotId(7);
/// 远端 Relay Job ID 槽位（由 RequestPipeline 写入）
pub const SLOT_TARGET_JOB: SlotId = SlotId(8);
/// 入站 raw stream 槽位（由 TakeInboundStream 写入）
pub const SLOT_INBOUND: SlotId = SlotId(9);
/// 出站 raw stream 槽位（由 TakeOutboundStream 写入）
pub const SLOT_OUTBOUND: SlotId = SlotId(10);
/// 组装后的 Tensor IO Handle 槽位（由 BuildTensorIo 写入）
pub const SLOT_TENSOR_IO: SlotId = SlotId(11);
/// Coordinator Job ID 槽位（Relay 用，由 compile_relay 写入）
pub const SLOT_COORDINATOR_JOB: SlotId = SlotId(12);
/// 模型分析结果槽位（AnalyzeModel 写入）
pub const SLOT_MODEL_INFO: SlotId = SlotId(13);
/// 文件分发动态槽位起始偏移（每个 peer 占 4 个槽位）
pub const DISTRIBUTE_DYNAMIC_SLOT_BASE: u32 = 14;

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
    /// 正向：Const(model) → Const(device) → Const(start) → Const(end) → CreateSession → RunProgram
    /// 补偿：ShutdownSession
    /// ```
    pub fn compile_run(
        &self,
        _job_id: JobId,
        model_path: String,
        device_preference: Option<String>,
    ) -> Result<TaskProgram, CompilerError> {
        if model_path.is_empty() {
            return Err(CompilerError::InvalidParameter("模型路径不能为空".to_string()));
        }

        let device = device_preference.unwrap_or_else(|| "cpu".to_string());
        let mut builder = TaskProgramBuilder::new();

        // 正向序列：Const(model) → Const(device) → Const(start) → Const(end) → CreateSession → RunProgram
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(model_path),
            dst: SLOT_MODEL,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(device),
            dst: SLOT_DEVICE,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(0),
            dst: SLOT_LAYER_START,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(u64::MAX),
            dst: SLOT_LAYER_END,
        });
        builder.push_instruction(TaskInstruction::CreateSession {
            model: SLOT_MODEL,
            device: SLOT_DEVICE,
            start: SLOT_LAYER_START,
            end: SLOT_LAYER_END,
            io: SLOT_IO,
            tensor_io: None,
            result: SLOT_SESSION,
        });
        builder.push_instruction(TaskInstruction::RunProgram {
            session: SLOT_SESSION,
            result: SLOT_RESULT,
        });

        // 补偿序列：ShutdownSession
        builder.push_compensation(TaskInstruction::ShutdownSession {
            session: SLOT_SESSION,
        });

        Ok(builder.build())
    }

    /// 编译 Coordinator 作业的 TaskProgram（分布式协调者 — 纯流水线推理）
    ///
    /// 前提：模型文件已在各节点本地（文件分发是独立命令）。
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(model/device/layers) → Const(peer) → RequestPipeline
    ///       → OpenTensorStream → TakeInbound → TakeOutbound → BuildTensorIo
    ///       → CreateSession(with tensor_io) → RunProgram
    /// 补偿：ShutdownSession
    /// ```
    pub fn compile_coordinator(
        &self,
        _job_id: JobId,
        model_path: String,
        peers: Vec<String>,
        device_preference: Option<String>,
        layer_start: usize,
        layer_end: usize,
    ) -> Result<TaskProgram, CompilerError> {
        if model_path.is_empty() {
            return Err(CompilerError::InvalidParameter("模型路径不能为空".to_string()));
        }
        if peers.is_empty() {
            return Err(CompilerError::InvalidParameter("Coordinator 至少需要一个 Peer".to_string()));
        }

        // 当前仅支持单 Worker（单 Peer）
        let peer_id = peers.into_iter().next().unwrap();
        if peer_id.is_empty() {
            return Err(CompilerError::InvalidParameter("peer_id 不能为空".to_string()));
        }

        let device = device_preference.unwrap_or_else(|| "cpu".to_string());
        let mut builder = TaskProgramBuilder::new();

        // ── 正向序列 ──────────────────────────────────────────
        // 1. 常量注入（必须在 RequestPipeline 之前，handler 需要从槽位读取参数）
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(model_path),
            dst: SLOT_MODEL,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(device),
            dst: SLOT_DEVICE,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(layer_start as u64),
            dst: SLOT_LAYER_START,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(layer_end as u64),
            dst: SLOT_LAYER_END,
        });

        // 2. 请求远端 Peer 加入 Pipeline，获取 relay_job_id
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(peer_id),
            dst: SLOT_PEER,
        });
        builder.push_instruction(TaskInstruction::RequestPipeline {
            peer: SLOT_PEER,
            model: SLOT_MODEL,
            device: SLOT_DEVICE,
            start: SLOT_LAYER_START,
            end: SLOT_LAYER_END,
            result: SLOT_TARGET_JOB,
        });

        // 3. 建立双向 Tensor Stream
        builder.push_instruction(TaskInstruction::OpenTensorStream {
            peer: SLOT_PEER,
            target_job: SLOT_TARGET_JOB,
        });
        builder.push_instruction(TaskInstruction::TakeInboundStream {
            result: SLOT_INBOUND,
        });
        builder.push_instruction(TaskInstruction::TakeOutboundStream {
            result: SLOT_OUTBOUND,
        });
        builder.push_instruction(TaskInstruction::BuildTensorIo {
            inbound: SLOT_INBOUND,
            outbound: SLOT_OUTBOUND,
            result: SLOT_TENSOR_IO,
        });

        // 4. 创建 ML Session（带 tensor_io）+ 运行推理
        builder.push_instruction(TaskInstruction::CreateSession {
            model: SLOT_MODEL,
            device: SLOT_DEVICE,
            start: SLOT_LAYER_START,
            end: SLOT_LAYER_END,
            io: SLOT_IO,
            tensor_io: Some(SLOT_TENSOR_IO),
            result: SLOT_SESSION,
        });
        builder.push_instruction(TaskInstruction::RunProgram {
            session: SLOT_SESSION,
            result: SLOT_RESULT,
        });

        // ── 补偿序列 ──────────────────────────────────────────
        builder.push_compensation(TaskInstruction::ShutdownSession {
            session: SLOT_SESSION,
        });

        Ok(builder.build())
    }

    /// 编译 Relay 作业的 TaskProgram（分布式 Worker — 纯流水线推理）
    ///
    /// 前提：模型分片已在本地（文件分发是独立命令）。
    /// 由远端 Core 的 route_network(PipelineFlow) 调用。
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(coordinator_peer/job) → OpenTensorStream
    ///       → TakeInbound → TakeOutbound → BuildTensorIo
    ///       → Const(model/device/layers) → CreateSession(with tensor_io) → RunProgram
    /// 补偿：ShutdownSession
    /// ```
    pub fn compile_relay(
        &self,
        _job_id: JobId,
        coordinator_peer_id: String,
        coordinator_job_id: u64,
        model_file_id: String,
        device_preference: Option<String>,
        layer_start: usize,
        layer_end: usize,
    ) -> Result<TaskProgram, CompilerError> {
        if coordinator_peer_id.is_empty() {
            return Err(CompilerError::InvalidParameter("coordinator_peer_id 不能为空".to_string()));
        }
        if model_file_id.is_empty() {
            return Err(CompilerError::InvalidParameter("model_file_id 不能为空".to_string()));
        }

        let device = device_preference.unwrap_or_else(|| "cpu".to_string());
        let mut builder = TaskProgramBuilder::new();

        // ── 正向序列 ──────────────────────────────────────────
        // 1. 注入 coordinator 信息，建立反向 Tensor Stream
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(coordinator_peer_id),
            dst: SLOT_PEER,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(coordinator_job_id),
            dst: SLOT_COORDINATOR_JOB,
        });
        builder.push_instruction(TaskInstruction::OpenTensorStream {
            peer: SLOT_PEER,
            target_job: SLOT_COORDINATOR_JOB,
        });
        builder.push_instruction(TaskInstruction::TakeInboundStream {
            result: SLOT_INBOUND,
        });
        builder.push_instruction(TaskInstruction::TakeOutboundStream {
            result: SLOT_OUTBOUND,
        });
        builder.push_instruction(TaskInstruction::BuildTensorIo {
            inbound: SLOT_INBOUND,
            outbound: SLOT_OUTBOUND,
            result: SLOT_TENSOR_IO,
        });

        // 2. 注入模型/设备/层范围参数
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(model_file_id),
            dst: SLOT_MODEL,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(device),
            dst: SLOT_DEVICE,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(layer_start as u64),
            dst: SLOT_LAYER_START,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(layer_end as u64),
            dst: SLOT_LAYER_END,
        });

        // 3. 创建 ML Session（带 tensor_io）+ 运行推理
        builder.push_instruction(TaskInstruction::CreateSession {
            model: SLOT_MODEL,
            device: SLOT_DEVICE,
            start: SLOT_LAYER_START,
            end: SLOT_LAYER_END,
            io: SLOT_IO,
            tensor_io: Some(SLOT_TENSOR_IO),
            result: SLOT_SESSION,
        });
        builder.push_instruction(TaskInstruction::RunProgram {
            session: SLOT_SESSION,
            result: SLOT_RESULT,
        });

        // ── 补偿序列 ──────────────────────────────────────────
        builder.push_compensation(TaskInstruction::ShutdownSession {
            session: SLOT_SESSION,
        });

        Ok(builder.build())
    }

    /// 编译 Distribute 作业的 TaskProgram（文件分发 — 模型分片 + 发送）
    ///
    /// 每个 peer 由用户指定分片范围 `(peer_id, layer_start, layer_end)`。
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(model) → AnalyzeModel
    ///       → 对每个 peer: Const(start/end/shard_name) → SplitModel → Const(peer) → SendFile
    /// 补偿：（空 — 临时分片由 Storage 自动管理）
    /// ```
    pub fn compile_distribute(
        &self,
        _job_id: JobId,
        model_path: String,
        peers: Vec<(String, usize, usize)>,
    ) -> Result<TaskProgram, CompilerError> {
        if model_path.is_empty() {
            return Err(CompilerError::InvalidParameter("模型路径不能为空".to_string()));
        }
        if peers.is_empty() {
            return Err(CompilerError::InvalidParameter("Distribute 至少需要一个 Peer".to_string()));
        }
        for (i, (peer_id, _, _)) in peers.iter().enumerate() {
            if peer_id.is_empty() {
                return Err(CompilerError::InvalidParameter(
                    format!("peers[{}] 的 peer_id 不能为空", i),
                ));
            }
        }

        let mut builder = TaskProgramBuilder::new();

        // 1. 注入模型路径 + 分析模型
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(model_path.clone()),
            dst: SLOT_MODEL,
        });
        builder.push_instruction(TaskInstruction::AnalyzeModel {
            model: SLOT_MODEL,
            result: SLOT_MODEL_INFO,
        });

        // 2. 对每个 peer 生成 SplitModel + SendFile
        //    动态槽位分配：每个 peer 占 4 个槽：layer_start, layer_end, shard_output, peer_id
        for (i, (peer_id, layer_start, layer_end)) in peers.iter().enumerate() {
            let base = DISTRIBUTE_DYNAMIC_SLOT_BASE + (i as u32) * 4;
            let slot_layer_start = SlotId(base);
            let slot_layer_end = SlotId(base + 1);
            let slot_shard = SlotId(base + 2);
            let slot_peer = SlotId(base + 3);

            // 生成分片文件名：{model_base}_shard_{start}_{end}
            let shard_name = format!("{}_shard_{}_{}", model_path, layer_start, layer_end);

            builder.push_instruction(TaskInstruction::Const {
                value: ConstValue::U64(*layer_start as u64),
                dst: slot_layer_start,
            });
            builder.push_instruction(TaskInstruction::Const {
                value: ConstValue::U64(*layer_end as u64),
                dst: slot_layer_end,
            });
            builder.push_instruction(TaskInstruction::Const {
                value: ConstValue::String(shard_name),
                dst: slot_shard,
            });
            builder.push_instruction(TaskInstruction::SplitModel {
                source: SLOT_MODEL,
                start: slot_layer_start,
                end: slot_layer_end,
                output: slot_shard,
            });
            builder.push_instruction(TaskInstruction::Const {
                value: ConstValue::String(peer_id.clone()),
                dst: slot_peer,
            });
            builder.push_instruction(TaskInstruction::SendFile {
                peer: slot_peer,
                file: slot_shard,
            });
        }

        // 补偿序列为空 — 临时分片由 Storage 自动管理
        Ok(builder.build())
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
    /// 注意：静态方法（无 &self），handler 可直接调用 `Compiler::build_run_ml_program(&params)`
    pub fn build_run_ml_program(params: &Pipeline_Params) -> Vec<Instruction> {
        vec![
            Instruction::Input,
            Instruction::Encode,
            Instruction::Set {
                target: Set_Target::Meta(META2, params.max_tokens as f64),
            },
            Instruction::Set {
                target: Set_Target::Flag(FLAG1, false),
            },
            Instruction::Prefill { input: TOKENID3 },
            Instruction::CopyMeta {
                src: META5,
                dst: META1,
            },
            Instruction::Sample {
                tensor_reg: TENSOR2,
            },
            Instruction::Decode,
            Instruction::Output,
            Instruction::Loop {
                body: vec![
                    Instruction::BreakIf,
                    Instruction::Inference {
                        input: Inference_Input::Tokens(TOKENID2),
                    },
                    Instruction::Sample {
                        tensor_reg: TENSOR2,
                    },
                    Instruction::Decode,
                    Instruction::Output,
                ],
            },
            Instruction::EndOutput,
        ]
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
        params: &Pipeline_Params,
    ) -> Vec<Instruction> {
        vec![
            Instruction::Input,
            Instruction::Encode,
            Instruction::Set {
                target: Set_Target::Meta(META2, params.max_tokens as f64),
            },
            Instruction::Set {
                target: Set_Target::Flag(FLAG1, false),
            },
            Instruction::Prefill { input: TOKENID3 },
            Instruction::CopyMeta {
                src: META5,
                dst: META1,
            },
            // Coordinator → 发送本地前向输出到下游 Worker
            Instruction::Send,
            // Coordinator ← 接收下游 Worker 回传的计算结果
            Instruction::Receive,
            // 从 TENSOR1（Receive 写入）采样，而非 TENSOR2（本地前向输出）
            Instruction::Sample {
                tensor_reg: TENSOR1,
            },
            Instruction::Decode,
            Instruction::Output,
            Instruction::Loop {
                body: vec![
                    Instruction::BreakIf,
                    Instruction::Inference {
                        input: Inference_Input::Tokens(TOKENID2),
                    },
                    Instruction::Send,
                    Instruction::Receive,
                    Instruction::Sample {
                        tensor_reg: TENSOR1,
                    },
                    Instruction::Decode,
                    Instruction::Output,
                ],
            },
            // 通知下游 Worker 推理结束
            Instruction::SendEOF,
            Instruction::EndOutput,
        ]
    }

    /// 构建分布式中继 Worker 的 ML 指令序列
    ///
    /// 生成的 ML 指令序列：
    /// ```text
    /// Loop [ Receive, BreakIf, Inference(TENSOR1), Send ]
    /// ```
    pub fn build_relay_ml_program() -> Vec<Instruction> {
        vec![
            Instruction::Loop {
                body: vec![
                    // Worker ← 接收上游 Coordinator 发来的张量
                    Instruction::Receive,
                    // EOF 帧时 FLAG1=true → BreakIf 退出循环
                    Instruction::BreakIf,
                    // 从 TENSOR1（Receive 写入）做前向推理
                    Instruction::Inference {
                        input: Inference_Input::Tensor(TENSOR1),
                    },
                    // Worker → 发送本地前向输出到上游 Coordinator
                    Instruction::Send,
                ],
            },
        ]
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
                self.compile_coordinator(
                    job_id, model_path, peers, params.device_preference,
                    params.layer_start.unwrap_or(0),
                    params.layer_end.unwrap_or(usize::MAX),
                )
            }
            JobKind::Relay => {
                let coordinator_peer_id = params.coordinator_peer_id.ok_or_else(|| {
                    CompilerError::InvalidParameter("Relay 作业需要 coordinator_peer_id".to_string())
                })?;
                let coordinator_job_id = params.coordinator_job_id.ok_or_else(|| {
                    CompilerError::InvalidParameter("Relay 作业需要 coordinator_job_id".to_string())
                })?;
                let model_file_id = params.model_path.ok_or_else(|| {
                    CompilerError::InvalidParameter("Relay 作业需要 model_file_id".to_string())
                })?;
                self.compile_relay(
                    job_id, coordinator_peer_id, coordinator_job_id, model_file_id,
                    params.device_preference,
                    params.layer_start.unwrap_or(0),
                    params.layer_end.unwrap_or(usize::MAX),
                )
            }
            JobKind::Distribute => {
                let model_path = params.model_path.ok_or_else(|| {
                    CompilerError::InvalidParameter("Distribute 作业需要 model_path".to_string())
                })?;
                let distribute_peers = params.distribute_peers.ok_or_else(|| {
                    CompilerError::InvalidParameter("Distribute 作业需要 distribute_peers".to_string())
                })?;
                self.compile_distribute(job_id, model_path, distribute_peers)
            }
        }
    }
}

/// 编译参数集合
pub struct CompileParams {
    pub model_path: Option<String>,
    pub device_preference: Option<String>,
    pub coordinator_peer_id: Option<String>,
    pub coordinator_job_id: Option<u64>,
    pub peers: Option<Vec<String>>,
    pub layer_start: Option<usize>,
    pub layer_end: Option<usize>,
    /// 文件分发目标节点列表 (peer_id, layer_start, layer_end)
    pub distribute_peers: Option<Vec<(String, usize, usize)>>,
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

// ═══════════════════════════════════════════════════════════
// 单元测试
// ═══════════════════════════════════════════════════════════
#[cfg(test)]
mod compiler_tests {
        use super::*;
        use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Inference_Input};
    
        /// 辅助：在指令序列中查找第一条满足谓词的指令索引
        fn find_instr<F>(program: &TaskProgram, predicate: F) -> Option<usize>
        where
            F: Fn(&TaskInstruction) -> bool,
        {
            program.instructions.iter().position(predicate)
        }
    
        fn test_job_id() -> JobId {
            JobId(42)
        }
    
        // ─── A 组：compile_run ────────────────────────────────
    
        /// TC-01: compile_run 正常编译 — 指令数量 6 + 补偿 1
        #[test]
        fn tc01_compile_run_structure() {
            let compiler = Compiler;
            let program = compiler
                .compile_run(test_job_id(), "model.gguf".into(), Some("cuda".into()))
                .unwrap();
    
            // 正向 6 条指令
            assert_eq!(program.instructions.len(), 6, "正向指令数量");
            // 补偿 1 条指令
            assert_eq!(program.compensation.len(), 1, "补偿指令数量");
    
            // 第 5 条（索引 4）是 CreateSession，tensor_io = None
            match &program.instructions[4] {
                TaskInstruction::CreateSession { tensor_io, io, .. } => {
                    assert!(tensor_io.is_none(), "Run 的 CreateSession 不应有 tensor_io");
                    assert_eq!(*io, SLOT_IO);
                }
                other => panic!("第 5 条指令应为 CreateSession，实际为 {:?}", other),
            }
    
            // 最后一条是 RunProgram
            match &program.instructions[5] {
                TaskInstruction::RunProgram { session, result } => {
                    assert_eq!(*session, SLOT_SESSION);
                    assert_eq!(*result, SLOT_RESULT);
                }
                other => panic!("最后一条指令应为 RunProgram，实际为 {:?}", other),
            }
    
            // 补偿是 ShutdownSession
            match &program.compensation[0] {
                TaskInstruction::ShutdownSession { session } => {
                    assert_eq!(*session, SLOT_SESSION);
                }
                other => panic!("补偿指令应为 ShutdownSession，实际为 {:?}", other),
            }
        }
    
        /// TC-02: compile_run 空路径 → InvalidParameter
        #[test]
        fn tc02_compile_run_empty_path() {
            let compiler = Compiler;
            let result = compiler.compile_run(test_job_id(), "".into(), None);
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("模型路径不能为空"), "错误消息: {}", err_msg);
        }
    
        // ─── B 组：compile_coordinator ────────────────────────
    
        /// TC-03: compile_coordinator 正常编译 — 指令数量 12 + 补偿 1
        #[test]
        fn tc03_compile_coordinator_structure() {
            let compiler = Compiler;
            let program = compiler
                .compile_coordinator(
                    test_job_id(),
                    "model.gguf".into(),
                    vec!["peer1".into()],
                    Some("cuda".into()),
                    0,
                    24,
                )
                .unwrap();
    
            assert_eq!(program.instructions.len(), 12, "正向指令数量");
            assert_eq!(program.compensation.len(), 1, "补偿指令数量");
    
            // 补偿是 ShutdownSession
            match &program.compensation[0] {
                TaskInstruction::ShutdownSession { session } => {
                    assert_eq!(*session, SLOT_SESSION);
                }
                other => panic!("补偿指令应为 ShutdownSession，实际为 {:?}", other),
            }
        }
    
        /// TC-04: RequestPipeline 在 OpenTensorStream 之前
        #[test]
        fn tc04_coordinator_request_before_open() {
            let compiler = Compiler;
            let program = compiler
                .compile_coordinator(
                    test_job_id(),
                    "model.gguf".into(),
                    vec!["peer1".into()],
                    None,
                    0,
                    24,
                )
                .unwrap();
    
            let idx_request = find_instr(&program, |i| {
                matches!(i, TaskInstruction::RequestPipeline { .. })
            })
            .expect("应包含 RequestPipeline");
    
            let idx_open = find_instr(&program, |i| {
                matches!(i, TaskInstruction::OpenTensorStream { .. })
            })
            .expect("应包含 OpenTensorStream");
    
            assert!(
                idx_request < idx_open,
                "RequestPipeline({}) 应在 OpenTensorStream({}) 之前",
                idx_request,
                idx_open
            );
        }
    
        /// TC-05: CreateSession 有 tensor_io = Some(SLOT_TENSOR_IO)
        #[test]
        fn tc05_coordinator_create_session_with_tensor_io() {
            let compiler = Compiler;
            let program = compiler
                .compile_coordinator(
                    test_job_id(),
                    "model.gguf".into(),
                    vec!["peer1".into()],
                    None,
                    0,
                    24,
                )
                .unwrap();
    
            let idx = find_instr(&program, |i| {
                matches!(i, TaskInstruction::CreateSession { .. })
            })
            .expect("应包含 CreateSession");
    
            match &program.instructions[idx] {
                TaskInstruction::CreateSession { tensor_io, .. } => {
                    assert_eq!(
                        *tensor_io,
                        Some(SLOT_TENSOR_IO),
                        "Coordinator 的 CreateSession 应有 tensor_io"
                    );
                }
                _ => unreachable!(),
            }
        }
    
        /// TC-06: compile_coordinator 空 model_path → InvalidParameter
        #[test]
        fn tc06_coordinator_empty_model_path() {
            let compiler = Compiler;
            let result = compiler.compile_coordinator(
                test_job_id(),
                "".into(),
                vec!["peer1".into()],
                None,
                0,
                24,
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("模型路径不能为空"), "错误消息: {}", err_msg);
        }
    
        /// TC-07: compile_coordinator 空 peers 列表 → InvalidParameter
        #[test]
        fn tc07_coordinator_empty_peers() {
            let compiler = Compiler;
            let result = compiler.compile_coordinator(
                test_job_id(),
                "model.gguf".into(),
                vec![],
                None,
                0,
                24,
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("至少需要一个 Peer"), "错误消息: {}", err_msg);
        }
    
        /// TC-08: compile_coordinator 空 peer_id → InvalidParameter
        #[test]
        fn tc08_coordinator_empty_peer_id() {
            let compiler = Compiler;
            let result = compiler.compile_coordinator(
                test_job_id(),
                "model.gguf".into(),
                vec!["".into()],
                None,
                0,
                24,
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("peer_id 不能为空"), "错误消息: {}", err_msg);
        }
    
        // ─── C 组：compile_relay ──────────────────────────────
    
        /// TC-09: compile_relay 正常编译 — 指令数量 12 + 补偿 1
        #[test]
        fn tc09_compile_relay_structure() {
            let compiler = Compiler;
            let program = compiler
                .compile_relay(
                    test_job_id(),
                    "12D3KooW_peer".into(),
                    100,
                    "model_shard.gguf".into(),
                    Some("cpu".into()),
                    8,
                    16,
                )
                .unwrap();
    
            assert_eq!(program.instructions.len(), 12, "正向指令数量");
            assert_eq!(program.compensation.len(), 1, "补偿指令数量");
    
            // 补偿是 ShutdownSession
            match &program.compensation[0] {
                TaskInstruction::ShutdownSession { session } => {
                    assert_eq!(*session, SLOT_SESSION);
                }
                other => panic!("补偿指令应为 ShutdownSession，实际为 {:?}", other),
            }
        }
    
        /// TC-10: SLOT_COORDINATOR_JOB 正确连接到 OpenTensorStream.target_job
        #[test]
        fn tc10_relay_coordinator_job_connects_to_open_tensor_stream() {
            let coordinator_job_id: u64 = 100;
            let compiler = Compiler;
            let program = compiler
                .compile_relay(
                    test_job_id(),
                    "12D3KooW_peer".into(),
                    coordinator_job_id,
                    "model_shard.gguf".into(),
                    None,
                    8,
                    16,
                )
                .unwrap();
    
            // 第 2 条（索引 1）应该是 Const 写入 SLOT_COORDINATOR_JOB
            match &program.instructions[1] {
                TaskInstruction::Const { value, dst } => {
                    assert_eq!(*dst, SLOT_COORDINATOR_JOB);
                    match value {
                        ConstValue::U64(v) => assert_eq!(*v, coordinator_job_id),
                        other => panic!("SLOT_COORDINATOR_JOB 应为 U64，实际为 {:?}", other),
                    }
                }
                other => panic!("第 2 条指令应为 Const，实际为 {:?}", other),
            }
    
            // OpenTensorStream 的 target_job 应为 SLOT_COORDINATOR_JOB
            let idx_open = find_instr(&program, |i| {
                matches!(i, TaskInstruction::OpenTensorStream { .. })
            })
            .expect("应包含 OpenTensorStream");
    
            match &program.instructions[idx_open] {
                TaskInstruction::OpenTensorStream { target_job, peer } => {
                    assert_eq!(*target_job, SLOT_COORDINATOR_JOB);
                    assert_eq!(*peer, SLOT_PEER);
                }
                _ => unreachable!(),
            }
        }
    
        /// TC-11: compile_relay 空 coordinator_peer_id → InvalidParameter
        #[test]
        fn tc11_relay_empty_coordinator_peer_id() {
            let compiler = Compiler;
            let result = compiler.compile_relay(
                test_job_id(),
                "".into(),
                100,
                "model.gguf".into(),
                None,
                0,
                16,
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(
                err_msg.contains("coordinator_peer_id 不能为空"),
                "错误消息: {}",
                err_msg
            );
        }
    
        /// TC-12: compile_relay 空 model_file_id → InvalidParameter
        #[test]
        fn tc12_relay_empty_model_file_id() {
            let compiler = Compiler;
            let result = compiler.compile_relay(
                test_job_id(),
                "12D3KooW_peer".into(),
                100,
                "".into(),
                None,
                0,
                16,
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(
                err_msg.contains("model_file_id 不能为空"),
                "错误消息: {}",
                err_msg
            );
        }
    
        // ─── D 组：ML 程序构建 ────────────────────────────────
    
        /// 辅助：检查 ML 指令序列中是否包含某类型的指令
        fn contains_ml_instr<F>(instrs: &[Instruction], predicate: F) -> bool
        where
            F: Fn(&Instruction) -> bool,
        {
            instrs.iter().any(&predicate)
        }
    
        /// TC-13: build_run_ml_program 指令数正确，末尾为 EndOutput
        #[test]
        fn tc13_run_ml_program_structure() {
            let params = Pipeline_Params::default();
            let program = Compiler::build_run_ml_program(&params);
    
            // Input, Encode, Set, Set, Prefill, CopyMeta, Sample, Decode, Output, Loop, EndOutput = 11
            assert_eq!(program.len(), 11, "run ML 指令数量");
    
            // 末尾为 EndOutput
            assert!(
                matches!(program.last(), Some(Instruction::EndOutput)),
                "最后一条应为 EndOutput"
            );
    
            // 不应包含 Send/Receive/SendEOF（单机推理无网络操作）
            assert!(
                !contains_ml_instr(&program, |i| matches!(i, Instruction::Send)),
                "单机推理不应有 Send"
            );
            assert!(
                !contains_ml_instr(&program, |i| matches!(i, Instruction::Receive)),
                "单机推理不应有 Receive"
            );
        }
    
        /// TC-14: build_coordinator_ml_program 含 Send/Receive/SendEOF，末尾 EndOutput
        #[test]
        fn tc14_coordinator_ml_program_structure() {
            let params = Pipeline_Params::default();
            let program = Compiler::build_coordinator_ml_program(&params);
    
            // Input, Encode, Set, Set, Prefill, CopyMeta, Send, Receive, Sample, Decode, Output, Loop, SendEOF, EndOutput = 14
            assert_eq!(program.len(), 14, "coordinator ML 指令数量");
    
            // 末尾为 EndOutput
            assert!(
                matches!(program.last(), Some(Instruction::EndOutput)),
                "最后一条应为 EndOutput"
            );
    
            // 应包含 Send, Receive, SendEOF
            assert!(
                contains_ml_instr(&program, |i| matches!(i, Instruction::Send)),
                "Coordinator 应有 Send"
            );
            assert!(
                contains_ml_instr(&program, |i| matches!(i, Instruction::Receive)),
                "Coordinator 应有 Receive"
            );
            assert!(
                contains_ml_instr(&program, |i| matches!(i, Instruction::SendEOF)),
                "Coordinator 应有 SendEOF"
            );
    
            // SendEOF 在 EndOutput 之前
            let idx_send_eof = program
                .iter()
                .position(|i| matches!(i, Instruction::SendEOF))
                .unwrap();
            let idx_end_output = program
                .iter()
                .position(|i| matches!(i, Instruction::EndOutput))
                .unwrap();
            assert!(
                idx_send_eof < idx_end_output,
                "SendEOF({}) 应在 EndOutput({}) 之前",
                idx_send_eof,
                idx_end_output
            );
        }
    
        /// TC-15: build_relay_ml_program 为单个 Loop，body 含 Receive/BreakIf/Inference(Tensor)/Send
        #[test]
        fn tc15_relay_ml_program_structure() {
            let program = Compiler::build_relay_ml_program();
    
            // 仅一条 Loop 指令
            assert_eq!(program.len(), 1, "relay ML 指令数量应为 1 (单个 Loop)");
    
            match &program[0] {
                Instruction::Loop { body } => {
                    // body: Receive, BreakIf, Inference(Tensor(TENSOR1)), Send = 4
                    assert_eq!(body.len(), 4, "Loop body 指令数量");
    
                    assert!(
                        matches!(body[0], Instruction::Receive),
                        "body[0] 应为 Receive"
                    );
                    assert!(
                        matches!(body[1], Instruction::BreakIf),
                        "body[1] 应为 BreakIf"
                    );
                    match &body[2] {
                        Instruction::Inference { input } => match input {
                            Inference_Input::Tensor(reg) => {
                                assert_eq!(reg.0, TENSOR1.0, "Inference 输入应为 TENSOR1");
                            }
                            other => panic!("Inference 输入应为 Tensor，实际为 {:?}", other),
                        },
                        other => panic!("body[2] 应为 Inference，实际为 {:?}", other),
                    }
                    assert!(
                        matches!(body[3], Instruction::Send),
                        "body[3] 应为 Send"
                    );
                }
                other => panic!("唯一指令应为 Loop，实际为 {:?}", other),
            }
        }
    
        // ─── E 组：compile_distribute ────────────────────────────
    
        /// TC-16: compile_distribute 正常编译 — 单 peer
        #[test]
        fn tc16_compile_distribute_single_peer() {
            let compiler = Compiler;
            let program = compiler
                .compile_distribute(
                    test_job_id(),
                    "model.gguf".into(),
                    vec![("peer_a".into(), 0, 12)],
                )
                .unwrap();
    
            // 正向：Const(model) + AnalyzeModel + 1 peer × (Const×3 + SplitModel + Const + SendFile) = 2 + 6 = 8
            assert_eq!(program.instructions.len(), 8, "正向指令数量 (单 peer)");
            // 补偿为空
            assert_eq!(program.compensation.len(), 0, "补偿指令数量");
    
            // 第 1 条：Const(model_path)
            match &program.instructions[0] {
                TaskInstruction::Const { value, dst } => {
                    assert_eq!(*dst, SLOT_MODEL);
                    match value {
                        ConstValue::String(s) => assert_eq!(s, "model.gguf"),
                        other => panic!("应为 String，实际为 {:?}", other),
                    }
                }
                other => panic!("第 1 条应为 Const，实际为 {:?}", other),
            }
    
            // 第 2 条：AnalyzeModel
            match &program.instructions[1] {
                TaskInstruction::AnalyzeModel { model, result } => {
                    assert_eq!(*model, SLOT_MODEL);
                    assert_eq!(*result, SLOT_MODEL_INFO);
                }
                other => panic!("第 2 条应为 AnalyzeModel，实际为 {:?}", other),
            }
    
            // 第 6 条（索引 5）：SplitModel
            match &program.instructions[5] {
                TaskInstruction::SplitModel { source, .. } => {
                    assert_eq!(*source, SLOT_MODEL, "SplitModel.source 应为 SLOT_MODEL");
                }
                other => panic!("第 6 条应为 SplitModel，实际为 {:?}", other),
            }
    
            // 最后一条：SendFile
            match program.instructions.last() {
                Some(TaskInstruction::SendFile { .. }) => {}
                other => panic!("最后一条应为 SendFile，实际为 {:?}", other),
            }
        }
    
        /// TC-17: compile_distribute 多 peer — 指令数量正确
        #[test]
        fn tc17_compile_distribute_multi_peer() {
            let compiler = Compiler;
            let program = compiler
                .compile_distribute(
                    test_job_id(),
                    "model.gguf".into(),
                    vec![
                        ("peer_a".into(), 0, 8),
                        ("peer_b".into(), 8, 16),
                        ("peer_c".into(), 16, 24),
                    ],
                )
                .unwrap();
    
            // 正向：2 + 3 peers × 6 = 20
            assert_eq!(program.instructions.len(), 20, "正向指令数量 (3 peers)");
            assert_eq!(program.compensation.len(), 0, "补偿指令数量");
    
            // 每个 peer 块包含一个 SplitModel 和一个 SendFile
            let split_count = program.instructions.iter()
                .filter(|i| matches!(i, TaskInstruction::SplitModel { .. }))
                .count();
            let send_count = program.instructions.iter()
                .filter(|i| matches!(i, TaskInstruction::SendFile { .. }))
                .count();
            assert_eq!(split_count, 3, "SplitModel 数量");
            assert_eq!(send_count, 3, "SendFile 数量");
        }
    
        /// TC-18: compile_distribute 动态槽位正确分配
        #[test]
        fn tc18_compile_distribute_dynamic_slots() {
            let compiler = Compiler;
            let program = compiler
                .compile_distribute(
                    test_job_id(),
                    "model.gguf".into(),
                    vec![
                        ("peer_a".into(), 0, 8),
                        ("peer_b".into(), 8, 16),
                    ],
                )
                .unwrap();
    
            // peer 0 的动态槽位基址 = 14，占位 14,15,16,17
            // peer 1 的动态槽位基址 = 18，占位 18,19,20,21
    
            // 查找第一个 SplitModel
            let idx0 = find_instr(&program, |i| matches!(i, TaskInstruction::SplitModel { .. })).unwrap();
            match &program.instructions[idx0] {
                TaskInstruction::SplitModel { start, end, output, .. } => {
                    assert_eq!(*start, SlotId(14), "peer 0 layer_start 槽位");
                    assert_eq!(*end, SlotId(15), "peer 0 layer_end 槽位");
                    assert_eq!(*output, SlotId(16), "peer 0 shard 槽位");
                }
                _ => unreachable!(),
            }
    
            // 查找第二个 SplitModel（跳过第一个）
            let idx1 = program.instructions.iter()
                .enumerate()
                .filter(|(_, i)| matches!(i, TaskInstruction::SplitModel { .. }))
                .nth(1)
                .map(|(idx, _)| idx)
                .unwrap();
            match &program.instructions[idx1] {
                TaskInstruction::SplitModel { start, end, output, .. } => {
                    assert_eq!(*start, SlotId(18), "peer 1 layer_start 槽位");
                    assert_eq!(*end, SlotId(19), "peer 1 layer_end 槽位");
                    assert_eq!(*output, SlotId(20), "peer 1 shard 槽位");
                }
                _ => unreachable!(),
            }
        }
    
        /// TC-19: compile_distribute 空路径 → InvalidParameter
        #[test]
        fn tc19_compile_distribute_empty_path() {
            let compiler = Compiler;
            let result = compiler.compile_distribute(
                test_job_id(),
                "".into(),
                vec![("peer_a".into(), 0, 8)],
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("模型路径不能为空"), "错误消息: {}", err_msg);
        }
    
        /// TC-20: compile_distribute 空 peers → InvalidParameter
        #[test]
        fn tc20_compile_distribute_empty_peers() {
            let compiler = Compiler;
            let result = compiler.compile_distribute(
                test_job_id(),
                "model.gguf".into(),
                vec![],
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("至少需要一个 Peer"), "错误消息: {}", err_msg);
        }
    
        /// TC-21: compile_distribute 空 peer_id → InvalidParameter
        #[test]
        fn tc21_compile_distribute_empty_peer_id() {
            let compiler = Compiler;
            let result = compiler.compile_distribute(
                test_job_id(),
                "model.gguf".into(),
                vec![("".into(), 0, 8)],
            );
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("peer_id 不能为空"), "错误消息: {}", err_msg);
        }
    
        /// TC-22: AnalyzeModel 在所有 SplitModel 之前
        #[test]
        fn tc22_compile_distribute_analyze_before_split() {
            let compiler = Compiler;
            let program = compiler
                .compile_distribute(
                    test_job_id(),
                    "model.gguf".into(),
                    vec![("peer_a".into(), 0, 12)],
                )
                .unwrap();
    
            let idx_analyze = find_instr(&program, |i| {
                matches!(i, TaskInstruction::AnalyzeModel { .. })
            })
            .expect("应包含 AnalyzeModel");
    
            let idx_split = find_instr(&program, |i| {
                matches!(i, TaskInstruction::SplitModel { .. })
            })
            .expect("应包含 SplitModel");
    
            assert!(
                idx_analyze < idx_split,
                "AnalyzeModel({}) 应在 SplitModel({}) 之前",
                idx_analyze,
                idx_split
            );
        }
}
