import QtQuick
import QtQuick.Controls
import TraceReel 1.0

TabButton {
    id: control
    property string iconName: ""
    readonly property color foreground: checked ? Theme.accent : Theme.text
    implicitHeight: 48
    padding: 8
    spacing: 7
    font.pixelSize: 13
    background: Rectangle {
        color: control.hovered || control.visualFocus ? Theme.surface_hover : "transparent"
    }
    contentItem: Item {
        implicitWidth: content.implicitWidth
        implicitHeight: content.implicitHeight
        Row {
            id: content
            anchors.centerIn: parent
            spacing: control.spacing
            UiIcon { name: control.iconName; color: control.foreground; anchors.verticalCenter: parent.verticalCenter; width: 18; height: 18 }
            Text { text: control.text; font: control.font; color: control.foreground; anchors.verticalCenter: parent.verticalCenter }
        }
    }
}
