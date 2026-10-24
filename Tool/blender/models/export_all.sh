#!/usr/bin/env bash
# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-23
#
# task_33 Step 1 — 一键「建模 → 存 .blend → 导出 .glb」。
#
# 用法（任意 cwd 均可）：
#   bash Tool/blender/models/export_all.sh
#
# 步骤（顺序固定，缺一不可）：
#   ① blender -b --factory-startup -P build_models.py     → 三件 .blend（内含 AABB / 三角数自断言）
#   ② tool/blender/blend2glb.sh ×3                        → GLB（export_format=GLB / export_apply / export_yup）
#   ③ file 校验                                           → glTF binary model, version 2
#   ④ 体积校验                                            → 单件 ≤ 100 KB
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

SENSORS_DIR="$ROOT/orion-simulation-world/assets/sensors"
CHASSIS_DIR="$ROOT/orion-simulation-world/assets/chassis"
mkdir -p "$SENSORS_DIR" "$CHASSIS_DIR"

echo "[export_all] root=$ROOT"

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

echo "[export_all] done"
