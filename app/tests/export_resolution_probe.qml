// 自动化回归：导出页「分辨率」下拉里的「与原始视频一致」（token `source`）必须
//   1) 是下拉第一项、标签是中文，选中后写回 `settings.encode_resolution = "source"`；
//   2) 工程里的 `source` 重新打开时仍原样保留（`settings.rs::load_orv` 不再把它归一成 1920x1080）；
//   3) 预览画布宽高比跟随**源视频**（`TraceReelItem.source_size`），而不是退回 16:9。
// 由 qmlprobe 驱动真实的 app/src/bin/gui.qml。
//
// 用法（自带素材，见 app/tests/test_export_resolution.ps1）：
//   $env:QMLPROBE_ORV = <造出来的 .trr 绝对路径>   # gui.qml 启动即加载这个工程
//   $env:QMLPROBE_FILE = "$PWD\app\tests\export_resolution_probe.qml"
//   $env:QMLPROBE_RUN_LOOP = "1"; $env:QMLPROBE_TIMEOUT_MS = "90000"
//   .\build.ps1 run --bin qmlprobe
//
// 判据打在这一行里：`EXPORT_RESOLUTION_PASS:` / `EXPORT_RESOLUTION_FAIL:`
import QtQuick
import QtQuick.Controls

Item {
    id: probe
    width: 8
    height: 8

    property var win: null
    property var video: null
    property var resRow: null
    property var resCombo: null
    property int step: 0
    property int wait: 0
    property string phase: "init"
    property string report: "PROBE 未开始"
    property string srcSize: ""
    property real srcAspect: 0

    // qmlprobe 在事件循环结束时会打印这个（[loop final] …）。
    function debugState() { return report }
    function say(s) { console.log("[probe] " + s) }
    function close(a, b) { return Math.abs(a - b) < 0.001 }
    // 「宽x高」→ 宽高比（与 gui.qml 的 win.aspectOf 同一算法，用来独立复算期望值）。
    function parseAspect(t) {
        var r = String(t).toLowerCase().split("x")
        return r.length === 2 && Number(r[0]) > 0 && Number(r[1]) > 0 ? Number(r[0]) / Number(r[1]) : 0
    }
    function state() { return win ? win.debugPreviewState() : "(窗口未创建)" }
    function busy() { return state().indexOf(" busy=true") >= 0 }

    function finish(ok, why) {
        report = (ok ? "EXPORT_RESOLUTION_PASS: " : "EXPORT_RESOLUTION_FAIL: ") + why
        say(report)
        timer.stop()
        Qt.quit()
    }

    // 递归找真实控件（ApplicationWindow 的内容项往下走）。导出页放在 StackLayout 里，
    // 所有页都是预先建好的，所以不用切页签就能拿到「分辨率」那一行。
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
    // 行里那个真正的下拉框（QQC2 ComboBox：currentIndex + activated 信号）。
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
    // 预览项 TraceReelItem：靠它独有的 outputAspect + source_size 认出来。
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

    Component.onCompleted: {
        var comp = Qt.createComponent(Qt.resolvedUrl("../src/bin/gui.qml"))
        if (comp.status !== Component.Ready) {
            finish(false, "gui.qml 加载失败: " + comp.errorString())
            return
        }
        win = comp.createObject(null)
        if (!win) { finish(false, "gui.qml 实例化失败（createObject 返回空）"); return }
        say("gui.qml 已实例化；工程由 QMLPROBE_ORV 提供")
        timer.start()
    }

    Timer {
        id: timer
        interval: 300
        repeat: true
        onTriggered: probe.advance()
    }

    function advance() {
        step++
        if (step > 4000) { finish(false, "总步数超限"); return }

        if (phase === "init") {
            video = findVideoItem(win.contentItem)
            if (!video) {
                if (step > 20) finish(false, "界面里找不到预览项 TraceReelItem")
                return
            }
            phase = "load"
            return
        }

        // 等启动时的那次 apply 落地：busy 落下 + 源尺寸已经探到。
        if (phase === "load") {
            if (!busy() && String(video.source_size).length > 0) {
                say("工程已加载: " + state() + " source_size=" + video.source_size)
                phase = "options"
                return
            }
            if (step > 300) { finish(false, "等待工程加载超时（busy / source_size 未就绪）: " + state() + " source_size=" + video.source_size); return }
            return
        }

        if (phase === "options") {
            resRow = findByLabel(win.contentItem, "分辨率")
            if (!resRow) { finish(false, "导出页找不到「分辨率」下拉行"); return }
            if (resRow.prop !== "encode_resolution") { finish(false, "「分辨率」行绑定的属性不对: " + resRow.prop); return }
            var vals = resRow.values, labs = resRow.labels
            if (!vals || !labs || vals.length !== 7 || labs.length !== 7) {
                finish(false, "分辨率选项数量不对: values=" + vals + " labels=" + labs); return
            }
            if (vals[0] !== "source" || labs[0] !== "与原始视频一致") {
                finish(false, "第一项应为 source /「与原始视频一致」，实际 " + vals[0] + " / " + labs[0]); return
            }
            // 工程里存的就是 source：重新打开后必须原样保留（不能被归一成 1920x1080）。
            if (win.debugGet("encode_resolution") !== "source") {
                finish(false, "工程里的 resolution 没保留 source: " + win.debugGet("encode_resolution")); return
            }
            resCombo = findComboBox(resRow)
            if (!resCombo) { finish(false, "「分辨率」行里找不到下拉框"); return }
            srcSize = String(video.source_size)
            srcAspect = parseAspect(srcSize)
            if (srcAspect <= 0) { finish(false, "源尺寸解析失败: " + srcSize); return }
            var a = Number(video.outputAspect)
            say("源尺寸=" + srcSize + " 源比例=" + srcAspect.toFixed(4) + " 预览画布比例=" + a.toFixed(4))
            if (!close(a, srcAspect)) {
                finish(false, "选「与原始视频一致」时预览画布比例应为源比例 " + srcAspect.toFixed(4) + "，实际 " + a.toFixed(4)); return
            }
            if (!close(srcAspect, 16 / 9) && close(a, 16 / 9)) {
                finish(false, "预览画布退回了 16:9：source_size 没传到 QML（比例=" + a.toFixed(4) + "）"); return
            }
            var idx = vals.indexOf("1280x720")
            if (idx < 0) { finish(false, "下拉里没有 1280x720"); return }
            resCombo.currentIndex = idx
            resCombo.activated(idx)
            wait = 0
            phase = "explicit"
            return
        }

        // 切到显式尺寸：预览比例必须跟着变成 16:9。
        // 代理属性没有 NOTIFY，`outputAspect` 靠 `refreshTick` 失效，而 ComboV 走的是
        // 350ms 防抖（`win.applyChange`）——所以这里必须轮询等，不能读完设置就立刻读比例。
        if (phase === "explicit") {
            var got = win.debugGet("encode_resolution")
            var a = Number(video.outputAspect)
            if (got === "1280x720" && close(a, 16 / 9)) {
                say("切到 1280x720 后 预览画布比例=" + a.toFixed(4))
                resCombo.currentIndex = 0
                resCombo.activated(0)
                wait = 0
                phase = "back"
                return
            }
            if (wait++ >= 40) {
                finish(false, "切到 1280x720 后预览比例没有变成 16:9（设置=" + got + " 比例=" + a.toFixed(4) + "）")
                return
            }
            return
        }

        // 再切回「与原始视频一致」：比例必须回到源比例。
        if (phase === "back") {
            var got2 = win.debugGet("encode_resolution")
            var a2 = Number(video.outputAspect)
            if (got2 === "source" && close(a2, srcAspect)) {
                finish(true, "下拉第一项=「与原始视频一致」(source)、写回正常；预览画布比例跟随源视频 " + srcSize
                            + "（" + srcAspect.toFixed(4) + "）→ 显式 1280x720（" + (16 / 9).toFixed(4) + "）→ 切回又是源比例")
                return
            }
            if (wait++ >= 40) {
                finish(false, "切回「与原始视频一致」后预览比例没有回到 " + srcAspect.toFixed(4) + "（设置=" + got2 + " 比例=" + a2.toFixed(4) + "）")
                return
            }
            return
        }
    }
}
