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
        property real draggedAngle: 0
        onAngleDragged: function(angle) { draggedAngle = angle }
        onCornerMoved: function(index,x,y) {
            var updated = corners.slice(); updated[index] = [x,y]; corners = updated
        }
    }
    function viewport() { return findChild(imageView, "receiptImageViewport") }
    function init() {
        tryCompare(findChild(imageView,"receiptZoomImage"),"status",Image.Ready)
        viewport().resetZoom()
        imageView.previewRotation = 0
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
    function test_preview_rotation_turns_the_picture_unless_corners_show() {
        var photo = findChild(imageView,"receiptZoomImage")
        compare(photo.rotation,0)
        imageView.previewRotation = 3.5
        wait(20)
        fuzzyCompare(photo.rotation,3.5,.001)
        compare(photo.transformOrigin,Item.Center)
        imageView.previewRotation = 0
        wait(20)
        compare(photo.rotation,0)
    }
    // 框跟着照片一起转：转画面只帮你看清/框准，四角本身（原图坐标）不变。
    function test_corners_turn_with_the_picture() {
        var area = findChild(imageView,"receiptPhotoArea")
        var handle = findChild(imageView,"receiptCorner0")
        imageView.previewRotation = 0
        wait(20)
        var cornersBefore = JSON.stringify(imageView.corners)
        var ptsBefore = JSON.stringify(area.points[0])
        var before = area.mapToItem(imageView, handle.x + handle.width / 2, handle.y + handle.height / 2)
        imageView.previewRotation = 6
        wait(20)
        var after = area.mapToItem(imageView, handle.x + handle.width / 2, handle.y + handle.height / 2)
        // 四角（原图归一化）与对应的图像像素都不动，屏幕位置绕画面中心转了 6°。
        compare(JSON.stringify(imageView.corners), cornersBefore)
        compare(JSON.stringify(area.points[0]), ptsBefore)
        var photo = findChild(imageView,"receiptZoomImage")
        var c = photo.mapToItem(imageView, photo.width / 2, photo.height / 2)
        var dx = before.x - c.x, dy = before.y - c.y
        var rad = 6 * Math.PI / 180
        verify(Math.abs(after.x - (c.x + dx * Math.cos(rad) - dy * Math.sin(rad))) < 2)
        verify(Math.abs(after.y - (c.y + dx * Math.sin(rad) + dy * Math.cos(rad))) < 2)
        imageView.previewRotation = 0
        wait(20)
    }
    function test_rotate_knob_turns_the_angle_and_shows_the_grid() {
        var layer = findChild(imageView,"receiptRotateLayer")
        var grid = findChild(imageView,"receiptAlignGrid")
        var knob = findChild(imageView,"receiptRotateKnob")
        // 能不能转由外面决定（有图且不忙），组件自己只管显示/隐藏。
        imageView.rotatable = false
        imageView.corners = [[.25,.25],[.75,.25],[.75,.75],[.25,.75]]
        wait(20)
        verify(!layer.visible)
        // 有图就能转（不用先识别）：角度为 0 时手柄在画面正上方。
        imageView.corners = []
        imageView.rotatable = true
        imageView.previewRotation = 0
        wait(20)
        verify(layer.visible)
        verify(knob.y + knob.height / 2 < layer.height / 2)
        verify(!grid.visible)
        // 按住手柄往右拖＝顺时针；转的时候出网格标，松手就收回去。
        mousePress(knob, 18, 18)
        wait(10)
        mouseMove(knob, 70, 10)
        wait(20)
        verify(imageView.rotating)
        verify(grid.visible)
        verify(imageView.draggedAngle > 5)
        mouseRelease(knob, 70, 10)
        wait(20)
        verify(!imageView.rotating)
        verify(!grid.visible)
        // 界面会把拖出来的角度立刻回灌到 previewRotation；手柄只跟角度走，
        // 不能再叠加一次拖动量，否则拖起来比指针快一倍。
        imageView.previewRotation = imageView.draggedAngle
        wait(20)
        var cx = layer.width / 2, cy = layer.height / 2
        var shown = Math.atan2(knob.x + knob.width / 2 - cx, cy - (knob.y + knob.height / 2)) * 180 / Math.PI
        fuzzyCompare(shown, imageView.draggedAngle, 1.0)
        // 手柄跟着角度绕：角度变了它就不在正上方。
        imageView.previewRotation = 20
        wait(20)
        verify(knob.x + knob.width / 2 > layer.width / 2)
        imageView.previewRotation = 0
        imageView.rotatable = false
        imageView.corners = [[.25,.25],[.75,.25],[.75,.75],[.25,.75]]
    }
}
