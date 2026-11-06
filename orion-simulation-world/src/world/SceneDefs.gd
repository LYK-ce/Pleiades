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
	# 室内走廊（task_49 §3.3 定稿 / §五 Step 2）：8 条 AABB，坐标与 §3.3 **一字不改**。
	#   对齐铁律（§3.2）：分量全为 0.5 整数倍 / 墙厚一律 0.5（=1 格）/ 墙高 3 m = 6 层格 /
	#   门洞宽 1.0 m = 2 格（63.5→cell 127、64.5→cell 129，127/128 两格空）。
	#   走廊内表面 x∈[50,74]（cell 100~147）、y∈[62,66]（cell 124~131，净宽 4 m）；西端 x=50 封死；
	#   小房间内表面 x∈[74.5,79]（cell 149~158）、y∈[62,66]；东墙 x=74 只占 cell 148。
	# 手算（cell = (floor(2x), floor(2z), floor(2y))）：8 条毛格数 864，房间南北墙与东墙在角上
	#   重叠 12 格（(158,*,123) 与 (158,*,132) 各 6），`_set_voxel` 去重后 **净 852 格**（预期日志值）。
	"corridor_indoor": {
		"name": "corridor_indoor",
		"obstacles": [
			# 01 走廊南墙：内表面 y=62（厚 0.5 = 1 格）
			{"name": "corridor_south", "origin": Vector3(50.0, 61.5, 0.0), "size": Vector3(24.0, 0.5, 3.0)},
			# 02 走廊北墙：内表面 y=66
			{"name": "corridor_north", "origin": Vector3(50.0, 66.0, 0.0), "size": Vector3(24.0, 0.5, 3.0)},
			# 03 西端封死：内表面 x=50
			{"name": "corridor_west", "origin": Vector3(49.5, 61.5, 0.0), "size": Vector3(0.5, 5.0, 3.0)},
			# 04 东墙下段：y 61.5~63.5（门洞下边界 63.5）
			{"name": "east_wall_low", "origin": Vector3(74.0, 61.5, 0.0), "size": Vector3(0.5, 2.0, 3.0)},
			# 05 东墙上段：y 64.5~66.5（门洞上边界 64.5）；04/05 之间 = 1.0 m 门洞
			{"name": "east_wall_high", "origin": Vector3(74.0, 64.5, 0.0), "size": Vector3(0.5, 2.0, 3.0)},
			# 06 房间东墙：内表面 x=79
			{"name": "room_east", "origin": Vector3(79.0, 61.5, 0.0), "size": Vector3(0.5, 5.0, 3.0)},
			# 07 房间北墙：内表面 y=66
			{"name": "room_north", "origin": Vector3(74.5, 66.0, 0.0), "size": Vector3(5.0, 0.5, 3.0)},
			# 08 房间南墙：内表面 y=62
			{"name": "room_south", "origin": Vector3(74.5, 61.5, 0.0), "size": Vector3(5.0, 0.5, 3.0)},
		],
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
