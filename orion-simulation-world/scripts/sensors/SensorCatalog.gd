# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# 传感器目录 SensorCatalog（COMPONENTS.md §7.6 / task_30 §7.2 D6）：sensor 名 -> 场景路径 / 默认参数。
#   全静态、无状态副作用；被 Step 6 的 WebSocketManager 与自检共用，与 ChassisCatalog 同构。
#
# 为什么用**运行时 load()** 而不是预加载（§7.6 / §11.1 / §9.1）：
#   Step 6（manager）先于 Step 8（Lidar2D.tscn）落地，预加载一个尚不存在的场景
#   会让**整脚本解析失败**；load() 失败只是返回 null（由调用方 Has() 先判再取）。
#
# manager 的 _CatalogSceneExists() 通过 get_script_constant_map() 读 SCENES 表，
#   所以 SCENES 必须是**脚本常量**（不能改成运行时赋值的变量）。
#
# 不声明全局类名（§9.1 E5）：调用方 load("res://scripts/sensors/SensorCatalog.gd") 后 call()。

extends RefCounted


## sensor 名 -> 场景路径（v1 只有 lidar_2d，§7.2 D6；imu / gnss 在 v1 **不做**：既不是零件
## 也不发消息——位姿由 `pose` 文本消息提供，见 COMPONENTS.md §8.1-8 / §10）。
const SCENES := {
	"lidar_2d": "res://sensors/Lidar2D.tscn",
}

## sensor 名 -> 默认参数（与 Lidar2D.gd 的默认值逐键一致）。
const DEFAULTS := {
	"lidar_2d": {
		"range": 12.0,
		"rays": 360,
		"rate_hz": 10.0,
	},
}

## 场景缓存：同一名字多次 GetScene() 返回**同一** PackedScene（避免重复读盘）。
static var _scene_cache: Dictionary = {}


## 目录内 = true；目录外名字一律 false（不 load、不报错）。
static func Has(p_name: String) -> bool:
	return SCENES.has(p_name)


## sensor 名 -> 场景。未登记 / 文件缺失 -> push_error + null（**设计如此**：调用方必须先 Has()）。
static func GetScene(p_name: String) -> PackedScene:
	if not SCENES.has(p_name):
		push_error("[sensor] 未登记的传感器：'%s'" % p_name)
		return null
	if _scene_cache.has(p_name):
		var cached: Variant = _scene_cache[p_name]
		if cached is PackedScene:
			return cached as PackedScene
	var path: String = str(SCENES[p_name])
	var scene: PackedScene = load(path) as PackedScene
	if scene == null:
		push_error("[sensor] 传感器场景加载失败：'%s' -> %s" % [p_name, path])
		return null
	_scene_cache[p_name] = scene
	return scene


## 默认参数**副本**（调用方改不到常量表）；未登记 -> {}。
static func GetDefaults(p_name: String) -> Dictionary:
	if not DEFAULTS.has(p_name):
		return {}
	var defaults: Dictionary = DEFAULTS[p_name]
	return defaults.duplicate(true)
