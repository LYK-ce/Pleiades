// Presented by KeJi
// Date ： 2026-05-11

//! 程序选择器 — 替代 Compiler，通过 TOML 模板加载程序定义。
//!
//! ## 编译期嵌入策略
//! 模板文件通过 `include_str!()` 在编译期嵌入为字符串常量。
//! 运行时仅在 `select()` / `load_ml_program()` 被调用时才做 TOML 反序列化，
//! 避免分发时携带外部 `programs/` 目录，同时不将所有模板预解析到内存中。

use serde::Deserialize;
use std::collections::HashMap;

use super::job::{JobId, JobKind};
use super::orchestrator_vm::OrchestratorInstruction;
use crate::ml_engine::ml_thread_engine_instruction::{
    Inference_Input, Instruction, Pipeline_Params, Set_Target,
};
use crate::ml_engine::ml_thread_register::{
    FLAG1, META1, META2, META5, TENSOR1, TENSOR2, TOKENID2, TOKENID3,
};
use crate::vm_base::{ConstValue, SlotId};

// ─── 约定槽位常量 ─────────────────────────────────────────────

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
/// 远端 Relay Job ID 槽位（保留）
pub const SLOT_TARGET_JOB: SlotId = SlotId(8);
/// Tensor IO Endpoint 槽位（由 Core 在 spawn 前预注入）
pub const SLOT_TENSOR_IO: SlotId = SlotId(11);
/// Coordinator Job ID 槽位（保留）
pub const SLOT_COORDINATOR_JOB: SlotId = SlotId(12);
/// 模型分析结果槽位
pub const SLOT_MODEL_INFO: SlotId = SlotId(13);
/// ML 程序模式槽位
pub const SLOT_ML_PROGRAM_MODE: SlotId = SlotId(15);

// ─── ReceiveFile 专用槽位 ───────────────────────────────────

pub const SLOT_RECEIVE_STREAM: SlotId = SlotId(100);
pub const SLOT_RECEIVE_FILE_NAME: SlotId = SlotId(101);
pub const SLOT_RECEIVE_FILE_SIZE: SlotId = SlotId(102);
pub const SLOT_RECEIVE_CHECKSUM: SlotId = SlotId(103);
pub const SLOT_RECEIVE_RESULT: SlotId = SlotId(104);

// ─── SendFile 专用槽位 ───────────────────────────────────────

pub const SLOT_SEND_FILE: SlotId = SlotId(200);
pub const SLOT_SEND_PEER: SlotId = SlotId(201);

// ─── Pipeline 编排专用槽位 ───────────────────────────────────

pub const SLOT_INFERENCE_ID: SlotId = SlotId(300);
pub const SLOT_PLAN: SlotId = SlotId(301);
pub const SLOT_STREAMS_RESULT: SlotId = SlotId(302);
pub const SLOT_WORKERS_RESULT: SlotId = SlotId(303);

// ─── 编译期嵌入所有模板 ─────────────────────────────────────
// 文件内容编译进二进制，无需运行时文件 IO，也无需分发 programs/ 目录。
// 运行时按需解析：仅在 select() / load_ml_program() 调用时做 TOML 反序列化。

const RUN_TMPL: &str = include_str!("../../programs/orchestrator/Run.tmpl");
const RELAY_TMPL: &str = include_str!("../../programs/orchestrator/Relay.tmpl");
const PIPELINE_TMPL: &str = include_str!("../../programs/orchestrator/Pipeline.tmpl");
const SEND_TMPL: &str = include_str!("../../programs/orchestrator/Send.tmpl");
const RECEIVE_TMPL: &str = include_str!("../../programs/orchestrator/ReceiveFile.tmpl");
const ML_RUN_TMPL: &str = include_str!("../../programs/ml/run.tmpl");
const ML_RELAY_TMPL: &str = include_str!("../../programs/ml/relay.tmpl");
const ML_COORDINATOR_TMPL: &str = include_str!("../../programs/ml/coordinator.tmpl");

// ─── 错误类型 ──────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SelectorError {
    #[error("不支持的 JobKind: {0:?}")]
    UnsupportedKind(JobKind),
    #[error("未知的槽位名: {0}")]
    UnknownSlot(String),
    #[error("未知的 ML 寄存器名: {0}")]
    UnknownRegister(String),
    #[error("TOML 解析失败: {0}")]
    TomlParse(String),
    #[error("未知的指令类型: {0}")]
    UnknownInstructionType(String),
    #[error("参数缺失: 变量 {0} 未提供值")]
    MissingVariable(String),
    #[error("不支持的值类型: {0}")]
    UnsupportedValueType(String),
}

// ─── TOML 反序列化中间结构 ─────────────────────────────────

#[derive(Deserialize)]
struct RawOrchTemplate {
    meta: RawMeta,
    #[serde(default)]
    instructions: Vec<RawOrchInstruction>,
}

#[derive(Deserialize)]
struct RawMeta {
    kind: String,
}

#[derive(Deserialize)]
struct RawOrchInstruction {
    #[serde(rename = "type")]
    inst_type: String,
    #[serde(default)]
    value_type: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    dst: Option<String>,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    io: Option<String>,
    #[serde(default)]
    tensor_io: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    stream: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    file_size: Option<String>,
    #[serde(default)]
    checksum: Option<String>,
    #[serde(default)]
    model_info: Option<String>,
    #[serde(default)]
    inference_id: Option<String>,
    #[serde(default)]
    plan: Option<String>,
}

#[derive(Deserialize)]
struct RawMLTemplate {
    #[serde(default)]
    instructions: Vec<RawMLInstruction>,
}

#[derive(Deserialize)]
struct RawMLInstruction {
    #[serde(rename = "type")]
    inst_type: String,
    #[serde(default)]
    target_type: Option<String>,
    #[serde(default)]
    target_reg: Option<String>,
    #[serde(default)]
    target_value: Option<toml::Value>,
    #[serde(default)]
    tensor_reg: Option<String>,
    #[serde(default)]
    input_type: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    dst: Option<String>,
    #[serde(default)]
    body: Option<Vec<RawMLInstruction>>,
}

// ─── 槽位名 → SlotId 查表 ─────────────────────────────────

fn resolve_slot(name: &str, vars: &HashMap<String, String>) -> Result<SlotId, SelectorError> {
    // 变量替换：$var_name → vars["var_name"]
    let name = if name.starts_with('$') {
        let var_name = &name[1..];
        vars.get(var_name)
            .ok_or_else(|| SelectorError::MissingVariable(var_name.to_string()))?
    } else {
        name
    };

    let id = match name {
        "SLOT_IO" => 0,
        "SLOT_MODEL" => 1,
        "SLOT_DEVICE" => 2,
        "SLOT_SESSION" => 3,
        "SLOT_RESULT" => 4,
        "SLOT_LAYER_START" => 5,
        "SLOT_LAYER_END" => 6,
        "SLOT_PEER" => 7,
        "SLOT_TARGET_JOB" => 8,
        "SLOT_TENSOR_IO" => 11,
        "SLOT_COORDINATOR_JOB" => 12,
        "SLOT_MODEL_INFO" => 13,
        "SLOT_ML_PROGRAM_MODE" => 15,
        "SLOT_RECEIVE_STREAM" => 100,
        "SLOT_RECEIVE_FILE_NAME" => 101,
        "SLOT_RECEIVE_FILE_SIZE" => 102,
        "SLOT_RECEIVE_CHECKSUM" => 103,
        "SLOT_RECEIVE_RESULT" => 104,
        "SLOT_SEND_FILE" => 200,
        "SLOT_SEND_PEER" => 201,
        "SLOT_INFERENCE_ID" => 300,
        "SLOT_PLAN" => 301,
        "SLOT_STREAMS_RESULT" => 302,
        "SLOT_WORKERS_RESULT" => 303,
        _ => return Err(SelectorError::UnknownSlot(name.to_string())),
    };
    Ok(SlotId(id))
}

fn resolve_slot_opt(
    name: &Option<String>,
    vars: &HashMap<String, String>,
) -> Result<Option<SlotId>, SelectorError> {
    match name {
        Some(n) => resolve_slot(n, vars).map(Some),
        None => Ok(None),
    }
}

fn resolve_slot_str(
    name: &str,
    vars: &HashMap<String, String>,
) -> Result<Option<SlotId>, SelectorError> {
    resolve_slot_opt(&Some(name.to_string()), vars)
}

fn resolve_value(
    value: &str,
    value_type: &str,
    vars: &HashMap<String, String>,
) -> Result<ConstValue, SelectorError> {
    // 变量替换
    let value = if value.starts_with('$') {
        let var_name = &value[1..];
        vars.get(var_name)
            .ok_or_else(|| SelectorError::MissingVariable(var_name.to_string()))?
    } else {
        value
    };

    match value_type {
        "String" => Ok(ConstValue::String(value.to_string())),
        "U64" => {
            let n: u64 = value.parse().map_err(|_| {
                SelectorError::UnsupportedValueType(format!("无法解析 U64: {}", value))
            })?;
            Ok(ConstValue::U64(n))
        }
        "F64" => {
            let n: f64 = value.parse().map_err(|_| {
                SelectorError::UnsupportedValueType(format!("无法解析 F64: {}", value))
            })?;
            Ok(ConstValue::F64(n))
        }
        "Bool" => {
            let b: bool = value.parse().map_err(|_| {
                SelectorError::UnsupportedValueType(format!("无法解析 Bool: {}", value))
            })?;
            Ok(ConstValue::Bool(b))
        }
        _ => Err(SelectorError::UnsupportedValueType(value_type.to_string())),
    }
}

// ─── ML 寄存器名 → 寄存器常量 ──────────────────────────────

fn resolve_token_reg(
    name: &str,
) -> Result<crate::ml_engine::ml_thread_register::Token_Reg, SelectorError> {
    match name {
        "TOKENID1" => Ok(crate::ml_engine::ml_thread_register::TOKENID1),
        "TOKENID2" => Ok(TOKENID2),
        "TOKENID3" => Ok(TOKENID3),
        "TOKENID4" => Ok(crate::ml_engine::ml_thread_register::TOKENID4),
        _ => Err(SelectorError::UnknownRegister(name.to_string())),
    }
}

fn resolve_tensor_reg(
    name: &str,
) -> Result<crate::ml_engine::ml_thread_register::Tensor_Reg, SelectorError> {
    match name {
        "TENSOR1" => Ok(TENSOR1),
        "TENSOR2" => Ok(TENSOR2),
        "TENSOR3" => Ok(crate::ml_engine::ml_thread_register::TENSOR3),
        "TENSOR4" => Ok(crate::ml_engine::ml_thread_register::TENSOR4),
        _ => Err(SelectorError::UnknownRegister(name.to_string())),
    }
}

fn resolve_flag_reg(
    name: &str,
) -> Result<crate::ml_engine::ml_thread_register::Flag_Reg, SelectorError> {
    match name {
        "FLAG1" => Ok(FLAG1),
        "FLAG2" => Ok(crate::ml_engine::ml_thread_register::FLAG2),
        "FLAG3" => Ok(crate::ml_engine::ml_thread_register::FLAG3),
        "FLAG4" => Ok(crate::ml_engine::ml_thread_register::FLAG4),
        _ => Err(SelectorError::UnknownRegister(name.to_string())),
    }
}

fn resolve_meta_reg(
    name: &str,
) -> Result<crate::ml_engine::ml_thread_register::Meta_Reg, SelectorError> {
    match name {
        "META1" => Ok(META1),
        "META2" => Ok(META2),
        "META3" => Ok(crate::ml_engine::ml_thread_register::META3),
        "META4" => Ok(crate::ml_engine::ml_thread_register::META4),
        "META5" => Ok(META5),
        "META6" => Ok(crate::ml_engine::ml_thread_register::META6),
        "META7" => Ok(crate::ml_engine::ml_thread_register::META7),
        "META8" => Ok(crate::ml_engine::ml_thread_register::META8),
        _ => Err(SelectorError::UnknownRegister(name.to_string())),
    }
}

// ─── ProgramSelector ───────────────────────────────────────

pub struct ProgramSelector;

impl ProgramSelector {
    /// 根据 JobKind 加载对应 orchestrator TOML 模板，替换变量，产出指令序列。
    pub fn select(
        kind: JobKind,
        _job_id: JobId,
        vars: HashMap<String, String>,
    ) -> Result<Vec<OrchestratorInstruction>, SelectorError> {
        let tmpl_text = match kind {
            JobKind::Run => RUN_TMPL,
            JobKind::Relay => RELAY_TMPL,
            JobKind::Pipeline => PIPELINE_TMPL,
            JobKind::Send => SEND_TMPL,
            JobKind::Receive => RECEIVE_TMPL,
            _ => return Err(SelectorError::UnsupportedKind(kind)),
        };

        // 按需解析：仅在 select() 调用时做 TOML 反序列化
        let template: RawOrchTemplate =
            toml::from_str(tmpl_text).map_err(|e| SelectorError::TomlParse(e.to_string()))?;

        let mut instructions = Vec::new();

        for raw in &template.instructions {
            let inst = Self::convert_orch_instruction(raw, &vars)?;
            instructions.push(inst);
        }

        Ok(instructions)
    }

    fn convert_orch_instruction(
        raw: &RawOrchInstruction,
        vars: &HashMap<String, String>,
    ) -> Result<OrchestratorInstruction, SelectorError> {
        match raw.inst_type.as_str() {
            "Const" => {
                let value_type = raw.value_type.as_deref().unwrap_or("String");
                let value = raw.value.as_deref().unwrap_or("");
                let dst = resolve_slot(raw.dst.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::Const {
                    value: resolve_value(value, value_type, vars)?,
                    dst,
                })
            }
            "Move" => {
                let src = resolve_slot(raw.src.as_deref().unwrap_or(""), vars)?;
                let dst = resolve_slot(raw.dst.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::Move { src, dst })
            }
            "CreateSession" => {
                let model = resolve_slot(raw.model.as_deref().unwrap_or(""), vars)?;
                let device = resolve_slot(raw.device.as_deref().unwrap_or(""), vars)?;
                let start = resolve_slot(raw.start.as_deref().unwrap_or(""), vars)?;
                let end = resolve_slot(raw.end.as_deref().unwrap_or(""), vars)?;
                let io = resolve_slot(raw.io.as_deref().unwrap_or(""), vars)?;
                let tensor_io = resolve_slot_opt(&raw.tensor_io, vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::CreateSession {
                    model,
                    device,
                    start,
                    end,
                    io,
                    tensor_io,
                    result,
                })
            }
            "ShutdownSession" => {
                let session = resolve_slot(raw.session.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::ShutdownSession { session })
            }
            "RunProgram" => {
                let session = resolve_slot(raw.session.as_deref().unwrap_or(""), vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::RunProgram { session, result })
            }
            "AnalyzeModel" => {
                let model = resolve_slot(raw.model.as_deref().unwrap_or(""), vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::AnalyzeModel { model, result })
            }
            "SplitModel" => {
                let source = resolve_slot(raw.model.as_deref().unwrap_or(""), vars)?;
                let start = resolve_slot(raw.start.as_deref().unwrap_or(""), vars)?;
                let end = resolve_slot(raw.end.as_deref().unwrap_or(""), vars)?;
                let output = resolve_slot(raw.file.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::SplitModel {
                    source,
                    start,
                    end,
                    output,
                })
            }
            "SendFile" => {
                let peer = resolve_slot(raw.peer.as_deref().unwrap_or(""), vars)?;
                let file = resolve_slot(raw.file.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::SendFile { peer, file })
            }
            "ReceiveFile" => {
                let stream = resolve_slot(raw.stream.as_deref().unwrap_or(""), vars)?;
                let file_name = resolve_slot(raw.file_name.as_deref().unwrap_or(""), vars)?;
                let file_size = resolve_slot(raw.file_size.as_deref().unwrap_or(""), vars)?;
                let checksum = resolve_slot(raw.checksum.as_deref().unwrap_or(""), vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::ReceiveFile {
                    stream,
                    file_name,
                    file_size,
                    checksum,
                    result,
                })
            }
            "PlanPipeline" => {
                let model_info = resolve_slot(raw.model_info.as_deref().unwrap_or(""), vars)?;
                let inference_id = resolve_slot(raw.inference_id.as_deref().unwrap_or(""), vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::PlanPipeline {
                    model_info,
                    inference_id,
                    result,
                })
            }
            "EstablishStreams" => {
                let plan = resolve_slot(raw.plan.as_deref().unwrap_or(""), vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::EstablishStreams { plan, result })
            }
            "JoinWorkers" => {
                let plan = resolve_slot(raw.plan.as_deref().unwrap_or(""), vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::JoinWorkers { plan, result })
            }
            "Abort" => Ok(OrchestratorInstruction::Abort {
                reason: raw.value.as_deref().unwrap_or("unknown").to_string(),
            }),
            _ => Err(SelectorError::UnknownInstructionType(raw.inst_type.clone())),
        }
    }

    /// 根据 ML mode 加载 ML 程序模板。
    pub fn load_ml_program(
        mode: &str,
        params: &Pipeline_Params,
    ) -> Result<Vec<Instruction>, SelectorError> {
        let tmpl_text = match mode {
            "run" => ML_RUN_TMPL,
            "relay" => ML_RELAY_TMPL,
            "coordinator" => ML_COORDINATOR_TMPL,
            _ => return Err(SelectorError::UnsupportedKind(JobKind::Run)),
        };

        // 按需解析：仅在 load_ml_program() 调用时做 TOML 反序列化
        let template: RawMLTemplate =
            toml::from_str(tmpl_text).map_err(|e| SelectorError::TomlParse(e.to_string()))?;

        let mut instructions = Vec::new();
        for raw in &template.instructions {
            let inst = Self::convert_ml_instruction(raw, params)?;
            instructions.push(inst);
        }
        Ok(instructions)
    }

    fn convert_ml_instruction(
        raw: &RawMLInstruction,
        params: &Pipeline_Params,
    ) -> Result<Instruction, SelectorError> {
        match raw.inst_type.as_str() {
            "Input" => Ok(Instruction::Input),
            "Encode" => Ok(Instruction::Encode),
            "Decode" => Ok(Instruction::Decode),
            "Output" => Ok(Instruction::Output),
            "EndOutput" => Ok(Instruction::EndOutput),
            "Send" => Ok(Instruction::Send),
            "Receive" => Ok(Instruction::Receive),
            "SendEOF" => Ok(Instruction::SendEOF),
            "BreakIf" => Ok(Instruction::BreakIf),
            "Set" => {
                let target_type = raw.target_type.as_deref().unwrap_or("Meta");
                let reg_name = raw.target_reg.as_deref().unwrap_or("");
                let target = match target_type {
                    "Meta" => {
                        let value = resolve_ml_f64(&raw.target_value, params)?;
                        Set_Target::Meta(resolve_meta_reg(reg_name)?, value)
                    }
                    "Flag" => {
                        let value = resolve_ml_bool(&raw.target_value);
                        Set_Target::Flag(resolve_flag_reg(reg_name)?, value)
                    }
                    _ => {
                        return Err(SelectorError::UnsupportedValueType(format!(
                            "unknown Set target_type: {}",
                            target_type
                        )))
                    }
                };
                Ok(Instruction::Set { target })
            }
            "CopyMeta" => {
                let src = resolve_meta_reg(raw.src.as_deref().unwrap_or(""))?;
                let dst = resolve_meta_reg(raw.dst.as_deref().unwrap_or(""))?;
                Ok(Instruction::CopyMeta { src, dst })
            }
            "Prefill" => {
                let input = resolve_token_reg(raw.input.as_deref().unwrap_or(""))?;
                Ok(Instruction::Prefill { input })
            }
            "Sample" => {
                let tensor_reg =
                    resolve_tensor_reg(raw.tensor_reg.as_deref().unwrap_or("TENSOR2"))?;
                Ok(Instruction::Sample { tensor_reg })
            }
            "Inference" => {
                let input_type = raw.input_type.as_deref().unwrap_or("Tokens");
                let input_name = raw.input.as_deref().unwrap_or("TOKENID2");
                let input = match input_type {
                    "Tokens" => Inference_Input::Tokens(resolve_token_reg(input_name)?),
                    "Tensor" => Inference_Input::Tensor(resolve_tensor_reg(input_name)?),
                    _ => {
                        return Err(SelectorError::UnsupportedValueType(format!(
                            "unknown Inference input_type: {}",
                            input_type
                        )))
                    }
                };
                Ok(Instruction::Inference { input })
            }
            "Loop" => {
                let body_raw = raw.body.as_ref().ok_or_else(|| {
                    SelectorError::UnknownInstructionType("Loop 缺少 body 指令序列".to_string())
                })?;
                let mut body = Vec::new();
                for raw_inst in body_raw {
                    body.push(Self::convert_ml_instruction(raw_inst, params)?);
                }
                Ok(Instruction::Loop { body })
            }
            _ => Err(SelectorError::UnknownInstructionType(raw.inst_type.clone())),
        }
    }
}

fn resolve_ml_f64(
    value: &Option<toml::Value>,
    params: &Pipeline_Params,
) -> Result<f64, SelectorError> {
    match value {
        Some(toml::Value::String(s)) => {
            if s == "$max_tokens" {
                Ok(params.max_tokens as f64)
            } else if s.starts_with('$') {
                Err(SelectorError::MissingVariable(s[1..].to_string()))
            } else {
                s.parse::<f64>().map_err(|_| {
                    SelectorError::UnsupportedValueType(format!("无法解析 f64: {}", s))
                })
            }
        }
        Some(toml::Value::Integer(i)) => Ok(*i as f64),
        Some(toml::Value::Float(f)) => Ok(*f),
        _ => Err(SelectorError::UnsupportedValueType(
            "target_value 缺失或类型不支持".to_string(),
        )),
    }
}

fn resolve_ml_bool(value: &Option<toml::Value>) -> bool {
    match value {
        Some(toml::Value::Boolean(b)) => *b,
        Some(toml::Value::String(s)) if s == "false" => false,
        Some(toml::Value::String(s)) if s == "true" => true,
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════════
// 单元测试
// ═══════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::super::orchestrator_vm::OrchestratorInstruction as OI;
    use super::*;

    fn vars(map: &[(&str, &str)]) -> HashMap<String, String> {
        map.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn load_run_template() {
        let v = vars(&[("model_path", "model.gguf"), ("device", "cuda")]);
        let instructions =
            ProgramSelector::select(crate::orchestrator::job::JobKind::Run, JobId(1), v).unwrap();

        assert_eq!(instructions.len(), 6);

        match &instructions[0] {
            OI::Const { value, dst } => {
                assert_eq!(dst.0, 1);
                match value {
                    ConstValue::String(s) => assert_eq!(s, "model.gguf"),
                    _ => panic!("expected String"),
                }
            }
            _ => panic!("expected Const"),
        }
    }

    #[test]
    fn load_relay_template() {
        let v = vars(&[
            ("model_path", "model.gguf"),
            ("device", "cuda"),
            ("layer_start", "0"),
            ("layer_end", "12"),
        ]);
        let instructions =
            ProgramSelector::select(crate::orchestrator::job::JobKind::Relay, JobId(2), v).unwrap();

        assert_eq!(instructions.len(), 7);
        match &instructions[5] {
            OI::CreateSession { tensor_io, .. } => {
                assert!(tensor_io.is_some());
                assert_eq!(tensor_io.unwrap().0, 11);
            }
            _ => panic!("expected CreateSession"),
        }
    }

    #[test]
    fn load_pipeline_template() {
        let v = vars(&[
            ("model_path", "model.gguf"),
            ("inference_id", "42"),
            ("device", "cuda"),
        ]);
        let instructions =
            ProgramSelector::select(crate::orchestrator::job::JobKind::Pipeline, JobId(3), v)
                .unwrap();

        assert_eq!(instructions.len(), 10);

        match &instructions[3] {
            OI::PlanPipeline {
                model_info,
                inference_id,
                result,
            } => {
                assert_eq!(model_info.0, 13);
                assert_eq!(inference_id.0, 300);
                assert_eq!(result.0, 301);
            }
            _ => panic!("expected PlanPipeline"),
        }
    }

    #[test]
    fn load_send_template() {
        let v = vars(&[("file_path", "/tmp/test.gguf"), ("peer_id", "12D3KooW")]);
        let instructions =
            ProgramSelector::select(crate::orchestrator::job::JobKind::Send, JobId(4), v).unwrap();

        assert_eq!(instructions.len(), 3);
        match &instructions[2] {
            OI::SendFile { peer, file } => {
                assert_eq!(peer.0, 201);
                assert_eq!(file.0, 200);
            }
            _ => panic!("expected SendFile"),
        }
    }

    #[test]
    fn load_receive_file_template() {
        let v = vars(&[
            ("file_name", "test.gguf"),
            ("file_size", "1024"),
            ("checksum", "abc123"),
        ]);
        let instructions =
            ProgramSelector::select(crate::orchestrator::job::JobKind::Receive, JobId(5), v)
                .unwrap();

        assert_eq!(instructions.len(), 4);
        match &instructions[3] {
            OI::ReceiveFile {
                stream,
                file_name,
                file_size,
                checksum,
                result,
            } => {
                assert_eq!(stream.0, 100);
                assert_eq!(file_name.0, 101);
                assert_eq!(file_size.0, 102);
                assert_eq!(checksum.0, 103);
                assert_eq!(result.0, 104);
            }
            _ => panic!("expected ReceiveFile"),
        }
    }

    #[test]
    fn load_ml_run_template() {
        let params = Pipeline_Params::default();
        let program = ProgramSelector::load_ml_program("run", &params).unwrap();

        assert_eq!(program.len(), 11);
        assert!(matches!(program.last(), Some(Instruction::EndOutput)));
    }

    #[test]
    fn load_ml_relay_template() {
        let params = Pipeline_Params::default();
        let program = ProgramSelector::load_ml_program("relay", &params).unwrap();

        assert_eq!(program.len(), 2);
        match &program[0] {
            Instruction::Loop { body } => {
                assert_eq!(body.len(), 4);
            }
            _ => panic!("expected Loop"),
        }
        assert!(matches!(&program[1], Instruction::SendEOF));
    }

    #[test]
    fn load_ml_coordinator_template() {
        let params = Pipeline_Params::default();
        let program = ProgramSelector::load_ml_program("coordinator", &params).unwrap();

        assert_eq!(program.len(), 14);
        assert!(matches!(program.last(), Some(Instruction::EndOutput)));
    }

    #[test]
    fn unsupported_kind_returns_error() {
        let v = vars(&[]);
        let result =
            ProgramSelector::select(crate::orchestrator::job::JobKind::Coordinator, JobId(1), v);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_variable_returns_error() {
        let v = vars(&[]);
        let result = ProgramSelector::select(crate::orchestrator::job::JobKind::Run, JobId(1), v);
        assert!(result.is_err());
    }
}
