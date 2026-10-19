# 统一测试入口：一次跑完仓库里的自动化测试（纯逻辑单测 + 无头 QML / 导出端到端回归）。
#
# 用法:
#   .\app\tests\run-all.ps1                      # 默认：三个 crate 的 cargo test + 全部无头回归
#   .\app\tests\run-all.ps1 -SkipCargo           # 只跑无头回归
#   .\app\tests\run-all.ps1 -IncludeDemo         # 加上依赖 demo 工程（孝德湖）的用例
#   .\app\tests\run-all.ps1 -SampleImage x.jpg   # 加上成绩条 OCR 识别（需指定样图）
#
# 依赖：Qt 6.8（含 qmltestrunner）、ffmpeg 运行包（tools/ffmpeg）、已初始化的 gyroflow 子模块。
# 素材缺失的用例会自动跳过：app 单测读环境变量，demo 工程不在仓库里。
param(
    [switch]$SkipCargo,
    [switch]$IncludeDemo,
    [string]$SampleImage = ''
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$script:results = @()
$script:failures = 0

function Invoke-Case {
    param([string]$Name, [scriptblock]$Run)
    Write-Host ''
    Write-Host "===== $Name ====="
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try {
        & $Run
        $script:results += [pscustomobject]@{ Name = $Name; Ok = $true; Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1) }
        Write-Host "PASS  $Name（$([math]::Round($sw.Elapsed.TotalSeconds, 1)) 秒）"
    } catch {
        $script:failures++
        $script:results += [pscustomobject]@{ Name = $Name; Ok = $false; Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1) }
        Write-Host "FAIL  $Name：$_"
    }
}

if (-not $SkipCargo) {
    Invoke-Case 'cargo test（tracereel / tracereel-core / receipt-ocr）' {
        & (Join-Path $projectRoot 'build.ps1') test -p tracereel -p tracereel-core -p receipt-ocr
        if ($LASTEXITCODE -ne 0) { throw "cargo test 失败（退出码 $LASTEXITCODE）" }
    }
}

$cases = @(
    'test_split_input.ps1',
    'test_qrt_splits.ps1',
    'test_route_rows.ps1',
    'test_receipt_zoom.ps1',
    'test_export_resolution.ps1',
    'test_export_range.ps1',
    'test_export_progress.ps1',
    'test_hud_scope.ps1'
)
foreach ($case in $cases) {
    $path = Join-Path $PSScriptRoot $case
    Invoke-Case $case { & $path }
}

if (Get-Command node -ErrorAction SilentlyContinue) {
    Invoke-Case 'test_receipt_import.cjs（票据导入脚本）' {
        & node (Join-Path $PSScriptRoot 'test_receipt_import.cjs')
        if ($LASTEXITCODE -ne 0) { throw "test_receipt_import.cjs 失败（退出码 $LASTEXITCODE）" }
    }
} else {
    Write-Host ''
    Write-Host '跳过 test_receipt_import.cjs：未找到 node'
}

if ($IncludeDemo) {
    $demoProject = Join-Path $projectRoot 'demo/20261011孝德湖/孝德湖上午.trr'
    if (Test-Path -LiteralPath $demoProject) {
        Invoke-Case 'test_export_decode_size_gyro.ps1（demo 工程）' {
            & (Join-Path $PSScriptRoot 'test_export_decode_size_gyro.ps1') -Project $demoProject
        }
    } else {
        Write-Host ''
        Write-Host '跳过 test_export_decode_size_gyro.ps1：demo 工程不存在'
    }
}

if ($SampleImage) {
    Invoke-Case 'test_receipt_ocr.ps1（成绩条样图）' {
        & (Join-Path $PSScriptRoot 'test_receipt_ocr.ps1') -SampleImage $SampleImage
    }
}

Write-Host ''
Write-Host '===== 汇总 ====='
$script:results | Format-Table -AutoSize | Out-String | Write-Host
if ($script:failures -gt 0) {
    throw "$($script:failures) 项失败"
}
Write-Host "全部通过（$($script:results.Count) 项）"