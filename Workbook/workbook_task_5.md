# Task 5: Lua 绑定重构

> start: 2026-05-21 | status: pending

## 目标
提取 15 处 mlua 闭包内联逻辑为独立 Rust fn，闭包降为薄胶水。

## 影响文件
- Src/VM/capability_binding.rs (9 处)
- Src/VM/local_stream.rs (3 处)
- Src/ML_Engine/lua_tensor.rs (2 处 + 公共函数)
- context.rs (1 处轻量)

## 公共工具
- parse_device_str() → 消除 3 处 device 解析重复
