# ML Engine层的设计方案

## 设计原则
- 采用指令泵设计，而非写死逻辑，向外提供API
- 采用Session设计，一个Session对应一个thread。当创建session时创建线程，销毁session时销毁线程。一个模型对应一个Session，如果要换模型，就需要销毁当前Session并创建新的Session。
- 所有指令下给Session，指令默认使用当前Session当中的配置
- 重新划分职责，thread仅作模型推理相关的职责，不再负责执行模型切分，传输等功能。也就是模型编排我们仍然交给control层来实现。
- 三平面通道架构：命令平面（cmd）、数据平面-Control（input_data/output_data）、数据平面-网络（tensor_streams）

## 通道架构
Session 与外部的通信分为三个平面：

### 命令平面
- cmd_rx (Control → Session)：传递会话生命周期控制命令，如 Run_Program、Shutdown。

### 数据平面 - Control
用于 Session 与 Control 层之间的数据交互，将命令平面与数据传输解耦。

- input_data_rx (Control → Session)：传递用户数据，如 Prompt 文本。Input 指令从此通道读取。
- output_data_tx (Session → Control)：传递推理结果，如流式 token 文本片段、模型信息等。Output 指令通过此通道发送。

### 数据平面 - 网络
用于 Session 与其他节点之间的张量传输，将节点间数据传输与 Control 层解耦。

- inbound_tensor_stream (上游节点 → Session)：接收来自上游节点的张量帧。
- outbound_tensor_stream (Session → 下游节点)：发送张量帧到下游节点。

这两者不是必须的，可以为None（单机推理场景）。

### 通道总览
| 通道 | 方向 | 平面 | 内容 | 生命周期 |
|---|---|---|---|---|
| cmd_rx | Control → Session | 命令平面 | Run_Program, Shutdown | Session级 |
| input_data_rx | Control → Session | 数据平面(Control) | Prompt(String), 未来可扩展 | Session级 |
| output_data_tx | Session → Control | 数据平面(Control) | Text(String), Info(Model_Info) | Session级 |
| inbound_tensor_stream | 上游节点 → Session | 数据平面(网络) | 张量帧 | Session级 |
| outbound_tensor_stream | Session → 下游节点 | 数据平面(网络) | 张量帧 | Session级 |

所有通道均在 Create_Session 时建立，生命周期与 Session 一致。

## ML Service层设计
向control层提供创建session，切分模型，分析模型等的方法。

### 方法
#### Create Session
async方法。内部流程：创建通道 → 启动OS线程 → 线程内阻塞加载模型 → 通过oneshot通道发送就绪信号 → 返回结果。如果模型加载失败，返回Err。

输入：
- session_id        session的唯一标识
- model_path        模型路径
- layer_start       从第几层模型开始加载
- layer_end         到第几层结束
- device            使用的device，cpu或者cuda
- tensor_io         张量IO句柄 (Option)，包含入站和出站张量流。单机推理时为None。

功能：
- 创建命令通道 (cmd_tx/cmd_rx)
- 创建数据通道 (input_data_tx/input_data_rx, output_data_tx/output_data_rx)
- 创建就绪信号通道 (oneshot)
- 启动OS线程，将 cmd_rx, input_data_rx, output_data_tx, tensor_io 及配置传入线程
- 线程内部：阻塞加载模型 → 初始化寄存器 → 通过oneshot发送 Ok(Model_Info) 或 Err
- await就绪信号，加载成功返回 (Session_Handle, output_data_rx, Model_Info)，失败返回 Err

#### Split Model
根据输入切分模型，根据当前的Split Model进行实现即可。这是独立方法，不走Session。

#### Analyze Model
根据输入的路径分析模型，根据当前的Analyze Model进行实现即可。这是独立方法，不走Session。

## Session Handle设计
Session Handle 是 Control 层持有的句柄，通过内部通道与Session线程通信。

### 字段
- session_id    Session的唯一标识
- cmd_tx        命令发送通道
- input_data_tx 数据输入发送通道

### 方法
#### Run_Program
提交指令序列到Session执行。

输入：
- program       指令序列 (Vec\<Instruction\>)
- params        执行参数 (Pipeline_Params)
- cancel_flag   取消标志 (Arc\<AtomicBool\>)

功能：
- 通过 cmd_tx 发送 Session_Command::Run_Program（附带 oneshot reply 通道）
- await oneshot reply 获取执行结果

返回：Result\<Pipeline_Result\>

#### Send_Input
向Session发送用户数据（如prompt文本）。

输入：
- input         引擎输入 (Engine_Input)

功能：
- 通过 input_data_tx 发送数据
- Input 指令会从 input_data_rx 中读取此数据

#### Shutdown
关闭Session，停止线程，释放资源。

功能：
- 通过 cmd_tx 发送 Session_Command::Shutdown

#### Get_Id
获取Session ID，纯访问器。

### 取消机制
当Session阻塞在 input_data_rx 等待用户输入时，Control可通过以下方式取消：
1. cancel_flag：Session在指令执行中定期检查
2. drop Session_Handle：input_data_tx 被drop后，input_data_rx.blocking_recv() 返回 None，Session视为取消

## Session 内部命令协议
```
enum Session_Command {
    Run_Program {
        program: Vec<Instruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<Pipeline_Result>>,
    },
    Shutdown,
}
```

## Session 设计
Session作为推理会话，有状态，独占线程。一个模型对应一个Session。kv cache暂时仍然由candle库自己管理，但是我们之后会进行手动管理。

它是一个容器变量，包含以下内容：
- id                    session id
- cmd_rx                命令接收通道（命令平面）
- input_data_rx         数据输入通道（数据平面-Control入）
- output_data_tx        数据输出通道（数据平面-Control出）
- backend               推理后端，也就是模型等内容
- config                创建时传入，只读
- tensor_io             张量IO句柄 (Option)，包含入站和出站张量流（数据平面-网络）
- register              寄存器

### Session Thread
Session的线程入口

输入：
- session_id        线程标识
- session_config    Session所需的配置信息
- cmd_rx            命令通道
- input_data_rx     数据输入通道
- output_data_tx    数据输出通道
- tensor_io         张量IO句柄 (Option)
- ready_tx          就绪信号通道 (oneshot)

功能：
- 阻塞的形式加载模型（独立线程不影响tokio）
- 初始化寄存器
- 通过 ready_tx 发送 Ok(Model_Info) 或 Err
- 创建Session实例，作为推理会话的上下文
- 进入指令泵循环：从cmd_rx接收Session_Command，执行指令序列

### 指令泵循环逻辑
```
loop {
    match cmd_rx.blocking_recv() {
        Run_Program { program, params, cancel_flag, reply } => {
            // 使用 session 的 register, backend, tensor_io 执行指令
            let result = Execute(&program, &mut session, params, cancel_flag);
            reply.send(result);
        }
        Shutdown => break,
        None => break,  // Handle被drop，通道关闭
    }
}
```

## 指令集
待设计。指令将操作 Session 内部的寄存器（参见 register.md），使用 Session 的 backend 进行推理，通过 Session 的数据通道进行I/O。
