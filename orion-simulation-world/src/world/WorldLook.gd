# Presented by KeJi
# Created Date ： 2026-10-28
# Modified Date ： 2026-11-06
#
# 世界观感（材质 + 天空 + 太阳）唯一入口（COMPONENTS.md §4.1.1 / task_38 §二.3）。
#   - 地面 = 混凝土铺装（三贴图 PBR，世界空间 triplanar，6 m 一循环）
#   - 体素 = 混凝土（三贴图 PBR，世界空间 triplanar，2 m 一循环）
#   - 天空 = PanoramaSkyMaterial + assets/hdri/sky_midday_2k.hdr
#   - Sun  = 与 HDRI 太阳方向对齐、energy 0.7、shadow_enabled 不动
# 缺资产：只 push_warning、绝不 push_error（--quit 的 0 ERROR 验收靠这条），整体降级为
#   「地面/天空保持 .tscn 原样 + 体素回到单色」，几何 / 碰撞 / OBSTACLES 一律不碰。
# city 档：只碰 3 个 Material + 1 个 Sky + Sun 的 transform/energy，不建节点、不改 .tscn。
# indoor 档（task_49 Step 4，走 Apply_Indoor）：**不换天空、不动太阳**，把背景切 BG_COLOR、环境光切
#   AMBIENT_SOURCE_COLOR，并建 1 个 IndoorLights 容器挂 3 盏不投影的 OmniLight3D（llvmpipe 基线 ~11 fps，
#   灯多 / 开阴影会直接压垮帧率）。Env / Sky 一律 duplicate 再回写（E30-②）。
# 时间只由 WorldClock 提供，本文件不取时间。

extends RefCounted

const TAG := "[world-look]"
## 坐标换轴唯一入口（task_30 §7.3）：室内 3 盏灯的位置常量是世界坐标，换轴只经它。
const WorldTransform := preload("res://src/common/WorldTransform.gd")

const TEXTURE_DIR := "res://assets/textures/"
const HDR_PATH := "res://assets/hdri/sky_midday_2k.hdr"

const GROUND_PREFIX := "ground_paving"
const VOXEL_PREFIX := "concrete"

## 一循环多少米（世界空间 triplanar，uv1_scale = 1/tile_m；task_38 §二.1 写死 6 m / 2 m）。
const GROUND_TILE_M := 6.0
const VOXEL_TILE_M := 2.0
## Step 2 接线（task_38 §三）：沥青路面 4 m 一循环（1K 图 ⇒ 256 px/m）、路缘石 1.5 m 一循环。
## 只接线、不新增资产（assets/ 字节零增量）。
const ASPHALT_TILE_M := 4.0
const CURB_TILE_M := 1.5
## ---- 室内档（task_49 Step 4 / 贴图来自 Step 3：wall_indoor = white_plaster_02、floor_indoor = terrazzo_tiles）
const INDOOR_WALL_PREFIX := "wall_indoor"
const INDOOR_FLOOR_PREFIX := "floor_indoor"
## 一循环多少米：墙 2 m / 地 4 m（Step 4-2 写死；薄片无 UV ⇒ 靠 _Make_Pbr 的 uv1_world_triplanar）。
const INDOOR_WALL_TILE_M := 2.0
const INDOOR_FLOOR_TILE_M := 4.0
## 踢脚线 / 门框 = 墙面贴图的压暗变体：有贴图走 tint（albedo 乘子），无贴图走 FALLBACK_COLORS × tint
## ⇒ 两条路径下"踢脚深一档、门框更深"都成立（口径 #22）。
const INDOOR_SKIRTING_TINT := 0.70
const INDOOR_FRAME_TINT := 0.45
## 背景 / 环境光（口径 #7：亮度是**功能参数**）。中性偏亮，保证人在 3~8 m 处能被 YOLO 检出；
## 过亮会丢纹理与明暗层次（验收 ② 的"一片死灰"是另一个极端）。调参记录见 tmp/task49/impl.md。
const INDOOR_BG_COLOR := Color(0.10, 0.10, 0.11, 1.0)
const INDOOR_AMBIENT_COLOR := Color(0.72, 0.72, 0.70, 1.0)
const INDOOR_AMBIENT_ENERGY := 0.80
## 3 盏灯：世界坐标（Z-up / 米），全在走廊中轴、z = 2.5 m（墙高 3 m 之下）；第 3 盏靠门洞，光溢进房间。
const INDOOR_LIGHTS_NAME := "IndoorLights"
const INDOOR_LIGHT_POSITIONS := [
	Vector3(55.0, 64.0, 2.5),
	Vector3(63.0, 64.0, 2.5),
	Vector3(71.0, 64.0, 2.5),
]
const INDOOR_LIGHT_ENERGY := 1.5
const INDOOR_LIGHT_RANGE_M := 10.0
const INDOOR_LIGHT_COLOR := Color(1.0, 0.96, 0.90, 1.0)
const INDOOR_LIGHT_SPECULAR := 0.25
## 缺资产时的降级底色（按 prefix 取；tint 见 Make_Pbr_Material）。只 warn、绝不 push_error。
const FALLBACK_COLORS := {
	"asphalt": Color(0.16, 0.16, 0.17),
	"curb": Color(0.55, 0.54, 0.51),
	"concrete": Color(0.55, 0.58, 0.62),
	# task_49 Step 4（口径 #22）：室内无贴图时的纯色底色 —— 墙浅灰 / 地面深灰；
	# 踢脚（× 0.70）与门框（× 0.45）由 Make_Pbr_Material 的 tint 再压深一档。
	"wall_indoor": Color(0.78, 0.77, 0.74),
	"floor_indoor": Color(0.42, 0.41, 0.40),
}
## Sun 能量（task_38 §二.1「约 0.7」）。
const SUN_ENERGY := 0.7
## 天空能量（HDRI 直出会过曝：task_38 实测近白像素占比 0.523 / 0.531 > 0.50）。
## 这一个旋钮同时压「背景天空」与「来自天空的环境光」（ambient 由 sky 的 radiance 立方图产生），
## 于是 ground/体素不再被冲到 0.95 以上；背景模式与环境光来源都不动（仍 BG_SKY / AMBIENT_SOURCE_SKY）。
## 调值记录见 tmp/task38/impl.md「亮度调参」一节。
const SKY_ENERGY := 0.5
## HDRI 太阳方向（**Godot 空间**，单位向量，从场景指向太阳）。
## 实测：assets/hdri/sky_midday_2k.hdr（2048×1024，-Y 1024 +X 2048）最亮像素 (1229, 267)，
## 峰值亮度 110947；按 Godot 天空 shader 的等距柱面反解
##   u = atan2(x, z)/2π + 0.5 = 0.60034、v = acos(-y)/π = 0.73877（图像上边 = 天顶）
## 得 elev = 42.98°、|azim| = 36.12°。**水平方向取镜像那一侧**（实测修正，plan.md §7 R2）：
##   按 +azim 落成 (0.4313, 0.6817, 0.5910) 时，Xvfb 探针对准它的 mean=0.3083、
##   对准反方向的 mean=0.8642（太阳在反方向）⇒ 翻 x 得下面的向量（|v| = 1.000008）。
const SUN_DIR := Vector3(-0.4313, 0.6817, 0.5910)

## 材质 / 贴图 / Sky 缓存（幂等：重复 Apply_All 拿到同一批实例）。
static var _cache: Dictionary = {}
static var _missing: PackedStringArray = PackedStringArray()
static var _warned: bool = false


# ================================================================ 对外工厂

static func Make_Ground_Material() -> StandardMaterial3D:
	if _cache.has("ground"):
		return _cache["ground"] as StandardMaterial3D
	var mat: StandardMaterial3D = _Make_Pbr(GROUND_PREFIX, GROUND_TILE_M)
	_cache["ground"] = mat                                    # 可以存 null（缺资产：不再重复探测）
	return mat


static func Make_Voxel_Material(fallback_color: Color) -> StandardMaterial3D:
	if _cache.has("voxel"):
		return _cache["voxel"] as StandardMaterial3D
	var mat: StandardMaterial3D = _Make_Pbr(VOXEL_PREFIX, VOXEL_TILE_M)
	if mat == null:
		mat = Make_Fallback_Voxel_Material(fallback_color)
	_cache["voxel"] = mat
	return mat


## 降级材质 = 原单色观感（World.VOXEL_COLOR + roughness 0.85），与 task_37 逐值相同。
static func Make_Fallback_Voxel_Material(fallback_color: Color) -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	mat.albedo_color = fallback_color
	mat.roughness = 0.85
	return mat


## 通用三贴图 PBR 材质工厂（task_38 §三：街道 / 立面接线用）。
## prefix = 贴图名前缀（asphalt / curb / concrete），tile_m = 世界空间一循环米数，
## tint = albedo 乘法压暗（1.0 = Color.WHITE，与 Step 1b 的 _Make_Pbr 逐值一致）。
## 缺任一贴图 ⇒ 返回纯色降级材质（FALLBACK_COLORS[prefix] × tint、roughness 0.85），**绝不 push_error**。
## 薄片没有 UV ⇒ 必须经这里拿材质（内部 uv1_triplanar + uv1_world_triplanar），否则整面只采一个 texel。
static func Make_Pbr_Material(prefix: String, tile_m: float, tint: float = 1.0) -> StandardMaterial3D:
	var key := "pbr:%s:%.3f:%.3f" % [prefix, tile_m, tint]
	if _cache.has(key):
		return _cache[key] as StandardMaterial3D
	var mat: StandardMaterial3D = _Make_Pbr(prefix, tile_m)
	if mat == null:
		mat = StandardMaterial3D.new()
		var base: Color = FALLBACK_COLORS.get(prefix, Color(0.5, 0.5, 0.5))
		mat.albedo_color = Color(base.r * tint, base.g * tint, base.b * tint, 1.0)
		mat.roughness = 0.85
	elif absf(tint - 1.0) > 1e-6:
		mat.albedo_color = Color(tint, tint, tint)
	_cache[key] = mat
	return mat


## ---- 室内档材质工厂（task_49 Step 4-2）
## 一律走 Make_Pbr_Material：有贴图 ⇒ 三贴图 PBR + 世界空间 triplanar；**缺任一张 ⇒ 纯色降级**
## （FALLBACK_COLORS[prefix] × tint、roughness 0.85），**绝不 push_error、绝不返回 null**
## ⇒ 视觉件在两条路径下都会建出来（口径 #22，验收 ② 因此两条都算 PASS）。
static func Make_Indoor_Wall_Material() -> StandardMaterial3D:
	return Make_Pbr_Material(INDOOR_WALL_PREFIX, INDOOR_WALL_TILE_M)


static func Make_Indoor_Floor_Material() -> StandardMaterial3D:
	return Make_Pbr_Material(INDOOR_FLOOR_PREFIX, INDOOR_FLOOR_TILE_M)


static func Make_Indoor_Skirting_Material() -> StandardMaterial3D:
	return Make_Pbr_Material(INDOOR_WALL_PREFIX, INDOOR_WALL_TILE_M, INDOOR_SKIRTING_TINT)


static func Make_Indoor_Frame_Material() -> StandardMaterial3D:
	return Make_Pbr_Material(INDOOR_WALL_PREFIX, INDOOR_WALL_TILE_M, INDOOR_FRAME_TINT)


static func Make_Sky_Material() -> PanoramaSkyMaterial:
	if _cache.has("sky"):
		return _cache["sky"] as PanoramaSkyMaterial
	var tex := _Load_Tex(HDR_PATH)
	var mat: PanoramaSkyMaterial = null
	if tex != null:
		mat = PanoramaSkyMaterial.new()
		mat.panorama = tex
		mat.energy_multiplier = SKY_ENERGY
	_cache["sky"] = mat
	return mat


static func Apply_City(world: Node3D) -> bool:
	if world == null:
		return false
	var ok := true

	# ---- 1) 地面：只换 surface 0 的 override material，mesh / 碰撞 / 位置一律不碰
	var mi := world.get_node_or_null("Ground/MeshInstance3D") as MeshInstance3D
	var ground_mat := Make_Ground_Material()
	if mi == null or ground_mat == null:
		ok = false                                            # 保留 .tscn 里的原单色材质 = 降级
	else:
		mi.set_surface_override_material(0, ground_mat)

	# ---- 2) 天空：**复制** Environment + Sky 再回写（绝不改 .tscn 的共享 SubResource）
	var env_node := world.get_node_or_null("WorldEnvironment") as WorldEnvironment
	var sky_mat := Make_Sky_Material()
	if env_node == null or env_node.environment == null or sky_mat == null:
		ok = false
	else:
		var env_copy: Environment = env_node.environment.duplicate() as Environment
		var sky: Sky = env_copy.sky
		if sky == null:
			sky = Sky.new()
		else:
			sky = sky.duplicate() as Sky
		sky.sky_material = sky_mat
		env_copy.sky = sky
		env_node.environment = env_copy                   # background_mode / ambient_light_source 原样带走

	# ---- 3) 太阳：对齐 HDRI 太阳方向 + 能量 0.7（shadow_enabled 不动）
	var sun := world.get_node_or_null("Sun") as DirectionalLight3D
	if sun == null:
		ok = false
	else:
		var travel := -SUN_DIR.normalized()                # 光线传播方向 = 从太阳射向场景
		var up_ref := Vector3.UP
		if absf(travel.dot(Vector3.UP)) > 0.99:            # 近垂直时换参考轴，避免 looking_at 退化
			up_ref = Vector3.FORWARD
		# Basis.looking_at 让 **-Z** 指向 travel；DirectionalLight3D 正是沿 -Z 发光 ⇒ basis.z == +SUN_DIR
		sun.global_transform = Transform3D(Basis.looking_at(travel, up_ref), sun.global_position)
		sun.light_energy = SUN_ENERGY

	_Warn_Missing_Once()
	return ok


## 按观感档分派（task_49 Step 4-2）：`look` 取 SceneDef 的 `look` 字段（"city" / "indoor"）。
## 默认 "city" ⇒ 老的 `Apply_All(world)` 调用点行为不变（口径 #3 的 city 零变化）。
static func Apply_All(world: Node3D, look: String = "city") -> bool:
	if look == "indoor":
		return Apply_Indoor(world)
	return Apply_City(world)


## 室内档：**不换天空、不动太阳、不动地面**。
##   ① 背景切 BG_COLOR（深灰）+ 环境光切 AMBIENT_SOURCE_COLOR（中性偏亮）⇒ 走廊里看不到天空；
##   ② `ambient_light_sky_contribution = 0` ⇒ 即使 env 上还挂着 Sky，也不让它再贡献环境光（E30-③ 的反面）；
##   ③ 3 盏不投影 OmniLight3D ⇒ 近处亮、远处暗，视觉件有明暗层次；
##   ④ Env / Sky 一律 **duplicate 再回写**，绝不改 .tscn 的共享 SubResource（E30-②，step.md:51 硬判据）。
## 返回 false = 环境节点缺失或灯没建全（贴图缺失**不算** false：那是合法的降级路径，见 Interior 的打印）。
static func Apply_Indoor(world: Node3D) -> bool:
	if world == null:
		return false
	var env_ok := false

	var env_node := world.get_node_or_null("WorldEnvironment") as WorldEnvironment
	if env_node == null or env_node.environment == null:
		push_warning("%s 室内档找不到 WorldEnvironment/Environment，背景与环境光保持 .tscn 原样" % TAG)
	else:
		var env_copy: Environment = env_node.environment.duplicate() as Environment
		var sky: Sky = env_copy.sky
		if sky != null:
			env_copy.sky = sky.duplicate() as Sky       # 复制一份，绝不改共享 SubResource
		env_copy.background_mode = Environment.BG_COLOR
		env_copy.background_color = INDOOR_BG_COLOR
		env_copy.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
		env_copy.ambient_light_color = INDOOR_AMBIENT_COLOR
		env_copy.ambient_light_energy = INDOOR_AMBIENT_ENERGY
		env_copy.ambient_light_sky_contribution = 0.0
		env_node.environment = env_copy
		env_ok = true

	var lights: int = _Build_Indoor_Lights(world)
	_Warn_Missing_Once()
	print("%s indoor: background=BG_COLOR ambient=COLOR(%.2f,%.2f,%.2f)x%.2f lights=%d" % [
		TAG, INDOOR_AMBIENT_COLOR.r, INDOOR_AMBIENT_COLOR.g, INDOOR_AMBIENT_COLOR.b,
		INDOOR_AMBIENT_ENERGY, lights])
	return env_ok and lights == INDOOR_LIGHT_POSITIONS.size()


## 3 盏 OmniLight3D（**不投影、小能量**，step.md:14 + 坑 1）：llvmpipe 只有 ~11 fps 基线，投影会直接压垮帧率。
## 幂等：先摘同名旧容器再建；位置常量是世界坐标，换轴只经 WorldTransform。
static func _Build_Indoor_Lights(world: Node3D) -> int:
	var old: Node = world.get_node_or_null(INDOOR_LIGHTS_NAME)
	if old != null:
		world.remove_child(old)
		old.queue_free()
	var root := Node3D.new()
	root.name = INDOOR_LIGHTS_NAME
	world.add_child(root)
	for i: int in range(INDOOR_LIGHT_POSITIONS.size()):
		var lamp := OmniLight3D.new()
		lamp.name = "IndoorLight_%d" % (i + 1)
		lamp.position = WorldTransform.PosToGodot(INDOOR_LIGHT_POSITIONS[i])
		lamp.light_color = INDOOR_LIGHT_COLOR
		lamp.light_energy = INDOOR_LIGHT_ENERGY
		lamp.omni_range = INDOOR_LIGHT_RANGE_M
		lamp.omni_attenuation = 1.0
		lamp.light_specular = INDOOR_LIGHT_SPECULAR
		lamp.shadow_enabled = false
		root.add_child(lamp)
	return root.get_child_count()


static func Get_Missing_Assets() -> PackedStringArray:
	return _missing.duplicate()


static func Reset_Cache() -> void:
	_cache.clear()
	_missing = PackedStringArray()
	_warned = false


# ================================================================ 私有

## 三贴图 PBR 材质：缺任一张返回 null（调用方负责降级）。
static func _Make_Pbr(prefix: String, tile_m: float) -> StandardMaterial3D:
	var albedo := _Load_Tex(TEXTURE_DIR + prefix + "_color.jpg")
	var normal := _Load_Tex(TEXTURE_DIR + prefix + "_nor.jpg")
	var orm := _Load_Tex(TEXTURE_DIR + prefix + "_orm.png")
	if albedo == null or normal == null or orm == null:
		return null

	var mat := StandardMaterial3D.new()
	mat.albedo_texture = albedo
	mat.albedo_color = Color.WHITE
	# 法线：GL 约定（Poly Haven 的 nor_gl；禁止 nor_dx —— COMPONENTS.md §9.1 E30）
	mat.normal_enabled = true
	mat.normal_texture = normal
	mat.normal_scale = 1.0
	# ORM：R=AO / G=Roughness / B=Metallic。先按通道显式挂，再写 orm_texture（任务文档要求的槽位）
	mat.ao_enabled = true
	mat.ao_texture = orm
	mat.ao_texture_channel = BaseMaterial3D.TEXTURE_CHANNEL_RED
	mat.roughness = 1.0
	mat.roughness_texture = orm
	mat.roughness_texture_channel = BaseMaterial3D.TEXTURE_CHANNEL_GREEN
	mat.metallic = 1.0
	mat.metallic_texture = orm
	mat.metallic_texture_channel = BaseMaterial3D.TEXTURE_CHANNEL_BLUE
	mat.orm_texture = orm
	# triplanar：世界空间（GridMap 每格 MODEL_MATRIX 不同 ⇒ 世界空间才不会出格间缝）
	mat.uv1_triplanar = true
	mat.uv1_world_triplanar = true
	mat.uv1_scale = Vector3(1.0 / tile_m, 1.0 / tile_m, 1.0 / tile_m)
	mat.uv1_offset = Vector3.ZERO
	mat.texture_filter = BaseMaterial3D.TEXTURE_FILTER_LINEAR_WITH_MIPMAPS_ANISOTROPIC
	return mat


## load() 前必须 ResourceLoader.exists()：未 import 时 load() 会打 ERROR（§9.1 E28）。
static func _Load_Tex(res_path: String) -> Texture2D:
	if _cache.has(res_path):
		return _cache[res_path] as Texture2D
	var tex: Texture2D = null
	if ResourceLoader.exists(res_path):
		tex = load(res_path) as Texture2D
	if tex == null:
		if not _missing.has(res_path):
			_missing.append(res_path)
	else:
		_cache[res_path] = tex
	return tex


static func _Warn_Missing_Once() -> void:
	if _missing.is_empty() or _warned:
		return
	_warned = true
	push_warning("%s 缺资产，已降级为原单色观感：%s" % [TAG, str(_missing)])
