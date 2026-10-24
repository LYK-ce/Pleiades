#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-23
"""task_33 Step 1 — 传感器 / 底盘低模构建脚本（Blender 4.5 无头，幂等）。

用法（工程根）：
    /vepfs-mlp2/c20250205/240804016/tool/blender/blender -b --factory-startup \\
        --python-exit-code 1 --python Tool/blender/models/build_models.py [lidar|camera|ugv ...]

产物（每个模型一个 .blend 源文件；glb 导出交给 export_all.sh → tool/blender/blend2glb.sh）：
    Tool/blender/models/lidar_tmini.blend
    Tool/blender/models/camera_module.blend
    Tool/blender/models/ugv_chassis.blend

坐标口径（与 Tool/models/make_ugv.py 一致）：
    1 BU = 1 m；Blender Z-up（+z = 高），+x = 车头 / 镜头朝向。
    网格坐标 = 世界坐标，所有对象 transform = IDENTITY（导出后 glTF 节点无 TRS）。
    export_yup=True 后 Godot 侧为 (x 长, y 高, z 宽) = Blender (x, z, -y)。

原点约定（task_33 D8）：
    lidar_tmini   → 罐体底面中心（= 扫描面中心），z_min == 0
    camera_module → 镜片前表面中心（= 光心），max.x == 0
    ugv_chassis   → 车体底面中心（贴地），z_min == 0

失败语义：任何 AABB / 三角数断言不符 → RuntimeError → 进程 EXIT≠0
（配合 --python-exit-code 1 与 export_all.sh 的 set -e，不会产出半成品 glb）。
"""

import math
import sys
from pathlib import Path

import bmesh
import bpy

# ──────────────────────────────────────────────────────────── 路径常量
PROJECT_ROOT = Path(__file__).resolve().parents[3]  # models → blender → Tool → Orion
BLEND_DIR = PROJECT_ROOT / "Tool/blender/models"
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

# ──────────────────────────────────────────────────────────── 预算 / 断言
TRI_LIMIT = 2000
GLB_LIMIT_B = 100 * 1024
AABB_TOL = 1e-4
EXPECT = {
    # 期望 AABB（Blender 世界系，米）
    "lidar_tmini": ((-0.040, -0.040, 0.000), (0.040, 0.040, 0.054)),
    "camera_module": ((-0.055, -0.015, -0.021), (0.000, 0.015, 0.015)),
    "ugv_chassis": ((-0.150, -0.125, 0.000), (0.150, 0.125, 0.250)),
}


# ════════════════════════════════════════════════════════════ 底层网格工具
def _New_Object(name, verts, faces, material, recalc=True):
    """用顶点 / 面表建一个 MeshInstance3D 的 Blender 对应物（对象 transform 恒 identity）。"""
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
    """AABB 逐值差 ≤ AABB_TOL + 三角数 ≤ TRI_LIMIT；不符抛 RuntimeError。"""
    mn, mx = Model_Aabb(objects)
    want_mn, want_mx = EXPECT[label]
    for axis in range(3):
        if abs(mn[axis] - want_mn[axis]) > AABB_TOL:
            raise RuntimeError("%s AABB min[%d]=%.6f 期望 %.6f" % (label, axis, mn[axis], want_mn[axis]))
        if abs(mx[axis] - want_mx[axis]) > AABB_TOL:
            raise RuntimeError("%s AABB max[%d]=%.6f 期望 %.6f" % (label, axis, mx[axis], want_mx[axis]))
    tris = Model_Tris(objects)
    if tris > TRI_LIMIT:
        raise RuntimeError("%s 三角数 %d 超预算 %d" % (label, tris, TRI_LIMIT))
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


# ════════════════════════════════════════════════════════════ 驱动
BUILDERS = {
    "lidar": ("lidar_tmini", Build_Lidar),
    "camera": ("camera_module", Build_Camera),
    "ugv": ("ugv_chassis", Build_Ugv),
}
DEFAULT_ORDER = ("lidar", "camera", "ugv")


def main(argv):
    wanted = [arg for arg in argv if not arg.startswith("-")] or list(DEFAULT_ORDER)
    unknown = [arg for arg in wanted if arg not in BUILDERS]
    if unknown:
        raise RuntimeError("未知模型键：%s（可选 lidar/camera/ugv）" % ",".join(unknown))
    for key in wanted:
        label, builder = BUILDERS[key]
        Reset_Scene()
        mats = Make_Materials()
        objs = builder(mats)
        if any(obj.type != "MESH" for obj in objs):
            raise RuntimeError("%s 出现非 MESH 对象（碰撞体 / 灯光一律禁止）" % label)
        mn, mx, tris = Assert_Model(label, objs)
        blend_path = BLEND_DIR / (label + ".blend")
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
