#!/bin/bash
# 编译 ArkBox 的 Finder Sync Extension（.appex），并 ad-hoc 签名。
# 产物：build/ArkBoxFinderSync.appex
set -euo pipefail

SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="$SRC_DIR/build/ArkBoxFinderSync.appex"
ARCH="$(uname -m)"
# 统一编成 arm64（与当前 macOS 构建目标一致；如需 universal 再扩展）
CLANG_ARCH="arm64"

echo ">> 清理旧产物"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS"

echo ">> clang 编译 ArkBoxFinderSync.m"
clang -dynamiclib -install_name "@executable_path/ArkBoxFinderSync" \
  -framework Foundation \
  -framework FinderSync \
  -framework Cocoa \
  -fmodules -fobjc-arc -O2 \
  -arch "$CLANG_ARCH" \
  "$SRC_DIR/ArkBoxFinderSync.m" \
  -o "$OUT/Contents/MacOS/ArkBoxFinderSync"

echo ">> 拷贝 Info.plist"
cp "$SRC_DIR/Info.plist" "$OUT/Contents/Info.plist"

echo ">> ad-hoc 签名 .appex"
codesign --force --sign - --entitlements "$SRC_DIR/Entitlements.plist" "$OUT"

echo ">> 校验"
codesign --verify --deep --strict "$OUT" && echo "SIGN_OK"
echo ">> 完成: $OUT"
