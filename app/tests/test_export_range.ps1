# 导出页「导出范围」+「自定义时间」的端到端回归（探针见 app/tests/export_range_probe.qml）。
#
# 用一个最小工程（只导视频）无窗口驱动真实的 app/src/bin/gui.qml，断言：
#   · 下拉从「分段导出」改名为「导出范围」，四挡 = 全部 / 逐段导出 / 自定义分段 / 自定义时间；
#   · 选「自定义分段」只出分段范围行，选「自定义时间」只出时间范围行；
#   · 时间行填 1:05 / 2:30 → 65000 / 150000 毫秒；结束填 0:30 时保存值仍是 30000（导出时才按到末尾处理）；
#   · 切回「全部」→ 模式 none（整段导出），但填过的起止仍原样保留。
#
# 用法: .\app\tests\test_export_range.ps1 [-SkipBuild]
param(
    [switch]$SkipBuild,
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe build failed' }
}
$ffmpeg = Join-Path $projectRoot 'tools/ffmpeg/bin/ffmpeg.exe'
if (-not (Test-Path $ffmpeg)) { throw "找不到 ffmpeg：$ffmpeg" }

$tmp = Join-Path $env:TEMP ("tracereel-export-range-" + $PID)
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $clip = Join-Path $tmp 'probe_clip.mp4'
    $trr = Join-Path $tmp 'probe_range.trr'
    & $ffmpeg -y -loglevel error -f lavfi -i 'testsrc=size=320x240:rate=24:duration=3' -c:v libx264 -pix_fmt yuv420p $clip
    if ($LASTEXITCODE -ne 0) { throw 'ffmpeg 生成测试视频失败' }
    $ini = "[video]`nvideoFilePath=$($clip -replace '\\','/')`nstartTimeOffset=0`n`n[encode]`nresolution=source`nframeRate=source`nincludeMap=false`nincludeHud=false`nincludeVideo=true`nclipMode=none`n"
    [IO.File]::WriteAllText($trr, $ini, (New-Object Text.UTF8Encoding($false)))

    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'          # 无窗口：本探针不需要真的出帧
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'export_range_probe.qml'
    $env:QMLPROBE_ORV = $trr
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '90000'
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | Write-Output
    $output = $logs -join "`n"
    if ($code -ne 0 -or $output -notmatch 'EXPORT_RANGE_PASS' -or $output -match 'EXPORT_RANGE_FAIL') {
        throw '导出范围端到端回归失败（见上面的 [probe] 行）'
    }

    # 真导出一段「自定义时间」：3 秒素材只导 1s→2.5s，成片时长应在 1.5 秒左右。
    $rangeTrr = Join-Path $tmp 'probe_time_range.trr'
    $ini2 = "[video]`nvideoFilePath=$($clip -replace '\\','/')`nstartTimeOffset=0`n`n[encode]`nresolution=source`nframeRate=source`nincludeMap=false`nincludeHud=false`nincludeVideo=true`nclipMode=time`nrangeStartMs=1000`nrangeEndMs=2500`n"
    [IO.File]::WriteAllText($rangeTrr, $ini2, (New-Object Text.UTF8Encoding($false)))
    $out = Join-Path $tmp 'range_out.mp4'
    $encLogs = & (Join-Path $projectRoot 'target/debug/encgui.exe') $rangeTrr --out $out 2>&1
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $out)) {
        $encLogs | Write-Output
        throw 'encgui 导出失败（自定义时间）'
    }
    $durText = (& (Join-Path $projectRoot 'tools/ffmpeg/bin/ffprobe.exe') -v error -show_entries format=duration -of csv=p=0 $out) -join ''
    $dur = [double]$durText.Trim()
    Write-Output ("[range] 自定义时间 1.0s→2.5s 导出时长 = {0:N2}s" -f $dur)
    if ([Math]::Abs($dur - 1.5) -gt 0.3) { throw "自定义时间导出的时长不对：期望约 1.5 秒，实际 $dur 秒" }
    Write-Output 'EXPORT_RANGE_EXPORT_PASS'
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_ORV -ErrorAction SilentlyContinue
}
