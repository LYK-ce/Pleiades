#!/usr/bin/env bash
# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-31
#
# task_33 Step 1 / task_42 Step 2 — 一键「建模 → 存 .blend → 导出 .glb」。
#
# 用法（任意 cwd 均可）：
#   bash Tool/blender/models/export_all.sh
#
# 步骤（顺序固定，缺一不可）：
#   ① blender -b --factory-startup -P build_models.py     → 三件 .blend（内含 AABB / 三角数自断言）
#   ② tool/blender/blend2glb.sh ×3                        → GLB（export_format=GLB / export_apply / export_yup）
#   ③ file 校验                                           → glTF binary model, version 2
#   ④ 体积校验                                            → 传感器 / 旧 ugv 单件 ≤ 100 KB
#   ⑤ ugv_m1（task_42）建模 + 导出 + 校验                 → 单件 ≤ 150 KB（按模型分段的体积门禁）
#
# 开关（缺省全都保持 task_33 的原有行为）：
#   ORION_BUILD_LEGACY=0   跳过 ①~④（不重写已入库的 3 个 .blend 与 sensors/*.glb）
#   ORION_BUILD_UGV_M1=0   跳过 ⑤（不构建 ugv_m1）
#   ORION_BLEND_OUT_DIR=…  指定 .blend 输出目录（缺省 = 本脚本所在目录）
#
# 任一环节失败 → 立即退出非 0（set -euo pipefail），不留半成品。
# 注意：本脚本**不跑** godot。导入与自检见 tmp/task33/plan.md §7-C / §7-D。

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
BLENDER_HOME=/vepfs-mlp2/c20250205/240804016/tool/blender
BLENDER="$BLENDER_HOME/blender"
BLEND2GLB="$BLENDER_HOME/blend2glb.sh"
GLB_LIMIT_B=$((100 * 1024))
BUILD_LEGACY="${ORION_BUILD_LEGACY:-1}"
BUILD_UGV_M1="${ORION_BUILD_UGV_M1:-1}"
M1_GLB_LIMIT_B=$((150 * 1024))          # ugv_m1 单独口径（麦克纳姆轮比传感器大一个量级）
BLEND_OUT_DIR="${ORION_BLEND_OUT_DIR:-$HERE}"

SENSORS_DIR="$ROOT/orion-simulation-world/assets/sensors"
CHASSIS_DIR="$ROOT/orion-simulation-world/assets/chassis"
mkdir -p "$SENSORS_DIR" "$CHASSIS_DIR"

echo "[export_all] root=$ROOT"

if [ "$BUILD_LEGACY" = "1" ]; then
	# ① 建模（只出 .blend；--python-exit-code 1 → 断言失败进程 EXIT≠0）
	"$BLENDER" -b --factory-startup --python-exit-code 1 --python "$HERE/build_models.py"

	# ② 导出 glb
	"$BLEND2GLB" "$HERE/lidar_tmini.blend"    "$SENSORS_DIR/lidar_tmini.glb"
	"$BLEND2GLB" "$HERE/camera_module.blend"  "$SENSORS_DIR/camera_module.glb"
	"$BLEND2GLB" "$HERE/ugv_chassis.blend"    "$CHASSIS_DIR/ugv_chassis.glb"

	# ③④ 校验
	for glb in "$SENSORS_DIR/lidar_tmini.glb" "$SENSORS_DIR/camera_module.glb" \
	           "$CHASSIS_DIR/ugv_chassis.glb"; do
		if [ ! -f "$glb" ]; then
			echo "[export_all] 失败：未生成 $glb" >&2
			exit 1
		fi
		size_b=$(stat -c %s "$glb")
		if [ "$size_b" -gt "$GLB_LIMIT_B" ]; then
			echo "[export_all] 失败：$glb 体积 ${size_b}B 超上限 ${GLB_LIMIT_B}B" >&2
			exit 1
		fi
		if ! file "$glb" | grep -q "glTF binary model"; then
			echo "[export_all] 失败：$(file "$glb") 不是 glTF binary" >&2
			exit 1
		fi
		echo "[export_all] OK  ${size_b}B  $glb"
	done
else
	echo "[export_all] 跳过 ①~④（ORION_BUILD_LEGACY=0）：不动旧 ugv / 传感器"
fi

if [ "$BUILD_UGV_M1" = "1" ]; then
	# ⑤ ugv_m1：单建单导（.blend 落 BLEND_OUT_DIR，默认本目录；本轮显式指到 tmp/）
	mkdir -p "$BLEND_OUT_DIR"
	ORION_BLEND_OUT_DIR="$BLEND_OUT_DIR" \
		"$BLENDER" -b --factory-startup --python-exit-code 1 \
		--python "$HERE/build_models.py" -- ugv_m1
	"$BLEND2GLB" "$BLEND_OUT_DIR/ugv_m1.blend" "$CHASSIS_DIR/ugv_m1.glb"

	m1_glb="$CHASSIS_DIR/ugv_m1.glb"
	if [ ! -f "$m1_glb" ]; then
		echo "[export_all] 失败：未生成 $m1_glb" >&2
		exit 1
	fi
	m1_b=$(stat -c %s "$m1_glb")
	if [ "$m1_b" -gt "$M1_GLB_LIMIT_B" ]; then
		echo "[export_all] 失败：$m1_glb 体积 ${m1_b}B 超上限 ${M1_GLB_LIMIT_B}B" >&2
		exit 1
	fi
	if ! file "$m1_glb" | grep -q "glTF binary model"; then
		echo "[export_all] 失败：$(file "$m1_glb") 不是 glTF binary" >&2
		exit 1
	fi
	echo "[export_all] OK  ${m1_b}B  $m1_glb"
else
	echo "[export_all] 跳过 ⑤（ORION_BUILD_UGV_M1=0）：不构建 ugv_m1"
fi

echo "[export_all] done"
