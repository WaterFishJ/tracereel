import QtQuick 2.15
import QtTest 1.3

TestCase {
    name: "ReceiptZoom"
    visible: true
    when: windowShown
    width: 500; height: 500
    ReceiptImageView {
        id: imageView
        anchors.fill: parent
        source: Qt.resolvedUrl("image.png")
        corners: [[.25,.25],[.75,.25],[.75,.75],[.25,.75]]
        onCornerMoved: function(index,x,y) {
            var updated = corners.slice(); updated[index] = [x,y]; corners = updated
        }
    }
    function viewport() { return findChild(imageView, "receiptImageViewport") }
    function init() {
        tryCompare(findChild(imageView,"receiptZoomImage"),"status",Image.Ready)
        viewport().resetZoom()
        imageView.corners = [[.25,.25],[.75,.25],[.75,.75],[.25,.75]]
        wait(50)
    }
    function test_wheel_keeps_cursor_position_and_corners() {
        var v = viewport(), before = JSON.stringify(imageView.corners)
        mouseWheel(v,200,200,0,120)
        wait(20)
        fuzzyCompare(imageView.zoom,1.2,.001)
        fuzzyCompare(v.contentX,40,.1)
        fuzzyCompare(v.contentY,40,.1)
        compare(JSON.stringify(imageView.corners),before)
        mouseWheel(v,200,200,0,-120)
        wait(20)
        fuzzyCompare(imageView.zoom,1,.001)
        fuzzyCompare(v.contentX,0,.1)
        fuzzyCompare(v.contentY,0,.1)
    }
    function test_corner_drag_after_zoom_does_not_pan_image() {
        var v = viewport()
        mouseWheel(v,200,200,0,120)
        wait(20)
        var x = v.contentX, y = v.contentY
        var handle = findChild(imageView,"receiptCorner0")
        var before = imageView.corners[0][0]
        mouseDrag(handle,14,14,30,0,Qt.LeftButton)
        verify(imageView.corners[0][0] > before + .04)
        fuzzyCompare(imageView.corners[0][1],.25,.01)
        fuzzyCompare(v.contentX,x,.1)
        fuzzyCompare(v.contentY,y,.1)
    }
    function test_zoom_limits_and_reset() {
        var v = viewport()
        v.zoomAt(100,200,200)
        compare(imageView.zoom,8)
        v.resetZoom()
        compare(imageView.zoom,1)
        compare(v.contentX,0)
        compare(v.contentY,0)
    }
}
