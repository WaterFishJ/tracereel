import QtQuick
import QtQuick.Window
import TraceReel 1.0

Window {
    width: 480; height: 270; visible: true
    property int phase: 0
    property string report: "SOURCE_FREE_EXPORT_PENDING"
    function debugState() { return report }
    TraceReelItem { id: preview; anchors.fill: parent }
    Component.onCompleted: preview.reload(orvPath)
    Timer { interval: 16; running: true; repeat: true; onTriggered: preview.tick() }
    Timer {
        interval: 100; running: true; repeat: true
        onTriggered: {
            if (preview.busy || preview.duration_ms <= 0) return
            if (phase === 0) {
                phase = 1
                // The harness passes a disposable project copy in its artifact directory.
                preview.start_encode(String(orvPath).replace(/\.trr$/i, "-export.mp4"))
            } else if (preview.encode_done) {
                report = preview.encode_error.length || preview.encode_aborted
                    ? "SOURCE_FREE_EXPORT_FAIL: " + preview.encode_error
                    : "SOURCE_FREE_EXPORT_PASS: " + preview.encode_output
                console.log(report)
                stop(); Qt.quit()
            }
        }
    }
}
