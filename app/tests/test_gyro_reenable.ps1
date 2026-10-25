# Real project regression: initial stabilization -> off -> on -> ready again.
param(
    [string]$Project = '',
    [switch]$SkipBuild,
    [int]$TimeoutSeconds = 600,
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $Project) {
    $Project = Join-Path $projectRoot '../demo/20261011孝德湖/孝德湖上午.trr'
}
if (-not (Test-Path -LiteralPath $Project)) { throw "找不到带运动数据的工程：$Project" }
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe build failed' }
}
$artifacts = Join-Path $projectRoot 'target/gyro-reenable'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$names = @('PATH','LOCALAPPDATA','QT_QPA_PLATFORM','QT_QPA_FONTDIR','QT_FORCE_STDERR_LOGGING',
    'QMLPROBE_FILE','QMLPROBE_ORV','QMLPROBE_RUN_LOOP','QMLPROBE_TIMEOUT_MS','QMLPROBE_SCREENSHOT','QMLPROBE_LOAD_ONLY')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:LOCALAPPDATA = Join-Path $artifacts 'user-data'
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'gyro_reenable_probe.qml'
    $env:QMLPROBE_ORV = [IO.Path]::GetFullPath($Project)
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = [string](($TimeoutSeconds + 10) * 1000)
    Remove-Item Env:QMLPROBE_SCREENSHOT,Env:QMLPROBE_LOAD_ONLY -ErrorAction SilentlyContinue
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | Set-Content -LiteralPath (Join-Path $artifacts 'qmlprobe.log')
    $logs | Select-String 'GYRO_REENABLE|start_gyro_load|gyro load finished'
    $output = $logs -join "`n"
    if ($code -ne 0 -or $output -notmatch 'GYRO_REENABLE_PASS:' -or $output -match 'GYRO_REENABLE_FAIL:') {
        throw "关闭后重新启用防抖回归失败，日志：$artifacts/qmlprobe.log"
    }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
