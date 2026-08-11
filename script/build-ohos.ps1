param(
    [ValidateSet('rust', 'hap', 'install', 'run', 'all')]
    [string]$Step = 'all',
    [string]$SdkRoot = 'C:\Program Files\Huawei\DevEco Studio\sdk\default\openharmony',
    [switch]$ReuseExistingNodeBundle,
    [ValidateSet('dev', 'release-fast', 'release')]
    [string]$RustProfile = 'release-fast',
    [ValidateSet('debug', 'release')]
    [string]$SigningProfile = 'debug',
    [string]$DebugDeviceUdid
)

$ErrorActionPreference = 'Stop'
$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$TargetTriple = 'x86_64-unknown-linux-ohos'
$HapRoot = Join-Path $RepositoryRoot 'crates\zed_ohos\hap'
$RustOutputDirectory = if ($RustProfile -eq 'dev') { 'debug' } else { $RustProfile }
$NativeLibrary = Join-Path $RepositoryRoot "target\$TargetTriple\$RustOutputDirectory\libzed_ohos.so"
$PackagedLibraryDirectory = Join-Path $HapRoot 'entry\libs\x86_64'
$Linker = Join-Path $PSScriptRoot 'ohos-x86_64-clang.cmd'
$Archiver = Join-Path $SdkRoot 'native\llvm\bin\llvm-ar.exe'
$Strip = Join-Path $SdkRoot 'native\llvm\bin\llvm-strip.exe'
$CxxCompiler = Join-Path $SdkRoot 'native\llvm\bin\clang++.exe'
$NativeBuildTools = Join-Path $SdkRoot 'native\build-tools\cmake\bin'
$Cmake = Join-Path $NativeBuildTools 'cmake.exe'
$Ninja = Join-Path $NativeBuildTools 'ninja.exe'
$DevEcoRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $SdkRoot))
$Hvigor = Join-Path $DevEcoRoot 'tools\hvigor\bin\hvigorw.bat'
$Hdc = Join-Path $SdkRoot 'toolchains\hdc.exe'
$SigningTool = Join-Path $SdkRoot 'toolchains\lib\hap-sign-tool.jar'
$PackingTool = Join-Path $SdkRoot 'toolchains\lib\app_packing_tool.jar'
$SigningStore = Join-Path $SdkRoot 'toolchains\lib\OpenHarmony.p12'
$ProfileCertificateName = if ($SigningProfile -eq 'debug') {
    'OpenHarmonyProfileDebug.pem'
} else {
    'OpenHarmonyProfileRelease.pem'
}
$ProfileTemplateName = if ($SigningProfile -eq 'debug') {
    'UnsgnedDebugProfileTemplate.json'
} else {
    'UnsgnedReleasedProfileTemplate.json'
}
$ProfileKeyAlias = "openharmony application profile $SigningProfile"
$ProfileCertificate = Join-Path $SdkRoot "toolchains\lib\$ProfileCertificateName"
$ProfileTemplate = Join-Path $SdkRoot "toolchains\lib\$ProfileTemplateName"
$HnpCli = Join-Path $SdkRoot 'toolchains\hnpcli.exe'
$HnpSourceDirectory = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\zedtools'
$HnpProbeSource = Join-Path $HnpSourceDirectory 'zed-hnp-probe.c'
$HnpStagingDirectory = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\zedtools'
$HnpProbe = Join-Path $HnpStagingDirectory 'bin\zed-hnp-probe'
$HnpOutputDirectory = Join-Path $HapRoot 'entry\hnp\x86_64'
$HnpPackage = Join-Path $HnpOutputDirectory 'zedtools.hnp'
$HnpPackageRoot = Join-Path $HapRoot 'entry\hnp'
$DashBinary = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\dash-ohos'
$ToyboxBinary = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\toybox-ohos'
$GitBundle = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\git-ohos.tar'
$GitBundleStamp = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\git-ohos.patch.sha256'
$GitPatch = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\git-ohos.patch'
$OpenSshBundle = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\openssh-ohos.tar'
$OpenSshBundleStamp = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\openssh-ohos.inputs.sha256'
$AskpassSource = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\zed-askpass.c'
$NodeBundle = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\node-ohos.tar'
$NodeBundleStamp = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\node-ohos.inputs.sha256'
$NodePatch = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\node-ohos.patch'
$NodeToolSource = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\zed-node-tool.c'
$NodeCompiler = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\ohos-clang++.sh'
$ToyboxConfig = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\toybox.config'
$ToyboxCompiler = Join-Path $RepositoryRoot 'crates\zed_ohos\hnp\ohos-clang.sh'
$DashBuilderImage = 'alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc'
$DashSourceCommit = '4bbf8721a3ac6401ced6a0454956801f6ba37256'
$GitSourceCommit = 'c44beea485f0f2feaf460e2ac87fdd5608d63cf0'
$CurlSourceCommit = 'cfbfb65047e85e6b08af65fe9cdbcf68e9ad496a'
$OpenSslSourceCommit = '0893a62353583343eb712adef6debdfbe597c227'
$OpenSshSourceCommit = 'e8dd756725e8800fcd0b3fd71ee6b4382d1e8fab'
$OpenSshBuildRevision = '1'
$NodeVersion = '22.23.2'
$NodeSourceCommit = 'aa4c77582be995286fc6e00aaf530dc7ade102a9'
$NodeBuildRevision = '2'
$NodePackageRevision = '4'
$ReadElf = Join-Path $SdkRoot 'native\llvm\bin\llvm-readelf.exe'

if (-not (Test-Path -LiteralPath $SdkRoot -PathType Container)) {
    throw "HarmonyOS SDK was not found at $SdkRoot"
}
if (-not (Test-Path -LiteralPath $Linker -PathType Leaf)) {
    throw "OHOS Clang wrapper was not found at $Linker"
}
foreach ($NativeBuildTool in @($CxxCompiler, $Cmake, $Ninja, $Strip)) {
    if (-not (Test-Path -LiteralPath $NativeBuildTool -PathType Leaf)) {
        throw "OHOS native build tool was not found at $NativeBuildTool"
    }
}

$env:OHOS_NDK_HOME = $SdkRoot
$env:DEVECO_SDK_HOME = Split-Path -Parent (Split-Path -Parent $SdkRoot)
$env:CC_x86_64_unknown_linux_ohos = $Linker
$env:CXX_x86_64_unknown_linux_ohos = $CxxCompiler
$env:AR_x86_64_unknown_linux_ohos = $Archiver
$env:CARGO_TARGET_X86_64_UNKNOWN_LINUX_OHOS_LINKER = $Linker
$env:CMAKE = $Cmake
$env:CMAKE_GENERATOR = 'Ninja'
$env:CMAKE_MAKE_PROGRAM = $Ninja
$env:PATH = "$NativeBuildTools;$env:PATH"
# OHOS exposes EOWNERDEAD and PTHREAD_MUTEX_ROBUST, but not the robust mutex
# functions that LMDB would otherwise call while initializing the prompt store.
$env:CFLAGS_x86_64_unknown_linux_ohos = (
    "$env:CFLAGS_x86_64_unknown_linux_ohos -DMDB_USE_ROBUST=0"
).Trim()

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Description failed with exit code $LASTEXITCODE"
    }
}

function Build-RustLibrary {
    Push-Location $RepositoryRoot
    try {
        $CargoArguments = @('build', '--target', $TargetTriple, '--package', 'zed_ohos')
        if ($RustProfile -ne 'dev') {
            $CargoArguments += @('--profile', $RustProfile)
        }
        Invoke-Checked {
            cargo @CargoArguments
        } "Rust OHOS $RustProfile build"
    } finally {
        Pop-Location
    }

    if (-not (Test-Path -LiteralPath $NativeLibrary -PathType Leaf)) {
        throw "Rust build did not produce $NativeLibrary"
    }
    New-Item -ItemType Directory -Path $PackagedLibraryDirectory -Force | Out-Null
    Copy-Item -LiteralPath $NativeLibrary -Destination $PackagedLibraryDirectory -Force
    $PackagedLibrary = Join-Path $PackagedLibraryDirectory 'libzed_ohos.so'
    Invoke-Checked {
        & $Strip --strip-unneeded $PackagedLibrary
    } 'Rust OHOS library stripping'
}

function Test-OhosExecutable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }
    $Notes = & $ReadElf -n $Path 2>$null
    return $LASTEXITCODE -eq 0 -and ($Notes -match 'OHOS')
}

function Build-HnpShell {
    if (Test-OhosExecutable -Path $DashBinary) {
        return
    }
    if ($null -eq (Get-Command docker -ErrorAction SilentlyContinue)) {
        throw 'Docker is required to build the static HarmonyOS HNP shell'
    }

    $DashBuildCommand = @"
set -eu
apk add --no-cache build-base autoconf automake git clang lld llvm >/dev/null
rm -rf /tmp/dash /tmp/empty
mkdir -p /tmp/empty
llvm-ar rc /tmp/empty/libssp_nonshared.a
git clone --quiet --depth 1 --branch v0.5.12 https://git.kernel.org/pub/scm/utils/dash/dash.git /tmp/dash
test "`$(git -C /tmp/dash rev-parse HEAD)" = "$DashSourceCommit"
cd /tmp/dash
autoreconf -fi >/dev/null
OHOS_CC="clang --target=x86_64-linux-ohos --sysroot=/ohos/native/sysroot -resource-dir=/ohos/native/llvm/lib/clang/15.0.4 -L/ohos/native/llvm/lib/x86_64-linux-ohos -L/tmp/empty -fuse-ld=lld -fno-stack-protector -Wno-unused-command-line-argument -Wno-deprecated-non-prototype"
CC="`$OHOS_CC" CC_FOR_BUILD=cc ./configure --host=x86_64-linux-ohos >/dev/null
make -j2 CC_FOR_BUILD=cc >/dev/null
test -x src/dash
cp src/dash /work/target/zed-ohos-hnp/dash-ohos
"@
    Invoke-Checked {
        & docker run --rm `
            -v "${RepositoryRoot}:/work" `
            -v "${SdkRoot}:/ohos:ro" `
            -w /work `
            $DashBuilderImage `
            sh -lc $DashBuildCommand
    } 'OHOS dash shell cross-build'
    if (-not (Test-OhosExecutable -Path $DashBinary)) {
        throw "Dash build did not produce an OHOS executable at $DashBinary"
    }
}

function Build-HnpTools {
    if (Test-OhosExecutable -Path $ToyboxBinary) {
        return
    }
    if ($null -eq (Get-Command docker -ErrorAction SilentlyContinue)) {
        throw 'Docker is required to build the native HarmonyOS HNP toolset'
    }

    $ToyboxBuildCommand = @'
set -eu
apk add --no-cache bash build-base git clang lld llvm linux-headers >/dev/null
rm -rf /tmp/toybox /tmp/empty
mkdir -p /tmp/empty
llvm-ar rc /tmp/empty/libssp_nonshared.a
git clone --quiet --depth 1 --branch 0.8.11 https://github.com/landley/toybox.git /tmp/toybox
git -C /tmp/toybox rev-parse HEAD | grep -qx 122bbe602f50b7fe747751370035f6fd55e674d0
chmod +x /work/crates/zed_ohos/hnp/ohos-clang.sh
cd /tmp/toybox
compiler=/work/crates/zed_ohos/hnp/ohos-clang.sh
make CC="$compiler" allnoconfig >/dev/null
while IFS= read -r option; do
  name=${option%%=*}
  sed -i "s/^# $name is not set$/$option/" .config
done < /work/crates/zed_ohos/hnp/toybox.config
NOSTRIP=1 make CC="$compiler" -j8 toybox >/dev/null
cp generated/unstripped/toybox /work/target/zed-ohos-hnp/toybox-ohos
'@
    Invoke-Checked {
        & docker run --rm `
            -v "${RepositoryRoot}:/work" `
            -v "${SdkRoot}:/ohos:ro" `
            -w /work `
            $DashBuilderImage `
            sh -lc $ToyboxBuildCommand
    } 'OHOS Toybox toolset cross-build'
    if (-not (Test-OhosExecutable -Path $ToyboxBinary)) {
        throw "Toybox build did not produce an OHOS executable at $ToyboxBinary"
    }
}

function Build-HnpGit {
    $ExpectedPatchHash = (Get-FileHash -LiteralPath $GitPatch -Algorithm SHA256).Hash
    $CachedPatchHash = if (Test-Path -LiteralPath $GitBundleStamp -PathType Leaf) {
        (Get-Content -LiteralPath $GitBundleStamp -Raw).Trim()
    } else {
        $null
    }
    if (
        (Test-Path -LiteralPath $GitBundle -PathType Leaf) -and
        $CachedPatchHash -eq $ExpectedPatchHash
    ) {
        return
    }
    if ($null -eq (Get-Command docker -ErrorAction SilentlyContinue)) {
        throw 'Docker is required to build Git for the HarmonyOS HNP'
    }

    $GitBuildCommand = @"
set -eu
apk add --no-cache bash build-base git clang lld llvm perl linux-headers autoconf automake libtool pkgconf ca-certificates findutils >/dev/null
rm -rf /tmp/openssl /tmp/curl /tmp/git /tmp/deps /tmp/git-install /tmp/git-stage /tmp/empty
mkdir -p /tmp/deps /tmp/empty /tmp/git-stage/bin /tmp/git-stage/libexec/git-core /tmp/git-stage/share/git-core
llvm-ar rc /tmp/empty/libssp_nonshared.a
compiler=/work/crates/zed_ohos/hnp/ohos-clang.sh
git clone --quiet --depth 1 --branch openssl-3.5.2 https://github.com/openssl/openssl.git /tmp/openssl
test "`$(git -C /tmp/openssl rev-parse HEAD)" = "$OpenSslSourceCommit"
cd /tmp/openssl
CC="`$compiler" AR=llvm-ar RANLIB=llvm-ranlib ./Configure linux-x86_64 no-shared no-tests no-apps no-docs no-module no-legacy --prefix=/tmp/deps --libdir=lib >/dev/null
make -s -j8 build_libs >/dev/null
make -s install_dev >/dev/null 2>&1
git clone --quiet --depth 1 --branch curl-8_15_0 https://github.com/curl/curl.git /tmp/curl
test "`$(git -C /tmp/curl rev-parse HEAD)" = "$CurlSourceCommit"
cd /tmp/curl
autoreconf -fi >/dev/null
PKG_CONFIG_PATH=/tmp/deps/lib/pkgconfig CC="`$compiler" AR=llvm-ar RANLIB=llvm-ranlib ./configure --build=x86_64-alpine-linux-musl --host=x86_64-pc-linux-gnu --prefix=/tmp/deps --disable-shared --enable-static --with-openssl=/tmp/deps --with-ca-bundle=/usr/share/zed/ca-certificates.crt --without-libpsl --without-zstd --without-brotli --without-libidn2 --without-nghttp2 --disable-ldap --disable-ldaps --disable-rtsp --disable-dict --disable-telnet --disable-tftp --disable-pop3 --disable-imap --disable-smb --disable-smtp --disable-gopher --disable-mqtt --disable-manual --disable-docs --disable-threaded-resolver >/dev/null
make -s -j8 >/dev/null
make -s install >/dev/null
git clone --quiet --depth 1 --branch v2.51.0 https://github.com/git/git.git /tmp/git
test "`$(git -C /tmp/git rev-parse HEAD)" = "$GitSourceCommit"
git -C /tmp/git apply /work/crates/zed_ohos/hnp/git-ohos.patch
cd /tmp/git
options='prefix=/usr RUNTIME_PREFIX=YesPlease NO_GETTEXT=YesPlease NO_TCLTK=YesPlease NO_PERL=YesPlease NO_PYTHON=YesPlease NO_EXPAT=YesPlease NO_OPENSSL=YesPlease NO_ICONV=YesPlease NO_INSTALL_HARDLINKS=YesPlease NO_REGEX=NeedsStartEnd NO_PTHREADS=YesPlease CURLDIR=/tmp/deps CURL_CONFIG=/tmp/deps/bin/curl-config'
make -s -j8 CC="`$compiler" AR=llvm-ar CFLAGS='-I/tmp/deps/include' LDFLAGS='-L/tmp/deps/lib' `$options
make -s CC="`$compiler" AR=llvm-ar CFLAGS='-I/tmp/deps/include' LDFLAGS='-L/tmp/deps/lib' DESTDIR=/tmp/git-install `$options install
cp /tmp/git-install/usr/bin/git /tmp/git-stage/bin/git
cp /tmp/git-install/usr/libexec/git-core/git-remote-http /tmp/git-stage/libexec/git-core/git-remote-http
cp /tmp/git-install/usr/libexec/git-core/git-remote-http /tmp/git-stage/libexec/git-core/git-remote-https
cp /tmp/git-install/usr/libexec/git-core/git-remote-http /tmp/git-stage/libexec/git-core/git-remote-ftp
find /tmp/git-install/usr/libexec/git-core -maxdepth 1 -type f -size -2M ! -name git -exec cp {} /tmp/git-stage/libexec/git-core/ \;
cp -R /tmp/git-install/usr/share/git-core/templates /tmp/git-stage/share/git-core/templates
mkdir -p /tmp/git-stage/share/certs
cp /etc/ssl/certs/ca-certificates.crt /tmp/git-stage/share/certs/ca-certificates.crt
tar -C /tmp/git-stage -cf /work/target/zed-ohos-hnp/git-ohos.tar .
"@
    Invoke-Checked {
        & docker run --rm `
            -v "${RepositoryRoot}:/work" `
            -v "${SdkRoot}:/ohos:ro" `
            -w /work `
            $DashBuilderImage `
            sh -lc $GitBuildCommand
    } 'OHOS Git cross-build'
    if (-not (Test-Path -LiteralPath $GitBundle -PathType Leaf)) {
        throw "Git build did not produce a package at $GitBundle"
    }
    Set-Content -LiteralPath $GitBundleStamp -Value $ExpectedPatchHash -NoNewline
}

function Build-HnpOpenSsh {
    $CompilerHash = (Get-FileHash -LiteralPath $ToyboxCompiler -Algorithm SHA256).Hash
    $ExpectedInputStamp = @(
        $OpenSshBuildRevision,
        $OpenSshSourceCommit,
        $OpenSslSourceCommit,
        $CompilerHash
    ) -join "`n"
    $CachedInputStamp = if (Test-Path -LiteralPath $OpenSshBundleStamp -PathType Leaf) {
        (Get-Content -LiteralPath $OpenSshBundleStamp -Raw).Trim()
    } else {
        $null
    }
    if (
        (Test-Path -LiteralPath $OpenSshBundle -PathType Leaf) -and
        $CachedInputStamp -eq $ExpectedInputStamp
    ) {
        return
    }
    if ($null -eq (Get-Command docker -ErrorAction SilentlyContinue)) {
        throw 'Docker is required to build OpenSSH for the HarmonyOS HNP'
    }

    $OpenSshBuildCommand = @"
set -eu
apk add --no-cache build-base autoconf automake git clang lld llvm linux-headers perl >/dev/null
rm -rf /tmp/openssl /tmp/openssh /tmp/deps /tmp/openssh-stage /tmp/empty
mkdir -p /tmp/deps /tmp/openssh-stage/bin /tmp/empty
llvm-ar rc /tmp/empty/libssp_nonshared.a
compiler=/work/crates/zed_ohos/hnp/ohos-clang.sh
git clone --quiet --depth 1 --branch openssl-3.5.2 https://github.com/openssl/openssl.git /tmp/openssl
test "`$(git -C /tmp/openssl rev-parse HEAD)" = "$OpenSslSourceCommit"
cd /tmp/openssl
CC="`$compiler" AR=llvm-ar RANLIB=llvm-ranlib ./Configure linux-x86_64 no-shared no-tests no-apps no-docs no-module no-legacy --prefix=/tmp/deps --libdir=lib >/dev/null
make -s -j8 build_libs >/dev/null
make -s install_dev >/dev/null 2>&1
git init --quiet /tmp/openssh
git -C /tmp/openssh remote add origin https://github.com/openssh/openssh-portable.git
git -C /tmp/openssh fetch --quiet --depth 1 origin $OpenSshSourceCommit
git -C /tmp/openssh checkout --quiet FETCH_HEAD
test "`$(git -C /tmp/openssh rev-parse HEAD)" = "$OpenSshSourceCommit"
cd /tmp/openssh
autoreconf -fi >/dev/null
ac_cv_header_linux_if_tun_h=no \
ac_cv_header_linux_if_h=no \
ac_cv_header_linux_seccomp_h=no \
ac_cv_header_linux_filter_h=no \
ac_cv_header_linux_audit_h=no \
CC="`$compiler" AR=llvm-ar RANLIB=llvm-ranlib \
CFLAGS=-I/tmp/deps/include LDFLAGS='-L/tmp/deps/lib -L/tmp/empty' \
./configure \
  --build=x86_64-alpine-linux-musl \
  --host=x86_64-pc-linux-gnu \
  --prefix=/usr \
  --sysconfdir=/etc/ssh \
  --with-ssl-dir=/tmp/deps \
  --without-zlib \
  --without-pam \
  --without-selinux \
  --without-libedit \
  --without-kerberos5 \
  --without-xauth \
  --without-shadow \
  --with-sandbox=no \
  --disable-strip >/dev/null
make -j8 ssh scp sftp >/dev/null
cp ssh scp sftp /tmp/openssh-stage/bin/
tar -C /tmp/openssh-stage -cf /work/target/zed-ohos-hnp/openssh-ohos.tar .
"@
    Invoke-Checked {
        & docker run --rm `
            -v "${RepositoryRoot}:/work" `
            -v "${SdkRoot}:/ohos:ro" `
            -w /work `
            $DashBuilderImage `
            sh -lc $OpenSshBuildCommand
    } 'OHOS OpenSSH cross-build'
    foreach ($Executable in @('ssh', 'scp', 'sftp')) {
        $VerificationDirectory = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\openssh-verify'
        New-Item -ItemType Directory -Path $VerificationDirectory -Force | Out-Null
        Invoke-Checked {
            tar -xf $OpenSshBundle -C $VerificationDirectory "./bin/$Executable"
        } "HarmonyOS OpenSSH $Executable verification staging"
        if (-not (Test-OhosExecutable -Path (Join-Path $VerificationDirectory "bin\$Executable"))) {
            throw "OpenSSH did not produce an OHOS $Executable executable"
        }
    }
    Set-Content -LiteralPath $OpenSshBundleStamp -Value $ExpectedInputStamp -NoNewline
}

function Build-HnpNode {
    if ($ReuseExistingNodeBundle) {
        if (-not (Test-Path -LiteralPath $NodeBundle -PathType Leaf)) {
            throw "Cannot reuse the HarmonyOS Node.js package because $NodeBundle does not exist"
        }
        Write-Warning 'Reusing the existing HarmonyOS Node.js package without updating its input stamp'
        return
    }

    $NodePatchHash = (Get-FileHash -LiteralPath $NodePatch -Algorithm SHA256).Hash
    $NodeToolHash = (Get-FileHash -LiteralPath $NodeToolSource -Algorithm SHA256).Hash
    $NodeCompilerHash = (Get-FileHash -LiteralPath $NodeCompiler -Algorithm SHA256).Hash
    $NodeCCompilerHash = (Get-FileHash -LiteralPath $ToyboxCompiler -Algorithm SHA256).Hash
    $ExpectedInputStamp = @(
        $NodePackageRevision,
        $NodeBuildRevision,
        $NodeSourceCommit,
        $NodePatchHash,
        $NodeToolHash,
        $NodeCompilerHash,
        $NodeCCompilerHash
    ) -join "`n"
    $CachedInputStamp = if (Test-Path -LiteralPath $NodeBundleStamp -PathType Leaf) {
        (Get-Content -LiteralPath $NodeBundleStamp -Raw).Trim()
    } else {
        $null
    }
    if (
        (Test-Path -LiteralPath $NodeBundle -PathType Leaf) -and
        $CachedInputStamp -eq $ExpectedInputStamp
    ) {
        return
    }
    if ($null -eq (Get-Command docker -ErrorAction SilentlyContinue)) {
        throw 'Docker is required to build Node.js for the HarmonyOS HNP'
    }

    # The package stamp also records the npm launcher. Rebuilding that small
    # launcher must not force a multi-hour V8 rebuild when the already-built
    # Node executable's own inputs are unchanged.
    $ExpectedNodeExecutableStamp = @(
        $NodeBuildRevision,
        $NodeSourceCommit,
        $NodePatchHash,
        $NodeCompilerHash,
        $NodeCCompilerHash
    ) -join "`n"
    $CachedInputLines = if ($null -ne $CachedInputStamp) {
        $CachedInputStamp -split "`r?`n"
    } else {
        @()
    }
    $CachedNodeExecutableStamp = if ($CachedInputLines.Count -ge 7) {
        @(
            $CachedInputLines[1],
            $CachedInputLines[2],
            $CachedInputLines[3],
            $CachedInputLines[5],
            $CachedInputLines[6]
        ) -join "`n"
    } else {
        $null
    }
    $BuiltNodeExecutable = Join-Path $RepositoryRoot "target\research\node-v$NodeVersion\out\Release\node"
    $BuildNodeExecutable =
        (-not (Test-Path -LiteralPath $BuiltNodeExecutable -PathType Leaf)) -or
        $CachedNodeExecutableStamp -ne $ExpectedNodeExecutableStamp
    $NodeExecutableBuildCommand = if ($BuildNodeExecutable) {
        'make -j2 -C out BUILDTYPE=Release V=0 node'
    } else {
        "echo 'Reusing cached Node.js executable; only HNP packaging inputs changed'"
    }
    $NodeConfigureCommand = if ($BuildNodeExecutable) {
        'rm -f config.gypi'
    } else {
        ':'
    }

    $NodeBuildCommand = @"
set -eu
apk add --no-cache bash build-base python3 linux-headers git clang lld llvm pkgconf >/dev/null
rm -rf /tmp/empty /tmp/node-stage
mkdir -p /tmp/empty /tmp/node-stage/bin /tmp/node-stage/lib/node_modules
llvm-ar rc /tmp/empty/libssp_nonshared.a
node_source=/work/target/research/node-v$NodeVersion
if ! test -d "`$node_source/.git"; then
  test ! -e "`$node_source"
  git clone --quiet --depth 1 --branch v$NodeVersion https://github.com/nodejs/node.git "`$node_source"
fi
test "`$(git -C "`$node_source" rev-parse HEAD)" = "$NodeSourceCommit"
if git -C "`$node_source" apply --check /work/crates/zed_ohos/hnp/node-ohos.patch; then
  git -C "`$node_source" apply /work/crates/zed_ohos/hnp/node-ohos.patch
elif git -C "`$node_source" apply --check --reverse /work/crates/zed_ohos/hnp/node-ohos.patch; then
  :
else
  echo 'Node.js source does not match the pinned OHOS patch' >&2
  exit 1
fi
chmod +x /work/crates/zed_ohos/hnp/ohos-clang.sh /work/crates/zed_ohos/hnp/ohos-clang++.sh
# Node's generated Makefile records the resolved cross-C++ compiler path.
# Recreate that stable path in every disposable Docker container so the
# incremental build remains valid instead of rebuilding all C++ objects.
ln -sf /work/crates/zed_ohos/hnp/ohos-clang++.sh /tmp/ohos-clangxx
cd "`$node_source"
$NodeConfigureCommand
if ! test -f config.gypi; then
  CC=/work/crates/zed_ohos/hnp/ohos-clang.sh \
  CXX=/work/crates/zed_ohos/hnp/ohos-clang++.sh \
  CC_host=cc CXX_host=c++ \
  python3 configure.py \
    --dest-os=openharmony \
    --dest-cpu=x64 \
    --cross-compiling \
    --with-intl=small-icu \
    --without-node-snapshot \
    --without-node-code-cache \
    --openssl-no-asm \
    --without-inspector
fi
# V8's generated initializer translation units are memory-intensive with the
# Alpine host compiler. Two jobs fit within the container limit while leaving
# enough host memory for the running PC emulator during a clean build.
# Invoke the generated target directly. The top-level `make node` wrapper
# recursively invokes `out/Makefile` without a goal, which also builds Node's
# C++ test executables and unrelated developer targets.
$NodeExecutableBuildCommand
test -x out/Release/node
cp out/Release/node /tmp/node-stage/bin/node
llvm-strip --strip-unneeded /tmp/node-stage/bin/node
/work/crates/zed_ohos/hnp/ohos-clang.sh \
  -std=c11 -O2 -Wall -Wextra -Werror \
  /work/crates/zed_ohos/hnp/zed-node-tool.c \
  -o /tmp/node-stage/bin/npm
llvm-strip --strip-unneeded /tmp/node-stage/bin/npm
cp /tmp/node-stage/bin/npm /tmp/node-stage/bin/npx
cp -R deps/npm /tmp/node-stage/lib/node_modules/npm
# HarmonyOS's HNP unzip implementation rejects zero-length ZIP entries. npm
# ships four empty marker/documentation files; none contain executable data.
find /tmp/node-stage -type f -empty -delete
tar -C /tmp/node-stage -cf /work/target/zed-ohos-hnp/node-ohos.tar .
"@
    Invoke-Checked {
        & docker run --rm `
            --memory=3000m `
            -v "${RepositoryRoot}:/work" `
            -v "${SdkRoot}:/ohos:ro" `
            -w /work `
            $DashBuilderImage `
            sh -lc $NodeBuildCommand
    } 'OHOS Node.js cross-build'
    if (-not (Test-Path -LiteralPath $NodeBundle -PathType Leaf)) {
        throw "Node.js build did not produce a package at $NodeBundle"
    }
    Set-Content -LiteralPath $NodeBundleStamp -Value $ExpectedInputStamp -NoNewline
}

function Set-HnpExecutablePermissions {
    Add-Type -AssemblyName System.IO.Compression
    $HnpStream = [System.IO.File]::Open(
        $HnpPackage,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::ReadWrite,
        [System.IO.FileShare]::None
    )
    $HnpArchive = [System.IO.Compression.ZipArchive]::new(
        $HnpStream,
        [System.IO.Compression.ZipArchiveMode]::Update,
        $false
    )
    try {
        foreach ($Entry in $HnpArchive.Entries) {
            $IsExecutable = (
                $Entry.FullName -like 'zedtools/bin/*' -or
                $Entry.FullName -like 'zedtools/libexec/git-core/*'
            ) -and -not $Entry.FullName.EndsWith('/')
            if ($IsExecutable) {
                # hnpcli.exe writes Windows ZIP attributes. The OHOS installer
                # derives chmod(0755) from the Unix S_IXOTH bit in this field.
                $Entry.ExternalAttributes = -2115174400 # 0100755 << 16 as Int32
            }
        }
    } finally {
        $HnpArchive.Dispose()
        $HnpStream.Dispose()
    }
}

function Build-Hnp {
    foreach ($RequiredFile in @(
        $HnpCli,
        $HnpProbeSource,
        (Join-Path $HnpSourceDirectory 'hnp.json'),
        $ToyboxConfig,
        $ToyboxCompiler,
        $GitPatch,
        $AskpassSource,
        $NodePatch,
        $NodeToolSource,
        $NodeCompiler
    )) {
        if (-not (Test-Path -LiteralPath $RequiredFile -PathType Leaf)) {
            throw "HNP build input was not found at $RequiredFile"
        }
    }

    $ExpectedHnpStagingDirectory = Join-Path $RepositoryRoot 'target\zed-ohos-hnp\zedtools'
    if ($HnpStagingDirectory -cne $ExpectedHnpStagingDirectory) {
        throw "Refusing to clean unexpected HNP staging directory $HnpStagingDirectory"
    }
    if (Test-Path -LiteralPath $HnpStagingDirectory -PathType Container) {
        Remove-Item -LiteralPath $HnpStagingDirectory -Recurse -Force
    }
    New-Item -ItemType Directory -Path (Split-Path -Parent $HnpProbe) -Force | Out-Null
    New-Item -ItemType Directory -Path $HnpOutputDirectory -Force | Out-Null
    Build-HnpShell
    Build-HnpTools
    Build-HnpGit
    Build-HnpOpenSsh
    Build-HnpNode
    Copy-Item -LiteralPath (Join-Path $HnpSourceDirectory 'hnp.json') -Destination $HnpStagingDirectory -Force
    Copy-Item -LiteralPath $DashBinary -Destination (Join-Path $HnpStagingDirectory 'bin\sh') -Force
    Copy-Item -LiteralPath $ToyboxBinary -Destination (Join-Path $HnpStagingDirectory 'bin\toybox') -Force
    Invoke-Checked {
        tar -xf $GitBundle -C $HnpStagingDirectory
    } 'HarmonyOS Git package staging'
    Invoke-Checked {
        tar -xf $NodeBundle -C $HnpStagingDirectory
    } 'HarmonyOS Node.js package staging'
    Invoke-Checked {
        tar -xf $OpenSshBundle -C $HnpStagingDirectory
    } 'HarmonyOS OpenSSH package staging'
    if (-not (Test-OhosExecutable -Path (Join-Path $HnpStagingDirectory 'bin\git'))) {
        throw 'The staged Git executable is not a HarmonyOS binary'
    }
    if (-not (Test-OhosExecutable -Path (Join-Path $HnpStagingDirectory 'bin\node'))) {
        throw 'The staged Node.js executable is not a HarmonyOS binary'
    }
    foreach ($Executable in @('ssh', 'scp', 'sftp')) {
        if (-not (Test-OhosExecutable -Path (Join-Path $HnpStagingDirectory "bin\$Executable"))) {
            throw "The staged OpenSSH $Executable executable is not a HarmonyOS binary"
        }
    }
    Invoke-Checked {
        & $Linker -std=c11 -O2 -Wall -Wextra -Werror $AskpassSource -o (Join-Path $HnpStagingDirectory 'bin\zed-askpass')
    } 'OHOS askpass helper build'
    Invoke-Checked {
        & $Linker -std=c11 -O2 -Wall -Wextra -Werror $HnpProbeSource -o $HnpProbe
    } 'OHOS HNP probe build'
    Invoke-Checked {
        & $HnpCli pack -i $HnpStagingDirectory -o $HnpOutputDirectory
    } 'OHOS HNP package build'
    if (-not (Test-Path -LiteralPath $HnpPackage -PathType Leaf)) {
        throw "HNP build did not produce $HnpPackage"
    }
    Set-HnpExecutablePermissions
}

function Build-Hap {
    if (-not (Test-Path -LiteralPath $Hvigor -PathType Leaf)) {
        throw "Hvigor was not found at $Hvigor"
    }
    Build-Hnp
    Push-Location $HapRoot
    try {
        Invoke-Checked {
            & $Hvigor assembleHap --no-daemon --mode module -p product=default -p module=entry@default
        } 'HarmonyOS HAP build'
    } finally {
        Pop-Location
    }
    Pack-HnpIntoHap
    Sign-Hap
}

function Pack-HnpIntoHap {
    if (-not (Test-Path -LiteralPath $PackingTool -PathType Leaf)) {
        throw "HarmonyOS packing tool was not found at $PackingTool"
    }

    $BuildRoot = Join-Path $HapRoot 'entry\build\default'
    $UnsignedHap = Join-Path $BuildRoot 'outputs\default\entry-default-unsigned.hap'
    Invoke-Checked {
        java '-Dfile.encoding=UTF-8' -jar $PackingTool `
            --mode hap `
            --force true `
            --lib-path (Join-Path $BuildRoot 'intermediates\stripped_native_libs\default') `
            --json-path (Join-Path $BuildRoot 'intermediates\package\default\module.json') `
            --resources-path (Join-Path $BuildRoot 'intermediates\res\default\resources') `
            --index-path (Join-Path $BuildRoot 'intermediates\res\default\resources.index') `
            --pack-info-path (Join-Path $BuildRoot 'outputs\default\pack.info') `
            --out-path $UnsignedHap `
            --ets-path (Join-Path $BuildRoot 'intermediates\loader_out\default\ets') `
            --pkg-context-path (Join-Path $BuildRoot 'intermediates\loader\default\pkgContextInfo.json') `
            --hnp-path $HnpPackageRoot
    } 'HarmonyOS HAP repack with HNP'
}

function Find-UnsignedHap {
    $Hap = Get-ChildItem -LiteralPath (Join-Path $HapRoot 'entry\build') -Recurse -Filter '*-unsigned.hap' |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $Hap) {
        throw 'Hvigor did not produce an unsigned HAP file'
    }
    return $Hap.FullName
}

function Find-SignedHap {
    $Hap = Get-ChildItem -LiteralPath (Join-Path $HapRoot 'entry\build') -Recurse -Filter '*-signed.hap' |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $Hap) {
        throw 'The signing step did not produce a signed HAP file'
    }
    return $Hap.FullName
}

function Sign-Hap {
    foreach ($RequiredFile in @($SigningTool, $SigningStore, $ProfileCertificate, $ProfileTemplate)) {
        if (-not (Test-Path -LiteralPath $RequiredFile -PathType Leaf)) {
            throw "HarmonyOS test signing material was not found at $RequiredFile"
        }
    }

    $UnsignedHap = Find-UnsignedHap
    $SigningDirectory = Join-Path $HapRoot 'entry\build\zed-signing'
    $UnsignedProfile = Join-Path $SigningDirectory 'zed-release-profile.json'
    $SignedProfile = Join-Path $SigningDirectory 'zed-release-profile.p7b'
    $ApplicationCertificate = Join-Path $SigningDirectory 'zed-application-release.pem'
    $ApplicationLeafCertificate = Join-Path $SigningDirectory 'zed-application-release-leaf.pem'
    $ApplicationCaCertificate = Join-Path $SigningDirectory 'openharmony-application-ca.pem'
    $ApplicationRootCertificate = Join-Path $SigningDirectory 'openharmony-application-root-ca.pem'
    $SignedHap = Join-Path (Split-Path -Parent $UnsignedHap) 'entry-default-signed.hap'
    New-Item -ItemType Directory -Path $SigningDirectory -Force | Out-Null

    $Profile = Get-Content -LiteralPath $ProfileTemplate -Raw | ConvertFrom-Json
    $Now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $Profile.validity.'not-before' = $Now - 300
    $Profile.validity.'not-after' = $Now + (10 * 365 * 24 * 60 * 60)
    $Profile.'bundle-info'.'bundle-name' = 'dev.zed.Zed'
    if ($SigningProfile -eq 'debug') {
        if ([string]::IsNullOrWhiteSpace($DebugDeviceUdid) -and (Test-Path -LiteralPath $Hdc -PathType Leaf)) {
            $UdidOutput = & $Hdc shell bm get -u 2>$null
            if ($LASTEXITCODE -eq 0 -and (($UdidOutput -join "`n") -match '[0-9A-Fa-f]{64}')) {
                $DebugDeviceUdid = $Matches[0]
            }
        }
        if ([string]::IsNullOrWhiteSpace($DebugDeviceUdid)) {
            Write-Warning 'No emulator UDID was detected; the SDK template debug-device list will be used'
        } else {
            $Profile.'debug-info'.'device-ids' = @($DebugDeviceUdid.ToUpperInvariant())
        }
    }
    $Profile.acls.'allowed-acls' = @(
        'ohos.permission.READ_PASTEBOARD',
        'ohos.permission.ALLOW_EXTERNAL_NATIVE_CODE'
    )
    $ProfileJson = $Profile | ConvertTo-Json -Depth 20
    [System.IO.File]::WriteAllText(
        $UnsignedProfile,
        $ProfileJson,
        [System.Text.UTF8Encoding]::new($false)
    )

    # The public SDK keystore exposes a self-signed certificate for the application
    # key. The profile template contains the CA-signed form of that same public key,
    # which is the leaf expected by hap-sign-tool and the package manager.
    $ApplicationCertificateField = if ($SigningProfile -eq 'debug') {
        'development-certificate'
    } else {
        'distribution-certificate'
    }
    $Profile.'bundle-info'.$ApplicationCertificateField |
        Set-Content -LiteralPath $ApplicationLeafCertificate -Encoding ascii
    Invoke-Checked {
        keytool -exportcert -rfc `
            -alias 'openharmony application ca' `
            -keystore $SigningStore `
            -storetype PKCS12 `
            -storepass 123456 `
            -file $ApplicationCaCertificate
    } 'Application CA certificate export'
    Invoke-Checked {
        keytool -exportcert -rfc `
            -alias 'openharmony application root ca' `
            -keystore $SigningStore `
            -storetype PKCS12 `
            -storepass 123456 `
            -file $ApplicationRootCertificate
    } 'Application root certificate export'
    Get-Content -LiteralPath @(
        $ApplicationLeafCertificate,
        $ApplicationCaCertificate,
        $ApplicationRootCertificate
    ) -Raw | Set-Content -LiteralPath $ApplicationCertificate -Encoding ascii

    Invoke-Checked {
        java -jar $SigningTool sign-profile `
            -mode localSign `
            -keyAlias $ProfileKeyAlias `
            -keyPwd 123456 `
            -profileCertFile $ProfileCertificate `
            -inFile $UnsignedProfile `
            -signAlg SHA256withECDSA `
            -keystoreFile $SigningStore `
            -keystorePwd 123456 `
            -outFile $SignedProfile
    } 'HarmonyOS profile signing'

    Invoke-Checked {
        java -jar $SigningTool sign-app `
            -mode localSign `
            -keyAlias 'openharmony application release' `
            -keyPwd 123456 `
            -appCertFile $ApplicationCertificate `
            -profileFile $SignedProfile `
            -profileSigned 1 `
            -inFile $UnsignedHap `
            -signAlg SHA256withECDSA `
            -keystoreFile $SigningStore `
            -keystorePwd 123456 `
            -outFile $SignedHap `
            -compatibleVersion 24 `
            -signCode 1
    } 'HarmonyOS HAP signing'
}

function Install-Hap {
    if (-not (Test-Path -LiteralPath $Hdc -PathType Leaf)) {
        throw "HDC was not found at $Hdc"
    }
    $Hap = Find-SignedHap
    $InstallOutput = & $Hdc install -r $Hap
    $InstallOutput | Write-Output
    $InstallText = $InstallOutput -join [Environment]::NewLine
    if ($LASTEXITCODE -ne 0 -or $InstallText -notmatch 'install bundle successfully') {
        throw "HAP installation failed: $InstallText"
    }
}

function Run-App {
    Invoke-Checked {
        & $Hdc shell aa start -a EntryAbility -b dev.zed.Zed
    } 'HarmonyOS app launch'
}

switch ($Step) {
    'rust' { Build-RustLibrary }
    'hap' {
        Build-RustLibrary
        Build-Hap
    }
    'install' { Install-Hap }
    'run' { Run-App }
    'all' {
        Build-RustLibrary
        Build-Hap
        Install-Hap
        Run-App
    }
}
