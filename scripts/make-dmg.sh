#!/usr/bin/env bash
#
# make-dmg.sh —— 把已嵌入 Quick Look / Finder Sync 扩展的 .app 重新打成 DMG
#
# 背景：tauri build 的 DMG 步骤发生在 bundle-into-app.sh 之前，
#       所以官方产出的 .dmg 里并不含那两个原生扩展。本脚本在扩展
#       嵌入之后重打一次，保证 DMG 与 .app 内容一致。
#
# 用法：
#   bash scripts/make-dmg.sh                       # 自动找 src-tauri/target/release/bundle/macos/*.app
#   bash scripts/make-dmg.sh --app /path/ArkBox.app
#   bash scripts/make-dmg.sh --no-sign             # 跳过外层 codesign
#
# 产物：src-tauri/target/release/bundle/dmg/ArkBox_<version>_<arch>.dmg
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
MACOS_DIR="$BUNDLE/macos"
DMG_DIR="$BUNDLE/dmg"

APP_PATH=""
NO_SIGN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)     APP_PATH="$2"; shift 2 ;;
    --no-sign) NO_SIGN=1; shift ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$APP_PATH" ]]; then
  APP_PATH="$(find "$MACOS_DIR" -maxdepth 1 -name '*.app' 2>/dev/null | head -1)"
fi
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "找不到 .app，请先运行 npm run tauri:build，或传 --app <path>" >&2
  exit 1
fi

APP_NAME="$(basename "$APP_PATH" .app)"
VERSION="$(defaults read "$APP_PATH/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo 0.0.0)"
# 与 tauri 官方产物命名保持一致（arm64 -> aarch64），避免同目录出现两份同内容 dmg
case "$(uname -m)" in
  arm64) ARCH="aarch64" ;;
  x86_64) ARCH="x64" ;;
  *) ARCH="$(uname -m)" ;;
esac

# 外层重签：嵌入扩展后原签名已失效。
# 注意不用 --deep —— 嵌套 bundle（.qlgenerator / .appex）由 bundle-into-app.sh
# 各自签名，外层签名本就不覆盖其内容，--deep 反而会破坏它们的 entitlements。
if [[ "$NO_SIGN" -eq 0 ]]; then
  echo "==> 外层重签 $(basename "$APP_PATH")"
  codesign --force --sign - --preserve-metadata=entitlements "$APP_PATH" 2>&1 | sed 's/^/    /' || true
fi

# 组装 staging：ArkBox.app + /Applications 软链（用户拖拽安装）
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/arkbox-dmg.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP_PATH" "$STAGING/"
ln -s /Applications "$STAGING/Applications"

OUT="$DMG_DIR/${APP_NAME}_${VERSION}_${ARCH}.dmg"
mkdir -p "$DMG_DIR"
rm -f "$OUT"

echo "==> 生成 DMG: $OUT"
hdiutil create \
  -srcfolder "$STAGING" \
  -volname "$APP_NAME" \
  -fs HFS+ -fsargs "-c c=64,a=16,e=16" \
  -format UDZO -imagekey zlib-level=9 \
  -ov "$OUT" >/dev/null

echo "==> 完成"
ls -lh "$OUT" | awk '{print "    " $5 "  " $9}'
echo "    挂载校验: hdiutil attach \"$OUT\""
