// 多地图框预览回归：由 qmlprobe 驱动真实的 app/src/bin/gui.qml，加载一个 2 框工程。
//
// 断言（判据行：MULTIBOX_PREVIEW_PASS / MULTIBOX_PREVIEW_FAIL）：
//   1) 工程加载后 `debug_map_frames()` 报 2 个框，属性与工程里的 boxes 一致；
//   2) 调 `apply_map_frames(原布局)` 触发逐框窗口配置：第 1 框（贴边）窗口 = 整幅预览尺寸
//      （工程没视频层，按规则贴边框铺满），第 2 框（自由）窗口 = 0.4×0.4 预览尺寸；
//      两框取景器独立：第 2 框（跟跑者居中 zoom=2）相机 scale=2，第 1 框（固定分段）scale=1；
//   3) 就地改第 2 框 viewZoom=1.5 → 只有它的相机 scale 变 1.5，框数不变、不触发重载；
//   4) 框数变化的 JSON（1 框）被拒绝（仍然 2 框）——结构变化必须走完整重载。
//
// 用法见 app/tests/test_multibox_preview.ps1。
import QtQuick

Item {
    id: probe
    width: 8
    height: 8

    property var win: null
    property var video: null
    property var editor: null
    property var layoutJson: ""
    property int step: 0
    property string phase: "init"
    property int wait: 0
    property string report: "PROBE 未开始"

    // qmlprobe 在事件循环结束时会打印这个（[loop final] …）。
    function debugState() { return report }
    function say(s) { console.log("[probe] " + s) }
    function finish(ok, why) {
        report = (ok ? "MULTIBOX_PREVIEW_PASS: " : "MULTIBOX_PREVIEW_FAIL: ") + why
        say(report)
        timer.stop()
        Qt.quit()
    }
    function close(a, b) { return Math.abs(Number(a) - Number(b)) < 0.6 }
    // 相机数值用更紧的容差（窗口尺寸可以是几百像素，缩放只有 0.5 的差也要能区分）。
    function near(a, b) { return Math.abs(Number(a) - Number(b)) < 0.02 }
    function state() { return win ? win.debugPreviewState() : "(窗口未创建)" }
    function busy() { return state().indexOf(" busy=true") >= 0 }
    function frames() {
        try { return JSON.parse(String(video.debug_map_frames())) } catch (e) { return null }
    }
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
    // 改一份 layout JSON 里第 index 个框的字段（保持其余字段原样）。
    function patchBox(json, index, patch) {
        var v = JSON.parse(json)
        if (!v.boxes || !v.boxes[index]) return json
        for (var k in patch) v.boxes[index][k] = patch[k]
        return JSON.stringify(v)
    }
    // 改第 index 个框的**轨迹显示样式**（嵌套 style 对象）。
    function patchStyle(json, index, patch) {
        var v = JSON.parse(json)
        if (!v.boxes || !v.boxes[index]) return json
        if (!v.boxes[index].style) v.boxes[index].style = {}
        for (var k in patch) v.boxes[index].style[k] = patch[k]
        return JSON.stringify(v)
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
            if (!video) { if (step > 20) finish(false, "界面里找不到预览项 TraceReelItem"); return }
            phase = "load"
            return
        }

        // 等启动加载落地（busy 落下 + 已有 2 个地图框的取景器）。
        if (phase === "load") {
            if (!busy()) {
                var f = frames()
                if (f && f.count === 2) {
                    say("工程已加载: " + state() + "  boxes=" + f.count)
                    layoutJson = String(win.debugGet("map_boxes"))
                    if (layoutJson.length < 3) { finish(false, "代理 map_boxes 为空"); return }
                    var parsed = JSON.parse(layoutJson)
                    if (!parsed.boxes || parsed.boxes.length !== 2) {
                        finish(false, "代理 map_boxes 里不是 2 个框: " + layoutJson); return
                    }
                    phase = "check-boxes"
                    wait = 0
                    return
                }
            }
            if (step > 300) { finish(false, "等待工程加载超时（busy / 地图框未就绪）: " + state()); return }
            return
        }

        // 逐框属性与取景器：调一次 apply_map_frames 让窗口按预览尺寸重配。
        if (phase === "check-boxes") {
            if (wait++ === 0) {
                video.apply_map_frames(layoutJson)
                return
            }
            var f = frames()
            if (!f || f.count !== 2) { finish(false, "apply_map_frames 后框数不是 2: " + JSON.stringify(f)); return }
            var b0 = f.boxes[0], b1 = f.boxes[1]
            if (!b0.camera || !b1.camera) { finish(false, "取景器缺少相机状态: " + JSON.stringify(f)); return }
            if (b0.mode !== 0) { finish(false, "第 1 框应为贴边(mode=0)，实际 " + b0.mode); return }
            if (b1.mode !== 4) { finish(false, "第 2 框应为自由摆放(mode=4)，实际 " + b1.mode); return }
            if (b1.viewMode !== 1 || !close(b1.viewZoom, 2.0)) {
                finish(false, "第 2 框取景应为 viewMode=1 zoom=2，实际 " + b1.viewMode + "/" + b1.viewZoom); return
            }
            var W = Number(video.width), H = Number(video.height)
            if (!(W > 0 && H > 0)) { finish(false, "预览项尺寸无效: " + W + "x" + H); return }
            // 无视频层：贴边第 1 框铺满整幅；第 2 框自由矩形 0.4×0.4。
            if (!close(b0.camera.windowWidth, W) || !close(b0.camera.windowHeight, H)) {
                finish(false, "第 1 框窗口应=" + W + "x" + H + "，实际 "
                       + b0.camera.windowWidth + "x" + b0.camera.windowHeight); return
            }
            if (!close(b1.camera.windowWidth, W * 0.4) || !close(b1.camera.windowHeight, H * 0.4)) {
                finish(false, "第 2 框窗口应=" + (W * 0.4) + "x" + (H * 0.4) + "，实际 "
                       + b1.camera.windowWidth + "x" + b1.camera.windowHeight); return
            }
            // 逐框取景独立：居中框 scale=zoom；固定分段框此时用默认取景（没有分段表 → 1）。
            if (!near(b1.camera.scale, 2.0)) { finish(false, "居中框相机 scale 应为 2，实际 " + b1.camera.scale); return }
            if (near(b0.camera.scale, b1.camera.scale)) {
                finish(false, "两框相机 scale 相同（逐框取景没生效）: " + b0.camera.scale); return
            }
            say("两框独立取景 ✓  框1 " + b0.camera.windowWidth + "x" + b0.camera.windowHeight
                + " scale=" + b0.camera.scale + "，框2 " + b1.camera.windowWidth + "x" + b1.camera.windowHeight
                + " scale=" + b1.camera.scale)
            phase = "live-edit"
            wait = 0
            return
        }

        // 就地改第 2 框取景缩放：框数不变 → apply_map_frames 必须生效且不重载。
        if (phase === "live-edit") {
            if (wait++ === 0) {
                video.apply_map_frames(patchBox(layoutJson, 1, { viewZoom: 1.5 }))
                return
            }
            var f = frames()
            if (!f || f.count !== 2) { finish(false, "就地改取景后框数不是 2: " + JSON.stringify(f)); return }
            if (!near(f.boxes[1].viewZoom, 1.5)) {
                finish(false, "就地改的 viewZoom 没生效: " + f.boxes[1].viewZoom); return
            }
            if (!near(f.boxes[1].camera.scale, 1.5)) {
                finish(false, "就地改后居中框相机 scale 应为 1.5，实际 " + f.boxes[1].camera.scale); return
            }
            if (near(f.boxes[0].camera.scale, 1.5)) {
                finish(false, "改第 2 框把第 1 框也改了（取景器串了）"); return
            }
            say("就地改第 2 框 viewZoom=1.5 ✓  scale=" + f.boxes[1].camera.scale)
            phase = "style-edit"
            wait = 0
            return
        }

        // 逐框样式就地改：改第 2 框的路线样式（渲染模式/范围/宽度/颜色），第 1 框必须原样；
        // 断言「设置里的 style」和「取景器里实际生效的 appliedStyle」都变了。
        if (phase === "style-edit") {
            if (wait++ === 0) {
                video.apply_map_frames(patchStyle(layoutJson, 1, {
                    renderMode: 1, renderScope: 1, width: 20, routeColor: "#00ff00"
                }))
                return
            }
            var f = frames()
            if (!f || f.count !== 2 || !f.boxes[0].appliedStyle || !f.boxes[1].appliedStyle) {
                finish(false, "逐框样式：取景器状态缺失: " + JSON.stringify(f)); return
            }
            var b1s = f.boxes[1].style, b1a = f.boxes[1].appliedStyle
            var b0s = f.boxes[0].style, b0a = f.boxes[0].appliedStyle
            if (b1s.renderMode !== 1 || b1a.renderMode !== 1) {
                finish(false, "第 2 框路线渲染模式没生效: 设置=" + b1s.renderMode + " 生效=" + b1a.renderMode); return
            }
            if (b1a.renderScope !== 1 || !near(b1a.routeWidth, 20)) {
                finish(false, "第 2 框渲染范围/线宽没生效: " + JSON.stringify(b1a)); return
            }
            if (!(b1a.routeColor[1] > 0.9 && b1a.routeColor[0] < 0.1)) {
                finish(false, "第 2 框路线颜色没生效: " + JSON.stringify(b1a.routeColor)); return
            }
            if (b0a.renderMode !== 0 || b0a.renderScope !== 0 || !near(b0a.routeWidth, 10)) {
                finish(false, "改第 2 框样式影响了第 1 框取景器: " + JSON.stringify(b0a)); return
            }
            if (b0s.renderMode !== 0 || b0s.renderScope !== 0) {
                finish(false, "改第 2 框样式影响了第 1 框设置: " + JSON.stringify(b0s)); return
            }
            say("逐框样式就地生效 ✓  框1 模式=" + b0a.renderMode + "/宽 " + b0a.routeWidth
                + "，框2 模式=" + b1a.renderMode + "/宽 " + b1a.routeWidth)
            phase = "deselect"
            wait = 0
            return
        }

        // 在预览空白处取消选中（selected=-1）后：面板要继续显示/编辑**最后一个选中的框**，
        // 就像选中时一样（只是预览里不再高亮）。
        if (phase === "deselect") {
            if (wait++ === 0) {
                editor = findEditor(win.contentItem)
                if (!editor) { finish(false, "界面里找不到地图布局编辑器 mapLayoutEditor"); return }
                editor.selectFrame(1)
                return
            }
            if (wait === 2) { editor.selected = -1; return }
            if (wait < 4) return
            if (editor.selected !== -1) { finish(false, "取消选中没生效: " + editor.selected); return }
            if (editor.activeIndex !== 1) {
                finish(false, "取消选中后应回退到最后一个选中的框（1），实际 " + editor.activeIndex); return
            }
            if (editor.currentBox === null) { finish(false, "取消选中后仍要显示最后选中框的信息"); return }
            if (Number(editor.currentBox.mode) !== 4) {
                finish(false, "回退显示的应是被最后选中的自由框（mode=4），实际 mode=" + editor.currentBox.mode); return
            }
            if (editor.current !== null) { finish(false, "取消选中后预览里不应再高亮"); return }
            var f = frames()
            if (!f || f.count !== 2) { finish(false, "取消选中状态不应改变框数: " + JSON.stringify(f)); return }
            say("取消选中后仍显示最后选中框的信息 ✓")
            editor.selectFrame(0)
            phase = "editor-add"
            wait = 0
            return
        }

        // 编辑器加框 = 框数变化 → 必须走完整重载并最终有 3 套取景器。
        if (phase === "editor-add") {
            if (wait++ === 0) {
                editor = findEditor(win.contentItem)
                if (!editor) { finish(false, "界面里找不到地图布局编辑器 mapLayoutEditor"); return }
                say("编辑器 boxCount=" + editor.boxCount)
                editor.addBox()
                return
            }
            if (busy()) { if (wait > 300) finish(false, "加框后完整重载超时（busy 一直不落下）"); return }
            var f = frames()
            if (f && f.count === 3) {
                say("编辑器加框 → 完整重载后 3 框 ✓")
                wait = 0
                phase = "editor-remove"
                return
            }
            if (wait > 300) { finish(false, "编辑器加框后框数不是 3: " + JSON.stringify(f)); return }
            return
        }

        // 编辑器删框（删刚加的选中框）→ 回到 2 框。
        if (phase === "editor-remove") {
            if (wait++ === 0) {
                editor.removeBox()
                return
            }
            if (busy()) { if (wait > 300) finish(false, "删框后完整重载超时（busy 一直不落下）"); return }
            var f = frames()
            if (f && f.count === 2) {
                say("编辑器删框 → 完整重载后回到 2 框 ✓")
                // 重新取一份布局 JSON（完整重载可能把取景参数收敛过）。
                layoutJson = String(win.debugGet("map_boxes"))
                wait = 0
                phase = "structure-reject"
                return
            }
            if (wait > 300) { finish(false, "编辑器删框后框数不是 2: " + JSON.stringify(f)); return }
            return
        }

        // 框数变化：必须被 apply_map_frames 拒绝（由 QML 走完整重载），内存里仍是 2 框。
        if (phase === "structure-reject") {
            if (wait++ === 0) {
                var one = JSON.parse(layoutJson)
                one.boxes = [one.boxes[0]]
                video.apply_map_frames(JSON.stringify(one))
                return
            }
            var f = frames()
            if (!f || f.count !== 2) {
                finish(false, "框数变化的 JSON 应被拒绝、仍保持 2 框，实际 " + (f ? f.count : "null")); return
            }
            say("框数变化被拒绝（仍 2 框）✓")
            // 拖一下播放头再确认预览没崩（相机/取景器还能用）。
            video.seek(2000)
            phase = "seek"
            wait = 0
            return
        }

        if (phase === "seek") {
            if (wait++ < 12) return
            var f = frames()
            if (!f || f.count !== 2 || !f.boxes[1].camera) {
                finish(false, "seek 后取景器状态异常: " + JSON.stringify(f)); return
            }
            finish(true, "2 框工程加载 + 逐框取景独立 + apply_map_frames 就地更新（取景/逐框样式）+ 编辑器加/删框完整重载 + 框数变化拒绝 + seek 均正常")
            return
        }
    }
}
