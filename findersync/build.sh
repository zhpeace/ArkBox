#!/usr/bin/env bash
# 编译 Finder Sync Extension (.appex)
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

APP="ArkBoxFSE.appex"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

echo "==> 编译 FinderSync extension"
clang -bundle -framework Foundation -framework AppKit -framework FinderSync \
  -fobjc-arc -mmacosx-version-min=10.15 -arch arm64 \
  -o "$APP/Contents/MacOS/ArkBoxFSE" findersync.m

cp Info.plist "$APP/Contents/"

echo "==> 代码签名 appex (ad-hoc)"
codesign --force --sign - --entitlements ArkBoxFSE.entitlements "$APP"

echo "完成: $DIR/$APP"
echo "   调试（需 GUI 会话）: pluginkit -a $DIR/$APP ; 或打开主 app 一次后 Finder 右键即出现"
