// 自动化回归：导出页「导出范围」下拉与「自定义时间」起止时间（由 qmlprobe 驱动真实的
// app/src/bin/gui.qml）。
//
// 判据：
//   · 下拉标签是「导出范围」，绑 `encode_seg_mode`，四个选项/标签 = 全部 / 逐段导出 /
//     自定义分段 / 自定义时间；旧的「分段导出」标签不再存在。
//   · 选「自定义分段」只显示分段范围行，选「自定义时间」只显示时间范围行。
//   · 时间行填 `1:05` / `2:30` → 工程里落成 65000 / 150000 毫秒；结束填得比开始还早
//     （`0:30`）时保存值仍是 30000，导出时才按「到末尾」处理。
//   · 切回「全部」后模式为 `none`（整段导出），但用户填过的起止仍原样保留。
//
// 用法见 app/tests/test_export_range.ps1；判据打在 `EXPORT_RANGE_PASS:` / `EXPORT_RANGE_FAIL:` 这一行。
import QtQuick
import QtQuick.Controls

Item {
    id: probe
    width: 8
    height: 8

    property var win: null
    property var item: null
    property int step: 0
    property int wait: 0
    property string phase: "init"
    property string report: "PROBE 未开始"

    function debugState() { return report }
    function finish(ok, msg) {
        report = (ok ? "EXPORT_RANGE_PASS: " : "EXPORT_RANGE_FAIL: ") + msg
        console.log("[probe] " + report)
        Qt.exit(ok ? 0 : 1)
    }
    function say(s) { console.log("[probe] " + s) }

    function findByLabel(node, label) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && c.label === label) return c
            var r = findByLabel(c, label)
            if (r) return r
        }
        return null
    }
    // 找文字为 text 的 Label（用来定位「分段范围」「时间范围」这两行的显示状态）。
    function findLabel(node, text) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && c.text === text && c.font !== undefined) return c
            var r = findLabel(c, text)
            if (r) return r
        }
        return null
    }
    function findComboBox(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && typeof c.currentIndex === "number" && typeof c.activated === "function") return c
            var r = findComboBox(c)
            if (r) return r
        }
        return null
    }
    function findVideoItem(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && typeof c.outputAspect === "number" && typeof c.source_size === "string") return c
            var r = findVideoItem(c)
            if (r) return r
        }
        return null
    }
    function findTabBar(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && String(c).indexOf("TabBar") >= 0 && typeof c.currentIndex === "number") return c
            var r = findTabBar(c)
            if (r) return r
        }
        return null
    }
    function mode() { return String(win.debugGet("encode_seg_mode")) }
    function rowShown(text) {
        var l = findLabel(win.contentItem, text)
        if (!l) return null
        return l.parent ? l.parent.visible : l.visible
    }

    Component.onCompleted: {
        var comp = Qt.createComponent(Qt.resolvedUrl("../src/bin/gui.qml"))
        if (comp.status !== Component.Ready) {
            finish(false, "gui.qml 加载失败: " + comp.errorString())
            return
        }
        win = comp.createObject(null)
        if (!win) { finish(false, "gui.qml 实例化失败（createObject 返回空）"); return }
        say("gui.qml 已实例化")
        timer.start()
    }

    Timer {
        id: timer
        interval: 250
        repeat: true
        onTriggered: probe.advance()
    }

    function advance() {
        step++
        if (step > 4000) { finish(false, "总步数超限"); return }

        if (phase === "init") {
            item = findVideoItem(win.contentItem)
            if (!item) {
                if (step > 20) finish(false, "界面里找不到预览项 TraceReelItem")
                return
            }
            var tabBar = findTabBar(win.contentItem)
            if (!tabBar) { finish(false, "界面里找不到页签栏"); return }
            tabBar.currentIndex = 4 // 导出页
            phase = "combo"
            return
        }

        if (phase === "combo") {
            if (wait++ < 8) return
            var row = findByLabel(win.contentItem, "导出范围")
            if (!row) { finish(false, "导出页找不到「导出范围」下拉行"); return }
            if (row.prop !== "encode_seg_mode") {
                finish(false, "「导出范围」绑定的属性不对: " + row.prop); return
            }
            var vals = row.values, labs = row.labels
            var wantVals = ["none", "all", "custom", "time"]
            var wantLabs = ["全部", "逐段导出", "自定义分段", "自定义时间"]
            if (!vals || vals.length !== 4) { finish(false, "「导出范围」选项数不对: " + vals); return }
            for (var i = 0; i < 4; ++i) {
                if (vals[i] !== wantVals[i]) { finish(false, "「导出范围」选项[" + i + "] 应为 " + wantVals[i] + "，实际 " + vals[i]); return }
                if (labs[i] !== wantLabs[i]) { finish(false, "「导出范围」标签[" + i + "] 应为 " + wantLabs[i] + "，实际 " + labs[i]); return }
            }
            if (findByLabel(win.contentItem, "分段导出") !== null) {
                finish(false, "旧的「分段导出」下拉还在"); return
            }
            probe.combo = findComboBox(row)
            if (!probe.combo) { finish(false, "「导出范围」行里找不到下拉框"); return }
            // 选「自定义分段」：只有分段范围行出现。
            probe.combo.currentIndex = 2
            probe.combo.activated(2)
            wait = 0
            phase = "custom"
            return
        }

        if (phase === "custom") {
            if (wait++ < 10) return
            if (mode() !== "custom") { finish(false, "选「自定义分段」后模式应为 custom，实际 " + mode()); return }
            if (rowShown("分段范围") !== true) { finish(false, "「自定义分段」时应显示分段范围行"); return }
            if (rowShown("时间范围") !== false) { finish(false, "「自定义分段」时不该显示时间范围行"); return }
            probe.combo.currentIndex = 3
            probe.combo.activated(3)
            wait = 0
            phase = "timeRow"
            return
        }

        if (phase === "timeRow") {
            if (wait++ < 10) return
            if (mode() !== "time") { finish(false, "选「自定义时间」后模式应为 time，实际 " + mode()); return }
            if (rowShown("时间范围") !== true) { finish(false, "「自定义时间」时应显示时间范围行"); return }
            if (rowShown("分段范围") !== false) { finish(false, "「自定义时间」时不该显示分段范围行"); return }
            win.setClipTime("start", "1:05")
            wait = 0
            phase = "start"
            return
        }

        if (phase === "start") {
            if (wait++ < 10) return
            var s = Number(win.debugGet("encode_clip_start_ms"))
            if (Math.abs(s - 65000) > 1) { finish(false, "开始时间 1:05 应落成 65000 毫秒，实际 " + s); return }
            win.setClipTime("end", "2:30")
            wait = 0
            phase = "end"
            return
        }

        if (phase === "end") {
            if (wait++ < 10) return
            var s2 = Number(win.debugGet("encode_clip_start_ms"))
            var e2 = Number(win.debugGet("encode_clip_end_ms"))
            if (Math.abs(e2 - 150000) > 1) { finish(false, "结束时间 2:30 应落成 150000 毫秒，实际 " + e2); return }
            say("时间范围落成 " + s2 + " → " + e2 + " 毫秒")
            // 结束填得比开始还早：保存值就是 range_end_ms（30000），导出时才按「到末尾」处理。
            win.setClipTime("end", "0:30")
            wait = 0
            phase = "reversed"
            return
        }

        if (phase === "reversed") {
            if (wait++ < 10) return
            var e3 = Number(win.debugGet("encode_clip_end_ms"))
            if (Math.abs(e3 - 30000) > 1) { finish(false, "结束时间 0:30 应落成 30000 毫秒，实际 " + e3); return }
            probe.combo.currentIndex = 0
            probe.combo.activated(0)
            wait = 0
            phase = "all"
            return
        }

        if (phase === "all") {
            if (wait++ < 12) return
            if (mode() !== "none") { finish(false, "选「全部」后模式应为 none，实际 " + mode()); return }
            // 切到「全部」不改用户填过的时间（存的是 range_*，导出时才按模式推导 clip_*），
            // 所以这里要能原样读回来 —— 切回「自定义时间」不用重填。
            var s4 = Number(win.debugGet("encode_clip_start_ms"))
            var e4 = Number(win.debugGet("encode_clip_end_ms"))
            if (Math.abs(s4 - 65000) > 1 || Math.abs(e4 - 30000) > 1) {
                finish(false, "切到「全部」后应保留填过的起止（65000/30000），实际 " + s4 + " / " + e4); return
            }
            finish(true, "导出范围四挡 + 自定义时间起止生效（1:05→65000、2:30→150000、填反→到末尾、全部不丢填过的值）")
            return
        }
    }

    property var combo: null
}
