# Task 2: Adapter

> 状态：待开始
> 创建日期：2026-07-07

## 目标

实现中层 Adapter，作为上层逻辑和底层 Device 之间的桥梁。

## 背景

当前架构缺少中层——上层直接调 `STM32Device::forward()` 和 `get_state()`，
没有统一的事件循环和调度层。引入 Adapter 后：

```
上层 Loop select!
  │
  ▼
Adapter（中层）
  ├── 聚合多个 Device 的事件/状态
  ├── 上行：Device 状态 → 业务事件
  └── 下行：业务命令 → Device 调用
  │
  ▼
Device（底层）
  STM32 / LiDAR / Camera ...
```

## 设计要点

- 统一的事件循环（`tokio::select!` 聚合多个 Device 的状态和命令）
- 不关心具体 Device 的协议细节
- 可扩展：新增设备只需在 select! 里加一个分支

## 人类评审

<!-- 在此区域写下评审意见 -->

