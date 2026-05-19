//Presented by KeJi
//Date ： 2026-05-19

## Core 调度问题

**现状：** `route_user()` 持有 `&mut self` 阻塞等待 `execute_lua_script` 完成，期间 Core 无法处理任何其他事件（包括网络消息、生命周期、Job 管理）。

**问题：** 长时间运行的 Lua 脚本（如 pipeline 推理、分布式协作）会独占 Core，等同于单线程阻塞。

**解决方向：** `route_user` 收到命令后 `tokio::spawn` 独立 task，在 `spawn_blocking` 中新建 Lua sandbox 跑脚本。

**注意点：**
- Lua 是 `!Send`，必须在目标线程上就地创建
- 当前 `Capabilities` (network, storage, event_bus) 均为 `Arc + Send + Sync`，跨线程安全
- `libp2p::Stream` 是 `Send` 的
- `RendezvousMap` 内部 `std::sync::Mutex + Arc`，跨线程安全
- 生产环境需考虑线程池或 actor 模式防止请求膨胀
