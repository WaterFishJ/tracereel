param([string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw -Encoding UTF8 (Join-Path $PSScriptRoot '../src/bin/gui.qml')

# 把 gui.qml 里「显示」那个 GroupBox 原样切出来（下一个 GroupBox 是「取景」）
$anchor = $source.IndexOf('title: "显示"')
if ($anchor -lt 0) { throw '显示 GroupBox not found' }
$start = $source.LastIndexOf('GroupBox {', $anchor)
$end = $source.IndexOf('GroupBox {', $start + 1)
if ($start -lt 0 -or $end -lt 0) { throw '显示 GroupBox bounds not found' }
$block = $source.Substring($start, $end - $start).TrimEnd()

$harness = @'
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
Item {
    id: win
    width: 600; height: 900
    property int refreshTick: 0
    property alias settings: settingsObj
    function applyChange(prop, note) { refreshTick++ }
    function refreshFields() { refreshTick++ }
    function pickColor(prop, opacityProp) {}
    function qtColor(v, opacity) { return v }
    function appendLog(level, message) {}
    // 「显示」组现在会引用轨迹列表的 hasMap（画面占比行）和预览项的实测色带端点
    // （关掉「配速色带自适应」时把滑杆对齐到它）——两个桩够编译与断言用。
    property alias trackHasMap: trackRows.hasMap
    QtObject { id: trackRows; property bool hasMap: true }
    QtObject { id: item; property real pace_auto_low: 6.5; property real pace_auto_high: 4.25 }
    QtObject {
        id: settingsObj
        property int route_render_mode: 1
        property int route_render_scope: 0
        property int route_tail_render_mode: 1
        property bool route_show_controls: true
        property bool route_show_runner: true
        property real route_width: 10
        property real route_tail_width: 10
        property real route_tail_length: 30
        property real route_control_radius: 15
        property real route_runner_radius: 5
        property int route_runner_style: 0
        property real route_low_pace: 15
        property real route_high_pace: 5
        property bool route_pace_auto: true
        property bool route_tail_pace_auto: true
        property real route_tail_low_pace: 15
        property real route_tail_high_pace: 5
        property real relative_width: 0.75
        // 「画面占比」在「没有视频」时灰显恒 100%：这两个键来自代理的同名属性。
        property string video_path: "clip.mp4"
        property bool encode_include_video: true
        property string route_color: "#000000"
        property real route_opacity: 80
        property string route_tail_color: "#000000"
        property real route_tail_opacity: 80
        property string route_control_color: "#000000"
        property real route_control_opacity: 255
        property string route_runner_color: "#0064ff"
        property real route_runner_opacity: 255
    }
    // 组件桩：只保留被测文本用到的接口（label/prop/hint/items/values + visible），
    // 而且**只能声明真实组件真的有的属性** —— 桩里多声明一个属性，桩测试就会掩盖
    // 真机的「Cannot assign to non-existent property」（脚本里有静态检查兜底）。
    component GroupBox: Column { property string title: "" }
    component SectionHeader: Item { property string text: "" }
    component SubSection: Column { property string title: "" }
    component PercentSpin: Item { property string label: ""; property string prop: ""; property bool forcedFull: false; property string hint: "" }
    component Combo: Item { property string label: ""; property string prop: ""; property string hint: ""; property var items: []; property var values: null; signal picked(int index) }
    component DSpin: Item { property string label: ""; property string prop: ""; property string hint: "" }
    component Check: Item { property string label: ""; property string prop: ""; property string hint: ""; property var valueOn: true; property var valueOff: false; signal uncheckedAction() }
    component ColorField: Item { property string label: ""; property string prop: ""; property string opacityProp: "" }
    component SliderField: Item { property string label: ""; property string prop: ""; property string hint: ""; property real from: 0; property real to: 100; property int decimals: 2; property bool alphaPercent: false }
__BLOCK__
}
'@

# —— 桩属性必须 ⊆ 真实组件属性（静态检查）——
# 桩测试跑的是假组件，所以桩里多声明一个真实组件没有的属性时，桩测试照样全绿，
# 真机一加载却报 “Cannot assign to non-existent property”。这个坑真出现过：
# 给 Combo 写 `hint: "..."`，桩有 hint、真实 Combo 没有 → TraceReel 直接打不开。
# 这里静态对一遍，把这类错误挡在跑测之前。
$realProps = @{}
foreach ($m in [regex]::Matches($source, '(?ms)^    component\s+(\w+)\s*:[^\r\n]*\r?\n(.*?)(?=^    component\s|\z)')) {
    $names = @([regex]::Matches($m.Groups[2].Value, '(?m)^        property\s+(?:alias\s+)?[\w\.]+\s+(\w+)') | ForEach-Object { $_.Groups[1].Value })
    $realProps[$m.Groups[1].Value] = $names
}
foreach ($line in ($harness -split "`r?`n")) {
    if ($line -notmatch '^\s*component\s+(\w+)\s*:') { continue }
    $name = $Matches[1]
    if (-not $realProps.ContainsKey($name)) { throw "桩组件 $name 在 gui.qml 里不存在（桩列表过期了）" }
    $stubProps = @([regex]::Matches($line, 'property\s+(?:alias\s+)?[\w\.]+\s+(\w+)') | ForEach-Object { $_.Groups[1].Value })
    $extra = @($stubProps | Where-Object { $realProps[$name] -notcontains $_ })
    if ($extra.Count -gt 0) {
        throw ("桩组件 {0} 多声明了真实组件没有的属性: {1} —— 桩测试会掩盖真机的 " +
               "'Cannot assign to non-existent property'，请先把 gui.qml 里的 {0} 补上这些属性（或改桩）" -f $name, ($extra -join ', '))
    }
}

$testDir = Join-Path ([System.IO.Path]::GetTempPath()) ('ov-route-rows-' + [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $testDir -ErrorAction Stop | Out-Null
} catch {
    $testDir = Join-Path $PSScriptRoot ('../../ovcheck/route-rows-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $testDir -Force | Out-Null
}
$harnessPath = Join-Path $testDir 'RouteRowsHarness.qml'
$testPath = Join-Path $testDir 'tst_route_rows.qml'
$resultPath = Join-Path $testDir 'results.txt'
$testExit = 1
try {
    [System.IO.File]::WriteAllText($harnessPath, $harness.Replace('__BLOCK__', $block))
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tst_route_rows.qml') -Destination $testPath
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
