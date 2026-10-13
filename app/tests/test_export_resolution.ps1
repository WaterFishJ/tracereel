# 导出页「分辨率」=「与原始视频一致」的端到端回归（探针见 app/tests/export_resolution_probe.qml）。
#
# 自己造素材：ffmpeg 生成一段 320x240（4:3）的小视频 + 一个 resolution=source 的工程，
# 再用 qmlprobe **无窗口**驱动真实的 app/src/bin/gui.qml，断言：
#   · 分辨率下拉第一项是「与原始视频一致」（token `source`）；
#   · 工程里存的 source 重新打开后原样保留（load_orv 不再归一成 1920x1080）；
#   · 预览画布比例跟随源视频（320x240 → 1.3333），切到 1280x720 变 16:9，切回又变源比例。
#
# 用法: .\app\tests\test_export_resolution.ps1 [-SkipBuild]
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

$tmp = Join-Path $env:TEMP ("tracereel-export-res-" + $PID)
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $clip = Join-Path $tmp 'probe_320x240.mp4'
    $trr = Join-Path $tmp 'probe_source.trr'
    # 4:3 的小视频：与 16:9 回退值区分得开，避免"比例对不对"测不出来。
    & $ffmpeg -y -loglevel error -f lavfi -i 'testsrc=size=320x240:rate=24:duration=2' -c:v libx264 -pix_fmt yuv420p $clip
    if ($LASTEXITCODE -ne 0) { throw 'ffmpeg 生成测试视频失败' }
    # 最小工程：缺的键由 Settings::from_ini 的默认值补齐。
    $ini = "[video]`nvideoFilePath=$($clip -replace '\\','/')`nstartTimeOffset=0`n`n[encode]`nresolution=source`nframeRate=source`nincludeMap=false`nincludeHud=false`nincludeVideo=true`n"
    [IO.File]::WriteAllText($trr, $ini, (New-Object Text.UTF8Encoding($false)))

    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'          # 无窗口：本探针不需要真的出帧
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'export_resolution_probe.qml'
    $env:QMLPROBE_ORV = $trr                     # gui.qml 启动即加载这个工程
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '90000'
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | Write-Output
    $output = $logs -join "`n"
    if ($code -ne 0 -or $output -notmatch 'EXPORT_RESOLUTION_PASS' -or $output -match 'EXPORT_RESOLUTION_FAIL') {
        throw '导出分辨率端到端回归失败（见上面的 [probe] 行）'
    }
    if ($output -notmatch 'source_size=320x240') { throw '源尺寸不是预期的 320x240（素材或探测有问题）' }
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
