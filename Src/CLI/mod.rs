//Presented by KeJi
//Date ： 2026-06-10

//! CLI 模式：EventBus → stdout + stdin REPL

use std::io::{self, BufRead, Write};
use tokio::sync::mpsc;

use crate::event_bus::{EventBus, Bus_Event, NotifyLevel};
use crate::orchestrator::command::UserCommand;
use crate::tui::parse_user_command;

/// 启动 EventBus → stdout 订阅任务
pub fn spawn_stdout_subscriber(event_bus: &EventBus) {
    let mut notify_rx = event_bus.Subscribe();
    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(Bus_Event::Notify { level, message }) => {
                    match level {
                        NotifyLevel::Info  => println!("{message}"),
                        NotifyLevel::Warn  => eprintln!("[WARN] {message}"),
                        NotifyLevel::Error => eprintln!("[ERROR] {message}"),
                    }
                }
                Ok(Bus_Event::Output { payload }) => {
                    println!("{payload}");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[CLI] 丢失 {} 条事件", n);
                }
                _ => {}
            }
        }
    });
}

/// 启动 stdin REPL 线程，返回 JoinHandle
pub fn spawn_stdin_repl(cmd_tx: mpsc::Sender<UserCommand>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        println!("Pleiades CLI. 输入 'quit' 退出, 'help' 查看命令.");
        loop {
            print!("> ");
            let _ = stdout.flush();
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => {
                    // EOF
                    let (reply_tx, _) = tokio::sync::oneshot::channel();
                    let _ = cmd_tx.blocking_send(UserCommand::Quit { reply: reply_tx });
                    break;
                }
                Err(_) => break,
                Ok(_) => {}
            }
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            // quit/exit 需构造 oneshot（parse_user_command 不处理）
            if line == "quit" || line == "exit" {
                let (reply_tx, _) = tokio::sync::oneshot::channel();
                let _ = cmd_tx.blocking_send(UserCommand::Quit { reply: reply_tx });
                break;
            }
            match parse_user_command(&line) {
                Ok(Some(cmd)) => {
                    if cmd_tx.blocking_send(cmd).is_err() {
                        eprintln!("[CLI] Orchestrator 已关闭");
                        break;
                    }
                }
                Ok(None) => {}
                Err(msg) => eprintln!("{msg}"),
            }
        }
    })
}
