//Presented by KeJi
//Created Date ： 2026-08-15
//Modified Date ： 2026-08-15

//! 命令入站消费者（Task 16）
//!
//! 订阅车端命令通道（request-response `DataType::Robot` 命令帧），
//! `decode_frame` → `parse_orion_frame` → `Command` → `robot_cmd_tx` → `main_loop`。
//! 与 `cluster_consumer`（遥测帧 → 表/地图）对称。

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::command::Command;
use super::protocol::{decode_frame, parse_orion_frame};

/// 入站命令消费 task（`Robot::launch` 中 spawn）
pub async fn command_consumer(
    mut rx: mpsc::Receiver<Vec<u8>>,
    robot_cmd_tx: mpsc::Sender<Command>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            frame = rx.recv() => {
                match frame {
                    Some(bytes) => {
                        match decode_frame(&bytes) {
                            Some(f) => match parse_orion_frame(&f) {
                                Some(cmd) => {
                                    if let Err(e) = robot_cmd_tx.send(cmd).await {
                                        warn!("[Cmd] 命令发送失败: {e}");
                                    }
                                }
                                None => warn!("[Cmd] 命令帧无法解析为 Command"),
                            },
                            None => warn!("[Cmd] 收到无法解析的 ORION 帧 ({} 字节)", bytes.len()),
                        }
                    }
                    None => break, // 通道关闭，退出
                }
            }
            _ = cancel.cancelled() => {
                debug!("command_consumer 退出");
                return;
            }
        }
    }
}
