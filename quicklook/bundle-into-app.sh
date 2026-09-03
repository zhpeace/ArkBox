#!/usr/bin/env bash
#
# bundle-into-app.sh —— 把 ArkBoxQL Quick Look 插件 + Finder Sync 扩展打进 Tauri 打包好的 .app
#
# 用法：
#   bash quicklook/bundle-into-app.sh                 # 自动查找 src-tauri/target/release/bundle/macos/*.app
#   bash quicklook/bundle-into-app.sh --app /path/To/ArkBox.app
#   bash quicklook/bundle-into-app.sh --no-sign      # 跳过 codesign（CI/沙箱测试用）
#   bash quicklook/bundle-into-app.sh --identity "Developer ID Application: ..."   # 分发时整包重签
#
# 说明：
#   - Quick Look 插件必须位于 .app/Contents/Library/QuickLook/ 才生效
#   - 插件内两个 Mach-O（生成器 dylib + bz-qlhelper）需代码签名，否则 QL 加载器拒绝
#   - 若传入 --identity，会对整包 .app 重新签名（分发 DMG 必备）
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
QLGEN="$SCRIPT_DIR/ArkBoxQL/ArkBoxQL.qlgenerator"
APPEX="$SCRIPT_DIR/../findersync/ArkBoxFSE.appex"

APP_PATH=""
NO_SIGN=0
SIGN_ID="-"          # ad-hoc 默认；用 --identity 覆盖
APP_RESIGN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)      APP_PATH="$2"; shift 2 ;;
    --no-sign)  NO_SIGN=1; shift ;;
    --identity) SIGN_ID="$2"; APP_RESIGN=1; shift 2 ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

# 自动查找 .app
if [[ -z "$APP_PATH" ]]; then
  CANDIDATE="$(find "$SCRIPT_DIR/../src-tauri/target/release/bundle/macos" -maxdepth 1 -name '*.app' 2>/dev/null | head -1)"
  if [[ -z "$CANDIDATE" ]]; then
    echo "未找到 .app，请先 'npm run tauri build'，或传 --app <path>" >&2
    exit 1
  fi
  APP_PATH="$CANDIDATE"
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "找不到 app: $APP_PATH" >&2
  exit 1
fi
if [[ ! -d "$QLGEN" ]]; then
  echo "找不到插件: $QLGEN （请先 bash quicklook/ArkBoxQL/build.sh）" >&2
  exit 1
fi

DEST="$APP_PATH/Contents/Library/QuickLook"
echo "==> 目标 app: $APP_PATH"
echo "==> 拷贝插件到 $DEST"
mkdir -p "$DEST"
rm -rf "$DEST/ArkBoxQL.qlgenerator"
cp -R "$QLGEN" "$DEST/ArkBoxQL.qlgenerator"
chmod +x "$DEST/ArkBoxQL.qlgenerator/Contents/MacOS/ArkBoxQL"
chmod +x "$DEST/ArkBoxQL.qlgenerator/Contents/MacOS/bz-qlhelper"

# 代码签名（Quick Look 插件内二进制必须签）
if [[ "$NO_SIGN" -eq 0 ]]; then
  echo "==> 代码签名插件"
  codesign --force --sign "$SIGN_ID" "$DEST/ArkBoxQL.qlgenerator/Contents/MacOS/bz-qlhelper" 2>&1 | sed 's/^/   /' || true
  codesign --force --sign "$SIGN_ID" "$DEST/ArkBoxQL.qlgenerator/Contents/MacOS/ArkBoxQL" 2>&1 | sed 's/^/   /' || true
  codesign --force --sign "$SIGN_ID" "$DEST/ArkBoxQL.qlgenerator" 2>&1 | sed 's/^/   /' || true
fi

# 拷贝 Finder Sync Extension（Finder 右键菜单直接项）
if [[ -d "$APPEX" ]]; then
  PLUGINS="$APP_PATH/Contents/PlugIns"
  mkdir -p "$PLUGINS"
  rm -rf "$PLUGINS/ArkBoxFSE.appex"
  cp -R "$APPEX" "$PLUGINS/ArkBoxFSE.appex"
  echo "==> 已拷贝 Finder Sync Extension -> $PLUGINS"
  if [[ "$NO_SIGN" -eq 0 ]]; then
    codesign --force --sign "$SIGN_ID" "$PLUGINS/ArkBoxFSE.appex" 2>&1 | sed 's/^/   /' || true
  fi
else
  echo "（未找到 $APPEX，跳过 Finder Sync；先 bash findersync/build.sh）"
fi

# 分发时整包重签
if [[ "$APP_RESIGN" -eq 1 ]]; then
  echo "==> 整包重新签名（identity=$SIGN_ID）"
  codesign --force --deep --sign "$SIGN_ID" "$APP_PATH" 2>&1 | sed 's/^/   /'
fi

echo "完成。安装/重启 Finder 后，Finder 选中压缩包按空格即可预览。"
echo "       调试: qlmanage -r ; qlmanage -p <某个.zip>"
