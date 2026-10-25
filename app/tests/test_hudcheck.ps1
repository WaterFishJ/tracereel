# hudcheck：无窗口像素回归（HUD + 地图摆放/外观 + 多地图框 + 单框等价性 + 性能烟测）。
#
# hudcheck 在内部内存造素材（不需要外部视频/地图文件），每项检查失败会直接 panic → 非 0 退出；
# 全部通过时打印 `HUD snapshots and deterministic time tests passed`。
# `--render-only` 跳过最后的完整 gui.qml 加载（那一步依赖真实窗口/截图，CI 上不需要）。
#
# 用法: .\app\tests\test_hudcheck.ps1 [-SkipBuild]
param(
    [switch]$SkipBuild,
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin hudcheck
    if ($LASTEXITCODE -ne 0) { throw 'hudcheck build failed' }
}

$out = Join-Path $projectRoot 'target/hudcheck-run'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
$logs = & (Join-Path $projectRoot 'target/debug/hudcheck.exe') $out --render-only 2>&1
$code = $LASTEXITCODE
$logs | Set-Content -LiteralPath (Join-Path $out 'hudcheck.log')
$logs | Select-Object -Last 6 | Write-Output
$output = $logs -join "`n"
if ($code -ne 0) { throw "hudcheck 失败（退出码 $code，见 target/hudcheck-run/hudcheck.log）" }
if ($output -notmatch 'HUD snapshots and deterministic time tests passed') {
    throw 'hudcheck 未输出通过标记（见 target/hudcheck-run/hudcheck.log）'
}
Write-Output "PASS: hudcheck（$out）"
