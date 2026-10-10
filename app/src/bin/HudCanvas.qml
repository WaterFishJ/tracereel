import QtQuick

Item {
    id: canvas
    required property var editor
    enabled:editor.enabled
    focus:true
    clip:true
    Keys.onPressed: function(event) {
        if(event.modifiers & Qt.ControlModifier) {
            if(event.key===Qt.Key_Z){editor.undo();event.accepted=true}
            if(event.key===Qt.Key_Y){editor.redoHistory();event.accepted=true}
        } else if(editor.current && !editor.current.locked) {
            var dx=0,dy=0,step=(event.modifiers & Qt.ShiftModifier)?10:1
            if(event.key===Qt.Key_Left)dx=-step
            if(event.key===Qt.Key_Right)dx=step
            if(event.key===Qt.Key_Up)dy=-step
            if(event.key===Qt.Key_Down)dy=step
            if(dx || dy){editor.begin();editor.setRect(editor.current.x+dx/width,editor.current.y+dy/height,editor.current.w,editor.current.h);editor.end();event.accepted=true}
            if(event.key===Qt.Key_Delete){editor.remove();event.accepted=true}
        }
    }
    function snap(value,size,axis) {
        if(!editor.snapping)return value
        var targets=[0,.05,.5,.95,1],best=value,distance=7/(axis==="x"?width:height)
        editor.layout.widgets.forEach(function(w,i){
            if(i!==editor.selected && w.visible) {
                var a=axis==="x"?w.x:w.y,s=axis==="x"?w.w:w.h
                targets.push(a,a+s/2,a+s)
            }
        })
        targets.forEach(function(t){[0,size/2,size].forEach(function(off){var d=Math.abs(value+off-t);if(d<distance){distance=d;best=t-off}})})
        return best
    }
    MouseArea {
        anchors.fill:parent
        property point origin
        property var rect:null
        onPressed:function(mouse){
            canvas.forceActiveFocus()
            var hit=-1,z=-Infinity
            editor.layout.widgets.forEach(function(w,i){
                if(!w.visible)return
                var cx=(w.x+w.w/2)*canvas.width,cy=(w.y+w.h/2)*canvas.height,a=-w.rotation*Math.PI/180
                var dx=mouse.x-cx,dy=mouse.y-cy,lx=dx*Math.cos(a)-dy*Math.sin(a),ly=dx*Math.sin(a)+dy*Math.cos(a)
                if(Math.abs(lx)<=w.w*canvas.width/2 && Math.abs(ly)<=w.h*canvas.height/2 && w.z>=z){hit=i;z=w.z}
            })
            editor.selected=hit
            rect=editor.current && !editor.current.locked?editor.copy(editor.current):null
            if(rect){origin=Qt.point(mouse.x,mouse.y);editor.begin()}
        }
        onPositionChanged:function(mouse) {
            if(!pressed || !rect)return
            editor.setRect(canvas.snap(rect.x+(mouse.x-origin.x)/canvas.width,rect.w,"x"),
                canvas.snap(rect.y+(mouse.y-origin.y)/canvas.height,rect.h,"y"),rect.w,rect.h)
        }
        onReleased:{if(rect)editor.end();rect=null}
        onCanceled:{if(rect)editor.end();rect=null}
    }
    Item {
        id:selection
        property var c:editor.current
        visible:c!==null && c.visible
        x:c?c.x*canvas.width:0;y:c?c.y*canvas.height:0
        width:c?c.w*canvas.width:0;height:c?c.h*canvas.height:0
        rotation:c?c.rotation:0
        Rectangle {anchors.fill:parent;color:"transparent";border.color:selection.c && selection.c.locked?"#ffca5c":"#00e5a0";border.width:2}
        MouseArea {
            anchors.fill:parent;enabled:selection.c && !selection.c.locked
            cursorShape:Qt.SizeAllCursor
            property point origin
            property var rect
            onPressed:function(m){canvas.forceActiveFocus();origin=mapToItem(canvas,m.x,m.y);rect=editor.copy(selection.c);editor.begin()}
            onPositionChanged:function(m){
                if(!pressed)return
                var p=mapToItem(canvas,m.x,m.y)
                editor.setRect(canvas.snap(rect.x+(p.x-origin.x)/canvas.width,rect.w,"x"),
                    canvas.snap(rect.y+(p.y-origin.y)/canvas.height,rect.h,"y"),rect.w,rect.h)
            }
            onReleased:editor.end()
            onCanceled:editor.end()
        }
        Rectangle {
            x:parent.width-6;y:parent.height-6;width:12;height:12;color:"#00e5a0"
            visible:selection.c && !selection.c.locked
            MouseArea {
                anchors.fill:parent;cursorShape:Qt.SizeFDiagCursor
                property point origin
                property var rect
                onPressed:function(m){origin=mapToItem(canvas,m.x,m.y);rect=editor.copy(selection.c);editor.begin()}
                onPositionChanged:function(m){
                    if(!pressed)return
                    var p=mapToItem(canvas,m.x,m.y),a=-rect.rotation*Math.PI/180,dx=p.x-origin.x,dy=p.y-origin.y
                    var dw=dx*Math.cos(a)-dy*Math.sin(a),dh=dx*Math.sin(a)+dy*Math.cos(a)
                    var nw=Math.max(.02,Math.min(1,rect.w+dw/canvas.width)),nh=Math.max(.02,Math.min(1,rect.h+dh/canvas.height))
                    // Preserve the opposite corner even when the selection is rotated.
                    var rw=(nw-rect.w)*canvas.width,rh=(nh-rect.h)*canvas.height,angle=-a
                    var sx=(rw*Math.cos(angle)-rh*Math.sin(angle)-rw)/2
                    var sy=(rw*Math.sin(angle)+rh*Math.cos(angle)-rh)/2
                    editor.setRect(rect.x+sx/canvas.width,rect.y+sy/canvas.height,nw,nh)
                }
                onReleased:editor.end()
                onCanceled:editor.end()
            }
        }
        Rectangle {
            x:parent.width/2-6;y:-20;width:12;height:12;radius:6;color:"#00e5a0"
            visible:selection.c && !selection.c.locked
            MouseArea {
                anchors.fill:parent;cursorShape:Qt.CrossCursor
                onPressed:editor.begin()
                onPositionChanged:function(m){
                    if(!pressed)return
                    var p=mapToItem(canvas,m.x,m.y),c=editor.current
                    var a=Math.atan2(p.y-(c.y+c.h/2)*canvas.height,p.x-(c.x+c.w/2)*canvas.width)*180/Math.PI+90
                    editor.setField("rotation",a,true)
                }
                onReleased:editor.end()
                onCanceled:editor.end()
            }
        }
    }
}
