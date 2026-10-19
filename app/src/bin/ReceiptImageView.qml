import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Item {
    id: view
    property url source
    property var corners: []
    property bool editable: true
    /// 预览的显示角度（度，正值＝顺时针）：只是把照片转过来方便看清/框准，
    /// 四角仍然按原图量，所以转多少都不影响识别结果。
    property real previewRotation: 0
    /// 能不能拖圆钮转图：转的是源图（校正图已经摆正了，不用再转），由调用方决定。
    property bool rotatable: false
    /// 正在拖旋转手柄（网格标只在此时出现，平时别挡住画面）。
    property bool rotating: false
    /// 拖动旋转时报出的新角度（度，正值＝顺时针；界面只负责算角度和夹范围）。
    signal angleDragged(real angle)
    readonly property real zoom: viewport.zoom
    signal cornerMoved(int index, real imageX, real imageY)
    ColumnLayout {
        anchors.fill: parent
        spacing: 4
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: view.rotatable ? "滚轮缩放 · 拖动图片平移 · 拖圆钮旋转" : "滚轮缩放 · 拖动图片平移"
                Layout.fillWidth: true
            }
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
                    // PreserveAspectFit 把画面居中放在这个 item 里，所以绕 item 中心转
                    // 就等于绕画面中心转。四角存在照片自己的坐标里，跟着一起转：
                    // 转画面只是帮你看清/框准，框到哪一块才是识别的依据。
                    rotation: view.previewRotation
                    transformOrigin: Item.Center
                    onSourceChanged: viewport.resetZoom()
                    Item {
                        id: photoArea
                        objectName: "receiptPhotoArea"
                        visible: view.corners.length === 4
                        width: photo.paintedWidth; height: photo.paintedHeight
                        anchors.centerIn: parent
                        // 四角在图像里的位置（像素）。
                        readonly property var points: {
                            if (view.corners.length !== 4 || !photo.paintedWidth || !photo.paintedHeight) return []
                            var w = photo.paintedWidth - 1, h = photo.paintedHeight - 1
                            var list = []
                            for (var i = 0; i < 4; i++) {
                                list.push([view.corners[i][0] * w, view.corners[i][1] * h])
                            }
                            return list
                        }
                        Canvas {
                            id: outline
                            anchors.fill: parent
                            onPaint: {
                                var ctx = getContext("2d"); ctx.clearRect(0,0,width,height)
                                var pts = photoArea.points
                                if (pts.length !== 4) return
                                ctx.strokeStyle = "#00dca0"; ctx.lineWidth = 2; ctx.beginPath()
                                for (var i = 0; i < 4; i++) {
                                    if (i === 0) ctx.moveTo(pts[i][0], pts[i][1])
                                    else ctx.lineTo(pts[i][0], pts[i][1])
                                }
                                ctx.closePath(); ctx.stroke()
                            }
                            onWidthChanged: requestPaint()
                            onHeightChanged: requestPaint()
                            Connections { target: photoArea; function onPointsChanged() { outline.requestPaint() } }
                        }
                        Repeater {
                            model: view.corners.length === 4 ? 4 : 0
                            Rectangle {
                                objectName: "receiptCorner" + index
                                width: 28; height: 28; radius: 14; color: "#00a77b"
                                x: photoArea.points[index] ? photoArea.points[index][0] - 14 : 0
                                y: photoArea.points[index] ? photoArea.points[index][1] - 14 : 0
                                Text { anchors.centerIn: parent; text: index + 1; color: "white" }
                                MouseArea {
                                    anchors.fill: parent
                                    enabled: view.editable
                                    preventStealing: true
                                    onPositionChanged: function(mouse) {
                                        if (!pressed || !photoArea.width || !photoArea.height) return
                                        var p = mapToItem(photoArea, mouse.x, mouse.y)
                                        view.cornerMoved(index,
                                                         Math.max(0, Math.min(1, p.x / (photoArea.width - 1))),
                                                         Math.max(0, Math.min(1, p.y / (photoArea.height - 1))))
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
            // 网格标 + 旋转手柄。故意盖在视口上（不进 Flickable），这样网格线是屏幕
            // 正交的 —— 用户把成绩表的行对准横线就摆正了；缩放到哪一格都不影响。
            Item {
                id: rotateLayer
                objectName: "receiptRotateLayer"
                anchors.fill: parent
                visible: view.rotatable

                Canvas {
                    id: alignGrid
                    objectName: "receiptAlignGrid"
                    anchors.fill: parent
                    visible: view.rotating
                    onVisibleChanged: requestPaint()
                    onWidthChanged: requestPaint()
                    onHeightChanged: requestPaint()
                    onPaint: {
                        var ctx = getContext("2d")
                        ctx.clearRect(0, 0, width, height)
                        if (!visible || width <= 0 || height <= 0) return
                        // 三横两竖的浅色线做参照，中间一条亮线对中线。
                        ctx.strokeStyle = "#66ffffff"
                        ctx.lineWidth = 1
                        ctx.beginPath()
                        for (var i = 1; i < 4; i++) {
                            var y = height * i / 4
                            ctx.moveTo(0, y); ctx.lineTo(width, y)
                        }
                        for (var j = 1; j < 3; j++) {
                            var x = width * j / 3
                            ctx.moveTo(x, 0); ctx.lineTo(x, height)
                        }
                        ctx.stroke()
                        ctx.strokeStyle = "#cc00dca0"
                        ctx.lineWidth = 1.5
                        ctx.beginPath()
                        ctx.moveTo(0, height / 2); ctx.lineTo(width, height / 2)
                        ctx.moveTo(width / 2, 0); ctx.lineTo(width / 2, height)
                        ctx.stroke()
                    }
                }

                // 手柄绕画面中心摆放，θ=0 时在正上方；它自己跟着角度转，顺带指示
                // 目前「表格上边」朝哪边。
                Rectangle {
                    id: rotateKnob
                    objectName: "receiptRotateKnob"
                    width: 36; height: 36; radius: 18
                    color: rotateDrag.drag.active ? "#00dca0" : "#00876a"
                    border.color: "#e8ffffff"; border.width: 1
                    opacity: view.editable ? 1 : 0.5
                    property real orbit: Math.max(50, Math.min(width > 0 ? rotateLayer.width : 0, rotateLayer.height) / 2 - 26)
                    // 只跟当前角度走：拖动时上报的新角度会被界面立刻回灌到
                    // previewRotation，如果这里再叠加一次拖动量，手柄就会跑得比指针快一倍。
                    property real theta: view.previewRotation * Math.PI / 180
                    x: rotateLayer.width / 2 + orbit * Math.sin(theta) - width / 2
                    y: rotateLayer.height / 2 - orbit * Math.cos(theta) - height / 2
                    Text {
                        anchors.centerIn: parent
                        text: "↻"
                        font.pixelSize: 20
                        color: "white"
                    }
                    MouseArea {
                        id: rotateDrag
                        anchors.fill: parent
                        enabled: view.editable
                        preventStealing: true
                        cursorShape: Qt.SizeAllCursor
                        property real startAngle: 0
                        property real startPointer: 0
                        function pointerAngle(mouse) {
                            var p = mapToItem(rotateLayer, mouse.x, mouse.y)
                            return Math.atan2(p.y - rotateLayer.height / 2, p.x - rotateLayer.width / 2)
                        }
                        onPressed: function(mouse) {
                            startPointer = pointerAngle(mouse)
                            startAngle = view.previewRotation
                            view.rotating = true
                        }
                        onPositionChanged: function(mouse) {
                            if (!pressed) return
                            var deg = (pointerAngle(mouse) - startPointer) * 180 / Math.PI
                            while (deg <= -180) deg += 360
                            while (deg > 180) deg -= 360
                            view.angleDragged(startAngle + deg)
                        }
                        onReleased: view.rotating = false
                        onCanceled: view.rotating = false
                    }
                }

                // 拖动时把角度显示在手柄旁边，松手后跟着手柄一起藏起来。
                Rectangle {
                    visible: view.rotating
                    x: Math.max(4, Math.min(rotateLayer.width - width - 4, rotateKnob.x + rotateKnob.width / 2 - width / 2))
                    y: Math.max(4, rotateKnob.y - height - 4)
                    width: angleLabel.implicitWidth + 16
                    height: angleLabel.implicitHeight + 8
                    radius: 4
                    color: "#cc10161b"
                    Text {
                        id: angleLabel
                        anchors.centerIn: parent
                        color: "#e8ffffff"
                        text: view.previewRotation.toFixed(1) + "°"
                    }
                }
            }
        }
    }
}
