import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Item {
    id: view
    property url source
    property var corners: []
    property bool editable: true
    readonly property real zoom: viewport.zoom
    signal cornerMoved(int index, real imageX, real imageY)

    ColumnLayout {
        anchors.fill: parent
        spacing: 4
        RowLayout {
            Layout.fillWidth: true
            Label { text: "滚轮缩放 · 拖动图片平移"; Layout.fillWidth: true }
            Label { text: Math.round(viewport.zoom * 100) + "%" }
            Button { text: "适应"; onClicked: viewport.resetZoom() }
        }
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Flickable {
                id: viewport
                objectName: "receiptImageViewport"
                anchors.fill: parent
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                property real zoom: 1
                contentWidth: width * zoom
                contentHeight: height * zoom
                function resetZoom() {
                    cancelFlick()
                    zoom = 1; contentX = 0; contentY = 0
                }
                function zoomAt(factor, x, y) {
                    cancelFlick()
                    var next = Math.max(1, Math.min(8, zoom * factor))
                    var ratio = next / zoom
                    var newX = (contentX + x) * ratio - x
                    var newY = (contentY + y) * ratio - y
                    zoom = next
                    contentX = Math.max(0, Math.min(contentWidth - width, newX))
                    contentY = Math.max(0, Math.min(contentHeight - height, newY))
                }
                Image {
                    id: photo
                    objectName: "receiptZoomImage"
                    width: viewport.contentWidth
                    height: viewport.contentHeight
                    source: view.source
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    onSourceChanged: viewport.resetZoom()
                    Item {
                        id: photoArea
                        visible: view.corners.length === 4
                        width: photo.paintedWidth; height: photo.paintedHeight
                        anchors.centerIn: parent
                        Canvas {
                            id: outline
                            anchors.fill: parent
                            onPaint: {
                                var ctx = getContext("2d"); ctx.clearRect(0,0,width,height)
                                if (view.corners.length !== 4) return
                                ctx.strokeStyle = "#00dca0"; ctx.lineWidth = 2; ctx.beginPath()
                                for (var i = 0; i < 4; i++) {
                                    var x = view.corners[i][0] * width, y = view.corners[i][1] * height
                                    if (i === 0) ctx.moveTo(x,y); else ctx.lineTo(x,y)
                                }
                                ctx.closePath(); ctx.stroke()
                            }
                            onWidthChanged: requestPaint()
                            onHeightChanged: requestPaint()
                            Connections { target: view; function onCornersChanged() { outline.requestPaint() } }
                        }
                        Repeater {
                            model: view.corners.length === 4 ? 4 : 0
                            Rectangle {
                                objectName: "receiptCorner" + index
                                width: 28; height: 28; radius: 14; color: "#00a77b"
                                x: view.corners[index][0] * photoArea.width - 14
                                y: view.corners[index][1] * photoArea.height - 14
                                Text { anchors.centerIn: parent; text: index + 1; color: "white" }
                                MouseArea {
                                    anchors.fill: parent
                                    enabled: view.editable
                                    preventStealing: true
                                    onPositionChanged: function(mouse) {
                                        if (!pressed || !photoArea.width || !photoArea.height) return
                                        var p = mapToItem(photoArea, mouse.x, mouse.y)
                                        view.cornerMoved(index, Math.max(0,Math.min(1,p.x/photoArea.width)), Math.max(0,Math.min(1,p.y/photoArea.height)))
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // A wheel-only overlay leaves pointer presses to the corner handles
            // and Flickable; its coordinates stay relative to the viewport.
            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.NoButton
                onWheel: function(wheel) {
                    var delta = wheel.angleDelta.y || wheel.pixelDelta.y
                    if (!delta || !view.source.toString().length) return
                    viewport.zoomAt(Math.pow(1.2, delta / 120), wheel.x, wheel.y)
                    wheel.accepted = true
                }
            }
        }
    }
}
