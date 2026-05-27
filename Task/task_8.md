# Task 8: TUI 优化

> Presented by KeJi
> Date: 2026-05-27

---

## 目标

优化 TUI 的显示体验和交互逻辑，使其更加直观和实用。

---

## 背景

当前 TUI 基于 ratatui 实现了 5 区域双输入框布局，但随着 Session / API 等新功能的加入，部分面板的显示内容已不匹配实际需求。

### 当前 TUI 布局

```
┌──────────────────────────────┬──────────────────┐
│ Log (70%)                    │ Network (30%)    │
│ 系统日志 + 事件通知            │ 节点列表          │
├──────────────────────────────┴──────────────────┤
│ Job (3行)                                       │
│ 推理进度 / 文件传输                               │
├─────────────────────────────────────────────────┤
│ Command Output (8行)                             │
│ 命令结果 + chat 流式 token                        │
├─────────────────────────────────────────────────┤
│ Prompt> (3行)                                   │
│ chat 对话输入                                    │
├─────────────────────────────────────────────────┤
│ pleiades> (3行)                                 │
│ 系统命令输入                                     │
└─────────────────────────────────────────────────┘
```

---

## 设计

### 1. Network 面板：显示 peer name 而非 peer_id

**问题**：当前 Network 面板显示截断的 PeerId（如 `12D3Koo...xyz`），对用户毫无意义。

**方案**：
- `Peer_Display` 新增 `name: String` 字段
- EventBus `peer_discovered` / `peer_connected` 事件 payload 增加 `peer_name`
- `Handle_Bus_Event` 解析 `peer_name` 并写入
- `network_panel.rs` 展示 name 而非 `Truncate_Peer_Id(peer_id)`
- 若 name 为空（旧节点），回退显示截断 peer_id

**影响文件**：
- `Src/TUI/app.rs` — `Peer_Display` 加 `name` 字段，`Update_Peer` 加 name 参数
- `Src/TUI/mod.rs` — `handle_state` 解析 `peer_name`
- `Src/TUI/network_panel.rs` — 渲染 name
- `Src/Network/` (EventBus 发送端) — 事件 payload 加 `peer_name`

---

## 实施计划

0. 从当前分支 `reforge` 创建新分支 `task8_advance`，所有改动在此分支上进行 ✅
1. Network 面板：peer_id → peer_name
2. <!-- TODO -->

---

## 人类评审

<!-- 在此区域写下评审意见 -->

