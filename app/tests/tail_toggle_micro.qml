// 微型探针：只查「显示尾巴」这个真勾选框能不能把设置写回去（不加载项目，几秒就结束）。
// 用 qmlprobe 跑：
//   $env:QMLPROBE_FILE="…/app/tests/tail_toggle_micro.qml"; $env:QMLPROBE_RUN_LOOP="1"; $env:QMLPROBE_TIMEOUT_MS="60000"
import QtQuick

Item {
    id: root
    property var win: null
    property string report: "未跑"

    function debugState() { return report }

    function findByLabel(node, label) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && c.label === label) return c
            var r = findByLabel(c, label)
            if (r) return r
        }
        return null
    }
    function findCheckBox(node) {
        if (!node || !node.children) return null
        var kids = node.children
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i]
            if (c && typeof c.toggle === "function") return c
            var r = findCheckBox(c)
            if (r) return r
        }
        return null
    }

    Component.onCompleted: {
        var comp = Qt.createComponent(Qt.resolvedUrl("../src/bin/gui.qml"))
        if (comp.status !== Component.Ready) { report = "加载失败: " + comp.errorString(); Qt.quit(); return }
        win = comp.createObject(null)
        if (!win) { report = "实例化失败"; Qt.quit(); return }
        var row = findByLabel(win.contentItem, "显示尾巴")
        if (!row) { report = "找不到「显示尾巴」行"; Qt.quit(); return }
        var cb = findCheckBox(row)
        if (!cb) { report = "行里找不到勾选框"; Qt.quit(); return }
        var s = []
        s.push("row.prop=" + row.prop + " valueOn=" + row.valueOn + " valueOff=" + row.valueOff)
        s.push("cb.checkable=" + cb.checkable + " cb.checked=" + cb.checked + " 设置=" + win.debugGet(row.prop))
        cb.toggle()
        s.push("toggle() 之后: cb.checked=" + cb.checked + " 设置=" + win.debugGet(row.prop))
        cb.checked = true
        s.push("赋 checked=true 之后: cb.checked=" + cb.checked + " 设置=" + win.debugGet(row.prop))
        cb.checked = false
        s.push("赋 checked=false 之后: cb.checked=" + cb.checked + " 设置=" + win.debugGet(row.prop))
        report = s.join(" | ")
        console.log("[micro] " + report)
        Qt.quit()
    }
}