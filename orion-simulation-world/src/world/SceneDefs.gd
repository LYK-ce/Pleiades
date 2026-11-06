# Presented by KeJi
# Created Date ： 2026-11-06
# Modified Date ： 2026-11-06
#
# 场景数据表（task_49 §三）：一个场景 = 一条数据。
#   - 纯常量表 + 两个只读访问器（Has / Get）；**不声明 class_name**（COMPONENTS.md §9.1 E5）。
#   - **不 preload World.gd**（World.gd 要 preload 本文件，会成环；Facade 的 E31-⑤ 是同一条坑）。
#   - 坐标一律**世界坐标（Z-up，米）**；`obstacles` 的 `origin` 是最小角、`size` 是尺寸、右开区间，
#     与 `World._fill_block_world()` 逐字同源。
#   - 体素对齐铁律（task_49 §3.2，写死在数据里不靠自觉）：分量全为 0.5 整数倍 / 墙厚一律 0.5 /
#     门洞 ≥ 1.0 且 0.5 整数倍 / 视觉件顶点同样 0.5 整数倍并与体素外表面共面。
#   - const 容器在 4.7.2 下**只读**：Get() 必须 duplicate(true) 再交出，否则调用方一写就报 read-only。
#   - 本文件不建节点、不读 ProjectSettings、不取时间（纯数据）。

extends RefCounted

const SCENES := {
	# 城市街道（= 2026-11-06 演示的那个场景）：数值与 World.gd 的 const 逐条相等（口径 #3）。
	"city": {
		"name": "city",
		# 数值 = World.gd 2026-11-06 的 OBSTACLES，**一字不改**（改一个数 = city 格数变 = 验收红）。
		"obstacles": [
			# 边界墙 ×4：标出 128×128 m 世界范围，高 2 m
			{"name": "wall_south", "origin": Vector3(0.0, 0.0, 0.0), "size": Vector3(128.0, 0.5, 2.0)},
			{"name": "wall_north", "origin": Vector3(0.0, 127.5, 0.0), "size": Vector3(128.0, 0.5, 2.0)},
			{"name": "wall_west", "origin": Vector3(0.0, 0.0, 0.0), "size": Vector3(0.5, 128.0, 2.0)},
			{"name": "wall_east", "origin": Vector3(127.5, 0.0, 0.0), "size": Vector3(0.5, 128.0, 2.0)},
			# 建筑 A / B：低层块状障碍
			{"name": "building_a", "origin": Vector3(24.0, 24.0, 0.0), "size": Vector3(10.0, 8.0, 4.0)},
			{"name": "building_b", "origin": Vector3(88.0, 28.0, 0.0), "size": Vector3(8.0, 10.0, 5.0)},
			# 长墙 C / D：走廊与直角绕行场景
			{"name": "wall_c", "origin": Vector3(36.0, 92.0, 0.0), "size": Vector3(30.0, 1.0, 2.0)},
			{"name": "wall_d", "origin": Vector3(34.0, 100.0, 0.0), "size": Vector3(1.0, 24.0, 3.0)},
			# 柱 E：8 m 高，UAV 高度参照
			{"name": "pillar_e", "origin": Vector3(78.0, 78.0, 0.0), "size": Vector3(2.0, 2.0, 8.0)},
			# 三级退台 F1/F2/F3：视觉层次
			{"name": "step_f1", "origin": Vector3(102.0, 100.0, 0.0), "size": Vector3(8.0, 8.0, 1.0)},
			{"name": "step_f2", "origin": Vector3(103.0, 101.0, 0.0), "size": Vector3(6.0, 6.0, 2.0)},
			{"name": "step_f3", "origin": Vector3(104.0, 102.0, 0.0), "size": Vector3(4.0, 4.0, 3.0)},
			# 散块 ×3：小障碍点缀
			{"name": "block_1", "origin": Vector3(60.0, 100.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
			{"name": "block_2", "origin": Vector3(66.0, 100.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
			{"name": "block_3", "origin": Vector3(72.0, 104.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
		],
		"spawn": Vector3(64.0, 64.0, 0.0),
		"spawn_clear_radius_m": 3.0,
		"spawn_clear_height_m": 4.0,
		"air_wall_height_m": 32.0,
		"props_street": true,
		"props_facade": true,
		"look": "city",
		"ceiling": false,
		"ceiling_z_m": 0.0,
	},
	# 室内走廊（task_49 §3.3）：**本步 obstacles 留空**（Step 2 才填 8 条 AABB）。
	# 下面注释里的 8 条就是 §3.3 的定稿数据，Step 2 只需去掉注释符号、搬进 "obstacles": [ ... ]。
	# 01 corridor_south  origin (50.0, 61.5, 0.0)  size (24.0, 0.5, 3.0)   # 内表面 y=62，墙厚 0.5
	# 02 corridor_north  origin (50.0, 66.0, 0.0)  size (24.0, 0.5, 3.0)   # 内表面 y=66
	# 03 corridor_west   origin (49.5, 61.5, 0.0)  size ( 0.5, 5.0, 3.0)   # 西端 x=50 封死
	# 04 east_wall_low   origin (74.0, 61.5, 0.0)  size ( 0.5, 2.0, 3.0)   # y 61.5~63.5
	# 05 east_wall_high  origin (74.0, 64.5, 0.0)  size ( 0.5, 2.0, 3.0)   # y 64.5~66.5
	# 06 room_east       origin (79.0, 61.5, 0.0)  size ( 0.5, 5.0, 3.0)   # 房间内表面 x=79
	# 07 room_north      origin (74.5, 66.0, 0.0)  size ( 5.0, 0.5, 3.0)
	# 08 room_south      origin (74.5, 61.5, 0.0)  size ( 5.0, 0.5, 3.0)
	# 门洞 = y∈[63.5, 64.5] = 1.0 m = 2 格（≥1.0 且 0.5 整数倍，§3.2-3）；墙高 3 m = 6 层。
	"corridor_indoor": {
		"name": "corridor_indoor",
		"obstacles": [],
		"spawn": Vector3(64.0, 64.0, 0.0),
		"spawn_clear_radius_m": 1.5,
		"spawn_clear_height_m": 2.5,
		"air_wall_height_m": 32.0,
		"props_street": false,
		"props_facade": false,
		"look": "indoor",
		"ceiling": false,
		"ceiling_z_m": 3.0,
	},
}


## 场景名是否存在（名字是内部契约，不做大小写 / 模糊匹配）。
static func Has(name: String) -> bool:
	return SCENES.has(name)


## 取一个场景的**深拷贝**（调用方可自由改写，绝不回写常量表）；未知名字返回 `{}`（调用方必须判空）。
static func Get(name: String) -> Dictionary:
	if not SCENES.has(name):
		return {}
	var def: Dictionary = SCENES[name]
	return def.duplicate(true)


## 全部已知场景名（顺序 = 声明顺序；只用于日志 / 诊断）。
static func Names() -> Array:
	return SCENES.keys()
