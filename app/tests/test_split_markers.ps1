param([string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$result = Join-Path $projectRoot 'target/split-markers-results.txt'
Remove-Item -LiteralPath $result -ErrorAction SilentlyContinue
$env:PATH = "$QtRoot/bin;$env:PATH"
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QUICK_CONTROLS_STYLE = 'Fusion'
& "$QtRoot/bin/qmltestrunner.exe" -input (Join-Path $PSScriptRoot 'tst_split_markers.qml') -o "$result,txt"
$code = $LASTEXITCODE
Get-Content -LiteralPath $result
if ($code -ne 0) { throw "分段跳转点测试失败（退出码 $code）" }
