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
//
// ⚠ refreshFields() 必须连同「轨迹列表按 settings 重新同步」一起模拟：
// 真机上 gui.qml 的 `function refreshFields() { refreshTick++; ... }` 会触发
// TrackList 的 `onRefreshTickChanged` → `syncFromSettings()`；只要 settings 里的
// route_path/map_extras 与列表行不一致，行就被当成「外部改动」清掉。
// 只把 refreshFields 当空函数，就测不出「导入分段把还没落地的地图行冲掉」这类跨组件 bug
// —— `.qrt` 有分段、当前工程没分段时地图没导入就是这么来的（0.3.0 回归）。
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
    function refreshFields() { refreshTick++; trackRows.syncFromSettings() }
    function appendLog(level, message) { logs.push(level + " " + message) }

    QtObject {
        id: settingsObj
        property string split_times: ""
        property string split_point_codes: ""
        property bool split_is_relative: true
        property string route_path: ""
        property string map_path: ""
    }
    // 轨迹列表：writeBack 把当前行落地，syncFromSettings 按 settings 重建（未落地的行被清掉）。
    QtObject {
        id: trackRows
        property string pendingQrtPath: ""
        property int writeBacks: 0
        property string mainPath: ""
        property int syncs: 0
        property bool wiped: false
        function writeBack() {
            writeBacks++
            settingsObj.route_path = mainPath
            settingsObj.map_path = mainPath
        }
        function syncFromSettings() {
            syncs++
            if (String(settingsObj.route_path) !== mainPath) {
                mainPath = String(settingsObj.route_path)
                wiped = true
            }
        }
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
