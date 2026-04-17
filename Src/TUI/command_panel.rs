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
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarState, Wrap},
};

use super::app::App;

/// 估算文本在指定宽度下的视觉行数（考虑自动换行）
///
/// 每个逻辑行按字符数 / 可用宽度 向上取整，空行计为 1 行。
fn Estimate_Visual_Line_Count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let mut count = 0usize;
    let mut has_lines = false;
    for line in text.lines() {
        has_lines = true;
        let char_count = line.chars().count();
        if char_count == 0 {
            count += 1;
        } else {
            count += (char_count + width - 1) / width;
        }
    }
    // 空字符串的 .lines() 返回空迭代器，至少算 1 行
    if !has_lines {
        count = 1;
    }
    count
}

/// 渲染 Command 面板
///
/// 显示推理输出文本和性能指标。
/// 使用 Paragraph::scroll() 实现按视觉行滚动，与 Wrap 正确配合。
///
/// # 参数
/// - `frame`: ratatui 帧
/// - `area`: 分配给此面板的区域
/// - `app`: 应用状态（可变引用，用于钳位 command_scroll）
pub fn Render(frame: &mut Frame, area: Rect, app: &mut App) {
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

    // 构建完整文本（不做手动切片）
    let full_text: String = if command_output.output_text.is_empty() {
        "等待推理输出...".to_string()
    } else if !command_output.completed {
        // 推理进行中，末尾加光标
        format!("{}█", command_output.output_text)
    } else {
        command_output.output_text.clone()
    };

    // 计算视觉行数（考虑 Wrap），用于滚动条和边界钳位
    // 减 1 为滚动条预留空间
    let text_area_width = chunks[0].width.saturating_sub(1) as usize;
    let total_visual_lines = Estimate_Visual_Line_Count(&full_text, text_area_width);
    let visible_height = chunks[0].height as usize;

    // 钳位 command_scroll，防止超出有效范围
    let max_scroll = total_visual_lines.saturating_sub(visible_height);
    app.command_scroll = app.command_scroll.min(max_scroll);

    let text_style = Style::default().fg(Color::White);
    let paragraph = Paragraph::new(full_text.as_str())
        .style(text_style)
        .wrap(Wrap { trim: false })
        .scroll((app.command_scroll as u16, 0));

    frame.render_widget(paragraph, chunks[0]);

    // 渲染滚动条（基于视觉行数）
    if total_visual_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(max_scroll)
            .position(app.command_scroll)
            .viewport_content_length(visible_height);
        let scrollbar = Scrollbar::new(ratatui::widgets::ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(
            scrollbar,
            chunks[0],
            &mut scrollbar_state,
        );
    }

    // 渲染状态行
    let status_line = if app.command_output.completed {
        Line::from(vec![
            Span::styled("✓ ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{} tok", app.command_output.token_count),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1} tok/s", app.command_output.tok_per_sec),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("总耗时 {:.1}s", app.command_output.total_secs),
                Style::default().fg(Color::Cyan),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                format!("{} tok", app.command_output.token_count),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1} tok/s", app.command_output.tok_per_sec),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}s", app.command_output.total_secs),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };

    let status = Paragraph::new(status_line);
    frame.render_widget(status, chunks[1]);
}
