param(
    # 要删掉的顶层块序号（1 起，逗号分隔）。缺省 = 列出块并退出。
    [string]$Remove = "",
    [string]$Qml = "app\src\bin\gui.qml"
)
# 二分定位 Qt 断言：把 gui.qml 的顶层块按序号删掉一部分，再跑 qmlprobe 看断言还在不在。
# 删除范围按大括号配平切分，删掉若干整块后文件仍是合法 QML（可能缺少 id 引用）。
$ErrorActionPreference = 'Stop'
Write-Output ("DEBUG Remove=[$Remove] args=[" + ($args -join "|") + "] firstline=[" + ((Get-Content $PSCommandPath -TotalCount 1)) + "]")
$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root $Qml
$lines = [System.IO.File]::ReadAllLines($src)

# ---- 找出 root 对象体（第一个 '{' 之后）里 depth==1 的块 ----
$chunks = New-Object System.Collections.Generic.List[object]
$depth = 0
$blockStart = -1
$blockLabel = ""
for ($i = 0; $i -lt $lines.Count; $i++) {
    $line = $lines[$i]
    $opens = ([regex]::Matches($line, '\{')).Count
    $closes = ([regex]::Matches($line, '\}')).Count
    if ($depth -eq 1 -and $opens -gt 0 -and $blockStart -lt 0) {
        $blockStart = $i
        $blockLabel = $line.Trim()
        if ($blockLabel.Length -gt 60) { $blockLabel = $blockLabel.Substring(0, 60) }
    }
    $depth += $opens - $closes
    if ($depth -eq 1 -and $blockStart -ge 0) {
        $chunks.Add([pscustomobject]@{ Start = $blockStart; End = $i; Label = $blockLabel })
        $blockStart = -1
    }
}
Write-Output "顶层块 $($chunks.Count) 个（$($lines.Count) 行）"

if (-not $Remove) {
    for ($k = 0; $k -lt $chunks.Count; $k++) {
        Write-Output ("  [{0,3}] 行 {1,5}-{2,-5} {3}" -f ($k + 1), ($chunks[$k].Start + 1), ($chunks[$k].End + 1), $chunks[$k].Label)
    }
    return
}

$drop = $Remove -split ',' | ForEach-Object { [int]$_.Trim() }
$keep = New-Object System.Collections.Generic.List[string]
$skip = New-Object 'System.Collections.Generic.HashSet[int]'
foreach ($k in $drop) { [void]$skip.Add($k) }
for ($i = 0; $i -lt $lines.Count; $i++) {
    $inDrop = $false
    for ($k = 0; $k -lt $chunks.Count; $k++) {
        if ($skip.Contains($k + 1) -and $i -ge $chunks[$k].Start -and $i -le $chunks[$k].End) { $inDrop = $true; break }
    }
    if (-not $inDrop) { $keep.Add($lines[$i]) }
}
$outQml = Join-Path $root 'demo\_bisect.qml'
[System.IO.File]::WriteAllLines($outQml, $keep)

# ---- 跑 probe ----
Get-Process -Name qmlprobe -ErrorAction SilentlyContinue | Stop-Process -Force
$QtRoot = if ($env:TRACEREEL_QT_ROOT) { $env:TRACEREEL_QT_ROOT } else { 'C:\Qt\6.8.2\msvc2022_64' }
$FfmpegBin = if ($env:TRACEREEL_FFMPEG_DIR) { Join-Path $env:TRACEREEL_FFMPEG_DIR 'bin' } else { Join-Path (Split-Path -Parent $PSScriptRoot) 'tools\ffmpeg\bin' }
$env:PATH = "$(Join-Path $QtRoot 'bin');$FfmpegBin;$env:PATH"
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QUICK_CONTROLS_STYLE = 'Fusion'
$env:QML_IMPORT_PATH = Join-Path $QtRoot 'qml'
$env:QMLPROBE_FILE = $outQml
$o = "$env:TEMP\bs_out.txt"; $e = "$env:TEMP\bs_err.txt"
Remove-Item $o, $e -ErrorAction SilentlyContinue
$p = Start-Process -FilePath (Join-Path $root 'target\debug\qmlprobe.exe') -RedirectStandardOutput $o -RedirectStandardError $e -PassThru -NoNewWindow
$finished = $p.WaitForExit(30000)
if (-not $finished) { $p.Kill(); Write-Output "删掉 [$Remove] -> 超时（弹窗）" ; return }
$err = Get-Content $e -ErrorAction SilentlyContinue
$so = Get-Content $o -ErrorAction SilentlyContinue
$assert = ($err | Select-String -Pattern 'ASSERT:').Count
$qmlErr = ($err | Select-String -Pattern 'is not defined|Cannot assign|non-existent|Unable to assign|Expected').Count
$startLine = ($so | Select-String -Pattern '^\[start\]').Line
$created = ($startLine -ne $null) -and ($startLine.Trim().Length -gt '[start]'.Length)
Write-Output ("删掉 [{0}] -> 断言 {1} 处 / 树已建立 {2} / QML 错误 {3} / 退出码 {4}" -f $Remove, $assert, $created, $qmlErr, $p.ExitCode)
if ($qmlErr -gt 0) { $err | Select-String -Pattern 'is not defined|Cannot assign|non-existent' | Select-Object -First 3 | ForEach-Object { "     $($_.Line.Trim())" } }
