//Presented by KeJi
//Date ： 2026-04-29

use std::collections::HashMap;

use super::slot::{SlotId, ConstValue};

/// 任务指令枚举，定义所有 TaskEngine 可解释执行的指令。
///
/// ## 指令分类
/// - **数据操作**: Const, Move
/// - **推理生命周期**: CreateSession, ShutdownSession, RunProgram, AnalyzeModel, SplitModel
/// - **网络操作**: SendFile, ReceiveFile
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
    // ─── Pipeline 规划（handler_scheduler.rs）─────────────────

    /// 规划 Pipeline 拓扑
    ///
    /// 1. 从 `model_info` 槽位读取模型分析结果
    /// 2. 从 `inference_id` 槽位读取 inference_id
    /// 3. 查询 PeerManager 获取可用节点快照
    /// 4. 调用 Scheduler 纯函数计算拓扑
    /// 5. 将 Pipeline_Plan 写入 `result` 槽位
    PlanPipeline {
        model_info: SlotId,
        inference_id: SlotId,
        result: SlotId,
    },

    // ─── Pipeline 编排（handler_network.rs）──────────────────

    /// Phase 1：建立所有张量流连接
    ///
    /// 1. 向所有 Worker 并发发送 `ESTABLISH_TENSOR_STREAM` 命令
    /// 2. Coordinator 自身打开到第一个 Worker 的出站流 + handshake
    /// 3. 等待所有 Worker 回复 OK
    /// 4. 将结果写入 `result` 槽位
    /// 任一 Worker 回复 FAIL → Abort
    EstablishStreams {
        plan: SlotId,
        result: SlotId,
    },
    /// Phase 2：通知所有 Worker 加入流水线
    ///
    /// 1. 向所有 Worker 并发发送 `JOIN_PIPELINE` 命令
    /// 2. 等待所有 Worker 回复 OK（超时 300s）
    /// 3. 将结果写入 `result` 槽位
    /// 任一 Worker 回复 FAIL → Abort
    JoinWorkers {
        plan: SlotId,
        result: SlotId,
    },
    /// Pipeline 清理（补偿指令）
    ///
    /// 清理已建立的张量流、通知已启动的 Worker 停止。
    /// 在 cancel 或 Abort 时执行。
    TeardownPipeline {
        plan: SlotId,
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
