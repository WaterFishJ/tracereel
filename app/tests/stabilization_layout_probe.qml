import QtQuick
import TraceReel 1.0
// Drive with qmlprobe using QMLPROBE_FILE and QMLPROBE_RUN_LOOP=1.
// QMLPROBE_TIMEOUT_MS=2500; require STABILIZATION_LAYOUT_PASS in stderr.
Item {
    id: probe
    property var win
    property var smoothing
    property var advanced
    property string testPalette: "light"
    property var videoScroll
    property int rows: 0
    property bool failed: false
    property bool completed: false
    function debugState() { return !completed ? "STABILIZATION_LAYOUT_PENDING" : failed ? "STABILIZATION_LAYOUT_FAIL" : "STABILIZATION_LAYOUT_PASS" }
    function check(ok, message) {
        if (!ok) { failed=true; console.error("STABILIZATION_LAYOUT_FAIL: "+message) }
    }
    function walk(node, setup) {
        if(!node)return
        if(setup) {
            if(node.objectName==="settingsTabs")node.currentIndex=2
            if(node.prop==="gyro_enabled" && node.children) {
                for(var k=0;k<node.children.length;k++) {
                    var toggle=node.children[k]
                    if(toggle.checked!==undefined && toggle.toggled && !toggle.checked) {
                        toggle.checked=true;toggle.toggled()
                    }
                }
            }
            if(node.advancedOpen!==undefined) { smoothing=node;node.advancedOpen=true }
            if(node.opened!==undefined && node.parent===smoothing)advanced=node
        } else if(node.spRef!==undefined) {
            rows++
            check(node.width<=node.parent.width+0.5,"parameter row exceeds container")
            for(var j=0;j<node.children.length;j++) {
                var child=node.children[j]
                if(child.visible && child.width>0) {
                    check(child.x>=-0.5 && child.x+child.width<=node.width+0.5,"parameter child is clipped")
                    if(child.font && child.font.family)check(child.font.family===win.font.family,"parameter font differs")
                    if(child.fontInfo) {
                        check(child.fontInfo.family===win.font.family,"font unexpectedly fell back")
                        check(String(child.color)===String(Theme.text),"parameter label is not primary text")
                    }
                }
            }
        }
        if(node.children)for(var i=0;i<node.children.length;i++)walk(node.children[i],setup)
    }
    function scrollToSmoothing(node) {
        if(!node)return
        if(node.contentY!==undefined && node.contentItem && smoothing) {
            var point=smoothing.mapToItem(node.contentItem,0,0)
            if(point.y>0 && point.y<node.contentHeight)node.contentY=point.y
        }
        if(node.children)for(var i=0;i<node.children.length;i++)scrollToSmoothing(node.children[i])
    }
    Component.onCompleted: {
        var comp=Qt.createComponent("../src/bin/gui.qml")
        if(comp.status!==Component.Ready){console.error(comp.errorString());Qt.quit();return}
        win=comp.createObject(null)
        if(win){win.width=1440;win.height=1000}
    }
    Timer {interval:50;running:true;repeat:false;onTriggered:{Theme.apply_theme(testPalette);walk(win.contentItem,true)}}
    Timer {
        interval:150;running:true;repeat:false
        onTriggered: {
            check(smoothing!==undefined,"smoothing controls missing")
            walk(win.contentItem,false)
            check(rows>4,"missing expanded parameter rows")
            scrollToSmoothing(win.contentItem)
        }
    }
    Timer {
        interval:1400;running:true;repeat:false
        onTriggered:{
            check(advanced!==undefined,"advanced section missing")
            if(advanced)check(Math.abs(advanced.height-advanced.expandedHeight)<1,"expanded animation clips content")
            completed=true
            if(!failed)console.log("STABILIZATION_LAYOUT_PASS: "+rows+" rows fit, fonts and colors match; expansion completes")
        }
    }
}
