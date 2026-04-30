//Presented by KeJi
//Date ： 2026-04-29

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
    /// 从 `start`/`end` 槽位读取层范围（U64），
    /// 从 `io` 槽位 **take** IoHandle，创建 ML Session，session_id 写入 `result` 槽位。
    /// `tensor_io` 为可选槽位：分布式推理时 take Tensor_IO_Handle，单机推理时为 None。
    CreateSession {
        model: SlotId,
        device: SlotId,
        start: SlotId,
        end: SlotId,
        io: SlotId,
        tensor_io: Option<SlotId>,
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
    /// 从入站 Stream 接收文件数据并存入 Storage
    ///
    /// 所有输入槽位由 Core 在 spawn Job 时注入 SlotFile：
    /// - `stream`: 入站 libp2p::Stream（take 语义）
    /// - `file_name`: 文件名（来自 in-band header）
    /// - `file_size`: 文件大小（来自 in-band header）
    /// - `checksum`: 发送方校验和（来自 in-band header）
    /// - `result`: 输出 file_id
    ReceiveFile {
        stream: SlotId,
        file_name: SlotId,
        file_size: SlotId,
        checksum: SlotId,
        result: SlotId,
    },
    /// 请求远端 Peer 加入 Pipeline，获取其 Relay Job ID
    ///
    /// 1. 从各槽位读取 PeerId、model_file_id、device、layer_start、layer_end
    /// 2. 构造 REQUEST_PIPELINE payload（携带 coordinator_job_id + 上述参数）
    /// 3. 通过 `send_data(peer, DataType::Command, payload)` 发送
    /// 4. 等待远端 Core 回复 relay_job_id
    /// 5. 将 relay_job_id（U64）存入 `result` 槽位
    RequestPipeline {
        peer: SlotId,
        model: SlotId,
        device: SlotId,
        start: SlotId,
        end: SlotId,
        result: SlotId,
    },
    // NOTE: OpenTensorStream / TakeInboundStream / TakeOutboundStream / BuildTensorIo
    // 已在 Phase 3 中移除。张量流连接现由 Core 在 spawn Job 之前完成（"先连接后启动"模式），
    // Tensor_IO_Endpoint 通过 SLOT_TENSOR_IO 预注入。

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
