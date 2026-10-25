import QtQuick
import QtQuick.Window
import TraceReel 1.0
Window {
    id: probe
    visible:true;width:320;height:240
    property var targets:[0,0,1000,1000,1033,1000,8000,2000,2000,1000,0,11000,1000]
    property int index:-1
    property int waits:0
    property real before:0
    property string phase:"paused"
    property var playingTargets:[8000,1000,1000,2000,1000]
    property var rapidTargets:[5000,1000,8000,2000]
    property int rapidIndex:0
    property bool extended:false
    property string report:"PREVIEW_SEEK_PENDING"
    function debugState(){return report}
    TraceReelItem {id:video;anchors.fill:parent}
    Component.onCompleted:{
        // Every whole second in both directions, in addition to repeated/sub-frame seeks.
        var seconds=[]
        for(var i=0;i<12;i++)seconds.push(i*1000)
        for(var j=11;j>=0;j--)seconds.push(j*1000)
        targets=targets.concat(seconds)
        video.reload(orvPath)
    }
    Timer {interval:16;running:true;repeat:true;onTriggered:video.tick()}
    Timer {
        id:rapid
        interval:20;repeat:true
        onTriggered:{
            before=video.debug_frames
            video.seek(rapidTargets[rapidIndex++])
            if(rapidIndex>=rapidTargets.length){stop();phase="waitRapid";waits=0}
        }
    }
    Timer {
        interval:50;running:true;repeat:true
        onTriggered:{
            if(video.busy || video.gyro_loading || video.duration_ms<=0 || video.debug_frames<=0)return
            if(!extended) {
                extended=true
                if(video.duration_ms>611000)
                    targets=targets.concat([17000,17000,18000,19000,600000,610000,17000,16000,17000])
            }
            if(phase==="rapid")return
            if(phase==="waitRapid" || phase==="playing") {
                var target=phase==="waitRapid"?2000:playingTargets[index]
                waits++
                if(video.debug_frames<=before || video.debug_frame_ms<target-20 || video.debug_frame_ms>target+(phase==="playing"?500:20)) {
                    if(waits>160){report="PREVIEW_SEEK_FAIL: "+phase+" target="+target+" displayed="+video.debug_frame_ms;console.log(report);Qt.quit()}
                    return
                }
                console.log("SEEK_LANDED: "+phase+" "+target+" displayed="+video.debug_frame_ms)
                if(phase==="waitRapid"){phase="playing";index=0;video.set_playing(true)}
                else index++
                if(index>=playingTargets.length){video.set_playing(false);report="PREVIEW_SEEK_PASS: paused/playback, every second, repeated, near, reverse and rapid seeks landed";console.log(report);stop();Qt.quit();return}
                before=video.debug_frames;waits=0;video.seek(playingTargets[index]);return
            }
            if(index>=0) {
                waits++
                if(video.debug_frames<=before || Math.abs(video.t_ms-targets[index])>20
                        || Math.abs(video.debug_frame_ms-targets[index])>20) {
                    if(waits>160){report="PREVIEW_SEEK_FAIL: target="+targets[index]+" actual="+video.t_ms+" displayed="+video.debug_frame_ms+" fresh="+(video.debug_frames-before);console.log(report);Qt.quit()}
                    return
                }
                console.log("SEEK_LANDED: "+targets[index]+" actual="+video.t_ms)
            }
            index++
            if(index>=targets.length){phase="rapid";rapid.start();return}
            before=video.debug_frames;waits=0
            video.seek(targets[index])
        }
    }
}
