# 2026-05-13

## 问题1: 脚本引擎选型
- 对比: Rhai vs Lua vs Python
- 结论: Lua (mlua) — 语法简洁、零学习成本、工业成熟度

## 问题2: Lua替代范围
- Vm_Base + ML_VM + Orchestrator_VM 全部替代
- Slot系统全删 → MlContext struct 内部管理状态
- 张量不穿Lua边界

## 问题3: Capabilities职责
- Lua负责全部逻辑 (调度/筛选/组合)
- Rust只提供纯API (查询/执行/IO)
- Scheduler模块全删 → 调度脚本化

## 问题4: 命令约定
- 内置命令(无前缀): Core直调能力接口
- 程序命令(/前缀): programs/*.lua

## 问题5: sync vs async 注册
- trait方法都是async fn → 统一用create_async_function
