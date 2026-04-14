//Presented by KeJi
//Date ： 2026-04-13

//! ML 执行引擎指令集（基于寄存器架构）
//!
//! 定义执行引擎的所有指令、推理输入参数类型、通道消息类型和执行结果。
//! 指令通过寄存器组（Register_File）在指令间传递数据，无隐式状态。
//!
//! ## 指令分类
//! - **数据输入**：Input（从 Control 层接收 prompt）
//! - **编解码**：Encode / Decode
//! - **推理**：Inference（支持 Token/Tensor 两种输入源）
//! - **采样**：Sample（采样 + 序列维护 + 终止检查）
//! - **Control I/O**：Output / EndOutput
//! - **网络 I/O**：Send / Receive / SendEOF
//! - **寄存器操作**：Set（写入字面量）
//! - **控制流**：Loop / BreakIf
//!
//! ## 寄存器约定（详见 instruction.md）
//! - TEXT1=Prompt, TEXT2=Chunk, TOKENID1=Full, TOKENID2=New, TOKENID3=Prompt tokens
//! - TENSOR1=Input, TENSOR2=Output, FLAG1=Break, FLAG4=Error
//! - META1=Offset, META2=Remaining, META3=Timestamp, META4=StepIdx
//!
//! ## 依赖关系
//! ```text
//! ml_thread_register.rs              ← 寄存器类型定义
//!          ↑
//! ml_thread_engine_instruction.rs    ← 指令集（引用寄存器类型）
//!          ↑
//! ml_thread_engine.rs                ← 指令执行器（依赖 instruction + register + backend）
//!          ↑
//! ml_inference_service.rs            ← ML 服务层（Session 创建 + 独立方法）
//!          ↑
//! Control 层                          ← 编排程序 + 提交执行
//! ```

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::time::Duration;

use super::ml_thread_register::{Flag_Reg, Meta_Reg, Tensor_Reg, Token_Reg};

// ============================================================
// 推理输入参数类型
// ============================================================

/// Inference 指令的输入来源
///
/// 指定 `Inference` 指令从哪个寄存器读取输入数据，
/// 以及如何解释输入（Token IDs 需要先 embedding，Tensor 直接 forward）。
///
/// ## 场景选择
/// - `Tokens`：Coordinator / 单机推理 —— 输入为 token IDs，内部先 embedding 再 forward
/// - `Tensor`：Worker 节点 —— 输入为上游节点传来的 hidden state tensor，直接 forward
#[derive(Debug, Clone)]
pub enum Inference_Input {
    /// 从 Token ID 寄存器读取，内部先做 embedding 再 forward
    ///
    /// 用于 Coordinator/单机场景：
    /// - Prefill 阶段：`Inference(Tokens(TOKENID3))`
    /// - Decode 阶段：`Inference(Tokens(TOKENID2))`
    Tokens(Token_Reg),

    /// 从 Tensor 寄存器读取，直接用张量做 forward
    ///
    /// 用于分布式 Worker 场景：
    /// - `Inference(Tensor(TENSOR1))`（从上游 Receive 的 hidden state）
    Tensor(Tensor_Reg),
}

// ============================================================
// Set 指令目标
// ============================================================

/// Set 指令的目标寄存器 + 值
///
/// 用于将字面量值写入指定寄存器，初始化运行时状态。
///
/// ## 用例
/// ```text
/// Set(Meta(META2, 120.0))   // 初始化 remaining = max_tokens
/// Set(Flag(FLAG1, false))   // 重置 break 标志
/// ```
#[derive(Debug, Clone)]
pub enum Set_Target {
    /// 写入 META 寄存器（f64 值）
    Meta(Meta_Reg, f64),
    /// 写入 FLAG 寄存器（bool 值）
    Flag(Flag_Reg, bool),
}

// ============================================================
// 指令集
// ============================================================

/// ML 执行引擎指令（寄存器架构）
///
/// 所有操作（编解码、推理、采样、网络收发、Control 通信）统一建模为指令。
/// 指令间通过寄存器组（Register_File）传递数据，无隐式上下文状态。
///
/// 由 ML Engine Session 线程的 `Execute()` 函数逐条执行。
/// Control 层通过编排不同的指令序列来实现不同场景。
///
/// ## 三种场景的程序编排
///
/// ### 单机推理
/// ```text
/// Input → Encode → Set(META2, max_tokens) → Inference(TOKENID3)
/// → Sample(TENSOR2) → Decode → Output
/// → Loop [ BreakIf, Inference(TOKENID2), Sample(TENSOR2), Decode, Output ]
/// → EndOutput
/// ```
///
/// ### Coordinator（分布式协调者）
/// ```text
/// Input → Encode → Set(META2, max_tokens) → Inference(TOKENID3)
/// → Send → Receive → Sample(TENSOR1) → Decode → Output
/// → Loop [ BreakIf, Inference(TOKENID2), Send, Receive, Sample(TENSOR1), Decode, Output ]
/// → SendEOF → EndOutput
/// ```
///
/// ### Worker（分布式工作节点）
/// ```text
/// Loop [ Receive, BreakIf, Inference(TENSOR1), Send ]
/// ```
#[derive(Debug, Clone)]
pub enum Instruction {
    // ===== 数据输入 =====

    /// 从 Control 层接收输入
    ///
    /// 阻塞读取 `input_data_rx` 通道，收到 prompt 文本后写入 **TEXT1**。
    /// 若通道关闭（Control drop 了发送端），视为取消执行。
    ///
    /// ### 寄存器写入
    /// - **TEXT1** ← prompt 文本
    Input,

    // ===== 编解码 =====

    /// 文本编码：TEXT1 → tokenize → TOKENID3
    ///
    /// ### 副作用
    /// 1. **TEXT1** → tokenize → **TOKENID3**（Prompt token IDs）
    /// 2. 清空 **TOKENID1**（准备接收生成序列）
    /// 3. **META1**(offset) = 0（重置位置偏移，准备 Prefill）
    ///
    /// ### 寄存器读取
    /// - **TEXT1**（输入 prompt）
    ///
    /// ### 寄存器写入
    /// - **TOKENID3** ← prompt token IDs
    /// - **TOKENID1** ← 清空
    /// - **META1** ← 0（Prefill 将从 offset=0 开始）
    Encode,

    /// Token 解码：TOKENID2 → TEXT2
    ///
    /// 增量解码：仅解码 **TOKENID2**（当前 step 新生成的 token），
    /// 结果写入 **TEXT2**（用于流式输出当前文本片段）。
    ///
    /// ### 寄存器读取
    /// - **TOKENID2**（当前新生成的单个 token ID）
    ///
    /// ### 寄存器写入
    /// - **TEXT2** ← 解码后的文本片段
    Decode,

    // ===== 寄存器操作 =====

    /// 将字面量值写入指定寄存器
    ///
    /// 用于初始化运行时状态，如 `Set(Meta(META2, max_tokens))` 设置剩余可生成数。
    ///
    /// ### 参数
    /// - `target`: 目标寄存器 + 值（`Set_Target` 枚举）
    Set {
        target: Set_Target,
    },

    /// 复制 META 寄存器值
    ///
    /// 将 `src` META 寄存器的值复制到 `dst` META 寄存器。
    /// 主要用于在 Prefill 后恢复 META1(offset) = META5(prompt_len)。
    ///
    /// ### 参数
    /// - `src`: 源 META 寄存器
    /// - `dst`: 目标 META 寄存器
    CopyMeta {
        src: Meta_Reg,
        dst: Meta_Reg,
    },

    // ===== 推理 =====

    /// Prefill 前向推理（处理整个 prompt 序列）
    ///
    /// 从指定 Token 寄存器读取 prompt token IDs，**强制 offset=0**，
    /// 执行模型前向传播。prompt_len 存入 **META5**，**不修改 META1**。
    /// 后续通过 `CopyMeta(META5, META1)` 恢复 offset。
    ///
    /// ### 寄存器读取
    /// - 由 `input` 参数指定的 Token 寄存器
    ///
    /// ### 寄存器写入
    /// - **TENSOR2** ← 模型输出张量（logits）
    /// - **META5** ← prompt token 数量（prompt_len，供 CopyMeta 使用）
    Prefill {
        input: Token_Reg,
    },

    /// 单步前向推理（decode 阶段）
    ///
    /// 根据 `input` 类型从对应寄存器读取输入数据，执行模型前向传播，
    /// 输出张量写入 **TENSOR2**。执行后自动递增 **META1** += 1。
    ///
    /// ### 输入类型
    /// - `Tokens(reg)`: 从 Token ID 寄存器读取 → embedding → forward
    /// - `Tensor(reg)`: 从 Tensor 寄存器读取 → 直接 forward
    ///
    /// ### 寄存器读取
    /// - 由 `input` 参数指定的寄存器
    /// - **META1**（当前 offset，用于位置编码和 KV cache）
    ///
    /// ### 寄存器写入
    /// - **TENSOR2** ← 模型输出张量（logits 或 hidden state）
    /// - **META1** += 1（递增位置偏移）
    Inference {
        input: Inference_Input,
    },

    // ===== 采样 =====

    /// 从 Tensor 寄存器采样下一个 token
    ///
    /// 核心生成指令，负责采样、序列维护和终止检查。
    ///
    /// ### 副作用（按顺序）
    /// 1. 从指定 Tensor 寄存器读取 logits，采样得到 next_token → **TOKENID2**
    /// 2. 将 next_token 追加到 **TOKENID1**（维护完整序列）
    /// 3. **META2**(remaining) -= 1
    /// 4. **META4**(step_idx) += 1
    /// 5. 若 next_token == eos_token_id → **FLAG1** = true
    /// 6. 若 **META2** ≤ 0 → **FLAG1** = true
    ///
    /// 注意：**META1**(offset) 由 Inference 指令负责递增，Sample 不再修改。
    ///
    /// ### 寄存器读取
    /// - 参数指定的 Tensor 寄存器（logits 张量）
    /// - **META2**, **META4**
    ///
    /// ### 寄存器写入
    /// - **TOKENID2** ← 新生成的单个 token ID
    /// - **TOKENID1** ← 追加新 token
    /// - **META2** -= 1, **META4** += 1
    /// - **FLAG1** ← true（若触发终止条件）
    Sample {
        tensor_reg: Tensor_Reg,
    },

    // ===== Control I/O =====

    /// 向 Control 层发送当前文本片段
    ///
    /// 将 **TEXT2** 的内容通过 `output_data_tx` 通道发送给 Control/TUI。
    /// 发送 `Engine_Output::Text(TEXT2)`。
    ///
    /// ### 寄存器读取
    /// - **TEXT2**（当前文本片段）
    Output,

    /// 通知 Control 层流式输出结束
    ///
    /// 通过 `output_data_tx` 发送 `Engine_Output::End` 信号。
    /// 所有场景均需在程序末尾调用，通知 Control 层本次推理输出已完成。
    EndOutput,

    // ===== 网络 I/O =====

    /// 发送张量到下游节点
    ///
    /// 将 **TENSOR2** 序列化后通过 `tensor_io.Send()` 发出。
    /// Fire-and-forget，无需等 ACK。
    ///
    /// ### 寄存器读取
    /// - **TENSOR2**（模型输出张量）
    Send,

    /// 从上游节点接收张量
    ///
    /// 从 `tensor_io.Receive()` 读取一帧。
    ///
    /// ### 副作用
    /// 1. 正常帧 → 反序列化为 Tensor → **TENSOR1**
    /// 2. EOF 哨兵帧（`offset=u64::MAX, length=0`）→ **FLAG1** = true
    ///
    /// ### 寄存器写入
    /// - **TENSOR1** ← 接收的张量（正常帧时）
    /// - **FLAG1** ← true（EOF 帧时）
    Receive,

    /// 发送 EOF 哨兵帧
    ///
    /// 推理结束时由协调者调用，通知下游节点推理会话结束。
    /// 帧格式：`offset=u64::MAX, length=0`。
    SendEOF,

    // ===== 控制流 =====

    /// 循环执行 body 指令序列
    ///
    /// 每次迭代结束后检查 **FLAG1**，若为 true 则退出循环。
    /// body 中通常包含 `BreakIf` 指令在循环开头检查退出条件。
    ///
    /// ### 控制流
    /// ```text
    /// loop {
    ///     for inst in body { Execute(inst); }
    ///     if FLAG1 == true { break; }
    /// }
    /// ```
    Loop {
        body: Vec<Instruction>,
    },

    /// 条件中断（检查 FLAG1）
    ///
    /// 检查 **FLAG1**，若为 true 则设置 `should_break` 并立即返回。
    /// 配合 Loop 使用实现 break 语义。
    ///
    /// ### 寄存器读取
    /// - **FLAG1**（统一终止标志）
    BreakIf,
}

// ============================================================
// Control ↔ Engine 通道消息类型
// ============================================================

/// Engine 输入消息（Control → Engine，数据平面）
///
/// Control 层通过 `input_data_tx` 发送给 Session 线程。
/// `Input` 指令从 `input_data_rx` 接收此消息。
#[derive(Debug, Clone)]
pub enum Engine_Input {
    /// 用户输入的 prompt 文本
    ///
    /// Input 指令收到后写入 **TEXT1**，供 Encode 指令使用。
    Prompt(String),
}

/// Engine 输出消息（Engine → Control，数据平面）
///
/// Session 线程通过 `output_data_tx` 发送给 Control/TUI。
#[derive(Debug, Clone)]
pub enum Engine_Output {
    /// 文本片段（流式输出）
    ///
    /// 由 `Output` 指令发送，包含 Decode 后的当前 step 文本片段。
    Text(String),

    /// 输出结束信号
    ///
    /// 由 `EndOutput` 指令发送，通知 Control 层本次推理输出已完成。
    End,

    /// 模型信息输出
    ///
    /// Session 创建时发送模型加载信息（由 Session_Thread 在初始化阶段发送）。
    Info(Model_Info),
}

// ============================================================
// Pipeline 参数（Control 层 → Engine）
// ============================================================

/// Pipeline 执行参数
///
/// 由 Control 层构造，通过 `Run_Program` 提交给 Session 线程。
/// Engine 内部用此参数配置采样行为和终止条件。
///
/// 注意：prompt 不在此处传递，而是在运行时通过 `Input` 指令
/// 从 Control 通道接收。`max_tokens` 通常通过 `Set(Meta(META2, value))`
/// 写入寄存器，但也保留在 params 中供 Engine 参考。
#[derive(Debug, Clone)]
pub struct Pipeline_Params {
    /// 最大生成 token 数
    ///
    /// 通常通过 `Set(Meta(META2, max_tokens))` 写入 META2 寄存器。
    /// Sample 指令每步递减 META2，到 0 时设 FLAG1=true 触发退出。
    pub max_tokens: usize,

    /// 采样温度（0.0 = greedy，越高越随机）
    ///
    /// 由 Sample 指令使用，控制采样随机性。
    pub temperature: f64,

    /// 随机种子
    ///
    /// 由 Sample 指令使用，保证可复现性。
    pub seed: u64,

    /// EOS token ID
    ///
    /// 如果为 None，由 Engine 从已加载模型中自动获取。
    /// Sample 指令采样后检查：若 next_token == eos_token_id → FLAG1=true。
    pub eos_token_id: Option<u32>,
}

impl Default for Pipeline_Params {
    fn default() -> Self {
        Self {
            max_tokens: 120,
            temperature: 0.8,
            seed: 299792458,
            eos_token_id: None,
        }
    }
}

// ============================================================
// Pipeline 结果（Engine → Control 层）
// ============================================================

/// Pipeline 执行结果
///
/// 由 Engine 执行完成后填充，通过 oneshot 返回给 Control 层。
/// 包含最终生成文本、token 序列和执行时间等统计信息。
///
/// 注意：推理过程中的实时输出通过 `Output` 指令 + `Engine_Output::Text` 通道传递，
/// `Pipeline_Result` 仅在整个 program 执行结束后返回。
///
/// 字段来源：
/// - `result_text`: **TEXT2** 最后一次写入的值
/// - `generated_tokens`: **TOKENID1** 的完整内容
/// - `prompt_tokens`: **TOKENID3** 的完整内容
/// - `total_steps`: **META4** 的值（已执行生成步数）
#[derive(Debug)]
pub struct Pipeline_Result {
    /// 最终生成的文本（TEXT2 寄存器最后状态）
    pub result_text: String,

    /// 生成的 token IDs（TOKENID1 寄存器内容）
    pub generated_tokens: Vec<u32>,

    /// 输入 prompt 的 token IDs（TOKENID3 寄存器内容）
    pub prompt_tokens: Vec<u32>,

    /// 模型信息（Session 创建时填充，Program 执行时透传）
    pub model_info: Option<Model_Info>,

    /// 执行总耗时
    pub duration: Duration,

    /// 已执行的生成步数（META4 寄存器值）
    pub total_steps: usize,
}

// ============================================================
// 模型信息
// ============================================================

/// 模型信息（从 Session 创建时获取）
///
/// 由 `Create_Session` 在模型加载后构造，随 `Pipeline_Result` 透传。
/// 也可通过 `Engine_Output::Info` 在 Session 创建时发送给 Control 层。
#[derive(Debug, Clone)]
pub struct Model_Info {
    /// 模型架构名称（如 "qwen3"）
    pub architecture: String,
    /// 总层数（transformer block 数量）
    pub num_layers: usize,
    /// 是否包含输入头（embedding）
    pub has_input_head: bool,
    /// 是否包含输出头（lm_head）
    pub has_output_head: bool,
    /// 是否包含 tokenizer
    pub has_tokenizer: bool,
    /// EOS token ID
    pub eos_token_id: u32,
}
