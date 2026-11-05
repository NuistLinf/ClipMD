#!/bin/zsh
# 重新生成应用图标 Resources/AppIcon.icns（用的是 scripts/icon/IconRenderer.swift 里的 capture 方案）
set -euo pipefail
cd "${0:A:h}/.."

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

swiftc -parse-as-library -O scripts/icon/IconRenderer.swift -o "$WORK/icon-renderer"
"$WORK/icon-renderer" "$WORK" capture >/dev/null

ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z $size $size "$WORK/capture.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$WORK/capture.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o Resources/AppIcon.icns
cp "$WORK/capture.png" design/AppIcon-1024.png

echo "已生成 Resources/AppIcon.icns（$(du -h Resources/AppIcon.icns | cut -f1)）"
