//Presented by KeJi
//Date ： 2026-04-07

//! Log 显示区
//!
//! 位于左上角，占 70% 宽度。显示带时间戳的滚动日志列表。
//! 支持 ↑↓ 键滚动查看历史日志。

#![allow(non_snake_case)]

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};

use super::app::App;

/// 渲染 Log 面板
///
/// 显示日志列表，支持滚动。最新日志在底部。
///
/// # 参数
/// - `frame`: ratatui 帧
/// - `area`: 分配给此面板的区域
/// - `app`: 应用状态引用
pub fn Render(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 📋 Log ")
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::DarkGray));

    // 计算可见区域高度（减去边框 2 行）
    let inner_height = area.height.saturating_sub(2) as usize;

    // 计算显示范围
    let total = app.logs.len();
    let start = if total > inner_height {
        // 如果日志数量超过可见高度，使用 scroll 偏移
        let max_scroll = total - inner_height;
        app.log_scroll.min(max_scroll)
    } else {
        0
    };
    let end = (start + inner_height).min(total);

    // 构建列表项
    let items: Vec<ListItem> = app.logs[start..end]
        .iter()
        .map(|log| {
            let style = if log.contains("[错误]") || log.contains("ERROR") || log.contains("失败") {
                Style::default().fg(Color::Red)
            } else if log.contains("✓") || log.contains("完成") {
                Style::default().fg(Color::Green)
            } else if log.contains("警告") || log.contains("WARN") {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(Line::from(Span::styled(log.as_str(), style)))
        })
        .collect();

    let list = List::new(items).block(block);

    frame.render_widget(list, area);
}
