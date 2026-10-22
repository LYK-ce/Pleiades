#!/usr/bin/env python3
"""UGV 低模生成脚本（Blender 4.5+ 无头模式，幂等）。

用法（工程根）:
    /vepfs-mlp2/c20250205/240804016/tool/blender/blender -b \
        --python-exit-code 1 --python Tool/models/make_ugv.py

产物:
    Tool/models/ugv_x3plus_lowpoly.blend             源文件（含相机/灯光）
    Tool/models/preview/ugv_{front,side,top}.png      三视图 480x360
    orion-simulation-world/assets/ugv_x3plus_lowpoly.glb   交付物（只含 5 个车辆对象）

尺寸口径：1 BU = 1 m；Blender +Z 上；车头朝 +X。
    DiffDrive.gd:41  _wheel_base  = 0.30  → 车体 X 长
    DiffDrive.gd:42  _track_width = 0.24  → 车体 Y 宽（Godot 局部 Z）
    DiffDrive.gd:43  _height      = 0.16  → 车体 Z 高
    DiffDrive.gd:33  WHEEL_RADIUS = 0.06  → 轮半径（轮底 z=0，贴地）
    DiffDrive.gd:159 轮心 x = ±wheel_base/2 = ±0.15
    DiffDrive.gd:160 轮心 z = ±track_width/2 → Blender y = ∓0.12
    DiffDrive.gd:161 轮心 y = WHEEL_RADIUS = 0.06
"""

import json
import math
import os
import struct
import sys
import time
from pathlib import Path

import bmesh
import bpy
from mathutils import Vector

# ---------------------------------------------------------------- 常量 / 路径

PROJECT_ROOT = Path(__file__).resolve().parents[2]          # Tool/models/make_ugv.py → Orion
BLEND_PATH = PROJECT_ROOT / "Tool/models/ugv_x3plus_lowpoly.blend"
GLB_PATH = PROJECT_ROOT / "orion-simulation-world/assets/ugv_x3plus_lowpoly.glb"
PREVIEW_DIR = PROJECT_ROOT / "Tool/models/preview"
PREVIEWS = {
    "front": PREVIEW_DIR / "ugv_front.png",
    "side": PREVIEW_DIR / "ugv_side.png",
    "top": PREVIEW_DIR / "ugv_top.png",
}
TMP_DIR = Path("/tmp")                                       # 临时文件只落 /tmp

# 车体尺寸（米）
HX = 0.150          # 0.30 / 2  (DiffDrive.gd:41)
HY = 0.120          # 0.24 / 2  (DiffDrive.gd:42)
GZ = 0.160          #           (DiffDrive.gd:43)
SX = 0.100          # 鼻锥收窄起点 x
NY = 0.050          # 车头正面半宽（鼻锥）
WHEEL_R = 0.060     # DiffDrive.gd:33
WHEEL_W = 0.040     # step.md:24
WHEEL_X = 0.150     # DiffDrive.gd:159
WHEEL_Y = 0.120     # DiffDrive.gd:160（Blender y）
WHEEL_Z = 0.060     # DiffDrive.gd:161

# 颜色（step.md:26）—— 直接作为 Principled 线性基色，不做 sRGB→linear 转换
COL_BODY = (0.85, 0.55, 0.15)
COL_WHEEL = (0.12, 0.12, 0.14)
COL_NOSE = (0.90, 0.20, 0.15)
MAT_BODY = "UGV_BodyOrange"
MAT_WHEEL = "UGV_WheelDarkGray"
MAT_NOSE = "UGV_NoseRed"

TRI_LIMIT = 3000
SAMPLES = 32                                                # step.md:17 上限 64
RES_X, RES_Y = 480, 360
ORTHO_SCALE = 0.46

VEHICLE_OBJECTS = ["Body", "WheelFL", "WheelFR", "WheelRL", "WheelRR"]

# §3.1 顶点表（世界坐标 = 网格坐标，对象都在原点）
BODY_VERTS = [
    (-HX, -HY, 0.00), (-HX, HY, 0.00), (-HX, HY, GZ), (-HX, -HY, GZ),   # 0-3 车尾
    (SX, -HY, 0.00), (SX, HY, 0.00), (SX, HY, GZ), (SX, -HY, GZ),       # 4-7 收窄起点
    (HX, -NY, 0.00), (HX, NY, 0.00), (HX, NY, GZ), (HX, -NY, GZ),       # 8-11 车头正面
]
BODY_FACES = [
    (0, 1, 2, 3),      # 0 车尾
    (0, 4, 5, 1),      # 1 底面·后段
    (4, 8, 9, 5),      # 2 底面·收窄段
    (3, 2, 6, 7),      # 3 顶面·后段
    (7, 6, 10, 11),    # 4 顶面·收窄段（红）
    (0, 3, 7, 4),      # 5 左侧·后段
    (4, 7, 11, 8),     # 6 左侧·收窄段（红）
    (1, 5, 6, 2),      # 7 右侧·后段
    (5, 9, 10, 6),     # 8 右侧·收窄段（红）
    (8, 11, 10, 9),    # 9 车头正面（红）
]

# §3.2 轮子摆位：Blender +Y = Godot 左侧（export_yup: (x,y,z) → (x,z,-y)）
WHEELS = [
    ("WheelFL", WHEEL_X, WHEEL_Y, WHEEL_Z),      # Godot 左前 (x+0.15, z-0.12)
    ("WheelFR", WHEEL_X, -WHEEL_Y, WHEEL_Z),     # Godot 右前 (x+0.15, z+0.12)
    ("WheelRL", -WHEEL_X, WHEEL_Y, WHEEL_Z),     # Godot 左后
    ("WheelRR", -WHEEL_X, -WHEEL_Y, WHEEL_Z),    # Godot 右后
]

SCENE = None


# ---------------------------------------------------------------- 工具

def log(msg):
    print("[make_ugv] " + msg, flush=True)


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
        make_material(MAT_WHEEL, COL_WHEEL, 0.85),
        make_material(MAT_NOSE, COL_NOSE, 0.45),
    )


# ---------------------------------------------------------------- 3 车体

def build_body(mat_body, mat_nose):
    me = bpy.data.meshes.new("Body")
    ob = bpy.data.objects.new("Body", me)
    bpy.context.collection.objects.link(ob)

    bm = bmesh.new()
    verts = [bm.verts.new(c) for c in BODY_VERTS]
    bm.verts.ensure_lookup_table()
    for idx in BODY_FACES:
        bm.faces.new([verts[i] for i in idx])
    bm.faces.ensure_lookup_table()
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])    # 统一外法线
    bm.normal_update()

    red = 0
    for f in bm.faces:
        c = f.calc_center_median()
        is_red = (f.normal.x > 0.35) or (f.normal.z > 0.9 and c.x > SX + 1e-9)
        f.material_index = 1 if is_red else 0
        red += int(is_red)

    bm.to_mesh(me)
    bm.free()
    me.validate(verbose=False)
    me.materials.append(mat_body)                            # slot 0
    me.materials.append(mat_nose)                            # slot 1
    me.update()

    if red != 4:
        raise RuntimeError("鼻锥红面数应为 4，实际 %d" % red)
    log("Body      tris=%d  verts=%d  red_faces=%d"
        % (mesh_tris(ob), len(me.vertices), red))
    return ob


# ---------------------------------------------------------------- 4 轮子

def build_wheels(mat_wheel):
    objs = []
    for name, x, y, z in WHEELS:
        bpy.ops.mesh.primitive_cylinder_add(
            vertices=16, radius=WHEEL_R, depth=WHEEL_W, end_fill_type="NGON",
            location=(x, y, z), rotation=(math.pi / 2.0, 0.0, 0.0),   # 轴 +Z → +Y
        )
        ob = bpy.context.active_object
        ob.name = name
        ob.data.name = name
        select_only([ob])
        bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
        bpy.ops.object.shade_flat()
        ob.data.materials.append(mat_wheel)
        objs.append(ob)
        log("%-9s tris=%d  loc=%s" % (name, mesh_tris(ob), fmt3(ob.location)))
        if (abs(ob.location.x - x) > 1e-6 or abs(ob.location.y - y) > 1e-6
                or abs(ob.location.z - z) > 1e-6):
            raise RuntimeError("%s 位置被改动: %s" % (name, fmt3(ob.location)))
    return objs


# ---------------------------------------------------------------- 5 相机/灯光/渲染设置

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
        SCENE.view_settings.view_transform = "Standard"   # 默认 AgX 会洗淡橙/红
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


# ---------------------------------------------------------------- 6 三视图

VIEWS = {
    #  name: (location, rotation_euler)
    "front": ((1.5, 0.0, 0.08), (math.pi / 2.0, 0.0, math.pi / 2.0)),   # 看 -X，up=+Z
    "side": ((0.0, -1.5, 0.08), (math.pi / 2.0, 0.0, 0.0)),             # 看 +Y，车头朝右
    "top": ((0.0, 0.0, 1.5), (0.0, 0.0, 0.0)),                          # 看 -Z，+X 朝右
}


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
        size = out.stat().st_size
        if size <= 1024:
            raise RuntimeError("预览图过小: %s %d B" % (out, size))
        log("render ok: %s %d B" % (out, size))


# ---------------------------------------------------------------- 7 导出 GLB

def export_glb(objs):
    select_only(objs, active=bpy.data.objects["Body"])
    bpy.ops.export_scene.gltf(
        filepath=str(GLB_PATH),
        export_format="GLB",
        export_yup=True,
        export_apply=True,          # 应用修改器
        use_selection=True,         # 只导 5 个车辆对象
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


# ---------------------------------------------------------------- 8 自检 GLB

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
    if len(names) != 5 or set(names) != set(VEHICLE_OBJECTS):
        raise RuntimeError("节点清单不符: %r" % names)
    if doc.get("cameras"):
        raise RuntimeError("GLB 里带了相机: %r" % doc["cameras"])
    ext = doc.get("extensionsUsed", [])
    if "KHR_lights_punctual" in ext:
        raise RuntimeError("GLB 里带了灯光: %r" % ext)
    log("glb nodes=5 %s cameras=0 lights=0" % names)

    tris = 0
    prims = 0
    for mesh in doc.get("meshes", []):
        for prim in mesh["primitives"]:
            tris += doc["accessors"][prim["indices"]]["count"] // 3
            prims += 1
    if tris != 260 or tris >= TRI_LIMIT:
        raise RuntimeError("GLB 三角数异常: %d" % tris)

    mats = doc.get("materials", [])
    if len(mats) != 3:
        raise RuntimeError("材质数应为 3，实际 %d" % len(mats))
    expected = {MAT_BODY: COL_BODY, MAT_WHEEL: COL_WHEEL, MAT_NOSE: COL_NOSE}
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


# ---------------------------------------------------------------- 9 存 .blend

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
    if backup.exists():                                          # 兜底：清掉备份，只留 4 类产物
        backup.unlink()
    size = BLEND_PATH.stat().st_size
    if size <= 0:
        raise RuntimeError(".blend 写入失败")
    log("save ok: %s %d B" % (BLEND_PATH, size))
    return size


# ---------------------------------------------------------------- 10 汇总

def summary(body, wheels, glb_info):
    all_objs = [body] + wheels

    total = sum(mesh_tris(o) for o in all_objs)
    if total != 260:
        raise RuntimeError("三角总数应为 260，实际 %d" % total)
    if total >= TRI_LIMIT:
        raise RuntimeError("三角总数 %d 超上限 %d" % (total, TRI_LIMIT))
    log("total_tris=%d (limit=%d ok)" % (total, TRI_LIMIT))

    lo, hi = aabb_of([body])
    size = [hi[i] - lo[i] for i in range(3)]
    log("body_aabb    min=%s max=%s size=%s" % (fmt3(lo), fmt3(hi), fmt3(size)))
    if abs(lo[2]) > 1e-6:
        raise RuntimeError("车体底面不在 z=0: %r" % lo[2])
    for axis, want in ((0, 0.300), (1, 0.240), (2, 0.160)):
        if abs(size[axis] - want) > 1e-6:
            raise RuntimeError("车体尺寸第 %d 轴 %.6f != %.6f"
                               % (axis, size[axis], want))
    if size[0] > 0.300 + 1e-6:
        raise RuntimeError("车体 X 超 0.30（含鼻标）: %.6f" % size[0])

    vlo, vhi = aabb_of(all_objs)
    vsize = [vhi[i] - vlo[i] for i in range(3)]
    log("vehicle_aabb min=%s max=%s size=%s" % (fmt3(vlo), fmt3(vhi), fmt3(vsize)))
    if abs(vsize[1] - 0.280) > 1e-6:
        raise RuntimeError("整机宽应为 0.28，实际 %.6f" % vsize[1])
    if abs(vsize[2] - 0.160) > 1e-6:
        raise RuntimeError("整机高应为 0.16，实际 %.6f" % vsize[2])

    return total


def check_paths():
    for p in (BLEND_PATH.parent, GLB_PATH.parent):
        if not p.is_dir():
            raise RuntimeError("父目录不存在: %s" % p)
    PREVIEW_DIR.mkdir(parents=True, exist_ok=True)          # 自建 preview/，不动别的文件
    TMP_DIR.mkdir(parents=True, exist_ok=True)
    log("dirs ok: %s" % PREVIEW_DIR)


def main():
    t0 = time.time()
    reset_scene()
    check_paths()
    mat_body, mat_wheel, mat_nose = make_materials()
    body = build_body(mat_body, mat_nose)
    wheels = build_wheels(mat_wheel)
    total = summary(body, wheels, None)
    cam = setup_rig()
    render_views(cam)
    export_glb([body] + wheels)
    info = verify_glb()
    blend_size = save_blend(cam)
    log("OK  %.1fs  (tris=%d, glb=%d B, blend=%d B)"
        % (time.time() - t0, info["tris"], GLB_PATH.stat().st_size, blend_size))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:                                # 明确失败，不静默
        sys.stdout.flush()
        sys.stderr.write("[make_ugv] FAILED: %s: %s\n"
                         % (type(exc).__name__, exc))
        raise
