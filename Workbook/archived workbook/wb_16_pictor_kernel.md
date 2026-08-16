# WB 16: Pictor Kernel 桥

> 状态：已完成（2026-08-16，人类测试通过）
> 对应任务：`Task/archived task/task_16_pictor_kernel.md`

## 完成内容

- 桥 crate `SrcPictorKernel/`（cdylib，gdext 0.5 + api-4-7，入口 `gdext_rust_init`）
- pleiades 车端核心改动：`Send_Data_Try` / `run_headless` / 命令入站（`DataType::Robot` 分支 + `command_consumer` + `parse_orion_frame` 迁入）/ LiDAR 探测范围 12m→5m
- WS 退役：`Src/WebSocket/` + `robot_control.html` + `ws_bind` 配置 + `tokio-tungstenite` 依赖 全删

## 架构边界（最终）

- **pleiades**（车端核心）：网络、机器人、车端入站命令
- **pictor-kernel**（桥，哑管道）：下行 `send_command`（NodeHandle `Send_Data_Try`）、上行转发 `robot_bus` 原始帧 + `event_bus` peer 事件

## 关键约定（新 agent 快速接入）

- 信号：`kernel_ready` / `robot_frame`(原始 ORION 帧) / `peer_discovered|left|connected|disconnected` / `peer_info_updated`；`poll()` 每帧调
- peer_id 统一 **hex**（event_bus 里 base58 → 桥转 hex）
- 入口符号 `gdext_rust_init`；`.gdextension` 每次改动需先 `godot --headless -e --quit` 触发扫描
- 编译：车端 `cargo build --release --bin orion-robot`；桥 `cargo build -p pictor-kernel`（不带 CUDA 需给 pleiades 依赖加 `default-features = false`）

## 遗留

- 编辑器进程 SIGABRT（未阻塞运行时）
- `.gitignore` 待补：`.config/`、`Log/`、`Pleiades_Workspace/`、`.kvcache/`（用户届时提醒）
- P3 Godot（Pictor）侧接入未做：拆 WS 栈、接桥信号/handle
