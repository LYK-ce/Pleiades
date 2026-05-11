//Presented by KeJi
//Date ： 2026-04-09

//! Job 显示区
//!
//! 位于中间，固定 3 行高度，始终可见。
//! 根据 Job_State 显示不同内容：
//! - Idle: 显示 "空闲"
//! - File_Transfer: 显示文件传输进度条（Gauge）
//! - Inference: 显示模型信息和阶段（纯文本）

#![allow(non_snake_case)]

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use super::app::{App, Job_State, Transfer_Direction};

/// 渲染 Job 面板
///
/// 根据当前 Job 状态渲染不同内容。
///
/// # 参数
/// - `frame`: ratatui 帧
/// - `area`: 分配给此面板的区域
/// - `app`: 应用状态引用
pub fn Render(frame: &mut Frame, area: Rect, app: &App) {
    match &app.job {
        Job_State::Idle => {
            Render_Idle(frame, area, &app.device);
        }
        Job_State::File_Transfer {
            direction,
            file_name,
            peer,
            sent,
            total,
        } => {
            Render_File_Transfer(frame, area, direction, file_name, peer, *sent, *total);
        }
        Job_State::Inference {
            model_name,
            device_count,
            layer_range,
            phase,
        } => {
            Render_Inference(
                frame,
                area,
                model_name,
                *device_count,
                layer_range,
                phase,
                &app.device,
            );
        }
    }
}

/// 渲染空闲状态（显示设备信息）
fn Render_Idle(frame: &mut Frame, area: Rect, device: &str) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ⚙️ Job ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::DarkGray));

    let device_color = if device == "cuda" {
        Color::Green
    } else {
        Color::Cyan
    };
    let text = Paragraph::new(Line::from(vec![
        Span::styled(
            "  状态: 空闲 │ 设备: ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            device.to_uppercase(),
            Style::default()
                .fg(device_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(block);

    frame.render_widget(text, area);
}

/// 渲染文件传输状态（带进度条）
fn Render_File_Transfer(
    frame: &mut Frame,
    area: Rect,
    direction: &Transfer_Direction,
    file_name: &str,
    peer: &str,
    sent: u64,
    total: u64,
) {
    let icon = match direction {
        Transfer_Direction::Send => "📤 发送",
        Transfer_Direction::Receive => "📥 接收",
    };

    let peer_short = if peer.len() > 12 {
        format!("{}...{}", &peer[..8], &peer[peer.len() - 4..])
    } else {
        peer.to_string()
    };

    let pct = if total > 0 {
        ((sent as f64 / total as f64) * 100.0) as u16
    } else {
        0
    };

    let mb_sent = sent as f64 / 1_048_576.0;
    let mb_total = total as f64 / 1_048_576.0;

    let title = format!(" ⚙️ Job: {} {} → {} ", icon, file_name, peer_short);

    let label = format!("{:.1}/{:.1} MB ({}%)", mb_sent, mb_total, pct);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .percent(pct)
        .label(label);

    frame.render_widget(gauge, area);
}

/// 渲染推理状态（纯文本，无进度条，显示设备信息）
fn Render_Inference(
    frame: &mut Frame,
    area: Rect,
    model_name: &str,
    device_count: usize,
    layer_range: &str,
    phase: &str,
    device: &str,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ⚙️ Job ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::DarkGray));

    let device_upper = device.to_uppercase();
    let info = if device_count > 1 {
        format!(
            "  {} │ {}台设备 │ {} │ {} │ 阶段: {}",
            model_name, device_count, layer_range, device_upper, phase
        )
    } else {
        format!(
            "  {} │ {} │ {} │ 阶段: {}",
            model_name, layer_range, device_upper, phase
        )
    };

    let text = Paragraph::new(info)
        .block(block)
        .style(Style::default().fg(Color::Yellow));

    frame.render_widget(text, area);
}
