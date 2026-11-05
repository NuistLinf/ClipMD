#!/bin/zsh
# 编译并安装到「应用程序」文件夹，然后打开。记录、设置都在 ~/Library 里，重装不会丢
set -euo pipefail
cd "${0:A:h}/.."

./scripts/build-app.sh

LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
pkill -x ClipMD || true
sleep 1
rm -rf /Applications/ClipMD.app
ditto build/ClipMD.app /Applications/ClipMD.app
# 让启动台、Spotlight、clipmd:// 链接都认「应用程序」里的这一份，不认编译目录里的
"$LSREGISTER" -u build/ClipMD.app 2>/dev/null || true
"$LSREGISTER" -f /Applications/ClipMD.app
open /Applications/ClipMD.app

echo "已安装到 /Applications/ClipMD.app"
