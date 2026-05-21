# Task 2: 清理 Src/ 目录中的文档文件

> start: 2026-05-21 | end: 2026-05-21 | status: ✅ done

## 执行记录

### 前置：重命名 classified_docs → archived_docs
### 2.1 移入 archived_docs/ (7 个)
orchestrator_implementation.md, pipeline.md, core_job.md, test_command.md,
advance.md (→ archived_docs/), TUI_reforge.md, Event_Bus_design.md
### 2.2 删除 (2 个)
Src/TUI/task.md, Src/EventBus/task.md
### 2.3 验证
- file_search Src/**/*.md → 0 结果 ✅
