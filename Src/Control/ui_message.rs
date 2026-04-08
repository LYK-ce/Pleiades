//Presented by KeJi
//Date ： 2026-04-07

//! UI 消息协议模块
//!
//! 定义 Control 层向 UI 层发送的消息类型。
//! 此类型是 Control 层的**输出接口**，任何 UI 实现（TUI/Web/GUI）
//! 都通过消费 `Ui_Message` 来获取显示数据。
//!
//! 依赖方向：UI → Control（UI 引用此类型），Control 不依赖任何 UI 模块。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// ============================================================
// Ui_Message — Control → UI 通信协议
// ============================================================

/// UI 消息（Control 层发送给 UI 层渲染）
///
/// 所有字段使用纯数据类型（String/u64/usize/f64），
/// 不包含任何 Network/ML_Engine/UI 框架的类型，
/// 确保与具体 UI 实现完全解耦。
#[derive(Debug, Clone)]
pub enum Ui_Message {
    /// 普通日志
    Log(String),

    /// 节点状态变化: "Idle" / "Busy"
    State_Change(String),

    /// 发现新节点
    Peer_Discovered(String),

    /// 节点离开
    Peer_Left(String),

    /// 连接建立
    Connection_Established(String),

    /// 连接断开
    Connection_Closed(String),

    /// Job 状态更新 — 文件传输（显示进度条）
    ///
    /// direction: "send" / "receive"
    File_Progress {
        file_name: String,
        direction: String,
        peer: String,
        sent: u64,
        total: u64,
    },

    /// Job 状态更新 — 推理/工作（不显示进度条）
    Job_Inference {
        model_name: String,
        device_count: usize,
        layer_range: String,
        phase: String,
    },

    /// Job 状态更新 — 恢复空闲
    Job_Idle,

    /// 推理输出 — 逐 token 追加
    Inference_Token(String),

    /// 推理完成
    Inference_Complete {
        text: String,
        tokens: usize,
        tok_per_sec: f64,
        total_secs: f64,
    },

    /// 错误信息
    Error(String),
}
