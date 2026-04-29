//Presented by KeJi
//Date ： 2026-04-29

use tokio::sync::oneshot;
use super::job::JobId;

// ============================================================
// UserCommand — 用户侧命令
// ============================================================

/// 用户命令，来自 CLI / TUI / GUI 等前端接入层。
///
/// 每个变体携带 `reply` 通道，Core 处理完命令后通过 reply 回传结果。
/// 前端 `await` 回复即可获得处理结果（非 fire-and-forget）。
pub enum UserCommand {
    /// 启动本地推理作业
    ///
    /// 回复：`Ok(JobId)` 成功分配的 Job ID；`Err(String)` 编译或分配失败原因
    Run {
        model_path: String,
        reply: oneshot::Sender<Result<JobId, String>>,
    },
    /// 取消指定作业
    ///
    /// 回复：`Ok(())` 已发送取消信号；`Err(String)` Job 不存在
    Cancel {
        job_id: JobId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// 请求优雅退出
    ///
    /// 回复：`()` 确认已进入关闭流程
    Quit {
        reply: oneshot::Sender<()>,
    },
    /// 查询当前节点列表
    ///
    /// 回复：`Ok(Vec<String>)` 节点列表；`Err(String)` 查询失败
    DisplayPeer {
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    /// 修改默认计算设备偏好
    ///
    /// 回复：`Ok(())` 设置成功；`Err(String)` 设置失败
    SetDevice {
        device: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// 分发模型分片到远端节点
    ///
    /// 对模型执行 AnalyzeModel → 对每个 peer 执行 SplitModel + SendFile。
    /// `peers` 中每个元素为 `(peer_id, layer_start, layer_end)`，
    /// 分片范围由用户指定（而非自动均分），因为用户了解各节点的计算能力。
    ///
    /// 回复：`Ok(JobId)` 成功分配的 Job ID；`Err(String)` 编译或分配失败原因
    DistributeModel {
        model_path: String,
        peers: Vec<(String, usize, usize)>,
        reply: oneshot::Sender<Result<JobId, String>>,
    },
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
/// - `REQUEST_PIPELINE`: 远端 Coordinator 请求本节点作为 Worker 加入流水线
/// - `VERIFY_FILE`: 文件传输阶段3，发送方校验接收方是否成功接收文件
#[derive(Debug, Clone)]
pub enum NetworkProtocol {
    /// 请求加入流水线（远端 Coordinator → 本节点）
    ///
    /// 格式: `REQUEST_PIPELINE|{coordinator_job_id}|{model_file_id}|{device}|{layer_start}|{layer_end}`
    ///
    /// 处理: Core 调用 `route_pipeline_flow()` 编译 + spawn Relay Job
    /// 回复: `OK|{relay_job_id}` 或 `REJECT|{reason}`
    Request_Pipeline {
        /// Coordinator 的 Job ID（用于 Relay 的 OpenTensorStream handshake）
        coordinator_job_id: u64,
        /// 模型文件 ID（本地已有的模型分片）
        model_file_id: String,
        /// 推理设备偏好（"cpu" / "cuda"）
        device: String,
        /// 模型层范围 — 起始层
        layer_start: usize,
        /// 模型层范围 — 结束层
        layer_end: usize,
    },
    /// 校验文件是否已成功接收
    ///
    /// 格式: `VERIFY_FILE|{file_name}`
    ///
    /// 处理: Core 查 StorageManager 确认文件存在
    /// 回复: `confirmed` 或 `failed|{reason}`
    Verify_File {
        /// 待校验的文件名（对应 Storage 中的 file_id）
        file_name: String,
    },
}

/// 反序列化: payload bytes → NetworkProtocol
///
/// # 格式（文本，`|` 分隔）
/// - `"REQUEST_PIPELINE|{coordinator_job_id}|{model_file_id}|{device}|{layer_start}|{layer_end}"`
/// - `"VERIFY_FILE|{file_name}"`
pub fn Parse_Network_Command(payload: &[u8]) -> Result<NetworkProtocol, String> {
    let text = std::str::from_utf8(payload)
        .map_err(|e| format!("payload 非 UTF-8: {}", e))?;
    let parts: Vec<&str> = text.split('|').collect();

    if parts.is_empty() {
        return Err("空 payload".to_string());
    }

    match parts[0] {
        "REQUEST_PIPELINE" => {
            if parts.len() != 6 {
                return Err(format!(
                    "REQUEST_PIPELINE 格式错误: 需要 6 个字段, 实际 {}. 格式: REQUEST_PIPELINE|coordinator_job_id|model_file_id|device|layer_start|layer_end",
                    parts.len()
                ));
            }
            let coordinator_job_id = parts[1].parse::<u64>()
                .map_err(|e| format!("coordinator_job_id 解析失败: {}", e))?;
            let model_file_id = parts[2].to_string();
            let device = parts[3].to_string();
            let layer_start = parts[4].parse::<usize>()
                .map_err(|e| format!("layer_start 解析失败: {}", e))?;
            let layer_end = parts[5].parse::<usize>()
                .map_err(|e| format!("layer_end 解析失败: {}", e))?;
            Ok(NetworkProtocol::Request_Pipeline {
                coordinator_job_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            })
        }
        "VERIFY_FILE" => {
            if parts.len() != 2 {
                return Err(format!(
                    "VERIFY_FILE 格式错误: 需要 2 个字段, 实际 {}. 格式: VERIFY_FILE|file_name",
                    parts.len()
                ));
            }
            Ok(NetworkProtocol::Verify_File {
                file_name: parts[1].to_string(),
            })
        }
        unknown => Err(format!("未知命令前缀: '{}'", unknown)),
    }
}

/// 序列化: NetworkProtocol → payload bytes
///
/// 供发送方（如 Coordinator Job 的 RequestPipeline 指令）使用。
pub fn Serialize_Network_Command(cmd: &NetworkProtocol) -> Vec<u8> {
    let text = match cmd {
        NetworkProtocol::Request_Pipeline {
            coordinator_job_id,
            model_file_id,
            device,
            layer_start,
            layer_end,
        } => format!(
            "REQUEST_PIPELINE|{}|{}|{}|{}|{}",
            coordinator_job_id, model_file_id, device, layer_start, layer_end
        ),
        NetworkProtocol::Verify_File { file_name } => {
            format!("VERIFY_FILE|{}", file_name)
        }
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
    fn test_parse_request_pipeline_ok() {
        let payload = b"REQUEST_PIPELINE|42|model_shard_0_15|cuda|0|15";
        let result = Parse_Network_Command(payload).unwrap();
        match result {
            NetworkProtocol::Request_Pipeline {
                coordinator_job_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            } => {
                assert_eq!(coordinator_job_id, 42);
                assert_eq!(model_file_id, "model_shard_0_15");
                assert_eq!(device, "cuda");
                assert_eq!(layer_start, 0);
                assert_eq!(layer_end, 15);
            }
            _ => panic!("expected Request_Pipeline"),
        }
    }

    #[test]
    fn test_parse_request_pipeline_wrong_field_count() {
        let payload = b"REQUEST_PIPELINE|42|model";
        let result = Parse_Network_Command(payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("需要 6 个字段"));
    }

    #[test]
    fn test_parse_verify_file_ok() {
        let payload = b"VERIFY_FILE|model_shard_0_15.gguf";
        let result = Parse_Network_Command(payload).unwrap();
        match result {
            NetworkProtocol::Verify_File { file_name } => {
                assert_eq!(file_name, "model_shard_0_15.gguf");
            }
            _ => panic!("expected Verify_File"),
        }
    }

    #[test]
    fn test_parse_verify_file_wrong_field_count() {
        let payload = b"VERIFY_FILE";
        let result = Parse_Network_Command(payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("需要 2 个字段"));
    }

    #[test]
    fn test_parse_unknown_command() {
        let payload = b"UNKNOWN_CMD|abc";
        let result = Parse_Network_Command(payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未知命令前缀"));
    }

    #[test]
    fn test_serialize_roundtrip_request_pipeline() {
        let cmd = NetworkProtocol::Request_Pipeline {
            coordinator_job_id: 99,
            model_file_id: "shard_a".to_string(),
            device: "cpu".to_string(),
            layer_start: 5,
            layer_end: 20,
        };
        let bytes = Serialize_Network_Command(&cmd);
        let parsed = Parse_Network_Command(&bytes).unwrap();
        match parsed {
            NetworkProtocol::Request_Pipeline {
                coordinator_job_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            } => {
                assert_eq!(coordinator_job_id, 99);
                assert_eq!(model_file_id, "shard_a");
                assert_eq!(device, "cpu");
                assert_eq!(layer_start, 5);
                assert_eq!(layer_end, 20);
            }
            _ => panic!("expected Request_Pipeline"),
        }
    }

    #[test]
    fn test_serialize_roundtrip_verify_file() {
        let cmd = NetworkProtocol::Verify_File {
            file_name: "test_file.gguf".to_string(),
        };
        let bytes = Serialize_Network_Command(&cmd);
        let parsed = Parse_Network_Command(&bytes).unwrap();
        match parsed {
            NetworkProtocol::Verify_File { file_name } => {
                assert_eq!(file_name, "test_file.gguf");
            }
            _ => panic!("expected Verify_File"),
        }
    }
}
