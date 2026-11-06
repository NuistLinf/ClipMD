#!/bin/zsh
# 从 macOS 版的图标（design/AppIcon-1024.png）生成 Windows 的 app/res/ClipMD.ico。
# macOS 图标四周留了约 10% 的透明边，Windows 图标习惯撑满，所以先裁掉一圈再缩放。
set -euo pipefail
cd "${0:A:h}/.."
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
sips -c 880 880 ../design/AppIcon-1024.png --out "$WORK/crop.png" >/dev/null
for size in 16 20 24 32 40 48 64 256; do
  sips -z $size $size "$WORK/crop.png" --out "$WORK/$size.png" >/dev/null
done
python3 - "$WORK" app/res/ClipMD.ico <<'PY'
import struct, sys, os
work, out = sys.argv[1], sys.argv[2]
sizes = [16, 20, 24, 32, 40, 48, 64, 256]
images = [open(os.path.join(work, f"{s}.png"), "rb").read() for s in sizes]
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries, data = b"", b""
for size, png in zip(sizes, images):
    dim = 0 if size >= 256 else size
    entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset + len(data))
    data += png
open(out, "wb").write(header + entries + data)
PY
echo "已生成 app/res/ClipMD.ico（$(du -h app/res/ClipMD.ico | cut -f1)）"
