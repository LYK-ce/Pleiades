//Presented by KeJi
//Date: 2026-03-21

use std::collections::HashMap;
use std::path::Path;

use ort::{
    session::{Session, builder::SessionBuilder},
    ep::*,
    value::{Tensor, DynValue},
};
use ndarray::ArrayD;
use serde::Deserialize;
use super::{RuntimeError, TensorPacket, DataType};
use tracing::info;


pub struct Runtime {
    session: Option<Session>,
    model_path: Option<String>,
}

// 配置运行环境
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub intra_threads: usize,
    pub inter_threads: usize,
    pub enable_profiling: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeInitConfig {
    pub device: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            intra_threads: num_cpus::get(),
            inter_threads: 1,
            enable_profiling: false,
        }
    }
}

/// 模型信息
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub path: String,
}

impl Runtime {
    /// 初始化 Runtime 实例
    pub fn new(_config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Ok(Runtime {
            session: None,
            model_path: None,
        })
    }

    /// 加载模型
    ///
    /// 注意：execution provider 应在 Init_ONNX 中全局配置
    pub fn load_model<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<ModelInfo, RuntimeError> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        
        // 创建session，使用全局配置的 execution providers
        let session = Session::builder()
            .map_err(|e| RuntimeError::ModelLoadFailed(e.to_string()))?
            .commit_from_file(path)
            .map_err(|e| RuntimeError::ModelLoadFailed(format!("加载 {} 失败: {}", path_str, e)))?;

        self.session = Some(session);
        self.model_path = Some(path_str.clone());
        
        info!("模型加载成功: {}", path_str);
        
        Ok(ModelInfo { path: path_str })
    }

    /// 执行推理 - 使用 ort 原生 Tensor
    /// 
    /// # Arguments
    /// * `inputs` - 输入张量映射，键为输入名，值为 ort::DynValue
    /// 
    /// # Returns
    /// * `HashMap<String, DynValue>` - 输出张量映射
    pub fn execute(
        &mut self,
        inputs: HashMap<String, DynValue>,
    ) -> Result<HashMap<String, DynValue>, RuntimeError> {
        
        let session = self.session.as_mut()
            .ok_or(RuntimeError::NoModelLoaded)?;

        // 执行推理
        let outputs = session.run(inputs)
            .map_err(|e| RuntimeError::InferenceFailed(e.to_string()))?;

        // 将 SessionOutputs 转换为 HashMap
        let mut result = HashMap::new();
        for (key, value) in outputs {
            result.insert(key.to_string(), value);
        }

        Ok(result)
    }

    /// 从 TensorPacket 执行推理
    /// 
    /// 这是提供给上层使用的便捷方法，自动处理 TensorPacket 和 ort::DynValue 之间的转换
    pub fn execute_from_packets(
        &mut self,
        packets: HashMap<String, TensorPacket>,
    ) -> Result<HashMap<String, TensorPacket>, RuntimeError> {
        // 将 TensorPacket 转换为 ort::DynValue
        let mut ort_inputs: HashMap<String, DynValue> = HashMap::new();

        for (name, packet) in packets {
            let tensor = packet_to_ort_tensor(&packet)?;
            ort_inputs.insert(name, tensor.into_dyn());
        }

        // 执行推理
        let ort_outputs = self.execute(ort_inputs)?;

        // 将 ort::DynValue 转换回 TensorPacket
        let mut result = HashMap::new();
        for (name, dyn_value) in ort_outputs {
            let packet = ort_value_to_packet(&name, dyn_value)?;
            result.insert(name, packet);
        }

        Ok(result)
    }

    /// 获取模型路径
    pub fn get_model_path(&self) -> Option<&String> {
        self.model_path.as_ref()
    }

    /// 卸载模型
    pub fn unload_model(&mut self) {
        if self.session.is_some() {
            info!("卸载模型: {}", self.model_path.as_ref().unwrap_or(&"unknown".to_string()));
            self.session = None;
            self.model_path = None;
        }
    }
}

/// 将 TensorPacket 转换为 ort::Tensor<f32>
fn packet_to_ort_tensor(packet: &TensorPacket) -> Result<Tensor<f32>, RuntimeError> {
    // 目前只支持 Float32，后续可以扩展
    if packet.dtype != DataType::Float32 {
        return Err(RuntimeError::TypeMismatch {
            expected: "Float32".to_string(),
            got: format!("{:?}", packet.dtype),
        });
    }

    let data = packet.to_f32_vec()
        .map_err(|e| RuntimeError::ConversionFailed(e.to_string()))?;

    // ort 2.0: 使用 (shape, data) 元组创建 Tensor
    let shape: Vec<i64> = packet.shape.iter().map(|&s| s as i64).collect();
    Tensor::from_array((shape, data))
        .map_err(|e| RuntimeError::ConversionFailed(e.to_string()))
}

/// 将 ort::DynValue 转换为 TensorPacket
fn ort_value_to_packet(name: &str, value: DynValue) -> Result<TensorPacket, RuntimeError> {
    // 使用 try_extract_tensor 从 DynValue 直接提取 f32 张量
    // 返回 (&Shape, &[f32])
    let result = value.try_extract_tensor::<f32>();
    
    match result {
        Ok((shape_ref, data_slice)) => {
            let shape: Vec<usize> = shape_ref.iter().map(|&s| s as usize).collect();
            let data: Vec<f32> = data_slice.to_vec();
            TensorPacket::from_f32_slice(name.to_string(), shape, &data)
                .map_err(|e| RuntimeError::ConversionFailed(e.to_string()))
        }
        Err(e) => {
            Err(RuntimeError::ConversionFailed(format!("提取张量失败: {}", e)))
        }
    }
}

/// 初始化 ONNX Runtime
pub fn Init_ONNX(config: RuntimeInitConfig) -> ort::Result<()> {
    let providers = match config.device.to_ascii_lowercase().as_str() {
        "cpu" => vec![CPUExecutionProvider::default().build()],
        "cuda" => vec![
            CUDAExecutionProvider::default().build(),
            CPUExecutionProvider::default().build(),
        ],
        "tensorrt" => vec![
            TensorRTExecutionProvider::default().build(),
            CPUExecutionProvider::default().build(),
        ],
        "coreml" | "metal" => vec![
            CoreMLExecutionProvider::default().build(),
            CPUExecutionProvider::default().build(),
        ],
        "directml" => vec![
            DirectMLExecutionProvider::default().build(),
            CPUExecutionProvider::default().build(),
        ],
        "openvino" => vec![
            OpenVINOExecutionProvider::default().build(),
            CPUExecutionProvider::default().build(),
        ],
        _ => vec![CPUExecutionProvider::default().build()],
    };

    ort::init()
        .with_execution_providers(providers)
        .commit();

    Ok(())
}
