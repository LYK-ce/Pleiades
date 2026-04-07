//Presented by KeJi
//Date ： 2026-04-07

//! 节点间命令协议模块
//!
//! 定义节点之间通过 `Send_Bytes(DataType::Command)` 传输的命令格式。
//! 使用简单的文本协议（`|` 分隔），便于 MVP 阶段调试。
//!
//! ## 命令列表
//! - `Work`: 通知节点进入 Busy 状态
//! - `Load`: 加载指定模型文件的指定层范围
//! - `Pipeline_Flow`: 设置推理结果转发目标节点

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use libp2p::PeerId;

// ============================================================
// 命令枚举
// ============================================================

/// 节点间命令
///
/// 通过 `NodeHandle::Send_Bytes(peer, DataType::Command, payload)` 发送，
/// 接收方在 `Handle_Inbound` 中解析并执行。
#[derive(Debug, Clone)]
pub enum Control_Command {
    /// 通知节点进入 Busy 状态
    ///
    /// 协调者在开始任务前发送，参与者收到后将状态从 Idle 切换为 Busy。
    Work,

    /// 加载模型
    ///
    /// 协调者在分发文件后发送，参与者收到后调用 ML_Service.Load_Model()。
    ///
    /// - `model_path`: 模型文件路径（相对于 Pleiades_Workspace/）
    /// - `start`: 起始层编号
    /// - `end`: 结束层编号
    Load {
        model_path: String,
        start: usize,
        end: usize,
    },

    /// 配置流水线转发目标
    ///
    /// 协调者在加载完成后发送，参与者收到后记住 next_peer，
    /// 推理完成后将结果 tensor 通过 Send_Bytes 发给 next_peer。
    ///
    /// - `next_peer`: 推理完成后结果发送的目标节点 PeerId
    Pipeline_Flow {
        next_peer: PeerId,
    },
}

// ============================================================
// 序列化（Command → bytes）
// ============================================================

/// 将命令序列化为字节流
///
/// 格式（文本，`|` 分隔）：
/// - `"WORK"`
/// - `"LOAD|<model_path>|<start>|<end>"`
/// - `"PIPELINE_FLOW|<next_peer_id>"`
///
/// # 参数
/// - `cmd`: 要序列化的命令
///
/// # 返回
/// 序列化后的字节流，用于 Send_Bytes 的 payload
pub fn Serialize_Command(cmd: &Control_Command) -> Vec<u8> {
    let text = match cmd {
        Control_Command::Work => "WORK".to_string(),
        Control_Command::Load {
            model_path,
            start,
            end,
        } => format!("LOAD|{}|{}|{}", model_path, start, end),
        Control_Command::Pipeline_Flow { next_peer } => {
            format!("PIPELINE_FLOW|{}", next_peer)
        }
    };
    text.into_bytes()
}

// ============================================================
// 反序列化（bytes → Command）
// ============================================================

/// 从字节流反序列化为命令
///
/// # 参数
/// - `payload`: 接收到的原始字节流
///
/// # 返回
/// 解析后的命令，或错误信息
pub fn Deserialize_Command(payload: &[u8]) -> Result<Control_Command, String> {
    let text = std::str::from_utf8(payload)
        .map_err(|e| format!("命令解析失败: 无效 UTF-8: {}", e))?;

    let parts: Vec<&str> = text.splitn(4, '|').collect();

    if parts.is_empty() {
        return Err("命令解析失败: 空 payload".to_string());
    }

    match parts[0] {
        "WORK" => Ok(Control_Command::Work),

        "LOAD" => {
            if parts.len() < 4 {
                return Err(format!(
                    "LOAD 命令格式错误: 需要 4 个字段, 实际 {} 个. 格式: LOAD|path|start|end",
                    parts.len()
                ));
            }
            let model_path = parts[1].to_string();
            let start = parts[2]
                .parse::<usize>()
                .map_err(|e| format!("LOAD 命令 start 解析失败: {}", e))?;
            let end = parts[3]
                .parse::<usize>()
                .map_err(|e| format!("LOAD 命令 end 解析失败: {}", e))?;
            Ok(Control_Command::Load {
                model_path,
                start,
                end,
            })
        }

        "PIPELINE_FLOW" => {
            if parts.len() < 2 {
                return Err(
                    "PIPELINE_FLOW 命令格式错误: 需要 2 个字段. 格式: PIPELINE_FLOW|peer_id"
                        .to_string(),
                );
            }
            let next_peer = parts[1]
                .parse::<PeerId>()
                .map_err(|e| format!("PIPELINE_FLOW 命令 peer_id 解析失败: {}", e))?;
            Ok(Control_Command::Pipeline_Flow { next_peer })
        }

        unknown => Err(format!("未知命令类型: '{}'", unknown)),
    }
}
