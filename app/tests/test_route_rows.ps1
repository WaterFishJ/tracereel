# 轨迹样式行（路线 / 尾巴 / 控制点 / 运动员）的回归。
#
# 历史：以前这个脚本从 gui.qml 里抽取「显示」GroupBox 拼桩测试（显示组现在已删除）。
# 多地图框改造后，四组轨迹样式搬进了「地图布局」的逐框子面板
# （app/src/bin/MapEditor.qml），对应断言（分组顺序、每组的行顺序、菜单「显示项 →
# 存储值」映射、颜色 + 独立透明度配对、模式/勾选显隐、滑杆/勾选交互、多框按钮可见性）
# 已整体迁进 tst_map_layout.qml。
#
# 本脚本保留原文件名（run-all.ps1 与 docs/开发与构建.md 都引用它），实际直接运行
# 迁移后的 QML 测试，不再重复维护第二套桩；覆盖不降，且和 MapEditor 的真实结构一致。
param([string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$test = Join-Path $PSScriptRoot 'tst_map_layout.qml'
$runner = Join-Path $QtRoot 'bin\qmltestrunner.exe'
if (-not (Test-Path $runner)) {
    Write-Output '跳过：未找到 qmltestrunner.exe'
    exit 0
}
$resultDir = Join-Path ([System.IO.Path]::GetTempPath()) ('tr-style-rows-' + [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $resultDir -Force -ErrorAction Stop | Out-Null
} catch {
    $resultDir = Join-Path $PSScriptRoot ('../../ovcheck/style-rows-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $resultDir -Force | Out-Null
}
$resultFile = Join-Path $resultDir 'tst_map_layout.txt'
$env:QML_IMPORT_PATH = Join-Path $QtRoot 'qml'
$env:QT_QUICK_CONTROLS_STYLE = 'Fusion'
$p = Start-Process -FilePath $runner -ArgumentList @(
    '-platform', 'offscreen', '-input', ('"' + $test + '"'), '-o', ('"' + $resultFile + ',txt"')
) -WindowStyle Hidden -PassThru -Wait
$code = $p.ExitCode
try {
    Get-Content -LiteralPath $resultFile
} finally {
    Remove-Item -LiteralPath $resultDir -Recurse -Force -ErrorAction SilentlyContinue
}
exit $code
