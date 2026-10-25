param(
    [switch]$SkipBuild,
    [string]$Project = '',
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$artifacts = Join-Path $projectRoot 'target/preview-seek'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe build failed' }
}
if (-not $Project) {
    $clip = Join-Path $artifacts 'clip.mp4'
    & (Join-Path $projectRoot 'tools/ffmpeg/bin/ffmpeg.exe') -y -loglevel error -f lavfi `
        -i 'testsrc2=size=320x240:rate=30:duration=12' -c:v libx264 -g 90 -pix_fmt yuv420p $clip
    if ($LASTEXITCODE -ne 0) { throw 'Could not generate seek fixture' }
    $Project = Join-Path $artifacts 'fixture.trr'
    $body = "[video]`nvideoFilePath=$($clip -replace '\\','/')`n[encode]`nresolution=320x240`nincludeMap=false`nincludeHud=false`n[gyro]`nenabled=false`n"
    [IO.File]::WriteAllText($Project, $body)
}
$names = @('PATH','QT_QPA_PLATFORM','QT_FORCE_STDERR_LOGGING','QMLPROBE_FILE','QMLPROBE_ORV',
    'QMLPROBE_RUN_LOOP','QMLPROBE_TIMEOUT_MS','QMLPROBE_SCREENSHOT','QMLPROBE_LOAD_ONLY')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'preview_seek_probe.qml'
    $env:QMLPROBE_ORV = [IO.Path]::GetFullPath($Project)
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '240000'
    Remove-Item Env:QMLPROBE_SCREENSHOT,Env:QMLPROBE_LOAD_ONLY -ErrorAction SilentlyContinue
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | Set-Content -LiteralPath (Join-Path $artifacts 'qmlprobe.log')
    $logs | Select-String 'PREVIEW_SEEK|SEEK_LANDED'
    $text = $logs -join "`n"
    if ($code -ne 0 -or $text -notmatch 'PREVIEW_SEEK_PASS:' -or $text -match 'PREVIEW_SEEK_FAIL:') {
        throw "Preview seek regression failed: $artifacts/qmlprobe.log"
    }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
