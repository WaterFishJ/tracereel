# Run the TraceReel GUI.
#
# Sets PATH so the exe can find the Qt6 DLLs and ffmpeg, then launches it with a
# .trr project (.orv is also supported). No path starts with an empty project.
#
# Freshness is decided by Cargo, never by file timestamps: every launch runs
# `cargo build` for the requested profile, and Cargo rebuilds exactly the
# crates whose content fingerprints changed. This covers every workspace
# member (app, tracereel-core, receipt-ocr, gyroflow-src) plus Cargo.toml/build.rs,
# which the old timestamp check missed — most notably receipt-ocr/src, so a
# freshly changed OCR engine used to launch a stale exe.
#
# Usage:  .\run.ps1                (clean start, no project; rebuilds if needed)
#         .\run.ps1 path\to\proj.trr
#         .\run.ps1 --Release [path\to\proj.trr]  (release build; -Release also works)
#         .\run.ps1 --SkipBuild [path\to\proj.trr] (skip the check; -SkipBuild also works)

param([switch]$Release, [switch]$SkipBuild)

# PowerShell only binds native-style `-Release` to a switch parameter. Accept
# `--Release` too so it cannot be mistaken for a project path while the Debug
# executable is launched. `--SkipBuild` is accepted the same way.
$scriptArgs = @($args)
foreach ($flag in @("--Release", "--SkipBuild")) {
    if ($scriptArgs -contains $flag) {
        if ($flag -eq "--Release") { $Release = $true } else { $SkipBuild = $true }
        $scriptArgs = @($scriptArgs | Where-Object { $_ -ne $flag })
    }
}
if ($scriptArgs.Count -gt 1) {
    throw "只接受一个工程文件路径，收到：$($scriptArgs -join ' ')"
}
$project = if ($scriptArgs.Count -eq 1) { $scriptArgs[0] } else { "" }

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$QtRoot = if ($env:TRACEREEL_QT_ROOT) { $env:TRACEREEL_QT_ROOT } else { "C:\Qt\6.8.2\msvc2022_64" }
$QtBin = Join-Path $QtRoot "bin"
# ffmpeg 运行时目录。必须与 build.ps1 / package.ps1 用**同一个**带软件 AV1
# 编码器的构建（libsvtav1 / libaom-av1）—— 旧的 avbuild "gpl-lite" 没有它们，
# 用它会让 CPU + AV1 导出报 "Unrecognized option 'cpu-used'"。
$FfmpegBin = if ($env:TRACEREEL_FFMPEG_DIR) { Join-Path $env:TRACEREEL_FFMPEG_DIR "bin" } else { Join-Path $Root "tools\ffmpeg\bin" }
if (-not (Test-Path (Join-Path $FfmpegBin "ffmpeg.exe"))) {
    throw "找不到 ffmpeg.exe：$FfmpegBin（获取方式见 packaging\ffmpeg获取说明.md）"
}
if (-not (Test-Path (Join-Path $QtBin "Qt6Core.dll"))) {
    throw "找不到 Qt：$QtRoot。请安装 Qt 6.8（msvc2022_64），或用 TRACEREEL_QT_ROOT 指定其目录。"
}

# Qt must know where its QML modules live.
$env:QML_IMPORT_PATH = Join-Path $QtRoot "qml"
$env:QML2_IMPORT_PATH = Join-Path $QtRoot "qml"
# Use a non-native style so all controls are uniformly sized and customizable
# (native Windows style has inconsistent control sizes and rejects custom indicators).
$env:QT_QUICK_CONTROLS_STYLE = "Fusion"
$env:PATH = "$QtBin;$FfmpegBin;$env:PATH"

# 启动前先确认这个 ffmpeg 有软件 AV1 编码器：没有的话「CPU + AV1」导出会失败
# （报出毫无线索的 "Unrecognized option 'cpu-used'"）。这里提前警告，不要等到导出。
# 注意：对数组用 -notmatch 是逐元素匹配，必须 join 成单个字符串再判断。
$encoders = (& (Join-Path $FfmpegBin "ffmpeg.exe") -hide_banner -encoders 2>&1) -join "`n"
if ($encoders -notmatch 'libsvtav1|libaom-av1|librav1e') {
    Write-Warning "当前 ffmpeg 没有软件 AV1 编码器（libsvtav1/libaom-av1）：$FfmpegBin"
    Write-Warning "「CPU + AV1」导出将不可用。请按 packaging\ffmpeg获取说明.md 更换构建。"
}

$exe = if ($Release) {
    Join-Path $Root "target\release\tracereel-gui.exe"
} else {
    Join-Path $Root "target\debug\tracereel-gui.exe"
}
$profileLabel = if ($Release) { "release" } else { "debug" }

# 正在运行的 exe 在 Windows 上不能被覆盖：cargo 重新链接那一步会失败在
#   error: failed to remove file `...\target\release\tracereel-gui.exe`
#   Caused by: 拒绝访问。 (os error 5)
# 用户看到的就是「编译失败」—— 只有 TraceReel 窗口还开着才会这样。提前拦下来，
# 直接说清楚该关掉哪个窗口，别让人去猜源码哪里写错了。
# （--SkipBuild 不编译，只再开一个窗口，所以不拦。）
if (-not $SkipBuild) {
    $running = @(Get-Process -Name "tracereel-gui" -ErrorAction SilentlyContinue |
        Where-Object { -not $_.Path -or $_.Path -ieq $exe })
    if ($running.Count -gt 0) {
        $pids = ($running | ForEach-Object { $_.Id }) -join "、"
        throw "TraceReel 正在运行（PID $pids），Windows 不允许覆盖正在运行的程序：cargo 会报「拒绝访问。(os error 5)」并显示编译失败。请先关闭 TraceReel 窗口（或结束该进程）后重试。"
    }
}

# Always let Cargo decide. Its freshness check works on content fingerprints
# and covers the whole dependency graph, so an unchanged tree only costs the
# sub-second check while a changed tree can never launch a stale binary.
if ($SkipBuild) {
    Write-Host "已跳过编译检查（--SkipBuild），直接启动现有可执行文件。"
} else {
    Write-Host "正在用 Cargo 检查 $profileLabel 版本是否需要重新编译…"
    $buildArgs = @("build")
    if ($Release) { $buildArgs += "--release" }
    $buildArgs += @("-p", "tracereel", "--bin", "tracereel-gui")
    & (Join-Path $Root "build.ps1") @buildArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Host "编译失败，已取消启动；请修复上面的错误后重试。"
        Write-Host "（若上面是「拒绝访问。(os error 5)」，说明 TraceReel 窗口还开着：先关掉它再重试。）"
        exit 1
    }
}

if (-not (Test-Path $exe)) {
    if ($Release -and -not $SkipBuild) {
        # The release build is the one just verified, so a missing release exe
        # means the GUI binary name changed or the profile is not configured.
        Write-Host "release 版 tracereel-gui.exe 不存在，请检查 Cargo.toml 的 [[bin]] 与 --release 编译结果。"
        exit 1
    }
    Write-Host "tracereel-gui.exe 不存在：$exe"
    exit 1
}

$exeTime = (Get-Item $exe).LastWriteTime
Write-Host ("启动 TraceReel（{0} 版，编译于 {1:yyyy-MM-dd HH:mm:ss}）：{2}" -f
    $profileLabel, $exeTime, $(if ($project) { $project } else { "空白工程" }))
if ($project) { & $exe $project } else { & $exe }
