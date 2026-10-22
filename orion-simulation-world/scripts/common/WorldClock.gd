# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# 统一 world_time 唯一入口（task_30 §6.3-3 / COMPONENTS.md §6.4、§7.1）。
# 语义：单调墙钟秒，起点为进程内首次 EnsureStarted() 的时刻；不受物理 tick / 帧率影响。
# 用途：Step 6/8 的所有下行帧时间戳一律取 Now()，让 N 条异步连接共享同一时间基。
# 不声明 class_name（COMPONENTS.md §9.1 E5）；调用方 preload 本脚本。

extends RefCounted

static var _started: bool = false
static var _start_usec: int = 0


## 幂等：首次调用记录起点，重复调用无副作用。
static func EnsureStarted() -> void:
	if _started:
		return
	_started = true
	_start_usec = Time.get_ticks_usec()


## 自起点以来的微秒数；未启动时自动启动（不会返回负值）。
static func NowUsec() -> int:
	EnsureStarted()
	return Time.get_ticks_usec() - _start_usec


## 自起点以来的秒数（float，单调不降）。
static func Now() -> float:
	return float(NowUsec()) / 1000000.0


## 仅测试用：清空起点，下次 Now() 重新计时。
static func Reset() -> void:
	_started = false
	_start_usec = 0
