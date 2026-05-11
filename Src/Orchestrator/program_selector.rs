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
use crate::ml_engine::ml_vm::slots::{
    SLOT_FLAG1 as SL_ML_FLAG1, SLOT_META1 as SL_ML_META1, SLOT_META2 as SL_ML_META2,
    SLOT_META4 as SL_ML_META4, SLOT_META5 as SL_ML_META5, SLOT_META6 as SL_ML_META6,
    SLOT_TENSOR1 as SL_ML_TENSOR1, SLOT_TENSOR2 as SL_ML_TENSOR2, SLOT_TEXT1 as SL_ML_TEXT1,
    SLOT_TEXT2 as SL_ML_TEXT2, SLOT_TOKENS1 as SL_ML_TOKENS1, SLOT_TOKENS2 as SL_ML_TOKENS2,
    SLOT_TOKENS3 as SL_ML_TOKENS3,
};
use crate::ml_engine::ml_vm::InferenceInputType as MlInputType;
use crate::ml_engine::ml_vm::MlInstruction as MlInst;
use crate::ml_engine::pipeline::Pipeline_Params;
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
pub const SLOT_HIDDEN_DIM: SlotId = SlotId(304);

// ─── 编译期嵌入所有模板 ─────────────────────────────────────
// 文件内容编译进二进制，无需运行时文件 IO，也无需分发 programs/ 目录。
// 运行时按需解析：仅在 select() / load_ml_program() 调用时做 TOML 反序列化。

const RUN_TMPL: &str = include_str!("../../programs/orchestrator/Run.tmpl");
const RELAY_TMPL: &str = include_str!("../../programs/orchestrator/Relay.tmpl");
const PIPELINE_TMPL: &str = include_str!("../../programs/orchestrator/Pipeline.tmpl");
const SEND_TMPL: &str = include_str!("../../programs/orchestrator/Send.tmpl");
const RECEIVE_TMPL: &str = include_str!("../../programs/orchestrator/ReceiveFile.tmpl");
const PROFILE_TMPL: &str = include_str!("../../programs/orchestrator/Profile.tmpl");
const ML_RUN_TMPL: &str = include_str!("../../programs/ml/run.tmpl");
const ML_RELAY_TMPL: &str = include_str!("../../programs/ml/relay.tmpl");
const ML_COORDINATOR_TMPL: &str = include_str!("../../programs/ml/coordinator.tmpl");
const ML_PROFILE_TMPL: &str = include_str!("../../programs/ml/profile.tmpl");

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
    tensor_slot: Option<String>,
    #[serde(default)]
    input_type: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    condition: Option<String>,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    dst: Option<String>,
    #[serde(default)]
    value_type: Option<String>,
    #[serde(default)]
    value: Option<toml::Value>,
    #[serde(default)]
    shape_slots: Option<Vec<String>>,
    #[serde(default)]
    delta: Option<f64>,
    #[serde(default)]
    target: Option<usize>,
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
        "SLOT_HIDDEN_DIM" => 304,
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

// ─── ML 槽位名 → SlotId 查表 ─────────────────────────────

fn resolve_ml_slot(name: &str) -> Result<SlotId, SelectorError> {
    let id = match name {
        "TEXT1" => SL_ML_TEXT1.0,
        "TEXT2" => SL_ML_TEXT2.0,
        "TEXT3" => 1002,
        "TEXT4" => 1003,
        "TOKENS1" => SL_ML_TOKENS1.0,
        "TOKENS2" => SL_ML_TOKENS2.0,
        "TOKENS3" => SL_ML_TOKENS3.0,
        "TOKENS4" => 1013,
        "TENSOR1" => SL_ML_TENSOR1.0,
        "TENSOR2" => SL_ML_TENSOR2.0,
        "TENSOR3" => 1022,
        "TENSOR4" => 1023,
        "FLAG1" => SL_ML_FLAG1.0,
        "FLAG2" => 1031,
        "FLAG3" => 1032,
        "FLAG4" => 1033,
        "META1" => SL_ML_META1.0,
        "META2" => SL_ML_META2.0,
        "META3" => 1042,
        "META4" => SL_ML_META4.0,
        "META5" => SL_ML_META5.0,
        "META6" => SL_ML_META6.0,
        "META7" => 1046,
        "META8" => 1047,
        _ => return Err(SelectorError::UnknownSlot(name.to_string())),
    };
    Ok(SlotId(id))
}

fn resolve_ml_const_value(
    raw_value: &Option<toml::Value>,
    value_type: &str,
    params: &Pipeline_Params,
    vars: &HashMap<String, String>,
) -> Result<ConstValue, SelectorError> {
    match raw_value {
        Some(toml::Value::Integer(i)) => match value_type {
            "U64" => Ok(ConstValue::U64(*i as u64)),
            "F64" => Ok(ConstValue::F64(*i as f64)),
            _ => Err(SelectorError::UnsupportedValueType(format!(
                "不支持的类型组合: {} + Integer",
                value_type
            ))),
        },
        Some(toml::Value::Float(f)) => match value_type {
            "F64" => Ok(ConstValue::F64(*f)),
            _ => Err(SelectorError::UnsupportedValueType(format!(
                "不支持的类型组合: {} + Float",
                value_type
            ))),
        },
        Some(toml::Value::Boolean(b)) => match value_type {
            "Bool" => Ok(ConstValue::Bool(*b)),
            _ => Err(SelectorError::UnsupportedValueType(format!(
                "不支持的类型组合: {} + Bool",
                value_type
            ))),
        },
        Some(toml::Value::String(s)) => {
            let value = if s == "$max_tokens" {
                params.max_tokens.to_string()
            } else if s.starts_with('$') {
                let var_name = &s[1..];
                vars.get(var_name).cloned().unwrap_or_else(|| {
                    // 回退到 params 中查找
                    match var_name {
                        "max_tokens" => params.max_tokens.to_string(),
                        _ => s.clone(),
                    }
                })
            } else {
                s.clone()
            };
            match value_type {
                "F64" => Ok(ConstValue::F64(value.parse().map_err(|_| {
                    SelectorError::UnsupportedValueType(format!("无法解析 F64: {}", value))
                })?)),
                "Bool" => Ok(ConstValue::Bool(value.parse().map_err(|_| {
                    SelectorError::UnsupportedValueType(format!("无法解析 Bool: {}", value))
                })?)),
                "U64" => Ok(ConstValue::U64(value.parse().map_err(|_| {
                    SelectorError::UnsupportedValueType(format!("无法解析 U64: {}", value))
                })?)),
                "String" => Ok(ConstValue::String(value)),
                _ => Err(SelectorError::UnsupportedValueType(value_type.to_string())),
            }
        }
        _ => Err(SelectorError::UnsupportedValueType(
            "value 缺失或类型不支持".to_string(),
        )),
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
            JobKind::Profile => PROFILE_TMPL,
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
            "Profile" => {
                let session = resolve_slot(raw.session.as_deref().unwrap_or(""), vars)?;
                let result = resolve_slot(raw.result.as_deref().unwrap_or(""), vars)?;
                Ok(OrchestratorInstruction::Profile { session, result })
            }
            "Abort" => Ok(OrchestratorInstruction::Abort {
                reason: raw.value.as_deref().unwrap_or("unknown").to_string(),
            }),
            _ => Err(SelectorError::UnknownInstructionType(raw.inst_type.clone())),
        }
    }
}

// ─── ML 程序加载（VM 版）──────────────────────────────────────

impl ProgramSelector {
    pub fn load_ml_program_vm(
        mode: &str,
        params: &Pipeline_Params,
        vars: &HashMap<String, String>,
    ) -> Result<Vec<MlInst>, SelectorError> {
        let tmpl_text = match mode {
            "run" => ML_RUN_TMPL,
            "relay" => ML_RELAY_TMPL,
            "coordinator" => ML_COORDINATOR_TMPL,
            "profile" => ML_PROFILE_TMPL,
            _ => return Err(SelectorError::UnsupportedKind(JobKind::Run)),
        };
        let template: RawMLTemplate =
            toml::from_str(tmpl_text).map_err(|e| SelectorError::TomlParse(e.to_string()))?;
        let mut instructions = Vec::new();
        for raw in &template.instructions {
            instructions.push(Self::convert_ml_instruction_vm(raw, params, vars)?);
        }
        Ok(instructions)
    }

    fn convert_ml_instruction_vm(
        raw: &RawMLInstruction,
        params: &Pipeline_Params,
        vars: &HashMap<String, String>,
    ) -> Result<MlInst, SelectorError> {
        match raw.inst_type.as_str() {
            "Input" => Ok(MlInst::Input),
            "Encode" => Ok(MlInst::Encode),
            "Decode" => Ok(MlInst::Decode),
            "Output" => Ok(MlInst::Output),
            "EndOutput" => Ok(MlInst::EndOutput),
            "Send" => Ok(MlInst::Send),
            "Receive" => Ok(MlInst::Receive),
            "SendEOF" => Ok(MlInst::SendEOF),
            "Const" => {
                let value_type = raw.value_type.as_deref().unwrap_or("String");
                let dst = resolve_ml_slot(raw.dst.as_deref().unwrap_or(""))?;
                Ok(MlInst::Const {
                    value: resolve_ml_const_value(&raw.value, value_type, params, vars)?,
                    dst,
                })
            }
            "Move" => {
                let src = resolve_ml_slot(raw.src.as_deref().unwrap_or(""))?;
                let dst = resolve_ml_slot(raw.dst.as_deref().unwrap_or(""))?;
                Ok(MlInst::Move { src, dst })
            }
            "Add" => {
                let dst = resolve_ml_slot(raw.dst.as_deref().unwrap_or(""))?;
                let delta = raw.delta.unwrap_or(0.0);
                Ok(MlInst::Add { dst, delta })
            }
            "Jump" => {
                let target = raw.target.unwrap_or(0);
                Ok(MlInst::Jump { target })
            }
            "JumpIf" => {
                let condition = resolve_ml_slot(raw.condition.as_deref().unwrap_or(""))?;
                let target = raw.target.unwrap_or(0);
                Ok(MlInst::JumpIf { condition, target })
            }
            "Prefill" => {
                let input = resolve_ml_slot(raw.input.as_deref().unwrap_or(""))?;
                Ok(MlInst::Prefill { input })
            }
            "Sample" => {
                let tensor_slot = resolve_ml_slot(
                    raw.tensor_slot
                        .as_deref()
                        .or(raw.tensor_reg.as_deref())
                        .unwrap_or("TENSOR2"),
                )?;
                Ok(MlInst::Sample { tensor_slot })
            }
            "FillTensor" => {
                let dst = resolve_ml_slot(raw.dst.as_deref().unwrap_or(""))?;
                let value = raw
                    .value
                    .as_ref()
                    .and_then(|v| v.as_float())
                    .unwrap_or(1.0f64) as f32;
                let shape_slots: Vec<SlotId> = raw
                    .shape_slots
                    .as_ref()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|s| resolve_ml_slot(s))
                    .collect::<Result<_, _>>()?;
                Ok(MlInst::FillTensor {
                    dst,
                    shape_slots,
                    value,
                })
            }
            "Inference" => {
                let input_type = raw.input_type.as_deref().unwrap_or("Tokens");
                let input = resolve_ml_slot(raw.input.as_deref().unwrap_or(""))?;
                let input_type = match input_type {
                    "Tokens" => MlInputType::Tokens,
                    "Tensor" => MlInputType::Tensor,
                    _ => {
                        return Err(SelectorError::UnsupportedValueType(format!(
                            "unknown Inference input_type: {}",
                            input_type
                        )))
                    }
                };
                Ok(MlInst::Inference { input_type, input })
            }
            _ => Err(SelectorError::UnknownInstructionType(raw.inst_type.clone())),
        }
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
    fn invalid_variable_returns_error() {
        let v = vars(&[]);
        let result = ProgramSelector::select(crate::orchestrator::job::JobKind::Run, JobId(1), v);
        assert!(result.is_err());
    }

    #[test]
    fn unsupported_kind_returns_error() {
        let v = vars(&[]);
        let result =
            ProgramSelector::select(crate::orchestrator::job::JobKind::Coordinator, JobId(1), v);
        assert!(result.is_err());
    }

    // ─── load_ml_program_vm 测试 ───────────────────

    #[test]
    fn load_ml_run_vm_template() {
        let params = Pipeline_Params::default();
        let prog = ProgramSelector::load_ml_program_vm("run", &params, &HashMap::new()).unwrap();
        assert_eq!(prog.len(), 16);
        // 前两条是 Input, Encode
        assert!(matches!(&prog[0], MlInst::Input));
        assert!(matches!(&prog[1], MlInst::Encode));
        // Const META2 = $max_tokens (default 120)
        if let MlInst::Const { value, dst } = &prog[2] {
            assert_eq!(dst.0, 1041); // SLOT_META2
            assert!(matches!(value, ConstValue::F64(120.0)));
        } else {
            panic!("expected Const");
        }
        // Const FLAG1 = false
        if let MlInst::Const { value, dst } = &prog[3] {
            assert_eq!(dst.0, 1030); // SLOT_FLAG1
            assert!(matches!(value, ConstValue::Bool(false)));
        } else {
            panic!("expected Const");
        }
        // 最后是 EndOutput
        assert!(matches!(&prog[15], MlInst::EndOutput));
        // JumpIf at index 9 targets index 15
        if let MlInst::JumpIf { condition, target } = &prog[9] {
            assert_eq!(condition.0, 1030); // FLAG1
            assert_eq!(*target, 15);
        } else {
            panic!("expected JumpIf");
        }
        // Jump at index 14 targets index 9 (loop back)
        if let MlInst::Jump { target } = &prog[14] {
            assert_eq!(*target, 9);
        } else {
            panic!("expected Jump");
        }
    }

    #[test]
    fn load_ml_relay_vm_template() {
        let params = Pipeline_Params::default();
        let prog = ProgramSelector::load_ml_program_vm("relay", &params, &HashMap::new()).unwrap();
        assert_eq!(prog.len(), 7);
        // JumpIf at 0, Receive at 1, JumpIf at 2, Inference at 3, Send at 4, Jump at 5, SendEOF at 6
        if let MlInst::JumpIf { target, .. } = &prog[0] {
            assert_eq!(*target, 6);
        } else {
            panic!("expected JumpIf");
        }
        assert!(matches!(&prog[6], MlInst::SendEOF));
    }

    #[test]
    fn load_ml_coordinator_vm_template() {
        let params = Pipeline_Params::default();
        let prog =
            ProgramSelector::load_ml_program_vm("coordinator", &params, &HashMap::new()).unwrap();
        assert_eq!(prog.len(), 21);
        assert!(matches!(&prog[0], MlInst::Input));
        // JumpIf at index 11 targets index 19
        if let MlInst::JumpIf { target, .. } = &prog[11] {
            assert_eq!(*target, 19);
        } else {
            panic!("expected JumpIf");
        }
        assert!(matches!(&prog[19], MlInst::SendEOF));
        assert!(matches!(&prog[20], MlInst::EndOutput));
    }
}
