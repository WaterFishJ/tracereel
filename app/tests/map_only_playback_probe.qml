import QtQuick

Item {
    id: probe
    property var win: null
    property var item: null
    property int step: 0
    property int phase: 0
    property real startedAt: 0
    property string report: "等待纯轨迹播放"
    function debugState() { return report }
    function finish(ok, why) {
        report = (ok ? "MAP_PLAYBACK_PASS: " : "MAP_PLAYBACK_FAIL: ") + why
        console.log(report)
        timer.stop()
        Qt.quit()
    }
    function findItem(node) {
        if (!node || !node.children) return null
        for (var i = 0; i < node.children.length; ++i) {
            var child = node.children[i]
            if (typeof child.hud_viewport === "function" && typeof child.seek === "function") return child
            var found = findItem(child)
            if (found) return found
        }
        return null
    }
    Component.onCompleted: {
        var c = Qt.createComponent(Qt.resolvedUrl("../src/bin/gui.qml"))
        if (c.status !== Component.Ready) { finish(false, c.errorString()); return }
        win = c.createObject(null)
        timer.start()
    }
    Timer {
        id: timer
        interval: 100
        repeat: true
        onTriggered: {
            step++
            if (step > 200) { finish(false, "超时，阶段=" + phase); return }
            if (!item) item = findItem(win.contentItem)
            if (!item || item.busy || item.duration_ms <= 0) return
            if (phase === 0) {
                if (Math.abs(item.duration_ms - 59000) > 1) { finish(false, "轨迹时长错误=" + item.duration_ms); return }
                var v = JSON.parse(String(item.hud_viewport(item.width, item.height)))
                if (Math.abs(v.width - item.width) > 1 || Math.abs(v.x) > 1) { finish(false, "无视频时 HUD 未铺满"); return }
                item.seek(10000)
                item.set_speed(2)
                item.set_playing(true)
                startedAt = Date.now()
                phase = 1
            } else if (phase === 1 && Date.now() - startedAt >= 800) {
                if (item.t_ms < 11000 || item.t_ms > 14000) { finish(false, "纯轨迹播放/倍速未推进=" + item.t_ms); return }
                if (item.debug_audio_running()) { finish(false, "纯轨迹不应启动音频"); return }
                item.set_playing(false)
                item.seek(30000)
                startedAt = Date.now()
                phase = 2
            } else if (phase === 2 && Date.now() - startedAt >= 300) {
                if (Math.abs(item.t_ms - 30000) > 1) { finish(false, "暂停拖动位置错误"); return }
                item.set_playing(true)
                startedAt = Date.now()
                phase = 3
            } else if (phase === 3 && Date.now() - startedAt >= 700) {
                if (item.t_ms < 31000) { finish(false, "拖动后恢复仍在等视频帧=" + item.t_ms); return }
                item.seek(58500)
                startedAt = Date.now()
                phase = 4
            } else if (phase === 4 && Date.now() - startedAt >= 700) {
                if (item.t_ms > 4000) { finish(false, "播到轨迹末尾未循环=" + item.t_ms); return }
                item.set_playing(false)
                finish(true, "默认勾选视频但未导入视频：轨迹时长、全幅 HUD、倍速播放、暂停拖动、恢复、循环均正常")
            }
        }
    }
}
