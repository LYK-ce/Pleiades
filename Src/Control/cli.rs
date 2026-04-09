//Presented by KeJi
//Date ： 2026-04-09

//! CLI 模块 - 用户命令行交互
//!
//! 在独立的阻塞线程中运行，读取用户输入并通过 channel 发送给 Control 层。
//! MVP 阶段只支持一个命令: `run <model_path> <prompt>`
//!
//! ## 运行方式
//! 使用 `tokio::task::spawn_blocking` 在独立线程中运行，
//! 内部使用 `blocking_send` / `blocking_recv` 与 Control 通信。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tracing::info;

// ============================================================
// CLI 命令枚举
// ============================================================

/// CLI 命令（从 CLI 线程发送给 Control 层）
pub enum CLI_Command {
    /// 运行推理任务
    Run {
        /// 模型文件路径
        model_path: PathBuf,
        /// 推理 prompt
        prompt: String,
        /// 结果回传通道
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// 设置设备 (cpu/cuda)
    SetDevice {
        /// 设备名称 ("cpu" 或 "cuda")
        device: String,
        /// 结果回传通道
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// 退出程序
    Quit,
}

// ============================================================
// CLI 主循环
// ============================================================

/// 启动 CLI 循环（在 spawn_blocking 中调用）
///
/// 阻塞读取 stdin，解析用户命令，通过 cli_tx 发送给 Control 层，
/// 然后通过 oneshot 同步等待结果并打印。
///
/// # 参数
/// - `cli_tx`: 命令发送通道（发给 Control 层的 select! 循环）
pub fn CLI_Loop(cli_tx: mpsc::Sender<CLI_Command>) {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut buf = String::new();

    println!();
    println!("=========================================");
    println!("  Pleiades - 分布式推理运行时框架");
    println!("  命令: run <model_path> <prompt>");
    println!("  退出: quit");
    println!("=========================================");
    println!();

    loop {
        // 打印提示符
        print!("pleiades> ");
        if io::stdout().flush().is_err() {
            break;
        }

        // 读取一行输入
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => {
                // EOF (Ctrl+D)
                println!();
                info!("CLI: 收到 EOF，退出");
                let _ = cli_tx.blocking_send(CLI_Command::Quit);
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[错误] 读取输入失败: {}", e);
                break;
            }
        }

        let input = buf.trim();
        if input.is_empty() {
            continue;
        }

        // 解析命令
        if input == "quit" || input == "exit" {
            info!("CLI: 用户输入退出命令");
            let _ = cli_tx.blocking_send(CLI_Command::Quit);
            break;
        }

        if input.starts_with("run ") {
            match Parse_Run_Command(input) {
                Ok((model_path, prompt)) => {
                    let (reply_tx, reply_rx) = oneshot::channel();

                    // 发送命令给 Control 层
                    if cli_tx
                        .blocking_send(CLI_Command::Run {
                            model_path,
                            prompt: prompt.clone(),
                            reply: reply_tx,
                        })
                        .is_err()
                    {
                        eprintln!("[错误] Control 层已关闭");
                        break;
                    }

                    // 同步等待结果
                    match reply_rx.blocking_recv() {
                        Ok(Ok(msg)) => {
                            println!("{}", msg);
                        }
                        Ok(Err(e)) => {
                            eprintln!("[错误] {}", e);
                        }
                        Err(_) => {
                            eprintln!("[错误] Control 层未响应");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[错误] {}", e);
                    eprintln!("用法: run <model_path> <prompt>");
                }
            }
        } else {
            eprintln!("[错误] 未知命令: '{}'", input);
            eprintln!("可用命令:");
            eprintln!("  run <model_path> <prompt>  - 运行分布式推理");
            eprintln!("  quit                       - 退出程序");
        }
    }

    info!("CLI 循环退出");
}

// ============================================================
// 命令解析
// ============================================================

/// 解析 run 命令
///
/// 格式: `run <model_path> <prompt>`
///
/// prompt 支持两种格式:
/// - 带引号: `run model.gguf "你好世界"`
/// - 不带引号: `run model.gguf 你好世界` (model_path 后的所有内容视为 prompt)
///
/// # 返回
/// (model_path, prompt)
fn Parse_Run_Command(input: &str) -> Result<(PathBuf, String), String> {
    // 去掉 "run " 前缀
    let rest = input.strip_prefix("run ").unwrap_or("").trim();

    if rest.is_empty() {
        return Err("缺少参数: model_path 和 prompt".to_string());
    }

    // 查找 model_path 和 prompt 的分割点
    // model_path 是第一个空格之前的部分
    let (model_path_str, prompt_str) = if let Some(space_idx) = rest.find(' ') {
        let path = &rest[..space_idx];
        let prompt = rest[space_idx + 1..].trim();
        (path, prompt)
    } else {
        return Err("缺少参数: prompt".to_string());
    };

    if prompt_str.is_empty() {
        return Err("缺少参数: prompt".to_string());
    }

    // 去掉 prompt 两端的引号（如果有）
    let prompt = if (prompt_str.starts_with('"') && prompt_str.ends_with('"'))
        || (prompt_str.starts_with('\'') && prompt_str.ends_with('\''))
    {
        prompt_str[1..prompt_str.len() - 1].to_string()
    } else {
        prompt_str.to_string()
    };

    let model_path = PathBuf::from(model_path_str);

    Ok((model_path, prompt))
}
