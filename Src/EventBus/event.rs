//Presented by KeJi
//Date ： 2026-04-24

//! EventBus 事件类型定义
//!
//! 定义 `Bus_Event` 枚举，包含所有通过 EventBus 广播的领域事件。
//! 所有字段使用纯数据类型（String/u64/usize/f64），不引入外部领域类型，
//! 确保 EventBus 模块与具体组件完全解耦。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// ============================================================
// Bus_Event — EventBus 领域事件
// ============================================================

/// EventBus 领域事件
///
/// 所有通过 EventBus 广播的事件类型。每个 variant 对应一次
/// UI 可渲染的最小信息单元。
///
/// ## 设计原则
/// - 所有字段使用纯数据类型，不依赖 `libp2p`、`orchestrator` 等外部 crate
/// - 必须实现 `Clone`（`broadcast` channel 要求）
/// - 带 `job_id` 字段以支持多 Job 并发场景
/// - 仅包含通知型事件，命令/请求不走 EventBus
#[derive(Debug, Clone)]
pub enum Bus_Event {
    // ===== 网络/节点事件 =====
    /// 发现新节点（mDNS / DHT）
    Peer_Discovered {
        /// 节点 ID（字符串形式）
        peer_id: String,
    },

    /// 节点离开（mDNS expired / cleanup）
    Peer_Left {
        /// 节点 ID（字符串形式）
        peer_id: String,
    },

    /// TCP 连接建立
    Connection_Established {
        /// 节点 ID（字符串形式）
        peer_id: String,
    },

    /// TCP 连接断开
    Connection_Closed {
        /// 节点 ID（字符串形式）
        peer_id: String,
    },

    // ===== 作业生命周期事件 =====
    /// 作业已创建
    Job_Created {
        /// 作业 ID
        job_id: u64,
        /// 作业类型（"Run" / "Coordinator" / "Relay"）
        kind: String,
        /// 模型名称
        model_name: String,
    },

    /// 作业状态变化
    Job_State_Changed {
        /// 作业 ID
        job_id: u64,
        /// 当前阶段（"Preparing" / "Executing" / "CleaningUp" / "Done"）
        phase: String,
    },

    /// 作业完成
    Job_Completed {
        /// 作业 ID
        job_id: u64,
        /// 结果描述（"Success" / "Cancelled" / "Failed: ..."）
        result: String,
    },

    // ===== 推理事件 =====
    /// 推理开始
    Inference_Started {
        /// 作业 ID
        job_id: u64,
        /// 模型名称
        model_name: String,
        /// 参与推理的设备数量
        device_count: usize,
        /// 本机负责的层范围（如 "0-15"）
        layer_range: String,
    },

    /// 推理 token 输出（逐 token 流式）
    Inference_Token {
        /// 作业 ID
        job_id: u64,
        /// 生成的 token 文本
        token: String,
    },

    /// 推理完成
    Inference_Completed {
        /// 作业 ID
        job_id: u64,
        /// 完整输出文本
        text: String,
        /// 生成的 token 总数
        tokens: usize,
        /// 生成速度（tokens/sec）
        tok_per_sec: f64,
        /// 总耗时（秒）
        total_secs: f64,
    },

    // ===== 文件传输事件 =====
    /// 文件传输进度
    File_Progress {
        /// 文件名
        file_name: String,
        /// 方向（"send" / "receive"）
        direction: String,
        /// 对端节点 ID（字符串形式）
        peer: String,
        /// 已传输字节数
        sent: u64,
        /// 总字节数
        total: u64,
    },

    // ===== 系统事件 =====
    /// 通用日志（供 UI 显示）
    Log {
        /// 日志消息
        message: String,
    },

    /// 错误通知
    Error {
        /// 错误消息
        message: String,
    },

    /// 设备变更
    Device_Changed {
        /// 设备名称（如 "cpu" / "cuda"）
        device: String,
    },

    // ===== 帮助系统 =====
    /// 帮助信息（由 Core 组装，TUI/GUI 渲染）
    HelpInfo {
        /// 内置命令列表
        builtin: Vec<HelpEntry>,
        /// 用户 Lua 命令列表
        user: Vec<HelpEntry>,
    },

    // ===== 命令结果 =====
    /// 通用命令执行结果，TUI 展示到 Command Output 面板
    CommandResult {
        /// 结果文本
        text: String,
        /// 是否已完成（true 表示最终结果，false 表示中间更新）
        completed: bool,
    },
}

/// 帮助条目：一条命令的用法和描述
#[derive(Debug, Clone)]
pub struct HelpEntry {
    /// 命令用法（如 "run <model>"）
    pub usage: String,
    /// 命令描述
    pub description: String,
}
