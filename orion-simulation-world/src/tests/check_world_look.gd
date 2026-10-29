# Presented by KeJi
# Created Date ： 2026-10-28
# Modified Date ： 2026-10-28
#
# 世界观感自检（task_38 Step 1b，COMPONENTS.md §4.1.1）：S 组 30 项结构（headless 也全跑）+
# R 组 13 项渲染（仅 Xvfb；headless 打印 13 条 [skip] 且**不计入 total**）。
# 把关：三贴图真在 / triplanar 世界空间且尺度对 / 天空是 PanoramaSkyMaterial / background_mode 与
# ambient_light_source 未变 / Ground 几何与 AABB 未变 / Sun 与 HDRI 太阳方向一致 /
# 落盘 3 张 1600×900 对比图（地面近景 / 建筑侧视 / UAV 俯视）+ 太阳方向探针。
# 本脚本**不 push_error**（失败一律 _check(..., false, detail) + quit(1)），否则 0 ERROR 验收假红。
#
# 跑法 ①（headless，结构档：total=30 + 13 条 [skip]）：
#   godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_world_look.gd
# 跑法 ②（Xvfb + 软件 OpenGL，S+R 全跑：total=43 + 5 张 jpg）：
#   DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 godot --display-driver x11 \
#     --rendering-driver opengl3 --audio-driver Dummy --path orion-simulation-world \
#     --fixed-fps 60 --script res://src/tests/check_world_look.gd
#
# 为什么分两档（COMPONENTS.md §9.1 E26）：--headless 是 dummy 渲染器，Can_Capture() 恒 false，
#   碰渲染器之前必须分流，否则 frame_post_draw 永不触发、get_texture().get_image() 打 ERROR。
# 坐标：机位一律世界坐标（Z-up，米）经 WorldTransform.PosToGodot 换轴，本文件无换轴字面量。
# 图片只落 tmp/task38/shots/（绝对路径），**绝不**写进 res://。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldLook := preload("res://src/world/WorldLook.gd")

const WORLD_SCENE := "res://src/scenes/World.tscn"
const CAMERA_SCENE := "res://src/scenes/sensors/CameraSensor.tscn"

## 图片落 tmp/task38/shots/（**绝不**写进 res://，否则污染仓库）
const OUT_DIR := "/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/task38/shots"
const SHOT_SIZE := Vector2i(1600, 900)
const SHOT_FOV := 70.0
const SHOT_QUALITY := 0.85
const MIN_BYTES := 10 * 1024

## 三机位（**世界坐标 Z-up / 米**，一律经 WorldTransform.PosToGodot 换轴）
const SHOTS := [
	# 地面近景：眼高 0.55 m，俯 4.5°，7 m 内全是铺装（看 6 m 一循环的接缝与法线）
	{"id": 0, "file": "look_00_ground_closeup.jpg",
		"eye": Vector3(48.0, 81.0, 0.55), "aim": Vector3(48.0, 74.0, 0.0)},
	# 建筑侧视：building_a（世界 x∈[24,34]、y∈[24,32]、高 4 m）西立面正对，机位离面 4 m、眼高 1.7 m
	{"id": 1, "file": "look_01_building_side.jpg",
		"eye": Vector3(20.0, 28.0, 1.70), "aim": Vector3(34.0, 28.0, 1.70)},
	# UAV 俯视：高 15 m，斜下 44°（不写纯垂直：look_at 与 UP 平行会退化）
	{"id": 2, "file": "look_02_uav_top.jpg",
		"eye": Vector3(58.0, 58.0, 15.0), "aim": Vector3(48.0, 48.0, 0.5)},
]

## 像素判据（与 check_camera_shots / check_pedestrian_shots **同值**，不得放宽）
const VAR_MIN := 0.002
const MEAN_MIN := 0.05
const MEAN_MAX := 0.95
const SPREAD_MIN := 0.15
const WHITE_MAX := 0.50
const DIFF_BYTE_MIN := 0.10
const DIFF_MEAN_MIN := 0.02
const DIFF_STEP := 8

## 太阳方向探针：10° 窄视角分别对准 SUN_DIR 与反方向，比均值
const PROBE_SIZE := Vector2i(160, 90)
const PROBE_FOV := 10.0
const PROBE_EYE_WORLD := Vector3(64.0, 64.0, 1.0)
const PROBE_MEAN_MIN := 0.60      # 实测原始 HDR：太阳窗口 100% 像素 > 1.0（必然压到白）
const PROBE_CTRL_MAX := 0.55      # 实测原始 HDR：反日窗口 mean = 0.115（远低于阈值）
const PROBE_MARGIN_MIN := 0.20

## 尺度判据（米制换算：1/6、1/2；容差 1e-6）
const GROUND_SCALE_WANT := 1.0 / 6.0
const VOXEL_SCALE_WANT := 1.0 / 2.0
const SCALE_TOL := 1e-6

## 几何不变（与 check_world.gd / World.gd 同值）
const CELLS_MIN := 12000
const CELLS_MAX := 20000
const GROUND_SIZE := Vector3(128.0, 1.0, 128.0)
const GROUND_CENTER := Vector3(64.0, -0.5, 64.0)
## 太阳方向自洽判据
const SUN_DOT_MIN := 0.9999
const SUN_ENERGY_TOL := 1e-4
const SUN_DIR_TOL := 1e-3
const SUN_UP_Y_MIN := 0.2
## HDRI 全景尺寸（实测 2048×1024）
const PANO_SIZE := Vector2i(2048, 1024)
## 几何容差
const GEO_TOL := 1e-3

var _total: int = 0
var _failed: int = 0
var _skipped: int = 0
var _world: Node3D = null
var _cam: Node3D = null
var _holder: Node3D = null
var _shots: Array = []
var _probe_means: Dictionary = {}


func _initialize() -> void:
	_run()


func _run() -> void:
	# ① 载 World（入树前关掉 WebSocketManager.autolisten，照 check_camera_shots:213-229）
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	if not _check("scene-loads", packed != null, WORLD_SCENE):
		_finish()
		return
	var node: Node = packed.instantiate()
	_world = node as Node3D
	if not _check("world-instance", _world != null):
		_finish()
		return
	var ws: Node = _world.get_node_or_null("Systems/WebSocketManager")
	if ws != null:
		ws.set("autolisten", false)          # 不监听 9999、不连 WS
	root.add_child(_world)
	await process_frame
	await process_frame

	# ② S 组其余 28 项（前 2 项 scene-loads / world-instance 已在上面的载入段断言）
	_Run_S_Group()

	# ③ R 组：13 项（仅真渲染器；headless 打印 [skip] 且不计入 total）
	var can_capture: bool = await _Cam_Can_Capture()
	if can_capture:
		await _Run_R13()
	else:
		_Skip_R13()

	# ④ 释放 + 退出（否则 "resources still in use at exit" 变成 ERROR）
	await _Teardown()
	_finish()


# ================================================================ S 组：结构 30 项

## S3~S30（28 项）：材质 / 天空 / 太阳 / 几何不变量。
func _Run_S_Group() -> void:
	var mi: MeshInstance3D = _world.get_node_or_null("Ground/MeshInstance3D") as MeshInstance3D
	var gm: Material = mi.get_surface_override_material(0) if mi != null else null
	var gs: StandardMaterial3D = gm as StandardMaterial3D

	var voxel: GridMap = _world.get_node_or_null("VoxelWorld") as GridMap
	var lib: MeshLibrary = voxel.mesh_library if voxel != null else null
	var vmesh: Mesh = lib.get_item_mesh(0) if lib != null else null
	var vmat: Material = vmesh.surface_get_material(0) if vmesh != null else null
	var vs: StandardMaterial3D = vmat as StandardMaterial3D

	var env_node: WorldEnvironment = _world.get_node_or_null("WorldEnvironment") as WorldEnvironment
	var env: Environment = env_node.environment if env_node != null else null
	var sky: Sky = env.sky if env != null else null
	var sky_mat: Material = sky.sky_material if sky != null else null
	var pano: PanoramaSkyMaterial = sky_mat as PanoramaSkyMaterial

	var sun: DirectionalLight3D = _world.get_node_or_null("Sun") as DirectionalLight3D

	# ---- S3 观感总开关（实例读脚本常量：const 不在属性表里，只能走 get_script_constant_map）
	var consts: Dictionary = _world.get_script().get_script_constant_map()
	var look_enabled: bool = bool(consts.get("LOOK_ENABLED", false))
	_check("look-enabled", look_enabled, "LOOK_ENABLED=%s" % str(look_enabled))

	# ---- S4 资产齐备（缺一张就红；降级路径另由 S30 覆盖）
	var missing: PackedStringArray = WorldLook.Get_Missing_Assets()
	_check("assets-complete", missing.is_empty(), "missing=%s" % str(missing))

	# ---- S5 本机 StandardMaterial3D 有 ORM / triplanar 槽位（先红就说明要降级写法）
	var props: Array[String] = []
	for p: Dictionary in StandardMaterial3D.new().get_property_list():
		props.append(String(p["name"]))
	var has_orm: bool = props.has("orm_texture")
	var has_tri: bool = props.has("uv1_triplanar")
	var has_world_tri: bool = props.has("uv1_world_triplanar")
	_check("stdmaterial-props", has_orm and has_tri and has_world_tri,
		"orm_texture=%s uv1_triplanar=%s uv1_world_triplanar=%s" % [
			str(has_orm), str(has_tri), str(has_world_tri)])

	# ---- S6~S14 地面材质
	_check("ground-material", gs != null, "class=%s" % (gm.get_class() if gm != null else "null"))
	_check("ground-albedo", gs != null and gs.albedo_texture != null,
		"albedo=%s" % _Tex_Desc(gs.albedo_texture if gs != null else null))
	_check("ground-normal", gs != null and gs.normal_texture != null,
		"normal=%s" % _Tex_Desc(gs.normal_texture if gs != null else null))
	_check("ground-orm", gs != null and gs.orm_texture != null,
		"orm=%s" % _Tex_Desc(gs.orm_texture if gs != null else null))
	_check("ground-normal-enabled", gs != null and gs.normal_enabled,
		"normal_enabled=%s" % str(gs != null and gs.normal_enabled))
	var chan_ok: bool = gs != null \
		and gs.ao_enabled and gs.ao_texture != null \
		and gs.ao_texture_channel == BaseMaterial3D.TEXTURE_CHANNEL_RED \
		and gs.roughness_texture != null \
		and gs.roughness_texture_channel == BaseMaterial3D.TEXTURE_CHANNEL_GREEN \
		and gs.metallic_texture != null \
		and gs.metallic_texture_channel == BaseMaterial3D.TEXTURE_CHANNEL_BLUE
	_check("ground-orm-channels", chan_ok,
		"ao=%s/%d rough=%s/%d metal=%s/%d" % [
			str(gs != null and gs.ao_texture != null),
			gs.ao_texture_channel if gs != null else -1,
			str(gs != null and gs.roughness_texture != null),
			gs.roughness_texture_channel if gs != null else -1,
			str(gs != null and gs.metallic_texture != null),
			gs.metallic_texture_channel if gs != null else -1])
	_check("ground-triplanar", gs != null and gs.uv1_triplanar and gs.uv1_world_triplanar,
		"triplanar=%s world=%s" % [
			str(gs != null and gs.uv1_triplanar), str(gs != null and gs.uv1_world_triplanar)])
	var gsc: Vector3 = gs.uv1_scale if gs != null else Vector3.ZERO
	_check("ground-scale", gs != null
		and absf(gsc.x - gsc.y) < SCALE_TOL and absf(gsc.y - gsc.z) < SCALE_TOL
		and absf(gsc.x - GROUND_SCALE_WANT) < SCALE_TOL
		and absf(gsc.x - 1.0) > SCALE_TOL,
		"uv1_scale=%s(期望 %.9f = 1/%.1f m)" % [str(gsc), GROUND_SCALE_WANT,
			WorldLook.GROUND_TILE_M])
	_check("ground-filter", gs != null
		and gs.texture_filter == BaseMaterial3D.TEXTURE_FILTER_LINEAR_WITH_MIPMAPS_ANISOTROPIC,
		"texture_filter=%d" % (gs.texture_filter if gs != null else -1))

	# ---- S15~S20 体素材质
	_check("voxel-material", vs != null, "class=%s" % (vmat.get_class() if vmat != null else "null"))
	_check("voxel-albedo", vs != null and vs.albedo_texture != null,
		"albedo=%s" % _Tex_Desc(vs.albedo_texture if vs != null else null))
	_check("voxel-normal", vs != null and vs.normal_texture != null,
		"normal=%s" % _Tex_Desc(vs.normal_texture if vs != null else null))
	_check("voxel-orm", vs != null and vs.orm_texture != null,
		"orm=%s" % _Tex_Desc(vs.orm_texture if vs != null else null))
	_check("voxel-triplanar", vs != null and vs.uv1_triplanar and vs.uv1_world_triplanar,
		"triplanar=%s world=%s" % [
			str(vs != null and vs.uv1_triplanar), str(vs != null and vs.uv1_world_triplanar)])
	var vsc: Vector3 = vs.uv1_scale if vs != null else Vector3.ZERO
	_check("voxel-scale", vs != null
		and absf(vsc.x - vsc.y) < SCALE_TOL and absf(vsc.y - vsc.z) < SCALE_TOL
		and absf(vsc.x - VOXEL_SCALE_WANT) < SCALE_TOL
		and absf(vsc.x - 1.0) > SCALE_TOL,
		"uv1_scale=%s(期望 %.9f = 1/%.1f m)" % [str(vsc), VOXEL_SCALE_WANT,
			WorldLook.VOXEL_TILE_M])

	# ---- S21~S23 天空 / 环境
	_check("sky-material", sky != null and pano != null,
		"sky=%s class=%s" % [str(sky != null), sky_mat.get_class() if sky_mat != null else "null"])
	_check("sky-panorama", pano != null and pano.panorama != null
		and pano.panorama.get_width() == PANO_SIZE.x
		and pano.panorama.get_height() == PANO_SIZE.y,
		"panorama=%s" % _Tex_Desc(pano.panorama if pano != null else null))
	_check("env-modes", env != null
		and env.background_mode == Environment.BG_SKY
		and env.ambient_light_source == Environment.AMBIENT_SOURCE_SKY,
		"bg=%d(==%d) ambient=%d(==%d)" % [
			env.background_mode if env != null else -1, Environment.BG_SKY,
			env.ambient_light_source if env != null else -1, Environment.AMBIENT_SOURCE_SKY])

	# ---- S24~S27 太阳
	var sun_basis_z: Vector3 = sun.global_transform.basis.z.normalized() if sun != null else Vector3.ZERO
	var sun_dir: Vector3 = WorldLook.SUN_DIR.normalized()
	var dot: float = sun_basis_z.dot(sun_dir)
	_check("sun-aligned", sun != null and dot > SUN_DOT_MIN,
		"basis.z=%s dot=%.6f(>%.4f)" % [str(sun_basis_z), dot, SUN_DOT_MIN])
	_check("sun-direction-sane",
		absf(WorldLook.SUN_DIR.length() - 1.0) < SUN_DIR_TOL and WorldLook.SUN_DIR.y > SUN_UP_Y_MIN,
		"|SUN_DIR|=%.6f y=%.4f(>%.2f)" % [
			WorldLook.SUN_DIR.length(), WorldLook.SUN_DIR.y, SUN_UP_Y_MIN])
	_check("sun-energy", sun != null and absf(sun.light_energy - WorldLook.SUN_ENERGY) < SUN_ENERGY_TOL,
		"light_energy=%.6f(期望 %.2f)" % [
			sun.light_energy if sun != null else -1.0, WorldLook.SUN_ENERGY])
	_check("sun-shadow", sun != null and sun.shadow_enabled,
		"shadow_enabled=%s" % str(sun != null and sun.shadow_enabled))

	# ---- S28 地面几何 / AABB 未变
	var ground: StaticBody3D = _world.get_node_or_null("Ground") as StaticBody3D
	var gpos: Vector3 = ground.global_position if ground != null else Vector3.ZERO
	var cs: CollisionShape3D = ground.get_node_or_null("CollisionShape3D") as CollisionShape3D \
		if ground != null else null
	var box: BoxShape3D = null
	if cs != null:
		box = cs.shape as BoxShape3D
	var box_size: Vector3 = box.size if box != null else Vector3.ZERO
	var mesh: Mesh = mi.mesh if mi != null else null
	var aabb_size: Vector3 = mesh.get_aabb().size if mesh != null else Vector3.ZERO
	_check("ground-geometry",
		ground != null and gpos.distance_to(GROUND_CENTER) < GEO_TOL
		and box != null and box_size.distance_to(GROUND_SIZE) < GEO_TOL
		and mesh != null and aabb_size.distance_to(GROUND_SIZE) < GEO_TOL,
		"pos=%s(期望 %s) box=%s aabb=%s" % [str(gpos), str(GROUND_CENTER),
			str(box_size), str(aabb_size)])

	# ---- S29 体素格数（几何不变）
	var cells: int = voxel.get_used_cells().size() if voxel != null else -1
	_check("voxel-cells", cells > CELLS_MIN and cells < CELLS_MAX,
		"cells=%d(%d~%d)" % [cells, CELLS_MIN, CELLS_MAX])

	# ---- S30 降级材质可用 + Apply_World_Look 幂等（不换实例、不泄漏）
	var fb: StandardMaterial3D = WorldLook.Make_Fallback_Voxel_Material(Color(0.55, 0.58, 0.62, 1.0))
	var fb_ok: bool = fb != null and fb.albedo_texture == null and fb.normal_texture == null \
		and not fb.uv1_triplanar \
		and fb.albedo_color.is_equal_approx(Color(0.55, 0.58, 0.62, 1.0))
	var ground_before: Material = mi.get_surface_override_material(0) if mi != null else null
	var sky_before: Material = env.sky.sky_material if env != null and env.sky != null else null
	var again: bool = bool(_world.Apply_World_Look())
	var ground_after: Material = mi.get_surface_override_material(0) if mi != null else null
	var sky_after: Material = env.sky.sky_material if env != null and env.sky != null else null
	_check("fallback-and-idempotent",
		fb_ok and again and ground_after == ground_before and sky_after == sky_before,
		"fallback=%s apply_again=%s ground_same=%s sky_same=%s" % [
			str(fb_ok), str(again), str(ground_after == ground_before),
			str(sky_after == sky_before)])


# ================================================================ R 组：真渲染 13 项

## 先闸后 await（E26-③）：先建机位拿到 CameraSensor，再问 Can_Capture()。
func _Cam_Can_Capture() -> bool:
	var shot: Dictionary = SHOTS[0]
	_Make_Holder(shot["eye"], shot["aim"])
	await process_frame
	await process_frame
	return _cam != null and bool(_cam.Can_Capture())


## 三张 1600×900 + 两张 160×90 探针 -> 13 条断言。
func _Run_R13() -> void:
	_Ensure_Out_Dir()
	for i in range(SHOTS.size()):
		await _Capture_Shot(i)
	for i in range(SHOTS.size()):
		var labels: Array[String] = _Shot_Labels(i)
		_Check_Shot_File(i, labels[0])
		_Check_Shot_Size(i, labels[1])
		_Check_Shot_Pixels(i, labels[2])

	_Check_Pairs()
	# **必须 await**：不 await 会让 _Teardown() 与探针并发，_cam 被提前释放（实测 base 'Nil'）
	await _Check_Probes()


## 单机位：移 holder -> Configure -> 等 2 帧 -> 抓拍落盘。
func _Capture_Shot(idx: int) -> void:
	var shot: Dictionary = SHOTS[idx]
	var path: String = "%s/%s" % [OUT_DIR, String(shot["file"])]
	_Place_Holder(shot["eye"], shot["aim"])
	_cam.Configure({"width": SHOT_SIZE.x, "height": SHOT_SIZE.y,
		"fov_deg": SHOT_FOV, "quality": SHOT_QUALITY})
	await process_frame
	await process_frame
	var saved: bool = await _cam.Capture_To_File(path)
	_shots.append({
		"id": int(shot["id"]),
		"path": path,
		"saved": saved,
		"bytes": _File_Size(path),
		"jpg": FileAccess.get_file_as_bytes(path),
	})
	print("[check] capture shot-%02d saved=%s cam=%s" % [int(shot["id"]), str(saved), path])


func _Check_Shot_File(idx: int, label: String) -> void:
	var data: Dictionary = _shots[idx]
	var path: String = String(data["path"])
	var bytes: int = int(data["bytes"])
	var jpg: PackedByteArray = data["jpg"]
	var head: String = jpg.slice(0, 3).hex_encode() if jpg.size() >= 3 else ""
	_check(label, bool(data["saved"]) and bytes > MIN_BYTES and head == "ffd8ff",
		"bytes=%d(>%d) head=%s saved=%s path=%s" % [
			bytes, MIN_BYTES, head, str(data["saved"]), path])


func _Check_Shot_Size(idx: int, label: String) -> void:
	var img: Image = _Decode(_shots[idx]["jpg"])
	var size: Vector2i = img.get_size() if img != null and not img.is_empty() else Vector2i(-1, -1)
	_check(label, size == SHOT_SIZE, "decoded=%dx%d(期望 %dx%d)" % [
		size.x, size.y, SHOT_SIZE.x, SHOT_SIZE.y])


func _Check_Shot_Pixels(idx: int, label: String) -> void:
	var img: Image = _Decode(_shots[idx]["jpg"])
	if img == null or img.is_empty():
		_check(label, false, "JPEG 解码失败：%s" % String(_shots[idx]["path"]))
		return
	var st: Dictionary = _Gray_Stats(img)
	var mean: float = float(st["mean"])
	var variance: float = float(st["var"])
	var spread: float = float(st["max"]) - float(st["min"])
	var white: float = float(st["white"])
	_check(label, variance > VAR_MIN and mean > MEAN_MIN and mean < MEAN_MAX
		and spread > SPREAD_MIN and white < WHITE_MAX,
		"var=%.6f(>%.3f) mean=%.4f(%.2f~%.2f) spread=%.3f(>%.2f) white=%.3f(<%.2f)" % [
			variance, VAR_MIN, mean, MEAN_MIN, MEAN_MAX, spread, SPREAD_MIN,
			white, WHITE_MAX])


## 三张两两不同（字节差异 + 灰度差异都要过）。
func _Check_Pairs() -> void:
	var pairs: Array = [[0, 1], [0, 2], [1, 2]]
	var details: Array[String] = []
	var ok: bool = _shots.size() == SHOTS.size()
	for pair: Array in pairs:
		var a: PackedByteArray = _shots[pair[0]]["jpg"]
		var b: PackedByteArray = _shots[pair[1]]["jpg"]
		var diff: Dictionary = _Pair_Diff(a, b)
		var byte_ratio: float = float(diff["byte_ratio"])
		var mean_abs: float = float(diff["mean_abs"])
		if byte_ratio <= DIFF_BYTE_MIN or mean_abs <= DIFF_MEAN_MIN:
			ok = false
		details.append("%d~%d byte=%.4f(>%.2f) mean_abs=%.5f(>%.2f)" % [
			pair[0], pair[1], byte_ratio, DIFF_BYTE_MIN, mean_abs, DIFF_MEAN_MIN])
	_check("shots-distinct", ok, "；".join(details))


## 太阳方向探针：同一台相机（10° / 160×90）对准 SUN_DIR 与 -SUN_DIR 各拍一张，比均值。
func _Check_Probes() -> void:
	var eye_g: Vector3 = WorldTransform.PosToGodot(PROBE_EYE_WORLD)
	var sun_dir: Vector3 = WorldLook.SUN_DIR.normalized()
	var sun_path: String = "%s/probe_sun.jpg" % OUT_DIR
	var anti_path: String = "%s/probe_anti.jpg" % OUT_DIR

	# 对照方向 = **同一仰角、反方位**（不是 -SUN_DIR：那会指向地平线以下、拍到地面，
	# 实测 mean=0.8065 把对照判据打红；plan.md 写的 -SUN_DIR 与“反日天空”不等价，见 impl.md 偏差）
	var anti_dir := Vector3(-sun_dir.x, sun_dir.y, -sun_dir.z)
	var probes: Array = [
		{"path": sun_path, "dir": sun_dir},
		{"path": anti_path, "dir": anti_dir},
	]
	for raw: Variant in probes:
		var spec: Dictionary = raw
		var aim_dir: Vector3 = spec["dir"]
		_holder.global_position = eye_g
		_holder.look_at(eye_g + aim_dir, Vector3.UP)
		_cam.Configure({"width": PROBE_SIZE.x, "height": PROBE_SIZE.y,
			"fov_deg": PROBE_FOV, "quality": SHOT_QUALITY})
		await process_frame
		await process_frame
		var saved: bool = await _cam.Capture_To_File(String(spec["path"]))
		var img: Image = _Decode(FileAccess.get_file_as_bytes(String(spec["path"])))
		var st: Dictionary = _Gray_Stats(img) if img != null and not img.is_empty() else {}
		var mean: float = float(st.get("mean", -1.0))
		_probe_means[String(spec["path"])] = mean
		print("[check] probe %s saved=%s mean=%.4f" % [String(spec["path"]), str(saved), mean])

	var sun_mean: float = float(_probe_means.get(sun_path, -1.0))
	var anti_mean: float = float(_probe_means.get(anti_path, -1.0))
	_check("sun-probe-sun", sun_mean >= PROBE_MEAN_MIN,
		"mean=%.4f(>=%.2f) %s" % [sun_mean, PROBE_MEAN_MIN, sun_path])
	_check("sun-probe-control", anti_mean >= 0.0 and anti_mean <= PROBE_CTRL_MAX,
		"mean=%.4f(<=%.2f) %s" % [anti_mean, PROBE_CTRL_MAX, anti_path])
	_check("sun-probe-margin", (sun_mean - anti_mean) >= PROBE_MARGIN_MIN,
		"diff=%.4f(>=%.2f) sun=%.4f anti=%.4f" % [
			sun_mean - anti_mean, PROBE_MARGIN_MIN, sun_mean, anti_mean])


## headless：R 组整体 SKIP（**不计入 total**，否则断言数和基线错位）。
func _Skip_R13() -> void:
	for label: String in _Render_Labels():
		print("[skip] %s（无渲染器：display=%s adapter='%s'）" % [
			label, DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])
		_skipped += 1


## R 组全部 label（13 项），与 _Check_Shot_* / 全局断言逐字同源，防两处漂移。
func _Render_Labels() -> Array[String]:
	var labels: Array[String] = []
	for i in range(SHOTS.size()):
		labels.append_array(_Shot_Labels(i))
	labels.append_array(["shots-distinct", "sun-probe-sun", "sun-probe-control", "sun-probe-margin"])
	return labels


func _Shot_Labels(idx: int) -> Array[String]:
	var tag: String = "shot-%02d" % int(SHOTS[idx]["id"])
	return ["%s-file" % tag, "%s-size" % tag, "%s-pixels" % tag]


# ================================================================ 机位 / 抓拍工具

## 机位 = 世界级 holder（不挂车、不挂世界外），CameraSensor 作为其子节点。
func _Make_Holder(eye_world: Vector3, aim_world: Vector3) -> Node3D:
	var holder := Node3D.new()
	holder.name = "LookCamera"
	_world.add_child(holder)
	_holder = holder
	_Place_Holder(eye_world, aim_world)
	var packed: PackedScene = load(CAMERA_SCENE) as PackedScene
	if packed != null:
		_cam = packed.instantiate() as Node3D
		if _cam != null:
			_cam.name = "CameraSensor"
			holder.add_child(_cam)
	return holder


## 移动既有 holder（同一台相机换机位）：位置换轴 + look_at。
func _Place_Holder(eye_world: Vector3, aim_world: Vector3) -> void:
	if _holder == null or not is_instance_valid(_holder):
		return
	_holder.global_position = WorldTransform.PosToGodot(eye_world)
	_holder.look_at(WorldTransform.PosToGodot(aim_world), Vector3.UP)


# ================================================================ 图像统计 / 工具

## 灰度统计（2 像素步长抽样）：`{mean, var, min, max, white}`。
func _Gray_Stats(img: Image) -> Dictionary:
	var sum_l: float = 0.0
	var sum2: float = 0.0
	var count: int = 0
	var lo: float = 1.0
	var hi: float = 0.0
	var white: int = 0
	for y in range(0, img.get_height(), 2):
		for x in range(0, img.get_width(), 2):
			var l: float = _Lum(img.get_pixel(x, y))
			sum_l += l
			sum2 += l * l
			lo = minf(lo, l)
			hi = maxf(hi, l)
			if l > 0.95:
				white += 1
			count += 1
	if count == 0:
		return {"mean": -1.0, "var": -1.0, "min": -1.0, "max": -1.0, "white": -1.0}
	var mean: float = sum_l / float(count)
	return {
		"mean": mean,
		"var": maxf(sum2 / float(count) - mean * mean, 0.0),
		"min": lo,
		"max": hi,
		"white": float(white) / float(count),
	}


## 两张 JPEG 的差异：`mean_abs` = 8 像素步长的灰度差均值、`byte_ratio` = 逐字节不同占比。
func _Pair_Diff(a: PackedByteArray, b: PackedByteArray) -> Dictionary:
	var img_a: Image = _Decode(a)
	var img_b: Image = _Decode(b)
	var mean_abs: float = -1.0
	if img_a != null and img_b != null and not img_a.is_empty() and not img_b.is_empty():
		var sum_l: float = 0.0
		var n: int = 0
		var h: int = mini(img_a.get_height(), img_b.get_height())
		var w: int = mini(img_a.get_width(), img_b.get_width())
		for y in range(0, h, DIFF_STEP):
			for x in range(0, w, DIFF_STEP):
				sum_l += absf(_Lum(img_a.get_pixel(x, y)) - _Lum(img_b.get_pixel(x, y)))
				n += 1
		mean_abs = sum_l / float(maxi(n, 1))
	var total: int = mini(a.size(), b.size())
	var diff: int = 0
	for i in range(total):
		if a[i] != b[i]:
			diff += 1
	return {"mean_abs": mean_abs, "byte_ratio": float(diff) / float(maxi(total, 1))}


func _Decode(jpg: PackedByteArray) -> Image:
	if jpg.is_empty():
		return null
	var img := Image.new()
	var err: Error = img.load_jpg_from_buffer(jpg)
	if err != OK:
		return null
	return img


func _Lum(c: Color) -> float:
	return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b


## 贴图描述（路径 + 尺寸），detail 用；null 打 "null"。
func _Tex_Desc(tex: Texture2D) -> String:
	if tex == null:
		return "null"
	return "%s %dx%d" % [tex.resource_path, tex.get_width(), tex.get_height()]


## 建落盘目录（已存在 = 成功）；失败只返回 false、**绝不打印 ERROR**。
func _Ensure_Out_Dir() -> bool:
	if OUT_DIR.is_empty() or not OUT_DIR.is_absolute_path():
		return false
	if DirAccess.dir_exists_absolute(OUT_DIR):
		return true
	DirAccess.make_dir_absolute(OUT_DIR)
	if not DirAccess.dir_exists_absolute(OUT_DIR):
		var parent: String = OUT_DIR.get_base_dir()
		if not parent.is_empty() and not DirAccess.dir_exists_absolute(parent):
			DirAccess.make_dir_absolute(parent)
		DirAccess.make_dir_absolute(OUT_DIR)
	return DirAccess.dir_exists_absolute(OUT_DIR)


func _File_Size(path: String) -> int:
	if not FileAccess.file_exists(path):
		return -1
	var handle: FileAccess = FileAccess.open(path, FileAccess.READ)
	if handle == null:
		return -2
	var size_b: int = handle.get_length()
	handle.close()
	return size_b


# ================================================================ 释放 / 断言 / 收尾

## 释放顺序：相机 -> holder -> 世界，每步 await 两帧
## （漏了会报 `ERROR: resources still in use at exit`，照 check_camera_shots.gd:944-964）。
func _Teardown() -> void:
	if _cam != null and is_instance_valid(_cam):
		_cam.queue_free()
		_cam = null
		await process_frame
		await process_frame
	if _holder != null and is_instance_valid(_holder):
		_holder.queue_free()
		_holder = null
		await process_frame
		await process_frame
	if _world != null and is_instance_valid(_world):
		_world.queue_free()
		_world = null
		await process_frame
		await process_frame


func _check(label: String, ok: bool, detail: String = "") -> bool:
	_total += 1
	if not ok:
		_failed += 1
	var suffix: String = "" if detail.is_empty() else " " + detail
	if ok:
		print("[check] %s ok%s" % [label, suffix])
	else:
		print("[check] %s FAIL%s" % [label, suffix])
	return ok


func _finish() -> void:
	print("[check] env display=%s adapter='%s' shots=%d" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name(), _shots.size()])
	print("[check] out_dir=%s" % OUT_DIR)
	if _skipped > 0:
		print("[check] skipped=%d（R 组：无真渲染器，不计入 total）" % _skipped)
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
