# 设计文档

这是一个MVP阶段的运行时框架，主要解决的问题是，为边缘设备集群实现一个统一的运行时框架，使得设备可以：
1. 设备之间自动发现彼此
2. 无需中央服务器，自动协商任务分配
3. 动态适应设备加入/退出
4. 每个设备运行相同代码，角色由运行时动态决定
5. 决策只依赖本地视图，无全局锁



##

## 环境
Rust
candle

## 整体架构设计
Pleiades/
├── Cargo.toml                 # 项目配置、依赖
├── Cargo.lock                 # 依赖锁定 (自动生成)
├── README.md                  # 项目说明
│
├── src/                       # 源代码
│   ├── main.rs                # 程序入口
│   ├── lib.rs                 # 库导出，将子模块导出，便于单元测试
│   ├── config.rs              # 常量、配置结构
│   ├── state.rs               # 全局状态定义 全局共享数据
│   ├── error.rs               # 错误类型定义
│   │
│   ├── Network/               # P2P 网络模块
│   │   ├── mod.rs             # 模块入口，导出公共接口
│   │   ├── node.rs            # Swarm管理、连接管理、事件处理
│   │   └── protocol.rs        # 消息结构定义（命令/文件传输协议）
│   │
│   ├── Runtime/               # 推理运行时模块
│   │   ├── mod.rs             # 模块入口
│   │   ├── model.rs           # 模型加载
│   │   ├── inference.rs       # 前向计算
│   │   ├── cache.rs           # KV Cache
│   │   └── tensor.rs          # 张量序列化
│   │
│   └── Control/               # 控制层模块
│       ├── mod.rs             # 模块入口
│       ├── scheduler.rs       # 调度器
│       ├── router.rs          # 路径选择
│       └── heartbeat.rs       # 心跳管理
│
├── tests/                     # 集成测试
│   └── integration_test.rs
│
├── benches/                   # 性能测试
│   └── inference_bench.rs
│
├── examples/                  # 示例代码
│   └── simple_node.rs
│
├── docs/                      # 文档 (非Rust)
│   ├── P2PLLM设计探究.md
│   └── ...
│
└── models/                    # 模型文件 (非Rust)
    └── .gitkeep


### 应用层

### 调度层

### 网络层

#### mod.rs
负责注册子文件，并控制对外暴露哪些内容
1. 声明子模块存在
2. 控制可见性，导出公共接口




#### Node.rs
网络层核心管理器，负责swarm管理，连接管理，事件处理以及提供对外接口。

##### 方法
Init(config)
输入： config，由main.rs解析得到的config，具体见Config/config.toml
功能： 
1. 生成节点身份，临时生成（当前阶段先临时生成，之后可能改成持久身份，因此无需配置文件输入）
2. 调用libp2p函数，进行初始化，配置传输层，启动多路复用，与加密，然后初始化Behavior
3. 创建Swarm
4. 初始化DHT表，根据WAN模式或者LAN模式决定从哪里发现节点

Start()
功能：
1. 启动监听
2. 进入事件循环，不断处理网络事件，主要处理以下事件：
    - 发现节点，加入DHT
    - 节点离开
    - 命令/文件 请求事件
    - DHT查询，路由更新等
    - 连接事件
    - 断开连接

Stop()
功能：
 停止网络，清理资源

Send_Command(peer, cmd)
输入
- peer 对方节点ID
- cmd  具体命令
功能：发送命令给指定节点

Send_File(Peer, path)
- peer 对方节点ID
- path 要发送的文件路径
向目标节点发送此路径下的文件

Send_Tensor(peer, tensor, request_id)
- peer 对方节点ID
- tensor 要发送的tensor
- request_id 请求id号，避免出现问题

Put_Record(key, value)
- key 要写入DHT的键
- value 要写入DHT的value
DHT写入

Get_Record(key)
- key 要读取的键
DHT读取

Get_Peers()
获取当前已连接的所有节点列表

Get_Peer_Info(peer_id)
输入
- peer_id 特定节点id
功能：获取特定节点的详细信息(延迟，带宽，负载)(mvp阶段只需要返回延迟信息)

Dial(addr)
- addr 
主动连接到指定地址的节点，根据WAN模式和LAN模式采取不同方式


Disconnect(peer)
- peer 对方节点ID
与peer节点主动断开连接

#### Protocol.rs

##### 消息结构定义

// ===== 命令协议 =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandRequest {
    Ping,                           // 心跳检测
    GetStatus,                      // 获取节点状态
    Custom { cmd: String, args: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandResponse {
    Pong { latency_ms: u64 },
    Status { load: f32, memory_free: u64 },
    Result { success: bool, data: String },
    Error { msg: String },
}

// ===== 文件传输协议 =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileRequest {
    Info { file_id: String },
    Chunk { file_id: String, offset: u64, length: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileResponse {
    Info { file_id: String, size: u64, hash: Vec<u8> },
    Chunk { offset: u64, data: Vec<u8> },
    Error { msg: String },
}

// ===== 张量传输协议（推理核心）=====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorRequest {
    pub request_id: String,         // 请求追踪ID
    pub tensor_data: Vec<u8>,       // 序列化后的张量
    pub shape: Vec<usize>,          // 张量形状
    pub dtype: TensorDtype,         // 数据类型
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TensorResponse {
    Ack { request_id: String },
    Error { request_id: String, msg: String },
}


##### codec编解码器
use libp2p::request_response::Codec;
use libp2p::StreamProtocol;

#[derive(Debug, Clone)]
pub struct PleiadesCodec;

impl Codec for PleiadesCodec {
    type Protocol = StreamProtocol;
    type Request = PleiadesRequest;   // 统一请求类型
    type Response = PleiadesResponse; // 统一响应类型
    
    // 实现 read_request, read_response, write_request, write_response
    // 使用 bincode进行序列化
}


##### 协议标识符
pub const COMMAND_PROTOCOL: &str = "/pleiades/cmd/1.0.0";
pub const FILE_PROTOCOL: &str = "/pleiades/file/1.0.0";
pub const TENSOR_PROTOCOL: &str = "/pleiades/tensor/1.0.0";



### 运行时层
基于Onnx runtime，为模型运行提供环境。
位于 Src/Runtime 目录下

包含以下内容
#### mod.rs
模块入口，负责声明子模块并导出公共类型：
- `Runtime` - 运行时主结构
- `RuntimeConfig` - 运行时配置
- `ExecutionBackend` - 执行后端枚举
- `RuntimeError` - 错误类型
- `IntermediateState` - 中间状态（用于分布式推理）
- `ModelInfo` - 模型信息

#### runtime.rs
运行时层具体实现

##### 数据类型
| Rust类型 | 说明 |
|----------|------|
| `RuntimeConfig` | 运行时配置结构 |
| `ExecutionBackend` | 执行后端枚举（Cpu/Cuda/Metal） |
| `RuntimeError` | 错误类型枚举 |
| `Runtime` | 运行时主结构，封装ONNX Session |
| `ModelInfo` | 模型元信息（输入输出名称、形状） |
| `IntermediateState` | 分布式推理中间状态 |

##### 方法
Init(config)
初始化方法，初始化Onnx runtime环境，读取config配置文件，确定后端采用什么，cpu或者gpu。并且检查是否有对应环境，没有对应环境的话就回退到cpu

Load_Model(path)
从对应路径加载模型，返回 `ModelInfo` 包含输入输出名称信息

Execute(input_name, input, output_name)
给定输入名称和数据，指定输出名称，运行模型并返回结果

Get_Model_Info()
获取已加载模型的元信息

Unload_Model()
卸载当前模型，释放资源

##### 张量数据类型支持
**当前限制**：仅支持 `f32` (float32) 类型

| 数据类型 | Rust 类型 | 支持状态 | 用途 |
|----------|-----------|----------|------|
| float32 | `f32` | ✅ 已支持 | 标准推理 |
| float16 | `half::f16` | ❌ 待扩展 | GPU 加速，省内存 |
| bfloat16 | `half::bf16` | ❌ 待扩展 | LLM 训练常用 |
| float64 | `f64` | ❌ 待扩展 | 高精度计算 |
| int8 | `i8` | ❌ 待扩展 | 量化模型 |
| uint8 | `u8` | ❌ 待扩展 | 量化模型 |
| int32 | `i32` | ❌ 待扩展 | token IDs |
| int64 | `i64` | ❌ 待扩展 | token IDs |
| bool | `bool` | ❌ 待扩展 | attention mask |

**选择 f32 作为初版的原因**：
1. 最通用 - 大多数预训练模型导出为 f32
2. 简单 - 不需要处理类型转换
3. 够用 - 对于 MVP 阶段目标足够

**扩展方案**（如需支持多类型）：
```rust
pub enum TensorData {
    F32(ArrayD<f32>),
    F16(ArrayD<f16>),
    I64(ArrayD<i64>),
    // ...
}
```

##### 执行后端
| 后端 | ExecutionProvider | 平台 | 检测方式 |
|------|-------------------|------|----------|
| CPU | `CPUExecutionProvider` | 全平台 | 默认可用 |
| CUDA | `CUDAExecutionProvider` | Windows/Linux | 检查 nvcuda.dll / libcuda.so |
| Metal | `CoreMLExecutionProvider` | macOS | 系统原生支持 |

后端选择逻辑：优先使用配置指定的后端，若不可用则自动回退到 CPU

##### 后续尝试方案：TensorBuffer原始字节输入（待验证）
为兼容多输入、多数据类型与P2P直传，后续可尝试在运行时层增加“原始字节张量通道”，核心目标是减少中间转换与复制开销。

**设计目标**：
1. 保持统一 `Execute` 入口，不为模型类型分裂接口
2. 输入支持 `bytes + dtype + shape + name` 描述
3. 支持多输入/多输出（如 `input_ids/attention_mask/token_type_ids`）
4. 与现有 `ArrayD<f32>` 路径并存，逐步迁移

**建议结构（草案）**：
```rust
pub enum TensorDtype {
    F16,
    F32,
    I64,
    I32,
    Bool,
}

pub struct TensorBuffer {
    pub name: String,
    pub shape: Vec<usize>,
    pub dtype: TensorDtype,
    pub bytes: Vec<u8>,
}
```

**执行流程（草案）**：
1. 接收多个 `TensorBuffer`
2. 校验 `shape` 与 `bytes` 长度是否匹配（`元素数 × dtype字节宽度`）
3. 在运行时边界将缓冲解释为对应类型张量
4. 调用一次 `Session::run` 完成多输入推理
5. 输出按同样方式可返回为结构化张量或原始字节

**风险与注意事项**：
- 需严格处理字节序与对齐
- 需保证推理期间底层缓冲生命周期有效
- 错误输入会导致运行时错误，需要强校验与清晰报错
- 该方案先作为实验路径，不影响当前MVP默认f32方案



### 日志组件
直接使用rust的tracing日志系统，在main.rs中进行初始化。

Pleiades采用Rust生态中的`tracing`日志框架，而非自行实现日志系统。选择`tracing`的原因：
1. **异步追踪支持**：原生支持async/await的span追踪，适合P2P异步场景
2. **libp2p兼容**：libp2p内部已使用tracing，日志格式统一
3. **结构化日志**：支持字段化日志输出，便于分析和检索

#### 依赖配置
```toml
# Cargo.toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

#### 属性（模块级配置）
- `EnvFilter`：通过`RUST_LOG`环境变量控制日志级别，支持模块级精细控制
- `fmt::Layer`：日志输出层，支持控制台pretty格式或JSON格式（生产环境）
- `Registry`：日志订阅器注册中心，组合多个Layer

#### 方法

**pub fn init()**
初始化日志系统，应在程序启动时调用一次
```rust
pub fn init() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("pleiades=info,libp2p=warn"))
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();
}
```

**日志宏（全局可用）**
| 宏 | 级别 | 用途 |
|---|---|---|
| `error!()` | ERROR | 严重错误，需立即处理 |
| `warn!()` | WARN | 警告，可能影响功能 |
| `info!()` | INFO | 重要状态变化（默认输出） |
| `debug!()` | DEBUG | 调试信息 |
| `trace!()` | TRACE | 详细追踪 |

**结构化日志示例**
```rust
use tracing::{info, warn, error, instrument};

// 基本日志
info!("节点启动");
warn!(peer_id = %peer, "连接断开");
error!(?err, "推理失败");

// 异步函数追踪
#[instrument(skip(swarm), fields(peer_id = %peer_id))]
async fn handle_request(swarm: &mut Swarm<_>, peer_id: PeerId, req: Request) {
    info!("处理请求");
    // span自动关联所有内部日志
}
```

**运行时级别控制**
```bash
# 通过环境变量控制
RUST_LOG=pleiades=debug,libp2p_kad=warn,ort=error cargo run
```

#### 日志级别规范
| 级别 | 场景 |
|---|---|
| ERROR | 系统错误、推理失败、网络异常 |
| WARN | 节点断开、超时重试、资源告警 |
| INFO | 节点加入/退出、模型加载、任务完成 |
| DEBUG | 消息收发详情、调度决策 |
| TRACE | 张量数据、协议细节 |

### 配置组件
文件名称 config.toml
位于 src/Config/目录下

包含以下内容
#### Log
level
log_file_path


### 测试组件
主入口
test.rs
其他文件
network_test.rs
runtime_test.rs
scheduler_test.rs
目前只需要实现test.rs，network_test.rs和runtime_test.rs文件

测试组件位于/test目录下

#### 文件
test.rs
测试的总入口，可以通过命令行的方式选择进行某一项测试
- all 测试所有
- network 测试网络
- runtime 测试运行时
- scheduler 测试调度器

##### 网络层测试
network_test.rs
网络层测试，根据配置文件进行选择，如果是LAN那么就进行局域网测试，如果是WAN就进行广域网测试。（目前我们主要把精力集中在局域网上面）
我们当前仅做部分的测试内容，具体测试方案如下：
1. 现在Pleiades_Workspace当中创建一个自己的工作目录，名称为“本机ID+Workspace”，接下来涉及文件的操作都在此进行
2. 启动监听网络后，等待另一台测试机接入网络。
3. 接入网络后，输出对方测试机的id
4. 在工作目录中创建一个名为"本机id+hello.txt"的文件，内容为"本机ID+hello",发送给对方。
5. 接收对方发送过来的文件。

我们将会先使用单机多实例方案进行测试，然后进行局域网内多台计算机的测试。

##### 运行时层测试
我们将会使用python来生成测试使用的onnx模型

包含以下文件
tests/test_models/generate_test_onnx_model.py
tests/test_models/input_and_output.npz

此脚本执行以下事项：
1. 生成以下模型并且随机初始化：3层MLP，ResNet-18，Deit-Tiny模型，并将其保存成MLP.onnx,ResNet-18.onnx和Deit-Tiny.onnx于tests/test_models/目录下
2. 随机初始化输入，分别在此三个模型上运行，得到输出结果，将输入，输出结果存储到input_and_output.npz
3. 将ResNet-18和Deit-Tiny从中间切分，分别保存成ResNet-18_part1.onnx,ResNet-18_part2.onnx,Deit-Tint_part1.onnx和Deit-Tint_part2.onnx，注意，为了测试我们运行时层截断连接，你应该在把残差连接也一起截断。
4. 随机初始化输入，分别在part1模型上运行，得到中间结果，并将中间结果放到part2上运行，得到最终输出，然后将输入，中间结果，最终输出保存到input_and_output.npz


version 0.5

