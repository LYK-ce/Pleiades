#!/bin/bash
# Pleiades 编译脚本
# 自动处理 nvcc 与 GCC 13 不兼容的问题

set -e

# 确保 gcc-12 软链接存在
if [ ! -f /tmp/gcc12/gcc ]; then
    mkdir -p /tmp/gcc12
    ln -sf /usr/bin/gcc-12 /tmp/gcc12/gcc
    ln -sf /usr/bin/g++-12 /tmp/gcc12/g++
    ln -sf /usr/bin/gcc-12 /tmp/gcc12/cc
fi

export PATH=/tmp/gcc12:$PATH

cd "$(dirname "$0")"

case "${1:-build}" in
    build)
        echo "=== cargo build --release ==="
        cargo build --release
        ;;
    check)
        echo "=== cargo check ==="
        cargo check
        ;;
    test)
        shift
        echo "=== cargo test $@ ==="
        cargo test "$@"
        ;;
    deploy)
        echo "=== cargo build --release + deploy ==="
        cargo build --release
        cp target/release/Pleiades /vepfs-mlp2/c20250205/240804016/Test_Environment/1/
        cp target/release/Pleiades /vepfs-mlp2/c20250205/240804016/Test_Environment/2/
        cp target/release/Pleiades /vepfs-mlp2/c20250205/240804016/Test_Environment/3/
        cp -r programs/* /vepfs-mlp2/c20250205/240804016/Test_Environment/1/programs/
        cp -r programs/* /vepfs-mlp2/c20250205/240804016/Test_Environment/2/programs/
        cp -r programs/* /vepfs-mlp2/c20250205/240804016/Test_Environment/3/programs/
        echo "✅ 部署完成"
        ;;
    clean)
        echo "=== cargo clean ==="
        cargo clean
        ;;
    *)
        echo "用法: ./build.sh [build|check|test|deploy|clean]"
        echo "  build   - release 编译"
        echo "  check   - 快速语法检查"
        echo "  test    - 运行测试（可传参: ./build.sh test -p Pleiades --lib storage）"
        echo "  deploy  - 编译 + 部署到测试环境"
        echo "  clean   - 清理编译缓存"
        exit 1
        ;;
esac
