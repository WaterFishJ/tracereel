// 自动化回归：复现并守住「播放中改轨迹显示 → 预览重载 → 视频黑屏 / 地图与 HUD 冻住 /
// 只有进度条在走」这个问题。由 qmlprobe 驱动真实的 app/src/bin/gui.qml。
//
// 用法（仓库根目录，先 .\build.ps1 build --bin tracereel-gui --bin qmlprobe）：
//   $env:QMLPROBE_FILE="$PWD\app\tests\route_display_probe.qml"
//   $env:QMLPROBE_RUN_LOOP="1"
//   $env:QMLPROBE_TIMEOUT_MS="600000"          # 10 分钟硬上限（需自备 4K 素材）
//   .\build.ps1 run --bin qmlprobe
//
// 判据（打在 stdout 的 [probe] 行与最后一行 [loop final] 里）：
//   1. 播放中 frames（item.debug_frames，已画到画面的预览帧数）持续增长 → 画面确实在出新帧；
//   2. 播放中改「轨迹显示」类设置（这里用轨迹线宽）→ Rust 侧 kind=「就地更新」（不是
//      「完整重载」），busy 没变 true，日志是「已更新 · …」，frames 继续增长
//      → 就地更新，没有整链重载、没有黑屏；
//   2b. 播放中在真实界面上把「尾巴渲染模式」切到「配速」（尾巴的第三态）→ 设置必须写回原版
//      编号 3（整数），kind 仍是「就地更新」，尾巴宽度保持可见、尾巴颜色转为隐藏、共用的
//      「配速色带自适应」跟着出现，frames 继续增长；
//   3. 播放中改「非显示」类设置（这里用录像出发时间偏移 +1 秒，必走完整重载）→ kind=
//      「完整重载」，播放位置 t_ms 没有被清零、没有被拽回开头，且 busy 落下后 frames 自动
//      恢复增长 → 重载完成即恢复，不需要用户再拖一次进度条或暂停再播放。
// 说明：场景 2 的值一律用"读回当前值再 +1"，否则写进去的可能和项目里已有的值相同，
// 改动成了空操作 → 白名单判"无差异" → 测不到重载（曾经因此假通过一次）。
import QtQuick

Item {
    id: probe
    width: 8
    height: 8

    // 默认工程结构完整但不含素材；真跑这个探针前，请把 demo/example.trr 里的路径改成真实文件。
    readonly property url orvUrl: Qt.resolvedUrl("../../demo/example.trr")

    property var win: null
    property int step: 0
    property int wait: 0
    property string phase: "init"
    property string report: "PROBE 未开始"
    property real frames0: 0
    property real framesA: 0
    property real framesB: 0
    property real tB: 0
    property real tBeforeFull: 0
    property real landed0: 0
    property bool busySeen: false
    property int tailBefore: -1
    property int tailExpect: -1
    property real tailFrames: 0

    // qmlprobe 在事件循环结束时会打印这个（[loop final] …）。
    function debugState() { return report }

    // 递归找真实控件（ApplicationWindow 的内容项往下走）。轨迹页放在 StackLayout 里，
    // 所有页都是预先建好的，所以不用切页签就能拿到「显示」组里的行。
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
    // 行里那个真正能点的勾选框（QQC2 CheckBox 有 toggle()）。
    function findCheckBox(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && typeof c.toggle === "function") return c
            var r = findCheckBox(c)
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

    function state() { return win ? win.debugPreviewState() : "(窗口未创建)" }
    function num(key) {
        var m = state().match(new RegExp(key + "=([0-9]+)"))
        return m ? parseInt(m[1]) : -1
    }
    function frames() { return num("frames") }
    function tms() { return num("t_ms") }
    function dur() { return num("dur") }
    function busy() { return state().indexOf(" busy=true") >= 0 }
    function lastLog() {
        var m = state().match(/log=\[(.*)\]$/)
        return m ? m[1] : ""
    }
    // Rust 侧最近一次改动走的路：「完整重载」/「就地更新」。这是"有没有重载"的直接证据，
    // 不能靠日志文案或 busy 猜（busy 是后台线程异步拉起的）。
    function kind() {
        var m = state().match(/kind=([^ ]*)/)
        return m ? m[1] : ""
    }
    function landed() { return num("landed") }
    function say(s) { console.log("[probe] " + s) }

    function finish(ok, why) {
        report = (ok ? "PROBE OK: " : "PROBE FAILED: ") + why + " | " + state()
        say(report)
        timer.stop()
        Qt.quit()
    }

    Component.onCompleted: {
        var comp = Qt.createComponent(Qt.resolvedUrl("../src/bin/gui.qml"))
        if (comp.status !== Component.Ready) {
            finish(false, "gui.qml 加载失败: " + comp.errorString())
            return
        }
        win = comp.createObject(null)
        if (!win) {
            finish(false, "gui.qml 实例化失败（createObject 返回空）")
            return
        }
        say("gui.qml 已实例化: " + orvUrl)
        phase = "load"
        timer.start()
    }

    Timer {
        id: timer
        interval: 400
        repeat: true
        onTriggered: probe.advance()
    }

    function advance() {
        step++
        if (step > 4000) { finish(false, "总步数超限（约 27 分钟）"); return }

        if (phase === "load") {
            if (wait === 0) { say("打开项目: " + orvUrl); win.loadProject(orvUrl); wait = 1; return }
            if (!busy() && dur() > 0 && frames() > 0) {
                say("加载完成 " + state())
                phase = "play"
                return
            }
            if (step > 250) { finish(false, "等待加载超时（busy / dur / frames 未就绪）: " + state()); return }
            return
        }

        if (phase === "play") {
            say("开始播放 " + win.debugPlay(true))
            frames0 = frames(); wait = 0
            phase = "playing"
            return
        }

        // 先确认「播放中确实在出新帧」，否则后面的判据没有意义。
        // 容忍最多 30 秒：项目刚加载时防抖平滑在后台算，预览会短暂停帧
        // （`[preview] dropped N of last 600 frames`），那是另一回事，不能误判成本 bug。
        if (phase === "playing") {
            if (frames() > frames0) {
                framesA = frames()
                say("播放中出帧正常: frames " + frames0 + " → " + framesA + "，t_ms=" + tms())
                phase = "apply_display"; wait = 0
                return
            }
            if (wait++ >= 75) {
                finish(false, "播放中 frames 长时间没有增长（" + frames0 + " → " + frames() + "）：预览本身没在出帧，判据失效")
                return
            }
            return
        }

        // 主场景：播放中改轨迹线宽（显示项）——应当就地更新，不重载、不黑屏。
        if (phase === "apply_display") {
            var s = win.debugApplyProp("route_width", 6, "回归：改轨迹线宽")
            say("改显示项后: " + s)
            var wRead = win.debugGet("route_width")
            if (wRead !== 6) {
                finish(false, "代理属性没写进去（route_width=" + wRead + "）：测试本身失效")
                return
            }
            if (kind() !== "就地更新") {
                finish(false, "改轨迹显示走了「" + kind() + "」而不是「就地更新」：轻量路径没生效")
                return
            }
            if (busy()) { finish(false, "改轨迹显示触发了完整重载（busy=true）：轻量路径没生效"); return }
            framesB = frames(); tB = tms(); wait = 0
            phase = "after_display"
            return
        }

        if (phase === "after_display") {
            if (frames() > framesB) {
                if (tms() <= tB) {
                    finish(false, "改显示项之后进度不再前进（t_ms " + tB + " → " + tms() + "）")
                    return
                }
                say("就地更新成功: frames " + framesB + " → " + frames() + "，t_ms " + tB + " → " + tms())
                // 第二场景：播放中在真实界面上把「尾巴渲染模式」切到「配速」——也必须就地更新。
                // 这一条专门守尾巴的第三态：真实下拉框的 items/values 映射（编号 3 = 配速）、
                // 写回值必须是整数 3（route_tail_render_mode 是 i32 代理属性，写成字符串会写不进去），
                // 以及切换后尾巴颜色/色带设置的联动显隐 —— 桩测试测不到这一层。
                tailFrames = frames()
                tailBefore = win.debugGet("route_tail_render_mode")
                wait = 0
                phase = "tail_mode"
                return
            }
            if (wait++ >= 75) {
                finish(false, "改显示项之后 frames 长时间不再增长（" + framesB + " → " + frames() + "）＝黑屏症状复现")
                return
            }
            return
        }

        // 第二场景：播放中在真实界面上把「尾巴渲染模式」切到「配速」——也必须就地更新、不黑屏。
        if (phase === "tail_mode") {
            var tailRow = findByLabel(win.contentItem, "尾巴渲染模式")
            if (!tailRow) { finish(false, "真实界面上找不到「尾巴渲染模式」选择器"); return }
            if (tailRow.prop !== "route_tail_render_mode") {
                finish(false, "「尾巴渲染模式」绑定的属性不对: " + tailRow.prop); return
            }
            if (!tailRow.items || tailRow.items.length !== 3
                || tailRow.items[2] !== "配速" || tailRow.values[2] !== 3) {
                finish(false, "「尾巴渲染模式」选项不对: items=" + tailRow.items + " values=" + tailRow.values); return
            }
            var tailCombo = findComboBox(tailRow)
            if (!tailCombo) { finish(false, "「尾巴渲染模式」行里找不到下拉框"); return }
            // 当前不是「配速」就切到「配速」，否则切回「简洁」——保证这次操作真的改了设置。
            tailExpect = (tailBefore === 3) ? 1 : 3
            var tailIdx = tailRow.values.indexOf(tailExpect)
            say("切换「尾巴渲染模式」前: 设置=" + tailBefore + "，当前项=" + tailCombo.currentIndex + "，切到 " + tailExpect)
            // 复现真实选择：真机点开会走 ComboBox 的 onActivated，设置是在那里写进去的。
            tailCombo.currentIndex = tailIdx
            tailCombo.activated(tailIdx)
            wait = 0
            phase = "tail_mode_wait"
            return
        }

        if (phase === "tail_mode_wait") {
            var tailNow = win.debugGet("route_tail_render_mode")
            if (tailNow === tailExpect) {
                if (kind() !== "就地更新") {
                    finish(false, "切「尾巴渲染模式」走了「" + kind() + "」而不是「就地更新」：轻量路径没生效"); return
                }
                if (busy()) { finish(false, "切「尾巴渲染模式」触发了完整重载（busy=true）"); return }
                var wRow = findByLabel(win.contentItem, "尾巴宽度 (像素)")
                if (wRow && wRow.visible !== true) {
                    finish(false, "「尾巴宽度 (像素)」在尾巴模式=" + tailExpect + " 时该显示: visible=" + wRow.visible); return
                }
                // 配速模式的颜色来自色带，所以「尾巴颜色」要消失；尾巴配速设置独立显示。
                var cRow = findByLabel(win.contentItem, "尾巴颜色")
                var colorWant = (tailExpect !== 3)
                if (cRow && cRow.visible !== colorWant) {
                    finish(false, "「尾巴颜色」显隐不对（尾巴模式=" + tailExpect + "，期望 visible=" + colorWant + "，实际 " + cRow.visible + "）"); return
                }
                var bRow = findByLabel(win.contentItem, "尾巴配速色带自适应")
                var bandWant = (tailExpect === 3)
                if (bRow && bRow.visible !== bandWant) {
                    finish(false, "「配速色带自适应」显隐不对（期望 visible=" + bandWant + "，实际 " + bRow.visible + "）"); return
                }
                say("「尾巴渲染模式」已写回: " + tailBefore + " → " + tailExpect
                    + "，尾巴颜色行 visible=" + (cRow ? cRow.visible : "(没找到)")
                    + "，色带设置行 visible=" + (bRow ? bRow.visible : "(没找到)") + "，kind=就地更新")
                wait = 0
                phase = "after_tail"
                return
            }
            if (wait++ >= 25) {
                finish(false, "切了「尾巴渲染模式」但设置没写回（期望 " + tailExpect + "，实际 " + tailNow + "）：下拉框没生效或写成了非整数")
                return
            }
            return
        }

        if (phase === "after_tail") {
            if (frames() > tailFrames) {
                say("切「尾巴渲染模式」后预览继续出帧: frames " + tailFrames + " → " + frames() + "，t_ms=" + tms())
                // 第三场景：播放中改**非显示**项（录像-出发时间偏移 +1 秒）——必须走完整重载。
                // 用"当前值 + 1"保证真的和项目里的值不同，否则改动是空操作、测不到重载。
                tBeforeFull = tms()
                landed0 = landed()
                var cur = win.debugGet("video_start_offset")
                say("改非显示项后: " + win.debugApplyProp("video_start_offset", cur + 1.0, "回归：改非显示项（完整重载）"))
                var curRead = win.debugGet("video_start_offset")
                say("非显示项读回: " + curRead + "（原值 " + cur + "）")
                if (curRead === cur) {
                    finish(false, "代理属性没写进去（video_start_offset 仍是 " + cur + "）：测试本身失效")
                    return
                }
                if (kind() !== "完整重载") {
                    finish(false, "改非显示项（录像-出发时间）走了「" + kind() + "」而不是「完整重载」：白名单漏了 video.startTimeOffset")
                    return
                }
                framesB = frames(); wait = 0
                busySeen = busy()      // `start_async_apply` 同步置 busy，这里通常已经是 true
                phase = "full_reload"
                return
            }
            if (wait++ >= 75) {
                finish(false, "切「尾巴渲染模式」之后 frames 长时间不再增长（" + tailFrames + " → " + frames() + "）＝黑屏症状复现")
                return
            }
            return
        }

        // 第二场景：播放中改非显示项 —— 允许完整重载，但位置必须保住、并且自动恢复出帧。
        // 等"加载中"拉起再落下（`apply_settings` 里的 `start_async_apply` 会同步置 busy）；
        // 万一那次加载快到没被采样到，也至少等 6 秒再判，避免在重载落地前就下结论。
        // 完整重载**落地**次数：等它涨了才说明这次重载真的算完了（不靠定时猜）。
        if (phase === "full_reload") {
            if (busy()) busySeen = true
            if (landed() > landed0) {
                if (tms() <= 0) { finish(false, "完整重载把播放位置清零了（t_ms=" + tms() + "）"); return }
                if (tms() + 3000 < tBeforeFull) {
                    finish(false, "完整重载后位置被拽回: t_ms " + tBeforeFull + " → " + tms())
                    return
                }
                say("完整重载结束: t_ms=" + tms() + "（改设置前 " + tBeforeFull + "），busy 观察到=" + busySeen)
                framesB = frames(); wait = 0
                phase = "after_full"
                return
            }
            if (wait++ >= 150) {
                finish(false, "完整重载迟迟没有落地（landed 没变）: " + state())
                return
            }
            return
        }

        if (phase === "after_full") {
            if (frames() > framesB) {
                finish(true, "改显示项＝就地更新不重载；改非显示项＝完整重载但保住播放位置并自动恢复出帧（frames "
                            + framesB + " → " + frames() + "，busy 观察到=" + busySeen + "）")
                return
            }
            if (wait++ >= 75) {
                finish(false, "完整重载落地后 frames 长时间不再增长（" + framesB + " → " + frames() + "）＝仍需手动拖进度条")
                return
            }
            return
        }
    }
}