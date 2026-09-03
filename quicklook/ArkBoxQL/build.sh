#!/usr/bin/env bash
#
# build.sh —— 构建 ArkBoxQL.qlgenerator
#
# 步骤：
#   1. 用 cargo 编译 Rust 辅助二进制 bz-qlhelper（release）
#   2. 用 clang 编译 C 端 Quick Look 生成器
#   3. 组装 .qlgenerator bundle（生成器 + bz-qlhelper 同放 Contents/MacOS）
#
# 仅编译 Apple Silicon (arm64)。如需 Intel 机器，给 clang 追加 -arch x86_64 并
# 用 cargo build --target x86_64-apple-darwin 重新编译 helper，再用 lipo 合并。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"
SRC_TAURI="$REPO/src-tauri"
BUILD="$ROOT/build"
OUT="$ROOT/ArkBoxQL.qlgenerator"

echo "==> 编译 bz-qlhelper (Rust)"
pushd "$SRC_TAURI" >/dev/null
cargo build --release --bin bz-qlhelper
popd >/dev/null
HELPER="$SRC_TAURI/target/release/bz-qlhelper"

echo "==> 编译 Quick Look 生成器 (C)"
mkdir -p "$BUILD"
# -install_name 必须显式指定：clang 默认把 -o 的绝对路径写进 LC_ID_DYLIB，
# 会让产物绑死到编译机的构建目录（拷到别的机器/清理 build 后即失效）。
clang -dynamiclib -fPIC -o "$BUILD/ArkBoxQL" "$ROOT/generate.c" \
    -install_name "@rpath/ArkBoxQL" \
    -framework QuickLook -framework CoreFoundation -framework CoreGraphics -framework ApplicationServices \
    -mmacosx-version-min=10.15 -arch arm64

echo "==> 组装 $OUT"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS"
mkdir -p "$OUT/Contents/Resources"
cp "$BUILD/ArkBoxQL" "$OUT/Contents/MacOS/ArkBoxQL"
cp "$HELPER"          "$OUT/Contents/MacOS/bz-qlhelper"
cp "$ROOT/Info.plist" "$OUT/Contents/Info.plist"

echo "==> 完成：$OUT"
echo "    安装：cp -r \"$OUT\" ~/Library/QuickLook/ && qlmanage -r"
