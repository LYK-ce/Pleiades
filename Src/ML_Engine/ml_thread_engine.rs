//Presented by KeJi
//Date ： 2026-04-13

//! ML Session 引擎核心
//!
//! 基于 Session 设计的推理引擎。一个 Session 对应一个 OS 线程，
//! 一个模型对应一个 Session。Session 通过三平面通道架构与外部通信。
//!
//! ## 核心组件
//! - `Session`：推理会话容器（线程私有，有状态）
//! - `Session_Handle`：Control 层持有的句柄，通过通道与 Session 通信
//! - `Session_Command`：命令平面协议
//! - `Session_Config`：Session 创建配置
//! - `Inference_Backend`：推理后端抽象
//! - `Session_Thread()`：线程入口函数
//! - `Execute()`：指令执行器
//!
//! ## 三平面通道架构
//! | 通道 | 方向 | 平面 | 内容 |
//! |---|---|---|---|
//! | cmd_rx | Control → Session | 命令平面 | Run_Program, Shutdown |
//! | input_data_rx | Control → Session | 数据平面(Control) | Prompt(String) |
//! | output_data_tx | Session → Control | 数据平面(Control) | Text(String), Info(Model_Info) |
//! | tensor_io | 上游/下游节点 ↔ Session | 数据平面(网络) | 张量帧 |
//!
//! ## 设计原则
//! - **Session = 容器**：持有 backend、register、config、通道、tensor_io
//! - **独占线程**：一个 Session 独占一个 OS 线程，不阻塞 tokio
//! - **指令泵**：所有操作通过指令序列（Program）驱动
//! - **每步可取消**：通过 cancel_flag 实现用户中途取消
//! - **错误即终止**：指令出错时设 FLAG4+FLAG1，Loop 检测后退出

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use candle_core::Tensor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn, error};

use super::gguf_model::{
    GGUF_Load_Model, GGUF_Model, GGUF_Unload_Model,
    GGUF_Model_Inference, GGUF_Encode, GGUF_Decode,
};
use super::gguf_tensor::{
    GGUF_Tensor_Packet, GGUF_Dtype,
    GGUF_Tensor_Serialize, GGUF_Tensor_Deserialize,
};
use super::ml_thread_engine_instruction::{
    Engine_Input, Engine_Output, Instruction, Model_Info, Pipeline_Params, Pipeline_Result,
    Inference_Input, Set_Target,
};
use super::ml_thread_register::{
    Register_File,
    TEXT1, TEXT2,
    TOKENID1, TOKENID2, TOKENID3,
    TENSOR1, TENSOR2,
    FLAG1, FLAG4,
    META1, META2, META4, META5,
};
use crate::network::tensor_stream_manager::Tensor_IO_Handle;

// ============================================================
// 推理后端
// ============================================================

/// 推理引擎后端
///
/// 当前仅实现 GGUF/Candle 后端。
/// 未来扩展时，在此枚举中添加新变体即可。
pub enum Inference_Backend {
    /// GGUF/Candle 后端
    GGUF(GGUF_Model),
}

// ============================================================
// Session 配置
// ============================================================

/// Session 创建配置
///
/// 由 `Create_Session` 构造，传入 `Session_Thread`。
/// Session 创建后只读，不可修改。
pub struct Session_Config {
    /// 模型文件路径
    pub model_path: PathBuf,
    /// 起始加载层编号（0 = embedding 输入层）
    pub layer_start: usize,
    /// 结束加载层编号（N+1 = 输出层）
    pub layer_end: usize,
    /// 使用的设备（"cpu" 或 "cuda"）
    pub device: String,
}

// ============================================================
// Session 命令协议（命令平面）
// ============================================================

/// Session 内部命令
///
/// 通过 cmd_tx/cmd_rx 通道在 Control 层和 Session 线程之间传递。
/// 仅两个变体：Run_Program 和 Shutdown。
pub enum Session_Command {
    /// 执行指令序列
    ///
    /// Session 线程收到后调用 `Execute(&program, &mut session, params, cancel_flag)`，
    /// 执行完成后通过 reply 返回 `Pipeline_Result`。
    Run_Program {
        /// 指令序列
        program: Vec<Instruction>,
        /// 执行参数（Control 层构造）
        params: Pipeline_Params,
        /// 取消标志（与 Control 层共享）
        cancel_flag: Arc<AtomicBool>,
        /// 回复通道
        reply: oneshot::Sender<Result<Pipeline_Result>>,
    },
    /// 关闭 Session，停止线程，释放资源
    Shutdown,
}

// ============================================================
// Session（推理会话容器，线程私有）
// ============================================================

/// 推理会话容器
///
/// 有状态，独占线程。一个模型对应一个 Session。
/// 所有字段为线程私有，不需要 `Arc<Mutex<>>`。
///
/// Session 由 `Session_Thread` 内部构造，不暴露给 Control 层。
/// Control 层通过 `Session_Handle` 与 Session 通信。
pub struct Session {
    /// Session 唯一标识
    pub id: String,

    /// 命令接收通道（命令平面）
    pub cmd_rx: mpsc::Receiver<Session_Command>,

    /// 数据输入通道（数据平面-Control入）
    /// Input 指令从此通道读取用户数据（如 Prompt 文本）
    pub input_data_rx: mpsc::Receiver<Engine_Input>,

    /// 数据输出通道（数据平面-Control出）
    /// Output 指令通过此通道发送推理结果（如流式 token 文本片段、模型信息等）
    pub output_data_tx: mpsc::Sender<Engine_Output>,

    /// 推理后端（模型等内容）
    pub backend: Inference_Backend,

    /// 创建时传入的配置，只读
    pub config: Session_Config,

    /// 张量 IO 句柄（数据平面-网络）
    /// 包含入站和出站张量流。单机推理时为 None。
    pub tensor_io: Option<Tensor_IO_Handle>,

    /// 寄存器组
    pub register: Register_File,
}

// ============================================================
// Session_Handle（Control 层持有的句柄）
// ============================================================

/// Session 句柄
///
/// Control 层持有的句柄，通过内部通道与 Session 线程通信。
/// 可 Clone，可 Send。
#[derive(Clone)]
pub struct Session_Handle {
    /// Session 唯一标识
    session_id: String,
    /// 命令发送通道（命令平面）
    cmd_tx: mpsc::Sender<Session_Command>,
    /// 数据输入发送通道（数据平面-Control入）
    input_data_tx: mpsc::Sender<Engine_Input>,
}

impl Session_Handle {
    /// 创建 Session_Handle 实例
    ///
    /// 由 `Create_Session` 内部调用。
    pub fn New(
        session_id: String,
        cmd_tx: mpsc::Sender<Session_Command>,
        input_data_tx: mpsc::Sender<Engine_Input>,
    ) -> Self {
        Self {
            session_id,
            cmd_tx,
            input_data_tx,
        }
    }

    /// Run_Program - 提交指令序列到 Session 执行
    ///
    /// 通过 cmd_tx 发送 `Session_Command::Run_Program`（附带 oneshot reply 通道），
    /// await oneshot reply 获取执行结果。
    ///
    /// # 参数
    /// - `program`: 指令序列 (`Vec<Instruction>`)
    /// - `params`: 执行参数 (`Pipeline_Params`)
    /// - `cancel_flag`: 取消标志 (`Arc<AtomicBool>`)
    ///
    /// # 返回
    /// `Result<Pipeline_Result>`
    pub async fn Run_Program(
        &self,
        program: Vec<Instruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<Pipeline_Result> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Session_Command::Run_Program {
                program,
                params,
                cancel_flag,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Session {} 线程已停止", self.session_id))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("Session {} 回复通道已关闭", self.session_id))?
    }

    /// Send_Input - 向 Session 发送用户数据（如 prompt 文本）
    ///
    /// 通过 input_data_tx 发送数据。
    /// Input 指令会从 input_data_rx 中读取此数据。
    ///
    /// # 参数
    /// - `input`: 引擎输入 (`Engine_Input`)
    pub async fn Send_Input(&self, input: Engine_Input) -> Result<()> {
        self.input_data_tx
            .send(input)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Session {} input_data 通道已关闭",
                    self.session_id
                )
            })
    }

    /// Shutdown - 关闭 Session，停止线程，释放资源
    ///
    /// 通过 cmd_tx 发送 `Session_Command::Shutdown`。
    pub async fn Shutdown(&self) -> Result<()> {
        self.cmd_tx
            .send(Session_Command::Shutdown)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Session {} 线程已停止（无法发送 Shutdown）",
                    self.session_id
                )
            })
    }

    /// Get_Id - 获取 Session ID
    pub fn Get_Id(&self) -> &str {
        &self.session_id
    }
}

// ============================================================
// Session_Thread（线程入口）
// ============================================================

/// Session 线程入口函数
///
/// 在独立 OS 线程中运行。执行以下步骤：
/// 1. 阻塞加载模型（独立线程不影响 tokio）
/// 2. 初始化寄存器
/// 3. 通过 ready_tx 发送 `Ok(Model_Info)` 或 `Err`
/// 4. 创建 Session 实例，作为推理会话的上下文
/// 5. 进入指令泵循环：从 cmd_rx 接收 Session_Command，执行指令序列
///
/// # 参数
/// - `session_id`: 线程标识
/// - `session_config`: Session 所需的配置信息
/// - `cmd_rx`: 命令通道
/// - `input_data_rx`: 数据输入通道
/// - `output_data_tx`: 数据输出通道
/// - `tensor_io`: 张量 IO 句柄 (Option)
/// - `ready_tx`: 就绪信号通道 (oneshot)
pub fn Session_Thread(
    session_id: String,
    session_config: Session_Config,
    cmd_rx: mpsc::Receiver<Session_Command>,
    input_data_rx: mpsc::Receiver<Engine_Input>,
    output_data_tx: mpsc::Sender<Engine_Output>,
    tensor_io: Option<Tensor_IO_Handle>,
    ready_tx: oneshot::Sender<Result<Model_Info>>,
) {
    info!(
        "Session [{}]: 线程已启动, 加载模型 {} (layers {}-{}, device: {})",
        session_id,
        session_config.model_path.display(),
        session_config.layer_start,
        session_config.layer_end,
        session_config.device
    );

    // ===== Step 1: 阻塞加载模型 =====
    let device = match session_config.device.to_lowercase().as_str() {
        "cpu" => candle_core::Device::Cpu,
        "cuda" => match candle_core::Device::new_cuda(0) {
            Ok(d) => d,
            Err(e) => {
                let err_msg = format!("Session [{}]: CUDA 设备初始化失败: {}", session_id, e);
                warn!("{}", err_msg);
                let _ = ready_tx.send(Err(anyhow::anyhow!(err_msg)));
                return;
            }
        },
        other => {
            let err_msg = format!(
                "Session [{}]: 不支持的设备类型: '{}'. 目前仅支持 'cpu' 和 'cuda'.",
                session_id, other
            );
            warn!("{}", err_msg);
            let _ = ready_tx.send(Err(anyhow::anyhow!(err_msg)));
            return;
        }
    };

    let gguf_model = match GGUF_Load_Model(
        session_config.layer_start,
        session_config.layer_end,
        &session_config.model_path,
        &device,
    ) {
        Ok(model) => model,
        Err(e) => {
            let err_msg = format!("Session [{}]: 模型加载失败: {}", session_id, e);
            warn!("{}", err_msg);
            let _ = ready_tx.send(Err(anyhow::anyhow!(err_msg)));
            return;
        }
    };

    // ===== Step 2: 初始化寄存器 =====
    let register = Register_File::Init(4096, 2048);

    // ===== Step 3: 构造 Model_Info 并通过 ready_tx 发送 =====
    let model_info = Model_Info {
        architecture: gguf_model.arch_info.architecture.clone(),
        num_layers: gguf_model.arch_info.num_layers,
        has_input_head: gguf_model.has_input_head,
        has_output_head: gguf_model.has_output_head,
        has_tokenizer: gguf_model.tokenizer.is_some(),
        eos_token_id: gguf_model.arch_info.eos_token_id,
    };

    info!(
        "Session [{}]: 模型加载完成 (arch: {}, layers: {}, input_head: {}, output_head: {}, tokenizer: {})",
        session_id,
        model_info.architecture,
        model_info.num_layers,
        model_info.has_input_head,
        model_info.has_output_head,
        model_info.has_tokenizer
    );

    if let Err(_) = ready_tx.send(Ok(model_info)) {
        warn!(
            "Session [{}]: 就绪信号发送失败（调用方可能已取消）",
            session_id
        );
        return;
    }

    // ===== Step 4: 创建 Session 实例 =====
    let backend = Inference_Backend::GGUF(gguf_model);

    let mut session = Session {
        id: session_id.clone(),
        cmd_rx,
        input_data_rx,
        output_data_tx,
        backend,
        config: session_config,
        tensor_io,
        register,
    };

    info!("Session [{}]: 进入指令泵循环", session_id);

    // ===== Step 5: 指令泵循环 =====
    loop {
        match session.cmd_rx.blocking_recv() {
            Some(Session_Command::Run_Program {
                program,
                params,
                cancel_flag,
                reply,
            }) => {
                info!(
                    "Session [{}]: 收到 Run_Program ({} 条指令)",
                    session_id,
                    program.len()
                );

                // 使用 session 的 register, backend, tensor_io 执行指令
                let result = Execute(&program, &mut session, params, cancel_flag);

                let _ = reply.send(result);
            }
            Some(Session_Command::Shutdown) => {
                info!("Session [{}]: 收到 Shutdown 命令，正在退出...", session_id);
                break;
            }
            None => {
                // Handle 被 drop，通道关闭
                info!(
                    "Session [{}]: 所有句柄已释放，退出指令泵",
                    session_id
                );
                break;
            }
        }
    }

    // 卸载模型
    match session.backend {
        Inference_Backend::GGUF(model) => {
            info!("Session [{}]: 卸载模型...", session_id);
            GGUF_Unload_Model(model);
        }
    }

    info!("Session [{}]: 线程已退出", session_id);
}

// ============================================================
// 指令执行上下文
// ============================================================

/// 指令执行上下文（Execute 内部使用）
///
/// 在指令序列执行期间维护的运行时状态。
/// 包含执行参数、终止控制和取消标志。
struct Execution_Context<'a> {
    /// 执行参数（来自 Control 层）
    params: &'a Pipeline_Params,
    /// 解析后的 EOS token ID
    eos_token_id: u32,
    /// 取消标志（与 Control 层共享）
    cancel_flag: Arc<AtomicBool>,
    /// Loop 退出标志（BreakIf 设置，Loop 检测后退出）
    should_break: bool,
    /// 采样 RNG 状态（简单 LCG，无需 rand 依赖）
    rng_state: u64,
}

// ============================================================
// 指令执行器
// ============================================================

/// 执行指令序列
///
/// 逐条执行 program 中的指令。使用 Session 的 backend、register、
/// tensor_io、input_data_rx、output_data_tx 等资源。
///
/// ## 执行流程
/// 1. 重置所有寄存器（清空上次残留状态）
/// 2. 解析 eos_token_id（优先使用 params 指定，否则从模型获取）
/// 3. 遍历指令序列，逐条执行
/// 4. 从寄存器中提取结果，构造 Pipeline_Result 返回
///
/// ## 错误处理
/// - 指令执行出错 → 设 FLAG4=true + FLAG1=true，Loop 检测后退出
/// - 用户取消 → 立即返回 Err("Program cancelled")
///
/// # 参数
/// - `program`: 指令序列
/// - `session`: 推理会话（可变引用）
/// - `params`: 执行参数
/// - `cancel_flag`: 取消标志（与 Control 层共享）
///
/// # 返回
/// - `Ok(Pipeline_Result)`: 全部指令执行完成
/// - `Err`: 某条指令执行失败或被取消
pub fn Execute(
    program: &[Instruction],
    session: &mut Session,
    params: Pipeline_Params,
    cancel_flag: Arc<AtomicBool>,
) -> Result<Pipeline_Result> {
    let start_time = Instant::now();

    // Step 1: 重置所有寄存器
    session.register.Reset_All();

    // Step 2: 解析 eos_token_id
    let eos_token_id = params.eos_token_id.unwrap_or_else(|| {
        match &session.backend {
            Inference_Backend::GGUF(m) => m.inference_config.eos_token,
        }
    });

    // Step 3: 构造执行上下文
    let mut ctx = Execution_Context {
        params: &params,
        eos_token_id,
        cancel_flag,
        should_break: false,
        rng_state: params.seed,
    };

    info!("Session [{}]: 开始执行 Program ({} 条指令, eos={})",
        session.id, program.len(), eos_token_id);

    // Step 4: 逐条执行
    Execute_Program(program, session, &mut ctx)?;

    // Step 5: 从寄存器提取结果
    let result = Pipeline_Result {
        result_text: session.register.Get_Text(TEXT2).to_string(),
        generated_tokens: session.register.Get_Token(TOKENID1).to_vec(),
        prompt_tokens: session.register.Get_Token(TOKENID3).to_vec(),
        model_info: None,
        duration: start_time.elapsed(),
        total_steps: session.register.Get_Meta(META4) as usize,
    };

    info!("Session [{}]: Program 执行完成 (steps={}, gen_tokens={}, elapsed={:?})",
        session.id, result.total_steps, result.generated_tokens.len(), result.duration);

    Ok(result)
}

/// 执行指令序列（内部递归，支持 Loop 嵌套）
///
/// 遍历指令列表，每条执行前检查取消标志和 should_break。
fn Execute_Program(
    program: &[Instruction],
    session: &mut Session,
    ctx: &mut Execution_Context,
) -> Result<()> {
    for inst in program {
        // 检查取消标志
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Program cancelled"));
        }
        // 检查 should_break（由 BreakIf 设置）
        if ctx.should_break {
            break;
        }
        Execute_Instruction(inst, session, ctx)?;
    }
    Ok(())
}

/// 执行单条指令
///
/// 根据指令类型分发到对应的执行逻辑。
/// 指令出错时设 FLAG4=true + FLAG1=true，不立即返回 Err
/// （由 Loop 的 FLAG1 检查触发退出）。
///
/// 唯一返回 Err 的情况：用户取消（cancel_flag）。
fn Execute_Instruction(
    inst: &Instruction,
    session: &mut Session,
    ctx: &mut Execution_Context,
) -> Result<()> {
    match inst {
        // ===== Input: 阻塞读取 input_data_rx → TEXT1 =====
        Instruction::Input => {
            info!("Session [{}]: [Input] 等待输入...", session.id);
            match session.input_data_rx.blocking_recv() {
                Some(Engine_Input::Prompt(text)) => {
                    info!("Session [{}]: [Input] 收到 prompt ({} chars)", session.id, text.len());
                    session.register.Set_Text(TEXT1, &text);
                }
                None => {
                    error!("Session [{}]: [Input] 输入通道已关闭", session.id);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Encode: TEXT1 → tokenize → TOKENID3, 清空 TOKENID1, META1 = 0 =====
        Instruction::Encode => {
            let prompt = session.register.Get_Text(TEXT1).to_string();
            info!("Session [{}]: [Encode] 编码 prompt ({} chars)", session.id, prompt.len());

            match Encode_With_Backend(&session.backend, &prompt) {
                Ok(token_ids) => {
                    let prompt_len = token_ids.len();
                    session.register.Set_Token(TOKENID3, token_ids);
                    session.register.Reset_Token(TOKENID1);
                    session.register.Set_Meta(META1, 0.0);
                    info!("Session [{}]: [Encode] 完成 ({} tokens, META1=0)",
                        session.id, prompt_len);
                }
                Err(e) => {
                    error!("Session [{}]: [Encode] 失败: {}", session.id, e);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Decode: TOKENID2 → TEXT2 =====
        Instruction::Decode => {
            let token_ids = session.register.Get_Token(TOKENID2).to_vec();
            match Decode_With_Backend(&session.backend, &token_ids) {
                Ok(text) => {
                    session.register.Set_Text(TEXT2, &text);
                }
                Err(e) => {
                    error!("Session [{}]: [Decode] 失败: {}", session.id, e);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Set: 将字面量值写入指定寄存器 =====
        Instruction::Set { target } => {
            match target {
                Set_Target::Meta(reg, value) => {
                    session.register.Set_Meta(*reg, *value);
                }
                Set_Target::Flag(reg, value) => {
                    session.register.Set_Flag(*reg, *value);
                }
            }
        }

        // ===== CopyMeta: src → dst =====
        Instruction::CopyMeta { src, dst } => {
            let value = session.register.Get_Meta(*src);
            session.register.Set_Meta(*dst, value);
        }

        // ===== Prefill: 批量前向推理 (prompt), offset=0, META5=prompt_len =====
        Instruction::Prefill { input: token_reg } => {
            let token_ids = session.register.Get_Token(*token_reg);
            let prompt_len = token_ids.len();
            info!("Session [{}]: [Prefill] tokens={}, offset=0", session.id, prompt_len);

            let device = Get_Device(&session.backend);
            match Tensor::new(token_ids, &device)
                .and_then(|t| t.unsqueeze(0))
            {
                Ok(input_tensor) => {
                    match Inference_With_Backend(&mut session.backend, &input_tensor, 0) {
                        Ok(output_tensor) => {
                            session.register.Set_Tensor(TENSOR2, output_tensor);
                            session.register.Set_Meta(META5, prompt_len as f64);
                            info!("Session [{}]: [Prefill] 完成, META5={}", session.id, prompt_len);
                        }
                        Err(e) => {
                            error!("Session [{}]: [Prefill] 推理失败: {}", session.id, e);
                            Set_Error_Flags(&mut session.register);
                        }
                    }
                }
                Err(e) => {
                    error!("Session [{}]: [Prefill] Tensor 构造失败: {}", session.id, e);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Inference: 单步前向推理 (decode), 输出 → TENSOR2, META1 += 1 =====
        Instruction::Inference { input } => {
            let offset = session.register.Get_Meta(META1) as usize;
            let device = Get_Device(&session.backend);

            // 根据输入类型构造 Tensor
            let input_tensor_result: Result<Tensor> = match input {
                Inference_Input::Tokens(reg) => {
                    let token_ids = session.register.Get_Token(*reg);
                    Tensor::new(token_ids, &device)
                        .and_then(|t| t.unsqueeze(0))
                        .map_err(|e| anyhow::anyhow!("Token → Tensor 失败: {}", e))
                }
                Inference_Input::Tensor(reg) => {
                    match session.register.Get_Tensor(*reg) {
                        Some(tensor) => Ok(tensor.clone()),
                        None => Err(anyhow::anyhow!("Tensor 寄存器为空")),
                    }
                }
            };

            match input_tensor_result {
                Ok(input_tensor) => {
                    match Inference_With_Backend(&mut session.backend, &input_tensor, offset) {
                        Ok(output_tensor) => {
                            session.register.Set_Tensor(TENSOR2, output_tensor);
                            // 仅 Tokens 输入（Coordinator decode）递增 offset
                            // Tensor 输入（Worker relay）不递增，offset 由 Receive 管理
                            if matches!(input, Inference_Input::Tokens(_)) {
                                session.register.Increment_Meta(META1, 1.0);
                            }
                        }
                        Err(e) => {
                            error!("Session [{}]: [Inference] 推理失败: {}", session.id, e);
                            Set_Error_Flags(&mut session.register);
                        }
                    }
                }
                Err(e) => {
                    error!("Session [{}]: [Inference] 输入准备失败: {}", session.id, e);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Sample: logits → 采样 → TOKENID2, 维护 META, 终止检查 =====
        Instruction::Sample { tensor_reg } => {
            match session.register.Get_Tensor(*tensor_reg) {
                Some(logits_tensor) => {
                    match Sample_Token(logits_tensor, ctx.params.temperature, &mut ctx.rng_state) {
                        Ok(next_token) => {
                            // 1. next_token → TOKENID2
                            session.register.Set_Token(TOKENID2, vec![next_token]);
                            // 2. 追加到 TOKENID1
                            session.register.Append_Token(TOKENID1, next_token);
                            // 3. META2(remaining) -= 1
                            session.register.Increment_Meta(META2, -1.0);
                            // 4. META4(step_idx) += 1
                            session.register.Increment_Meta(META4, 1.0);
                            // 5. EOS 检查
                            // 注意: META1(offset) 由 Inference 指令负责递增
                            if next_token == ctx.eos_token_id {
                                info!("Session [{}]: [Sample] EOS token 检测到", session.id);
                                session.register.Set_Flag(FLAG1, true);
                            }
                            // 6. remaining 检查
                            if session.register.Get_Meta(META2) <= 0.0 {
                                info!("Session [{}]: [Sample] 达到最大生成数", session.id);
                                session.register.Set_Flag(FLAG1, true);
                            }
                        }
                        Err(e) => {
                            error!("Session [{}]: [Sample] 采样失败: {}", session.id, e);
                            Set_Error_Flags(&mut session.register);
                        }
                    }
                }
                None => {
                    error!("Session [{}]: [Sample] Tensor 寄存器为空", session.id);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Output: TEXT2 → output_data_tx =====
        Instruction::Output => {
            let text = session.register.Get_Text(TEXT2).to_string();
            if let Err(e) = session.output_data_tx.blocking_send(Engine_Output::Text(text)) {
                warn!("Session [{}]: [Output] 发送失败: {}", session.id, e);
                Set_Error_Flags(&mut session.register);
            }
        }

        // ===== EndOutput: 发送结束信号 =====
        Instruction::EndOutput => {
            info!("Session [{}]: [EndOutput] 发送输出结束信号", session.id);
            if let Err(e) = session.output_data_tx.blocking_send(Engine_Output::End) {
                warn!("Session [{}]: [EndOutput] 发送失败: {}", session.id, e);
            }
        }

        // ===== Send: TENSOR2 → 序列化 → tensor_io.Send() =====
        Instruction::Send => {
            match session.register.Get_Tensor(TENSOR2) {
                Some(tensor) => {
                    match Tensor_To_Bytes(tensor) {
                        Ok(bytes) => {
                            let offset = session.register.Get_Meta(META1) as u64;
                            match session.tensor_io.as_mut() {
                                Some(tio) => {
                                    if let Err(e) = tio.Send(offset, &bytes) {
                                        error!("Session [{}]: [Send] 发送失败: {}", session.id, e);
                                        Set_Error_Flags(&mut session.register);
                                    }
                                }
                                None => {
                                    error!("Session [{}]: [Send] tensor_io 不可用（单机模式？）", session.id);
                                    Set_Error_Flags(&mut session.register);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Session [{}]: [Send] 张量序列化失败: {}", session.id, e);
                            Set_Error_Flags(&mut session.register);
                        }
                    }
                }
                None => {
                    error!("Session [{}]: [Send] TENSOR2 为空", session.id);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Receive: tensor_io.Receive() → TENSOR1, EOF → FLAG1 =====
        Instruction::Receive => {
            match session.tensor_io.as_mut() {
                Some(tio) => {
                    match tio.Receive() {
                        Ok(offset) => {
                            if offset == u64::MAX {
                                // EOF 哨兵帧
                                info!("Session [{}]: [Receive] 收到 EOF", session.id);
                                session.register.Set_Flag(FLAG1, true);
                            } else {
                                // 正常数据帧 → 反序列化 → TENSOR1
                                let buffer = tio.Get_Buffer().to_vec();
                                match Bytes_To_Tensor(&buffer, &Get_Device(&session.backend)) {
                                    Ok(tensor) => {
                                        session.register.Set_Tensor(TENSOR1, tensor);
                                        // 将接收到的 offset 写入 META1（Worker 用此 offset 做推理）
                                        session.register.Set_Meta(META1, offset as f64);
                                    }
                                    Err(e) => {
                                        error!("Session [{}]: [Receive] 张量反序列化失败: {}", session.id, e);
                                        Set_Error_Flags(&mut session.register);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Session [{}]: [Receive] 接收失败: {}", session.id, e);
                            Set_Error_Flags(&mut session.register);
                        }
                    }
                }
                None => {
                    error!("Session [{}]: [Receive] tensor_io 不可用（单机模式？）", session.id);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== SendEOF: 发送 EOF 哨兵帧 =====
        Instruction::SendEOF => {
            info!("Session [{}]: [SendEOF] 发送 EOF", session.id);
            match session.tensor_io.as_mut() {
                Some(tio) => {
                    if let Err(e) = tio.Send_EOF() {
                        error!("Session [{}]: [SendEOF] 发送失败: {}", session.id, e);
                        Set_Error_Flags(&mut session.register);
                    }
                }
                None => {
                    error!("Session [{}]: [SendEOF] tensor_io 不可用", session.id);
                    Set_Error_Flags(&mut session.register);
                }
            }
        }

        // ===== Loop: 循环执行 body，每次迭代后检查 FLAG1 =====
        Instruction::Loop { body } => {
            let mut iteration = 0u64;
            loop {
                // 检查取消
                if ctx.cancel_flag.load(Ordering::Relaxed) {
                    return Err(anyhow::anyhow!("Program cancelled"));
                }

                // 执行 body
                Execute_Program(body, session, ctx)?;

                iteration += 1;

                // 检查 FLAG1（由 Sample/Receive/BreakIf 设置）
                if session.register.Get_Flag(FLAG1) {
                    info!("Session [{}]: [Loop] FLAG1=true, 退出循环 (迭代 {}次)",
                        session.id, iteration);
                    // 重置 should_break（Loop 已处理）
                    ctx.should_break = false;
                    break;
                }
            }
        }

        // ===== BreakIf: 检查 FLAG1, 若 true 则设 should_break =====
        Instruction::BreakIf => {
            if session.register.Get_Flag(FLAG1) {
                ctx.should_break = true;
            }
        }
    }

    Ok(())
}

// ============================================================
// 辅助函数 — 后端操作
// ============================================================

/// 设置错误标志：FLAG4=true (ERROR) + FLAG1=true (BREAK)
fn Set_Error_Flags(register: &mut Register_File) {
    register.Set_Flag(FLAG4, true);
    register.Set_Flag(FLAG1, true);
}

/// 使用后端执行 Encode（TEXT → token IDs）
fn Encode_With_Backend(backend: &Inference_Backend, prompt: &str) -> Result<Vec<u32>> {
    match backend {
        Inference_Backend::GGUF(model) => GGUF_Encode(model, prompt),
    }
}

/// 使用后端执行 Decode（token IDs → TEXT）
fn Decode_With_Backend(backend: &Inference_Backend, token_ids: &[u32]) -> Result<String> {
    match backend {
        Inference_Backend::GGUF(model) => GGUF_Decode(model, token_ids),
    }
}

/// 使用后端执行 Inference（input Tensor + offset → output Tensor）
fn Inference_With_Backend(
    backend: &mut Inference_Backend,
    input: &Tensor,
    offset: usize,
) -> Result<Tensor> {
    match backend {
        Inference_Backend::GGUF(model) => GGUF_Model_Inference(model, input, offset),
    }
}

/// 获取后端设备
fn Get_Device(backend: &Inference_Backend) -> candle_core::Device {
    match backend {
        Inference_Backend::GGUF(model) => model.device.clone(),
    }
}

// ============================================================
// 辅助函数 — 张量序列化 / 反序列化
// ============================================================

/// Candle Tensor → 字节流（GGUF_Tensor_Packet 格式序列化）
///
/// 用于 Send 指令：将 TENSOR2 序列化后通过网络发送。
fn Tensor_To_Bytes(tensor: &Tensor) -> Result<Vec<u8>> {
    let shape = tensor.dims().to_vec();
    let f32_data: Vec<f32> = tensor.flatten_all()?.to_vec1::<f32>()
        .map_err(|e| anyhow::anyhow!("Tensor → f32 失败: {}", e))?;
    let raw_bytes: Vec<u8> = f32_data.iter().flat_map(|f| f.to_le_bytes()).collect();
    let packet = GGUF_Tensor_Packet::New(
        "hidden_state".to_string(),
        shape,
        GGUF_Dtype::F32,
        raw_bytes,
    );
    GGUF_Tensor_Serialize(&packet)
        .map_err(|e| anyhow::anyhow!("张量序列化失败: {}", e))
}

/// 字节流 → Candle Tensor（GGUF_Tensor_Packet 格式反序列化）
///
/// 用于 Receive 指令：从网络接收的字节流重建 Tensor。
fn Bytes_To_Tensor(bytes: &[u8], device: &candle_core::Device) -> Result<Tensor> {
    let packet = GGUF_Tensor_Deserialize(bytes)
        .map_err(|e| anyhow::anyhow!("张量反序列化失败: {}", e))?;
    let f32_data: Vec<f32> = packet
        .data
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    let tensor = Tensor::new(&f32_data[..], device)?
        .reshape(&*packet.shape)?;
    Ok(tensor)
}

// ============================================================
// 辅助函数 — 采样
// ============================================================

/// 从 logits Tensor 中采样下一个 token
///
/// ## 采样策略
/// - `temperature <= 0.0`：贪心采样（argmax）
/// - `temperature > 0.0`：温度缩放 + softmax + 随机采样
///
/// ## 参数
/// - `logits`: 模型输出 logits，shape 可为 `[1, seq_len, vocab_size]` 或 `[1, vocab_size]`
/// - `temperature`: 采样温度
/// - `rng_state`: 随机数生成器状态（LCG，可变引用，每次调用后更新）
fn Sample_Token(
    logits: &Tensor,
    temperature: f64,
    rng_state: &mut u64,
) -> Result<u32> {
    // 提取最后一个 position 的 logits → [vocab_size]
    let logits = Extract_Last_Logits(logits)?;

    if temperature <= 0.0 {
        // 贪心采样：argmax
        let token = logits
            .argmax(0)
            .map_err(|e| anyhow::anyhow!("argmax 失败: {}", e))?
            .to_scalar::<u32>()
            .map_err(|e| anyhow::anyhow!("to_scalar 失败: {}", e))?;
        Ok(token)
    } else {
        // 温度采样：logits / temperature → softmax → 随机采样
        let scaled = (&logits / temperature)
            .map_err(|e| anyhow::anyhow!("温度缩放失败: {}", e))?;
        let probs = candle_nn::ops::softmax(&scaled, 0)
            .map_err(|e| anyhow::anyhow!("softmax 失败: {}", e))?;
        let probs_vec: Vec<f32> = probs.to_vec1::<f32>()
            .map_err(|e| anyhow::anyhow!("probs → vec 失败: {}", e))?;

        // LCG 随机数：state = state * 6364136223846793005 + 1442695040888963407
        *rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let random = (*rng_state >> 33) as f32 / (u32::MAX as f32);

        // 累积概率采样
        let mut cumulative = 0.0f32;
        for (i, &p) in probs_vec.iter().enumerate() {
            cumulative += p;
            if cumulative > random {
                return Ok(i as u32);
            }
        }

        // 兜底：返回最后一个 token
        Ok((probs_vec.len() - 1) as u32)
    }
}

/// 从 logits Tensor 中提取最后一个 position 的 logits
///
/// 支持多种 shape：
/// - `[1, seq_len, vocab_size]` → 取 `[seq_len-1]` → `[vocab_size]`
/// - `[1, vocab_size]` → squeeze → `[vocab_size]`
/// - `[vocab_size]` → 直接返回
fn Extract_Last_Logits(logits: &Tensor) -> Result<Tensor> {
    let dims = logits.dims();
    match dims.len() {
        3 => {
            // [batch, seq_len, vocab_size] → 取最后一个 position
            let seq_len = dims[1];
            let last = logits
                .get(0)
                .map_err(|e| anyhow::anyhow!("get batch 0 失败: {}", e))?
                .get(seq_len - 1)
                .map_err(|e| anyhow::anyhow!("get last position 失败: {}", e))?;
            Ok(last)
        }
        2 => {
            // [batch, vocab_size] → squeeze batch dim
            logits
                .squeeze(0)
                .map_err(|e| anyhow::anyhow!("squeeze 失败: {}", e))
        }
        1 => {
            // [vocab_size] → 直接克隆
            logits
                .clone()
                .contiguous()
                .map_err(|e| anyhow::anyhow!("contiguous 失败: {}", e))
        }
        _ => Err(anyhow::anyhow!(
            "不支持的 logits shape: {:?}",
            dims
        )),
    }
}
