#!/bin/sh
exec clang++ \
  -stdlib=libc++ \
  -nostdinc++ \
  -isystem /ohos/native/llvm/include/libcxx-ohos/include/c++/v1 \
  --target=x86_64-linux-ohos \
  --sysroot=/ohos/native/sysroot \
  -resource-dir=/ohos/native/llvm/lib/clang/15.0.4 \
  -L/ohos/native/llvm/lib/x86_64-linux-ohos \
  -L/tmp/empty \
  -fuse-ld=lld \
  -fno-stack-protector \
  -Wno-unused-command-line-argument \
  "$@" \
  -lc++_shared
