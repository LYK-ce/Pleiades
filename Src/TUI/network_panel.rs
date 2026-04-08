//Presented by KeJi
//Date ： 2026-04-07

//! Network 显示区
//!
//! 位于右上角，占 30% 宽度。显示已知节点列表及其连接状态。
//! 已连接节点用绿色圆点标记，已断开节点用红色圆点标记。

#![allow(non_snake_case)]

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};

use super::app::App;

/// 渲染 Network 面板
///
/// 显示已知节点列表，用颜色区分连接状态。
///
/// # 参数
/// - `frame`: ratatui 帧
/// - `area`: 分配给此面板的区域
/// - `app`: 应用状态引用
pub fn Render(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 📡 Network ")
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::DarkGray));

    if app.peers.is_empty() {
        // 无节点时显示提示
        let items = vec![
            ListItem::new(Line::from(Span::styled(
                "  等待节点连接...",
                Style::default().fg(Color::DarkGray),
            ))),
        ];
        let list = List::new(items).block(block);
        frame.render_widget(list, area);
        return;
    }

    // 构建节点列表
    let items: Vec<ListItem> = app
        .peers
        .iter()
        .map(|peer| {
            let (icon, color) = if peer.connected {
                ("●", Color::Green)
            } else {
                ("○", Color::Red)
            };

            // 截断 PeerId 显示（取前 8 位 + 后 4 位）
            let display_id = Truncate_Peer_Id(&peer.peer_id);
            let status = if peer.connected { "已连接" } else { "已断开" };

            let line = Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(display_id, Style::default().fg(Color::White)),
                Span::raw(" "),
                Span::styled(status, Style::default().fg(color)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// 截断 PeerId 以便显示
///
/// 格式: "12D3Koo...最后4位"
fn Truncate_Peer_Id(peer_id: &str) -> String {
    if peer_id.len() > 16 {
        let prefix = &peer_id[..8];
        let suffix = &peer_id[peer_id.len() - 4..];
        format!("{}...{}", prefix, suffix)
    } else {
        peer_id.to_string()
    }
}
