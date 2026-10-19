// 导出进度弹窗的统一性回归（qmlprobe 驱动真实的 app/src/bin/gui.qml，offscreen）。
//
// 目标（用户要求）：直接导出的「编码」窗口和「导出队列」里正在导出的那一项，都要显示
//   · 导出配置（与队列每项同一段文字，来自 export_queue::configuration）
//   · 已用时 / 剩余用时 / 当前帧 / 总帧数 / 帧/秒
// 其余统计（总用时、视频当前、视频总长、输出大小）不再出现。
//
// 判据打在 `EXPORT_PROGRESS_PASS:` / `EXPORT_PROGRESS_FAIL:` 这一行；跑法见
// app/tests/test_export_progress.ps1。
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
    function say(s) { console.log("[probe] " + s) }
    function finish(ok, msg) {
        report = (ok ? "EXPORT_PROGRESS_PASS: " : "EXPORT_PROGRESS_FAIL: ") + msg
        say(report)
        Qt.quit()
    }

    // 窗口内容由 gui.qml 的 `debugExportWindow` 摊平成文本（QML 侧没有按标题找窗口的接口）。
    function exportTexts(which) {
        var r = win.debugExportWindow(which)
        return r && r.texts ? r.texts : []
    }
    function hasText(list, needle) {
        for (var i = 0; i < list.length; i++) if (list[i].indexOf(needle) >= 0) return true
        return false
    }
    // 找出带 `configText` 属性的控件（导出配置那一行）。

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
        if (step > 2000) { finish(false, "总步数超限"); return }

        if (phase === "init") {
            item = findVideoItem(win.contentItem)
            if (!item) { if (step > 20) finish(false, "界面里找不到预览项 TraceReelItem"); return }
            // 走用户真正点的那条路：工程里已填好输出路径，startExport 会打开「编码」窗口。
            win.startExport()
            phase = "exporting"
            wait = 0
            return
        }

        if (phase === "exporting") {
            // 等 export 真的跑起来（配置文本由 start_encode 写入）。
            if (wait++ < 40) return
            var cfg = String(item.encode_config || "")
            if (cfg.length === 0) { finish(false, "开始导出后 encode_config 仍是空的（弹窗没有导出配置可显示）"); return }
            if (cfg.indexOf("导出范围：") < 0) { finish(false, "导出配置里没有「导出范围：」这一行：" + cfg); return }
            if (cfg.indexOf("编码器：") < 0) { finish(false, "导出配置里没有「编码器：」这一行：" + cfg); return }
            item.abort_encode()
            phase = "encodewin"
            wait = 0
            return
        }

        if (phase === "encodewin") {
            if (wait++ < 12) return
            var texts = exportTexts("encode")
            if (texts.length === 0) { finish(false, "「编码」窗口里读不到任何文案"); return }
            var need = ["已用时", "剩余用时", "当前帧", "总帧数", "帧/秒"]
            for (var i = 0; i < need.length; i++) {
                if (!hasText(texts, need[i])) { finish(false, "「编码」窗口缺少「" + need[i] + "」"); return }
            }
            var gone = ["总用时", "视频当前", "视频总长", "当前大小", "总大小", "分段导出"]
            for (var j = 0; j < gone.length; j++) {
                if (hasText(texts, gone[j])) { finish(false, "「编码」窗口还留着「" + gone[j] + "」"); return }
            }
            var info = win.debugExportWindow("encode")
            if (String(info.config).indexOf("导出范围：") < 0) {
                finish(false, "「编码」窗口的配置行没有显示导出配置（config=" + info.config + "）"); return
            }
            if (info.stats < 1) { finish(false, "「编码」窗口没有导出统计组件（ExportStats）"); return }
            phase = "queue"
            return
        }

        if (phase === "queue") {
            win.addExportQueue()
            wait = 0
            phase = "queue-done"
            return
        }

        if (phase === "queue-done") {
            if (wait++ < 12) return
            var q = win.debugExportWindow("queue")
            if (!q || q.texts.length === 0) { finish(false, "「导出队列」窗口里读不到任何文案"); return }
            if (!hasText(q.texts, "导出范围：")) {
                finish(false, "导出队列的条目里没有「导出范围：」配置行"); return
            }
            // 正在导出的那一项才显示统计；这里只确认队列条目真的复用了同一个统计组件。
            if (q.stats < 1) { finish(false, "导出队列里没有复用导出统计组件（ExportStats）"); return }
            finish(true, "「编码」窗口只有导出配置 + 已用时/剩余用时/当前帧/总帧数/帧/秒（用时/视频/大小分组已移除），导出队列复用同一套统计与配置文字")
        }
    }

    function findVideoItem(node) {
        if (!node) return null
        // TraceReelItem 有 debug_frames / start_encode 这两个成员
        if (node.start_encode !== undefined && node.debug_frames !== undefined) return node
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var r = findVideoItem(kids[i])
            if (r) return r
        }
        return null
    }
}
