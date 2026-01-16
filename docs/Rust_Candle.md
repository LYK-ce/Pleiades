#Presented by KeJi
#Date : 2026-01-14

# Rust Candle Runtime 文档

## 1. Candle 简介

Candle是Hugging Face开发的Rust深度学习框架，轻量、无Python依赖，适合边缘部署和P2P场景。

**核心特性：**
- 纯Rust实现，零Python依赖
- 支持CPU/CUDA/Metal后端
- 原生支持safetensors格式
- 内存占用小，启动快

## 2. 核心依赖

```toml
[dependencies]
candle-core = "0.8"        # 张量操作
candle-nn = "0.8"          # 神经网络层
candle-transformers = "0.8" # 预训练模型实现
safetensors = "0.4"        # 模型格式
tokenizers = "0.20"        # 分词器
```

## 3. 基本使用

### 3.1 设备选择

```rust
use candle_core::{Device, Result};

fn Get_Device() -> Result<Device> {
    #[cfg(feature = "cuda")]
    { Ok(Device::new_cuda(0)?) }
    #[cfg(feature = "metal")]
    { Ok(Device::new_metal(0)?) }
    #[cfg(not(any(feature = "cuda", feature = "metal")))]
    { Ok(Device::Cpu) }
}
```

### 3.2 模型加载

```rust
use candle_core::{DType, Tensor};
use candle_nn::VarBuilder;
use safetensors::SafeTensors;
use std::fs;

fn Load_Model(model_path: &str, device: &Device) -> Result<VarBuilder> {
    let model_data = fs::read(model_path)?;
    let tensors = SafeTensors::deserialize(&model_data)?;
    let var_builder = VarBuilder::from_safetensors(vec![tensors], DType::F16, device);
    Ok(var_builder)
}
```

### 3.3 推理执行

```rust
use candle_transformers::models::llama::{Llama, Config};

fn Run_Inference(
    model: &Llama,
    input_ids: &Tensor,
    seq_len: usize,
) -> Result<Tensor> {
    let logits = model.forward(input_ids, seq_len)?;
    Ok(logits)
}
```

## 4. 模型格式

### 4.1 推荐格式：SafeTensors

| 格式 | 优点 | 缺点 |
|------|------|------|
| SafeTensors | 安全/快速/Rust原生支持 | 需转换 |
| GGUF | 量化支持好 | 需额外处理 |
| PyTorch (.pt) | 通用 | 需pickle/不安全 |

**推荐：SafeTensors**
- 内存映射加载，零拷贝
- 无代码执行风险
- Candle原生支持

### 4.2 格式转换

```python
# Python脚本：PyTorch转SafeTensors
from safetensors.torch import save_file
import torch

model = torch.load("model.pt")
save_file(model, "model.safetensors")
```

## 5. 模型切分（分布式推理）

### 5.1 切分策略

**层间切分（Pipeline Parallelism）：**
- 按Transformer层切分
- 每个节点运行连续几层
- 适合P2P场景

```
节点A: Embedding + Layer 0-7
节点B: Layer 8-15
节点C: Layer 16-23 + LM_Head
```

### 5.2 切分实现

```rust
/// 层范围配置
struct Layer_Config {
    start_layer: usize,
    end_layer: usize,
    has_embedding: bool,
    has_lm_head: bool,
}

/// 加载指定层范围的权重
fn Load_Partial_Model(
    safetensors_path: &str,
    config: &Layer_Config,
    device: &Device,
) -> Result<HashMap<String, Tensor>> {
    let data = fs::read(safetensors_path)?;
    let tensors = SafeTensors::deserialize(&data)?;
    
    let mut partial_weights = HashMap::new();
    
    for (name, tensor) in tensors.tensors() {
        let should_load = Match_Layer_Range(&name, config);
        if should_load {
            let tensor = tensor.load(device)?;
            partial_weights.insert(name.to_string(), tensor);
        }
    }
    
    Ok(partial_weights)
}

fn Match_Layer_Range(tensor_name: &str, config: &Layer_Config) -> bool {
    // embedding层
    if tensor_name.contains("embed") {
        return config.has_embedding;
    }
    // lm_head层
    if tensor_name.contains("lm_head") {
        return config.has_lm_head;
    }
    // transformer层：解析层号
    if let Some(layer_num) = Extract_Layer_Number(tensor_name) {
        return layer_num >= config.start_layer && layer_num < config.end_layer;
    }
    false
}

fn Extract_Layer_Number(name: &str) -> Option<usize> {
    // 匹配 "layers.N." 或 "h.N." 模式
    let re = regex::Regex::new(r"(?:layers|h)\.(\d+)\.").ok()?;
    re.captures(name)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}
```

### 5.3 分布式推理流程

```rust
/// 中间状态（跨节点传输）
struct Intermediate_State {
    hidden_states: Vec<f16>,  // 序列化的hidden states
    shape: Vec<usize>,        // 张量形状
    seq_len: usize,
}

impl Intermediate_State {
    fn Serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
    
    fn Deserialize(data: &[u8]) -> Self {
        bincode::deserialize(data).unwrap()
    }
    
    fn To_Tensor(&self, device: &Device) -> Result<Tensor> {
        Tensor::from_vec(self.hidden_states.clone(), &self.shape, device)
    }
}

/// 单节点前向传播
fn Forward_Partial(
    model_layers: &[TransformerLayer],
    input: Tensor,
    config: &Layer_Config,
) -> Result<Tensor> {
    let mut hidden = input;
    
    for layer in model_layers {
        hidden = layer.forward(&hidden)?;
    }
    
    Ok(hidden)
}
```

## 6. P2P集成架构

### 6.1 运行时接口

```rust
/// Runtime命令（通过P2P接收）
enum Runtime_Command {
    LoadModel {
        model_path: String,
        layer_config: Layer_Config,
    },
    RunInference {
        input_data: Vec<u8>,  // 序列化的输入或中间状态
        request_id: String,
    },
    UnloadModel,
}

/// Runtime响应（通过P2P发送）
enum Runtime_Response {
    ModelLoaded { success: bool },
    InferenceResult {
        request_id: String,
        output_data: Vec<u8>,  // 序列化的输出或中间状态
    },
    Error { message: String },
}
```

### 6.2 完整工作流

```
1. 协调节点分配层范围给各节点
2. 各节点通过P2P下载对应权重分片
3. 各节点加载权重到Runtime
4. 推理请求流程：
   输入 → 节点A(embed+layer0-7) → 中间状态 
        → 节点B(layer8-15) → 中间状态
        → 节点C(layer16-23+head) → 输出
5. 结果返回请求方
```

## 7. 性能优化

### 7.1 量化支持

```rust
// 使用量化权重减少传输和内存
let var_builder = VarBuilder::from_safetensors(
    tensors,
    DType::BF16,  // 或 DType::F16
    device
);
```

### 7.2 KV Cache

```rust
struct Kv_Cache {
    k_cache: Vec<Tensor>,
    v_cache: Vec<Tensor>,
    seq_len: usize,
}

impl Kv_Cache {
    fn Update(&mut self, layer_idx: usize, k: Tensor, v: Tensor) {
        self.k_cache[layer_idx] = Tensor::cat(&[&self.k_cache[layer_idx], &k], 1)?;
        self.v_cache[layer_idx] = Tensor::cat(&[&self.v_cache[layer_idx], &v], 1)?;
    }
}
```

## 8. 依赖关系

```
T1(Rust_Libp2p.md) → 本文档(Rust_Candle.md)
                          ↓
              P2P传输模型权重 + 中间状态
```

## 9. 总结

| 需求 | Candle解决方案 |
|------|---------------|
| 模型加载 | VarBuilder + SafeTensors |
| 推理执行 | candle-transformers模型 |
| 模型格式 | SafeTensors（推荐） |
| 模型切分 | 按层范围加载权重 |
| 分布式传输 | 序列化Intermediate_State |
| P2P集成 | Runtime_Command/Response |
