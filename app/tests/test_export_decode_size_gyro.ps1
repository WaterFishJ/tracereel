# 导出解码尺寸（带防抖）端到端回归：驱动**真实 gui.qml** 导出一小段带运动数据的视频，
# 验证「导出解码尺寸 ≠ 预览防抖尺寸」时会在编码线程上按导出尺寸重新加载防抖（探针见
# app/tests/export_decode_size_probe.qml）。
#
# 需要一份**带运动数据**的真实素材（默认用本机 demo 工程；素材是个人内容、不入库）。
# 整段导出要几十分钟，所以探针编够约 60 帧就主动中止（中止是预期结果，不是失败）。
# 用法: .\app\tests\test_export_decode_size_gyro.ps1 [-Project <trr>] [-SkipBuild]
param(
    [string]$Project = '',
    [switch]$SkipBuild,
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $Project) {
    $Project = [IO.Path]::GetFullPath((Join-Path $projectRoot '../demo/20261011孝德湖/孝德湖上午.trr'))
}
if (-not (Test-Path $Project)) {
    throw "找不到带运动数据的工程：$Project（用 -Project 指定一个真实素材的 .trr）"
}
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe build failed' }
}

$tmp = Join-Path $env:TEMP ("tracereel-export-gyro-" + $PID)
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $out = Join-Path $tmp 'out.mp4'
    $trr = Join-Path $tmp 'probe.trr'
    # 只把输出路径换成临时文件：时长限制交给探针（编够一小段就主动中止 —— GUI 这一路
    # 没法从工程里限制时长，剪辑范围不是代理属性，临时设置文件里写的是默认值）。
    $lines = Get-Content -LiteralPath $Project
    $kept = $lines | Where-Object { $_ -notmatch '^outputFilePath\s*=' }
    $body = @("outputFilePath=$($out -replace '\\','/')")
    $idx = [Array]::IndexOf($kept, '[encode]')
    if ($idx -lt 0) { throw "工程里没有 [encode] 段：$Project" }
    $merged = @($kept[0..$idx]) + $body + @($kept[($idx + 1)..($kept.Count - 1)])
    [IO.File]::WriteAllLines($trr, $merged, (New-Object Text.UTF8Encoding($false)))

    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'export_decode_size_probe.qml'
    $env:QMLPROBE_ORV = $trr
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '900000'
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | Write-Output
    $text = $logs -join "`n"
    if ($code -ne 0 -or $text -notmatch 'EXPORT_GYRO_PASS' -or $text -match 'EXPORT_GYRO_FAIL') {
        throw '导出解码尺寸（带防抖）端到端回归失败（见上面的 [probe] 行）'
    }
    if ($text -notmatch '按导出尺寸 \d+x\d+ 重新加载防抖') {
        throw '导出没有按导出尺寸重新加载防抖：这条重载路径没被走到'
    }
    if (-not (Test-Path $out)) { throw "导出文件没有生成：$out" }
    $appLog = Get-ChildItem $tmp -Filter 'tracereel-*.log' | Select-Object -First 1
    if ($appLog) {
        $hit = Select-String -Path $appLog.FullName -Pattern '解码尺寸 \d+x\d+'
        if (-not $hit) { throw "导出日志里没有『解码尺寸』一行：$($appLog.FullName)" }
        $hit.Line
    }
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
