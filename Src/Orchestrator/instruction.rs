// Presented by KeJi
// Date ： 2026-04-23

use std::collections::HashMap;

use super::slot::{SlotId, ConstValue};

/// 任务指令枚举，定义所有 TaskEngine 可解释执行的指令。
#[derive(Debug, Clone)]
pub enum TaskInstruction {
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
    /// 从 `preferred` 槽位读取设备偏好字符串，向 ComputeManager 申请租约，结果写入 `result` 槽位。
    /// Phase 2 占位返回空 Stub
    AcquireDevice {
        preferred: SlotId,
        result: SlotId,
    },
    /// 从 `model` 槽位读取模型路径，从 `device` 槽位 **take** 设备租约，
    /// 从 `io` 槽位 **take** IoHandle，创建 ML Session，句柄写入 `result` 槽位
    CreateSession {
        model: SlotId,
        device: SlotId,
        io: SlotId,
        result: SlotId,
    },
    /// 从 `session` 槽位 **take** Session 句柄，调用 Inference Capability 关闭
    ShutdownSession {
        session: SlotId,
    },
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