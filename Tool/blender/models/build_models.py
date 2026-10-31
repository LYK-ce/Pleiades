#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-31
"""task_33 Step 1 / task_42 Step 2 — 传感器 / 底盘低模构建脚本（Blender 4.5 无头，幂等）。

用法（工程根）：
    /vepfs-mlp2/c20250205/240804016/tool/blender/blender -b --factory-startup \\
        --python-exit-code 1 --python Tool/blender/models/build_models.py [lidar|camera|ugv|ugv_m1 ...]

产物（每个模型一个 .blend 源文件；glb 导出交给 export_all.sh → tool/blender/blend2glb.sh）：
    Tool/blender/models/lidar_tmini.blend
    Tool/blender/models/camera_module.blend
    Tool/blender/models/ugv_chassis.blend
    Tool/blender/models/ugv_m1.blend          （路径可用 ORION_BLEND_OUT_DIR 覆盖）

ORION_BLEND_OUT_DIR（task_42 Step 2）：设了就把 .blend 存到该目录（本轮落 tmp/task42/step2/，
避免在 Tool/blender/models/ 下新增未跟踪的 ugv_m1.blend）；不设 = 旧行为逐字不变。

坐标口径（与 Tool/models/make_ugv.py 一致）：
    1 BU = 1 m；Blender Z-up（+z = 高），+x = 车头 / 镜头朝向。
    网格坐标 = 世界坐标，所有对象 transform = IDENTITY（导出后 glTF 节点无 TRS）。
    export_yup=True 后 Godot 侧为 (x 长, y 高, z 宽) = Blender (x, z, -y)。

原点约定（task_33 D8）：
    lidar_tmini   → 罐体底面中心（= 扫描面中心），z_min == 0
    camera_module → 镜片前表面中心（= 光心），max.x == 0
    ugv_chassis   → 车体底面中心（贴地），z_min == 0
    ugv_m1        → 车体底面中心（贴地），z_min == 0；+x = 车头，包络 0.30 × 0.25 × 0.25

失败语义：任何 AABB / 三角数断言不符 → RuntimeError → 进程 EXIT≠0
（配合 --python-exit-code 1 与 export_all.sh 的 set -e，不会产出半成品 glb）。
"""

import math
import os
import sys
from pathlib import Path

import bmesh
import bpy

# ──────────────────────────────────────────────────────────── 路径常量
PROJECT_ROOT = Path(__file__).resolve().parents[3]  # models → blender → Tool → Orion
BLEND_DIR = PROJECT_ROOT / "Tool/blender/models"
# task_42 Step 2：本轮 .blend 落 tmp/（白名单外的新 .blend 不入库）；缺省仍是 BLEND_DIR
_blend_env = os.environ.get("ORION_BLEND_OUT_DIR", "").strip()
BLEND_OUT_DIR = Path(_blend_env) if _blend_env else BLEND_DIR
GLB_SENSORS = PROJECT_ROOT / "orion-simulation-world/assets/sensors"
GLB_CHASSIS = PROJECT_ROOT / "orion-simulation-world/assets/chassis"

# ──────────────────────────────────────────────────────────── 分段数
SEG_BASE = 32
SEG_CAN = 32
SEG_WINDOW = 32
SEG_BARREL = 24
SEG_LENS = 24
SEG_WHEEL = 24

# ──────────────────────────────────────────────────────────── 雷达 lidar_tmini
LIDAR_BASE_D = 0.080          # 底座直径
LIDAR_BASE_H = 0.005          # 底座高
LIDAR_CAN_D = 0.070           # 罐体直径
LIDAR_CAN_H = 0.045           # 罐体高
WINDOW_D = 0.0716             # 扫描窗直径（比罐体大 0.8 mm，避开共面 z-fighting）
WINDOW_Z0 = 0.020             # 扫描窗下沿
WINDOW_Z1 = 0.035             # 扫描窗上沿
BUMP_SIZE = (0.006, 0.006, 0.004)   # 顶盖「正前方」凸起
BUMP_X = 0.030                # 凸起中心 x（+x = 正前方）

# ──────────────────────────────────────────────────────────── 相机 camera_module
CAM_BODY = (0.040, 0.030, 0.030)     # 机身 x × y × z
CAM_BARREL_D = 0.020                 # 镜头筒直径
CAM_BARREL_L = 0.015                 # 镜组沿 +x 的总伸出长（含镜片厚）
CAM_LENS_D = 0.016                   # 镜片直径
CAM_LENS_T = 0.0005                  # 镜片厚
CAM_BARREL_SPAN = CAM_BARREL_L - CAM_LENS_T  # 镜头筒净长（前端收到 −0.0005，藏到镜片后）
CAM_BRACKET = (0.040, 0.030, 0.006)  # 支架薄板（机身正下方）
CAM_BODY_X0 = -0.055                 # 机身尾端 x
CAM_BODY_X1 = -0.015                 # 机身前端 x（镜头筒起点）

# ──────────────────────────────────────────────────────────── UGV 底盘 ugv_chassis
UGV_HULL = (0.28, 0.21, 0.12)        # 下车体 x × y × z
UGV_NOSE = (0.02, 0.21, 0.12)        # 车鼻前脸立板
UGV_ROOF = (0.14, 0.14, 0.01)        # 车顶平台（雷达安装面）
UGV_CABIN_PROFILE = [(-0.11, 0.12), (0.15, 0.12), (0.11, 0.24), (-0.11, 0.24)]
UGV_CABIN_HALF_Y = 0.100
UGV_HULL_X0 = -0.150                 # 下车体尾端
UGV_HULL_X1 = 0.130                  # 下车体前端（车鼻立板起点）
UGV_WHEEL_R = 0.030                  # 轮半径（⌀0.06）
UGV_WHEEL_W = 0.020                  # 轮宽
UGV_WHEEL_X = 0.120                  # 轮心 x（±）
UGV_WHEEL_Y = 0.115                  # 轮心 y（±）→ 轮外缘 ±0.125 = 车宽 0.25 的一半
UGV_WHEEL_Z = 0.030                  # 轮心 z = 轮半径（轮底贴地）
UGV_WHEEL_TAGS = (("FL", 1.0, -1.0), ("FR", 1.0, 1.0), ("RL", -1.0, -1.0), ("RR", -1.0, 1.0))

# ──────────────────────────────────────────────────────────── UGV M1 风格底盘 ugv_m1（task_42 Step 2）
# 包络 0.30 × 0.25 × 0.25（x 长 / y 宽 / z 高），原点 = 底面中心，+x = 车头。
# 麦克纳姆轮只做视觉（不做横移运动学）。
M1_MIN = (-0.150, -0.125, 0.000)
M1_MAX = (0.150, 0.125, 0.250)
M1_TRI_LIMIT = 12000                 # 本模型专用上限（辊子会涨面；传感器仍是 TRI_LIMIT）
M1_WHEEL_R = 0.045                   # 包络半径 → 轮外缘 |x| = 0.105 + 0.045 = 0.150
M1_WHEEL_W = 0.025                   # 轮宽
M1_WHEEL_X = 0.105                   # 轮心 x（±）：取 AABB 允许的最大值，保住 ⌀90 mm 轮径
M1_WHEEL_Y = 0.1125                  # 轮心 y（±）→ 端盖外端面 ±0.125 = 车宽极值
M1_WHEEL_Z = 0.045                   # 轮心 z；辊子最低点 z = 0.045 − (0.039 + 0.006) = 0
M1_HUB_R, M1_HUB_SEG = 0.030, 16     # 轮毂
M1_CAP_R, M1_CAP_SEG, M1_CAP_T = 0.013, 8, 0.0045   # 端盖占最外 4.5 mm，与轮毂端面错开
M1_ROLLER_N = 8                      # 每排 8 个，φ = −90° + 45°·k（含 x 极值与触地点）
M1_ROLLER_R, M1_ROLLER_L, M1_ROLLER_SEG = 0.006, 0.010, 6
M1_ROLLER_RC = 0.039                 # 辊子中心半径 = 0.045 − 0.006
M1_ROW_DY = 0.0045                   # 两排沿 y 偏移
M1_TILT_DEG = 45.0                   # 辊子轴向与轮轴成 45°
M1_WHEEL_TAGS = (("FL", 1.0, -1.0), ("FR", 1.0, 1.0), ("RL", -1.0, -1.0), ("RR", -1.0, 1.0))

# 车身 v3（人类第 3 轮反馈：深色车顶 / 车身压低 / 摄像头坐车身顶面；v2 的侧视轮廓不动）
# 共面规避规则：所有「叠放」构件的贴合面一律再嵌入 0.5 mm（写在注释里，别删），否则 Cycles/EEVEE 出 z-fighting。
# v3 取法：**可见面（顶/前/侧）逐字等于人类表格**，只有被上层件压住的不可见下表面 = 定值 − 0.5 mm。
M1_HULL = (0.280, 0.195, 0.095)            # 下车体长条方盒 x × y × z（v3 压低：顶面 0.155 → 0.125）
M1_HULL_C = (0.000, 0.0, 0.0775)           # → x[-0.140, 0.140] y[±0.0975] z[0.030, 0.125]
M1_DECK_PROFILE = [(0.0900, 0.1245), (0.0200, 0.2000), (-0.1400, 0.2000), (-0.1400, 0.1245)]
M1_DECK_HALF_Y = 0.0930                    # 引擎盖斜坡 + 上部舱体 = **一整块**四边形棱柱（无内部共面）
M1_ROOF = (0.160, 0.189, 0.0105)           # 车顶平台 = **深色**（v3；比舱体宽 1.5 mm 形成檐口）
M1_ROOF_C = (-0.060, 0.0, 0.20475)         # → x[-0.140, 0.020] y[±0.0945] z[0.1995, 0.2100]
M1_LIDAR = (0.045, 0.045, 0.0405)          # 激光雷达黑方盒
M1_LIDAR_C = (-0.0375, 0.0, 0.22975)       # → x[-0.060,-0.015] y[±0.0225] z[0.2095, 0.2500]（顶面 = AABB +z）
M1_LIDAR_BAND = (0.043, 0.047, 0.010)      # 蓝色扫描窗带（只向 ±y 凸 1 mm，x/z 端点埋进雷达）
M1_LIDAR_BAND_C = (-0.0375, 0.0, 0.2290)
# 前摄像头 v3：黑壳 + 蓝镜头**合并成一个节点**（Add_Front_Camera_M1）→ FrontCamera 的 AABB
# 恰好 = 人类表格 x[0.115, 0.150]（壳 0.115..0.1475，镜头端面正好 0.1500 = AABB +x）。
M1_CAM = (0.0325, 0.056, 0.0355)           # 壳：x[0.115, 0.1475] y[±0.028] z[0.1245, 0.1600]
M1_CAM_C = (0.13125, 0.0, 0.14225)         # → 坐在车身顶面（0.1245 嵌 Hull 0.5 mm），明显低于车顶 0.210
M1_CAM_LENS_R = 0.012                      # 镜头半径（φ0.024）
M1_CAM_LENS_SEG = 8                        # 镜头分段
M1_CAM_LENS_CX = 0.1470                    # 镜头中心 x → 轴向 +x，x[0.1440, 0.1500]（后端埋进壳体 0.5 mm）
M1_CAM_LENS_L = 0.0060                     # 镜头长
M1_NOSE_PLATE = (0.018, 0.199, 0.065)      # 浅灰横向前保险杠（节点名 NosePlate）
M1_NOSE_PLATE_C = (0.139, 0.0, 0.0825)     # → x[0.130, 0.148] y[±0.0995] z[0.050, 0.115]
M1_NOSE_LED = (0.0025, 0.120, 0.008)       # 蓝 LED 条
M1_NOSE_LED_C = (0.14875, 0.0, 0.0755)     # 前端面 = 0.1500 → AABB +x 极值（与前轮辊子同位）
M1_SIDE_ARMOR = (0.250, 0.0025, 0.075)     # 侧甲板薄板（外凸 2 mm，内面埋进车体 0.5 mm）
M1_SIDE_ARMOR_CY = 0.09825                 # y 0.0970..0.0995（外表面 0.0995 < 轮内端面 0.100）
M1_SIDE_ARMOR_CZ = 0.0825                  # z 0.045..0.120（v3 跟着车身压低）
M1_REAR_PANEL = (0.0065, 0.180, 0.075)     # 尾板（前沿埋进车体 0.5 mm）
M1_REAR_PANEL_C = (-0.14275, 0.0, 0.0825)  # → x[-0.146, -0.1395] y[±0.090] z[0.045, 0.120]（顶面不露头）

# ──────────────────────────────────────────────────────────── 材质（恰好 3 种）
METAL_GRAY_NAME = "ORION_MetalGray"
MATTE_BLACK_NAME = "ORION_MatteBlack"
GLASS_BLUE_NAME = "ORION_GlassBlue"
METAL_GRAY_COLOR = (0.55, 0.57, 0.60)
MATTE_BLACK_COLOR = (0.06, 0.06, 0.07)
GLASS_BLUE_COLOR = (0.20, 0.55, 0.95)
METAL_METALLIC, METAL_ROUGH = 0.80, 0.35
BLACK_METALLIC, BLACK_ROUGH = 0.00, 0.70
GLASS_METALLIC, GLASS_ROUGH = 0.00, 0.15
GLASS_ALPHA = 0.45                   # 半透明蓝（必须同时设 Alpha 插槽 + BLENDED，见 plan R5）

# ──────────────────────────────────────────────────────────── UGV M1 风格底盘材质（4 种 ≤ 5，无贴图）
M1_DARK_NAME = "ORION_UGV_DarkGray"
M1_LIGHT_NAME = "ORION_UGV_LightGray"
M1_BLACK_NAME = "ORION_UGV_Black"
M1_BLUE_NAME = "ORION_UGV_LedBlue"
M1_DARK_COLOR = (0.070, 0.076, 0.084)
M1_LIGHT_COLOR = (0.620, 0.640, 0.660)
M1_BLACK_COLOR = (0.022, 0.022, 0.028)
M1_BLUE_COLOR = (0.120, 0.420, 0.950)
M1_DARK_METALLIC, M1_DARK_ROUGH = 0.55, 0.42
M1_LIGHT_METALLIC, M1_LIGHT_ROUGH = 0.30, 0.38
M1_BLACK_METALLIC, M1_BLACK_ROUGH = 0.10, 0.72
M1_BLUE_METALLIC, M1_BLUE_ROUGH = 0.00, 0.22

# ──────────────────────────────────────────────────────────── 预算 / 断言
TRI_LIMIT = 2000
TRI_LIMITS = {"ugv_m1": M1_TRI_LIMIT}      # 按模型区分：传感器 2000，ugv_m1 12000
GLB_LIMIT_B = 100 * 1024
AABB_TOL = 1e-4
EXPECT = {
    # 期望 AABB（Blender 世界系，米）
    "lidar_tmini": ((-0.040, -0.040, 0.000), (0.040, 0.040, 0.054)),
    "camera_module": ((-0.055, -0.015, -0.021), (0.000, 0.015, 0.015)),
    "ugv_chassis": ((-0.150, -0.125, 0.000), (0.150, 0.125, 0.250)),
    "ugv_m1": (M1_MIN, M1_MAX),
}


# ════════════════════════════════════════════════════════════ 底层网格工具
def _New_Object(name, verts, faces, material, recalc=True, smooth=None):
    """用顶点 / 面表建一个 MeshInstance3D 的 Blender 对应物（对象 transform 恒 identity）。

    smooth=None → 全部平面着色（旧三件的行为逐字不变）；
    smooth=[bool, ...] → 逐面写 use_smooth（长度必须与 validate() 后的面数一致）。
    """
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    mesh.validate(verbose=False)
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    if recalc:
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(mesh)
        bm.free()
    mesh.materials.append(material)
    if smooth is not None:
        # validate() 可能删掉退化面 → 索引会错位；错位会静默毁掉法线与体积口径，必须硬失败
        if len(mesh.polygons) != len(smooth):
            raise RuntimeError("%s 面数 %d 与 smooth 标记数 %d 不一致"
                               % (name, len(mesh.polygons), len(smooth)))
        for i, poly in enumerate(mesh.polygons):
            poly.use_smooth = bool(smooth[i])
    mesh.update()
    return obj


def _Cylinder_Data(radius, height, segments, caps):
    """沿本地 +z 的圆柱，z ∈ [−h/2, +h/2]。返回 (verts, faces)，法线朝外。"""
    verts = []
    for i in range(segments):
        ang = 2.0 * math.pi * i / segments
        verts.append((radius * math.cos(ang), radius * math.sin(ang), -height * 0.5))
    for i in range(segments):
        ang = 2.0 * math.pi * i / segments
        verts.append((radius * math.cos(ang), radius * math.sin(ang), height * 0.5))
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        faces.append((i, j, segments + j, segments + i))
    if caps:
        faces.append(tuple(range(segments - 1, -1, -1)))       # 底盖（−z）
        faces.append(tuple(range(segments, 2 * segments)))     # 顶盖（+z）
    return verts, faces


def Add_Cylinder(name, radius, height, z_bottom, segments, material, *,
                 open_ends=False, center_x=0.0, center_y=0.0):
    """沿 +z 的圆柱；z 由 z_bottom 起算。open_ends=True → 无底盖顶盖（薄壁筒）。"""
    verts, faces = _Cylinder_Data(radius, height, segments, not open_ends)
    z_mid = z_bottom + height * 0.5
    moved = [(x + center_x, y + center_y, z + z_mid) for (x, y, z) in verts]
    return _New_Object(name, moved, faces, material, recalc=not open_ends)


def Add_Box(name, size, center, material):
    """轴对齐长方体：size = (sx, sy, sz)，center = 中心点。"""
    hx, hy, hz = size[0] * 0.5, size[1] * 0.5, size[2] * 0.5
    cx, cy, cz = center
    verts = [
        (cx - hx, cy - hy, cz - hz), (cx + hx, cy - hy, cz - hz),
        (cx + hx, cy + hy, cz - hz), (cx - hx, cy + hy, cz - hz),
        (cx - hx, cy - hy, cz + hz), (cx + hx, cy - hy, cz + hz),
        (cx + hx, cy + hy, cz + hz), (cx - hx, cy + hy, cz + hz),
    ]
    faces = [
        (0, 3, 2, 1), (4, 5, 6, 7), (1, 2, 6, 5),
        (0, 4, 7, 3), (0, 1, 5, 4), (3, 7, 6, 2),
    ]
    return _New_Object(name, verts, faces, material)


def Add_Prism(name, profile_xz, y_half, material):
    """逆时针 (x, z) 多边形沿 ±y 挤出；用于带前挡斜面的舱体。"""
    n = len(profile_xz)
    verts = [(x, -y_half, z) for (x, z) in profile_xz]
    verts += [(x, y_half, z) for (x, z) in profile_xz]
    faces = [tuple(range(n))[::-1], tuple(range(n, 2 * n))]
    for i in range(n):
        j = (i + 1) % n
        faces.append((i, j, n + j, n + i))
    return _New_Object(name, verts, faces, material)


def Add_Wheel(name, center, radius, width, segments, material):
    """圆柱绕 X 轴转 90°（轴 → y），旋转直接烘焙进网格，对象 transform 保持 identity。"""
    verts, faces = _Cylinder_Data(radius, width, segments, True)
    # 绕 X 轴 +90°：(x, y, z) → (x, −z, y)，本地 z 轴 → 世界 y 轴
    rotated = [(x, -z, y) for (x, y, z) in verts]
    cx, cy, cz = center
    moved = [(x + cx, y + cy, z + cz) for (x, y, z) in rotated]
    return _New_Object(name, moved, faces, material)


def _Append_Cylinder(verts, faces, smooth, center, axis, radius, length, segments, e1_hint):
    """把「轴向 axis、半径 radius、长 length」的圆柱追加进 verts/faces（供合并成单节点网格）。

    AABB 精确性依赖：0 号顶点方向 = e1_hint 在「垂直于轴」平面上的投影方向。
    e1_hint 取「轮心 → 辊子中心」的径向单位向量 ⇒ 辊子在最外方向的顶点半径恰为 radius。
    """
    from mathutils import Vector
    u = Vector(axis).normalized()
    hint = Vector(e1_hint)
    e1 = (hint - u * hint.dot(u)).normalized()
    e2 = u.cross(e1)
    base = len(verts)
    for sign in (-0.5, 0.5):
        c = Vector(center) + u * (length * sign)
        for i in range(segments):
            ang = 2.0 * math.pi * i / segments
            verts.append(tuple(c + e1 * (radius * math.cos(ang)) + e2 * (radius * math.sin(ang))))
    for i in range(segments):
        j = (i + 1) % segments
        faces.append((base + i, base + j, base + segments + j, base + segments + i))
        smooth.append(True)                      # 侧面平滑 → 导出时共享法线，glb 体积减半
    faces.append(tuple(range(base, base + segments)))                 # −u 端盖
    smooth.append(False)
    faces.append(tuple(range(base + segments, base + 2 * segments)))  # +u 端盖
    smooth.append(False)


def Add_Cylinder_Axis(name, center, axis, radius, length, segments, material,
                      e1_hint=(1.0, 0.0, 0.0)):
    """任意轴向圆柱（对象 transform 仍为 IDENTITY，旋转烘进网格）。"""
    verts, faces, smooth = [], [], []
    _Append_Cylinder(verts, faces, smooth, center, axis, radius, length, segments, e1_hint)
    return _New_Object(name, verts, faces, material, smooth=smooth)


def Add_Slab(name, x0, x1, yz_profile, material):
    """(y, z) 多边形沿 x ∈ [x0, x1] 挤出（斜切侧甲板 / 楔形件用）。"""
    n = len(yz_profile)
    verts = [(x0, y, z) for (y, z) in yz_profile]
    verts += [(x1, y, z) for (y, z) in yz_profile]
    faces = [tuple(range(n))[::-1], tuple(range(n, 2 * n))]
    for i in range(n):
        j = (i + 1) % n
        faces.append((i, j, n + j, n + i))
    return _New_Object(name, verts, faces, material)


def Add_Wheel_M1(tag, sign_x, sign_y, material, hub_material):
    """麦克纳姆轮：轮毂 + 端盖 + 两排 8 个 45° 辊子，合并成**一个** MESH 节点 `Wheel<tag>`。"""
    cx, cy, cz = sign_x * M1_WHEEL_X, sign_y * M1_WHEEL_Y, M1_WHEEL_Z
    verts, faces, smooth = [], [], []

    def cyl(center, axis, r, ln, segs, e1):
        _Append_Cylinder(verts, faces, smooth, center, axis, r, ln, segs, e1)

    # 轮毂（轴向 y）：内侧 −W/2，外侧让出 CAP_T 给端盖 → 两平面错开，无共面 z-fighting
    hub_v0, hub_v1 = -M1_WHEEL_W * 0.5, M1_WHEEL_W * 0.5 - M1_CAP_T
    cyl((cx, cy + sign_y * (hub_v0 + hub_v1) * 0.5, cz), (0.0, 1.0, 0.0),
        M1_HUB_R, hub_v1 - hub_v0, M1_HUB_SEG, (1.0, 0.0, 0.0))
    n_hub_faces = len(faces)            # ← 新增：轮毂 = 16 侧面 + 2 端盖；后面整体刷成材质槽 1
    # 端盖外端面 y = cy ± 0.0125 精确 → 定义 AABB 的 ±y 极值
    cyl((cx, cy + sign_y * (M1_WHEEL_W * 0.5 - M1_CAP_T * 0.5), cz), (0.0, 1.0, 0.0),
        M1_CAP_R, M1_CAP_T, M1_CAP_SEG, (1.0, 0.0, 0.0))
    # 辊子：两排 × 8 个，φ = −90° + 45°k（含 0°/180° → x 极值，−90° → z = 0 触地）
    c45, s45 = math.cos(math.radians(M1_TILT_DEG)), math.sin(math.radians(M1_TILT_DEG))
    for row in (1.0, -1.0):
        for k in range(M1_ROLLER_N):
            phi = math.radians(-90.0 + 360.0 * k / M1_ROLLER_N)
            rhat = (math.cos(phi), 0.0, math.sin(phi))
            that = (-math.sin(phi), 0.0, math.cos(phi))
            center = (cx + M1_ROLLER_RC * rhat[0], cy + row * M1_ROW_DY,
                      cz + M1_ROLLER_RC * rhat[2])
            axis = (s45 * row * that[0], c45, s45 * row * that[2])
            cyl(center, axis, M1_ROLLER_R, M1_ROLLER_L, M1_ROLLER_SEG, rhat)
    obj = _New_Object("Wheel" + tag, verts, faces, material, smooth=smooth)
    # 参考图是「银色轮毂 + 黑辊子」：槽 0 = 黑（端盖/辊子），槽 1 = 浅灰（轮毂）
    obj.data.materials.append(hub_material)
    for poly in obj.data.polygons[:n_hub_faces]:
        poly.material_index = 1
    return obj


def Add_Front_Camera_M1(name, shell_size, shell_center, lens_r, lens_seg,
                        lens_cx, lens_len, lens_cz, shell_mat, lens_mat):
    """前摄像头 = 黑壳 + 朝前（+x）镜头，**合并成一个 MESH 节点**（v3）。

    合并理由（plan R2）：节点 AABB 必须逐轴等于人类表格 x[0.115, 0.150]，而镜头又要凸出壳体
    → 只有并成一个节点，壳与镜头才落在同一 AABB 里（分开建节点则 x 上限会各算各的）。
    壳的顶点 / 面顺序与 `Add_Box` 逐字一致（前 6 个面），镜头由 `_Append_Cylinder` 追加
    （它自己往 smooth 里追加 8×True + 2×False）；槽 0 = 黑壳，槽 1 = 蓝镜头。
    """
    hx, hy, hz = shell_size[0] * 0.5, shell_size[1] * 0.5, shell_size[2] * 0.5
    cx, cy, cz = shell_center
    verts = [
        (cx - hx, cy - hy, cz - hz), (cx + hx, cy - hy, cz - hz),
        (cx + hx, cy + hy, cz - hz), (cx - hx, cy + hy, cz - hz),
        (cx - hx, cy - hy, cz + hz), (cx + hx, cy - hy, cz + hz),
        (cx + hx, cy + hy, cz + hz), (cx - hx, cy + hy, cz + hz),
    ]
    faces = [
        (0, 3, 2, 1), (4, 5, 6, 7), (1, 2, 6, 5),
        (0, 4, 7, 3), (0, 1, 5, 4), (3, 7, 6, 2),
    ]
    smooth = [False] * 6
    _Append_Cylinder(verts, faces, smooth, (lens_cx, 0.0, lens_cz), (1.0, 0.0, 0.0),
                     lens_r, lens_len, lens_seg, (0.0, 0.0, 1.0))
    obj = _New_Object(name, verts, faces, shell_mat, smooth=smooth)
    obj.data.materials.append(lens_mat)
    for poly in obj.data.polygons[6:]:     # 下标 ≥ 6 的面 = 镜头
        poly.material_index = 1
    return obj


# ════════════════════════════════════════════════════════════ 场景 / 材质
def Reset_Scene():
    """清空 --factory-startup 自带的 Cube / Camera / Light 与全部网格、材质。"""
    for obj in list(bpy.context.scene.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    for coll in (bpy.data.meshes, bpy.data.materials, bpy.data.cameras,
                 bpy.data.lights, bpy.data.images):
        for item in list(coll):
            coll.remove(item)


def _Make_Material(name, color, metallic, roughness, alpha=1.0, blended=False):
    mat = bpy.data.materials.get(name)
    if mat is None:
        mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is None:
        raise RuntimeError("材质 %s 缺少 Principled BSDF 节点" % name)
    bsdf.inputs["Base Color"].default_value = (color[0], color[1], color[2], 1.0)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Alpha"].default_value = alpha
    # plan R5：只写 Base Color 的 alpha → 导出 alphaMode 丢失；必须 Alpha 插槽 + BLENDED
    mat.surface_render_method = "BLENDED" if blended else "DITHERED"
    try:
        mat.blend_method = "BLEND" if blended else "OPAQUE"
    except (AttributeError, TypeError):
        pass
    return mat


def Make_Materials():
    """全工程恰好 3 种材质：金属灰 / 哑光黑 / 半透明蓝。"""
    return {
        "metal": _Make_Material(METAL_GRAY_NAME, METAL_GRAY_COLOR, METAL_METALLIC, METAL_ROUGH),
        "black": _Make_Material(MATTE_BLACK_NAME, MATTE_BLACK_COLOR, BLACK_METALLIC, BLACK_ROUGH),
        "glass": _Make_Material(GLASS_BLUE_NAME, GLASS_BLUE_COLOR, GLASS_METALLIC,
                                GLASS_ROUGH, alpha=GLASS_ALPHA, blended=True),
    }


def Make_Materials_M1():
    """ugv_m1：4 种材质（深灰 / 浅灰 / 黑 / 蓝点缀），无贴图 → 不产生任何 sidecar .png。

    材质名统一 ORION_UGV_*（**不含** nose）：check_models 的 by-mat 通道不能再命中第二个「鼻子」。
    """
    return {
        "dark": _Make_Material(M1_DARK_NAME, M1_DARK_COLOR, M1_DARK_METALLIC, M1_DARK_ROUGH),
        "light": _Make_Material(M1_LIGHT_NAME, M1_LIGHT_COLOR, M1_LIGHT_METALLIC, M1_LIGHT_ROUGH),
        "black": _Make_Material(M1_BLACK_NAME, M1_BLACK_COLOR, M1_BLACK_METALLIC, M1_BLACK_ROUGH),
        "blue": _Make_Material(M1_BLUE_NAME, M1_BLUE_COLOR, M1_BLUE_METALLIC, M1_BLUE_ROUGH),
    }


# ════════════════════════════════════════════════════════════ 度量 / 断言
def Model_Aabb(objects):
    mn = [float("inf")] * 3
    mx = [float("-inf")] * 3
    for obj in objects:
        mw = obj.matrix_world
        for vert in obj.data.vertices:
            co = mw @ vert.co
            for axis in range(3):
                mn[axis] = min(mn[axis], co[axis])
                mx[axis] = max(mx[axis], co[axis])
    return tuple(mn), tuple(mx)


def Model_Tris(objects):
    total = 0
    for obj in objects:
        for poly in obj.data.polygons:
            total += len(poly.vertices) - 2
    return total


def Assert_Model(label, objects):
    """AABB 逐值差 ≤ AABB_TOL + 三角数 ≤ 该模型上限；不符抛 RuntimeError。

    三角上限按模型查 TRI_LIMITS（传感器 2000 / ugv_m1 12000），不再一刀切。
    """
    mn, mx = Model_Aabb(objects)
    want_mn, want_mx = EXPECT[label]
    for axis in range(3):
        if abs(mn[axis] - want_mn[axis]) > AABB_TOL:
            raise RuntimeError("%s AABB min[%d]=%.6f 期望 %.6f" % (label, axis, mn[axis], want_mn[axis]))
        if abs(mx[axis] - want_mx[axis]) > AABB_TOL:
            raise RuntimeError("%s AABB max[%d]=%.6f 期望 %.6f" % (label, axis, mx[axis], want_mx[axis]))
    tris = Model_Tris(objects)
    limit = TRI_LIMITS.get(label, TRI_LIMIT)
    if tris > limit:
        raise RuntimeError("%s 三角数 %d 超预算 %d" % (label, tris, limit))
    return mn, mx, tris


def Save_Blend(path):
    """存 .blend 源文件（compress=True，与 Tool/models/*.blend 的体积口径一致）。"""
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(path), compress=True)
    return Path(path).stat().st_size


# ════════════════════════════════════════════════════════════ 三件模型
def Build_Lidar(mats):
    """雷达：底座 + 罐体 + 360° 扫描窗环带 + 顶盖「正前方」凸起。原点 = 罐体底面中心。"""
    objs = []
    base_r = LIDAR_BASE_D * 0.5
    can_r = LIDAR_CAN_D * 0.5
    win_r = WINDOW_D * 0.5
    objs.append(Add_Cylinder("Base", base_r, LIDAR_BASE_H, 0.0, SEG_BASE, mats["metal"]))
    objs.append(Add_Cylinder("Can", can_r, LIDAR_CAN_H, LIDAR_BASE_H, SEG_CAN, mats["metal"]))
    objs.append(Add_Cylinder("Window", win_r, WINDOW_Z1 - WINDOW_Z0, WINDOW_Z0,
                             SEG_WINDOW, mats["glass"], open_ends=True))
    objs.append(Add_Box("FrontBump", BUMP_SIZE,
                        (BUMP_X, 0.0, LIDAR_BASE_H + LIDAR_CAN_H + BUMP_SIZE[2] * 0.5),
                        mats["black"]))
    return objs


def Build_Camera(mats):
    """相机：机身 + 镜头筒 + 镜片 + 支架。原点 = 镜片前表面中心（光心，max.x == 0）。"""
    objs = []
    body_cx = (CAM_BODY_X0 + CAM_BODY_X1) * 0.5
    objs.append(Add_Box("Body", CAM_BODY, (body_cx, 0.0, 0.0), mats["black"]))
    barrel_r = CAM_BARREL_D * 0.5
    barrel_x_mid = CAM_BODY_X1 + CAM_BARREL_SPAN * 0.5
    verts, faces = _Cylinder_Data(barrel_r, CAM_BARREL_SPAN, SEG_BARREL, True)
    # 绕 Y 轴 +90°：(x, y, z) → (z, y, −x)，本地 z 轴 → 世界 +x 轴
    rotated = [(z + barrel_x_mid, y, -x) for (x, y, z) in verts]
    objs.append(_New_Object("Barrel", rotated, faces, mats["metal"]))
    lens_r = CAM_LENS_D * 0.5
    lens_verts, lens_faces = _Cylinder_Data(lens_r, CAM_LENS_T, SEG_LENS, True)
    lens_rot = [(z - CAM_LENS_T * 0.5, y, -x) for (x, y, z) in lens_verts]
    objs.append(_New_Object("Lens", lens_rot, lens_faces, mats["glass"]))
    bracket_z = (CAM_BODY[2] * 0.5 + CAM_BRACKET[2] * 0.5)
    objs.append(Add_Box("Bracket", CAM_BRACKET, (body_cx, 0.0, -bracket_z), mats["black"]))
    return objs


def Build_Ugv(mats):
    """UGV 底盘：下车体 + 车鼻立板 + 带前挡斜面的舱体 + 车顶平台 + 四轮。

    原点 = 车体底面中心，车头朝 +x；总包络 0.30 × 0.25 × 0.25。
    """
    objs = []
    hull_x, hull_y, hull_z = UGV_HULL
    hull_cx = (UGV_HULL_X0 + UGV_HULL_X1) * 0.5
    objs.append(Add_Box("Hull", (hull_x, hull_y, hull_z),
                        (hull_cx, 0.0, hull_z * 0.5), mats["black"]))
    nose_cx = (UGV_HULL_X1 + UGV_HULL_X1 + UGV_NOSE[0]) * 0.5
    objs.append(Add_Box("NosePlate", UGV_NOSE, (nose_cx, 0.0, UGV_NOSE[2] * 0.5), mats["metal"]))
    objs.append(Add_Prism("Cabin", UGV_CABIN_PROFILE, UGV_CABIN_HALF_Y, mats["metal"]))
    roof_h = UGV_ROOF[2]
    roof_z = UGV_CABIN_PROFILE[3][1] + roof_h * 0.5
    objs.append(Add_Box("Roof", UGV_ROOF, (0.0, 0.0, roof_z), mats["metal"]))
    for tag, sign_x, sign_y in UGV_WHEEL_TAGS:
        objs.append(Add_Wheel("Wheel" + tag,
                              (sign_x * UGV_WHEEL_X, sign_y * UGV_WHEEL_Y, UGV_WHEEL_Z),
                              UGV_WHEEL_R, UGV_WHEEL_W, SEG_WHEEL, mats["black"]))
    return objs


def Build_Ugv_M1(mats):
    """M1 风格底盘 v3（task_42 Step 2 / 人类第 2~3 轮反馈）：按**侧视轮廓**建模 ——
    下车体长条方盒 + 浅灰前保险杠(`NosePlate`) + 前摄像头(**坐在车身顶面**、镜头朝 +x)
    + 引擎盖斜坡 ∧ 平车顶(**深色**) + 车顶后段激光雷达 + 四轮麦克纳姆（银色轮毂 / 黑辊子）。

    v3 相对 v2 只动三件：车顶材质 dark、上半身整体压低（Hull 顶 0.125 / 甲板 0.200）、
    摄像头从车顶高度落到车身顶面（顶 0.160，明显低于车顶 0.210）。
    原点 = 底面中心（贴地，z_min == 0），+x = 车头；总包络 0.30 × 0.25 × 0.25。
    不建后部「喇叭/塔」（参考图后部隆起是喇叭，人类明确删）。
    麦克纳姆轮**只做视觉**（两排 45° 辊子），不引入横移运动学。
    """
    objs = []
    for tag, sign_x, sign_y in M1_WHEEL_TAGS:
        objs.append(Add_Wheel_M1(tag, sign_x, sign_y, mats["black"], mats["light"]))
    # 下车体长条方盒（深灰）
    objs.append(Add_Box("Hull", M1_HULL, M1_HULL_C, mats["dark"]))
    # 侧甲板薄板（左右各一，外凸 2 mm）
    for sign_y, tag in ((1.0, "R"), (-1.0, "L")):
        objs.append(Add_Box("SideArmor" + tag, M1_SIDE_ARMOR,
                            (0.0, sign_y * M1_SIDE_ARMOR_CY, M1_SIDE_ARMOR_CZ), mats["dark"]))
    # 引擎盖斜坡 + 上部舱体：一整块四边形棱柱 → 斜坡面与舱体之间没有内部共面
    objs.append(Add_Prism("Deck", M1_DECK_PROFILE, M1_DECK_HALF_Y, mats["dark"]))
    # 车顶平台（**深色**，v3：v2 的浅灰被人否决成了「白盖板」；车顶从斜坡顶一直到车尾）
    objs.append(Add_Box("Roof", M1_ROOF, M1_ROOF_C, mats["dark"]))
    # 车顶后段激光雷达（黑方盒，顶面 z = 0.250 撑 AABB）+ 蓝色扫描窗带
    objs.append(Add_Box("Lidar", M1_LIDAR, M1_LIDAR_C, mats["black"]))
    objs.append(Add_Box("LidarBand", M1_LIDAR_BAND, M1_LIDAR_BAND_C, mats["blue"]))
    # 前摄像头（v3：坐在车身顶面、明显低于车顶；壳 + 蓝镜头合并成单节点，AABB = x[0.115, 0.150]）
    objs.append(Add_Front_Camera_M1("FrontCamera", M1_CAM, M1_CAM_C,
                                    M1_CAM_LENS_R, M1_CAM_LENS_SEG, M1_CAM_LENS_CX,
                                    M1_CAM_LENS_L, M1_CAM_C[2],
                                    mats["black"], mats["blue"]))
    # 浅灰横向前保险杠（节点名 NosePlate，M4 by-name 命中）+ 蓝 LED 条
    objs.append(Add_Box("NosePlate", M1_NOSE_PLATE, M1_NOSE_PLATE_C, mats["light"]))
    objs.append(Add_Box("NoseLed", M1_NOSE_LED, M1_NOSE_LED_C, mats["blue"]))
    # 尾板
    objs.append(Add_Box("RearPanel", M1_REAR_PANEL, M1_REAR_PANEL_C, mats["dark"]))
    return objs


def _Node(objs, name):
    """按名取唯一节点（缺失 / 重名一律 RuntimeError，断言里不允许 KeyError 崩栈）。"""
    hits = [o for o in objs if o.name == name]
    if len(hits) != 1:
        raise RuntimeError("ugv_m1 节点 %s 数量 %d（期望 1）" % (name, len(hits)))
    return hits[0]


def _ZRange(objs, name):
    """节点的 (z_lo, z_hi)（Blender z = 高）。"""
    lo, hi = Model_Aabb([_Node(objs, name)])
    return lo[2], hi[2]


def Assert_Ugv_M1(objs):
    """ugv_m1 专属断言：节点命名 / 材质数 / 车头节点位置 / v3 逐件几何。不符 → RuntimeError → EXIT≠0。"""
    names = {o.name for o in objs}
    for tag, _, _ in M1_WHEEL_TAGS:
        if "Wheel" + tag not in names:
            raise RuntimeError("ugv_m1 缺少轮子节点 Wheel%s（check_models 按名对齐）" % tag)
    nose = next((o for o in objs if "nose" in o.name.lower()), None)
    if nose is None:
        raise RuntimeError("ugv_m1 缺少名字含 nose 的节点（M4 依赖）")
    nlo, nhi = Model_Aabb([nose])
    if (nlo[0] + nhi[0]) * 0.5 <= 0.02:
        raise RuntimeError("ugv_m1 NosePlate 中心 x=%.4f 未过 0.02（M4 只认 +x 半轴）"
                           % ((nlo[0] + nhi[0]) * 0.5))
    if len(bpy.data.materials) > 5:
        raise RuntimeError("ugv_m1 材质数 %d > 5" % len(bpy.data.materials))

    # ── v3（人类第 3 轮）：逐件硬判，容差 1e-4
    def near(a, b):
        return abs(a - b) <= 1e-4

    def z_is(name, z0, z1):
        lo, hi = _ZRange(objs, name)
        if not (near(lo, z0) and near(hi, z1)):
            raise RuntimeError("ugv_m1 %s z=[%.4f,%.4f] 期望 [%.4f,%.4f]" % (name, lo, hi, z0, z1))

    def aabb_is(name, x0, x1, y0, y1, z0, z1):
        lo, hi = Model_Aabb([_Node(objs, name)])
        got = (lo[0], hi[0], lo[1], hi[1], lo[2], hi[2])
        want = (x0, x1, y0, y1, z0, z1)
        if not all(near(g, w) for g, w in zip(got, want)):
            raise RuntimeError("ugv_m1 %s x[%.4f,%.4f] y[%.4f,%.4f] z[%.4f,%.4f] "
                               "期望 x[%.4f,%.4f] y[%.4f,%.4f] z[%.4f,%.4f]" % ((name,) + got + want))

    # ① 车身压低（人类 v3 表格逐件硬判）
    z_is("Hull", 0.0300, 0.1250)
    z_is("SideArmorR", 0.0450, 0.1200)
    z_is("SideArmorL", 0.0450, 0.1200)
    z_is("Deck", 0.1245, 0.2000)
    z_is("Roof", 0.1995, 0.2100)
    z_is("Lidar", 0.2095, 0.2500)
    z_is("RearPanel", 0.0450, 0.1200)
    aabb_is("FrontCamera", 0.1150, 0.1500, -0.0280, 0.0280, 0.1245, 0.1600)   # 壳 + 镜头并集
    deck_hi = Model_Aabb([_Node(objs, "Deck")])[1]
    if not near(deck_hi[0], 0.0900):
        raise RuntimeError("ugv_m1 Deck 引擎盖前缘 x_hi=%.4f 期望 0.0900（v3 斜坡起点）" % deck_hi[0])
    # v3 第 1 条：车顶必须**深色**（v2 的浅灰被否决成「白盖板」）
    roof_mat = _Node(objs, "Roof").data.materials[0].name
    if roof_mat != M1_DARK_NAME:
        raise RuntimeError("ugv_m1 Roof 材质 %s，v3 要求深色 %s" % (roof_mat, M1_DARK_NAME))
    # ② 摄像头「明显低于车顶」：顶面 ≤ 车顶底面 − 0.030
    cam_lo, cam_hi = Model_Aabb([_Node(objs, "FrontCamera")])
    roof_z_lo = _ZRange(objs, "Roof")[0]
    if cam_hi[2] > roof_z_lo - 0.030:
        raise RuntimeError("ugv_m1 FrontCamera 顶 z=%.4f 未明显低于车顶（车顶底 z=%.4f，要求差 ≥0.030）"
                           % (cam_hi[2], roof_z_lo))
    # ③ 斜坡必须从摄像头**后方**起坡（不是从摄像头顶上起坡）
    if deck_hi[0] > cam_lo[0] - 0.020:
        raise RuntimeError("ugv_m1 引擎盖斜坡起点 x=%.4f 未退到摄像头后方（摄像头前缘 x=%.4f）"
                           % (deck_hi[0], cam_lo[0]))
    # ④ 禁「喇叭塔」：除激光雷达外，任何构件都不得高过车顶平台顶面 0.210
    for obj in objs:
        if obj.name in ("Lidar", "LidarBand"):
            continue
        ohi = Model_Aabb([obj])[1]
        if ohi[2] > 0.2100 + 1e-4:
            raise RuntimeError("ugv_m1 %s 高过车顶平台（z=%.4f），只允许激光雷达高出"
                               % (obj.name, ohi[2]))


# ════════════════════════════════════════════════════════════ 驱动
BUILDERS = {
    "lidar": ("lidar_tmini", Build_Lidar),
    "camera": ("camera_module", Build_Camera),
    "ugv": ("ugv_chassis", Build_Ugv),
    "ugv_m1": ("ugv_m1", Build_Ugv_M1),
}
DEFAULT_ORDER = ("lidar", "camera", "ugv")


def main(argv):
    wanted = [arg for arg in argv if not arg.startswith("-")] or list(DEFAULT_ORDER)
    unknown = [arg for arg in wanted if arg not in BUILDERS]
    if unknown:
        raise RuntimeError("未知模型键：%s（可选 lidar/camera/ugv/ugv_m1）" % ",".join(unknown))
    for key in wanted:
        label, builder = BUILDERS[key]
        Reset_Scene()
        mats = Make_Materials_M1() if key == "ugv_m1" else Make_Materials()
        objs = builder(mats)
        if any(obj.type != "MESH" for obj in objs):
            raise RuntimeError("%s 出现非 MESH 对象（碰撞体 / 灯光一律禁止）" % label)
        mn, mx, tris = Assert_Model(label, objs)
        if key == "ugv_m1":
            Assert_Ugv_M1(objs)
        blend_path = BLEND_OUT_DIR / (label + ".blend")
        size_b = Save_Blend(blend_path)
        size = (mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2])
        print("[build] %-13s objects=%d  tris=%d  min=(%.4f,%.4f,%.4f) max=(%.4f,%.4f,%.4f) "
              "size=(%.4f,%.4f,%.4f)  blend=-> %s (%.1f KB)"
              % (label, len(objs), tris, mn[0], mn[1], mn[2], mx[0], mx[1], mx[2],
                 size[0], size[1], size[2], blend_path, size_b / 1024.0))
    print("[build] done  models=%s" % ",".join(wanted))
    return 0


def Cli_Args(argv):
    """只取 `--` 之后的参数（Blender 自己的 argv 里混着路径与开关，不能直接当模型键）。"""
    if "--" in argv:
        return argv[argv.index("--") + 1:]
    return []


if __name__ == "__main__":
    sys.exit(main(Cli_Args(sys.argv)))
