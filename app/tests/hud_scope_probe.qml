// 自动化回归：HUD 的显示范围与「预览跟随导出勾选」（由 qmlprobe 驱动真实的 app/src/bin/gui.qml）。
//
// 工程由 `QMLPROBE_ORV` 提供，跑哪一组看工程本身：
//   · 工程里没勾「导出 → 视频」（只导地图+HUD）：断言 HUD 画布铺满整帧
//     （老代码按视频区算，HUD 会被挤进宽度为 0 的区域、整块看不见）、
//     取消勾选「导出 → HUD」后预览里的 HUD 也立刻消失。
//   · 工程里勾了「视频」：`scope = video` 时 HUD 只占视频区（左边空出地图带），
//     把「显示范围」切到「整幅画面」后铺满整帧 —— 也就是 HUD 能压到地图上。
// 两种都检查：HUD 页不再有「预览」勾选、「显示范围」下拉绑到 `hud_scope` 且选项/标签正确。
//
// 用法见 app/tests/test_hud_scope.ps1；判据打在 `HUD_SCOPE_PASS:` / `HUD_SCOPE_FAIL:` 这一行。
import QtQuick
import QtQuick.Controls

Item {
    id: probe
    width: 8
    height: 8

    property var win: null
    property var item: null
    property var canvas: null
    property var scopeRow: null
    property int step: 0
    property int wait: 0
    property string phase: "init"
    property string report: "PROBE 未开始"
    property real mapShare: 0.25
    property real frames0: 0
    property string playExpect: ""

    function debugState() { return report }
    function say(s) { console.log("[probe] " + s) }
    function close(a, b) { return Math.abs(a - b) < 1.5 }
    function state() { return win ? win.debugPreviewState() : "(窗口未创建)" }
    function busy() { return state().indexOf(" busy=true") >= 0 }

    // 预览播放检查：没勾「导出 → 视频」时预览不该再解码视频、也不该出声
    // （老代码这两件事都照做，倍速越高越卡）；勾着视频时必须照常出帧、照常出声，
    // 免得把正常播放改坏。expect = "off" / "on"。
    function startPlaybackCheck(expect) {
        playExpect = expect
        frames0 = item.debug_frames
        item.seek(300)
        item.set_speed(2)
        item.set_playing(true)
        wait = 0
        phase = "playing"
        say("2x 播放检查开始：expect=" + expect + " frames0=" + frames0)
    }

    function checkPlayback() {
        item.set_playing(false)
        var grew = (item.debug_frames - frames0) >= 3
        var audio = item.debug_audio_running()
        var frames = item.debug_frames
        if (playExpect === "off") {
            if (grew) {
                finish(false, "没勾「导出 → 视频」时预览还在解码视频（帧数 " + frames0 + " → " + frames + "）")
                return
            }
            if (audio) { finish(false, "没勾「导出 → 视频」时预览仍有声音（音频会话还活着）"); return }
            finish(true, "没勾视频：2x 播放不解码视频（帧数停在 " + frames + "）、不出声")
            return
        }
        if (!grew) {
            finish(false, "勾着视频时 2x 播放不出帧（帧数 " + frames0 + " → " + frames + "）")
            return
        }
        if (!audio) { finish(false, "勾着视频时预览音频没有起解码会话"); return }
        finish(true, "勾着视频：2x 播放照常出帧（" + frames0 + " → " + frames + "）、音频会话在")
    }

    function finish(ok, why) {
        report = (ok ? "HUD_SCOPE_PASS: " : "HUD_SCOPE_FAIL: ") + why
        say(report)
        timer.stop()
        Qt.quit()
    }

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
    function findByName(node, name) {
        if (!node) return null
        if (node.objectName === name) return node
        if (node.children) for (var i = 0; i < node.children.length; i++) {
            var found = findByName(node.children[i], name)
            if (found) return found
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
    // HUD 画布（HudCanvas）：有 `videoRect` 和 `editor`、没有 `label` 的那个。
    function findCanvas(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && c.videoRect !== undefined && c.editor !== undefined && c.label === undefined) return c
            var r = findCanvas(c)
            if (r) return r
        }
        return null
    }
    // 页签栏（QQC2 TabBar）：HUD 画布只在 HUD 页可见，探针得先切过去。
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
    function viewport() { return JSON.parse(String(item.hud_viewport(item.width, item.height))) }
    function vpText() {
        var v = viewport()
        return "(" + Math.round(v.x) + "," + Math.round(v.y) + "," + Math.round(v.width) + "," + Math.round(v.height) + ")"
    }
    function fullFrame(v) {
        return close(v.x, 0) && close(v.y, 0) && close(v.width, item.width) && close(v.height, item.height)
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
            item = findVideoItem(win.contentItem)
            if (!item) {
                if (step > 20) finish(false, "界面里找不到预览项 TraceReelItem")
                return
            }
            phase = "checks"
            return
        }

        // 公共检查：HUD 页不该再有「预览」勾选；新增的「显示范围」要绑到 hud_scope。
        if (phase === "checks") {
            if (findByLabel(win.contentItem, "预览") !== null) {
                finish(false, "HUD 页仍然有「预览」勾选（应该改成跟随导出勾选）")
                return
            }
            scopeRow = findByLabel(win.contentItem, "显示范围")
            if (!scopeRow) { finish(false, "HUD 页找不到「显示范围」下拉行"); return }
            if (scopeRow.prop !== "hud_scope") {
                finish(false, "「显示范围」绑定的属性不对: " + scopeRow.prop); return
            }
            var vals = scopeRow.values, labs = scopeRow.labels
            if (!vals || vals.length !== 2 || vals[0] !== "video" || vals[1] !== "frame") {
                finish(false, "「显示范围」选项不对: values=" + vals); return
            }
            if (!labs || labs[0] !== "视频区" || labs[1] !== "整幅画面") {
                finish(false, "「显示范围」标签不对: labels=" + labs); return
            }
            canvas = findCanvas(win.contentItem)
            if (!canvas) { finish(false, "界面里找不到 HUD 画布"); return }
            var tabBar = findTabBar(win.contentItem)
            if (!tabBar) { finish(false, "界面里找不到页签栏"); return }
            if (tabBar.currentIndex !== 3) {
                tabBar.currentIndex = 3
            }
            var share = Number(win.debugGet("map_relative_width"))
            if (share > 0 && share < 1) mapShare = share
            phase = "loaded"
            return
        }

        // 等启动那次 apply 落地（busy 落下即可，地图/HUD 版面此时已经算过）。
        if (phase === "loaded") {
            if (busy()) {
                if (step > 300) { finish(false, "等待工程加载超时: " + state()); return }
                return
            }
            var noVideo = win.debugGet("encode_include_video") === false
            say("工程已加载: " + state() + " includeVideo=" + win.debugGet("encode_include_video")
                + " includeHud=" + win.debugGet("encode_include_hud") + " hud_viewport=" + vpText())
            // Fixed layouts now expose width/height in the map layout editor.
            var shareField = findByName(win.contentItem, "mapStripShare")
            if (!shareField) { finish(false, "轨迹页找不到地图布局的占比输入框"); return }
            if (noVideo) {
                if (shareField.enabled) { finish(false, "没有视频时「画面占比」应灰显只读"); return }
                if (String(shareField.text) !== "100.0") {
                    finish(false, "没有视频时「画面占比」应固定显示 100.0，实际 " + shareField.text); return
                }
            } else {
                if (!shareField.enabled) { finish(false, "有视频时「画面占比」不该灰显"); return }
                var wantShare = (mapShare * 100).toFixed(1)
                if (String(shareField.text) !== wantShare) {
                    finish(false, "有视频时「画面占比」应显示 " + wantShare + "，实际 " + shareField.text); return
                }
            }
            if (noVideo) {
                if (!win.debugGet("encode_include_hud")) {
                    finish(false, "测试工程应该勾着「导出 → HUD」: " + win.debugGet("encode_include_hud")); return
                }
                if (!fullFrame(viewport())) {
                    finish(false, "只导地图+HUD 时 HUD 应铺满整帧 " + Math.round(item.width) + "x" + Math.round(item.height)
                           + "，实际 " + vpText()); return
                }
                if (!canvas.visible) { finish(false, "勾了「导出 → HUD」但 HUD 画布没显示"); return }
                // 取消勾选「导出 → HUD」：预览里的 HUD 也必须跟着消失。
                win.debugApplyProp("encode_include_hud", false)
                wait = 0
                phase = "hudOff"
                return
            }
            var v = viewport()
            var left = mapShare * item.width
            if (!close(v.x, left)) {
                finish(false, "「视频区」挡位下 HUD 左边应空出地图带 " + Math.round(left) + "，实际 " + vpText()); return
            }
            var combo = findComboBox(scopeRow)
            if (!combo) { finish(false, "「显示范围」行里找不到下拉框"); return }
            combo.currentIndex = 1
            combo.activated(1)
            wait = 0
            phase = "frame"
            return
        }

        if (phase === "hudOff") {
            if (!canvas.visible && win.debugGet("encode_include_hud") === false) {
                say("只导地图+HUD：HUD 铺满整帧 " + vpText() + "；取消勾选「导出 → HUD」后预览里的 HUD 也消失")
                // 这个工程没有视频层：2x 播放时不该解码视频、不该出声。
                startPlaybackCheck("off")
                return
            }
            if (wait++ >= 20) {
                finish(false, "取消勾选 HUD 后预览画布仍然显示（visible=" + canvas.visible
                       + " includeHud=" + win.debugGet("encode_include_hud") + "）"); return
            }
            return
        }

        if (phase === "frame") {
            var v2 = viewport()
            if (win.debugGet("hud_scope") === "frame" && fullFrame(v2)) {
                say("「视频区」时 HUD 让出地图带 " + Math.round(mapShare * item.width)
                    + " 像素，切「整幅画面」后铺满整帧 " + vpText())
                // 勾着视频的工程：2x 播放必须照常出帧、照常有声。
                startPlaybackCheck("on")
                return
            }
            if (wait++ >= 20) {
                finish(false, "切到「整幅画面」后 HUD 没有铺满整帧: scope=" + win.debugGet("hud_scope") + " " + vpText()); return
            }
            return
        }
        if (phase === "playing") {
            if (wait++ < 6) return
            checkPlayback()
            return
        }
    }
}
