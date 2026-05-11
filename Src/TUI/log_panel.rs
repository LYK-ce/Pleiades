//Presented by KeJi
//Date ： 2026-04-30

//! Log 显示区
//!
//! 位于左上角，占 70% 宽度。显示带时间戳的滚动日志列表。
//! 支持 ↑↓ 键滚动查看历史日志。文本自动换行。

#![allow(non_snake_case)]

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::App;

/// 渲染 Log 面板
///
/// 显示日志列表，支持滚动和自动换行。最新日志在底部。
/// 使用 Paragraph + Wrap 实现长文本自动换行（替代 List 的截断行为）。
///
/// # 参数
/// - `frame`: ratatui 帧
/// - `area`: 分配给此面板的区域
/// - `app`: 应用状态引用
pub fn Render(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 📋 Log ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::DarkGray));

    // 构建带样式的行
    let lines: Vec<Line> = app
        .logs
        .iter()
        .map(|log| {
            let style = if log.contains("[错误]") || log.contains("ERROR") || log.contains("失败")
            {
                Style::default().fg(Color::Red)
            } else if log.contains("✓") || log.contains("完成") {
                Style::default().fg(Color::Green)
            } else if log.contains("警告") || log.contains("WARN") {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(Span::styled(log.as_str(), style))
        })
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.log_scroll as u16, 0));

    frame.render_widget(paragraph, area);
}
