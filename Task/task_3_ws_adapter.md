# Task 3: WebSocket 适配

> 状态：待开始
> 创建日期：2026-07-07

## 目标

将 WebSocket 遥控服务适配到 Robot 架构，作为上层命令源。

## 背景

当前 WS 服务通过命令行参数方式接收 `cmd_tx` + `state`，已经能和 Robot 对接：

```
WS 客户端
  │  JSON: {"cmd":"forward","speed":50}
  ▼
parse_command() → Command::Forward(50) → cmd_tx.send()
                                          │
                                          ▼
                                    Robot (select!)
```

电报方面直接读 `Robot.state`。WS 服务运行在独立线程 + 独立 tokio runtime。

## 待讨论

- WS 独立线程 + runtime 是否合理？
- 是否需要双向通信（WS 主动推送传感器数据给客户端）？
- 与 Robot 的生命周期协调（谁先退出、如何通知）？

## 人类评审

<!-- 在此区域写下评审意见 -->

