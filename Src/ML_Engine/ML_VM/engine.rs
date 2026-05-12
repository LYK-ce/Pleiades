//Presented by KeJi
//Date ： 2026-05-11

//! ML_VM 执行引擎。
//!
//! 基于 Vm_Base::Vm 构建，负责 ML 领域指令的 step/execute。
//! 所有操作通过槽位（Vm_Base::SlotFile + MlSlots）读写数据，
//! 控制流通过 IP + Jump/JumpIf 实现。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use candle_core::Tensor;
use tracing::{error, info, warn};

use crate::llm_io::IoHandle;
use crate::ml_engine::capability::ML_Engine_Error;
use crate::ml_engine::pipeline::{Pipeline_Params, Pipeline_Result};
use crate::tensor_io::Tensor_IO_Endpoint;
use crate::vm_base::{SlotId, SlotValue, StepResult, Vm};

use super::super::gguf_model::{GGUF_Decode, GGUF_Encode, GGUF_Model, GGUF_Model_Inference};
use super::super::gguf_tensor::{
    GGUF_Dtype, GGUF_Tensor_Deserialize, GGUF_Tensor_Packet, GGUF_Tensor_Serialize,
};
use super::instruction::{InferenceInputType, MlInstruction};
use super::slots::{
    MlSlots, SLOT_FLAG1, SLOT_FLAG4, SLOT_META1, SLOT_META2, SLOT_META4, SLOT_META5, SLOT_META6,
    SLOT_TENSOR1, SLOT_TENSOR2, SLOT_TEXT1, SLOT_TEXT2, SLOT_TOKENS1, SLOT_TOKENS2, SLOT_TOKENS3,
};

// ============================================================
// ML_VM
// ============================================================

pub struct ML_VM<'a> {
    pub vm: Vm,
    pub ml_slots: MlSlots,
    backend: &'a mut GGUF_Model,
    io_handle: &'a mut IoHandle,
    tensor_io: &'a mut Option<Tensor_IO_Endpoint>,
    program: Vec<MlInstruction>,
    pub temperature: f64,
    rng_state: u64,
    eos_token_id: u32,
    inference_duration: Duration,
}

impl<'a> ML_VM<'a> {
    pub fn new(
        backend: &'a mut GGUF_Model,
        io_handle: &'a mut IoHandle,
        tensor_io: &'a mut Option<Tensor_IO_Endpoint>,
    ) -> Self {
        let eos_token_id = backend.inference_config.eos_token;
        Self {
            vm: Vm::new(),
            ml_slots: MlSlots::new(),
            backend,
            io_handle,
            tensor_io,
            program: Vec::new(),
            temperature: 0.8,
            rng_state: 299792458,
            eos_token_id,
            inference_duration: Duration::ZERO,
        }
    }

    pub fn set_temperature(&mut self, temperature: f64) {
        self.temperature = temperature;
    }

    pub fn set_eos_token_id(&mut self, eos_token_id: u32) {
        self.eos_token_id = eos_token_id;
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.rng_state = seed;
    }

    pub fn load(&mut self, program: Vec<MlInstruction>) {
        self.program = program;
        self.reset_state();
    }

    fn reset_state(&mut self) {
        self.vm = Vm::new();
        self.ml_slots.reset_all();
        self.inference_duration = Duration::ZERO;
        for i in 0..4u32 {
            self.vm.slots.set(SlotId(1030 + i), SlotValue::Bool(false));
        }
        for i in 0..8u32 {
            self.vm.slots.set(SlotId(1040 + i), SlotValue::F64(0.0));
        }
    }

    pub fn execute(
        &mut self,
        program: Vec<MlInstruction>,
        cancel_flag: &AtomicBool,
    ) -> Result<(), ML_Engine_Error> {
        self.load(program);
        loop {
            match self.step(cancel_flag) {
                StepResult::Continue | StepResult::Ready => continue,
                StepResult::Done => return Ok(()),
                StepResult::Abort(reason) => return Err(ML_Engine_Error::ProgramFailed(reason)),
            }
        }
    }

    pub fn execute_with_params(
        &mut self,
        program: Vec<MlInstruction>,
        params: &Pipeline_Params,
        cancel_flag: &AtomicBool,
    ) -> Result<Pipeline_Result, ML_Engine_Error> {
        self.temperature = params.temperature;
        self.rng_state = params.seed;
        if let Some(eos) = params.eos_token_id {
            self.eos_token_id = eos;
        }
        let start = Instant::now();

        self.execute(program, cancel_flag)?;

        // 若模板使用了 Timer/Sub 写入 META10，用它覆盖 inference_duration（排除 warmup）
        if let Ok(timer_secs) = self.vm.slots.get_f64(SlotId(1049)) {
            if timer_secs > 0.0 {
                self.inference_duration = Duration::from_secs_f64(timer_secs);
            }
        }

        let result = Pipeline_Result {
            result_text: self
                .vm
                .slots
                .get_string(SLOT_TEXT2)
                .unwrap_or(&String::new())
                .clone(),
            generated_tokens: self
                .ml_slots
                .take_token_ids(SLOT_TOKENS1)
                .unwrap_or_default(),
            prompt_tokens: self
                .ml_slots
                .take_token_ids(SLOT_TOKENS3)
                .unwrap_or_default(),
            model_info: None,
            duration: start.elapsed(),
            inference_duration: self.inference_duration,
            total_steps: self.vm.slots.get_f64(SLOT_META4).unwrap_or(0.0) as usize,
        };
        Ok(result)
    }

    // ============================================================
    // 单步执行
    // ============================================================

    pub fn step(&mut self, cancel_flag: &AtomicBool) -> StepResult {
        if cancel_flag.load(Ordering::Relaxed) {
            return StepResult::Abort("cancelled".into());
        }
        if self.vm.ip >= self.program.len() {
            return StepResult::Done;
        }
        let inst = self.program[self.vm.ip].clone();
        self.vm.ip += 1;
        match inst {
            MlInstruction::Const { value, dst } => self.vm.handle_const(value, dst),
            MlInstruction::Move { src, dst } => self.vm.handle_move(src, dst),
            MlInstruction::Add { dst, delta } => self.vm.handle_add(dst, delta),
            MlInstruction::Jump { target } => self.vm.handle_jump(target),
            MlInstruction::JumpIf { condition, target } => {
                self.vm.handle_jump_if(condition, target)
            }
            MlInstruction::Sub { src, dst } => self.vm.handle_sub(src, dst),
            MlInstruction::Timer { slot } => self.vm.handle_timer(slot),

            MlInstruction::Input => self.handle_input(),
            MlInstruction::Encode => self.handle_encode(),
            MlInstruction::Decode => self.handle_decode(),
            MlInstruction::Prefill { input } => self.handle_prefill(input),
            MlInstruction::Inference { input_type, input } => {
                let t0 = Instant::now();
                let r = self.handle_inference(input_type, input);
                self.inference_duration += t0.elapsed();
                r
            }
            MlInstruction::Sample { tensor_slot } => self.handle_sample(tensor_slot),
            MlInstruction::Output => self.handle_output(),
            MlInstruction::EndOutput => self.handle_end_output(),
            MlInstruction::Send => self.handle_send(),
            MlInstruction::Receive => self.handle_receive(),
            MlInstruction::SendEOF => self.handle_send_eof(),
            MlInstruction::FillTensor {
                dst,
                shape_slots,
                value,
            } => self.handle_fill_tensor(dst, shape_slots, value),
        }
    }

    fn handle_input(&mut self) -> StepResult {
        info!("[Input] 等待输入...");
        match self.io_handle.input_rx.blocking_recv() {
            Some(text) => {
                self.vm.slots.set(SLOT_TEXT1, SlotValue::String(text));
                StepResult::Continue
            }
            None => StepResult::Abort("Input: 输入通道已关闭".into()),
        }
    }

    fn handle_encode(&mut self) -> StepResult {
        let text = match self.vm.slots.get_string(SLOT_TEXT1) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("Encode: {}", e)),
        };
        match GGUF_Encode(self.backend, &text) {
            Ok(token_ids) => {
                let prompt_len = token_ids.len();
                self.ml_slots.set_token_ids(SLOT_TOKENS3, token_ids);
                self.ml_slots.clear_token_ids(SLOT_TOKENS1);
                self.vm.slots.set(SLOT_META1, SlotValue::F64(0.0));
                info!("[Encode] 完成 ({} tokens)", prompt_len);
                StepResult::Continue
            }
            Err(e) => {
                error!("[Encode] 失败: {}", e);
                self.set_error_flags();
                StepResult::Abort(format!("Encode: {}", e))
            }
        }
    }

    fn handle_decode(&mut self) -> StepResult {
        let token_ids = self
            .ml_slots
            .take_token_ids(SLOT_TOKENS2)
            .unwrap_or_default();
        match GGUF_Decode(self.backend, &token_ids) {
            Ok(text) => {
                self.ml_slots.set_token_ids(SLOT_TOKENS2, token_ids);
                self.vm.slots.set(SLOT_TEXT2, SlotValue::String(text));
                StepResult::Continue
            }
            Err(e) => {
                error!("[Decode] 失败: {}", e);
                self.set_error_flags();
                StepResult::Abort(format!("Decode: {}", e))
            }
        }
    }

    fn handle_prefill(&mut self, input: SlotId) -> StepResult {
        let token_ids = match self.ml_slots.get_token_ids(input) {
            Some(ids) => ids.to_vec(),
            None => return StepResult::Abort(format!("Prefill: Token 槽位 {} 为空", input.0)),
        };
        let prompt_len = token_ids.len();
        info!("[Prefill] tokens={}, offset=0", prompt_len);

        let device = self.backend.device.clone();
        let input_tensor = match Tensor::new(token_ids, &device).and_then(|t| t.unsqueeze(0)) {
            Ok(t) => t,
            Err(e) => return StepResult::Abort(format!("Prefill: Tensor 构造失败: {}", e)),
        };

        match GGUF_Model_Inference(self.backend, &input_tensor, 0) {
            Ok(output_tensor) => {
                self.ml_slots.set_tensor(SLOT_TENSOR2, output_tensor);
                self.vm
                    .slots
                    .set(SLOT_META5, SlotValue::F64(prompt_len as f64));
                info!("[Prefill] 完成, META5={}", prompt_len);
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("Prefill: 推理失败: {}", e)),
        }
    }

    fn handle_inference(&mut self, input_type: InferenceInputType, input: SlotId) -> StepResult {
        let offset = self.vm.slots.get_f64(SLOT_META1).unwrap_or(0.0) as usize;
        let device = self.backend.device.clone();

        let input_tensor = match input_type {
            InferenceInputType::Tokens => {
                let token_ids = match self.ml_slots.get_token_ids(input) {
                    Some(ids) => ids.to_vec(),
                    None => {
                        return StepResult::Abort(format!("Inference: Token 槽位 {} 为空", input.0))
                    }
                };
                match Tensor::new(token_ids, &device).and_then(|t| t.unsqueeze(0)) {
                    Ok(t) => t,
                    Err(e) => {
                        return StepResult::Abort(format!("Inference: Token→Tensor 失败: {}", e))
                    }
                }
            }
            InferenceInputType::Tensor => match self.ml_slots.get_tensor(input) {
                Some(t) => t.clone(),
                None => {
                    return StepResult::Abort(format!("Inference: Tensor 槽位 {} 为空", input.0))
                }
            },
        };

        let seq_len_before = input_tensor.dim(1).unwrap_or(1);

        match GGUF_Model_Inference(self.backend, &input_tensor, offset) {
            Ok(output_tensor) => {
                // 确保 GPU/CPU 计算完成后再存储结果
                if let Err(e) = self.backend.device.synchronize() {
                    return StepResult::Abort(format!("Inference: sync 失败: {}", e));
                }
                self.ml_slots.set_tensor(SLOT_TENSOR2, output_tensor);
                tracing::info!(
                    "[Inference] 完成, seq_len_before={}, offset={}",
                    seq_len_before,
                    offset
                );
                let increment = match input_type {
                    InferenceInputType::Tokens => 1.0,
                    InferenceInputType::Tensor => seq_len_before as f64,
                };
                self.vm.slots.set(
                    SLOT_META1,
                    SlotValue::F64(self.vm.slots.get_f64(SLOT_META1).unwrap_or(0.0) + increment),
                );
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("Inference: 推理失败: {}", e)),
        }
    }

    fn handle_sample(&mut self, tensor_slot: SlotId) -> StepResult {
        let logits_tensor = match self.ml_slots.get_tensor(tensor_slot) {
            Some(t) => t.clone(),
            None => {
                return StepResult::Abort(format!("Sample: Tensor 槽位 {} 为空", tensor_slot.0))
            }
        };

        let next_token = match self.sample_token(&logits_tensor) {
            Ok(t) => t,
            Err(e) => return StepResult::Abort(format!("Sample: 采样失败: {}", e)),
        };

        self.ml_slots.set_token_ids(SLOT_TOKENS2, vec![next_token]);
        self.ml_slots.append_token_id(SLOT_TOKENS1, next_token);
        self.vm.slots.set(
            SLOT_META2,
            SlotValue::F64(self.vm.slots.get_f64(SLOT_META2).unwrap_or(0.0) - 1.0),
        );
        self.vm.slots.set(
            SLOT_META4,
            SlotValue::F64(self.vm.slots.get_f64(SLOT_META4).unwrap_or(0.0) + 1.0),
        );

        if next_token == self.eos_token_id {
            info!("[Sample] EOS token 检测到");
            self.vm.slots.set(SLOT_FLAG1, SlotValue::Bool(true));
        }
        if self.vm.slots.get_f64(SLOT_META2).unwrap_or(0.0) <= 0.0 {
            info!("[Sample] 达到最大生成数");
            self.vm.slots.set(SLOT_FLAG1, SlotValue::Bool(true));
        }

        StepResult::Continue
    }

    fn handle_output(&mut self) -> StepResult {
        let text = self
            .vm
            .slots
            .get_string(SLOT_TEXT2)
            .unwrap_or(&String::new())
            .clone();
        if let Err(e) = self.io_handle.output_tx.blocking_send(text) {
            warn!("[Output] 发送失败: {}", e);
            self.set_error_flags();
            return StepResult::Abort(format!("Output: {}", e));
        }
        StepResult::Continue
    }

    fn handle_end_output(&self) -> StepResult {
        info!("[EndOutput] 输出结束");
        StepResult::Continue
    }

    fn handle_send(&mut self) -> StepResult {
        let tensor = match self.ml_slots.get_tensor(SLOT_TENSOR2) {
            Some(t) => t.clone(),
            None => return StepResult::Abort("Send: TENSOR2 为空".into()),
        };
        let bytes = match self.tensor_to_bytes(&tensor) {
            Ok(b) => b,
            Err(e) => return StepResult::Abort(format!("Send: 序列化失败: {}", e)),
        };
        let offset = self.vm.slots.get_f64(SLOT_META1).unwrap_or(0.0) as u64;
        tracing::info!("[Send] 开始发送, offset={}, bytes={}", offset, bytes.len());
        match self.tensor_io.as_mut() {
            Some(tio) => {
                if let Err(e) = tio.Send(offset, &bytes) {
                    return StepResult::Abort(format!("Send: 发送失败: {}", e));
                }
                StepResult::Continue
            }
            None => StepResult::Abort("Send: tensor_io 不可用".into()),
        }
    }

    fn handle_receive(&mut self) -> StepResult {
        tracing::info!("[Receive] 开始接收...");
        match self.tensor_io.as_mut() {
            Some(tio) => match tio.Receive() {
                Ok(offset) => {
                    if offset == u64::MAX {
                        info!("[Receive] 收到 EOF");
                        self.vm.slots.set(SLOT_FLAG1, SlotValue::Bool(true));
                    } else {
                        let buffer = tio.Get_Buffer().to_vec();
                        match self.bytes_to_tensor(&buffer) {
                            Ok(tensor) => {
                                self.ml_slots.set_tensor(SLOT_TENSOR1, tensor);
                                self.vm.slots.set(SLOT_META6, SlotValue::F64(offset as f64));
                            }
                            Err(e) => {
                                return StepResult::Abort(format!("Receive: 反序列化失败: {}", e))
                            }
                        }
                    }
                    StepResult::Continue
                }
                Err(e) => StepResult::Abort(format!("Receive: 接收失败: {}", e)),
            },
            None => StepResult::Abort("Receive: tensor_io 不可用".into()),
        }
    }

    fn handle_send_eof(&mut self) -> StepResult {
        info!("[SendEOF] 发送 EOF");
        match self.tensor_io.as_mut() {
            Some(tio) => {
                if let Err(e) = tio.Send_EOF() {
                    return StepResult::Abort(format!("SendEOF: 发送失败: {}", e));
                }
                StepResult::Continue
            }
            None => StepResult::Abort("SendEOF: tensor_io 不可用".into()),
        }
    }

    fn handle_fill_tensor(
        &mut self,
        dst: SlotId,
        shape_slots: Vec<SlotId>,
        value: f32,
    ) -> StepResult {
        let shape: Vec<usize> = shape_slots
            .iter()
            .map(|s| self.vm.slots.get_f64(*s).unwrap_or(1.0) as usize)
            .collect();
        let device = self.backend.device.clone();
        let tensor = match Tensor::full(value, &shape[..], &device) {
            Ok(t) => t,
            Err(e) => return StepResult::Abort(format!("FillTensor: 创建失败: {}", e)),
        };
        self.ml_slots.set_tensor(dst, tensor);
        StepResult::Continue
    }

    // ============================================================
    // 辅助方法
    // ============================================================

    fn set_error_flags(&mut self) {
        self.vm.slots.set(SLOT_FLAG4, SlotValue::Bool(true));
        self.vm.slots.set(SLOT_FLAG1, SlotValue::Bool(true));
    }

    fn sample_token(&mut self, logits: &Tensor) -> Result<u32> {
        let logits = Self::extract_last_logits(logits)?;

        if self.temperature <= 0.0 {
            let token = logits
                .argmax(0)
                .map_err(|e| anyhow::anyhow!("argmax 失败: {}", e))?
                .to_scalar::<u32>()
                .map_err(|e| anyhow::anyhow!("to_scalar 失败: {}", e))?;
            Ok(token)
        } else {
            let scaled =
                (&logits / self.temperature).map_err(|e| anyhow::anyhow!("温度缩放失败: {}", e))?;
            let probs = candle_nn::ops::softmax(&scaled, 0)
                .map_err(|e| anyhow::anyhow!("softmax 失败: {}", e))?;
            let probs_vec: Vec<f32> = probs
                .to_vec1::<f32>()
                .map_err(|e| anyhow::anyhow!("probs → vec 失败: {}", e))?;

            self.rng_state = self
                .rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let random = (self.rng_state >> 33) as f32 / (u32::MAX as f32);

            let mut cumulative = 0.0f32;
            for (i, &p) in probs_vec.iter().enumerate() {
                cumulative += p;
                if cumulative > random {
                    return Ok(i as u32);
                }
            }
            Ok((probs_vec.len() - 1) as u32)
        }
    }

    fn extract_last_logits(logits: &Tensor) -> Result<Tensor> {
        let dims = logits.dims();
        match dims.len() {
            3 => {
                let seq_len = dims[1];
                let last = logits
                    .get(0)
                    .map_err(|e| anyhow::anyhow!("get batch 0 失败: {}", e))?
                    .get(seq_len - 1)
                    .map_err(|e| anyhow::anyhow!("get last position 失败: {}", e))?;
                Ok(last)
            }
            2 => logits
                .squeeze(0)
                .map_err(|e| anyhow::anyhow!("squeeze 失败: {}", e)),
            1 => Ok(logits.clone()),
            _ => Err(anyhow::anyhow!("不支持的 logits shape: {:?}", dims)),
        }
    }

    fn tensor_to_bytes(&self, tensor: &Tensor) -> Result<Vec<u8>> {
        let shape = tensor.dims().to_vec();
        let f32_data: Vec<f32> = tensor
            .flatten_all()?
            .to_vec1::<f32>()
            .map_err(|e| anyhow::anyhow!("Tensor → f32 失败: {}", e))?;
        let raw_bytes: Vec<u8> = f32_data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let packet = GGUF_Tensor_Packet::New(
            "hidden_state".to_string(),
            shape,
            GGUF_Dtype::F32,
            raw_bytes,
        );
        GGUF_Tensor_Serialize(&packet).map_err(|e| anyhow::anyhow!("张量序列化失败: {}", e))
    }

    fn bytes_to_tensor(&self, bytes: &[u8]) -> Result<Tensor> {
        let packet = GGUF_Tensor_Deserialize(bytes)
            .map_err(|e| anyhow::anyhow!("张量反序列化失败: {}", e))?;
        let f32_data: Vec<f32> = packet
            .data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        let device = self.backend.device.clone();
        let tensor = Tensor::new(&f32_data[..], &device)?.reshape(&*packet.shape)?;
        Ok(tensor)
    }
}
