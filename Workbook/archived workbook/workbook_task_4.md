# Task 4: 验证 LocalTensorStream

> start: 2026-05-21 | end: 2026-05-21 | status: ✅ done

## 执行记录

### 4.1 重构: lua_binding.rs → VM/local_stream.rs
- 与 network_stream.rs, storage_handle.rs 并列

### 4.2 LocalStreamHub 注入 Capabilities
- Capabilities +local_stream_hub, main.rs 创建, spawn 注册

### 4.3 local_coord.lua + local_work.lua
- 新建脚本, 本地双线程流水线

### 4.4 API 对齐 network (后续改进)
- send_tensor: (stream, LuaTensor, offset) 直接接受 tensor
- recv_tensor: (stream, device) → (LuaTensor, offset) 直接返回 tensor
- 移除手动 to_bytes/to_tensor 样板

### 调试记录
- duplex buffer 64KB→16MB (hidden tensor ~106KB)
- pcall 不适用于 async 函数 (Future 未 poll 直接 drop → early eof)
- to_bytes Vec<u8>→Lua table 问题 → lua.create_string 显式转换
- 最终: API 对齐 network 接口, 端点验证通过 ✅
