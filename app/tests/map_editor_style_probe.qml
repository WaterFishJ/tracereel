// 逐框轨迹样式 + 框标签溢出的真实 GUI 集成探针（qmlprobe 驱动真实的 app/src/bin/gui.qml）。
//
// 背景：路线/尾巴/控制点/运动员四组配置已改为逐框独立（校准时间仍全局），且「地图布局」
// 编辑器的框标签改成 Flow 自动换行 + 操作按钮单独一行（框多时不把按钮挤出可视区）。
// 本探针对真实界面断言：
//   1) 2 框工程加载后，选中第 2 框调 `editor.setStyle(...)` / `editor.setStyleCheck(...)`：
//      · 第 2 框 debug_map_frames().boxes[1].style 与 appliedStyle 都变（取景器实际生效）；
//      · 第 1 框 style/appliedStyle 完全没变；
//      · 框数仍 2、debug_full_landed 不变（逐框样式改动走 apply_map_frames 就地更新，不重载）。
//   2) 双向隔离：选中第 1 框改样式，第 2 框不受影响。
//   3) 连续 addBox() 到 8 个框；mapFrameAdd/mapFrameRemove 仍在编辑器宽度内
//      可见可点；mapFrameTab_7 可见且点击后选中第 8 框（debug_map_frames 仍正常）。
//
// 判据行：`MAP_EDITOR_STYLE_PASS:` / `MAP_EDITOR_STYLE_FAIL:`；用法见 test_map_editor_style.ps1。
import QtQuick

Item {
    id: probe
    width: 8
    height: 8

    property var win: null
    property var video: null
    property var editor: null
    property int step: 0
    property int wait: 0
    property string phase: "init"
    property string report: "PROBE 未开始"
    /// 样式阶段开始前的完整重载落地次数（样式改动不允许让它增加）。
    property real landedBefore: -1
    /// 框 1 的 style+appliedStyle 快照（改框 2 时它必须原样）。
    property string box0Snapshot: ""
    /// 框 2 的 style+appliedStyle 快照（双向隔离时改框 1，它必须原样）。
    property string box2Snapshot: ""
    property int targetCount: 0

    // qmlprobe 在事件循环结束时会打印这个（[loop final] …）。
    function debugState() { return report }
    function say(s) { console.log("[probe] " + s) }
    function finish(ok, why) {
        report = (ok ? "MAP_EDITOR_STYLE_PASS: " : "MAP_EDITOR_STYLE_FAIL: ") + why
        say(report)
        timer.stop()
        Qt.quit()
    }
    function state() { return win ? win.debugPreviewState() : "(窗口未创建)" }
    function busy() { return state().indexOf(" busy=true") >= 0 }
    function near(a, b, eps) {
        return Math.abs(Number(a) - Number(b)) <= (eps === undefined ? 0.02 : eps)
    }
    function landed() { return Number(video.debug_full_landed) }
    function frames() {
        try { return JSON.parse(String(video.debug_map_frames())) } catch (e) { return null }
    }
    function styleOf(f, i) {
        return (f && f.boxes && f.boxes[i] && f.boxes[i].style) ? f.boxes[i].style : null
    }
    function appliedOf(f, i) {
        return (f && f.boxes && f.boxes[i] && f.boxes[i].appliedStyle) ? f.boxes[i].appliedStyle : null
    }
    function colorNear(arr, r, g, b) {
        return !!arr && near(arr[0], r) && near(arr[1], g) && near(arr[2], b)
    }
    function snap(f, i) {
        return JSON.stringify({ style: styleOf(f, i), applied: appliedOf(f, i) })
    }

    // ---- 在真实界面树里找控件 ----
    function findVideoItem(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && typeof c.debug_map_frames === "function") return c
            var r = findVideoItem(c)
            if (r) return r
        }
        return null
    }
    function findEditor(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && String(c.objectName) === "mapLayoutEditor") return c
            var r = findEditor(c)
            if (r) return r
        }
        return null
    }
    function findByObjectName(node, name) {
        if (!node) return null
        if (String(node.objectName) === name) return node
        if (!node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var r = findByObjectName(kids[i], name)
            if (r) return r
        }
        return null
    }

    // 逐框样式阶段的公共断言：框数 2 + 没触发完整重载 + 框 1 原样。返回错误串（空=通过）。
    function commonError(f) {
        if (!f || f.count !== 2) return "框数不是 2: " + (f ? f.count : "null")
        if (!appliedOf(f, 0) || !appliedOf(f, 1)) return "debug_map_frames 缺少 appliedStyle: " + JSON.stringify(f)
        if (!near(landed(), landedBefore, 1e-9)) {
            return "逐框样式改动触发了完整重载（debug_full_landed " + landedBefore + " → " + landed() + "）"
        }
        if (snap(f, 0) !== box0Snapshot) {
            return "改第 2 框样式影响了第 1 框: 现在=" + snap(f, 0) + " 之前=" + box0Snapshot
        }
        return ""
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
        if (step > 4000) { finish(false, "总步数超限（phase=" + phase + "）"); return }

        if (phase === "init") {
            video = findVideoItem(win.contentItem)
            editor = findEditor(win.contentItem)
            if (!video || !editor) {
                if (step > 20) finish(false, "界面里找不到预览项 TraceReelItem 或地图布局编辑器 mapLayoutEditor")
                return
            }
            say("已找到 TraceReelItem 与 mapLayoutEditor（boxCount=" + editor.boxCount + "）")
            phase = "load"
            wait = 0
            return
        }

        // ---- 等 2 框工程加载落地，并记录初始状态 ----
        if (phase === "load") {
            if (busy()) {
                if (wait++ > 300) finish(false, "等待 2 框工程加载超时: " + state())
                return
            }
            var f = frames()
            if (!f || f.count !== 2 || !styleOf(f, 0) || !appliedOf(f, 0) || !styleOf(f, 1) || !appliedOf(f, 1)) {
                if (wait++ > 300) finish(false, "等待 2 框取景器/样式就绪超时: " + JSON.stringify(f))
                return
            }
            if (editor.boxCount !== 2) {
                if (wait++ > 300) finish(false, "编辑器 boxCount 不是 2: " + editor.boxCount)
                return
            }
            // 初始样式来自 .trr：renderMode=0 / width=10 / routeColor=#000000 / showRunner=true。
            if (Number(styleOf(f, 1).renderMode) !== 0 || Number(appliedOf(f, 1).renderMode) !== 0) {
                finish(false, "初始 renderMode 不是 0: style=" + styleOf(f, 1).renderMode
                       + " applied=" + appliedOf(f, 1).renderMode)
                return
            }
            if (!near(appliedOf(f, 1).routeWidth, 10) || !near(appliedOf(f, 1).routeWidth, 10)) {
                finish(false, "初始路线宽度不是 10: " + JSON.stringify(appliedOf(f, 1)))
                return
            }
            landedBefore = landed()
            box0Snapshot = snap(f, 0)
            editor.selectFrame(1)
            if (editor.selected !== 1) {
                finish(false, "editor.selectFrame(1) 没生效，selected=" + editor.selected)
                return
            }
            say("2 框工程已加载 ✓ landed=" + landedBefore + "  选中框 2")
            phase = "style-renderMode"
            wait = 0
            return
        }

        // ---- 1) 第 2 框：路线渲染模式 ----
        if (phase === "style-renderMode") {
            if (wait++ === 0) { editor.setStyle("renderMode", 1, false); return }
            var f = frames()
            var err = commonError(f)
            if (err !== "") { finish(false, "routeRenderMode: " + err); return }
            var s = styleOf(f, 1), a = appliedOf(f, 1)
            if (Number(s.renderMode) !== 1 || Number(a.renderMode) !== 1) {
                if (wait < 30) return
                finish(false, "第 2 框路线渲染模式没生效: style=" + s.renderMode + " applied=" + a.renderMode)
                return
            }
            if (Number(editor.layout.boxes[1].style.renderMode) !== 1) {
                finish(false, "编辑器布局里的 style.renderMode 没更新: " + editor.layout.boxes[1].style.renderMode)
                return
            }
            if (Number(styleOf(f, 0).renderMode) !== 0 || Number(appliedOf(f, 0).renderMode) !== 0) {
                finish(false, "改第 2 框 renderMode 影响了第 1 框")
                return
            }
            say("逐框 renderMode 生效 ✓ 框2 style/applied=1，框1 仍 0")
            phase = "style-color"
            wait = 0
            return
        }

        // ---- 2) 第 2 框：路线颜色 ----
        if (phase === "style-color") {
            if (wait++ === 0) { editor.setStyle("routeColor", "#00ff00", false); return }
            var f = frames()
            var err = commonError(f)
            if (err !== "") { finish(false, "routeColor: " + err); return }
            var s = styleOf(f, 1), a = appliedOf(f, 1)
            if (String(s.routeColor).toLowerCase() !== "#00ff00" || !colorNear(a.routeColor, 0, 1, 0)) {
                if (wait < 30) return
                finish(false, "第 2 框路线颜色没生效: style=" + s.routeColor + " applied=" + JSON.stringify(a.routeColor))
                return
            }
            if (String(styleOf(f, 0).routeColor).toLowerCase() !== "#000000"
                    || !colorNear(appliedOf(f, 0).routeColor, 0, 0, 0)) {
                finish(false, "改第 2 框颜色影响了第 1 框")
                return
            }
            say("逐框 routeColor 生效 ✓ 框2 绿，框1 黑")
            phase = "style-width"
            wait = 0
            return
        }

        // ---- 3) 第 2 框：路线宽度（debug style 摘要不含 width，查 appliedStyle.routeWidth + 编辑器模型）----
        if (phase === "style-width") {
            if (wait++ === 0) { editor.setStyle("width", 20, false); return }
            var f = frames()
            var err = commonError(f)
            if (err !== "") { finish(false, "width: " + err); return }
            var a = appliedOf(f, 1)
            if (!near(a.routeWidth, 20)) {
                if (wait < 30) return
                finish(false, "第 2 框路线宽度没生效: appliedStyle.routeWidth=" + a.routeWidth)
                return
            }
            if (!near(editor.layout.boxes[1].style.width, 20)) {
                finish(false, "编辑器布局里的 style.width 没更新: " + editor.layout.boxes[1].style.width)
                return
            }
            if (!near(appliedOf(f, 0).routeWidth, 10) || !near(editor.layout.boxes[0].style.width, 10)) {
                finish(false, "改第 2 框宽度影响了第 1 框: applied=" + appliedOf(f, 0).routeWidth
                       + " model=" + editor.layout.boxes[0].style.width)
                return
            }
            say("逐框 width 生效 ✓ 框2 appliedStyle.routeWidth=20，框1 仍 10")
            phase = "style-showRunner"
            wait = 0
            return
        }

        // ---- 4) 第 2 框：运动员显隐（走 setStyleCheck）----
        if (phase === "style-showRunner") {
            if (wait++ === 0) { editor.setStyleCheck("showRunner", false); return }
            var f = frames()
            var err = commonError(f)
            if (err !== "") { finish(false, "showRunner: " + err); return }
            var s = styleOf(f, 1), a = appliedOf(f, 1)
            if (s.showRunner !== false || a.showRunner !== false) {
                if (wait < 30) return
                finish(false, "第 2 框运动员显隐没生效: style=" + s.showRunner + " applied=" + a.showRunner)
                return
            }
            if (styleOf(f, 0).showRunner !== true || appliedOf(f, 0).showRunner !== true) {
                finish(false, "改第 2 框 showRunner 影响了第 1 框")
                return
            }
            say("逐框 showRunner 生效 ✓ 框2=false，框1=true")
            phase = "bidirectional"
            wait = 0
            return
        }

        // ---- 5) 双向隔离：选中第 1 框改样式，第 2 框必须原样 ----
        if (phase === "bidirectional") {
            if (wait++ === 0) {
                var f = frames()
                box2Snapshot = snap(f, 1)
                editor.selectFrame(0)
                if (editor.selected !== 0) { finish(false, "editor.selectFrame(0) 没生效"); return }
                editor.setStyle("routeColor", "#0000ff", false)
                editor.setStyle("width", 33, false)
                return
            }
            var f = frames()
            if (!f || f.count !== 2) { finish(false, "双向隔离：框数不是 2"); return }
            if (!near(landed(), landedBefore, 1e-9)) {
                finish(false, "双向隔离：改第 1 框样式触发了完整重载（" + landedBefore + " → " + landed() + "）")
                return
            }
            var a0 = appliedOf(f, 0)
            if (String(styleOf(f, 0).routeColor).toLowerCase() !== "#0000ff"
                    || !colorNear(a0.routeColor, 0, 0, 1) || !near(a0.routeWidth, 33)) {
                if (wait < 30) return
                finish(false, "双向隔离：第 1 框样式没生效: " + JSON.stringify({ s: styleOf(f, 0), a: a0 }))
                return
            }
            if (snap(f, 1) !== box2Snapshot) {
                finish(false, "双向隔离：改第 1 框把第 2 框也改了: 现在=" + snap(f, 1) + " 之前=" + box2Snapshot)
                return
            }
            say("双向隔离 ✓ 框1 蓝/宽33，框2 仍绿/宽20/隐藏运动员")
            targetCount = 8
            phase = "overflow"
            wait = 0
            return
        }

        // ---- 6) 连续加框到 8 个：每次框数变化都会完整重载，逐次等落地 ----
        if (phase === "overflow") {
            if (busy()) {
                if (wait++ > 300) finish(false, "加框后完整重载超时（target=" + targetCount + "）: " + state())
                return
            }
            var f = frames()
            if (!f) { if (wait++ > 300) finish(false, "加框后 debug_map_frames 不可用"); return }
            if (editor.boxCount === 8 && f.count === 8) {
                say("已通过 editor.addBox() 加到 8 框 ✓（debug_full_landed=" + landed() + "）")
                phase = "buttons"
                wait = 0
                return
            }
            if (editor.boxCount < 8) {
                targetCount = editor.boxCount + 1
                editor.addBox()
                wait = 0
                return
            }
            // 编辑器已有 8 框、取景器还在重载路上
            if (wait++ > 300) {
                finish(false, "加框后取景器框数没跟上: editor=" + editor.boxCount + " frames=" + f.count)
            }
            return
        }

        // ---- 7) 8 框时：操作按钮仍在编辑器宽度内可见可点；标签 7 可选中 ----
        if (phase === "buttons") {
            if (wait++ === 0) { return } // 等一帧完成布局
            var names = ["mapFrameAdd", "mapFrameRemove"]
            var problems = []
            for (var i = 0; i < names.length; i++) {
                var b = findByObjectName(editor, names[i])
                if (!b) { problems.push(names[i] + " 找不到"); continue }
                if (!b.visible) problems.push(names[i] + " visible=false")
                if (!b.enabled) problems.push(names[i] + " enabled=false")
                var p = b.mapToItem(editor, 0, 0)
                if (p.x < -0.5 || p.x + b.width > editor.width + 0.5) {
                    problems.push(names[i] + " 超出编辑器宽度: x=" + p.x.toFixed(1)
                                  + " width=" + b.width.toFixed(1) + " editor.width=" + editor.width.toFixed(1))
                }
                if (p.y < -0.5) problems.push(names[i] + " y=" + p.y.toFixed(1) + " 在编辑器上方")
            }
            if (problems.length > 0) {
                finish(false, "8 框时操作按钮不可见/不可点: " + problems.join("；"))
                return
            }
            var tab7 = findByObjectName(editor, "mapFrameTab_7")
            if (!tab7) { finish(false, "8 框时找不到标签 mapFrameTab_7"); return }
            if (!tab7.visible || !tab7.enabled) {
                finish(false, "mapFrameTab_7 不可见/不可点: visible=" + tab7.visible + " enabled=" + tab7.enabled)
                return
            }
            say("8 框按钮布局 ✓ add/duplicate/remove 均在编辑器宽度内（editor.width="
                + editor.width.toFixed(1) + "）；mapFrameTab_7 可见")
            phase = "select-tab"
            wait = 0
            return
        }

        // 点击第 8 个标签 → 选中第 8 框，预览框布局查看仍正常。
        if (phase === "select-tab") {
            if (wait++ === 0) {
                var tab7 = findByObjectName(editor, "mapFrameTab_7")
                if (!tab7) { finish(false, "点击标签前找不到 mapFrameTab_7"); return }
                tab7.clicked()
                return
            }
            if (editor.selected !== 7) {
                if (wait < 30) return
                finish(false, "点击 mapFrameTab_7 后没有选中第 8 框: selected=" + editor.selected)
                return
            }
            var f = frames()
            if (!f || f.count !== 8 || !f.boxes[7]) {
                if (wait < 30) return
                finish(false, "选中第 8 框后 debug_map_frames 不正常: " + JSON.stringify(f))
                return
            }
            if (Number(f.boxes[7].mode) !== 4) {
                finish(false, "第 8 框应为自由摆放(mode=4)，实际 " + f.boxes[7].mode)
                return
            }
            finish(true, "逐框样式（renderMode/routeColor/width/showRunner）就地生效且双向隔离、不触发完整重载；"
                   + "8 框时添加/复制/删除按钮仍在编辑器宽度内且可见可点、mapFrameTab_7 可选中")
            return
        }

        finish(false, "未知阶段: " + phase)
    }
}
