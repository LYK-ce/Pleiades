# Task 1: 文档清理与重组

> start: 2026-05-21 | end: 2026-05-21 | status: ✅ done

## 执行记录

### 1.1 删除 classified_docs/ 过时文件 (27 个)
orchestrator_vm_design.md, ml_vm_design.md, profiler_design.md, scheduler_design.md,
ML_design.md, core_instruction_handler.md, core_instruction.md, ML_Engine_reforge.md,
"ML Engine new design.md", report_4.30.md, 中期总结.md, instruction.md, executor.md,
slot.md, register.md, control.md, Orchestrator.md, Pleiades_Design_Document_v2.md,
Pleiades设计文档.md, problems.md, modification.md, IO.md, stream.md, portal.md,
传输层设计探讨 .md, 优化编译方案.md, Src/Orchestrator/scheduler.md

### 1.2 修正 STALE 路径 (Src/Lua/ → Src/VM/)
- docs/code_review_lua_integration.md
- docs/capabilities_detail.md
- docs/lua_integration_plan.md
- docs/event_bus_reforge.md
- docs/summary.md (也修正了 Vm_Base 条目 + Lua→VM 模块名)

### 1.3 归档历史文档 (docs/ → classified_docs/)
- docs/lua.md, docs/Pleiades_v0.2.md, docs/lua_migration_summary.md,
  docs/orchestrator_reforge.md, docs/ml_engine_code_review.md

### 1.4 删除空文件
- Pleiades_doc.md (root)

### 1.5 验证
- grep "Src/Lua/" docs/ → 0 hits
- grep "Vm_Base|ML_VM|Orchestrator_VM|program_selector" docs/ → 全部为准确的历史对比/已删除声明

