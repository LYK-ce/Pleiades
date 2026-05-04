//Presented by KeJi
//Date ： 2026-04-29

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
/// 远端 Relay Job ID 槽位（保留，未来可能被 Coordinator 编排逻辑使用）
pub const SLOT_TARGET_JOB: SlotId = SlotId(8);
/// 组装后的 Tensor IO Endpoint 槽位（由 Core 在 spawn 前预注入）
pub const SLOT_TENSOR_IO: SlotId = SlotId(11);
/// Coordinator Job ID 槽位（保留，未使用）
pub const SLOT_COORDINATOR_JOB: SlotId = SlotId(12);
/// ML 程序模式槽位（由 Compiler 写入，RunProgram handler 读取以分派不同 ML 程序）
/// 值: "run"(默认/单机) | "relay"(Worker中继) | "coordinator"(分布式协调者)
pub const SLOT_ML_PROGRAM_MODE: SlotId = SlotId(15);
/// 模型分析结果槽位（AnalyzeModel 写入）
pub const SLOT_MODEL_INFO: SlotId = SlotId(13);
/// 文件分发动态槽位起始偏移（每个 peer 占 4 个槽位）
pub const DISTRIBUTE_DYNAMIC_SLOT_BASE: u32 = 14;

// ─── ReceiveFile 专用槽位 ────────────────────────────────
// Core 在 FileStreamArrived 时注入 stream 到 SLOT_RECEIVE_STREAM，
// 其余元数据由 compile_receive_file 生成的 Const 指令写入。

/// 入站 stream 槽位（Core 外部注入，compile_receive_file 引用）
pub const SLOT_RECEIVE_STREAM: SlotId = SlotId(100);
/// 文件名槽位（Const 指令写入）
pub const SLOT_RECEIVE_FILE_NAME: SlotId = SlotId(101);
/// 文件大小槽位（Const 指令写入）
pub const SLOT_RECEIVE_FILE_SIZE: SlotId = SlotId(102);
/// 校验和槽位（Const 指令写入）
pub const SLOT_RECEIVE_CHECKSUM: SlotId = SlotId(103);
/// 接收结果槽位（ReceiveFile 指令写入 file_id）
pub const SLOT_RECEIVE_RESULT: SlotId = SlotId(104);

// ─── Pipeline 编排专用槽位 ────────────────────────────────
/// Pipeline inference_id 槽位（PlanPipeline 读取）
pub const SLOT_INFERENCE_ID: SlotId = SlotId(300);
/// Pipeline 拓扑规划槽位（PlanPipeline 写入，EstablishStreams/JoinWorkers 读取）
pub const SLOT_PLAN: SlotId = SlotId(301);
/// EstablishStreams 结果槽位
pub const SLOT_STREAMS_RESULT: SlotId = SlotId(302);
/// JoinWorkers 结果槽位
pub const SLOT_WORKERS_RESULT: SlotId = SlotId(303);

// ─── SendFile（单文件发送）专用槽位 ────────────────────────
/// 发送文件路径槽位
pub const SLOT_SEND_FILE: SlotId = SlotId(200);
/// 发送目标节点 ID 槽位
pub const SLOT_SEND_PEER: SlotId = SlotId(201);

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

    /// 编译 Pipeline 作业的 TaskProgram（分布式协调者 — 三阶段流水线编排）
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(inference_id→PLAN) → EstablishStreams → JoinWorkers
    ///       → Const(model) → Const(device) → Const(start) → Const(end)
    ///       → Const("coordinator"→MODE) → CreateSession(tensor_io) → RunProgram
    /// 补偿：TeardownPipeline → ShutdownSession
    /// ```
    ///
    /// `inference_id` 由 Core 在 route_user 中预生成并传入。
    /// plan 槽位当前仅存储 inference_id（Scheduler 未实现时的占位）。
    /// 后续 Scheduler 实现后，plan 将包含完整拓扑信息。
    pub fn compile_pipeline(
        &self,
        _job_id: JobId,
        inference_id: u64,
        model_path: String,
        device: String,
    ) -> Result<TaskProgram, CompilerError> {
        if model_path.is_empty() {
            return Err(CompilerError::InvalidParameter("模型路径不能为空".to_string()));
        }

        let mut builder = TaskProgramBuilder::new();

        // ─── 正向序列 ─────────────────────────────────────────

        // 1. 准备模型路径 + inference_id
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(model_path),
            dst: SLOT_MODEL,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(inference_id),
            dst: SLOT_INFERENCE_ID,
        });

        // 2. 分析模型（获取层数等信息）
        builder.push_instruction(TaskInstruction::AnalyzeModel {
            model: SLOT_MODEL,
            result: SLOT_MODEL_INFO,
        });

        // 3. 规划拓扑（查询 PeerManager + Scheduler 均分）
        builder.push_instruction(TaskInstruction::PlanPipeline {
            model_info: SLOT_MODEL_INFO,
            inference_id: SLOT_INFERENCE_ID,
            result: SLOT_PLAN,
        });

        // 4. Phase 1: 建立张量流
        builder.push_instruction(TaskInstruction::EstablishStreams {
            plan: SLOT_PLAN,
            result: SLOT_STREAMS_RESULT,
        });

        // 5. Phase 2: 通知 Worker 加入流水线
        builder.push_instruction(TaskInstruction::JoinWorkers {
            plan: SLOT_PLAN,
            result: SLOT_WORKERS_RESULT,
        });

        // 6. Phase 3: Coordinator 自身启动推理
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(device),
            dst: SLOT_DEVICE,
        });
        // 注：SLOT_LAYER_START / SLOT_LAYER_END 由 EstablishStreams handler 从 plan 中提取并动态写入
        // ML 程序模式标记：告诉 RunProgram handler 使用 coordinator ML 程序
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String("coordinator".to_string()),
            dst: SLOT_ML_PROGRAM_MODE,
        });
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

        // ─── 补偿序列 ─────────────────────────────────────────

        // 清理张量流 + 通知 Worker 停止
        builder.push_compensation(TaskInstruction::TeardownPipeline {
            plan: SLOT_PLAN,
        });
        // 关闭 ML Session
        builder.push_compensation(TaskInstruction::ShutdownSession {
            session: SLOT_SESSION,
        });

        Ok(builder.build())
    }

    /// 编译 Relay 作业的 TaskProgram（分布式 Worker — 流水线推理）
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(model) → Const(device) → Const(start) → Const(end) → Const("relay") → CreateSession(tensor_io) → RunProgram
    /// 补偿：ShutdownSession
    /// ```
    ///
    /// `RunProgram` handler 读取 SLOT_ML_PROGRAM_MODE="relay" 后使用 `build_relay_ml_program()` 生成 ML 指令序列：
    /// ```text
    /// Loop [ Receive, BreakIf, Inference(TENSOR1), Send ]
    /// SendEOF
    /// ```
    pub fn compile_relay(
        &self,
        _job_id: JobId,
        model_file_id: String,
        device: String,
        layer_start: usize,
        layer_end: usize,
    ) -> Result<TaskProgram, CompilerError> {
        if model_file_id.is_empty() {
            return Err(CompilerError::InvalidParameter("model_file_id 不能为空".to_string()));
        }

        let mut builder = TaskProgramBuilder::new();

        // 正向序列：Const(model) → Const(device) → Const(start) → Const(end) → Const(mode) → CreateSession → RunProgram
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
        // ML 程序模式标记：告诉 RunProgram handler 使用 relay ML 程序
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String("relay".to_string()),
            dst: SLOT_ML_PROGRAM_MODE,
        });
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

        // 补偿序列：ShutdownSession
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

    /// 编译 Send 作业的 TaskProgram（向指定节点发送单个文件）
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(file_path) → Const(peer_id) → SendFile
    /// 补偿：（空 — 发送失败无需回滚）
    /// ```
    pub fn compile_send(
        &self,
        _job_id: JobId,
        file_path: String,
        peer_id: String,
    ) -> Result<TaskProgram, CompilerError> {
        if file_path.is_empty() {
            return Err(CompilerError::InvalidParameter("文件路径不能为空".to_string()));
        }
        if peer_id.is_empty() {
            return Err(CompilerError::InvalidParameter("节点 ID 不能为空".to_string()));
        }

        let mut builder = TaskProgramBuilder::new();

        // 1. 注入文件路径
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(file_path),
            dst: SLOT_SEND_FILE,
        });

        // 2. 注入目标节点 ID
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(peer_id),
            dst: SLOT_SEND_PEER,
        });

        // 3. 发送文件
        builder.push_instruction(TaskInstruction::SendFile {
            peer: SLOT_SEND_PEER,
            file: SLOT_SEND_FILE,
        });

        // 补偿为空 — 发送失败无需回滚
        Ok(builder.build())
    }

    /// 编译 ReceiveFile 作业的 TaskProgram（接收方文件接收）
    ///
    /// Core 在 FileStreamArrived 中调用此方法，生成只含一条 ReceiveFile 指令的程序。
    /// **入站 stream 由 Core 在 spawn 前通过 `executor.inject_slot()` 注入**，
    /// 其余元数据（file_name/file_size/checksum）由此方法生成 Const 指令写入。
    ///
    /// 生成的指令序列：
    /// ```text
    /// 正向：Const(file_name) → Const(file_size) → Const(checksum) → ReceiveFile
    /// 补偿：（空 — 接收失败时 handler 自行清理损坏文件）
    /// ```
    pub fn compile_receive_file(
        &self,
        _job_id: JobId,
        file_name: String,
        file_size: u64,
        checksum: String,
    ) -> Result<TaskProgram, CompilerError> {
        if file_name.is_empty() {
            return Err(CompilerError::InvalidParameter("file_name 不能为空".to_string()));
        }

        let mut builder = TaskProgramBuilder::new();

        // 1. 注入元数据（Const 指令）
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(file_name),
            dst: SLOT_RECEIVE_FILE_NAME,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::U64(file_size),
            dst: SLOT_RECEIVE_FILE_SIZE,
        });
        builder.push_instruction(TaskInstruction::Const {
            value: ConstValue::String(checksum),
            dst: SLOT_RECEIVE_CHECKSUM,
        });

        // 2. ReceiveFile 指令（stream 槽由 Core 外部注入）
        builder.push_instruction(TaskInstruction::ReceiveFile {
            stream: SLOT_RECEIVE_STREAM,
            file_name: SLOT_RECEIVE_FILE_NAME,
            file_size: SLOT_RECEIVE_FILE_SIZE,
            checksum: SLOT_RECEIVE_CHECKSUM,
            result: SLOT_RECEIVE_RESULT,
        });

        // 补偿为空 — ReceiveFile handler 自行清理损坏文件
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
    /// SendEOF
    /// ```
    pub fn build_relay_ml_program() -> Vec<Instruction> {
        vec![
            Instruction::Loop {
                body: vec![
                    // Worker ← 接收上游节点发来的张量
                    Instruction::Receive,
                    // EOF 帧时 FLAG1=true → BreakIf 退出循环
                    Instruction::BreakIf,
                    // 从 TENSOR1（Receive 写入）做前向推理
                    Instruction::Inference {
                        input: Inference_Input::Tensor(TENSOR1),
                    },
                    // Worker → 发送本地前向输出到下游节点
                    Instruction::Send,
                ],
            },
            // 循环退出后向下游节点发送 EOF，形成 pipeline 链式终止传播
            Instruction::SendEOF,
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
                // 旧版 Coordinator 已废弃，使用 Pipeline 替代
                Err(CompilerError::InvalidParameter(
                    "Coordinator 已废弃，请使用 Pipeline 命令".to_string()
                ))
            }
            JobKind::Pipeline => {
                // Pipeline 由 Core 直接调用 compile_pipeline，不经过通用 compile 接口
                // （因为需要额外的 inference_id 参数）
                Err(CompilerError::InvalidParameter(
                    "Pipeline 不通过通用 compile 接口，请使用 compile_pipeline".to_string()
                ))
            }
            JobKind::Relay => {
                let model_file_id = params.model_path.ok_or_else(|| {
                    CompilerError::InvalidParameter("Relay 作业需要 model_file_id".to_string())
                })?;
                let device = params.device_preference.unwrap_or_else(|| "cpu".to_string());
                self.compile_relay(
                    job_id,
                    model_file_id,
                    device,
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
            JobKind::Send => {
                let file_path = params.model_path.ok_or_else(|| {
                    CompilerError::InvalidParameter("Send 作业需要 file_path (model_path)".to_string())
                })?;
                let peer_id = params.send_peer_id.ok_or_else(|| {
                    CompilerError::InvalidParameter("Send 作业需要 send_peer_id".to_string())
                })?;
                self.compile_send(job_id, file_path, peer_id)
            }
            JobKind::Receive => {
                let file_name = params.receive_file_name.ok_or_else(|| {
                    CompilerError::InvalidParameter("Receive 作业需要 receive_file_name".to_string())
                })?;
                let file_size = params.receive_file_size.ok_or_else(|| {
                    CompilerError::InvalidParameter("Receive 作业需要 receive_file_size".to_string())
                })?;
                let checksum = params.receive_checksum.ok_or_else(|| {
                    CompilerError::InvalidParameter("Receive 作业需要 receive_checksum".to_string())
                })?;
                self.compile_receive_file(job_id, file_name, file_size, checksum)
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
    /// ReceiveFile 作业：文件名
    pub receive_file_name: Option<String>,
    /// ReceiveFile 作业：文件大小
    pub receive_file_size: Option<u64>,
    /// ReceiveFile 作业：校验和
    pub receive_checksum: Option<String>,
    /// Send 作业：目标节点 ID
    pub send_peer_id: Option<String>,
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
    
        // ─── B 组：compile_coordinator / compile_relay ────────
        // 注：compile_coordinator 和 compile_relay 当前为 todo!() 占位符，
        // 待新三阶段 Pipeline 方案实现后补充测试。

        // ─── C 组：ML 程序构建 ────────────────────────────────
    
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
    
        /// TC-15: build_relay_ml_program 为 Loop + SendEOF，body 含 Receive/BreakIf/Inference(Tensor)/Send
        #[test]
        fn tc15_relay_ml_program_structure() {
            let program = Compiler::build_relay_ml_program();
    
            // Loop + SendEOF = 2 条指令
            assert_eq!(program.len(), 2, "relay ML 指令数量应为 2 (Loop + SendEOF)");
    
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
                other => panic!("program[0] 应为 Loop，实际为 {:?}", other),
            }

            // SendEOF: 循环退出后向下游传播 EOF
            assert!(
                matches!(program[1], Instruction::SendEOF),
                "program[1] 应为 SendEOF"
            );
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
