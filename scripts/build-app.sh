#!/bin/zsh
# 编译 release 版并打包成 build/ClipMD.app（只需要 Command Line Tools，不需要 Xcode）
set -euo pipefail
cd "${0:A:h}/.."

swift build -c release

APP=build/ClipMD.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp .build/release/ClipMD "$APP/Contents/MacOS/ClipMD"
cp Resources/Info.plist "$APP/Contents/Info.plist"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
# 去掉调试符号，减小体积
strip -x "$APP/Contents/MacOS/ClipMD"
# 本机自用的临时签名
codesign --force --sign - "$APP"

echo "已生成 $APP（$(du -sh "$APP" | cut -f1)）"
