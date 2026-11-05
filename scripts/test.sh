#!/bin/zsh
# 跑单元测试。只装了 Command Line Tools 时，要手动告诉编译器 Swift Testing 在哪；
# 另外 CLT 里 _Testing_Foundation 的模块链接是坏的，所以关掉 cross-import overlay。
set -euo pipefail
cd "${0:A:h}/.."

FW=/Library/Developer/CommandLineTools/Library/Developer/Frameworks
LIB=/Library/Developer/CommandLineTools/Library/Developer/usr/lib

if [[ -d $FW/Testing.framework ]] && ! xcode-select -p | grep -q Xcode.app; then
  swift test \
    -Xswiftc -F$FW -Xswiftc -Xfrontend -Xswiftc -disable-cross-import-overlays \
    -Xlinker -F$FW -Xlinker -rpath -Xlinker $FW -Xlinker -rpath -Xlinker $LIB "$@"
else
  swift test "$@"
fi
