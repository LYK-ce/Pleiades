#!/usr/bin/env python3
"""UAV 四旋翼低模生成脚本（Blender 4.5+ 无头模式，幂等）。

用法（工程根）:
    /vepfs-mlp2/c20250205/240804016/tool/blender/blender -b \
        --python-exit-code 1 --python Tool/models/make_uav.py

产物:
    Tool/models/uav_quadrotor_lowpoly.blend             源文件（含相机/灯光）
    Tool/models/preview/uav_{front,side,top}.png         三视图 480x360
    orion-simulation-world/assets/uav_quadrotor_lowpoly.glb   交付物（只含 9 个飞行器对象）

尺寸口径：1 BU = 1 m；Blender +Z 上；机头朝 +X。
    Quadrotor.gd:68   _size        = 0.40  → 机身 X/Y 边长
    Quadrotor.gd:50   BODY_HEIGHT  = 0.12  → 机身 Z 高
    Quadrotor.gd:52   ROTOR_LIFT   = 0.01  → 旋翼中心 z = 0.12 + 0.01 = 0.13
    Quadrotor.gd:248 旋翼中心 x = ±_size*0.5 = ±0.20
    Quadrotor.gd:249 旋翼中心 Godot z = ∓0.20 → Blender y = ±0.20
    Quadrotor.tscn:17-20  旋翼 top/bottom_radius = 0.12、height = 0.02、segments = 16
"""

import json
import math
import struct
import sys
import time
from pathlib import Path

import bmesh
import bpy
from mathutils import Vector

# ---------------------------------------------------------------- 常量 / 路径

PROJECT_ROOT = Path(__file__).resolve().parents[2]          # Tool/models/make_uav.py → Orion
BLEND_PATH = PROJECT_ROOT / "Tool/models/uav_quadrotor_lowpoly.blend"
GLB_PATH = PROJECT_ROOT / "orion-simulation-world/assets/uav_quadrotor_lowpoly.glb"
PREVIEW_DIR = PROJECT_ROOT / "Tool/models/preview"
PREVIEWS = {
    "front": PREVIEW_DIR / "uav_front.png",
    "side": PREVIEW_DIR / "uav_side.png",
    "top": PREVIEW_DIR / "uav_top.png",
}

# 机身尺寸（米）
HX = 0.200          # 0.40 / 2  (Quadrotor.gd:68)
HY = 0.200          # 0.40 / 2  (Quadrotor.gd:68)
GZ = 0.120          #           (Quadrotor.gd:50 BODY_HEIGHT)
SX = 0.185          # 机头收窄起点 x（15 mm 内收）
NY = 0.070          # 机头正面半宽 → 前脸 0.14 宽
NZ0 = 0.030         # 机头正面下沿
NZ1 = 0.090         # 机头正面上沿 → 前脸 0.06 高

# 旋翼（Quadrotor.tscn:17-20 / Quadrotor.gd:52）
ROTOR_R = 0.120
ROTOR_H = 0.020
ROTOR_SEG = 16
ROTOR_X = 0.200     # Quadrotor.gd:248
ROTOR_Y = 0.200     # Quadrotor.gd:249（Blender y）
ROTOR_Z = 0.130     # BODY_HEIGHT + ROTOR_LIFT

# 机臂（step.md:20 截面 0.03 x 0.02）
ARM_W = 0.030
ARM_BASE_Z0 = 0.118     # 底面埋进机身顶 2 mm（不共面、不悬空）
ARM_BASE_Z1 = 0.138     # 顶面低于旋翼盘顶 0.14
# 四根机臂都从机身中心 (0,0) 出发，顶/底两面若完全同高，就会在 X 交叉处出现
# 「两块严格共面的面重叠」——Cycles 会让它们互相遮挡成一片死黑（实测 19x18 px 黑斑）。
# 每条臂递增 0.4 mm：共面变成 0.4 mm 台阶（480 px 视口下 < 1 px，看不出来），
# 交叉处不再有共面重叠；最高 0.1392 仍低于旋翼盘顶 0.14。
ARM_Z_STEP = 0.0004
ARM_Z_OFFSETS = {"ArmFL": 0.0, "ArmFR": 0.0004, "ArmRL": 0.0008, "ArmRR": 0.0012}

# 颜色（step.md:22）—— 直接作为 Principled 线性基色，不做 sRGB→linear 转换
COL_BODY = (0.30, 0.45, 0.65)
COL_ROTOR = (0.15, 0.16, 0.18)
COL_NOSE = (0.90, 0.20, 0.15)
MAT_BODY = "UAV_BodyBlueGray"
MAT_ROTOR = "UAV_RotorDarkGray"
MAT_NOSE = "UAV_NoseRed"

TRI_LIMIT = 3000
SAMPLES = 32                                                # step.md:15 上限 64
RES_X, RES_Y = 480, 360
ORTHO_SCALE = 0.90                                          # 覆盖旋翼跨度 0.64
CAM_Z = 0.07                                                # 整机竖直中点 (0.14/2)

# §3.1 顶点表（世界坐标 = 网格坐标，对象都在原点）
BODY_VERTS = [
    (-HX, -HY, 0.000), (-HX, HY, 0.000), (-HX, HY, GZ), (-HX, -HY, GZ),   # 0-3 机尾
    (SX, -HY, 0.000), (SX, HY, 0.000), (SX, HY, GZ), (SX, -HY, GZ),       # 4-7 收窄起点
    (HX, -NY, NZ0), (HX, NY, NZ0), (HX, NY, NZ1), (HX, -NY, NZ1),         # 8-11 机头正面
]
BODY_FACES = [
    (0, 1, 2, 3),      # 0 机尾
    (0, 4, 5, 1),      # 1 底面·后段
    (4, 8, 9, 5),      # 2 底面·收窄段
    (3, 2, 6, 7),      # 3 顶面·后段
    (7, 6, 10, 11),    # 4 顶面·收窄段（红）
    (0, 3, 7, 4),      # 5 左侧·后段
    (4, 7, 11, 8),     # 6 左侧·收窄段（红）
    (1, 5, 6, 2),      # 7 右侧·后段
    (5, 9, 10, 6),     # 8 右侧·收窄段（红）
    (8, 11, 10, 9),    # 9 机头正面（红）
]
RED_FACE_IDS = (4, 6, 8, 9)                 # §3.1：按面表下标显式指定，不靠法线猜

# §3.2 旋翼摆位：Blender +Y = Godot 左侧（export_yup: (x,y,z) → (x,z,-y)）
ROTORS = [
    ("RotorFL", ROTOR_X, ROTOR_Y, ROTOR_Z),      # Godot ( 0.2, 0.13, -0.2)
    ("RotorFR", ROTOR_X, -ROTOR_Y, ROTOR_Z),     # Godot ( 0.2, 0.13,  0.2)
    ("RotorRL", -ROTOR_X, ROTOR_Y, ROTOR_Z),     # Godot (-0.2, 0.13, -0.2)
    ("RotorRR", -ROTOR_X, -ROTOR_Y, ROTOR_Z),    # Godot (-0.2, 0.13,  0.2)
]

# §3.3 机臂箱体的 6 个四边形（顶点下标见 build_arms 的生成顺序）
ARM_FACES = [
    (0, 2, 6, 4),      # 底
    (1, 5, 7, 3),      # 顶
    (0, 4, 5, 1),      # 侧面 dw-
    (2, 3, 7, 6),      # 侧面 dw+
    (0, 1, 3, 2),      # 端面 s=0（机身中心）
    (4, 6, 7, 5),      # 端面 s=R（旋翼中心）
]

VEHICLE_OBJECTS = ["Body", "RotorFL", "RotorFR", "RotorRL", "RotorRR",
                   "ArmFL", "ArmFR", "ArmRL", "ArmRR"]
EXPECT_TRIS = 308           # 20 + 4*60 + 4*12
EXPECT_PRIMS = 10           # Body 占 2 个材质 → 2 个 primitive，其余 8 个对象各 1 个
EXPECT_NODES = 9

SCENE = None


# ---------------------------------------------------------------- 工具

def log(msg):
    print("[make_uav] " + msg, flush=True)


def fmt3(v):
    return "(%0.3f,%0.3f,%0.3f)" % (v[0], v[1], v[2])


def aabb_of(objects):
    lo = [float("inf")] * 3
    hi = [float("-inf")] * 3
    for ob in objects:
        for corner in ob.bound_box:
            w = ob.matrix_world @ Vector(corner)
            for i in range(3):
                lo[i] = min(lo[i], w[i])
                hi[i] = max(hi[i], w[i])
    return lo, hi


def select_only(objs, active=None):
    bpy.ops.object.select_all(action="DESELECT")
    for ob in objs:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = active or objs[0]


def mesh_tris(ob):
    me = ob.data
    me.calc_loop_triangles()
    return len(me.loop_triangles)


def object_size(ob):
    lo, hi = aabb_of([ob])
    return [hi[i] - lo[i] for i in range(3)]


# ---------------------------------------------------------------- 1 清场

def reset_scene():
    global SCENE
    bpy.ops.wm.read_factory_settings(use_empty=True)      # 幂等：不留 Cube/Camera/Light
    SCENE = bpy.context.scene
    SCENE.unit_settings.system = "METRIC"
    SCENE.unit_settings.scale_length = 1.0
    SCENE.unit_settings.length_unit = "METERS"
    if bpy.app.version < (4, 5, 0):
        raise RuntimeError("需要 Blender >= 4.5，当前 %s" % bpy.app.version_string)
    log("blender %s / cycles cpu / samples=%d / %dx%d"
        % (bpy.app.version_string.split()[0], SAMPLES, RES_X, RES_Y))


# ---------------------------------------------------------------- 2 材质

def make_material(name, color, roughness):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]           # 按节点名取，不用 nodes[0]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Metallic"].default_value = 0.0
    bsdf.inputs["Roughness"].default_value = roughness
    mat.diffuse_color = (*color, 1.0)                       # 视口兜底
    mat.roughness = roughness
    log("material %-18s base=%s rough=%.2f metallic=0.00"
        % (name, fmt3(color), roughness))
    return mat


def make_materials():
    return (
        make_material(MAT_BODY, COL_BODY, 0.55),
        make_material(MAT_ROTOR, COL_ROTOR, 0.85),
        make_material(MAT_NOSE, COL_NOSE, 0.45),
    )


# ---------------------------------------------------------------- 3 机身

def build_body(mat_body, mat_nose):
    me = bpy.data.meshes.new("Body")
    ob = bpy.data.objects.new("Body", me)
    bpy.context.collection.objects.link(ob)

    bm = bmesh.new()
    verts = [bm.verts.new(c) for c in BODY_VERTS]
    bm.verts.ensure_lookup_table()
    for fi, idx in enumerate(BODY_FACES):
        f = bm.faces.new([verts[i] for i in idx])
        f.material_index = 1 if fi in RED_FACE_IDS else 0   # 建面时就定材质，不依赖顺序
    bm.faces.ensure_lookup_table()
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])    # 统一外法线
    bm.normal_update()

    red = 0
    nose_face = None
    deepest = None
    for f in bm.faces:
        if f.material_index == 1:
            red += 1
            if deepest is None or f.calc_center_median().x > deepest.x:
                deepest = f.calc_center_median()
        if nose_face is None or f.normal.x > nose_face.normal.x:
            nose_face = f
    nose_mat = nose_face.material_index                   # bm.free() 前先取值

    bm.to_mesh(me)
    bm.free()
    me.validate(verbose=False)
    me.materials.append(mat_body)                            # slot 0
    me.materials.append(mat_nose)                            # slot 1
    me.update()

    if red != 4:
        raise RuntimeError("机头红面数应为 4，实际 %d" % red)
    if nose_mat != 1:
        raise RuntimeError("机头正面（法线 +X 最强）未标记为红")
    if deepest is None or deepest.x < SX - 1e-9:
        raise RuntimeError("红面未落在机头收窄段内")
    log("Body      tris=%d  verts=%d  red_faces=%d"
        % (mesh_tris(ob), len(me.vertices), red))
    return ob


# ---------------------------------------------------------------- 4 旋翼

def build_rotors(mat_rotor):
    objs = []
    for name, x, y, z in ROTORS:
        bpy.ops.mesh.primitive_cylinder_add(
            vertices=ROTOR_SEG, radius=ROTOR_R, depth=ROTOR_H,
            end_fill_type="NGON", location=(x, y, z),
            # 不加 rotation —— 圆柱默认轴就是 +Z，旋翼是平盘
        )
        ob = bpy.context.active_object
        ob.name = name
        ob.data.name = name
        select_only([ob])
        bpy.ops.object.shade_flat()
        ob.data.materials.append(mat_rotor)
        objs.append(ob)

        if (abs(ob.location.x - x) > 1e-6 or abs(ob.location.y - y) > 1e-6
                or abs(ob.location.z - z) > 1e-6):
            raise RuntimeError("%s 位置被改动: %s" % (name, fmt3(ob.location)))
        size = object_size(ob)
        for axis, want in ((0, 2 * ROTOR_R), (1, 2 * ROTOR_R), (2, ROTOR_H)):
            if abs(size[axis] - want) > 1e-6:                # z 才是薄的那一维
                raise RuntimeError("%s 包围盒第 %d 轴 %.6f != %.6f（轴被转错？）"
                                   % (name, axis, size[axis], want))
        log("%-9s tris=%d  loc=%s" % (name, mesh_tris(ob), fmt3(ob.location)))
    return objs


# ---------------------------------------------------------------- 5 机臂

def build_arms(mat_body):
    objs = []
    for name, sx, sy, _sz in ROTORS:
        arm_name = "Arm" + name[-2:]
        dz_off = ARM_Z_OFFSETS[arm_name]
        z0, z1 = ARM_BASE_Z0 + dz_off, ARM_BASE_Z1 + dz_off
        a = math.atan2(sy, sx)
        u = Vector((math.cos(a), math.sin(a), 0.0))          # 沿对角线方向
        p = Vector((-math.sin(a), math.cos(a), 0.0))         # 横向
        R = math.hypot(sx, sy)                               # 0.282843
        verts = []
        for s in (0.0, R):                                   # 机身中心 → 旋翼中心
            c = u * s
            for dw in (-ARM_W / 2.0, ARM_W / 2.0):
                for dz in (z0, z1):
                    verts.append(tuple(c + p * dw + Vector((0.0, 0.0, dz))))

        me = bpy.data.meshes.new(arm_name)
        ob = bpy.data.objects.new(arm_name, me)
        bpy.context.collection.objects.link(ob)
        bm = bmesh.new()
        bv = [bm.verts.new(c) for c in verts]
        bm.verts.ensure_lookup_table()
        for idx in ARM_FACES:
            bm.faces.new([bv[i] for i in idx])
        bm.faces.ensure_lookup_table()
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
        bm.to_mesh(me)
        bm.free()
        me.validate(verbose=False)
        me.materials.append(mat_body)                        # 机臂用机身材质
        me.update()
        objs.append(ob)

        lo, hi = aabb_of([ob])
        if abs(lo[2] - z0) > 1e-6 or abs(hi[2] - z1) > 1e-6:
            raise RuntimeError("%s z 范围错: %s ~ %s" % (arm_name, lo[2], hi[2]))
        if z1 >= ROTOR_Z + ROTOR_H / 2.0:
            raise RuntimeError("%s 顶面 %.4f 不低于旋翼盘顶" % (arm_name, z1))
        if z0 >= GZ:
            raise RuntimeError("%s 底面 %.4f 未埋进机身顶 %.3f（会悬空）" % (arm_name, z0, GZ))
        for v in verts:
            if abs(v[0]) > ROTOR_X + ARM_W / 2.0 + 1e-6 or abs(v[1]) > ROTOR_Y + ARM_W / 2.0 + 1e-6:
                raise RuntimeError("%s 顶点越界: %s" % (arm_name, fmt3(v)))
        # 端面四角必须落在旋翼盘正下方（被盘遮住），证明"连得上"
        tip = max(math.hypot(v[0] - sx, v[1] - sy) for v in verts[4:])   # 只取 s=R 端
        if tip > ROTOR_R:
            raise RuntimeError("%s 端点超出旋翼盘半径: %.6f" % (arm_name, tip))
        log("%-9s tris=%d  bbox_z=(%.4f,%.4f)  tip_r=%.6f"
            % (arm_name, mesh_tris(ob), lo[2], hi[2], R))
    return objs


# ---------------------------------------------------------------- 6 相机/灯光/渲染设置

def add_sun(name, energy, elev_deg, azim_deg):
    data = bpy.data.lights.new(name, type="SUN")
    data.energy = energy
    data.angle = math.radians(12.0)
    ob = bpy.data.objects.new(name, data)
    bpy.context.collection.objects.link(ob)
    ob.location = (0.0, 0.0, 1.0)
    ob.rotation_euler = (math.radians(90.0 - elev_deg), 0.0, math.radians(azim_deg))
    return ob


def setup_rig():
    cam_data = bpy.data.cameras.new("PreviewCam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = ORTHO_SCALE
    cam_data.clip_start = 0.01
    cam_data.clip_end = 100.0
    cam = bpy.data.objects.new("PreviewCam", cam_data)
    bpy.context.collection.objects.link(cam)
    SCENE.camera = cam

    add_sun("Key", 2.2, 52.0, 35.0)
    add_sun("Fill", 1.0, 78.0, -150.0)
    add_sun("Rim", 1.2, 20.0, 190.0)

    world = bpy.data.worlds.new("PreviewWorld")
    world.use_nodes = True
    bg = world.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (0.32, 0.33, 0.35, 1.0)
    bg.inputs["Strength"].default_value = 0.55
    SCENE.world = world

    r = SCENE.render
    r.engine = "CYCLES"                    # 后台 enum 只报 EEVEE_NEXT，但赋值有效
    if r.engine != "CYCLES":
        raise RuntimeError("无法切到 Cycles，实际 %r" % r.engine)
    SCENE.cycles.device = "CPU"
    SCENE.cycles.samples = SAMPLES
    SCENE.cycles.use_denoising = True
    SCENE.cycles.use_adaptive_sampling = True
    try:
        SCENE.view_settings.view_transform = "Standard"   # 默认 AgX 会洗淡蓝/红
        SCENE.view_settings.look = "None"
    except Exception as exc:                              # 拿不到就保持默认
        log("warn: view_transform 保持默认 (%s)" % exc)
    SCENE.view_settings.exposure = 0.0
    r.resolution_x = RES_X
    r.resolution_y = RES_Y
    r.resolution_percentage = 100
    r.film_transparent = False
    r.image_settings.file_format = "PNG"
    r.image_settings.color_mode = "RGB"
    r.image_settings.compression = 15
    r.filepath = str(PREVIEWS["front"])
    log("rig ok: ortho_scale=%.2f view_transform=%s"
        % (ORTHO_SCALE, SCENE.view_settings.view_transform))
    return cam


# ---------------------------------------------------------------- 7 三视图

VIEWS = {
    #  name: (location, rotation_euler)
    "front": ((1.5, 0.0, CAM_Z), (math.pi / 2.0, 0.0, math.pi / 2.0)),   # 看 -X，正对红机头
    "side": ((0.0, -1.5, CAM_Z), (math.pi / 2.0, 0.0, 0.0)),             # 看 +Y，机头朝右
    "top": ((0.0, 0.0, 1.5), (0.0, 0.0, 0.0)),                           # 看 -Z，+X 朝右
}


def png_size(path):
    data = path.read_bytes()[:33]
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise RuntimeError("不是 PNG: %s" % path)
    w, h = struct.unpack_from(">II", data, 16)
    return w, h


def center_dark_count(path, box=100, thr=0.02):
    """中心 box×box 区域内"死黑"像素数（三通道都 < thr）。

    机臂顶面完全共面重叠时，Cycles 会让两块面互相遮挡，交叉处渲染成一片纯黑；
    这个计数就是那条缺陷的回归守卫（正常渲染中心区几乎为 0）。
    """
    img = bpy.data.images.load(str(path), check_existing=False)
    try:
        img.colorspace_settings.name = "Non-Color"       # 读原始值，不做色彩空间换算
        w, h = img.size
        px = img.pixels[:]
        x0, y0 = (w - box) // 2, (h - box) // 2
        dark = 0
        for yy in range(y0, y0 + box):
            base = yy * w
            for xx in range(x0, x0 + box):
                i = (base + xx) * 4
                if max(px[i], px[i + 1], px[i + 2]) < thr:
                    dark += 1
        return dark
    finally:
        bpy.data.images.remove(img)


def render_views(cam):
    for name in ("front", "side", "top"):
        loc, rot = VIEWS[name]
        cam.location = loc
        cam.rotation_euler = rot
        cam.data.ortho_scale = ORTHO_SCALE
        out = PREVIEWS[name]
        SCENE.render.filepath = str(out)
        try:
            bpy.ops.render.render(write_still=True)
        except RuntimeError:
            if not SCENE.cycles.use_denoising:
                raise
            log("warn: denoise 失败，关闭后重渲 %s" % name)
            SCENE.cycles.use_denoising = False
            bpy.ops.render.render(write_still=True)
        if not out.exists():
            raise RuntimeError("预览图未生成: %s" % out)
        size = out.stat().st_size
        if size <= 1024:
            raise RuntimeError("预览图过小: %s %d B" % (out, size))
        w, h = png_size(out)
        if (w, h) != (RES_X, RES_Y):
            raise RuntimeError("预览图尺寸 %dx%d != %dx%d" % (w, h, RES_X, RES_Y))
        log("render ok: %s %dx%d %d B" % (out, w, h, size))
        if name == "top":
            dark = center_dark_count(out)
            if dark > 50:
                raise RuntimeError("俯视图中心有 %d 个死黑像素（几何共面重叠？）" % dark)
            log("top view center ok: dark_px=%d/10000" % dark)


# ---------------------------------------------------------------- 8 导出 GLB

def export_glb(objs):
    select_only(objs, active=bpy.data.objects["Body"])
    bpy.ops.export_scene.gltf(
        filepath=str(GLB_PATH),
        export_format="GLB",
        export_yup=True,
        export_apply=True,          # 应用修改器
        use_selection=True,         # 只导 9 个飞行器对象
        export_cameras=False,
        export_lights=False,
        export_extras=False,
        export_materials="EXPORT",
        export_normals=True,
        export_texcoords=False,
    )
    if not GLB_PATH.exists():
        raise RuntimeError("GLB 未生成: %s" % GLB_PATH)
    size = GLB_PATH.stat().st_size
    if size <= 1024:
        raise RuntimeError("GLB 过小: %d B" % size)
    log("export ok: %s %d B" % (GLB_PATH, size))
    return size


# ---------------------------------------------------------------- 9 自检 GLB

def verify_glb():
    data = GLB_PATH.read_bytes()
    magic, version, length = struct.unpack_from("<4sII", data, 0)
    if magic != b"glTF":
        raise RuntimeError("GLB magic 错: %r" % magic)
    if version != 2:
        raise RuntimeError("GLB version 错: %d" % version)
    if length != len(data):
        raise RuntimeError("GLB 头长度 %d != 文件 %d" % (length, len(data)))
    chunk_len, chunk_type = struct.unpack_from("<I4s", data, 12)
    if chunk_type != b"JSON":
        raise RuntimeError("首个 chunk 不是 JSON: %r" % chunk_type)
    doc = json.loads(data[20:20 + chunk_len].decode("utf-8"))

    log("glb header ok: magic=%s version=%d length=%d json_chunk=%d B"
        % (magic.decode(), version, length, chunk_len))

    names = [n.get("name") for n in doc.get("nodes", [])]
    if len(names) != EXPECT_NODES or set(names) != set(VEHICLE_OBJECTS):
        raise RuntimeError("节点清单不符: %r" % names)
    if doc.get("cameras"):
        raise RuntimeError("GLB 里带了相机: %r" % doc["cameras"])
    ext = doc.get("extensionsUsed", [])
    if "KHR_lights_punctual" in ext:
        raise RuntimeError("GLB 里带了灯光: %r" % ext)
    log("glb nodes=%d %s cameras=0 lights=0" % (len(names), sorted(names)))

    # R1 复核：Blender (x,y,z) → glTF (x,z,-y)，旋翼左右不能反
    for node in doc["nodes"]:
        want = {n: (x, z, -y) for n, x, y, z in ROTORS}.get(node["name"])
        got = node.get("translation", [0.0, 0.0, 0.0])
        if want is None:
            continue
        if max(abs(got[i] - want[i]) for i in range(3)) > 1e-5:
            raise RuntimeError("%s 导出位置错: %r != %r" % (node["name"], got, want))

    tris = 0
    prims = 0
    for mesh in doc.get("meshes", []):
        for prim in mesh["primitives"]:
            tris += doc["accessors"][prim["indices"]]["count"] // 3
            prims += 1
    if tris != EXPECT_TRIS or tris >= TRI_LIMIT:
        raise RuntimeError("GLB 三角数异常: %d" % tris)
    if prims != EXPECT_PRIMS:
        raise RuntimeError("GLB prim 数应为 %d（Body 双材质拆 2 个），实际 %d"
                           % (EXPECT_PRIMS, prims))

    mats = doc.get("materials", [])
    if len(mats) != 3:
        raise RuntimeError("材质数应为 3，实际 %d" % len(mats))
    expected = {MAT_BODY: COL_BODY, MAT_ROTOR: COL_ROTOR, MAT_NOSE: COL_NOSE}
    worst = 0.0
    for m in mats:
        got = m["pbrMetallicRoughness"]["baseColorFactor"]
        want = expected[m["name"]]
        worst = max(worst, max(abs(got[i] - want[i]) for i in range(3)))
        if abs(got[3] - 1.0) > 1e-6:
            raise RuntimeError("%s alpha != 1" % m["name"])
    if worst > 1e-6:
        raise RuntimeError("baseColorFactor 偏差过大: %g" % worst)
    log("glb tris=%d prims=%d materials=%s"
        % (tris, prims, sorted(m["name"] for m in mats)))
    log("glb baseColorFactor ok 3/3 (max_err<1e-6)")
    return {"tris": tris, "prims": prims, "nodes": names}


# ---------------------------------------------------------------- 10 存 .blend

def save_blend(cam):
    loc, rot = VIEWS["front"]
    cam.location = loc
    cam.rotation_euler = rot
    bpy.data.orphans_purge(do_local_ids=True, do_linked_ids=True, do_recursive=True)
    BLEND_PATH.parent.mkdir(parents=True, exist_ok=True)
    backup = BLEND_PATH.with_suffix(BLEND_PATH.suffix + "1")      # .blend1
    try:
        bpy.context.preferences.filepaths.save_version = 0        # 不留 .blend1 备份
    except Exception as exc:
        log("warn: 无法关闭 .blend1 备份 (%s)" % exc)
    bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_PATH), compress=True,
                                check_existing=False, relative_remap=False)
    if backup.exists():                                          # 兜底：清掉备份
        backup.unlink()
    size = BLEND_PATH.stat().st_size
    if size <= 0:
        raise RuntimeError(".blend 写入失败")
    log("save ok: %s %d B" % (BLEND_PATH, size))
    return size


# ---------------------------------------------------------------- 11 汇总

def summary(body, rotors, arms, glb_info):
    all_objs = [body] + rotors + arms

    total = sum(mesh_tris(o) for o in all_objs)
    if total != EXPECT_TRIS:
        raise RuntimeError("三角总数应为 %d，实际 %d" % (EXPECT_TRIS, total))
    if total >= TRI_LIMIT:
        raise RuntimeError("三角总数 %d 超上限 %d" % (total, TRI_LIMIT))
    log("total_tris=%d (limit=%d ok)" % (total, TRI_LIMIT))

    lo, hi = aabb_of([body])
    size = [hi[i] - lo[i] for i in range(3)]
    log("body_aabb    min=%s max=%s size=%s" % (fmt3(lo), fmt3(hi), fmt3(size)))
    if abs(lo[2]) > 1e-6:
        raise RuntimeError("机身底面不在 z=0: %r" % lo[2])
    for axis, want in ((0, 0.400), (1, 0.400), (2, 0.120)):
        if abs(size[axis] - want) > 1e-6:
            raise RuntimeError("机身尺寸第 %d 轴 %.6f != %.6f"
                               % (axis, size[axis], want))

    vlo, vhi = aabb_of(all_objs)
    vsize = [vhi[i] - vlo[i] for i in range(3)]
    log("vehicle_aabb min=%s max=%s size=%s" % (fmt3(vlo), fmt3(vhi), fmt3(vsize)))
    for axis, want in ((0, 0.640), (1, 0.640), (2, 0.140)):
        if abs(vsize[axis] - want) > 1e-6:
            raise RuntimeError("整机尺寸第 %d 轴 %.6f != %.6f"
                               % (axis, vsize[axis], want))
    return total


def check_paths():
    for p in (BLEND_PATH.parent, GLB_PATH.parent):
        if not p.is_dir():
            raise RuntimeError("父目录不存在: %s" % p)
    PREVIEW_DIR.mkdir(parents=True, exist_ok=True)          # 自建 preview/，不动别人的图
    log("dirs ok: %s" % PREVIEW_DIR)


def main():
    t0 = time.time()
    reset_scene()
    check_paths()
    mat_body, mat_rotor, mat_nose = make_materials()
    body = build_body(mat_body, mat_nose)
    rotors = build_rotors(mat_rotor)
    arms = build_arms(mat_body)
    summary(body, rotors, arms, None)
    cam = setup_rig()
    render_views(cam)
    export_glb([body] + rotors + arms)
    info = verify_glb()
    blend_size = save_blend(cam)
    log("OK  %.1fs  (tris=%d, glb=%d B, blend=%d B)"
        % (time.time() - t0, info["tris"], GLB_PATH.stat().st_size, blend_size))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:                                # 明确失败，不静默
        sys.stdout.flush()
        sys.stderr.write("[make_uav] FAILED: %s: %s\n"
                         % (type(exc).__name__, exc))
        raise
