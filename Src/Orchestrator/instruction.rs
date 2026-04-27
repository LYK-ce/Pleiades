//Presented by KeJi
//Date ： 2026-04-27

use std::collections::HashMap;

use super::slot::{SlotId, ConstValue};

/// 任务指令枚举，定义所有 TaskEngine 可解释执行的指令。
///
/// ## 指令分类
/// - **数据操作**: Const, Move
/// - **推理生命周期**: CreateSession, ShutdownSession, RunProgram, AnalyzeModel, SplitModel
/// - **网络操作**: SendFile, ReceiveFile, OpenTensorStream
/// - **控制流**: JumpIf, Abort
#[derive(Debug, Clone)]
pub enum TaskInstruction {
    // ─── 数据操作（handler_data.rs）─────────────────────────

    /// 将常量值写入目标槽位（value 限制为可 Clone 的 ConstValue 子集）
    Const {
        value: ConstValue,
        dst: SlotId,
    },
    /// 从源槽位取出值，写入目标槽位
    Move {
        src: SlotId,
        dst: SlotId,
    },

    // ─── 推理生命周期（handler_inference.rs）────────────────

    /// 从 `model` 槽位读取模型路径，从 `device` 槽位读取设备字符串，
    /// 从 `io` 槽位 **take** IoHandle，创建 ML Session，session_id 写入 `result` 槽位
    CreateSession {
        model: SlotId,
        device: SlotId,
        io: SlotId,
        result: SlotId,
    },
    /// 从 `session` 槽位读取 session_id，调用 ML Engine 关闭 Session
    ShutdownSession {
        session: SlotId,
    },
    /// 向指定 Session 提交 ML 指令序列执行推理
    /// `session` 槽位提供 session_id，执行结果写入 `result` 槽位
    RunProgram {
        session: SlotId,
        result: SlotId,
    },
    /// 分析模型文件结构（无需 Session），结果写入 `result` 槽位
    AnalyzeModel {
        model: SlotId,
        result: SlotId,
    },
    /// 切分模型文件，`source` 为源文件，`start`/`end` 为层范围，`output` 为输出文件 ID
    SplitModel {
        source: SlotId,
        start: SlotId,
        end: SlotId,
        output: SlotId,
    },

    // ─── 网络操作（handler_network.rs）──────────────────────

    /// 将本地文件发送到远端 Peer，`peer` 槽位提供 PeerId，`file` 槽位提供 file_id
    SendFile {
        peer: SlotId,
        file: SlotId,
    },
    /// 从远端 Peer 接收文件，文件 ID 写入 `result` 槽位
    ReceiveFile {
        result: SlotId,
    },
    /// 与远端 Peer 建立 Tensor Stream 连接，Tensor_IO_Handle 写入 `result` 槽位
    OpenTensorStream {
        peer: SlotId,
        result: SlotId,
    },

    // ─── 控制流（handler_control.rs）────────────────────────

    /// 从 `condition` 槽位读取布尔值。为真时，指令指针跳转到 `TaskProgram.labels` 中对应标签的索引
    JumpIf {
        condition: SlotId,
        label: String,
    },
    /// 立即终止正向执行，触发补偿链
    Abort {
        reason: String,
    },
}

/// 编译后的任务程序，包含正向指令序列、补偿序列和标签映射。
#[derive(Debug, Clone)]
pub struct TaskProgram {
    /// 正向执行序列
    pub instructions: Vec<TaskInstruction>,
    /// 补偿/清理序列（Cancel 或 Abort 时执行）
    pub compensation: Vec<TaskInstruction>,
    /// 标签名 → 指令索引映射
    pub labels: HashMap<String, usize>,
}
