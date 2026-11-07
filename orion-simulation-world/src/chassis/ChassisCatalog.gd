# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-11-07
#
# 底盘目录 ChassisCatalog（COMPONENTS.md §7.6）：chassis 名 -> 场景路径 / 默认参数。
#   全静态、无状态副作用；被 Vehicle._resolve_chassis_scene() 与 Step 6 的 manager 共用。
#
# 为什么用**运行时 load()** 而不是预加载（§7.6 / §11.1 / §9.1）：
#   Step 6（manager）先于 Step 7（Quadrotor.tscn）落地，预加载一个尚不存在的场景
#   会让**整脚本解析失败**；load() 失败只是返回 null（由调用方 Has() 先判再取）。
#
# `pedestrian`（task_35 Step 1）是"特殊底盘"：它登记在目录里，好让外部脚本用**同一套**
#   chassis_hello / motion_cmd / pose 协议驱动行人；但 Step 2 起由 `kind="pedestrian"` 单独分流到
#   `Pedestrians` 容器（**不进** `Vehicles`、不占 `_PickSpawn` 的 avatar 排位）。Step 1 只登记，
#   不改变任何既有语义（既有自检没有任何路径会装配 pedestrian）。
#
# 不声明全局类名（§9.1 E5）：调用方 load("res://src/chassis/ChassisCatalog.gd") 后 call()。

extends RefCounted


## chassis 名 -> 场景路径（quadrotor_uav 的文件已于 Step 7 落地；Step 6 只做 Has() 判定）。
const SCENES := {
	"diff_drive_ugv": "res://src/scenes/chassis/DiffDrive.tscn",
	"quadrotor_uav": "res://src/scenes/chassis/Quadrotor.tscn",
	"pedestrian": "res://src/scenes/props/Pedestrian.tscn",       # task_35 Step 1
}

## chassis 名 -> 默认参数（与各底盘 GetDefaultParams() 逐键一致）。
## task_50 Step 2：diff_drive_ugv 按口径 #2 定为 0.80（长）× 0.50（高）× 0.55（宽），
##   速度为口径 #12（2.2 m/s、180 °/s）。
##   `body_length` 是**碰撞盒 X 边长（车身全长）**，`wheel_base` 只用于轮子视觉摆位 ——
##   两者同名不同义，别照抄旧口径把盒长塞进 wheel_base（那会让 0.8 m 的车鼻/尾各嵌墙 0.15 m）。
const DEFAULTS := {
	"diff_drive_ugv": {
		"max_speed": 2.2,          # m/s（口径 #12：车比跑者 1.8 快）
		"max_turn_rate": 180.0,    # 度/秒（= π rad/s，口径 #12）
		"wheel_base": 0.50,        # m（轴距；只用于轮子摆位）
		"track_width": 0.55,       # m（横向 = 外观 AABB z，碰撞盒 Z 边长）
		"height": 0.50,            # m = 外观 AABB y（碰撞盒 Y 边长）
		"body_length": 0.80,       # m（纵向 = 外观 AABB x，碰撞盒 X 边长；task_50 口径 #2）
	},
	"quadrotor_uav": {
		"max_speed": 2.0,
		"max_climb_rate": 1.0,
		"max_yaw_rate": 90.0,
		"size": 0.40,
	},
	"pedestrian": {
		"max_speed": 1.26,       # m/s = 走路动画自带步速（脚不滑）
		"max_turn_rate": 180.0,  # 度/秒（与 diff_drive_ugv 同口径）
	},
}

## 场景缓存：同一名字多次 GetScene() 返回**同一** PackedScene（避免重复读盘）。
static var _scene_cache: Dictionary = {}


## 目录内 = true；目录外名字一律 false（不 load、不报错）。
static func Has(p_name: String) -> bool:
	return SCENES.has(p_name)


## chassis 名 -> 场景。未登记 / 文件缺失 -> push_error + null（**设计如此**：调用方必须先 Has()）。
static func GetScene(p_name: String) -> PackedScene:
	if not SCENES.has(p_name):
		push_error("[chassis] 未登记的底盘：'%s'" % p_name)
		return null
	if _scene_cache.has(p_name):
		var cached: Variant = _scene_cache[p_name]
		if cached is PackedScene:
			return cached as PackedScene
	var path: String = str(SCENES[p_name])
	var scene: PackedScene = load(path) as PackedScene
	if scene == null:
		push_error("[chassis] 底盘场景加载失败：'%s' -> %s" % [p_name, path])
		return null
	_scene_cache[p_name] = scene
	return scene


## 默认参数**副本**（调用方改不到常量表）；未登记 -> {}。
static func GetDefaults(p_name: String) -> Dictionary:
	if not DEFAULTS.has(p_name):
		return {}
	var defaults: Dictionary = DEFAULTS[p_name]
	return defaults.duplicate(true)
