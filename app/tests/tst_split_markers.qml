import QtQuick
import QtTest
import "../src/bin" as UI

Item {
    width: 500; height: 150
    UI.SplitMarkers { id: marks; x:40; y:80; width:400; height:20; duration:10000 }
    SignalSpy { id: activation; target:marks; signalName:"activated" }
    TestCase {
        name:"SplitMarkers"
        when:windowShown
        function init() {
            marks.duration=10000
            marks.keyboardIndex=0
            marks.points=[{index:0,timeMs:0,tooltip:"分段 1 · 起点"},
                {index:1,timeMs:5000,tooltip:"分段 2 · 31 → 32\n视频时间 0:05"},
                {index:2,timeMs:10000,tooltip:"分段 3"},
                {index:3,timeMs:11000,tooltip:"超出视频"}]
            activation.clear()
        }
        function test_positions_and_out_of_range() {
            compare(marks.visiblePoints.length,3)
            var middle=findChild(marks,"splitJumpMarker_1")
            verify(middle!==null)
            compare(middle.x+middle.width/2,200)
            marks.duration=20000
            compare(marks.visiblePoints.length,4)
            middle=findChild(marks,"splitJumpMarker_1")
            compare(middle.x+middle.width/2,100)
            marks.duration=10000
        }
        function test_hover_tip_and_click() {
            mouseMove(marks,200,5)
            tryCompare(marks,"hoveredIndex",1)
            var tip=findChild(marks,"splitJumpTooltip")
            verify(tip!==null)
            tryCompare(tip,"visible",true)
            compare(tip.text,marks.points[1].tooltip)
            mouseClick(marks,200,5,Qt.LeftButton)
            compare(activation.count,1)
            compare(activation.signalArguments[0][0],1)
            mouseClick(marks,100,5,Qt.LeftButton)
            compare(activation.count,1,"Clicking between markers must not jump")
        }
        function test_empty_and_keyboard() {
            marks.forceActiveFocus()
            keyClick(Qt.Key_Right)
            keyClick(Qt.Key_Return)
            compare(activation.count,1)
            compare(activation.signalArguments[0][0],1)
            marks.points=[]
            compare(marks.visiblePoints.length,0)
            keyClick(Qt.Key_Return)
            compare(activation.count,1)
        }
    }
}
