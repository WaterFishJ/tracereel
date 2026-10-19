# 导出进度弹窗一致性回归：直接导出的「编码」窗口与「导出队列」里正在导出的那一项必须显示
# 同一套内容 —— 导出配置 + 已用时 / 剩余用时 / 当前帧 / 总帧数 / 帧/秒；
# 其余（总用时、视频当前/总长、输出大小）都不该出现。
#
# 做法：ffmpeg 造一段 3 秒素材 + 最小工程（只勾「视频」，输出路径指向临时目录），
# 用 qmlprobe 驱动真实 gui.qml 走「点导出」→ 弹窗起来 → 断言内容 → 再放进导出队列，
# 断言队列条目持有同一段配置文字并复用同一个统计组件。
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

$tmp = Join-Path $env:TEMP ("tracereel-export-progress-" + $PID)
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

function Write-Utf8([string]$path, [string]$text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding($false)))
}

try {
    $clip = Join-Path $tmp 'clip.mp4'
    & $ffmpeg -y -loglevel error -f lavfi -i 'testsrc2=size=320x240:rate=30:duration=3' `
        -f lavfi -i 'sine=frequency=440:duration=3' -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest $clip
    if ($LASTEXITCODE -ne 0) { throw 'ffmpeg 生成测试素材失败' }

    $clipPosix = $clip -replace '\\', '/'
    $outPosix = (Join-Path $tmp 'out.mp4') -replace '\\', '/'
    $trr = Join-Path $tmp 'progress.trr'
    Write-Utf8 $trr @"
[video]
videoFilePath=$clipPosix

[encode]
resolution=source
frameRate=source
includeMap=false
includeHud=false
includeVideo=true
clipMode=none
outputFilePath=$outPosix
"@

    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'export_progress_probe.qml'
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '120000'
    $env:QMLPROBE_ORV = $trr
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | ForEach-Object { Write-Host $_ }
    $text = $logs -join "`n"
    if ($code -ne 0 -or $text -notmatch 'EXPORT_PROGRESS_PASS' -or $text -match 'EXPORT_PROGRESS_FAIL') {
        throw "导出进度弹窗一致性回归失败（exit=$code）"
    }
    Write-Output 'EXPORT_PROGRESS_TEST_PASS'
}
finally {
    Remove-Item Env:QMLPROBE_ORV -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_FILE -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
