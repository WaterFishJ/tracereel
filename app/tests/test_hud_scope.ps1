# HUD 显示范围 / 「预览跟随导出勾选」的端到端回归。
#
# 覆盖两件事：
#   1) 预览（app/tests/hud_scope_probe.qml 驱动真实 gui.qml）：
#      · HUD 页不再有「预览」勾选，改成「显示范围」（视频区 / 整幅画面）；
#      · 只导地图+HUD（没有视频面板）时 HUD 铺满整帧 —— 老代码按视频区算，宽度为 0，HUD 整块看不见；
#      · 取消勾选「导出 → HUD」后，预览里的 HUD 也立刻消失（预览就是按导出三个勾选合成的）；
#      · 有视频时切到「整幅画面」，HUD 能铺到地图那半边。
#   2) 导出（真实导出管线 encgui --map-only）：只导地图+HUD 时，HUD 必须画进画面里
#      —— 用「勾 HUD / 不勾 HUD」两次导出的帧差来判，并要求差分出现在左半边（地图区）。
#
# 素材自造（ffmpeg 生成一张地图 + 一段小视频 + 一条 GPX），不依赖个人 demo。
# 用法: .\app\tests\test_hud_scope.ps1 [-SkipBuild] [-KeepArtifacts]
param(
    [switch]$SkipBuild,
    [switch]$KeepArtifacts,
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe --bin encgui
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe / encgui build failed' }
}
$ffmpeg = Join-Path $projectRoot 'tools/ffmpeg/bin/ffmpeg.exe'
if (-not (Test-Path $ffmpeg)) { throw "找不到 ffmpeg：$ffmpeg" }

$tmp = Join-Path $env:TEMP ("tracereel-hud-scope-" + $PID)
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$artifacts = Join-Path $projectRoot 'target/hud-scope'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null

function Write-Utf8([string]$path, [string]$text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding($false)))
}

try {
    $map = Join-Path $tmp 'map.png'
    $clip = Join-Path $tmp 'clip.mp4'
    $gpx = Join-Path $tmp 'track.gpx'
    # 地图：一片带条纹的底色，HUD 压上去时像素差别看得清。
    & $ffmpeg -y -loglevel error -f lavfi -i 'color=c=0x2f6b3f:s=800x600' -frames:v 1 $map
    if ($LASTEXITCODE -ne 0) { throw 'ffmpeg 生成地图失败' }
    # 带音轨（440 Hz 正弦）：「没勾视频就不出声 / 勾了视频就有声」要靠它断言。
    & $ffmpeg -y -loglevel error -f lavfi -i 'testsrc=size=320x240:rate=24:duration=3' `
        -f lavfi -i 'sine=frequency=440:duration=3' -c:v libx264 -pix_fmt yuv420p `
        -c:a aac -b:a 96k -shortest $clip
    if ($LASTEXITCODE -ne 0) { throw 'ffmpeg 生成测试视频失败' }

    # GPX：60 个点、每秒一个，够 HUD 显示时间和配速。
    $sb = [Text.StringBuilder]::new()
    [void]$sb.AppendLine('<?xml version="1.0" encoding="UTF-8"?>')
    [void]$sb.AppendLine('<gpx version="1.1" creator="tracereel-test"><trk><trkseg>')
    $base = [datetime]'2026-01-01T00:00:00Z'
    for ($i = 0; $i -lt 60; $i++) {
        $t = $base.AddSeconds($i).ToString('yyyy-MM-ddTHH:mm:ssZ')
        $lat = 23.0 + $i * 0.00005
        $lon = 113.0 + [math]::Sin($i / 8.0) * 0.0004
        [void]$sb.AppendLine("  <trkpt lat=`"$lat`" lon=`"$lon`"><ele>80</ele><time>$t</time></trkpt>")
    }
    [void]$sb.AppendLine('</trkseg></trk></gpx>')
    Write-Utf8 $gpx $sb.ToString()

    $mapPosix = $map -replace '\\', '/'
    $clipPosix = $clip -replace '\\', '/'
    $gpxPosix = $gpx -replace '\\', '/'

    # 只导地图+HUD：没有视频文件，includeVideo=false。画面占比 0.25 是老代码出错的前提
    # （按视频区算 → 左边先空出 25%，视频区宽 = 0 → HUD 整块塌掉）。
    $mapOnly = Join-Path $tmp 'map_only.trr'
    Write-Utf8 $mapOnly @"
[map]
imageFilePath=$mapPosix
relativeWidth=0.25

[hud]
scope=video
gpxFilePath=$gpxPosix

[encode]
resolution=640x480
frameRate=30
includeMap=true
includeVideo=false
includeHud=true
"@

    # 带视频：用来验证「视频区 → 整幅画面」这一挡真的能把 HUD 摆到地图上。
    $withVideo = Join-Path $tmp 'with_video.trr'
    Write-Utf8 $withVideo @"
[video]
videoFilePath=$clipPosix
startTimeOffset=0

[map]
imageFilePath=$mapPosix
relativeWidth=0.25

[hud]
scope=video
gpxFilePath=$gpxPosix

[encode]
resolution=320x240
frameRate=24
includeMap=true
includeVideo=true
includeHud=true
"@

    # 有视频文件、但「导出 → 视频」没勾：预览里没有视频轨，因此既不该解码视频、
    # 也不该出声（老代码两件都做，选倍速时还更卡）。
    $silent = Join-Path $tmp 'silent.trr'
    Write-Utf8 $silent @"
[video]
videoFilePath=$clipPosix
startTimeOffset=0

[map]
imageFilePath=$mapPosix
relativeWidth=0.25

[hud]
scope=video
gpxFilePath=$gpxPosix

[encode]
resolution=320x240
frameRate=24
includeMap=true
includeVideo=false
includeHud=true
"@

    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'hud_scope_probe.qml'
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '120000'

    foreach ($case in @(
            @{ Name = '只导地图+HUD'; Trr = $mapOnly },
            @{ Name = '带视频'; Trr = $withVideo },
            @{ Name = '有视频但没勾视频'; Trr = $silent })) {
        $env:QMLPROBE_ORV = $case.Trr
        $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
        $code = $LASTEXITCODE
        $logs | Write-Output
        $output = $logs -join "`n"
        if ($code -ne 0 -or $output -notmatch 'HUD_SCOPE_PASS' -or $output -match 'HUD_SCOPE_FAIL') {
            throw ("HUD 显示范围回归失败（{0}，见上面的 [probe] 行）" -f $case.Name)
        }
    }

    # ---- 导出侧：只导地图+HUD，HUD 必须真的画进画面 ----
    # 素材是纯色地图，没有轨迹线，所以画面里任何"不是地图色"的像素都是 HUD 画上去的：
    # 既验证它画了，也能看出它落在哪半边（左半边=地图区，即"HUD 显示在地图上"）。
    $outHud = Join-Path $tmp 'map_only_hud.mp4'
    $encgui = Join-Path $projectRoot 'target/debug/encgui.exe'
    & $encgui $mapOnly --out $outHud --map-only --start 0 --end 2 2>&1 | Write-Output
    if (-not (Test-Path $outHud) -or (Get-Item $outHud).Length -eq 0) { throw '只导地图+HUD 的导出没有产出文件' }

    $frameA = Join-Path $tmp 'frame_hud.png'
    & $ffmpeg -y -loglevel error -ss 1 -i $outHud -frames:v 1 $frameA
    if ($LASTEXITCODE -ne 0) { throw "抽帧失败：$outHud" }
    Copy-Item -Force $frameA (Join-Path $artifacts 'map_only_hud.png')

    Add-Type -AssemblyName System.Drawing
    $a = [System.Drawing.Bitmap]::FromFile($frameA)
    try {
        $w = $a.Width; $h = $a.Height
        # 地图底色（ffmpeg color=c=0x2f6b3f），编码后会有轻微色偏，用容差比较。
        $mapR = 0x2f; $mapG = 0x6b; $mapB = 0x3f
        $hud = 0; $hudLeft = 0; $minX = $w; $minY = $h; $maxX = 0; $maxY = 0; $sampled = 0
        for ($y = 0; $y -lt $h; $y += 2) {
            for ($x = 0; $x -lt $w; $x += 2) {
                $sampled++
                $c = $a.GetPixel($x, $y)
                $d = [math]::Abs($c.R - $mapR) + [math]::Abs($c.G - $mapG) + [math]::Abs($c.B - $mapB)
                if ($d -gt 60) {
                    $hud++
                    if ($x -lt ($w / 2)) { $hudLeft++ }
                    if ($x -lt $minX) { $minX = $x }
                    if ($x -gt $maxX) { $maxX = $x }
                    if ($y -lt $minY) { $minY = $y }
                    if ($y -gt $maxY) { $maxY = $y }
                }
            }
        }
        $share = if ($sampled -gt 0) { $hud / $sampled } else { 0 }
        Write-Output ("[hud-scope] 导出帧 {0}x{1}：HUD 像素占 {2:P1}，{3:P1} 落在左半边；包围盒 x={4}..{5} y={6}..{7}" -f `
            $w, $h, $share, ($(if ($hud -gt 0) { $hudLeft / $hud } else { 0 })), $minX, $maxX, $minY, $maxY)
        if ($share -lt 0.01) { throw ("只导地图+HUD 时画面里找不到 HUD（非地图色像素只有 {0:P2}）" -f $share) }
        if ($minX -ge ($w / 2)) { throw ("HUD 只出现在右半边（x>={0}），说明它还挤在视频区里、没铺到地图上" -f $minX) }
    }
    finally {
        $a.Dispose()
    }
    Write-Output 'HUD_SCOPE_EXPORT_PASS: 只导地图+HUD 时 HUD 正常画在画面上，并且铺到了地图区（左半边）'
}
finally {
    if (-not $KeepArtifacts) { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
    else { Write-Output ("[hud-scope] 素材保留在 " + $tmp) }
    Remove-Item Env:QMLPROBE_ORV -ErrorAction SilentlyContinue
}
