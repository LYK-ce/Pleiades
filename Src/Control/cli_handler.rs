//Presented by KeJi
//Date ： 2026-04-17

//! CLI 命令处理器模块
//!
//! 专门处理 CLI 命令，包含 Handle_CLI_Command 函数，导出 CLI_Handler 结构体。
//! 从 control.rs 中提取 CLI 相关逻辑，减少代码耦合。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{info, warn};
use libp2p::PeerId;

use super::cli_command::CLI_Command;
use super::ui_message::Ui_Message;
use crate::config::Update_Config;
use crate::network::node_handle::{InboundRequest, NodeHandle};
use crate::network::network_service::NetworkEvent;
use crate::peer_management::PeerHandle;

// ============================================================
// CLI_Action — 命令处理后的控制流指示
// ============================================================

/// CLI 命令处理后的控制流动作
#[derive(Debug, Clone, PartialEq)]
pub enum CLI_Action {
    /// 继续事件循环
    Continue,
    /// 退出事件循环
    Break,
}

// ============================================================
// CLI_Handler 结构体
// ============================================================

/// CLI 命令处理器
///
/// 封装所有 CLI 命令的处理逻辑，提供统一的 Handle_CLI_Command 接口。
pub struct CLI_Handler;

impl CLI_Handler {
    /// 处理 CLI 命令的主函数
    ///
    /// 接收 CLI_Command（取得所有权），处理命令逻辑，
    /// 内部完成 reply 通道的回复，返回 CLI_Action 指示控制流。
    pub async fn Handle_CLI_Command(
        cmd: CLI_Command,
        state: &mut super::control::Node_State,
        next_peer: &mut Option<PeerId>,
        node_handle: &NodeHandle,
        peer_handle: &PeerHandle,
        device: &mut String,
        config_path: &PathBuf,
        cli_rx: &mut mpsc::Receiver<CLI_Command>,
        inbound_rx: &mut mpsc::Receiver<InboundRequest>,
        event_rx: &mut mpsc::Receiver<NetworkEvent>,
        ui_tx: &mpsc::Sender<Ui_Message>,
    ) -> CLI_Action {
        match cmd {
            CLI_Command::Run { model_path: _, reply } => {
                // Run 命令由 Control_Loop 状态机直接处理，不经过 CLI_Handler
                warn!("CLI_Handler: Run 命令应由 Control_Loop 直接处理");
                let _ = reply.send(Err("Run 命令应由 Control_Loop 直接处理".to_string()));
                CLI_Action::Continue
            }
            CLI_Command::Input { prompt: _, reply } => {
                // 在主循环中收到 Input → 没有活跃的 Session
                let _ = reply.send(Err("没有活跃的推理会话，请先执行 run 命令".to_string()));
                CLI_Action::Continue
            }
            CLI_Command::SetDevice { device: new_device, reply } => {
                info!("CLI_Handler: 处理 set-device 命令: {}", new_device);
                let result = Self::Handle_Set_Device(device, config_path, &new_device, ui_tx).await;
                let _ = reply.send(result);
                CLI_Action::Continue
            }
            CLI_Command::Quit => {
                info!("CLI_Handler: 处理退出命令，正在关闭...");
                if let Err(e) = node_handle.Stop().await {
                    warn!("网络节点关闭失败: {}", e);
                }
                CLI_Action::Break
            }
            CLI_Command::DisplayPeer { reply } => {
                info!("CLI_Handler: 处理 display-peer 命令");
                let result = Self::Handle_Display_Peer(peer_handle, ui_tx).await;
                let _ = reply.send(result);
                CLI_Action::Continue
            }
        }
    }

    // ============================================================
    // 辅助函数：发送 UI 消息
    // ============================================================

    /// 发送 UI 消息的辅助函数（async 版本）
    async fn Send_Ui(ui_tx: &mpsc::Sender<Ui_Message>, msg: Ui_Message) {
        let _ = ui_tx.send(msg).await;
    }

    // ============================================================
    // Handle_Set_Device — 设备切换处理
    // ============================================================

    /// 处理 set-device 命令
    ///
    /// 1. 修改 Control 层持有的 device 变量
    /// 2. 将修改写回 config.toml 文件（持久化，下次启动使用新设置）
    /// 3. 通知 TUI 更新设备显示
    async fn Handle_Set_Device(
        device: &mut String,
        config_path: &Path,
        new_device: &str,
        ui_tx: &mpsc::Sender<Ui_Message>,
    ) -> Result<String, String> {
        let old_device = device.clone();

        // 更新 Control 层持有的 device
        *device = new_device.to_string();
        info!("设备已切换: {} → {}", old_device, new_device);

        // 写回 config.toml
        if let Err(e) = Update_Config(config_path, "Runtime", "device", new_device) {
            warn!("配置文件写入失败: {} (设备已切换但未持久化)", e);
            Self::Send_Ui(ui_tx, Ui_Message::Log(format!(
                "⚠ 配置文件写入失败: {} (设备已切换但未持久化)", e
            ))).await;
        } else {
            info!("配置文件已更新: device = {}", new_device);
        }

        // 通知 TUI 更新设备显示
        Self::Send_Ui(ui_tx, Ui_Message::Device_Change(new_device.to_string())).await;

        Ok(format!(
            "设备已切换: {} → {}\n配置已保存，下次启动将默认使用 {}",
            old_device.to_uppercase(),
            new_device.to_uppercase(),
            new_device.to_uppercase()
        ))
    }

    // ============================================================
    // Handle_Display_Peer — 显示节点信息
    // ============================================================

    /// 处理 display-peer 命令，显示所有节点信息
    async fn Handle_Display_Peer(
        peer_handle: &PeerHandle,
        ui_tx: &mpsc::Sender<Ui_Message>,
    ) -> Result<String, String> {
        info!("正在获取节点信息...");
        
        // 获取所有节点
        let peers = match peer_handle.list_peers().await {
            Ok(peers) => peers,
            Err(e) => {
                let error_msg = format!("获取节点列表失败: {}", e);
                warn!("{}", error_msg);
                return Err(error_msg);
            }
        };
        
        if peers.is_empty() {
            let msg = "当前没有连接的节点".to_string();
            Self::Send_Ui(ui_tx, Ui_Message::Log(msg.clone())).await;
            return Ok(msg);
        }
        
        // 构建显示信息
        let mut output = String::new();
        output.push_str("当前连接的节点:\n");
        output.push_str("================\n");
        
        for (i, peer) in peers.iter().enumerate() {
            // 使用Debug格式化显示节点信息
            output.push_str(&format!("{}. {:?}\n", i + 1, peer));
        }
        
        Self::Send_Ui(ui_tx, Ui_Message::Log(output.clone())).await;
        Ok(output)
    }
}
