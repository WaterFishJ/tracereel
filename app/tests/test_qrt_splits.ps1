param([string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw -Encoding UTF8 (Join-Path $PSScriptRoot '../src/bin/gui.qml')

# 把 gui.qml 里「.qrt 分段导入」那一段原样切出来（到拖放处理前为止）。
$start = $source.IndexOf('// ---------- .qrt 分段导入 ----------')
if ($start -lt 0) { throw '.qrt 分段导入 section not found' }
$end = $source.IndexOf('// ---------- 拖放到预览区', $start)
if ($end -lt 0) { throw '.qrt 分段导入 section end not found' }
$block = $source.Substring($start, $end - $start).TrimEnd()

$harness = @'
import QtQuick
import QtQuick.Controls

// 被测函数来自 app/src/bin/gui.qml，这里只提供它们用到的宿主接口。
Item {
    id: win
    width: 400; height: 300
    property alias settings: settingsObj
    property alias rows: trackRows
    property alias preview: item
    property alias dlg: qrtSplitDialog
    property int refreshTick: 0
    property var applied: []
    property var logs: []
    function applyChange(prop, note) { refreshTick++; applied.push(prop) }
    function refreshFields() { refreshTick++ }
    function appendLog(level, message) { logs.push(level + " " + message) }

    QtObject {
        id: settingsObj
        property string split_times: ""
        property string split_point_codes: ""
        property bool split_is_relative: true
    }
    // 轨迹列表：只要 writeBack 计数、pendingQrt* 两个字段。
    QtObject {
        id: trackRows
        property int pendingQrtIndex: -1
        property string pendingQrtPath: ""
        property int writeBacks: 0
        function writeBack() { writeBacks++ }
    }
    QtObject {
        id: qrtSplitDialog
        property int opens: 0
        function open() { opens++ }
        function close() { }
    }
    // 预览项：.qrt 解析结果由测试直接摆布。
    QtObject {
        id: item
        property bool hasSplits: true
        property string encoded: ""
        function qrt_has_splits(path) { return hasSplits }
        function qrt_split_times(path) { return encoded }
    }
__BLOCK__
}
'@

$harness = $harness.Replace('__BLOCK__', $block)

$testDir = Join-Path ([System.IO.Path]::GetTempPath()) ('ov-qrt-splits-' + [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $testDir -ErrorAction Stop | Out-Null
} catch {
    $testDir = Join-Path $PSScriptRoot ('../../ovcheck/qrt-splits-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $testDir -Force | Out-Null
}
$harnessPath = Join-Path $testDir 'QrtSplitsHarness.qml'
$testPath = Join-Path $testDir 'tst_qrt_splits.qml'
$resultPath = Join-Path $testDir 'results.txt'
$testExit = 1
try {
    [System.IO.File]::WriteAllText($harnessPath, $harness)
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tst_qrt_splits.qml') -Destination $testPath
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
