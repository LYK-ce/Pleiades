//Presented by KeJi
//Date ： 2026-04-07

//! Command 显示区
//!
//! 位于 Job 面板下方，按需弹出。
//! 用于显示 run 命令的推理输出、性能指标等。
//! 当 `app.command_output` 为 Some 时才渲染。

#![allow(non_snake_case)]

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::app::App;

/// 渲染 Command 面板
///
/// 显示推理输出文本和性能指标。
///
/// # 参数
/// - `frame`: ratatui 帧
/// - `area`: 分配给此面板的区域
/// - `app`: 应用状态引用
pub fn Render(frame: &mut Frame, area: Rect, app: &App) {
    let command_output = &app.command_output;

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 💬 Command ")
        .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::DarkGray));

    // 将面板内部分为两部分：输出文本 + 底部状态行
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Min(1),       // 输出文本
        Constraint::Length(1),    // 状态行
    ])
    .split(inner);

    // 渲染输出文本
    let output_text = if command_output.output_text.is_empty() {
        "等待推理输出...".to_string()
    } else if !command_output.completed {
        // 推理进行中，末尾加光标
        format!("{}█", command_output.output_text)
    } else {
        command_output.output_text.clone()
    };

    let text_style = Style::default().fg(Color::White);
    let paragraph = Paragraph::new(output_text)
        .style(text_style)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, chunks[0]);

    // 渲染状态行
    let status_line = if command_output.completed {
        Line::from(vec![
            Span::styled("✓ ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{} tok", command_output.token_count),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1} tok/s", command_output.tok_per_sec),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("总耗时 {:.1}s", command_output.total_secs),
                Style::default().fg(Color::Cyan),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                format!("{} tok", command_output.token_count),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1} tok/s", command_output.tok_per_sec),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}s", command_output.total_secs),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };

    let status = Paragraph::new(status_line);
    frame.render_widget(status, chunks[1]);
}
