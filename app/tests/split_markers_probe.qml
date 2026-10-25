import QtQuick

// Fixture: relative splits 1|1|1, video departure offset 1 second, duration 5 sec.
Item {
    property var win
    property var video
    property var markers
    property int stage:0
    property int ticks:0
    property string report:"SPLIT_MARKERS_PENDING"
    function debugState(){return report}
    function finish(ok,why){report=(ok?"SPLIT_MARKERS_PASS: ":"SPLIT_MARKERS_FAIL: ")+why;console.log(report);timer.stop();Qt.quit()}
    function visit(node) {
        if(!node)return
        if(node.source_size!==undefined && node.outputAspect!==undefined)video=node
        if(node.objectName==="previewSplitMarkers")markers=node
        if(node.text==="跳转")finish(false,"old row jump button remains")
        if(node.children)for(var i=0;i<node.children.length;i++)visit(node.children[i])
    }
    Component.onCompleted:{
        var component=Qt.createComponent("../src/bin/gui.qml")
        win=component.createObject(null)
        if(!win){finish(false,component.errorString());return}
        visit(win.contentItem);timer.start()
    }
    Timer {id:timer;interval:100;repeat:true;onTriggered:{
        ticks++
        if(ticks>100){finish(false,"preview load timeout");return}
        if(!video || !markers || video.busy || video.duration_ms<=0)return
        if(stage===0) {
            var points=markers.visiblePoints
            if(points.length!==3){finish(false,"expected 3 markers, got "+points.length);return}
            for(var i=0;i<3;i++)if(Math.abs(points[i].timeMs-(i+1)*1000)>1){finish(false,"incorrect relative split/offset");return}
            if(points[1].tooltip.indexOf("31 → 32")<0){finish(false,"point codes missing from tooltip");return}
            markers.activated(1);stage=1
        } else {
            finish(Math.abs(video.t_ms-2000)<1,"relative splits + video offset; marker 2 seeks to "+video.t_ms+"ms")
        }
    }}
}
