// 探针：驱动真实 gui.qml 跑一次**带防抖**的导出，专测「导出解码尺寸 ≠ 预览防抖尺寸」时
// 在编码线程上按导出尺寸重新加载防抖这条路（GUI 传进来的是预览那套限高 720 的稳定化器，
// gyroflow 的稳定化器与帧尺寸强绑定，不重载就会 SizeMismatch）。
// 由 app/tests/test_export_decode_size_gyro.ps1 驱动；需要真实带运动数据的素材与一段很短
// 的 clip（脚本会把 clipEndMs 写小，否则要导整段视频）。
//
// 判据打在这一行里：`EXPORT_GYRO_PASS:` / `EXPORT_GYRO_FAIL:`
import QtQuick
import QtQuick.Controls

Item {
    id: probe
    width: 8
    height: 8

    property var win: null
    property var video: null
    property int step: 0
    property int wait: 0
    property int encodedFrames: 0
    property string phase: "init"
    property string report: "PROBE 未开始"

    function debugState() { return report }
    function say(s) { console.log("[probe] " + s) }
    function finish(ok, why) {
        report = (ok ? "EXPORT_GYRO_PASS: " : "EXPORT_GYRO_FAIL: ") + why
        say(report)
        timer.stop()
        Qt.quit()
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
        interval: 500
        repeat: true
        onTriggered: probe.advance()
    }

    function advance() {
        step++
        if (step > 2400) { finish(false, "总步数超限（约 20 分钟）"); return }

        if (phase === "init") {
            video = findVideoItem(win.contentItem)
            if (!video) {
                if (step > 20) finish(false, "界面里找不到预览项 TraceReelItem")
                return
            }
            say("找到预览项，等预览防抖加载完（要读完整段运动数据）")
            phase = "wait_ready"
            return
        }

        // 预览那套稳定化器先按 720 级尺寸加载好 —— 这正是导出要面对的前提。
        if (phase === "wait_ready") {
            if (!video.gyro_loading && !video.busy && String(video.source_size).length > 0) {
                say("就绪：源 " + video.source_size + "，开始导出（应提示按导出尺寸重新准备防抖）")
                win.startExport()
                wait = 0
                phase = "starting"
                return
            }
            if (step > 900) {
                finish(false, "等待预览防抖就绪超时: busy=" + video.busy + " gyro_loading=" + video.gyro_loading)
                return
            }
            return
        }

        if (phase === "starting") {
            if (video.encoding) {
                say("导出已启动；弹窗提示=" + video.encode_notice)
                wait = 0
                phase = "exporting"
                return
            }
            if (String(video.encode_error).length > 0) { finish(false, "导出没启动就报错: " + video.encode_error); return }
            if (wait++ > 40) {
                finish(false, "导出没有启动（检查工程里的输出文件路径与 clip 设置）: " + win.debugPreviewState())
                return
            }
            return
        }

        if (phase === "exporting") {
            if (video.encoding) {
                // GUI 这一路没法从工程里限制时长（剪辑范围不是代理属性，临时设置文件写的是
                // 默认值），整段要几十分钟 —— 所以编够一小段就主动中止，只验证「按导出尺寸
                // 重新加载防抖」这条路能正常出帧。
                if (video.encode_frame >= 60) {
                    encodedFrames = video.encode_frame
                    say("已编码 " + encodedFrames + " 帧，主动中止（整段要几十分钟）")
                    video.abort_encode()
                    wait = 0
                    phase = "aborting"
                    return
                }
                if (wait % 20 === 0) {
                    say("导出中 " + video.encode_frame + "/" + video.encode_total_frames
                        + " 弹窗提示=" + video.encode_notice)
                }
                wait++
                if (wait > 2400) { finish(false, "导出长时间没有开始出帧"); return }
                return
            }
            if (String(video.encode_error).length > 0 && !video.encode_aborted) {
                finish(false, "导出报错: " + video.encode_error); return
            }
            if (video.encode_done) {
                finish(true, "带防抖导出完成（" + video.encode_total_frames + " 帧）")
                return
            }
            if (wait++ > 60) { finish(false, "导出状态没收敛: " + win.debugPreviewState()); return }
            return
        }

        if (phase === "aborting") {
            if (!video.encoding) {
                if (String(video.encode_error).length > 0 && !video.encode_aborted) {
                    finish(false, "中止时导出报错: " + video.encode_error)
                    return
                }
                finish(true, "按导出尺寸重新加载防抖后正常编码 " + encodedFrames + " 帧（随后主动中止；中止是预期结果）")
                return
            }
            if (wait++ > 200) { finish(false, "中止没有生效"); return }
            return
        }
    }
}
