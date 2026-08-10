#!/bin/sh
exec clang \
  --target=aarch64-linux-ohos \
  --sysroot=/ohos/native/sysroot \
  -resource-dir=/ohos/native/llvm/lib/clang/15.0.4 \
  -L/ohos/native/llvm/lib/aarch64-linux-ohos \
  -L/tmp/empty \
  -fuse-ld=lld \
  -fno-stack-protector \
  -Wno-unused-command-line-argument \
  "$@"
