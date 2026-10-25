import QtQuick
import TraceReel 1.0

// Real gui.qml: off -> on -> off -> on, keeping parameter values throughout.
// Run with qmlprobe, QMLPROBE_RUN_LOOP=1, QMLPROBE_TIMEOUT_MS=6000.
Item {
    id: probe
    property var win
    property var options
    property var toggle
    property var smoothing
    property var preset
    property int stage: 0
    property string before: ""
    property string report: "STABILIZATION_TOGGLE_PENDING"
    property bool failed: false
    function debugState() { return report }
    function check(ok, why) {
        if(!ok){failed=true;console.error("STABILIZATION_TOGGLE_FAIL: "+why)}
    }
    function visit(node, callback) {
        if(!node)return
        callback(node)
        if(node.children)for(var i=0;i<node.children.length;i++)visit(node.children[i],callback)
    }
    function values() {
        var result=[]
        visit(smoothing,function(node){
            if(node.spRef!==undefined && node.children)
                for(var i=0;i<node.children.length;i++)
                    if(node.children[i].editingFinished)result.push(String(node.children[i].text))
        })
        return JSON.stringify(result)
    }
    function setEnabled(on) { toggle.checked=on;toggle.toggled() }
    Component.onCompleted: {
        var component=Qt.createComponent("../src/bin/gui.qml")
        win=component.createObject(null)
        if(!win){console.error(component.errorString());Qt.quit();return}
        visit(win.contentItem,function(node){
            if(node.objectName==="settingsTabs")node.currentIndex=2
            if(node.objectName==="stabilizationOptions")options=node
            if(node.advancedOpen!==undefined)smoothing=node
            if(node.prop==="gyro_preset")preset=node
            if(node.prop==="gyro_enabled" && node.children)
                for(var i=0;i<node.children.length;i++)
                    if(node.children[i].checked!==undefined && node.children[i].toggled)toggle=node.children[i]
            check(node.title!=="稳定化","separate stabilization card remains")
        })
    }
    Timer {
        interval:700;running:true;repeat:true
        onTriggered: {
            if(!options || !toggle || !smoothing || !preset){check(false,"controls missing");Qt.quit();return}
            if(stage===0) {
                check(!toggle.checked,"new project must start with stabilization off")
                check(!options.visible && options.height===0,"off state must not occupy space")
                check(!preset.visible && !smoothing.visible,"off state exposes configuration")
                before=values()
                setEnabled(true)
            } else if(stage===1 || stage===3) {
                check(options.visible && options.expanded,"on state does not reveal configuration")
                check(preset.visible && smoothing.visible,"preset/smoothing controls missing after enabling")
                check(Math.abs(options.height-options.children[0].implicitHeight)<1,"expanded content is clipped")
                check(values()===before,"toggle changed smoothing parameters")
                if(stage===1)setEnabled(false)
                else {
                    report=failed?"STABILIZATION_TOGGLE_FAIL":"STABILIZATION_TOGGLE_PASS"
                    console.log(report);stop();Qt.quit()
                }
            } else if(stage===2) {
                check(!options.visible && options.height===0,"disabling leaves an empty configuration area")
                check(!preset.visible && !smoothing.visible,"configuration still visible while disabled")
                setEnabled(true)
            }
            stage++
        }
    }
}
