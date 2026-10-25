# 多地图框预览端到端回归（探针见 app/tests/multibox_preview_probe.qml）。
#
# 自己造素材：一张纯色地图 + 一个 2 框工程（第 1 框贴边固定分段、第 2 框自由跟跑者居中），
# 再用 qmlprobe **无窗口**驱动真实的 app/src/bin/gui.qml，断言：
#   · 工程加载后 2 个地图框都在、属性与工程一致；
#   · 两框取景器互相独立（居中框 scale=zoom、固定分段框另一套）；
#   · apply_map_frames 就地改第 2 框取景缩放立即生效（不重载、不动第 1 框）；
#   · 框数变化的 JSON 被拒绝（必须走完整重载）。
#
# 用法: .\app\tests\test_multibox_preview.ps1 [-SkipBuild]
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

$artifacts = Join-Path $projectRoot 'target/multibox-preview'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
try {
    # 纯色地图（本探针验证版面/取景器接线，不验证路线像素）。
    Add-Type -AssemblyName System.Drawing
    $mapPath = Join-Path $artifacts 'map.png'
    $bmp = [Drawing.Bitmap]::new(400, 400)
    $graphics = [Drawing.Graphics]::FromImage($bmp)
    $graphics.Clear([Drawing.Color]::FromArgb(255, 120, 40, 20))
    $graphics.Dispose()
    $bmp.Save($mapPath, [Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $map = $mapPath -replace '\\', '/'

    # 2 框布局：第 1 框贴边（固定分段），第 2 框自由（跟跑者居中 zoom=2）。
    $boxes = '{"version":1,"boxes":[{"mode":0,"share":0.25,"x":0.03,"y":0.05,"w":0.3,"h":0.6,"rotation":0,"opacity":1,"shape":0,"feather":0,"corner":0,"viewMode":0,"viewZoom":1,"lrMargin":20,"tbMargin":20,"sensitivity":0.5},{"mode":4,"x":0.55,"y":0.15,"w":0.4,"h":0.4,"rotation":0,"opacity":1,"shape":0,"feather":0,"corner":0,"viewMode":1,"viewZoom":2,"lrMargin":20,"tbMargin":20,"sensitivity":0.5}]}'
    $trr = Join-Path $artifacts 'multibox.trr'
    $ini = "[map]`nimageFilePath=$map`nboxes=$boxes`n`n[encode]`nincludeMap=true`nincludeVideo=false`nincludeHud=false`nresolution=640x360`n`n[window]`nwidth=1280`nheight=720`n"
    [IO.File]::WriteAllText($trr, $ini, (New-Object Text.UTF8Encoding($false)))

    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'multibox_preview_probe.qml'
    $env:QMLPROBE_ORV = $trr
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '120000'
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | Set-Content -LiteralPath (Join-Path $artifacts 'qmlprobe.log')
    $logs | Write-Output
    $output = $logs -join "`n"
    if ($code -ne 0 -or $output -notmatch 'MULTIBOX_PREVIEW_PASS' -or $output -match 'MULTIBOX_PREVIEW_FAIL') {
        throw "多地图框预览回归失败（见上面的 [probe] 行 / target/multibox-preview/qmlprobe.log）"
    }
    Write-Output "PASS: 多地图框预览回归（$artifacts）"
}
finally {
    Remove-Item Env:QMLPROBE_FILE -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_ORV -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_RUN_LOOP -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_TIMEOUT_MS -ErrorAction SilentlyContinue
}
