//Presented by KeJi
//Date ： 2026-04-29

use super::job::JobId;
use tokio::sync::oneshot;

// ============================================================
// UserCommand — 用户侧命令
// ============================================================

/// 用户命令，来自 CLI / TUI / GUI 等前端接入层。
///
/// 所有命令均为 fire-and-forget，Core 通过 EventBus 发布 `Bus_Event::CommandResult`
/// 或更具体的事件（如 HelpInfo）将结果推送给前端。
pub enum UserCommand {
    /// 执行 Lua 策略脚本（通用命令入口）
    ///
    /// `command` 对应 `programs/*.lua` 中声明的 `COMMAND` 值，
    /// `params` 以 `HashMap<String, String>` 传递。
    /// Core 通过 ProgramRegistry 查找对应脚本，调用 `execute(params, caps)`。
    Execute {
        command: String,
        params: std::collections::HashMap<String, String>,
    },
    /// 启动本地推理作业
    Run { script: String, model_path: String },
    /// 取消指定作业
    Cancel { job_id: JobId },
    /// 请求优雅退出
    Quit { reply: oneshot::Sender<()> },
    /// 查询当前节点列表
    DisplayPeer,
    /// 修改默认计算设备偏好
    SetDevice { device: String },
    /// 分发模型分片到远端节点
    DistributeModel {
        model_path: String,
        peers: Vec<(String, usize, usize)>,
    },
    /// 列出 Storage 当前追踪的所有文件
    List,
    /// 向指定节点发送单个文件
    Send { file_path: String, peer_id: String },
    /// Profile 指定模型的单层推理耗时
    Profile { model_id: String },
    /// 请求帮助信息
    ///
    /// Core 通过 EventBus 发布 `Bus_Event::HelpInfo`，前端订阅渲染。
    /// 无需 reply 通道。
    Help,
}

// ============================================================
// NetworkProtocol — 网络侧命令协议
// ============================================================

/// 网络侧命令协议，定义节点间通过 `DataType::Command` 传输的命令格式。
///
/// 使用简单的文本协议（`|` 分隔），便于调试和扩展。
/// 接收方通过 `Parse_Network_Command()` 解析，发送方通过 `Serialize_Network_Command()` 序列化。
///
/// ## 命令列表
/// - `ESTABLISH_TENSOR_STREAM`: 请求 Worker 向指定 target 打开一条张量流
/// - `JOIN_PIPELINE`: 通知 Worker 启动推理 Job（加载模型后回复 OK）
#[derive(Debug, Clone)]
pub enum NetworkProtocol {
    /// 请求 Worker 向指定 target 打开一条张量流（Phase 1）
    ///
    /// 格式: `ESTABLISH_TENSOR_STREAM|{inference_id}|{target_peer_id}`
    ///
    /// 处理: Worker 打开出站流 → handshake → 注册到 TensorPortSwitch
    /// 回复: `OK` 或 `FAIL|{reason}`
    Establish_Tensor_Stream {
        /// 分布式推理全局唯一 ID
        inference_id: u64,
        /// 目标节点的 PeerId（Worker 需向此节点打开出站张量流）
        target_peer_id: String,
    },

    /// 通知 Worker 启动推理 Job（Phase 2）
    ///
    /// 格式: `JOIN_PIPELINE|{inference_id}|{model_file_id}|{device}|{layer_start}|{layer_end}`
    ///
    /// 处理: Worker 取出张量流 → 加载模型 → spawn Job → 回复 OK
    /// 回复: `OK` 或 `FAIL|{reason}`（超时 300s）
    Join_Pipeline {
        /// 分布式推理全局唯一 ID（关联张量流）
        inference_id: u64,
        /// 模型文件 ID（本地已有的模型分片）
        model_file_id: String,
        /// 推理设备偏好（"cpu" / "cuda"）
        device: String,
        /// 模型层范围 — 起始层
        layer_start: usize,
        /// 模型层范围 — 结束层
        layer_end: usize,
    },

    /// Profile 请求（Phase 1）
    ///
    /// 格式: `PROFILE|{model_id}|{layer_count}|{device}`
    /// 回复: `OK|{duration_micros}` 或 `FAIL|model not found`
    Profile_Request {
        model_id: String,
        layer_count: u32,
        device: String,
    },
}

/// 反序列化: payload bytes → NetworkProtocol
///
/// # 格式（文本，`|` 分隔）
/// - `"ESTABLISH_TENSOR_STREAM|{inference_id}|{target_peer_id}"`
/// - `"JOIN_PIPELINE|{inference_id}|{model_file_id}|{device}|{layer_start}|{layer_end}"`
pub fn Parse_Network_Command(payload: &[u8]) -> Result<NetworkProtocol, String> {
    let text = std::str::from_utf8(payload).map_err(|e| format!("payload 非 UTF-8: {}", e))?;
    let parts: Vec<&str> = text.split('|').collect();

    if parts.is_empty() {
        return Err("空 payload".to_string());
    }

    match parts[0] {
        "ESTABLISH_TENSOR_STREAM" => {
            if parts.len() != 3 {
                return Err(format!(
                    "ESTABLISH_TENSOR_STREAM 格式错误: 需要 3 个字段, 实际 {}. 格式: ESTABLISH_TENSOR_STREAM|inference_id|target_peer_id",
                    parts.len()
                ));
            }
            let inference_id = parts[1]
                .parse::<u64>()
                .map_err(|e| format!("inference_id 解析失败: {}", e))?;
            let target_peer_id = parts[2].to_string();
            Ok(NetworkProtocol::Establish_Tensor_Stream {
                inference_id,
                target_peer_id,
            })
        }
        "JOIN_PIPELINE" => {
            if parts.len() != 6 {
                return Err(format!(
                    "JOIN_PIPELINE 格式错误: 需要 6 个字段, 实际 {}. 格式: JOIN_PIPELINE|inference_id|model_file_id|device|layer_start|layer_end",
                    parts.len()
                ));
            }
            let inference_id = parts[1]
                .parse::<u64>()
                .map_err(|e| format!("inference_id 解析失败: {}", e))?;
            let model_file_id = parts[2].to_string();
            let device = parts[3].to_string();
            let layer_start = parts[4]
                .parse::<usize>()
                .map_err(|e| format!("layer_start 解析失败: {}", e))?;
            let layer_end = parts[5]
                .parse::<usize>()
                .map_err(|e| format!("layer_end 解析失败: {}", e))?;
            Ok(NetworkProtocol::Join_Pipeline {
                inference_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            })
        }
        "PROFILE" => {
            if parts.len() != 4 {
                return Err(format!(
                    "PROFILE 格式错误: 需要 4 个字段, 实际 {}. 格式: PROFILE|model_id|layer_count|device",
                    parts.len()
                ));
            }
            let model_id = parts[1].to_string();
            let layer_count = parts[2]
                .parse::<u32>()
                .map_err(|e| format!("layer_count 解析失败: {}", e))?;
            let device = parts[3].to_string();
            Ok(NetworkProtocol::Profile_Request {
                model_id,
                layer_count,
                device,
            })
        }
        unknown => Err(format!("未知命令前缀: '{}'", unknown)),
    }
}

/// 序列化: NetworkProtocol → payload bytes
///
/// 供发送方（如 Coordinator 的三阶段编排逻辑）使用。
pub fn Serialize_Network_Command(cmd: &NetworkProtocol) -> Vec<u8> {
    let text = match cmd {
        NetworkProtocol::Establish_Tensor_Stream {
            inference_id,
            target_peer_id,
        } => format!(
            "ESTABLISH_TENSOR_STREAM|{}|{}",
            inference_id, target_peer_id
        ),
        NetworkProtocol::Join_Pipeline {
            inference_id,
            model_file_id,
            device,
            layer_start,
            layer_end,
        } => format!(
            "JOIN_PIPELINE|{}|{}|{}|{}|{}",
            inference_id, model_file_id, device, layer_start, layer_end
        ),
        NetworkProtocol::Profile_Request {
            model_id,
            layer_count,
            device,
        } => format!("PROFILE|{}|{}|{}", model_id, layer_count, device),
    };
    text.into_bytes()
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn test_parse_unknown_command() {
        let payload = b"UNKNOWN_CMD|abc";
        let result = Parse_Network_Command(payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未知命令前缀"));
    }

    #[test]
    fn test_parse_establish_tensor_stream_ok() {
        let payload = b"ESTABLISH_TENSOR_STREAM|12345678|12D3KooWAbCdEfGhIjKlMnOpQrStUvWxYz";
        let result = Parse_Network_Command(payload).unwrap();
        match result {
            NetworkProtocol::Establish_Tensor_Stream {
                inference_id,
                target_peer_id,
            } => {
                assert_eq!(inference_id, 12345678);
                assert_eq!(target_peer_id, "12D3KooWAbCdEfGhIjKlMnOpQrStUvWxYz");
            }
            _ => panic!("expected Establish_Tensor_Stream"),
        }
    }

    #[test]
    fn test_parse_establish_tensor_stream_wrong_field_count() {
        let payload = b"ESTABLISH_TENSOR_STREAM|123";
        let result = Parse_Network_Command(payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("需要 3 个字段"));
    }

    #[test]
    fn test_serialize_roundtrip_establish_tensor_stream() {
        let cmd = NetworkProtocol::Establish_Tensor_Stream {
            inference_id: 9999,
            target_peer_id: "12D3KooWPeerB".to_string(),
        };
        let bytes = Serialize_Network_Command(&cmd);
        let parsed = Parse_Network_Command(&bytes).unwrap();
        match parsed {
            NetworkProtocol::Establish_Tensor_Stream {
                inference_id,
                target_peer_id,
            } => {
                assert_eq!(inference_id, 9999);
                assert_eq!(target_peer_id, "12D3KooWPeerB");
            }
            _ => panic!("expected Establish_Tensor_Stream"),
        }
    }

    #[test]
    fn test_parse_join_pipeline_ok() {
        let payload = b"JOIN_PIPELINE|55555|model_shard_10_19|cuda|10|19";
        let result = Parse_Network_Command(payload).unwrap();
        match result {
            NetworkProtocol::Join_Pipeline {
                inference_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            } => {
                assert_eq!(inference_id, 55555);
                assert_eq!(model_file_id, "model_shard_10_19");
                assert_eq!(device, "cuda");
                assert_eq!(layer_start, 10);
                assert_eq!(layer_end, 19);
            }
            _ => panic!("expected Join_Pipeline"),
        }
    }

    #[test]
    fn test_parse_join_pipeline_wrong_field_count() {
        let payload = b"JOIN_PIPELINE|123|model";
        let result = Parse_Network_Command(payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("需要 6 个字段"));
    }

    #[test]
    fn test_serialize_roundtrip_join_pipeline() {
        let cmd = NetworkProtocol::Join_Pipeline {
            inference_id: 77777,
            model_file_id: "shard_20_29".to_string(),
            device: "cpu".to_string(),
            layer_start: 20,
            layer_end: 29,
        };
        let bytes = Serialize_Network_Command(&cmd);
        let parsed = Parse_Network_Command(&bytes).unwrap();
        match parsed {
            NetworkProtocol::Join_Pipeline {
                inference_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            } => {
                assert_eq!(inference_id, 77777);
                assert_eq!(model_file_id, "shard_20_29");
                assert_eq!(device, "cpu");
                assert_eq!(layer_start, 20);
                assert_eq!(layer_end, 29);
            }
            _ => panic!("expected Join_Pipeline"),
        }
    }
}
