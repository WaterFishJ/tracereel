import QtQuick

Canvas {
    id: root
    property string name: ""
    property color color: "#242424"
    implicitWidth: 20
    implicitHeight: 20
    onNameChanged: requestPaint()
    onColorChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onPaint: {
        var c = getContext("2d")
        c.reset()
        c.scale(width / 24, height / 24)
        c.strokeStyle = color
        c.lineWidth = 1.7
        c.lineCap = "round"
        c.lineJoin = "round"
        function line(points) {
            c.beginPath(); c.moveTo(points[0], points[1])
            for (var i = 2; i < points.length; i += 2) c.lineTo(points[i], points[i + 1])
            c.stroke()
        }
        function circle(x, y, r) { c.beginPath(); c.arc(x, y, r, 0, Math.PI * 2); c.stroke() }
        switch (name) {
        case "folder":
            line([3,19,3,6,9,6,11,8,21,8,21,19,3,19]); break
        case "appearance":
            circle(12,12,8)
            circle(8,10,0.6); circle(12,7.5,0.6); circle(16,10,0.6)
            line([19,16,14,16,14,20]); break
        case "log":
            line([6,3,18,3,18,21,6,21,6,3])
            line([9,8,15,8]); line([9,12,15,12]); line([9,16,13,16]); break
        case "queue":
            line([4,6,14,6]); line([4,11,14,11]); line([4,16,10,16])
            line([17,12,21,16,17,20]); line([13,16,21,16]); break
        case "help":
            circle(12,12,9)
            c.beginPath(); c.arc(12,9,2.5,Math.PI,Math.PI*2.5); c.stroke()
            line([12,11.5,12,14]); circle(12,17,0.25); break
        case "route":
            circle(5,18,2); circle(19,6,2)
            c.beginPath(); c.moveTo(5,16); c.lineTo(5,9)
            c.bezierCurveTo(5,4,15,5,15,11)
            c.bezierCurveTo(15,17,19,17,19,8); c.stroke(); break
        case "split":
            line([6,21,6,3,19,3,16,7,19,11,6,11]); break
        case "video":
            line([3,6,16,6,16,18,3,18,3,6])
            line([16,10,21,7,21,17,16,14]); break
        case "hud":
            line([3,9,3,4,8,4]); line([16,4,21,4,21,9])
            line([21,15,21,20,16,20]); line([8,20,3,20,3,15])
            line([7,14,10,10,13,13,17,9]); break
        case "export":
            line([5,14,5,20,19,20,19,14])
            line([12,15,12,3]); line([7,8,12,3,17,8]); break
        }
    }
}
