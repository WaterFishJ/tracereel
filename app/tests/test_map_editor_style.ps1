# 逐框轨迹样式 + 地图框标签溢出的真实 GUI 集成回归（探针见 app/tests/map_editor_style_probe.qml）。
#
# 自己造素材：一张纯色地图 + 一个 2 框工程（第 1 框贴边固定分段、第 2 框自由跟跑者居中，
# 两个框都带逐框 style），再用 qmlprobe **无窗口**驱动真实的 app/src/bin/gui.qml，断言：
#   · 选中第 2 框调 editor.setStyle/setStyleCheck → 第 2 框 style 与取景器 appliedStyle 都变，
#     第 1 框完全不变，框数不变、不触发完整重载（debug_full_landed 不变）；
#   · 双向隔离：选中第 1 框改样式，第 2 框不变；
#   · 连续 addBox() 到 8 框后，mapFrameAdd/mapFrameRemove 仍在编辑器
#     宽度内可见可点；mapFrameTab_7 可见且点击后选中第 8 框。
#
# 用法: .\app\tests\test_map_editor_style.ps1 [-SkipBuild]
param(
    [switch]$SkipBuild,
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))

# —— 实现就绪门禁：MapEditor.qml 必须已有逐框样式编辑器（setStyle/setStyleCheck + boxStyle_ 控件
#    + 标签 Flow 换行），gui.qml 必须已删掉旧的全局「显示」组。未就绪时给出明确原因，
#    避免把「实现还在改」误报成像素/行为回归。
$editorQml = Get-Content -Raw -Encoding UTF8 (Join-Path $projectRoot 'app/src/bin/MapEditor.qml')
$guiQml = Get-Content -Raw -Encoding UTF8 (Join-Path $projectRoot 'app/src/bin/gui.qml')
$missing = @()
if ($editorQml -notmatch 'function setStyle') { $missing += 'MapEditor.qml 缺 function setStyle' }
if ($editorQml -notmatch 'function setStyleCheck') { $missing += 'MapEditor.qml 缺 function setStyleCheck' }
if ($editorQml -notmatch 'boxStyle_') { $missing += 'MapEditor.qml 缺 boxStyle_ 控件' }
if ($editorQml -notmatch 'Flow \{') { $missing += 'MapEditor.qml 缺标签 Flow 换行' }
if ($guiQml -match 'title: "显示"') { $missing += 'gui.qml 仍含旧的全局「显示」组' }
if ($missing.Count -gt 0) {
    throw ('逐框样式/标签溢出实现未就绪：' + ($missing -join '；') + '（等 survey-settings-ui 完成后再跑）')
}

if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe build failed' }
}

$artifacts = Join-Path $projectRoot 'target/map-editor-style'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
try {
    # 纯色地图（本探针验证逐框样式接线与编辑器布局，不验证路线像素）。
    Add-Type -AssemblyName System.Drawing
    $mapPath = Join-Path $artifacts 'map.png'
    $bmp = [Drawing.Bitmap]::new(400, 400)
    $graphics = [Drawing.Graphics]::FromImage($bmp)
    $graphics.Clear([Drawing.Color]::FromArgb(255, 120, 40, 20))
    $graphics.Dispose()
    $bmp.Save($mapPath, [Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $map = $mapPath -replace '\\', '/'
    $mapIni = $mapPath -replace '\\', '/'

    # 2 框工程：两框都带逐框 style（初始 renderMode=0 / width=10 / routeColor=#000000 /
    # showRunner=true）；无视频、无 HUD，只验证编辑器 ↔ 预览取景器的样式接线。
    $boxes = @(
        '{"version":1,"boxes":[',
        '{"mode":0,"share":0.25,"x":0.03,"y":0.05,"w":0.3,"h":0.6,"rotation":0,"opacity":1,"shape":0,"feather":0,"corner":0,',
        '"viewMode":0,"viewZoom":1,"lrMargin":20,"tbMargin":20,"sensitivity":0.5,',
        '"style":{"renderMode":0,"renderScope":0,"width":10,"routeColor":"#000000","showRunner":true}},',
        '{"mode":4,"x":0.55,"y":0.15,"w":0.4,"h":0.4,"rotation":0,"opacity":1,"shape":0,"feather":0,"corner":0,',
        '"viewMode":1,"viewZoom":2,"lrMargin":20,"tbMargin":20,"sensitivity":0.5,',
        '"style":{"renderMode":0,"renderScope":0,"width":10,"routeColor":"#000000","showRunner":true}}',
        ']}'
    ) -join ''
    $trr = Join-Path $artifacts 'map_editor_style.trr'
    $ini = "[map]`nimageFilePath=$mapIni`nboxes=$boxes`n`n[encode]`nincludeMap=true`nincludeVideo=false`nincludeHud=false`nresolution=640x360`n`n[window]`nwidth=1280`nheight=720`n"
    [IO.File]::WriteAllText($trr, $ini, (New-Object Text.UTF8Encoding($false)))

    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'map_editor_style_probe.qml'
    $env:QMLPROBE_ORV = $trr
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '180000'
    $logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
    $code = $LASTEXITCODE
    $logs | Set-Content -LiteralPath (Join-Path $artifacts 'qmlprobe.log')
    $logs | Write-Output
    $output = $logs -join "`n"
    if ($code -ne 0 -or $output -notmatch 'MAP_EDITOR_STYLE_PASS' -or $output -match 'MAP_EDITOR_STYLE_FAIL') {
        throw "逐框样式/标签溢出集成回归失败（见上面的 [probe] 行 / target/map-editor-style/qmlprobe.log）"
    }
    Write-Output "PASS: 逐框轨迹样式 + 8 框标签溢出集成回归（$artifacts）"
}
finally {
    Remove-Item Env:QMLPROBE_FILE -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_ORV -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_RUN_LOOP -ErrorAction SilentlyContinue
    Remove-Item Env:QMLPROBE_TIMEOUT_MS -ErrorAction SilentlyContinue
}
