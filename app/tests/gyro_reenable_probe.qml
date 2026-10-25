import QtQuick

// Load a real motion-bearing project, then exercise the actual GUI switch.
// QMLPROBE_ORV supplies the project; the source project is never saved/changed.
Item {
    property var win
    property var video
    property var toggle
    property string phase: "initial"
    property string report: "GYRO_REENABLE_PENDING"
    property real landed: 0
    property int ticks: 0
    property bool restarted: false
    function debugState() { return report }
    function finish(ok, detail) {
        report=(ok?"GYRO_REENABLE_PASS: ":"GYRO_REENABLE_FAIL: ")+detail
        console.log(report);timer.stop();Qt.quit()
    }
    function visit(node) {
        if(!node)return
        if(node.outputAspect!==undefined && node.source_size!==undefined)video=node
        if(node.prop==="gyro_enabled" && node.children)
            for(var i=0;i<node.children.length;i++)
                if(node.children[i].checked!==undefined && node.children[i].toggled)toggle=node.children[i]
        if(node.children)for(var j=0;j<node.children.length;j++)visit(node.children[j])
    }
    function setEnabled(on) { toggle.checked=on;toggle.toggled() }
    Component.onCompleted: {
        var component=Qt.createComponent("../src/bin/gui.qml")
        win=component.createObject(null)
        if(!win){finish(false,component.errorString());return}
        visit(win.contentItem)
        if(!video || !toggle){finish(false,"preview/switch missing");return}
        timer.start()
    }
    Timer {id:timer;interval:250;repeat:true;onTriggered:advance()}
    function advance() {
        ticks++
        var status=String(video.gyro_status)
        if(ticks>2400){finish(false,"timeout in "+phase+": "+status);return}
        if(phase==="initial") {
            if(video.busy || video.gyro_loading)return
            if(status.indexOf("已就绪")<0) {
                if(status.indexOf("无运动数据")>=0 || status.indexOf("加载失败")>=0)finish(false,"initial: "+status)
                return
            }
            console.log("GYRO_REENABLE_INITIAL_READY: "+status)
            landed=video.debug_full_landed
            setEnabled(false);phase="off"
        } else if(phase==="off") {
            if(status!=="防抖: 关闭")return
            console.log("GYRO_REENABLE_OFF: "+status)
            setEnabled(true);phase="reenabling"
        } else {
            if(status.indexOf("无运动数据")>=0 || status.indexOf("加载失败")>=0){finish(false,"reenabled: "+status);return}
            if(video.gyro_loading && !restarted) {
                restarted=true;console.log("GYRO_REENABLE_LOADING: "+status)
            }
            if(!video.gyro_loading && status.indexOf("已就绪")>=0) {
                if(!restarted){finish(false,"no stabilizer reload observed");return}
                if(video.debug_full_landed!==landed){finish(false,"unexpected full video/map reload");return}
                finish(true,"initial ready -> off -> loading -> ready; video metadata retained")
            }
        }
    }
}
