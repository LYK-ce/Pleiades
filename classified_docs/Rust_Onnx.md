#Presented by KeJi
#Date : 2026-01-14

# Rust ONNX Runtime 文档

## 1. 为什么选择ONNX Runtime

| 需求 | ONNX Runtime满足度 |
|------|-------------------|
| 性能 | ≥PyTorch |
| 模型通用性 | 任意ONNX模型 |
| 单文件分发 | 计算图+权重 |
| 层/子图切分 | 工具支持 |
| Rust集成 | ort crate |
| 静态打包 | 支持 |
| GPU自动回退 | 支持 |

## 2. 核心依赖

```toml
[dependencies]
ort = { version = "2.0", features = ["static", "download-binaries"] }
ndarray = "0.16"

# GPU支持（可选）
# ort = { version = "2.0", features = ["cuda"] }
```

## 3. ONNX格式

### 3.1 单文件自包含

```
model.onnx 包含：
├── 计算图（算子 + 连接关系）
├── 权重数据
├── 输入/输出定义（名称、形状、类型）
└── 元数据（opset版本等）

→ 无需config.json
→ 无需模型架构代码
→ 加载即推理
```

### 3.2 模型转换

```python
# PyTorch → ONNX
import torch

model = MyModel()
dummy_input = torch.randn(1, 3, 224, 224)

torch.onnx.export(
    model,
    dummy_input,
    "model.onnx",
    input_names=["input"],
    output_names=["output"],
    dynamic_axes={"input": {0: "batch"}, "output": {0: "batch"}},
    opset_version=17,
)
```

## 4. 基本使用

### 4.1 初始化与加载

```rust
use ort::{Session, GraphOptimizationLevel};

fn Init_Runtime() -> ort::Result<()> {
    ort::init()
        .with_name("Pleiades")
        .commit()?;
    Ok(())
}

fn Load_Model(model_path: &str) -> ort::Result<Session> {
    Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(model_path)
}
```

### 4.2 推理执行

```rust
use ort::{Session, Value, inputs};
use ndarray::{Array4, ArrayD};

fn Run_Inference(
    session: &Session,
    input_data: Array4<f32>,
) -> ort::Result<ArrayD<f32>> {
    let outputs = session.run(inputs!["input" => input_data.view()]?)?;
    let output = outputs["output"].try_extract_tensor::<f32>()?;
    Ok(output.to_owned())
}
```

### 4.3 完整示例

```rust
use ort::{Session, GraphOptimizationLevel, inputs};
use ndarray::Array4;

fn main() -> ort::Result<()> {
    // 初始化
    ort::init().with_name("Pleiades").commit()?;
    
    // 加载模型
    let session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file("model.onnx")?;
    
    // 准备输入
    let input = Array4::<f32>::zeros((1, 3, 224, 224));
    
    // 推理
    let outputs = session.run(inputs!["input" => input.view()]?)?;
    
    // 获取输出
    let output = outputs["output"].try_extract_tensor::<f32>()?;
    println!("Output shape: {:?}", output.shape());
    
    Ok(())
}
```

## 5. 执行后端自动选择

### 5.1 后端检测

```rust
enum Execution_Backend {
    Cuda,
    Metal,
    Cpu,
}

fn Detect_Best_Backend() -> Execution_Backend {
    // 检测CUDA
    if Is_Cuda_Available() {
        return Execution_Backend::Cuda;
    }
    
    // 检测Metal (macOS)
    #[cfg(target_os = "macos")]
    if Is_Metal_Available() {
        return Execution_Backend::Metal;
    }
    
    Execution_Backend::Cpu
}

fn Is_Cuda_Available() -> bool {
    #[cfg(target_os = "windows")]
    let result = libloading::Library::new("nvcuda.dll").is_ok();
    
    #[cfg(target_os = "linux")]
    let result = libloading::Library::new("libcuda.so").is_ok();
    
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let result = false;
    
    result
}
```

### 5.2 动态后端配置

```rust
use ort::{Session, ExecutionProviderDispatch, CUDAExecutionProvider, CPUExecutionProvider};

fn Create_Session_With_Auto_Backend(model_path: &str) -> ort::Result<Session> {
    let backend = Detect_Best_Backend();
    
    let providers: Vec<ExecutionProviderDispatch> = match backend {
        Execution_Backend::Cuda => vec![
            CUDAExecutionProvider::default().build(),
            CPUExecutionProvider::default().build(),  // 回退
        ],
        #[cfg(target_os = "macos")]
        Execution_Backend::Metal => vec![
            ort::CoreMLExecutionProvider::default().build(),
            CPUExecutionProvider::default().build(),
        ],
        _ => vec![
            CPUExecutionProvider::default().build(),
        ],
    };
    
    Session::builder()?
        .with_execution_providers(providers)?
        .commit_from_file(model_path)
}
```

## 6. 模型切分（分布式推理）

### 6.1 子图提取

ONNX支持提取模型的子图：

```python
# Python工具：提取子图
import onnx
from onnx.utils import extract_model

# 提取layer_0到layer_7
extract_model(
    "full_model.onnx",
    "part_0.onnx",
    input_names=["input"],           # 子图输入
    output_names=["layer_7_output"], # 子图输出
)

# 提取layer_8到layer_15
extract_model(
    "full_model.onnx",
    "part_1.onnx",
    input_names=["layer_7_output"],
    output_names=["layer_15_output"],
)

# 提取layer_16到输出
extract_model(
    "full_model.onnx",
    "part_2.onnx",
    input_names=["layer_15_output"],
    output_names=["output"],
)
```

### 6.2 分布式推理流程

```
节点A: part_0.onnx (input → layer_7_output)
    ↓ 传输中间结果
节点B: part_1.onnx (layer_7_output → layer_15_output)
    ↓ 传输中间结果
节点C: part_2.onnx (layer_15_output → output)
```

### 6.3 中间状态传输

```rust
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct Intermediate_State {
    data: Vec<f32>,
    shape: Vec<usize>,
    tensor_name: String,
}

impl Intermediate_State {
    fn From_Tensor(tensor: &ArrayD<f32>, name: &str) -> Self {
        Self {
            data: tensor.iter().cloned().collect(),
            shape: tensor.shape().to_vec(),
            tensor_name: name.to_string(),
        }
    }
    
    fn To_Array(&self) -> ArrayD<f32> {
        ArrayD::from_shape_vec(
            ndarray::IxDyn(&self.shape),
            self.data.clone()
        ).unwrap()
    }
    
    fn Serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
    
    fn Deserialize(data: &[u8]) -> Self {
        bincode::deserialize(data).unwrap()
    }
}
```

## 7. P2P集成架构

### 7.1 运行时接口

```rust
#[derive(Serialize, Deserialize)]
enum Runtime_Command {
    LoadModel {
        model_path: String,  // 本地路径或P2P标识
    },
    RunInference {
        request_id: String,
        input_data: Vec<u8>,  // 序列化的输入
        input_name: String,
    },
    RunPartialInference {
        request_id: String,
        intermediate: Vec<u8>,  // 序列化的Intermediate_State
    },
    UnloadModel,
}

#[derive(Serialize, Deserialize)]
enum Runtime_Response {
    ModelLoaded {
        success: bool,
        input_info: Vec<(String, Vec<i64>)>,  // (name, shape)
        output_info: Vec<(String, Vec<i64>)>,
    },
    InferenceResult {
        request_id: String,
        output_data: Vec<u8>,
        is_intermediate: bool,
    },
    Error {
        message: String,
    },
}
```

### 7.2 Runtime管理器

```rust
struct Runtime_Manager {
    session: Option<Session>,
    model_info: Option<Model_Info>,
}

struct Model_Info {
    input_names: Vec<String>,
    output_names: Vec<String>,
    is_partial: bool,  // 是否为子图
}

impl Runtime_Manager {
    fn new() -> Self {
        Self { session: None, model_info: None }
    }
    
    fn Load(&mut self, model_path: &str) -> ort::Result<()> {
        let session = Create_Session_With_Auto_Backend(model_path)?;
        
        let input_names: Vec<String> = session
            .inputs
            .iter()
            .map(|i| i.name.clone())
            .collect();
            
        let output_names: Vec<String> = session
            .outputs
            .iter()
            .map(|o| o.name.clone())
            .collect();
        
        self.model_info = Some(Model_Info {
            input_names,
            output_names,
            is_partial: false,  // 根据实际判断
        });
        self.session = Some(session);
        
        Ok(())
    }
    
    fn Infer(&self, input: ArrayD<f32>) -> ort::Result<ArrayD<f32>> {
        let session = self.session.as_ref().ok_or("No model loaded")?;
        let info = self.model_info.as_ref().unwrap();
        
        let outputs = session.run(
            inputs![&info.input_names[0] => input.view()]?
        )?;
        
        let output = outputs[&info.output_names[0]]
            .try_extract_tensor::<f32>()?;
            
        Ok(output.to_owned())
    }
}
```

## 8. 完整工作流

```
1. 协调节点分配子图给各节点
2. 各节点通过P2P下载对应子图(.onnx)
3. 各节点加载子图到Runtime
4. 推理流程：
   输入 → 节点A(part_0) → 中间状态 
        → 节点B(part_1) → 中间状态
        → 节点C(part_2) → 输出
5. 结果返回请求方
```

## 9. 静态打包

### 9.1 单文件可执行

```toml
[dependencies]
ort = { version = "2.0", features = ["static", "download-binaries"] }
```

编译后：
- CPU版本：单个exe，约100-150MB
- GPU版本：exe + CUDA运行时（用户系统需有）

### 9.2 分发策略

```rust
// 启动时检测并报告
fn Report_Environment() {
    let backend = Detect_Best_Backend();
    match backend {
        Execution_Backend::Cuda => {
            println!("[INFO] 检测到CUDA，使用GPU推理");
        }
        Execution_Backend::Metal => {
            println!("[INFO] 检测到Metal，使用Apple GPU推理");
        }
        Execution_Backend::Cpu => {
            println!("[INFO] 使用CPU推理");
        }
    }
}
```

## 10. 性能优化

### 10.1 图优化级别

```rust
// Level3 = 最大优化
Session::builder()?
    .with_optimization_level(GraphOptimizationLevel::Level3)?
```

### 10.2 并行执行

```rust
Session::builder()?
    .with_intra_threads(4)?   // 算子内并行
    .with_inter_threads(2)?   // 算子间并行
```

### 10.3 内存优化

```rust
Session::builder()?
    .with_memory_pattern(true)?     // 内存模式优化
    .with_allocator(arena_allocator)?  // 使用arena分配器
```

## 11. 依赖关系

```
T1(Rust_Libp2p.md) → 本文档(Rust_Onnx.md)
                          ↓
              P2P传输ONNX模型 + 中间状态
```

## 12. 总结

| 需求 | ONNX Runtime解决方案 |
|------|---------------------|
| 模型格式 | ONNX（单文件自包含） |
| 模型加载 | Session::commit_from_file |
| 推理执行 | session.run() |
| 模型切分 | onnx.utils.extract_model |
| GPU加速 | ExecutionProvider自动选择 |
| 分布式传输 | Intermediate_State序列化 |
| P2P集成 | Runtime_Command/Response |
| 静态打包 | features = ["static"] |
