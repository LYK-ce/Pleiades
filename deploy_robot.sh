#!/bin/bash
# orion-robot 编译 + 部署脚本
#
# 用法:
#   ./deploy_robot.sh                    # 编译并复制到默认工作路径
#   ROBOT_WORK_DIR=/path ./deploy_robot.sh  # 自定义目标路径
#
# 功能:
#   1. 编译 target/release/orion-robot
#   2. 复制到工作路径覆盖原有 binary

set -e

# ─── gcc-12 兼容（nvcc 不支持 GCC 13）────────────
if [ ! -f /tmp/gcc12/gcc ]; then
    mkdir -p /tmp/gcc12
    ln -sf /usr/bin/gcc-12 /tmp/gcc12/gcc
    ln -sf /usr/bin/g++-12 /tmp/gcc12/g++
    ln -sf /usr/bin/gcc-12 /tmp/gcc12/cc
fi
export PATH=/tmp/gcc12:$PATH

# ─── 定位项目根目录 ────────────────────────────
cd "$(dirname "$0")"

# ─── 目标工作路径（可用环境变量覆盖）────────────
TARGET_DIR="${ROBOT_WORK_DIR:-/home/jetson/Workspace/robot}"

echo "=== cargo build --release --bin orion-robot ==="
cargo build --release --bin orion-robot

echo "=== 复制到 $TARGET_DIR ==="
mkdir -p "$TARGET_DIR"
cp target/release/orion-robot "$TARGET_DIR/orion-robot"
chmod +x "$TARGET_DIR/orion-robot"

echo "✅ 部署完成"
ls -la "$TARGET_DIR/orion-robot"
