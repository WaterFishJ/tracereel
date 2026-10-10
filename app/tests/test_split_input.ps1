param([string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw -Encoding UTF8 (Join-Path $PSScriptRoot '../src/bin/gui.qml')
$start = $source.IndexOf('    component SplitList: Column {')
$end = $source.IndexOf('    component VideoList: Column {', $start)
if ($start -lt 0 -or $end -lt 0) { throw 'SplitList component not found' }
$component = $source.Substring($start, $end - $start).Replace('Theme.text', '"#eeeeee"')
$harness = @'
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
Item {
    id: win
    width: 600; height: 500
    property int refreshTick: 0
    property alias times: settings.split_times
    property alias codes: settings.split_point_codes
    property alias list: splits
    function applyChange(prop) { refreshTick++ }
    function applyNow(prop) { refreshTick++ }
    function appendLog(level, message) {}
    function listCounts() { return "" }
    function jumpToSplit(index) {}
    QtObject { id: settings; property string split_times: ""; property string split_point_codes: "" }
    component TButton: Button { property bool small: false }
__COMPONENT__
    SplitList { id: splits }
}
'@
$testDir = Join-Path ([System.IO.Path]::GetTempPath()) ('ov-split-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testDir | Out-Null
$harnessPath = Join-Path $testDir 'SplitHarness.qml'
$testPath = Join-Path $testDir 'tst_split_input.qml'
$resultPath = Join-Path $testDir 'results.txt'
try {
    [System.IO.File]::WriteAllText($harnessPath, $harness.Replace('__COMPONENT__', $component))
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tst_split_input.qml') -Destination $testPath
    $env:PATH = "$QtRoot\bin;$env:PATH"
    $env:QML_IMPORT_PATH = "$QtRoot\qml"
    $env:QT_QUICK_CONTROLS_STYLE = 'Fusion'
    $runner = Start-Process -FilePath "$QtRoot\bin\qmltestrunner.exe" -ArgumentList @('-platform', 'offscreen', '-input', ('"' + $testPath + '"'), '-o', ('"' + $resultPath + ',txt"')) -WindowStyle Hidden -PassThru -Wait
    $testExit = $runner.ExitCode
    Get-Content -LiteralPath $resultPath
} finally {
    Remove-Item -LiteralPath $harnessPath, $testPath, $resultPath -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $testDir -ErrorAction SilentlyContinue
}
exit $testExit
