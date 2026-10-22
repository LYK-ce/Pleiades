# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# 工程骨架自检（非组件）：校验主场景与 [sim_world] 关键设置可被引擎读到。
# 用法：godot --headless --path orion-simulation-world --script res://tests/check_project.gd

extends SceneTree

const EXPECTED_SETTINGS := {
	"sim_world/network/ws_port": 9999,
	"sim_world/world/voxel_size": 0.5,
	"sim_world/world/cells_x": 256,
	"sim_world/world/cells_y": 256,
	"sim_world/world/layers": 64,
	"physics/common/physics_ticks_per_second": 60,
}


func _initialize() -> void:
	var failed: int = 0
	var version := str(Engine.get_version_info().get("string", ""))
	print("[check] engine=", version)
	if not version.begins_with("4.7"):
		print("[fail] 引擎版本不是 4.7.x")
		failed += 1

	var main_scene := str(ProjectSettings.get_setting("application/run/main_scene", ""))
	print("[check] main_scene=", main_scene)
	if main_scene.is_empty() or not ResourceLoader.exists(main_scene):
		print("[fail] 主场景缺失或不可加载")
		failed += 1

	for key in EXPECTED_SETTINGS:
		var actual = ProjectSettings.get_setting(key, null)
		var want = EXPECTED_SETTINGS[key]
		if actual == null:
			print("[fail] 缺少设置 ", key)
			failed += 1
			continue
		var ok: bool = float(actual) == float(want)
		print("[check] ", key, "=", actual, " ", "ok" if ok else "MISMATCH(want %s)" % [want])
		if not ok:
			failed += 1

	if failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", failed)
		quit(1)
