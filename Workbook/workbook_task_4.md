# Task 4: 验证 LocalTensorStream

> start: 2026-05-21 | status: pending

## 方案

两个 `exec` 命令启动两个 Lua 线程，共享同一个 `Arc<LocalStreamHub>`，通过流 ID 匹配。

```
TUI> exec local_work    (spin up thread B first, waiting to accept)
TUI> exec local_coord   (thread A opens, both connect and start)
```

### 数据流
```
Thread A (coord):                      Thread B (work):
  open_stream("fwd") ──Hub──→          accept_stream("fwd")
  accept_stream("bwd") ←──Hub──        open_stream("bwd")
  
  forward → send_tensor(fwd)           recv_tensor(fwd) → forward
  recv_tensor(bwd) → sample/decode     send_tensor(bwd)
```

### 改动清单
- **4.1** 重构: lua_binding.rs 从 Orchestrator/local_tensor_stream/ → VM/local_stream.rs
  - 新建 VM/local_stream.rs, VM/mod.rs +pub mod, 删除旧文件
- **4.2** Capabilities +local_stream_hub, main.rs 创建, spawn 注册
- **4.3** programs/user/local_coord.lua + local_work.lua (新建)
- **4.4** docs/capabilities.md +local_tensor API 文档
