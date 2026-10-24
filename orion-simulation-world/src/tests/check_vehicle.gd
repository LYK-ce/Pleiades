# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-23
#
# Vehicle 组件自检（Step 4）：场景形态 / 空车稳定 / 装配（底盘 + 传感器）/ 运动 / 位姿 / 边界。
# 用法：timeout 180 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_vehicle.gd
# 期望：total=40 failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR。
#
# 决定本文件写法的实测事实（plan.md §2）：
#   P4  PackedScene.pack() 可以打包"运行时 set_script 的 Node3D"当底盘/传感器替身（不落 .tscn）
#   P12 按 §6.1 顺序释放并等帧，退出时无 ERROR / ObjectDB leaked
#   P15 含 await 的辅助函数必须被 await 调用；本文件所有辅助函数都不含 await，await 只内联在 _run
#   P16 一次 null 解引用会让协程中断、quit() 不执行 → 入口一律先判空，命令一律带 timeout
#   P18 节点上未知方法是动态的：一律 .get(...) / .call(...) 访问替身与脚本方法
#   P20 task_33 Step 3：占位尺寸 = 车体模型 AABB（0.30×0.25×0.25，D11）；
#       mount.pitch 单位度、正 = 抬头 / 负 = 低头、缺省 0（不预置任何角度）
#   帧数相关断言只用 --fixed-fps 60 下的累计量（30 帧 = 0.5 s），不逐帧比对
#
# 注意（GDScript 4.7 类型推断陷阱）：instantiate() / load() 的返回值一律显式标注类型。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")

const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const VEHICLE_SCRIPT := "res://src/vehicle/Vehicle.gd"
const WORLD_SCENE := "res://src/scenes/World.tscn"
const DOUBLE_CHASSIS := "res://src/tests/doubles/TestChassis.gd"
const DOUBLE_SENSOR := "res://src/tests/doubles/TestSensor.gd"

const TOL := 0.001
## A18/A19：30 帧 = 0.5 s；替身 0.5 m/s + 0.2 rad/s → 位移 0.25 m、rotation.y +0.10 rad。
const MOVE_FRAMES := 30
const WANT_MOVE_M := 0.25
const WANT_YAW_GODOT := 0.10

## A33 白盒 lint：Vehicle 里不允许出现的符号（越界做 Step 5/6/8 的事，或违反 §9.1）。
const BANNED_TOKENS := [
	"class_name", "push_error(", "intersect_ray", "direct_space_state",
	"PhysicsRayQueryParameters3D", "WorldToCell", "WebSocketPeer",
]

## 出生点：世界坐标（Z-up）= pleiades-sim 的 DEFAULT_ORIGIN。
const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)

var _total: int = 0
var _failed: int = 0
var _harness_errors: int = 0

# 信号计数（A12 / A21）
var _chassis_signal_count: int = 0
var _chassis_signal_arg: String = ""
var _sensor_signal_count: int = 0
var _sensor_signal_arg: String = ""


func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ 0. 场景探测（树外）
	var vehicle_packed: PackedScene = load(VEHICLE_SCENE)
	var probe: Node = null
	if vehicle_packed != null:
		probe = vehicle_packed.instantiate()
	if probe == null:
		_fatal("scene-loads", "%s 不可加载或实例化" % VEHICLE_SCENE)
		_finish()
		return

	_check("scene-loads", vehicle_packed != null, VEHICLE_SCENE)

	# A2 根节点：名字 / 类型 / 脚本
	var probe_script: Script = probe.get_script()
	_check("root-type", probe.name == "Vehicle" and probe is CharacterBody3D
		and probe_script != null and probe_script.resource_path == VEHICLE_SCRIPT,
		"name=%s class=%s script=%s" % [probe.name, probe.get_class(),
			probe_script.resource_path if probe_script != null else "null"])

	# A4 节点树（空车 = 壳 + 两个空挂载点）
	var n_body: Node = probe.get_node_or_null("Body")
	var n_nose: Node = probe.get_node_or_null("Body/Nose")
	var n_shape: Node = probe.get_node_or_null("CollisionShape3D")
	var n_chassis: Node = probe.get_node_or_null("Chassis")
	var n_sensors: Node = probe.get_node_or_null("Sensors")
	_check("node-tree", n_body is MeshInstance3D and n_nose is MeshInstance3D
		and n_shape is CollisionShape3D and n_chassis is Node3D and n_sensors is Node3D
		and n_chassis.get_child_count() == 0 and n_sensors.get_child_count() == 0,
		"body=%s nose=%s shape=%s chassis=%s sensors=%s" % [n_body, n_nose, n_shape,
			n_chassis, n_sensors])

	# A5 E16 护栏：4 个 @export 节点引用已在实例化时绑定（树外，_ready 尚未跑）
	var exp_body: Variant = probe.get("body")
	var exp_shape: Variant = probe.get("collision_shape")
	var exp_chassis: Variant = probe.get("chassis_root")
	var exp_sensors: Variant = probe.get("sensors_root")
	_check("exports-bound", exp_body != null and exp_body == n_body
		and exp_shape != null and exp_shape == n_shape
		and exp_chassis != null and exp_chassis == n_chassis
		and exp_sensors != null and exp_sensors == n_sensors,
		"body=%s shape=%s chassis=%s sensors=%s" % [exp_body, exp_shape, exp_chassis,
			exp_sensors])

	# A6 原点约定：原点 = 底面中心 → body/shape 都上抬 height/2
	var origin_ok := false
	var origin_detail := "body/shape 缺失"
	var body_probe: MeshInstance3D = n_body as MeshInstance3D
	var shape_probe: CollisionShape3D = n_shape as CollisionShape3D
	if body_probe != null and shape_probe != null and body_probe.mesh is BoxMesh:
		var mesh_probe: BoxMesh = body_probe.mesh as BoxMesh
		var shape_box: BoxShape3D = shape_probe.shape as BoxShape3D
		var half_h: float = mesh_probe.size.y * 0.5
		origin_ok = absf(body_probe.position.y - half_h) < 1e-4 \
			and absf(shape_probe.position.y - half_h) < 1e-4 \
			and shape_box != null and shape_box.size.is_equal_approx(mesh_probe.size)
		origin_detail = "mesh=%s body.y=%.4f shape.y=%.4f half=%.4f" % [mesh_probe.size,
			body_probe.position.y, shape_probe.position.y, half_h]
	_check("origin-convention", origin_ok, origin_detail)

	# A6b 占位尺寸（task_33 Step 3）：空车占位与车体模型 AABB 逐轴一致（0.30 × 0.25 × 0.25）
	var ph_ok := false
	var ph_detail := "占位缺失"
	if body_probe != null and shape_probe != null and body_probe.mesh is BoxMesh \
			and shape_probe.shape is BoxShape3D:
		var ph_mesh: BoxMesh = body_probe.mesh as BoxMesh
		var ph_shape: BoxShape3D = shape_probe.shape as BoxShape3D
		ph_ok = ph_mesh.size.is_equal_approx(Vector3(0.30, 0.25, 0.25)) \
			and ph_shape.size.is_equal_approx(Vector3(0.30, 0.25, 0.25))
		ph_detail = "mesh=%s shape=%s" % [ph_mesh.size, ph_shape.size]
	_check("placeholder-size", ph_ok, ph_detail)

	# A7 树外 instantiate + free：NodePath 解析不依赖场景树
	var probe2: Node = vehicle_packed.instantiate()
	var outside_ok: bool = probe2 != null and probe2.get_node_or_null("Chassis") != null
	if probe2 != null:
		probe2.free()
	_check("instances-outside-tree", outside_ok, "树外实例化/释放无副作用")

	probe.free()

	# ============================================================ 1. 世界 + 空车
	var world_packed: PackedScene = load(WORLD_SCENE)
	if world_packed == null:
		_fatal("world-loads", WORLD_SCENE)
		_finish()
		return
	var world: Node3D = world_packed.instantiate()
	root.add_child(world)
	await physics_frame
	await physics_frame
	var vehicles_root: Node3D = world.get_node_or_null("Vehicles") as Node3D
	if vehicles_root == null:
		_fatal("vehicles-root", "World/Vehicles 缺失")
		_finish()
		return

	var vehicle: CharacterBody3D = vehicle_packed.instantiate()
	vehicles_root.add_child(vehicle)
	vehicle.global_position = WorldTransform.PosToGodot(SPAWN_WORLD)
	vehicle.rotation.y = 0.0
	vehicle.connect("chassis_installed", _on_chassis_installed)
	vehicle.connect("sensor_attached", _on_sensor_attached)
	await physics_frame
	await physics_frame

	var chassis_root: Node3D = vehicle.chassis_root
	var sensors_root: Node3D = vehicle.sensors_root
	if chassis_root == null or sensors_root == null:
		_fatal("mount-points", "chassis_root=%s sensors_root=%s" % [chassis_root, sensors_root])
		_finish()
		return

	# A3 组 + 物理层（场景里写死，脚本不重复设置）
	_check("group-layers", vehicle.is_in_group(&"vehicle")
		and vehicle.collision_layer == 4 and vehicle.collision_mask == 7,
		"group=%s layer=%d mask=%d" % [vehicle.is_in_group(&"vehicle"),
			vehicle.collision_layer, vehicle.collision_mask])

	# A8 空车稳定：无底盘时 _physics_process 直接 return，30 帧不漂移
	var y0: float = vehicle.global_position.y
	await _wait_physics(30)
	_check("empty-car-stable", absf(vehicle.global_position.y - y0) < 1e-4
		and vehicle.velocity.is_zero_approx() and is_instance_valid(vehicle),
		"dy=%.6f v=%.6f" % [absf(vehicle.global_position.y - y0), vehicle.velocity.length()])

	# A9 出生位姿：Godot (64,0,64) → 世界 (64,64,0)，yaw 0
	var pose_spawn: Dictionary = vehicle.GetPose()
	var pose_alias: Dictionary = vehicle.get_pose()
	_check("pose-spawn", absf(float(pose_spawn.get("x", -1.0)) - 64.0) < TOL
		and absf(float(pose_spawn.get("y", -1.0)) - 64.0) < TOL
		and absf(float(pose_spawn.get("z", -1.0))) < TOL
		and absf(float(pose_spawn.get("yaw", 99.0))) < TOL
		and pose_alias == pose_spawn,
		"%s" % [pose_spawn])

	# A10 yaw 符号：rotation.y = -PI/2 → 世界 yaw = +PI/2
	vehicle.rotation.y = -PI * 0.5
	var pose_yaw: float = float(vehicle.GetPose().get("yaw", 0.0))
	_check("pose-yaw-sign", absf(pose_yaw - PI * 0.5) < TOL,
		"rotation.y=-PI/2 -> yaw=%.4f (want +%.4f)" % [pose_yaw, PI * 0.5])
	vehicle.rotation.y = 0.0

	# ============================================================ 2. 挂底盘
	var chassis_scene: PackedScene = _pack_double(DOUBLE_CHASSIS, "TestChassis")

	# A11 装配
	var applied: bool = vehicle.ApplyChassis("test_chassis", {"max_speed": 2.0}, chassis_scene)
	var chassis_child: Node = null
	if chassis_root.get_child_count() > 0:
		chassis_child = chassis_root.get_child(0)
	_check("apply-chassis", applied and vehicle.HasChassis()
		and vehicle.GetChassisName() == "test_chassis"
		and chassis_root.get_child_count() == 1
		and chassis_child != null and chassis_child.name == "TestChassis",
		"applied=%s name=%s children=%d child=%s" % [applied, vehicle.GetChassisName(),
			chassis_root.get_child_count(), chassis_child])

	if chassis_child == null:
		_fatal("chassis-double", "测试替身未挂上，后续断言无法进行")
		_finish()
		return

	# A12 信号
	_check("chassis-signal", _chassis_signal_count == 1 and _chassis_signal_arg == "test_chassis",
		"count=%d arg=%s" % [_chassis_signal_count, _chassis_signal_arg])

	# A13 Configure(self, 合并参数)：默认值 <- 传入值（底盘收到的 = 车辆记下的）
	var cfg_vehicle: Variant = chassis_child.get("configured_vehicle")
	var cfg_params: Dictionary = chassis_child.get("configured_params")
	var effective_params: Dictionary = vehicle.GetChassisParams()
	_check("chassis-configure", cfg_vehicle == vehicle and effective_params == cfg_params
		and absf(float(cfg_params.get("max_speed", 0.0)) - 2.0) < TOL
		and absf(float(cfg_params.get("wheel_base", 0.0)) - 0.30) < TOL,
		"vehicle_ok=%s params=%s effective=%s" % [cfg_vehicle == vehicle, cfg_params,
			effective_params])

	# A14 拒绝：名字解析不到场景（Step 5 未落地 + 未显式传 scene）→ 状态不变
	var name_before: String = vehicle.GetChassisName()
	var count_before: int = chassis_root.get_child_count()
	var rejected_null: bool = vehicle.ApplyChassis("no_such_chassis", {}, null)
	_check("apply-chassis-reject-null", not rejected_null
		and vehicle.GetChassisName() == name_before
		and chassis_root.get_child_count() == count_before,
		"rejected=%s name=%s children=%d" % [rejected_null, vehicle.GetChassisName(),
			chassis_root.get_child_count()])

	# A15 拒绝：根节点不是 Node3D（Control）→ 状态不变
	var bogus_scene: PackedScene = _pack_bogus()
	var rejected_type: bool = vehicle.ApplyChassis("bogus", {}, bogus_scene)
	_check("apply-chassis-reject-type", not rejected_type
		and vehicle.GetChassisName() == name_before
		and chassis_root.get_child_count() == count_before,
		"rejected=%s name=%s children=%d" % [rejected_type, vehicle.GetChassisName(),
			chassis_root.get_child_count()])

	# A16 换底盘：先摘旧再挂新（同帧不会改名成 TestChassis2）
	var old_chassis: Node = chassis_child
	var replaced: bool = vehicle.ApplyChassis("test_chassis_2", {}, chassis_scene)
	await _wait_physics(1)
	var new_child: Node = null
	if chassis_root.get_child_count() > 0:
		new_child = chassis_root.get_child(0)
	_check("replace-chassis", replaced and chassis_root.get_child_count() == 1
		and new_child != null and new_child != old_chassis
		and not is_instance_valid(old_chassis),
		"children=%d new=%s old_valid=%s" % [chassis_root.get_child_count(), new_child,
			is_instance_valid(old_chassis)])

	if new_child == null:
		_fatal("chassis-replace", "换底盘后没有子节点，后续断言无法进行")
		_finish()
		return

	# A17 运动指令转发（canonical + snake_case 别名）
	vehicle.SetMotionCommand({"action": "forward", "speed": 50})
	var cmd_forward: String = str(new_child.get("last_cmd").get("action", ""))
	vehicle.apply_motion({"action": "left", "speed": 80})
	var cmd_left: String = str(new_child.get("last_cmd").get("action", ""))
	_check("motion-forwarded", cmd_forward == "forward" and cmd_left == "left",
		"forward=%s left=%s" % [cmd_forward, cmd_left])

	# A18 物理步进：30 帧 → 位移 0.25 m、rotation.y +0.10（左转 = rotation.y 增大）
	var x_before: float = vehicle.global_position.x
	var rot_before: float = vehicle.rotation.y
	await _wait_physics(MOVE_FRAMES)
	var dx: float = vehicle.global_position.x - x_before
	var drot: float = vehicle.rotation.y - rot_before
	_check("physics-step", absf(dx - WANT_MOVE_M) < 0.02
		and absf(drot - WANT_YAW_GODOT) < 0.01,
		"dx=%.4f (want %.2f) d_rot_y=%.4f (want %.2f)" % [dx, WANT_MOVE_M, drot,
			WANT_YAW_GODOT])

	# A19 位姿跟随：世界 x 前进、世界 yaw 与 rotation.y 反号
	var pose_moved: Dictionary = vehicle.GetPose()
	_check("pose-follows", float(pose_moved.get("x", 0.0)) > 64.2
		and absf(float(pose_moved.get("yaw", 0.0)) + WANT_YAW_GODOT) < 0.01,
		"%s (want x>64.2, yaw~-%.2f)" % [pose_moved, WANT_YAW_GODOT])

	# A20 底盘被外部释放：不崩、速度归零、只断引用（名字保留）
	var name_at_free: String = vehicle.GetChassisName()
	var doomed: Node = chassis_root.get_child(0)
	if doomed != null:
		doomed.queue_free()
	await _wait_physics(10)
	_check("chassis-free-guard", is_instance_valid(vehicle)
		and vehicle.velocity.is_zero_approx() and not vehicle.HasChassis()
		and vehicle.GetChassisName() == name_at_free
		and chassis_root.get_child_count() == 0,
		"v=%.6f has=%s name=%s children=%d" % [vehicle.velocity.length(),
			vehicle.HasChassis(), vehicle.GetChassisName(), chassis_root.get_child_count()])

	# ============================================================ 3. 挂传感器（此时无底盘）
	var sensor_scene: PackedScene = _pack_double(DOUBLE_SENSOR, "TestSensor")

	# A21 挂载 + 命名 + 信号
	var sensor_first: Node = vehicle.AttachSensor("lidar_front", sensor_scene,
		{"x": 0.1, "y": 0.0, "z": 0.2, "yaw": 0.0}, {"rays": 360})
	var sensor_holder: Node3D = sensor_first as Node3D
	_check("attach-sensor", sensor_holder != null
		and sensors_root.get_node_or_null("lidar_front") == sensor_holder
		and _sensor_signal_count == 1,
		"node=%s signal=%d" % [sensor_holder, _sensor_signal_count])

	# A22 mount 语义：车体局部 + 世界轴向（x 前 / z 高 / yaw 绕 z）
	var mount_ok := false
	var mount_detail := "传感器缺失"
	if sensor_holder != null:
		var mount_pos: Vector3 = sensor_holder.position
		mount_ok = mount_pos.distance_to(Vector3(0.1, 0.2, 0.0)) < TOL
		mount_detail = "pos=%s" % [mount_pos]
		var prev_sensor: Node3D = sensor_holder
		var sensor_yaw: Node = vehicle.AttachSensor("lidar_front", sensor_scene,
			{"yaw": PI * 0.5}, {"rays": 360})
		var sensor_yaw_holder: Node3D = sensor_yaw as Node3D
		mount_ok = mount_ok and sensor_yaw_holder != null and sensor_yaw_holder != prev_sensor \
			and absf(sensor_yaw_holder.rotation.y + PI * 0.5) < TOL
		mount_detail += " yaw_node=%s rot_y=%.4f" % [sensor_yaw_holder,
			sensor_yaw_holder.rotation.y if sensor_yaw_holder != null else 0.0]
		sensor_holder = sensor_yaw_holder
	_check("mount-mapping", mount_ok, mount_detail)

	if sensor_holder == null:
		_fatal("sensor-double", "测试替身未挂上，后续断言无法进行")
		_finish()
		return

	# A22b/A22c/A22d mount.pitch（task_33 Step 3）：单位度；正 = 抬头、负 = 低头；缺省 0 = 不预置角度。
	#   语义依据：光轴 = 传感器局部 −Z，rotation.x = pitch（欧拉序 YXZ）。
	#   三次探针同一 slot（先卸后挂）且都传 rays=360（A23 要读 configured_params）。
	var pitch_default_ok := false
	var pitch_default_detail := "缺省挂载失败"
	var pitch_down_ok := false
	var pitch_down_detail := "挂载失败"
	var pitch_up_ok := false
	var pitch_up_detail := "挂载失败"
	var s_default: Node3D = vehicle.AttachSensor("lidar_front", sensor_scene,
		{"x": 0.1, "z": 0.2}, {"rays": 360}) as Node3D
	if s_default != null:
		pitch_default_ok = s_default.rotation.x == 0.0 and absf(s_default.rotation.y) < TOL
		pitch_default_detail = "rot_x=%.6f rot_y=%.6f" % [s_default.rotation.x,
			s_default.rotation.y]
	var s_down: Node3D = vehicle.AttachSensor("lidar_front", sensor_scene,
		{"yaw": 0.0, "pitch": -30.0}, {"rays": 360}) as Node3D
	if s_down != null:
		pitch_down_ok = absf(s_down.rotation_degrees.x + 30.0) < 1e-3 \
			and absf(s_down.rotation.y) < TOL \
			and absf((-s_down.global_basis.z).y + 0.5) < 1e-3
		pitch_down_detail = "rot_x=%.2f axis_y=%.4f" % [s_down.rotation_degrees.x,
			(-s_down.global_basis.z).y]
	var s_up: Node3D = vehicle.AttachSensor("lidar_front", sensor_scene,
		{"pitch": 15.0}, {"rays": 360}) as Node3D
	if s_up != null:
		var want_up: float = sin(deg_to_rad(15.0))
		pitch_up_ok = absf((-s_up.global_basis.z).y - want_up) < 1e-3
		pitch_up_detail = "axis_y=%.4f want=%.4f" % [(-s_up.global_basis.z).y, want_up]
	_check("mount-pitch-default", pitch_default_ok, pitch_default_detail)
	_check("mount-pitch-down", pitch_down_ok, pitch_down_detail)
	_check("mount-pitch-up", pitch_up_ok, pitch_up_detail)
	# 挂回无 pitch 的 mount：A23 读 configured_params.rays、A24 要求 slots.size()==1、A25 抽样。
	sensor_holder = vehicle.AttachSensor("lidar_front", sensor_scene,
		{"x": 0.1, "y": 0.0, "z": 0.2, "yaw": 0.0}, {"rays": 360}) as Node3D
	if sensor_holder == null:
		_fatal("sensor-remount", "pitch 探针后重挂失败，后续断言无法进行")
		_finish()
		return

	# A23 Configure(params) 已转发
	var sensor_params: Variant = sensor_holder.get("configured_params")
	var rays_now: int = 0
	if sensor_params is Dictionary:
		var sensor_params_dict: Dictionary = sensor_params
		rays_now = int(sensor_params_dict.get("rays", 0))
	_check("sensor-configure", rays_now == 360, "rays=%d params=%s" % [rays_now, sensor_params])

	# A24 slot 表：键名 = 节点名，GetSensor 返回同一节点
	var slots: Dictionary = vehicle.GetStreamSlots()
	_check("stream-slots", slots.size() == 1 and slots.has("lidar_front")
		and vehicle.GetSensor("lidar_front") == sensor_holder,
		"slots=%s node=%s" % [slots.keys(), vehicle.GetSensor("lidar_front")])

	# A25 按需采样（canonical + snake_case 别名）
	var samples: Dictionary = vehicle.SampleSensors()
	var samples_alias: Dictionary = vehicle.sample_sensors()
	var want_sample: Dictionary = sensor_holder.Sample() if sensor_holder != null else {}
	_check("sample-sensors", samples.size() == 1 and samples.get("lidar_front") == want_sample
		and samples_alias == samples,
		"samples=%s want=%s" % [samples, want_sample])

	# A26 同 slot 重挂：子节点数不变、新节点 ≠ 旧节点、旧节点释放
	var prev_holder: Node3D = sensor_holder
	var reattached: Node = vehicle.AttachSensor("lidar_front", sensor_scene, {}, {"rays": 64})
	await _wait_physics(1)
	var re_holder: Node3D = reattached as Node3D
	_check("reattach-same-slot", re_holder != null and re_holder != prev_holder
		and sensors_root.get_child_count() == 1
		and not is_instance_valid(prev_holder),
		"children=%d new=%s old_valid=%s" % [sensors_root.get_child_count(), re_holder,
			is_instance_valid(prev_holder)])
	if re_holder == null:
		_fatal("sensor-reattach", "重挂后没有传感器节点")
		_finish()
		return
	sensor_holder = re_holder

	# A27 多 slot
	var imu_node: Node = vehicle.AttachSensor("imu", sensor_scene,
		{"x": 0.0, "y": 0.0, "z": 0.1}, {"rays": 1})
	var imu_holder: Node3D = imu_node as Node3D
	_check("multi-slot", vehicle.GetStreamSlots().size() == 2
		and sensors_root.get_child_count() == 2 and vehicle.GetSensor("imu") == imu_holder,
		"slots=%d children=%d imu=%s" % [vehicle.GetStreamSlots().size(),
			sensors_root.get_child_count(), imu_holder])

	# A28 卸指定 slot
	vehicle.DetachSensor("imu")
	await _wait_physics(1)
	_check("detach-sensor", vehicle.GetStreamSlots().size() == 1
		and sensors_root.get_child_count() == 1
		and sensors_root.get_node_or_null("imu") == null,
		"slots=%d children=%d" % [vehicle.GetStreamSlots().size(),
			sensors_root.get_child_count()])

	# A29 卸不存在的 slot：空操作
	vehicle.DetachSensor("nope")
	vehicle.DetachSensor("")
	_check("detach-missing", vehicle.GetStreamSlots().size() == 1
		and sensors_root.get_child_count() == 1,
		"slots=%d children=%d" % [vehicle.GetStreamSlots().size(),
			sensors_root.get_child_count()])

	# A30 非法 slot
	var blank_a: Node = vehicle.AttachSensor("   ", sensor_scene)
	var blank_b: Node = vehicle.AttachSensor("", sensor_scene)
	_check("bad-slot", blank_a == null and blank_b == null
		and sensors_root.get_child_count() == 1,
		"a=%s b=%s children=%d" % [blank_a, blank_b, sensors_root.get_child_count()])

	# A31 传感器根节点不是 Node3D
	var bogus_sensor_scene: PackedScene = _pack_bogus()
	var bad_sensor: Node = vehicle.AttachSensor("bogus_slot", bogus_sensor_scene)
	_check("bad-sensor-type", bad_sensor == null and sensors_root.get_child_count() == 1,
		"node=%s children=%d" % [bad_sensor, sensors_root.get_child_count()])

	# A32 传感器先于底盘：两者可共存，采样仍可用
	var sensor_only_ok: bool = (not vehicle.HasChassis()
		and vehicle.GetSensor("lidar_front") != null
		and vehicle.SampleSensors().has("lidar_front"))
	var applied_again: bool = vehicle.ApplyChassis("test_chassis_3", {}, chassis_scene)
	var both_ok: bool = (applied_again and vehicle.HasChassis()
		and vehicle.GetSensor("lidar_front") != null
		and chassis_root.get_child_count() == 1
		and vehicle.SampleSensors().has("lidar_front"))
	_check("sensor-before-chassis", sensor_only_ok and both_ok,
		"sensor_only=%s applied=%s both=%s" % [sensor_only_ok, applied_again, both_ok])

	# ============================================================ 4. 边界 / 跨组件契约
	# A33 白盒 lint：不做射线 / 世界几何 / 网络 / 全局类名；必须有 move_and_slide
	var vehicle_script: Script = load(VEHICLE_SCRIPT) as Script
	var source: String = vehicle_script.source_code if vehicle_script != null else ""
	var code: String = _strip_comment_lines(source)
	var hits: Array[String] = []
	for token: String in BANNED_TOKENS:
		if code.find(token) >= 0:
			hits.append(token)
	_check("white-box-boundaries", vehicle_script != null and hits.is_empty()
		and code.find("move_and_slide") >= 0 and code.find("_physics_process") >= 0,
		"banned=%s len=%d" % [hits, source.length()])

	# A34 跨组件契约：相机按 group "vehicle" 选中这台空车
	var camera: Camera3D = world.get_node_or_null("ObserverCamera") as Camera3D
	var picked: Node3D = null
	var camera_hit := false
	if camera != null:
		picked = camera.SelectNextTarget()
		camera_hit = picked == vehicle and camera.GetTarget() == vehicle
	_check("camera-integration", camera_hit, "picked=%s cam_target=%s" % [picked,
		camera.GetTarget() if camera != null else null])

	# A35 车身尺寸唯一入口：UAV 尺寸也守"原点 = 底面中心"
	vehicle.SetBodySize(Vector3(0.4, 0.12, 0.4))
	var body_live: MeshInstance3D = vehicle.body
	var shape_live: CollisionShape3D = vehicle.collision_shape
	var size_ok := false
	var size_detail := "body/shape 缺失"
	if body_live != null and shape_live != null \
		and body_live.mesh is BoxMesh and shape_live.shape is BoxShape3D:
		var mesh_live: BoxMesh = body_live.mesh as BoxMesh
		var shape_live_box: BoxShape3D = shape_live.shape as BoxShape3D
		size_ok = absf(body_live.position.y - 0.06) < 1e-4 \
			and absf(shape_live.position.y - 0.06) < 1e-4 \
			and mesh_live.size.is_equal_approx(Vector3(0.4, 0.12, 0.4)) \
			and shape_live_box.size.is_equal_approx(Vector3(0.4, 0.12, 0.4))
		size_detail = "body.y=%.4f shape.y=%.4f mesh=%s shape=%s" % [body_live.position.y,
			shape_live.position.y, mesh_live.size, shape_live_box.size]
	_check("set-body-size", size_ok, size_detail)

	# A36 全程无自检装置层面的错误
	_check("no-harness-errors", _harness_errors == 0, "harness_errors=%d" % _harness_errors)

	# ============================================================ 5. 清理（P12：退出时无 ERROR）
	vehicle.DetachSensor("lidar_front")
	vehicle.queue_free()
	await process_frame
	await process_frame
	world.queue_free()
	await process_frame
	await process_frame
	_finish()


# ================================================================ 测试装置

## 现场打包替身场景（P4）：先命名再 pack，实例化后节点名保留，且不落 .tscn。
func _pack_double(script_path: String, node_name: String) -> PackedScene:
	var probe := Node3D.new()
	probe.name = node_name
	var script: Script = load(script_path) as Script
	if script != null:
		probe.set_script(script)
	var packed := PackedScene.new()
	if packed.pack(probe) != OK:
		_check("pack-double:" + node_name, false, script_path)
	probe.free()
	return packed


## 根节点不是 Node3D 的"劣质场景"（A15 / A31 用）。
func _pack_bogus() -> PackedScene:
	var bogus := Control.new()
	bogus.name = "Bogus"
	var packed := PackedScene.new()
	packed.pack(bogus)
	bogus.free()
	return packed


## 去掉以 # 开头的注释行，只留可执行代码（A33 白盒 lint 用）。
func _strip_comment_lines(source: String) -> String:
	var code := ""
	for raw_line: String in source.split("\n"):
		if raw_line.strip_edges().begins_with("#"):
			continue
		code += raw_line + "\n"
	return code


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		await physics_frame


func _on_chassis_installed(chassis_name: String) -> void:
	_chassis_signal_count += 1
	_chassis_signal_arg = chassis_name


func _on_sensor_attached(slot: String, _sensor: Node) -> void:
	_sensor_signal_count += 1
	_sensor_signal_arg = slot


# ================================================================ 断言与收尾

func _check(label: String, ok: bool, detail: String = "") -> bool:
	_total += 1
	if ok:
		print("[check] ", label, " ok", (" " + detail) if not detail.is_empty() else "")
		return true
	_failed += 1
	print("[check] ", label, " FAIL", (" " + detail) if not detail.is_empty() else "")
	return false


## 结构性前置条件失败：计数 + 中止（成功路径不占断言编号，保证 total 稳定）。
func _fatal(label: String, detail: String) -> void:
	_total += 1
	_failed += 1
	_harness_errors += 1
	print("[check] ", label, " FAIL ", detail)


func _finish() -> void:
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
