# 地图布局导出的真实像素回归。
#
# 单框变体（保留改造前的 8 个）：贴边左/右、自由摆放、旋转、圆形/羽化/圆角外观、map-only，
#   工程里**没有** [map] boxes 键 —— 走「旧字段迁移成 1 个框」的兼容路径。
# 多框变体（本文件后半段）：[map] boxes=<紧凑 JSON>（结构见 design-notes/多地图框-实现方案.md
#   §3.2、§4、§8；字段 mode/share/x/y/w/h/rotation/opacity/shape/feather/corner/
#   viewMode/viewZoom/lrMargin/tbMargin/sensitivity）：
#     multi2-docked-free : 2 框，第 1 框固定左侧（固定分段取景）+ 第 2 框自由（跟跑者居中取景）
#     multi2-appearance  : 2 框都自由，不同外观（圆形+羽化 / 圆角+半透明）
#     multi3-map-only    : map-only 2 框（无视频）+ 只改框 2 取景缩放时框 1 像素不变
#     multi4-three-boxes : 3 框快速烟测
#   断言：每框矩形内必须是地图像素、框外不能出现地图色；两个不同取景方式的框画面内容
#   （平均色）必须不同；多框画布尺寸与单框基线一致；框数/几何按 §4 版面表。
#
# 末尾会跑 tst_map_layout.qml（多框编辑器交互）；需要跳过时传 -SkipQmlTest。
param([switch]$SkipBuild, [switch]$SkipQmlTest, [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin encgui
    if ($LASTEXITCODE -ne 0) { throw 'encgui build failed' }
}
$artifacts = Join-Path $projectRoot 'target/map-layout-export'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$ffmpeg = Join-Path $projectRoot 'tools/ffmpeg/bin/ffmpeg.exe'
$encgui = Join-Path $projectRoot 'target/debug/encgui.exe'
$env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
$env:QT_QPA_PLATFORM = 'offscreen'
$map = (Join-Path $artifacts 'map.png') -replace '\\', '/'
$clip = (Join-Path $artifacts 'clip.mp4') -replace '\\', '/'
$gpx = (Join-Path $artifacts 'track.gpx') -replace '\\', '/'
& $ffmpeg -y -loglevel error -f lavfi -i 'color=c=0xdc3c28:s=1000x1000' -frames:v 1 $map
if ($LASTEXITCODE -ne 0) { throw 'map generation failed' }
& $ffmpeg -y -loglevel error -f lavfi -i 'color=c=0x1e5ac8:s=320x180:r=24:d=1' -c:v libx264 -pix_fmt yuv420p $clip
if ($LASTEXITCODE -ne 0) { throw 'clip generation failed' }
[IO.File]::WriteAllText($gpx, '<gpx version="1.1"><trk><trkseg><trkpt lat="30" lon="119"><time>2026-01-01T00:00:00Z</time></trkpt><trkpt lat="30.0001" lon="119.0001"><time>2026-01-01T00:00:01Z</time></trkpt></trkseg></trk></gpx>', [Text.UTF8Encoding]::new($false))
Add-Type -AssemblyName System.Drawing

# =====================================================================================
# 采样 / 断言辅助（多框用例与单框用例共用同一套像素判据）
# =====================================================================================

# 浮点数 → JSON 数字文本（InvariantCulture，避免本地化的小数逗号；紧凑、无多余尾零）。
function Format-JsonNumber {
    param([double]$Value)
    return $Value.ToString('0.######', [Globalization.CultureInfo]::InvariantCulture)
}

# 一个地图框，字段名与 settings.rs 里 MapBox 的 serde 短名一致（Lead 约定 camelCase）。
function New-MapBox {
    param(
        [int]$Mode = 4, [double]$Share = 0.25,
        [double]$X = 0.0, [double]$Y = 0.0, [double]$W = 0.3, [double]$H = 0.3,
        [double]$Rotation = 0.0, [double]$Opacity = 1.0,
        [int]$Shape = 0, [double]$Feather = 0.0, [double]$Corner = 0.0,
        [int]$ViewMode = 0, [double]$ViewZoom = 1.0,
        [double]$LrMargin = 20.0, [double]$TbMargin = 20.0, [double]$Sensitivity = 0.5)
    return [ordered]@{
        mode = $Mode; share = $Share; x = $X; y = $Y; w = $W; h = $H
        rotation = $Rotation; opacity = $Opacity; shape = $Shape
        feather = $Feather; corner = $Corner
        viewMode = $ViewMode; viewZoom = $ViewZoom
        lrMargin = $LrMargin; tbMargin = $TbMargin; sensitivity = $Sensitivity
    }
}

# 多框 → {"version":1,"boxes":[...]} 紧凑 JSON（单行，可直接放进 INI 的 boxes= 值）。
# 注意：不要在这里用 -f 格式化多段字符串拼接（PowerShell 里 -f 与 + 的优先级容易出错），
# 逐个字段拼字符串最稳。
function ConvertTo-MapBoxesJson {
    param($Boxes)
    $items = @()
    foreach ($b in $Boxes) {
        $fields = @(
            ('"mode":' + [string][int]$b.mode),
            ('"share":' + (Format-JsonNumber ([double]$b.share))),
            ('"x":' + (Format-JsonNumber ([double]$b.x))),
            ('"y":' + (Format-JsonNumber ([double]$b.y))),
            ('"w":' + (Format-JsonNumber ([double]$b.w))),
            ('"h":' + (Format-JsonNumber ([double]$b.h))),
            ('"rotation":' + (Format-JsonNumber ([double]$b.rotation))),
            ('"opacity":' + (Format-JsonNumber ([double]$b.opacity))),
            ('"shape":' + [string][int]$b.shape),
            ('"feather":' + (Format-JsonNumber ([double]$b.feather))),
            ('"corner":' + (Format-JsonNumber ([double]$b.corner))),
            ('"viewMode":' + [string][int]$b.viewMode),
            ('"viewZoom":' + (Format-JsonNumber ([double]$b.viewZoom))),
            ('"lrMargin":' + (Format-JsonNumber ([double]$b.lrMargin))),
            ('"tbMargin":' + (Format-JsonNumber ([double]$b.tbMargin))),
            ('"sensitivity":' + (Format-JsonNumber ([double]$b.sensitivity)))
        )
        $items += ('{' + ($fields -join ',') + '}')
    }
    return '{"version":1,"boxes":[' + ($items -join ',') + ']}'
}

# 写一个地图布局测试工程。框数 > 1 时才写 [map] boxes；旧键照写第 1 框镜像
# （与 settings.rs 的保存规则一致：单框工程逐字节保持旧格式）。
function Write-MapBoxProject {
    param(
        [string]$Path, [string]$MapImage, [string]$VideoPath, [string]$GpxPath,
        $Boxes, [bool]$IncludeVideo = $true, [string]$Resolution = '320x180')
    $b0 = $Boxes[0]
    $lines = @()
    $lines += '[video]'
    $lines += "videoFilePath=$VideoPath"
    $lines += '[map]'
    $lines += "imageFilePath=$MapImage"
    $lines += "relativeWidth=$(Format-JsonNumber ([double]$b0.share))"
    $lines += "placement=$([int]$b0.mode)"
    $lines += "frameX=$(Format-JsonNumber ([double]$b0.x))"
    $lines += "frameY=$(Format-JsonNumber ([double]$b0.y))"
    $lines += "frameWidth=$(Format-JsonNumber ([double]$b0.w))"
    $lines += "frameHeight=$(Format-JsonNumber ([double]$b0.h))"
    $lines += "frameRotation=$(Format-JsonNumber ([double]$b0.rotation))"
    $lines += "frameShape=$([int]$b0.shape)"
    $lines += "frameOpacity=$(Format-JsonNumber ([double]$b0.opacity))"
    $lines += "frameFeather=$(Format-JsonNumber ([double]$b0.feather))"
    $lines += "frameCorner=$(Format-JsonNumber ([double]$b0.corner))"
    if ($Boxes.Count -gt 1) {
        $lines += ('boxes=' + (ConvertTo-MapBoxesJson $Boxes))
    }
    $lines += '[hud]'
    $lines += "gpxFilePath=$GpxPath"
    $lines += '[gyro]'
    $lines += 'enabled=false'
    $lines += '[encode]'
    $lines += "resolution=$Resolution"
    $lines += 'frameRate=24'
    $lines += 'includeMap=true'
    $lines += ('includeVideo=' + $IncludeVideo.ToString().ToLowerInvariant())
    $lines += 'includeHud=false'
    $lines += 'backend=software'
    $lines += 'audioCodec=none'
    [IO.File]::WriteAllText($Path, (($lines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
}

# 真实导出 3 帧并抽第 1 帧为 PNG，返回 PNG 路径（失败时把 encgui 日志路径写进异常）。
function Invoke-EncguiExport {
    param([string]$Name, [string]$ProjectPath, [switch]$MapOnly)
    $out = Join-Path $artifacts "$Name.mp4"
    $cliArgs = @($ProjectPath, '--out', $out, '--end', '0.125', '--max-frames', '3')
    if ($MapOnly) { $cliArgs += '--map-only' }
    $logs = & $encgui @cliArgs 2>&1
    $code = $LASTEXITCODE
    $logs | Set-Content -LiteralPath (Join-Path $artifacts "$Name.log")
    if ($code -ne 0 -or (($logs -join "`n") -match '(?m)^ERROR ')) {
        throw "导出失败: $Name（退出码 $code，日志 $artifacts\$Name.log）"
    }
    $png = Join-Path $artifacts "$Name.png"
    & $ffmpeg -y -loglevel error -i $out -frames:v 1 $png
    if ($LASTEXITCODE -ne 0) { throw "抽帧失败: $Name（$out）" }
    return $png
}

function Get-FrameBitmap {
    param([string]$PngPath)
    return [Drawing.Bitmap]::new($PngPath)
}

function Get-PixelAt {
    param($Bitmap, [int]$X, [int]$Y)
    $x = [Math]::Max(0, [Math]::Min($Bitmap.Width - 1, $X))
    $y = [Math]::Max(0, [Math]::Min($Bitmap.Height - 1, $Y))
    return $Bitmap.GetPixel($x, $y)
}

function Format-Pixel {
    param($Pixel)
    return ('({0},{1},{2})' -f $Pixel.R, $Pixel.G, $Pixel.B)
}

# 地图色：纯红地图 0xdc3c28，以及环状图案地图的两圈颜色（(235,40,40) 红 / (235,210,40) 黄）。
# 视频蓝 0x1e5ac8=(30,90,200) 故意**不**算地图色 —— 框外露出视频不算越界。
# 图案两圈只在 G 通道差别大、R/B 都保持"R 高 B 低"，所以 4:2:0 边界混色仍然是地图色。
function Test-MapColor {
    param($Pixel)
    $redBand = ($Pixel.R -gt 150) -and ($Pixel.B -lt 110)
    $blueBand = ($Pixel.B -gt 180) -and ($Pixel.G -lt 75)
    return ($redBand -or $blueBand)
}

# 视频蓝（H.264 4:2:0 解码后允许小幅漂移）。
function Test-VideoBlue {
    param($Pixel)
    return ($Pixel.B -gt 150) -and ($Pixel.G -gt 55) -and ($Pixel.R -lt 90)
}

function Test-BackgroundColor {
    param($Pixel)
    $black = ($Pixel.R -lt 70) -and ($Pixel.G -lt 70) -and ($Pixel.B -lt 70)
    return ($black -or (Test-VideoBlue $Pixel))
}

function Assert-PixelMap {
    param($Bitmap, [int]$X, [int]$Y, [string]$What)
    $p = Get-PixelAt $Bitmap $X $Y
    if (-not (Test-MapColor $p)) {
        throw ("{0}: ({1},{2}) 应为地图像素，实际 RGB={3}" -f $What, $X, $Y, (Format-Pixel $p))
    }
}

function Assert-PixelVideoBlue {
    param($Bitmap, [int]$X, [int]$Y, [string]$What)
    $p = Get-PixelAt $Bitmap $X $Y
    if (-not (Test-VideoBlue $p)) {
        throw ("{0}: ({1},{2}) 应为视频蓝（框外露出视频），实际 RGB={3}" -f $What, $X, $Y, (Format-Pixel $p))
    }
}

function Assert-CanvasSize {
    param($Bitmap, [int]$ExpectedW, [int]$ExpectedH, [string]$What)
    if ($Bitmap.Width -ne $ExpectedW -or $Bitmap.Height -ne $ExpectedH) {
        throw ("{0}: 画布 {1}x{2}，期望 {3}x{4}" -f $What, $Bitmap.Width, $Bitmap.Height, $ExpectedW, $ExpectedH)
    }
}

# 点是否在（旋转 theta 度、绕自身中心的）矩形内；Expand 为向外扩张的像素数（负数=内缩）。
function Test-InsideBox {
    param([double]$X, [double]$Y, $Box, [double]$Expand = 0.0)
    $cx = $Box.x + $Box.w / 2.0
    $cy = $Box.y + $Box.h / 2.0
    $rad = -$Box.rotation * [Math]::PI / 180.0
    $cos = [Math]::Cos($rad)
    $sin = [Math]::Sin($rad)
    $dx = $X - $cx
    $dy = $Y - $cy
    # 画布 → 框本地：绕中心反向旋转（绘制时是 translate(中心)+rotate(theta)）。
    $lx = $dx * $cos - $dy * $sin
    $ly = $dx * $sin + $dy * $cos
    return ([Math]::Abs($lx) -le ($Box.w / 2.0 + $Expand)) -and ([Math]::Abs($ly) -le ($Box.h / 2.0 + $Expand))
}

# 按 §4 版面表把 box 配置换算成画布像素矩形（只处理不透明矩形框；圆形外观另做针对性断言）。
#   - 第 1 框 mode 0/1 且勾了视频：贴边条带；
#   - 第 1 框 mode 0/1 且无视频：整幅画面；
#   - 其余（含所有第 2+ 框）：自由摆放的归一化矩形 × 画布。
function Get-EffectiveBoxRect {
    param($Box, [int]$Index, [int]$CanvasW, [int]$CanvasH, [bool]$VideoOn)
    if ($Index -eq 0 -and [int]$Box.mode -ne 4) {
        if (-not $VideoOn) {
            return [pscustomobject]@{ x = 0.0; y = 0.0; w = [double]$CanvasW; h = [double]$CanvasH; rotation = 0.0; circle = $false }
        }
        $stripW = [double]$Box.share * $CanvasW
        $x = 0.0
        if ([int]$Box.mode -eq 1) { $x = $CanvasW - $stripW }
        return [pscustomobject]@{ x = $x; y = 0.0; w = $stripW; h = [double]$CanvasH; rotation = 0.0; circle = $false }
    }
    return [pscustomobject]@{
        x = [double]$Box.x * $CanvasW; y = [double]$Box.y * $CanvasH
        w = [double]$Box.w * $CanvasW; h = [double]$Box.h * $CanvasH
        rotation = [double]$Box.rotation; circle = ([int]$Box.shape -eq 1)
    }
}

function Format-BoxRects {
    param($Rects)
    $parts = @()
    for ($i = 0; $i -lt $Rects.Count; $i++) {
        $r = $Rects[$i]
        $parts += ('框{0}=({1:0.#},{2:0.#},{3:0.#},{4:0.#},rot={5:0.#})' -f $i, $r.x, $r.y, $r.w, $r.h, $r.rotation)
    }
    return ($parts -join ' ')
}

# 不透明矩形多框的整幅扫描：框内必须是地图色，框外不能出现地图色。
# 框边 3px 内、以及框外 5px 内跳过（H.264 4:2:0 色度渗色/抗锯齿）。
# 返回每框的有效矩形（像素坐标），供后续几何/内容断言复用。
function Assert-OpaqueCoverage {
    param($Bitmap, $Boxes, [int]$CanvasW, [int]$CanvasH, [bool]$VideoOn, [string]$What)
    $rects = @()
    for ($i = 0; $i -lt $Boxes.Count; $i++) {
        $rects += Get-EffectiveBoxRect -Box $Boxes[$i] -Index $i -CanvasW $CanvasW -CanvasH $CanvasH -VideoOn $VideoOn
    }
    foreach ($r in $rects) {
        if ($r.circle) { throw "Assert-OpaqueCoverage 只用于矩形框：$What" }
    }
    $checked = 0
    $bad = 0
    for ($y = 1; $y -lt $CanvasH - 1; $y += 2) {
        for ($x = 1; $x -lt $CanvasW - 1; $x += 2) {
            $inside = $false
            foreach ($r in $rects) {
                if (Test-InsideBox -X $x -Y $y -Box $r -Expand (-3.0)) { $inside = $true; break }
            }
            $p = $Bitmap.GetPixel($x, $y)
            if ($inside) {
                $checked++
                if (-not (Test-MapColor $p)) {
                    $bad++
                    if ($bad -le 8) { Write-Output ("  [{0}] 框内非地图色 @({1},{2}) RGB={3}" -f $What, $x, $y, (Format-Pixel $p)) }
                }
                continue
            }
            $near = $false
            foreach ($r in $rects) {
                if (Test-InsideBox -X $x -Y $y -Box $r -Expand 5.0) { $near = $true; break }
            }
            if ($near) { continue }
            if (Test-MapColor $p) {
                throw ("{0}: 框外出现地图色 @({1},{2}) RGB={3}（{4}）" -f $What, $x, $y, (Format-Pixel $p), (Format-BoxRects $rects))
            }
        }
    }
    # 允许 2% 的采样点落在图案色环边界/编码渗色上（真实少画一整个框会远超这个比例）。
    $allowed = [Math]::Max(3, [int]($checked * 0.02))
    if ($bad -gt $allowed) {
        throw ("{0}: 框内地图覆盖率不足（{1}/{2} 个采样点不是地图色，允许 {3}）" -f $What, $bad, $checked, $allowed)
    }
    return ,$rects
}

function Assert-RectNear {
    param($Rect, [double]$X, [double]$Y, [double]$W, [double]$H, [string]$What, [double]$Tolerance = 1.5)
    $detail = ("期望 ({0:0.#},{1:0.#},{2:0.#},{3:0.#})，实际 ({4:0.#},{5:0.#},{6:0.#},{7:0.#})" -f `
        $X, $Y, $W, $H, $Rect.x, $Rect.y, $Rect.w, $Rect.h)
    if ([Math]::Abs($Rect.x - $X) -gt $Tolerance -or [Math]::Abs($Rect.y - $Y) -gt $Tolerance -or
        [Math]::Abs($Rect.w - $W) -gt $Tolerance -or [Math]::Abs($Rect.h - $H) -gt $Tolerance) {
        throw ("{0}: 矩形位置/尺寸不对（{1}）" -f $What, $detail)
    }
}

# 框内缩进后的采样区域（docked 左/上取整；用于平均色/跨导出比较）。
function Get-BoxInterior {
    param($Box, [double]$InsetRatio)
    return [pscustomobject]@{
        x = $Box.x + $Box.w * $InsetRatio
        y = $Box.y + $Box.h * $InsetRatio
        w = $Box.w * (1.0 - 2.0 * $InsetRatio)
        h = $Box.h * (1.0 - 2.0 * $InsetRatio)
    }
}

function Get-RegionAverageColor {
    param($Bitmap, $Region, [int]$Step = 2)
    $x0 = [Math]::Max(0, [int][Math]::Round($Region.x))
    $y0 = [Math]::Max(0, [int][Math]::Round($Region.y))
    $x1 = [Math]::Min($Bitmap.Width - 1, [int][Math]::Round($Region.x + $Region.w) - 1)
    $y1 = [Math]::Min($Bitmap.Height - 1, [int][Math]::Round($Region.y + $Region.h) - 1)
    if ($x1 -lt $x0 -or $y1 -lt $y0) { throw ("平均色采样区域为空: ({0:0.#},{1:0.#},{2:0.#},{3:0.#})" -f $Region.x, $Region.y, $Region.w, $Region.h) }
    $sumR = 0.0; $sumG = 0.0; $sumB = 0.0; $n = 0
    for ($y = $y0; $y -le $y1; $y += $Step) {
        for ($x = $x0; $x -le $x1; $x += $Step) {
            $p = $Bitmap.GetPixel($x, $y)
            $sumR += $p.R; $sumG += $p.G; $sumB += $p.B; $n++
        }
    }
    return @(($sumR / $n), ($sumG / $n), ($sumB / $n))
}

function Assert-AverageColorDiffers {
    param($AvgA, $AvgB, [string]$What, [double]$MinDiff = 40.0)
    $diff = [Math]::Max([Math]::Abs($AvgA[0] - $AvgB[0]), [Math]::Max([Math]::Abs($AvgA[1] - $AvgB[1]), [Math]::Abs($AvgA[2] - $AvgB[2])))
    if ($diff -lt $MinDiff) {
        throw ("{0}: 两块区域平均色太接近（最大通道差 {1:0.0}，要求 ≥ {2:0.0}；A=({3:0},{4:0},{5:0}) B=({6:0},{7:0},{8:0})）" -f `
            $What, $diff, $MinDiff, $AvgA[0], $AvgA[1], $AvgA[2], $AvgB[0], $AvgB[1], $AvgB[2])
    }
}

# 两次导出的同一区域应基本一致（容差内；允许少量编码噪声/边缘像素）。
function Assert-RegionSimilar {
    param($BitmapA, $BitmapB, $Region, [string]$What, [int]$Tolerance = 12, [double]$MinRatio = 0.95)
    if ($BitmapA.Width -ne $BitmapB.Width -or $BitmapA.Height -ne $BitmapB.Height) {
        throw ("{0}: 两次导出画布尺寸不同 {1}x{2} vs {3}x{4}" -f $What, $BitmapA.Width, $BitmapA.Height, $BitmapB.Width, $BitmapB.Height)
    }
    $x0 = [Math]::Max(0, [int][Math]::Round($Region.x))
    $y0 = [Math]::Max(0, [int][Math]::Round($Region.y))
    $x1 = [Math]::Min($BitmapA.Width - 1, [int][Math]::Round($Region.x + $Region.w) - 1)
    $y1 = [Math]::Min($BitmapA.Height - 1, [int][Math]::Round($Region.y + $Region.h) - 1)
    $total = 0
    $bad = 0
    for ($y = $y0; $y -le $y1; $y += 2) {
        for ($x = $x0; $x -le $x1; $x += 2) {
            $a = $BitmapA.GetPixel($x, $y)
            $b = $BitmapB.GetPixel($x, $y)
            $total++
            if ([Math]::Abs($a.R - $b.R) -gt $Tolerance -or
                [Math]::Abs($a.G - $b.G) -gt $Tolerance -or
                [Math]::Abs($a.B - $b.B) -gt $Tolerance) { $bad++ }
        }
    }
    $allowed = [int][Math]::Floor($total * (1.0 - $MinRatio))
    if ($bad -gt $allowed) {
        throw ("{0}: 该区域在两次导出之间变化过大（{1}/{2} 个采样点超出容差 {3}，允许 {4}）" -f $What, $bad, $total, $Tolerance, $allowed)
    }
}

# 两次导出的同一区域平均色应有明显差异（用于「只改框 2 的取景 → 框 2 画面必须变」）。
function Assert-RegionDiffers {
    param($BitmapA, $BitmapB, $Region, [string]$What, [double]$MinMeanDiff = 18.0)
    $avgA = Get-RegionAverageColor -Bitmap $BitmapA -Region $Region
    $avgB = Get-RegionAverageColor -Bitmap $BitmapB -Region $Region
    $mean = ([Math]::Abs($avgA[0] - $avgB[0]) + [Math]::Abs($avgA[1] - $avgB[1]) + [Math]::Abs($avgA[2] - $avgB[2])) / 3.0
    if ($mean -lt $MinMeanDiff) {
        throw ("{0}: 两次导出该区域平均色差异过小（平均差 {1:0.0}，要求 ≥ {2:0.0}；A=({3:0},{4:0},{5:0}) B=({6:0},{7:0},{8:0})）" -f `
            $What, $mean, $MinMeanDiff, $avgA[0], $avgA[1], $avgA[2], $avgB[0], $avgB[1], $avgB[2])
    }
}

# 环状色环图案地图：一帧 256x256，之后用 ffmpeg 放大到 1024（边界在 60 地图像素）。
#   第 0 圈 (d<15px) = 红 (235,40,40)，第 1 圈 = 黄 (235,210,40)，如此交替。
#   两圈只在 G 通道差别大（R/B 都保持"地图色"：R 高、B 低），这样：
#     - 地图色判据（R>150 且 B<110）对两圈和 4:2:0 边界混色都成立 → 框内覆盖扫描不会误报；
#     - 不同缩放倍数下"中心窗口跨过的圈数"不同 → 平均色的 G 通道能可靠区分取景差异。
#   地图中心永远对准每个框的中心，所以只改某框缩放也不会影响其它框。
function New-RingPatternMap {
    param([string]$Path, [int]$Size = 256, [double]$Period = 15.0)
    $bmp = [Drawing.Bitmap]::new($Size, $Size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $rect = [Drawing.Rectangle]::new(0, 0, $Size, $Size)
        $data = $bmp.LockBits($rect, [Drawing.Imaging.ImageLockMode]::WriteOnly, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $stride = $data.Stride
            $buffer = [byte[]]::new($stride * $Size)
            $center = $Size / 2.0
            for ($y = 0; $y -lt $Size; $y++) {
                $dy = ($y + 0.5) - $center
                $row = $y * $stride
                for ($x = 0; $x -lt $Size; $x++) {
                    $dx = ($x + 0.5) - $center
                    $distance = [Math]::Sqrt($dx * $dx + $dy * $dy)
                    $band = [int]([Math]::Floor($distance / $Period)) % 2
                    $i = $row + $x * 4
                    # Format32bppArgb 在内存里是 B,G,R,A 顺序（小端 ARGB）。
                    if ($band -eq 0) {
                        $buffer[$i] = 40; $buffer[$i + 1] = 40; $buffer[$i + 2] = 235
                    } else {
                        $buffer[$i] = 40; $buffer[$i + 1] = 210; $buffer[$i + 2] = 235
                    }
                    $buffer[$i + 3] = 255
                }
            }
            [System.Runtime.InteropServices.Marshal]::Copy($buffer, 0, $data.Scan0, $buffer.Length)
        } finally {
            $bmp.UnlockBits($data)
        }
        $bmp.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $bmp.Dispose()
    }
}

# =====================================================================================
# 1) 单框变体：旧字段迁移路径（改造前行为，保留原用例与像素判据）
# =====================================================================================
foreach ($variant in @(
    @{ Mode=0; Angle=0; Video=$true }, @{ Mode=1; Angle=0; Video=$true },
    @{ Mode=4; Angle=0; Video=$true }, @{ Mode=4; Angle=33; Video=$true },
    @{ Mode=4; Angle=-45; Video=$false },
    @{ Mode=4; Angle=0; Video=$true; Shape=1; Opacity=1; Feather=0; Corner=0 },
    @{ Mode=4; Angle=31; Video=$true; Shape=0; Opacity=.5; Feather=.1; Corner=.25 },
    @{ Mode=4; Angle=0; Video=$true; Shape=1; Opacity=.5; Feather=.1; Corner=0 })) {
    $mode = $variant.Mode
    $angle = $variant.Angle
    $video = $variant.Video.ToString().ToLowerInvariant()
    $name = "map-$mode-$angle-$video"
    $styled = $variant.ContainsKey('Shape')
    $shape = if ($styled) { $variant.Shape } else { 0 }
    $opacity = if ($styled) { $variant.Opacity } else { 1 }
    $feather = if ($styled) { $variant.Feather } else { 0 }
    $corner = if ($styled) { $variant.Corner } else { 0 }
    if ($styled) { $name += "-style-$shape-$opacity-$feather-$corner" }
    $trr = Join-Path $artifacts "$name.trr"
    [IO.File]::WriteAllText($trr, @"
[video]
videoFilePath=$clip
[map]
imageFilePath=$map
relativeWidth=0.25
placement=$mode
frameX=0.2
frameY=0.2
frameWidth=0.35
frameHeight=0.55
frameRotation=$angle
frameShape=$shape
frameOpacity=$opacity
frameFeather=$feather
frameCorner=$corner
[hud]
gpxFilePath=$gpx
[gyro]
enabled=false
[encode]
resolution=320x180
frameRate=24
includeMap=true
includeVideo=$video
includeHud=false
backend=software
audioCodec=none
"@, [Text.UTF8Encoding]::new($false))
    $out = Join-Path $artifacts "$name.mp4"
    $arguments = @($trr, '--out', $out, '--end', '0.125', '--max-frames', '3')
    if (-not $variant.Video) { $arguments += '--map-only' }
    $logs = & $encgui @arguments 2>&1
    $code = $LASTEXITCODE
    $logs | Set-Content (Join-Path $artifacts "$name.log")
    if ($code -ne 0 -or ($logs -join "`n") -match '(?m)^ERROR ') { throw "export failed: $name (see artifacts log)" }
    $png = Join-Path $artifacts "$name.png"
    & $ffmpeg -y -loglevel error -i $out -frames:v 1 $png
    if ($LASTEXITCODE -ne 0) { throw "frame decode failed: $name" }
    $rect = switch ($mode) {
        0 { @(0, 0, 80, 180) }
        1 { @(240, 0, 80, 180) }
        2 { @(0, 0, 320, 45) }
        3 { @(0, 135, 320, 45) }
        4 { @(64, 36, 112, 99) }
    }
    $radians = $angle * [Math]::PI / 180
    $bitmap = [Drawing.Bitmap]::new($png)
    $insideCount = 0
    $colorError = 0.0
    $colorSamples = 0
    try {
        for ($y=0; $y -lt 180; $y+=2) {
            for ($x=0; $x -lt 320; $x+=2) {
                $dx = $x+0.5-$rect[0]-$rect[2]/2
                $dy = $y+0.5-$rect[1]-$rect[3]/2
                $lx = [Math]::Abs($dx*[Math]::Cos($radians)+$dy*[Math]::Sin($radians))
                $ly = [Math]::Abs(-$dx*[Math]::Sin($radians)+$dy*[Math]::Cos($radians))
                $pixel = $bitmap.GetPixel($x,$y)
                if ($styled) {
                    $short = [Math]::Min($rect[2],$rect[3])
                    if ($shape -eq 1) {
                        $distance = $short/2-[Math]::Sqrt($lx*$lx+$ly*$ly)
                    } else {
                        $radius = $corner*$short
                        $qx = $lx-($rect[2]/2-$radius); $qy = $ly-($rect[3]/2-$radius)
                        $mx = [Math]::Max($qx,0); $my = [Math]::Max($qy,0)
                        $distance = $radius-[Math]::Sqrt($mx*$mx+$my*$my)-[Math]::Min([Math]::Max($qx,$qy),0)
                    }
                    if ($feather -gt 0) {
                        $t = [Math]::Max(0.0,[Math]::Min(1.0,$distance/($feather*$short)))
                        $alpha = $opacity*$t*$t*(3-2*$t)
                    } else { $alpha = if ($distance -gt 0) { $opacity } else { 0 } }
                    if ($feather -gt 0 -or [Math]::Abs($distance) -gt 4) {
                        $expected = @((30+190*$alpha),(90-30*$alpha),(200-160*$alpha))
                        $actual = @($pixel.R,$pixel.G,$pixel.B)
                        for ($channel=0; $channel -lt 3; $channel++) {
                            # H.264 4:2:0 interpolates chroma along the feathered edge.
                            # The unencoded painter checks use a stricter 3-level tolerance.
                            $error = [Math]::Abs($actual[$channel]-$expected[$channel])
                            $colorError += $error; $colorSamples++
                            if ($error -gt 30) { throw "mask/opacity mismatch: $name at $x,$y channel=$channel" }
                        }
                    }
                    if ($distance -gt 6) { $insideCount++ }
                    continue
                }
                $red = $pixel.R -gt 150 -and $pixel.G -lt 100 -and $pixel.B -lt 100
                if ($lx -lt $rect[2]/2-4 -and $ly -lt $rect[3]/2-4) {
                    if (-not $red) { throw "map missing: $name at $x,$y" }
                    $insideCount++
                }
                if ($lx -gt $rect[2]/2+4 -or $ly -gt $rect[3]/2+4) {
                    if ($red) { throw "map outside frame: $name at $x,$y" }
                }
            }
        }
        if ($insideCount -lt 1000) { throw "too few map pixels: $name" }
        if ($colorSamples -gt 0 -and $colorError/$colorSamples -gt 6) { throw "average mask/opacity error too large: $name" }
    } finally { $bitmap.Dispose() }
    Write-Output "PASS: $name"
}
Write-Output "Map placement export checks passed: $artifacts"

# =====================================================================================
# 2) 多地图框变体（[map] boxes=<紧凑 JSON>）
# =====================================================================================
$canvasW = 320
$canvasH = 180

# 2.0 环状色环图案地图（1024x1024）：用于「不同取景方式 → 画面内容不同」与
#     「只改框 2 的取景缩放 → 框 1 像素不变」两条逐框独立断言。
$patternSmall = Join-Path $artifacts 'map-pattern-256.png'
$mapPattern = Join-Path $artifacts 'map-pattern.png'
New-RingPatternMap -Path $patternSmall
& $ffmpeg -y -loglevel error -i $patternSmall -vf 'scale=1024:1024:flags=neighbor' -frames:v 1 $mapPattern
if ($LASTEXITCODE -ne 0) { throw '图案地图放大失败' }
$mapPatternIni = ($mapPattern -replace '\\', '/')

# ---- 2.1 两个框：第 1 框固定左侧（固定分段取景）+ 第 2 框自由摆放（跟跑者居中取景）----
# 期望版面（§4 表：多框 + 第 1 框贴边、有视频）：
#   框 1 = 左侧条带 80x180（share=0.25），视频区 = 右侧 240x180（上下黑边）；
#   框 2 = 自由矩形 (144,36,112,90)，叠加在视频区上。
$boxesDockedFree = @(
    (New-MapBox -Mode 0 -Share 0.25 -ViewMode 0),
    (New-MapBox -Mode 4 -X 0.45 -Y 0.20 -W 0.35 -H 0.50 -ViewMode 1 -ViewZoom 2.0)
)
$trrDockedFree = Join-Path $artifacts 'map-multi2-docked-free.trr'
Write-MapBoxProject -Path $trrDockedFree -MapImage $mapPatternIni -VideoPath $clip -GpxPath $gpx -Boxes $boxesDockedFree -IncludeVideo $true
# 单框基线（旧字段迁移路径、无 boxes 键）：用于「多框画布尺寸与单框一致」和「加第 2 个框不改变第 1 个框」。
$trrBaseline = Join-Path $artifacts 'map-multi2-baseline-single.trr'
Write-MapBoxProject -Path $trrBaseline -MapImage $mapPatternIni -VideoPath $clip -GpxPath $gpx -Boxes @($boxesDockedFree[0]) -IncludeVideo $true
$pngMulti = Invoke-EncguiExport -Name 'map-multi2-docked-free' -ProjectPath $trrDockedFree
$pngBaseline = Invoke-EncguiExport -Name 'map-multi2-baseline-single' -ProjectPath $trrBaseline
$bmpMulti = Get-FrameBitmap $pngMulti
$bmpBaseline = Get-FrameBitmap $pngBaseline
try {
    Assert-CanvasSize -Bitmap $bmpMulti -ExpectedW $canvasW -ExpectedH $canvasH -What '多框：贴边+自由'
    Assert-CanvasSize -Bitmap $bmpBaseline -ExpectedW $canvasW -ExpectedH $canvasH -What '单框基线'
    if ($bmpMulti.Width -ne $bmpBaseline.Width -or $bmpMulti.Height -ne $bmpBaseline.Height) {
        throw ("多框导出的画布尺寸与单框不一致：多框 {0}x{1}，单框 {2}x{3}" -f $bmpMulti.Width, $bmpMulti.Height, $bmpBaseline.Width, $bmpBaseline.Height)
    }
    $rects = Assert-OpaqueCoverage -Bitmap $bmpMulti -Boxes $boxesDockedFree -CanvasW $canvasW -CanvasH $canvasH -VideoOn $true -What '多框：贴边+自由'
    if ($rects.Count -ne 2) { throw "多框：应有 2 个框，实际 $($rects.Count)" }
    Assert-RectNear -Rect $rects[0] -X 0 -Y 0 -W 80 -H 180 -What '框 1（固定左侧 25%）'
    Assert-RectNear -Rect $rects[1] -X 144 -Y 36 -W 112 -H 90 -What '框 2（自由 45%,20%,35%,50%）'
    # 框外右侧中部仍是视频蓝（贴边框只挤占左侧条带，多框不会把视频区算错）。
    Assert-PixelVideoBlue -Bitmap $bmpMulti -X 300 -Y 90 -What '框 1 外的视频区（x=300,y=90）'
    # 两个框取景方式/缩放不同 → 画面内容（平均色）必须明显不同。
    $avgBox1 = Get-RegionAverageColor -Bitmap $bmpMulti -Region (Get-BoxInterior -Box $rects[0] -InsetRatio 0.10)
    $avgBox2 = Get-RegionAverageColor -Bitmap $bmpMulti -Region (Get-BoxInterior -Box $rects[1] -InsetRatio 0.20)
    Assert-AverageColorDiffers -AvgA $avgBox1 -AvgB $avgBox2 -What '多框：固定分段框 vs 跟跑者居中框（内容应不同）' -MinDiff 40
    # 加第 2 个框不改变第 1 个框：与单框基线逐像素比较（只允许编码噪声；H.264 码率控制
    # 会因整帧内容不同而轻微改变量化，故容差放宽到 24、要求 90% 采样点一致 —— 真实少画/错位
    # 会让整块区域变成视频蓝，差异远大于这个容差）。
    Assert-RegionSimilar -BitmapA $bmpBaseline -BitmapB $bmpMulti -Region (Get-BoxInterior -Box $rects[0] -InsetRatio 0.04) -What '框 1（单框基线 vs 多框工程）' -Tolerance 24 -MinRatio 0.90
    $script:singleBoxCanvasW = $bmpBaseline.Width
    $script:singleBoxCanvasH = $bmpBaseline.Height
    Write-Output ('PASS: multi2-docked-free（框1 80x180 固定分段 / 框2 112x90 跟跑者居中；平均色 框1=({0:0},{1:0},{2:0}) 框2=({3:0},{4:0},{5:0})）' -f `
        $avgBox1[0], $avgBox1[1], $avgBox1[2], $avgBox2[0], $avgBox2[1], $avgBox2[2])
} finally {
    $bmpMulti.Dispose()
    $bmpBaseline.Dispose()
}

# ---- 2.2 两个框都自由、不同外观：框 1 圆形+羽化+0.9 不透明，框 2 圆角+0.5 不透明 ----
# 期望：圆心 = 红×0.9 + 视频蓝×0.1 ≈ (201,63,56)；框 2 圆心 = 红/蓝各半 ≈ (125,75,120)；
# 圆外/圆角外/框外必须透出视频蓝（遮罩逐框生效、互不影响）。
$boxesAppearance = @(
    (New-MapBox -Mode 4 -X 0.05 -Y 0.08 -W 0.38 -H 0.56 -Shape 1 -Feather 0.08 -Opacity 0.9),
    (New-MapBox -Mode 4 -X 0.50 -Y 0.50 -W 0.40 -H 0.40 -Shape 0 -Corner 0.25 -Opacity 0.5)
)
$trrAppearance = Join-Path $artifacts 'map-multi2-appearance.trr'
Write-MapBoxProject -Path $trrAppearance -MapImage $map -VideoPath $clip -GpxPath $gpx -Boxes $boxesAppearance -IncludeVideo $true
$pngAppearance = Invoke-EncguiExport -Name 'map-multi2-appearance' -ProjectPath $trrAppearance
$bmpAppearance = Get-FrameBitmap $pngAppearance
try {
    Assert-CanvasSize -Bitmap $bmpAppearance -ExpectedW $script:singleBoxCanvasW -ExpectedH $script:singleBoxCanvasH -What '多框：外观（应与单框画布一致）'
    # 框 1（自由 16%,8%,38%,56% → 像素 16,14.4,121.6,100.8）圆心 (77,65)。
    $center1 = Get-PixelAt -Bitmap $bmpAppearance -X 77 -Y 65
    if (-not ($center1.R -gt 170 -and ($center1.R - $center1.G) -gt 90 -and ($center1.R - $center1.B) -gt 90)) {
        throw ("多框外观：框 1 圆心应为地图红×0.9（≈(201,63,56)），实际 RGB={0}" -f (Format-Pixel $center1))
    }
    # 框 1 矩形左上角在圆外 → 透出视频。
    Assert-PixelVideoBlue -Bitmap $bmpAppearance -X 21 -Y 19 -What '多框外观：框 1 圆外角落 (21,19)'
    # 框 2（自由 50%,50%,40%,40% → 像素 160,90,128,72）圆心 (224,126)：0.5 红叠 0.5 蓝。
    $center2 = Get-PixelAt -Bitmap $bmpAppearance -X 224 -Y 126
    if ($center2.R -lt 105 -or $center2.R -gt 150 -or $center2.G -lt 55 -or $center2.G -gt 100 -or $center2.B -lt 95 -or $center2.B -gt 145) {
        throw ("多框外观：框 2 圆心应为红/蓝各半（≈(125,75,120)），实际 RGB={0}（不透明度没按框生效？）" -f (Format-Pixel $center2))
    }
    if ($center2.R -gt 180) { throw ("多框外观：框 2 圆心 R={0} 说明 0.5 不透明度没生效" -f $center2.R) }
    # 框 2 左上圆角外（圆角半径 = 0.25×min(128,72)=18）→ 视频蓝；(162,92) 距圆角圆心约 22.6px，
    # 在圆角外 4px 以上，避开 4:2:0 色度渗色。
    Assert-PixelVideoBlue -Bitmap $bmpAppearance -X 162 -Y 92 -What '多框外观：框 2 圆角外角落 (162,92)'
    # 框外采样点都应是视频区（画布无黑边：视频与输出同为 16:9）。
    foreach ($pt in @(@(5, 175), @(310, 10), @(5, 90))) {
        Assert-PixelVideoBlue -Bitmap $bmpAppearance -X $pt[0] -Y $pt[1] -What ("多框外观：框外背景 ({0},{1})" -f $pt[0], $pt[1])
    }
    Write-Output 'PASS: multi2-appearance（圆形+羽化 0.9 / 圆角+半透明 0.5，遮罩与不透明度逐框生效）'
} finally {
    $bmpAppearance.Dispose()
}

# ---- 2.3 map-only 两个框（无视频，图案地图）+ 逐框独立取景（两次导出对照）----
# 期望版面（§4 表：多框 + 第 1 框贴边、无视频）：框 1 = 整幅画面，框 2 = 自由矩形 (176,99,121.6,68.4)。
# 框 1 固定分段（scale=1）、框 2 跟跑者居中（viewZoom=2）→ 两块区域平均色明显不同。
# 第二份工程只把框 2 的 viewZoom 改成 0.3：框 1 区域必须不变、框 2 画面必须变（逐框独立）。
$boxesMapOnly = @(
    (New-MapBox -Mode 0 -Share 0.25 -ViewMode 0),
    (New-MapBox -Mode 4 -X 0.55 -Y 0.55 -W 0.38 -H 0.38 -ViewMode 1 -ViewZoom 2.0)
)
$boxesMapOnlyB = @(
    $boxesMapOnly[0],
    (New-MapBox -Mode 4 -X 0.55 -Y 0.55 -W 0.38 -H 0.38 -ViewMode 1 -ViewZoom 0.3)
)
$trrMapOnly = Join-Path $artifacts 'map-multi3-map-only.trr'
Write-MapBoxProject -Path $trrMapOnly -MapImage $mapPatternIni -VideoPath $clip -GpxPath $gpx -Boxes $boxesMapOnly -IncludeVideo $false
$trrMapOnlyB = Join-Path $artifacts 'map-multi3-map-only-box2zoom03.trr'
Write-MapBoxProject -Path $trrMapOnlyB -MapImage $mapPatternIni -VideoPath $clip -GpxPath $gpx -Boxes $boxesMapOnlyB -IncludeVideo $false
$pngMapOnly = Invoke-EncguiExport -Name 'map-multi3-map-only' -ProjectPath $trrMapOnly -MapOnly
$pngMapOnlyB = Invoke-EncguiExport -Name 'map-multi3-map-only-box2zoom03' -ProjectPath $trrMapOnlyB -MapOnly
$bmpMapOnly = Get-FrameBitmap $pngMapOnly
$bmpMapOnlyB = Get-FrameBitmap $pngMapOnlyB
try {
    Assert-CanvasSize -Bitmap $bmpMapOnly -ExpectedW $script:singleBoxCanvasW -ExpectedH $script:singleBoxCanvasH -What 'map-only 多框（应与单框画布一致）'
    $rects = Assert-OpaqueCoverage -Bitmap $bmpMapOnly -Boxes $boxesMapOnly -CanvasW $canvasW -CanvasH $canvasH -VideoOn $false -What 'map-only 多框'
    if ($rects.Count -ne 2) { throw "map-only 多框：应有 2 个框，实际 $($rects.Count)" }
    Assert-RectNear -Rect $rects[0] -X 0 -Y 0 -W 320 -H 180 -What 'map-only 框 1（无视频 → 整幅）'
    Assert-RectNear -Rect $rects[1] -X 176 -Y 99 -W 121.6 -H 68.4 -What 'map-only 框 2（自由 55%,55%,38%,38%）'
    # 无视频时画布四角仍由框 1 铺满（不会出现黑底）。
    foreach ($pt in @(@(2, 2), @(317, 2), @(2, 177))) {
        Assert-PixelMap -Bitmap $bmpMapOnly -X $pt[0] -Y $pt[1] -What ("map-only 多框：框 1 铺满角落 ({0},{1})" -f $pt[0], $pt[1])
    }
    # 两个取景方式不同 → 平均色不同。
    $box1Region = [pscustomobject]@{ x = 10.0; y = 40.0; w = 80.0; h = 100.0 }
    $avgBox1 = Get-RegionAverageColor -Bitmap $bmpMapOnly -Region $box1Region
    $avgBox2 = Get-RegionAverageColor -Bitmap $bmpMapOnly -Region (Get-BoxInterior -Box $rects[1] -InsetRatio 0.20)
    Assert-AverageColorDiffers -AvgA $avgBox1 -AvgB $avgBox2 -What 'map-only：固定分段框 vs 跟跑者居中框（内容应不同）' -MinDiff 40
    # 逐框独立：（A）框2 viewZoom=2.0 vs（B）框2 viewZoom=0.3。
    $box2Region = Get-BoxInterior -Box $rects[1] -InsetRatio 0.20
    Assert-RegionSimilar -BitmapA $bmpMapOnly -BitmapB $bmpMapOnlyB -Region $box1Region -What 'map-only：只改框 2 取景缩放时框 1 区域应不变' -Tolerance 24 -MinRatio 0.90
    Assert-RegionDiffers -BitmapA $bmpMapOnly -BitmapB $bmpMapOnlyB -Region $box2Region -What 'map-only：只改框 2 取景缩放时框 2 画面必须变化' -MinMeanDiff 18
    if ($bmpMapOnlyB.Width -ne $bmpMapOnly.Width -or $bmpMapOnlyB.Height -ne $bmpMapOnly.Height) {
        throw ("map-only 多框：两次导出画布尺寸不同 {0}x{1} vs {2}x{3}" -f $bmpMapOnly.Width, $bmpMapOnly.Height, $bmpMapOnlyB.Width, $bmpMapOnlyB.Height)
    }
    Write-Output ('PASS: multi3-map-only（框1 整幅固定分段 / 框2 跟跑者居中；框1 平均色=({0:0},{1:0},{2:0}) 框2=({3:0},{4:0},{5:0})）' -f `
        $avgBox1[0], $avgBox1[1], $avgBox1[2], $avgBox2[0], $avgBox2[1], $avgBox2[2])
} finally {
    $bmpMapOnly.Dispose()
    $bmpMapOnlyB.Dispose()
}

# ---- 2.4 三个框快速烟测（第 1 框贴边 + 两个自由框，纯红地图便于判定越界）----
$boxesThree = @(
    (New-MapBox -Mode 0 -Share 0.25),
    (New-MapBox -Mode 4 -X 0.30 -Y 0.06 -W 0.28 -H 0.40),
    (New-MapBox -Mode 4 -X 0.62 -Y 0.55 -W 0.30 -H 0.40)
)
$trrThree = Join-Path $artifacts 'map-multi4-three-boxes.trr'
Write-MapBoxProject -Path $trrThree -MapImage $map -VideoPath $clip -GpxPath $gpx -Boxes $boxesThree -IncludeVideo $true
$pngThree = Invoke-EncguiExport -Name 'map-multi4-three-boxes' -ProjectPath $trrThree
$bmpThree = Get-FrameBitmap $pngThree
try {
    Assert-CanvasSize -Bitmap $bmpThree -ExpectedW $script:singleBoxCanvasW -ExpectedH $script:singleBoxCanvasH -What '3 框烟测（应与单框画布一致）'
    $rects = Assert-OpaqueCoverage -Bitmap $bmpThree -Boxes $boxesThree -CanvasW $canvasW -CanvasH $canvasH -VideoOn $true -What '3 框烟测'
    if ($rects.Count -ne 3) { throw "3 框烟测：应有 3 个框，实际 $($rects.Count)" }
    Assert-RectNear -Rect $rects[0] -X 0 -Y 0 -W 80 -H 180 -What '3 框：框 1（固定左侧 25%）'
    Assert-RectNear -Rect $rects[1] -X 96 -Y 10.8 -W 89.6 -H 72 -What '3 框：框 2（30%,6%,28%,40%）'
    Assert-RectNear -Rect $rects[2] -X 198.4 -Y 99 -W 96 -H 72 -What '3 框：框 3（62%,55%,30%,40%）'
    # 三个框中心都要有地图（纯红）。
    foreach ($pt in @(@(40, 90), @(141, 47), @(246, 135))) {
        Assert-PixelMap -Bitmap $bmpThree -X $pt[0] -Y $pt[1] -What ("3 框：框中心 ({0},{1})" -f $pt[0], $pt[1])
    }
    # 框外（视频区）不能有地图色。
    foreach ($pt in @(@(300, 90), @(90, 90))) {
        Assert-PixelVideoBlue -Bitmap $bmpThree -X $pt[0] -Y $pt[1] -What ("3 框：框外视频区 ({0},{1})" -f $pt[0], $pt[1])
    }
    Write-Output 'PASS: multi4-three-boxes（3 框几何/覆盖/越界检查）'
} finally {
    $bmpThree.Dispose()
}
Write-Output "Multi map box export checks passed: $artifacts"

# =====================================================================================
# 3) 地图布局编辑器的无头交互回归（按多框列表模型：加/复制/删框、逐框取景、预览点选拖动、
#    贴边框不可拖、撤销、外部重载）。默认运行；需要临时跳过时传 -SkipQmlTest。
# =====================================================================================
if ($SkipQmlTest) {
    Write-Output '跳过 tst_map_layout.qml：按参数 -SkipQmlTest 跳过'
} else {
    $qmlTest = Join-Path $PSScriptRoot 'tst_map_layout.qml'
    $qmlRunner = Join-Path $QtRoot 'bin/qmltestrunner.exe'
    if (Test-Path $qmlRunner) {
        $resultFile = Join-Path $artifacts 'tst_map_layout.txt'
        $env:QML_IMPORT_PATH = Join-Path $QtRoot 'qml'
        $env:QT_QUICK_CONTROLS_STYLE = 'Fusion'
        $runner = Start-Process -FilePath $qmlRunner -ArgumentList @(
            '-platform', 'offscreen', '-input', ('"' + $qmlTest + '"'), '-o', ('"' + $resultFile + ',txt"')
        ) -WindowStyle Hidden -PassThru -Wait
        Get-Content -LiteralPath $resultFile
        if ($runner.ExitCode -ne 0) { throw "地图布局 QML 交互测试失败（退出码 $($runner.ExitCode)）" }
    } else {
        Write-Output "跳过 tst_map_layout.qml：未找到 qmltestrunner.exe"
    }
}
