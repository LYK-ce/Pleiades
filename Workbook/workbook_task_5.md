# Task 5: Lua 绑定重构

> start: 2026-05-21 | status: pending

## 目标
提取 15 处 mlua 闭包内联逻辑为独立 Rust fn，闭包降为薄胶水。

## 实施步骤

### 5.0 UserData 迁移 — ML_Engine → VM
- [x] `Src/ML_Engine/lua_tensor.rs` → 移除 `impl mlua::UserData for LuaTensor { add_methods }` 整块
- [x] 新建 `Src/VM/lua_tensor_binding.rs` → 移入上述 impl 块
- [x] `Src/VM/mod.rs` → 新增 `pub mod lua_tensor_binding`
- [x] 验证：`ML_Engine/lua_tensor.rs` 零 mlua 依赖

### 5.1 lua_tensor_binding.rs 内联提取
- [x] dims — 格式适配（slice→Lua table），绑定层天然职责，不提取
- [x] parse_device_str() 放入 `ML_Engine/lua_tensor.rs`，pub(crate)
- [x] `bytes_to_tensor_str(data, device_str)` + `LuaTensor::to_device_str(device_str)` 两个业务入口封装 parse_device_str
- [x] to_device / tensor_from_bytes / recv_tensor(×2) 四处闭包改为调用业务入口，消除 device 枚举重复

### 5.2 capability_binding.rs 内联提取 (9 处)
- [x] storage_list → 格式适配，不提取
- [x] storage_checksum → 不需要提取
- [x] storage_flush → 格式适配，不提取
- [x] ml.analyze_model → 格式适配，不提取
- [x] ml.tensor_from_bytes → 被 5.1 bytes_to_tensor_str 覆盖
- [x] network.send_data → 不需要提取
- [x] network.send_tensor → 代理+协议调用，不提取
- [x] network.recv_tensor → 被 5.1 bytes_to_tensor_str 覆盖
- [x] network.send_eof → 已足够薄，不提取

### 5.3 local_stream.rs 内联提取 (3 处)
- [x] send_tensor → 代理+协议调用，不提取
- [x] recv_tensor → 被 5.1 bytes_to_tensor_str 覆盖
- [x] send_eof → 已足够薄，不提取

### 5.4 验证
- [x] `cargo check --no-default-features` 通过
- [x] `cargo test --no-default-features --lib` 103 passed, 0 failed
