# Presented by KeJi
# Created Date ： 2026-11-07
# Modified Date ： 2026-11-07
#
# 轿车底盘 sedan（task_50 Step 3 / 口径 #14：只平移）。
#   挂在 Vehicle/Chassis 下，**只吃 motion_cmd.v**（m/s，clamp 0~max_speed）；
#   dir / omega / action / speed 一律不看：朝向只在 spawn 时由 params.heading_deg 定死，
#   运行期 Step() 的 yaw_rate 恒 0 ⇒ 车身不转 ⇒ 纯平移（不转向、不倒车、不掉头、不差速）。
#
# 四条与其他底盘的差别（都是口径要的，不是省事）：
#   ① 无速度环斜坡：v=0 **同一帧**归零，v=1.0 第一帧就到 1.0（口径 #14 要"直线开到停下"的可预测性，
#      不是 DiffDrive 那套 MAX_ACCEL_MPS2 斜坡）；
#   ② 不倒车：v 夹在 [0, max_speed]，负值当 0（step.md：出现倒车即 FAIL）；
#   ③ 不转向：yaw_rate 恒 0.0（step.md：并发一帧 dir=1.57 若车身转了即 FAIL）；
#   ④ 位置推进**不在本文件**：D8 规定唯一位置更新点是 Vehicle._physics_process()
#      （拿 Step() 的控制量 -> rotation.y += yaw_rate*dt -> move_and_slide()）。
#      本文件若再推进一次，同一物理帧位移会翻倍（= 2 倍速），所以这里**没有** _physics_process：
#      step.md 说的 move_and_slide() 由宿主 Vehicle 执行，本文件只负责"给对控制量"。
#      推论：**sedan 必须经 Vehicle 装配**（kind="robot" + chassis="sedan" -> Vehicles）；
#      单独当根节点挂进树不会动。
#
# 尺寸（口径 #1 目标尺寸表 = 真实乘用车）：车身 4.50 长 × 1.50 高 × 1.80 宽（米），
#   碰撞盒 = 外观（D11：模型 = 碰撞盒）。资产是 Kenney 卡通低模（sedan 2.55×1.30×1.50 /
#   suv 2.70×1.30×1.50，回读见 assets/props/cars/README.md §2），逐轴缩到目标值写在 Body 上
#   （assets/ 不在本 step 的允许改动路径，不能重写 glb 的 scale）；代价是轮子变椭圆，
#   高位俯视机位下可接受（口径 #17 机位不动）。
#
# 不声明全局类名（COMPONENTS.md §9.1 E5）。

extends "res://src/chassis/ChassisModel.gd"

const WorldTransform := preload("res://src/common/WorldTransform.gd")

## 目录名（ChassisCatalog.SCENES / DEFAULTS 的键）。
const CHASSIS_NAME := "sedan"
## 缺省最大线速度（m/s）：口径 #12。
const DEFAULT_MAX_SPEED := 2.2
## 缺省车身尺寸（米，长 × 高 × 宽）：口径 #1 目标尺寸表。
const DEFAULT_BODY_LENGTH := 4.50
const DEFAULT_TRACK_WIDTH := 1.80
const DEFAULT_HEIGHT := 1.50
## 缺省外观（assets/props/cars/ 的资产 ID）。
const DEFAULT_MODEL := "kenney_sedan"
## 缺省朝向（世界系角度，0 = +X 东、逆时针为正）：-90° = 朝北。
##   venue 的路南北向（x∈[56,64]）；北排车朝北、南排朝南，另一台在 hello 里写 heading_deg=90。
const DEFAULT_HEADING_DEG := -90.0

## model 名 -> 外观场景。键 = assets/props/cars/ 的资产 ID；别名让剧本可以只写 sedan / suv。
const MODEL_SCENES := {
	"kenney_sedan": preload("res://assets/props/cars/kenney_sedan/model.glb"),
	"kenney_suv": preload("res://assets/props/cars/kenney_suv/model.glb"),
}
const MODEL_ALIASES := {
	"sedan": "kenney_sedan",
	"suv": "kenney_suv",
}
## 每款外观的**原始 AABB**（X 长 × Y 高 × Z 宽，米）= 缩放的分母（与 cars/README.md §2 同源，
##   本计划已回读 glb 复核过）。
const MODEL_SIZE := {
	"kenney_sedan": Vector3(2.55, 1.30, 1.50),
	"kenney_suv": Vector3(2.70, 1.30, 1.50),
}

# ---------------------------------------------------------------- 生效参数
var _max_speed := DEFAULT_MAX_SPEED
var _body_length := DEFAULT_BODY_LENGTH
var _track_width := DEFAULT_TRACK_WIDTH
var _height := DEFAULT_HEIGHT

# ---------------------------------------------------------------- 运动状态
var _v_target := 0.0        # 目标线速度（m/s，SetCommand 写，clamp 0.._max_speed）
var _v_now := 0.0           # Step 算出的生效线速度（GetSpeed 用）
var _model_key := ""        # 已装外观的规范名（"" = 还没装）

var _warned: Dictionary = {}


# ================================================================ 契约

## 默认参数：与 ChassisCatalog.DEFAULTS["sedan"] **逐键一致**（5 键；model / heading_deg 不进参数表，
##   见计划 §2 决策 6）。入树前也会被 Vehicle._merge_params() 调用 → 只返回常量，不碰场景树。
func GetDefaultParams() -> Dictionary:
	return {
		"max_speed": DEFAULT_MAX_SPEED,
		"max_turn_rate": 0.0,
		"body_length": DEFAULT_BODY_LENGTH,
		"track_width": DEFAULT_TRACK_WIDTH,
		"height": DEFAULT_HEIGHT,
	}


## 装配：参数夹到合法域 -> 状态清零 -> GROUNDED -> 车身尺寸 -> 外观 -> 朝向。
## 顺序是硬约束：ApplyBodySize 先定 Vehicle 的形状；_Install_Model 用"目标尺寸/原始 AABB"算缩放；
##   _Apply_Heading 只改宿主朝向（不碰位置）。
func Configure(p_vehicle: Node3D, p_params: Dictionary) -> void:
	super.Configure(p_vehicle, p_params)
	_max_speed = maxf(GetParam("max_speed", DEFAULT_MAX_SPEED), 0.0)
	_body_length = maxf(GetParam("body_length", DEFAULT_BODY_LENGTH), MIN_SIZE)
	_track_width = maxf(GetParam("track_width", DEFAULT_TRACK_WIDTH), MIN_SIZE)
	_height = maxf(GetParam("height", DEFAULT_HEIGHT), MIN_SIZE)
	# 换底盘/重装配不继承旧状态
	_v_target = 0.0
	_v_now = 0.0
	ApplyMotionMode(CharacterBody3D.MOTION_MODE_GROUNDED)
	# 车身/碰撞形状唯一入口：只传尺寸，抬升由 Vehicle 负责（R5：绝不双重抬升）
	ApplyBodySize(Vector3(_body_length, _height, _track_width))
	_Install_Model(str(params.get("model", DEFAULT_MODEL)))
	_Apply_Heading(GetParam("heading_deg", DEFAULT_HEADING_DEG))


## 只认 v（m/s）：clampf 到 [0, max_speed]。dir / omega / action / speed **一律忽略**。
## 不给 v = 停车（老协议 action="forward" + speed=100 在轿车上**不会开动** —— 口径 #14 要的）。
func SetCommand(cmd: Dictionary) -> void:
	super.SetCommand(cmd)                       # 基类只存 last_cmd，不解释语义
	var raw_v: float = _to_float(cmd["v"], 0.0) if cmd.has("v") else 0.0
	_v_target = clampf(raw_v, 0.0, _max_speed) if is_finite(raw_v) else 0.0


## 纯控制量：沿**当前朝向**（spawn 定死、运行期不变）前进；yaw_rate 恒 0（不转向）。
## 无斜坡：_v_now 就是 _v_target 的 clamp，同一帧到位（v=0 当帧停）。
## 不写 position、不调物理推进 API（D8）；树外走 basis 而非 global_basis（否则 ERROR）。
func Step(_current_velocity: Vector3, _delta: float) -> Dictionary:
	_v_now = clampf(_v_target, 0.0, _max_speed)
	var basis_now: Basis = global_basis if is_inside_tree() else basis
	var vel: Vector3 = basis_now * Vector3(_v_now, 0.0, 0.0)   # 沿当前朝向
	vel.y = 0.0                                                # 平面运动：不跟重力抢 y
	return {"velocity": vel, "yaw_rate": 0.0}                  # 恒 0 ⇒ 只平移


# ================================================================ 查询（探针 / 日志用）

## 当前生效线速度（m/s，0..max_speed）。
func GetSpeed() -> float:
	return _v_now


## 生效车身尺寸（米，Vector3(body_length, height, track_width)）。
func GetBodySize() -> Vector3:
	return Vector3(_body_length, _height, _track_width)


## 已装外观的规范名（"kenney_sedan" / "kenney_suv"）；未装 = ""。
func GetModelName() -> String:
	return _model_key


# ================================================================ 私有

## 装外观（幂等：同一个规范名再来一次不重建）。
## 未知 model -> push_warning（每类一次）+ 回落缺省，**不拒 spawn**。
## 缩放写在 Body 上：Body 的 transform 由本函数保证恒等，缩放因此沿车体 X/Y/Z 三轴
##   （写在外观根节点上会跟着根节点的 R_y(+90°) 转，长/宽互换）。
func _Install_Model(raw_name: String) -> void:
	var key := _Model_Key(raw_name)
	if key == _model_key:
		return
	var body: Node3D = get_node_or_null("Body") as Node3D
	if body == null:
		body = Node3D.new()                     # 场景缺 Body 时兜底（正常由 Sedan.tscn 给）
		body.name = "Body"
		add_child(body)
	for child: Node in body.get_children():
		body.remove_child(child)                # 先摘再 free：名字/位置立刻可复用
		child.queue_free()
	var model: Node = (MODEL_SCENES[key] as PackedScene).instantiate()
	var holder: Node3D = model as Node3D
	if holder == null:
		_Warn_Once("model-scene", "[sedan] 外观场景 '%s' 实例化失败（不是 Node3D）" % key)
		return
	body.add_child(holder)
	var size: Vector3 = MODEL_SIZE[key]
	body.position = Vector3.ZERO                # 轮底 y=0 / 水平居中：资产已归一（口径 #7）
	body.rotation = Vector3.ZERO                # 恒等：缩放的轴 = 车体轴（见函数头注）
	body.scale = Vector3(_body_length / size.x, _height / size.y, _track_width / size.z)
	_model_key = key
	print("[sedan] model=%s scale=(%.4f, %.4f, %.4f) target=%.2f×%.2f×%.2f"
		% [key, body.scale.x, body.scale.y, body.scale.z,
			_body_length, _height, _track_width])


## model 名 -> 规范名：去空白/小写 -> 别名映射 -> 未登记则 warn 一次并回落缺省。
func _Model_Key(raw_name: String) -> String:
	var key := raw_name.strip_edges().to_lower()
	if MODEL_ALIASES.has(key):
		key = str(MODEL_ALIASES[key])
	if not MODEL_SCENES.has(key):
		_Warn_Once("model", "[sedan] 未知 model '%s'（合法：%s），回落 %s"
			% [raw_name, str(MODEL_SCENES.keys()), DEFAULT_MODEL])
		key = DEFAULT_MODEL
	return key


## 朝向：世界 yaw（度）-> 宿主 Vehicle 的 rotation.y（换轴只经 WorldTransform）。
## 只改**朝向**、不碰位置（D8）。为什么改宿主而不是改自己：GetPose() 取的是 Vehicle 的
##   global_transform（E23：位姿只能走 GetPose）；朝向转在 Sedan 自己身上会出现
##   "车朝北走、pose 报 yaw=0"，下游（地面站/剧本）全错。
func _Apply_Heading(deg: float) -> void:
	if vehicle == null or not is_instance_valid(vehicle):
		return
	if not is_finite(deg):
		_Warn_Once("heading", "[sedan] heading_deg 非法，回落 %.1f°" % DEFAULT_HEADING_DEG)
		deg = DEFAULT_HEADING_DEG
	vehicle.rotation.y = WorldTransform.YawToGodot(deg_to_rad(deg))


## 每类告警只出一次（每帧刷屏没有意义；与 Pedestrian._Warn_Once 同款）。
func _Warn_Once(key: String, message: String) -> void:
	if _warned.has(key):
		return
	_warned[key] = true
	push_warning(message)
