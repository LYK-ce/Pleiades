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
文件名称 Runtime.py
位于 Src/ 目录下

运行时层（Runtime.py）是边缘设备的核心执行引擎，主要负责三大功能：一是通过 Get_System_Snapshot() 方法作为资源探针，实时监测设备的CPU、内存、温度、网络状态等系统资源信息并以字典形式返回；二是通过 Load_Model() 方法加载并管理PyTorch深度学习模型；三是通过 Execute() 方法执行模型推理任务，接收模型输入数据并返回推理结果。运行时层为上层调度层提供了设备资源感知能力和模型执行能力，使得每个边缘设备能够根据自身资源状况动态参与分布式推理任务。

class 名称 Runtime
依赖 Logger.py

#### 属性
model   这个属性用于指明当前要运行的模型，这里是指torch模型

#### 方法

Get_System_Snapshot
输入    无
输出    字典
此函数是资源探针方法，它将获取当前设备的cpu状况，内存，温度，网络状态等等信息，先判断是否存在logger，如果存在调用logger的Log方法，flag为snapshotflag，输入为设备信息。然后通过字典的方式返回。

Load_Model
输入    模型路径
输出    bool
此函数将先先判断是否存在logger，如果存在调用logger的Log方法，flag为Load，输入为prepare loading通过torch加载，将目标路径下的模型加载并赋值给model，然后再次调用Log方法，输入为Loading Success/False，根据Load结果确定，返回是否成功加载

Execute
输入    模型输入
输出    模型输出
此函数先判断model是否已经有模型了，调用logger进行Log，输入为是否存在模型，然后将模型输入送给model进行执行，再次调用logger进行Log，输入为Execution是否成功，如果不成功把错误信息进行输入，并将执行结果返回给调用者。


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


version 0.5

