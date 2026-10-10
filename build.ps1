# Build helper for TraceReel.
#
# Sets the non-obvious environment variables needed to build & link
# gyroflow_core (wgpu), ffmpeg (FFMPEG_DIR / LIBCLANG_PATH) and the Qt6 GUI
# (qmetaobject / qttypes: QT_INCLUDE_PATH + QT_LIBRARY_PATH must both be set,
# and qmake must be on PATH).
#
# Usage:  .\build.ps1 [cargo args...]   e.g.  .\build.ps1 build --release
#
# 可用环境变量覆盖（详见 README 的「构建前置」）：
#   TRACEREEL_QT_ROOT     Qt 6.8 的 msvc2022_64 目录
#   TRACEREEL_FFMPEG_DIR  含 bin\ffmpeg.exe 与 include/lib 的 ffmpeg 目录
#   LIBCLANG_PATH         含 libclang.dll 的目录

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$QtRoot = if ($env:TRACEREEL_QT_ROOT) { $env:TRACEREEL_QT_ROOT } else { "C:\Qt\6.8.2\msvc2022_64" }
if (-not (Test-Path (Join-Path $QtRoot "bin\qmake.exe"))) {
    throw "找不到 Qt：$QtRoot。请安装 Qt 6.8（msvc2022_64），或用 TRACEREEL_QT_ROOT 指定其目录。"
}

# ffmpeg dev directory. 必须含**软件 AV1 编码器**（libsvtav1 / libaom-av1），
# 否则 AV1 硬件不可用时无法回退到 CPU 编码。获取方式见 packaging/ffmpeg获取说明.md。
# 旧的 avbuild "gpl-lite" 构建没有任何软件 AV1 编码器，不要再用。
$FFmpegDir = if ($env:TRACEREEL_FFMPEG_DIR) { $env:TRACEREEL_FFMPEG_DIR } else { Join-Path $Root "tools\ffmpeg" }

# libclang（rust-cpp / qttypes 需要）：优先 LIBCLANG_PATH，其次常见安装位置。
$LibClang = $env:LIBCLANG_PATH
if (-not $LibClang) {
    $candidates = @()
    if ($env:ProgramFiles) { $candidates += (Join-Path $env:ProgramFiles "LLVM\bin") }
    if ($env:LOCALAPPDATA) {
        $pipClang = Get-ChildItem (Join-Path $env:LOCALAPPDATA "Programs\Python\*\Lib\site-packages\clang\native") -Directory -ErrorAction SilentlyContinue
        $candidates += @($pipClang | ForEach-Object { $_.FullName })
    }
    $LibClang = $candidates | Where-Object { $_ -and (Test-Path (Join-Path $_ "libclang.dll")) } | Select-Object -First 1
}
if (-not $LibClang) {
    throw "未找到 libclang。请安装 LLVM，或用 LIBCLANG_PATH 指向含 libclang.dll 的目录。"
}

$env:FFMPEG_DIR = $FFmpegDir
$env:LIBCLANG_PATH = $LibClang
$env:PATH = "$QtRoot\bin;$FFmpegDir\bin;$env:PATH"

$env:QT_INCLUDE_PATH = "$QtRoot\include"
$env:QT_LIBRARY_PATH = "$QtRoot\lib"
$env:QML_IMPORT_PATH = "$QtRoot\qml"
$env:QML2_IMPORT_PATH = "$QtRoot\qml"

Write-Host "FFMPEG_DIR=$env:FFMPEG_DIR"
Write-Host "QT_INCLUDE_PATH=$env:QT_INCLUDE_PATH"
Write-Host "QT_LIBRARY_PATH=$env:QT_LIBRARY_PATH"

Push-Location $Root
$cargoExit = 1
try {
    cargo @args
    $cargoExit = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $cargoExit
