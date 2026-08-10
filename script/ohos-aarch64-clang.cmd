@echo off
setlocal

if "%OHOS_NDK_HOME%"=="" (
  echo OHOS_NDK_HOME must point to the OpenHarmony SDK root. 1>&2
  exit /b 2
)

"%OHOS_NDK_HOME%\native\llvm\bin\clang.exe" --target=aarch64-linux-ohos --sysroot="%OHOS_NDK_HOME%\native\sysroot" -D__MUSL__ %*
