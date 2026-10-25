import QtQuick
import QtQuick.Controls

Item {
    id: root
    property var points: []
    property real duration: 0
    property color accentColor: "#4c9aff"
    property color markerColor: "#b6bbc2"
    readonly property var visiblePoints: points.filter(function(point) {
        return duration > 0 && isFinite(point.timeMs) && point.timeMs >= 0 && point.timeMs <= duration
    })
    readonly property int hoveredIndex: pointer.containsMouse ? nearest(pointer.mouseX) : -1
    property int keyboardIndex: 0
    onVisiblePointsChanged: keyboardIndex = Math.max(0, Math.min(keyboardIndex, visiblePoints.length - 1))
    signal activated(int index)
    implicitHeight: 16
    activeFocusOnTab: true
    enabled: visiblePoints.length > 0
    Accessible.name: "分段快捷跳转点"
    Accessible.description: "左右方向键选择分段，回车跳转"
    function position(point) { return duration > 0 ? point.timeMs / duration * width : 0 }
    function nearest(x) {
        var best = -1, distance = 11
        for (var i = 0; i < visiblePoints.length; ++i) {
            var d = Math.abs(x - position(visiblePoints[i]))
            if (d < distance) { best = i; distance = d }
        }
        return best
    }
    Keys.onLeftPressed: keyboardIndex = Math.max(0, keyboardIndex - 1)
    Keys.onRightPressed: keyboardIndex = Math.min(visiblePoints.length - 1, keyboardIndex + 1)
    Keys.onReturnPressed: activateKeyboard()
    Keys.onEnterPressed: activateKeyboard()
    function activateKeyboard() {
        if (keyboardIndex >= 0 && keyboardIndex < visiblePoints.length)
            activated(visiblePoints[keyboardIndex].index)
    }
    Repeater {
        model: root.visiblePoints
        delegate: Rectangle {
            required property int index
            required property var modelData
            objectName: "splitJumpMarker_" + modelData.index
            property bool highlighted: root.hoveredIndex === index || (root.activeFocus && root.keyboardIndex === index)
            x: root.position(modelData) - width / 2
            y: 2
            width: highlighted ? 6 : 4
            height: highlighted ? 12 : 9
            radius: 2
            color: highlighted ? root.accentColor : root.markerColor
        }
    }
    MouseArea {
        id: pointer
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: root.hoveredIndex >= 0 ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: if (root.hoveredIndex >= 0) root.activated(root.visiblePoints[root.hoveredIndex].index)
    }
    ToolTip {
        objectName: "splitJumpTooltip"
        visible: root.hoveredIndex >= 0 && !pointer.pressed
        delay: 200
        text: root.hoveredIndex >= 0 ? root.visiblePoints[root.hoveredIndex].tooltip : ""
        x: root.hoveredIndex >= 0 ? root.position(root.visiblePoints[root.hoveredIndex]) - width / 2 : 0
        y: -height - 12
    }
}
