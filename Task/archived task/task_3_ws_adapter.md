# Task 3: WebSocket

> 状态：设计完成
> 创建日期：2026-07-07

## 目标

WebSocket 遥控服务，作为 Robot 的上层命令源。

## 文件结构

```
Src/Robot/websocket.rs
```

## 设计

### 集成方式

```
main.rs
  │
  ├── Robot::launch()
  └── websocket::spawn(port, event_bus, robot.cmd_tx, robot.state)
        │
        └── tokio::spawn → run_server()
              ├── accept 循环 → handle_connection
              │     └── JSON → Command → cmd_tx.send()
              └── telemetry_loop → state.read() → EventBus
```

### 关键决策

| 决策 | 说明 |
|------|------|
| tokio::spawn | 共享主 runtime，不再独立线程 |
| cmd_tx | JSON 命令 → Command 枚举 → Robot 主循环 |
| state | 直接读，200ms 定时推 EventBus |
| 断开 | 自动发送 Command::Stop |
| 单向 | 目前只收命令，不回推传感器数据给客户端 |

## 待实现

- 双向通信：传感器数据推送给 WS 客户端
- 协议约定：server → client 消息格式

## 人类评审

<!-- 在此区域写下评审意见 -->

