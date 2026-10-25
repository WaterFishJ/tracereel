import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 多地图框编辑器。
//
// 数据形状（与 Rust `crate::settings::MapLayout` 的 JSON 完全一致）：
//   {version:1, boxes:[{mode,share,x,y,w,h,rotation,opacity,shape,feather,corner,
//                       viewMode,viewZoom,lrMargin,tbMargin,sensitivity}]}
// - 从 `source.map_boxes`（聚合 JSON 字符串）读、编辑后原样写回；
// - 拖框/改取景（框数不变）时直接调 `renderer.apply_map_frames(json)` 就地更新；
// - 增/删/复制框（框数变化）发 `structureChanged()`，由 gui.qml 走完整重载。
//
// 与 `HudCanvas.qml` 的契约（预览里拖动/缩放/旋转/吸附靠它）：
//   layout.widgets[]（每框压平 + 有效矩形 + visible/locked/z）、selected、current、
//   copy()、begin()/end()、setRect()、setField()、snapping、enabled、undo()。
// 贴边（mode 0/1）的框只可被选中，locked=true，不可拖动；第 2+ 框固定自由摆放。
Column {
    id: root
    objectName: "mapLayoutEditor"
    required property var source
    required property var renderer
    property bool snapping: true
    property bool editing: true
    property bool hasVideo: false

    /// 解析后的布局对象；`boxes` 是唯一真相，`widgets` 是给 HudCanvas 的压平视图。
    property var layout: ({version: 1, boxes: [], widgets: []})
    property var widgets: []
    /// 选中的框下标（HudCanvas 在预览里点选时也会写它；-1 = 在预览空白处点了取消选中）。
    property int selected: 0
    /// 最近一次选中的框：没有选中任何框时，面板继续显示/编辑它（预览里不再高亮）。
    property int lastSelected: 0
    onSelectedChanged: {
        if (selected >= 0 && layout && layout.boxes && selected < layout.boxes.length) lastSelected = selected
    }
    /// 主题色由 gui.qml 传入（Theme.*）。默认值用于没有 Theme 单例的测试环境，
    /// 与浅色主题一致，保证离屏测试也能正常渲染。
    property color textColor: "#111111"
    property color mutedColor: "#6b6b6b"
    property color accentColor: "#116cad"
    property color textOnAccent: "#ffffff"
    property color buttonColor: "#fbfbfb"
    property color borderColor: "#d5d5d5"
    property color fieldColor: "#ffffff"
    property color surfaceHoverColor: "#f4f5f7"
    property real uiScale: 1.0
    /// 当前渲染器里已有的框数；只有框数没变时才做就地更新（见 publish）。
    property int appliedCount: -1
    /// 每次重建自增，供数值框/滑杆在外部变化后同步显示。
    property int revision: 0
    property var undoStack: []
    property string gestureBefore: ""
    property string lastSent: ""
    property string error: ""

    signal changed()
    signal structureChanged()

    spacing: 7

    readonly property int boxCount: layout && layout.boxes ? layout.boxes.length : 0
    /// 面板当前显示/编辑的框：优先选中的；没选中时回退到最近一次选中的框。
    readonly property int activeIndex: (layout && layout.boxes && selected >= 0 && selected < layout.boxes.length)
                                       ? selected
                                       : ((layout && layout.boxes && lastSelected >= 0
                                           && lastSelected < layout.boxes.length) ? lastSelected : -1)
    readonly property var currentBox: activeIndex >= 0 ? layout.boxes[activeIndex] : null
    /// 预览里的选中框（取消选中后为 null，所以不再画手柄/高亮）。
    readonly property var current: (selected >= 0 && selected < widgets.length) ? widgets[selected] : null
    /// 当前框的轨迹显示样式（保证非空；缺省时用默认/全局值兜底）。
    readonly property var currentStyle: (currentBox && currentBox.style)
                                        ? currentBox.style : defaultStyle()

    property var viewModeNames: ["固定分段", "跟跑者居中", "跑者居中+分段定向"]
    property var placementNames: ["固定左侧", "固定右侧", "自由摆放"]
    property var placementValues: [0, 1, 4]

    // ---------------------------------------------------------------- 数据模型

    function defaultBox() {
        return {mode: 0, share: 0.25, x: 0.03, y: 0.05, w: 0.3, h: 0.6,
                rotation: 0, opacity: 1, shape: 0, feather: 0, corner: 0,
                viewMode: 0, viewZoom: 1.0, lrMargin: 20.0, tbMargin: 20.0, sensitivity: 0.5}
    }

    function copy(v) { return JSON.parse(JSON.stringify(v)) }

    /// 逐框轨迹样式的默认值。必须与 Rust `MapBoxStyle::default()`（= 全局
    /// `Settings::default().route` 的显示字段）逐项一致。
    function defaultStyle() {
        return {
            renderMode: 0, renderScope: 0, width: 10.0,
            tailRenderMode: 3, tailWidth: 10.0, tailLength: 30.0,
            controlRadius: 15.0, showControls: false,
            routeColor: "#000000", routeOpacity: 80.0,
            tailColor: "#000000", tailOpacity: 204.0,
            controlColor: "#000000", controlOpacity: 255.0,
            runnerRadius: 15.0, showRunner: true,
            runnerColor: "#ff5d00", runnerOpacity: 255.0, runnerStyle: 1,
            lowPace: 15.0, highPace: 5.0, paceAuto: true,
            tailLowPace: 16.204162961116385, tailHighPace: 5.117530921453715,
            tailPaceAuto: true
        }
    }

    /// 没有逐框样式时的兜底：用代理上的全局 route_* 显示字段（旧工程/旧探针）。
    /// 代理属性不存在（单元测试 stub）时回默认值。
    function sourceGlobalStyle() {
        var d = defaultStyle()
        if (!source) return d
        function num(prop, fallback) {
            if (source[prop] === undefined || source[prop] === null) return fallback
            var v = Number(source[prop])
            return isFinite(v) ? v : fallback
        }
        function str(prop, fallback) {
            var v = source[prop]
            return (v === undefined || v === null || String(v).trim().length === 0) ? fallback : String(v)
        }
        function bool(prop, fallback) {
            return source[prop] === undefined ? fallback : !!source[prop]
        }
        return {
            renderMode: num("route_render_mode", d.renderMode),
            renderScope: num("route_render_scope", d.renderScope),
            width: num("route_width", d.width),
            tailRenderMode: num("route_tail_render_mode", d.tailRenderMode),
            tailWidth: num("route_tail_width", d.tailWidth),
            tailLength: num("route_tail_length", d.tailLength),
            controlRadius: num("route_control_radius", d.controlRadius),
            showControls: bool("route_show_controls", d.showControls),
            routeColor: str("route_color", d.routeColor),
            routeOpacity: num("route_opacity", d.routeOpacity),
            tailColor: str("route_tail_color", d.tailColor),
            tailOpacity: num("route_tail_opacity", d.tailOpacity),
            controlColor: str("route_control_color", d.controlColor),
            controlOpacity: num("route_control_opacity", d.controlOpacity),
            runnerRadius: num("route_runner_radius", d.runnerRadius),
            showRunner: bool("route_show_runner", d.showRunner),
            runnerColor: str("route_runner_color", d.runnerColor),
            runnerOpacity: num("route_runner_opacity", d.runnerOpacity),
            runnerStyle: num("route_runner_style", d.runnerStyle),
            lowPace: num("route_low_pace", d.lowPace),
            highPace: num("route_high_pace", d.highPace),
            paceAuto: bool("route_pace_auto", d.paceAuto),
            tailLowPace: num("route_tail_low_pace", d.tailLowPace),
            tailHighPace: num("route_tail_high_pace", d.tailHighPace),
            tailPaceAuto: bool("route_tail_pace_auto", d.tailPaceAuto)
        }
    }

    /// 样式对象兜底：缺字段用 fallback 补、数值/布尔/颜色按类型归一。
    function normalizeStyle(st, fallback) {
        var d = fallback || defaultStyle()
        var out = {}
        for (var k in d) {
            var v = (st && st[k] !== undefined && st[k] !== null) ? st[k] : d[k]
            if (typeof d[k] === "boolean") out[k] = !!v
            else if (typeof d[k] === "number") {
                var n = Number(v)
                out[k] = isFinite(n) ? n : d[k]
            } else out[k] = String(v)
        }
        return out
    }

    /// 缺字段/脏值兜底：补齐默认框字段；第 2+ 框强制自由摆放。
    function normalizeBox(b, index) {
        var d = defaultBox()
        var out = {}
        for (var k in d) {
            var v = (b && b[k] !== undefined && b[k] !== null) ? Number(b[k]) : NaN
            out[k] = isFinite(v) ? v : d[k]
        }
        if (index > 0) out.mode = 4
        if (out.mode !== 0 && out.mode !== 1 && out.mode !== 4) out.mode = index > 0 ? 4 : 0
        if (out.viewMode !== 0 && out.viewMode !== 1 && out.viewMode !== 2) out.viewMode = 0
        out.style = normalizeStyle(b ? b.style : null, sourceGlobalStyle())
        return out
    }

    /// 一个框在画布上的有效矩形（归一化）：
    /// - 左贴边：x=0, w=share, h=1；右贴边：x=1-share, w=share, h=1；
    /// - 自由摆放：原 x/y/w/h（旋转按框自己的 rotation）。
    function effectiveRect(b) {
        var mode = Number(b.mode)
        if (mode === 0) return {x: 0, y: 0, w: Number(b.share), h: 1, rotation: 0, shape: 0, corner: 0}
        if (mode === 1) return {x: 1 - Number(b.share), y: 0, w: Number(b.share), h: 1, rotation: 0, shape: 0, corner: 0}
        return {x: Number(b.x), y: Number(b.y), w: Number(b.w), h: Number(b.h),
                rotation: Number(b.rotation), shape: Number(b.shape), corner: Number(b.corner)}
    }

    function makeWidget(b, index) {
        var out = {}
        for (var k in b) out[k] = b[k]
        var r = effectiveRect(b)
        out.index = index
        out.x = r.x
        out.y = r.y
        out.w = r.w
        out.h = r.h
        out.rotation = r.rotation
        out.shape = r.shape
        out.corner = r.corner
        out.visible = true
        // 贴边框只可选中不可拖；自由框 z 高于贴边框（点选时优先命中自由框）。
        out.locked = Number(b.mode) !== 4
        out.z = (Number(b.mode) === 4 ? 1000 : 0) + index
        return out
    }

    function rebuild() {
        var boxes = layout.boxes ? layout.boxes : []
        var ws = []
        for (var i = 0; i < boxes.length; ++i) ws.push(makeWidget(boxes[i], i))
        widgets = ws
        layout = {version: 1, boxes: boxes, widgets: ws}
        revision++
    }

    function viewModeName(b) {
        var m = b ? Number(b.viewMode) : 0
        return root.viewModeNames[m >= 0 && m < root.viewModeNames.length ? m : 0]
    }
    function placementName(b) {
        var mode = b ? Number(b.mode) : 0
        var i = root.placementValues.indexOf(mode)
        return root.placementNames[i < 0 ? 0 : i]
    }

    // ---------------------------------------------------------------- 读写/应用

    /// 把布局写回代理，并在框数不变时让预览就地更新。
    function publish() {
        var json = JSON.stringify({version: 1, boxes: layout.boxes})
        lastSent = json
        if (source && source.map_boxes !== undefined) source.map_boxes = json
        if (renderer && typeof renderer.apply_map_frames === "function"
                && layout.boxes.length === appliedCount) {
            renderer.apply_map_frames(json)
        }
        changed()
    }

    /// 外部变化（打开工程 / 保存配置 / 结构重载后）重新读 `source.map_boxes`。
    function sync() {
        if (!source || source.map_boxes === undefined || source.map_boxes === null) return
        var raw = String(source.map_boxes)
        if (raw === lastSent) {
            // 结构变化后完整重载已把取景器换成新框数：这里把基数对齐，之后的拖动才会
            // 继续走 apply_map_frames。重载在途时提前对齐可能让一次拖动被 Rust 拒绝，
            // 属可接受的过渡态（下一次编辑会同步过去）。
            if (layout.boxes && layout.boxes.length > 0) appliedCount = layout.boxes.length
            return
        }
        if (raw.length === 0) return
        var boxes = null
        try {
            var obj = JSON.parse(raw)
            if (!obj || Number(obj.version) !== 1 || !obj.boxes || !obj.boxes.length) {
                throw new Error("bad layout")
            }
            boxes = []
            for (var i = 0; i < obj.boxes.length; ++i) boxes.push(normalizeBox(obj.boxes[i], i))
        } catch (e) {
            error = "地图框布局解析失败：" + e
            boxes = [normalizeBox(defaultBox(), 0)]
        }
        layout = {version: 1, boxes: boxes}
        // 取消选中（selected=-1）是合法状态：不要强行选回 0；只把越界下标夹回来。
        if (selected >= boxes.length) selected = boxes.length - 1
        if (lastSelected < 0 || lastSelected >= boxes.length) lastSelected = Math.max(0, boxes.length - 1)
        rebuild()
        lastSent = raw
        appliedCount = boxes.length
        undoStack = []
        gestureBefore = ""
    }

    // ---------------------------------------------------------------- 撤销

    function begin() {
        if (!gestureBefore.length) {
            gestureBefore = JSON.stringify({boxes: layout.boxes, selected: activeIndex})
        }
    }
    function end() {
        if (gestureBefore.length) {
            var now = JSON.stringify({boxes: layout.boxes, selected: activeIndex})
            if (gestureBefore !== now) undoStack = undoStack.concat([gestureBefore]).slice(-100)
            gestureBefore = ""
        }
    }
    function undo() {
        if (!undoStack.length) return
        var snapshot = JSON.parse(undoStack[undoStack.length - 1])
        undoStack = undoStack.slice(0, -1)
        var oldCount = layout.boxes.length
        layout = {version: 1, boxes: snapshot.boxes}
        selected = Math.max(0, Math.min(Number(snapshot.selected), snapshot.boxes.length - 1))
        rebuild()
        publish()
        if (layout.boxes.length !== oldCount) structureChanged()
    }

    // ---------------------------------------------------------------- 字段编辑

    /// HudCanvas 的旋转手柄与数值框都走这里。`key` 用 JSON 里的短名。
    function setField(key, value, continuous) {
        if (!currentBox) return
        var n = Number(value)
        if (!isFinite(n)) return
        if (!continuous) begin()
        var boxes = copy(layout.boxes)
        var b = boxes[activeIndex]
        if (key === "mode") b.mode = activeIndex > 0 ? 4 : n
        else if (key === "shape") b.shape = n === 1 ? 1 : 0
        else if (key === "viewMode") b.viewMode = (n === 1 || n === 2) ? n : 0
        else if (key === "rotation") b.rotation = Math.max(-360, Math.min(360, n))
        else if (key === "opacity") b.opacity = Math.max(0, Math.min(1, n))
        else if (key === "feather") b.feather = Math.max(0, Math.min(0.5, n))
        else if (key === "corner") b.corner = Math.max(0, Math.min(0.5, n))
        else if (key === "share") b.share = Math.max(0, Math.min(1, n))
        else b[key] = n
        layout = {version: 1, boxes: boxes}
        rebuild()
        publish()
        if (!continuous) end()
    }

    /// 预览里拖动自由框。HudCanvas 的拖拽/缩放都会调用它。
    function setRect(x, y, w, h) {
        if (!currentBox || Number(currentBox.mode) !== 4) return
        var boxes = copy(layout.boxes)
        var b = boxes[activeIndex]
        b.x = Math.max(-1, Math.min(1, Number(x)))
        b.y = Math.max(-1, Math.min(1, Number(y)))
        b.w = Math.max(0.02, Math.min(1, Number(w)))
        b.h = Math.max(0.02, Math.min(1, Number(h)))
        layout = {version: 1, boxes: boxes}
        rebuild()
        publish()
    }

    // ---------------------------------------------------------------- 结构操作

    function selectFrame(index) {
        if (index < 0 || index >= layout.boxes.length) return
        selected = index
    }

    /// 添加框：自由摆放（右下角画中画），取景方式与第 1 框相反，其余取景参数继承。
    function addBox() {
        if (!layout.boxes.length) return
        begin()
        var boxes = copy(layout.boxes)
        var first = boxes[0]
        var nb = normalizeBox(first, boxes.length)
        nb.mode = 4
        nb.x = 0.6; nb.y = 0.55; nb.w = 0.34; nb.h = 0.4
        nb.rotation = 0; nb.opacity = 1; nb.shape = 0; nb.feather = 0; nb.corner = 0
        nb.viewMode = Number(first.viewMode) === 1 ? 0 : 1
        boxes.push(nb)
        layout = {version: 1, boxes: boxes}
        selected = boxes.length - 1
        rebuild()
        publish()
        end()
        appliedCount = boxes.length   // 等完整重载落地（重载在途时拖动可能被 Rust 拒绝）
        structureChanged()
    }

    /// 复制选中框（含取景参数），位置偏移一点避免完全重叠。
    function duplicateBox() {
        if (!layout.boxes.length) return
        begin()
        var boxes = copy(layout.boxes)
        var src = boxes[activeIndex] || boxes[0]
        var nb = normalizeBox(src, boxes.length)
        nb.mode = 4
        if (Number(src.mode) === 4) {
            nb.x = Math.max(-1, Math.min(0.94, Number(src.x) + 0.04))
            nb.y = Math.max(-1, Math.min(0.94, Number(src.y) + 0.04))
        } else {
            nb.x = 0.6; nb.y = 0.55; nb.w = 0.34; nb.h = 0.4
        }
        boxes.push(nb)
        layout = {version: 1, boxes: boxes}
        selected = boxes.length - 1
        rebuild()
        publish()
        end()
        appliedCount = boxes.length
        structureChanged()
    }

    /// 删除选中框；只剩 1 个时禁用（按钮的 enabled 也拦一层）。
    function removeBox() {
        if (layout.boxes.length <= 1) return
        begin()
        var boxes = copy(layout.boxes)
        boxes.splice(activeIndex, 1)
        layout = {version: 1, boxes: boxes}
        if (selected >= boxes.length) selected = boxes.length - 1
        selected = Math.max(0, selected)
        lastSelected = Math.max(0, Math.min(lastSelected, boxes.length - 1))
        rebuild()
        publish()
        end()
        appliedCount = boxes.length
        structureChanged()
    }

    Component.onCompleted: {
        if (!layout.boxes || !layout.boxes.length) {
            layout = {version: 1, boxes: [normalizeBox(defaultBox(), 0)]}
            rebuild()
        }
        sync()
    }

    // ---------------------------------------------------------------- 逐框轨迹样式

    /// 当前样式字段值（currentBox 为空/缺字段时回默认值）。
    function styleValue(key) {
        var st = currentBox ? currentBox.style : null
        if (st && st[key] !== undefined && st[key] !== null) return st[key]
        return defaultStyle()[key]
    }

    /// 写一个逐框样式字段：改 layout.boxes[activeIndex].style → rebuild/publish。
    /// 框数不变，publish 会走 `renderer.apply_map_frames` 即时生效（不重载）。
    function setStyle(key, value, continuous) {
        if (!currentBox) return
        if (!continuous) begin()
        var boxes = copy(layout.boxes)
        var b = boxes[activeIndex]
        if (!b.style) b.style = sourceGlobalStyle()
        b.style[key] = value
        layout = {version: 1, boxes: boxes}
        rebuild()
        publish()
        if (!continuous) end()
    }

    /// 勾选类样式（配速色带自适应）：取消勾选时先把低/高配速对齐到**本框当前实际
    /// 生效**的色带（renderer.map_box_pace_range，返回该框端点 JSON），再取消勾选 ——
    /// 否则滑杆会跳回工程里存的旧值、画面颜色跟着突变一下。
    function setStyleCheck(key, checked) {
        if (checked) {
            setStyle(key, true, false)
            return
        }
        if ((key === "paceAuto" || key === "tailPaceAuto")
                && renderer && typeof renderer.map_box_pace_range === "function") {
            var range = null
            try { range = JSON.parse(String(renderer.map_box_pace_range(activeIndex))) } catch (e) { range = null }
            if (range) {
                begin()
                var boxes = copy(layout.boxes)
                var b = boxes[activeIndex]
                if (!b.style) b.style = sourceGlobalStyle()
                if (key === "paceAuto") {
                    if (isFinite(Number(range.low)) && Number(range.low) > 0) b.style.lowPace = Number(range.low)
                    if (isFinite(Number(range.high)) && Number(range.high) > 0) b.style.highPace = Number(range.high)
                } else {
                    if (isFinite(Number(range.tailLow)) && Number(range.tailLow) > 0) b.style.tailLowPace = Number(range.tailLow)
                    if (isFinite(Number(range.tailHigh)) && Number(range.tailHigh) > 0) b.style.tailHighPace = Number(range.tailHigh)
                }
                b.style[key] = false
                layout = {version: 1, boxes: boxes}
                rebuild()
                publish()
                end()
                return
            }
        }
        setStyle(key, false, false)
    }

    // 颜色存储约定与 gui.qml 一致：设置里只存 `#rrggbb`，不透明度是独立的 0..255
    // 样式字段；QML 颜色字面量是 `#aarrggbb`（alpha 在前），跨这一步要换序。
    function qtColor(v, opacity) {
        var t = String(v === undefined || v === null ? "" : v).trim()
        var hex = t
        if (t.length === 9 && t.charAt(0) === "#") hex = "#" + t.substr(7, 2) + t.substr(1, 6)
        if (opacity === undefined || opacity < 0 || hex.length !== 7 || hex.charAt(0) !== "#") return hex
        var a = Math.round(Math.max(0, Math.min(255, Number(opacity))))
        var s = a.toString(16)
        if (s.length < 2) s = "0" + s
        return "#" + s + hex.substr(1, 6)
    }
    function qtRgba(v, opacity) {
        var c = root.qtColor(v, opacity)
        if (c.length !== 9 || c.charAt(0) !== "#") return "transparent"
        return Qt.rgba(parseInt(c.substr(3, 2), 16) / 255,
                       parseInt(c.substr(5, 2), 16) / 255,
                       parseInt(c.substr(7, 2), 16) / 255,
                       parseInt(c.substr(1, 2), 16) / 255)
    }
    // QColorDialog 给回的 color → 存储用的 "#rrggbb"（alpha 由独立的透明度滑杆负责）。
    function colorHex(c) {
        function hx(x) {
            var n = Math.round(Math.max(0, Math.min(1, Number(x))) * 255)
            var s = n.toString(16)
            return s.length < 2 ? "0" + s : s
        }
        return "#" + hx(c.r) + hx(c.g) + hx(c.b)
    }
    function pickStyleColor(colorKey, opacityKey) {
        stylePicker.colorKey = colorKey
        stylePicker.opacityKey = opacityKey
        try { stylePicker.selectedColor = root.qtRgba(root.styleValue(colorKey), Number(root.styleValue(opacityKey))) } catch (e) { }
        stylePicker.open()
    }

    // ---------------------------------------------------------------- 框标签条

    // 标签单独一行、Flow 自动换行：框一多也不会把下面的操作按钮挤出编辑区。
    Flow {
        id: frameTabFlow
        width: parent.width
        spacing: 4
        Repeater {
            model: root.layout.boxes
            delegate: Button {
                required property int index
                objectName: "mapFrameTab_" + index
                text: (index + 1) + "·" + root.viewModeName(root.layout.boxes[index])
                font.pixelSize: 12
                implicitHeight: 26
                implicitWidth: Math.min(root.width - 8, Math.max(64, contentItem.implicitWidth + 20))
                palette.buttonText: root.activeIndex === index ? root.textOnAccent : root.textColor
                background: Rectangle {
                    radius: 3
                    border.width: 1
                    border.color: root.activeIndex === index ? root.accentColor : root.borderColor
                    color: root.activeIndex === index ? root.accentColor : root.buttonColor
                }
                onClicked: root.selectFrame(index)
            }
        }
    }
    // 操作按钮单独一行常驻：框数量再多也始终在编辑器宽度内可点。
    RowLayout {
        width: parent.width
        spacing: 4
        Button {
            objectName: "mapFrameAdd"
            text: "+ 添加"
            font.pixelSize: 12
            Layout.preferredHeight: 26
            onClicked: root.addBox()
            ToolTip.visible: hovered
            ToolTip.text: "新增一个自由摆放的地图框（取景方式默认与第 1 框相反）"
            ToolTip.delay: 400
        }
        Button {
            objectName: "mapFrameRemove"
            text: "删除"
            font.pixelSize: 12
            Layout.preferredHeight: 26
            enabled: root.layout.boxes.length > 1
            onClicked: root.removeBox()
            ToolTip.visible: hovered && !enabled
            ToolTip.text: "至少要保留一个地图框"
            ToolTip.delay: 400
        }
        Item { Layout.fillWidth: true }
    }

    // ---------------------------------------------------------------- 选中框子面板

    // 地图位置：只有第 1 框可选「固定左侧/右侧」；第 2+ 框固定自由摆放。
    RowLayout {
        width: parent.width
        visible: root.currentBox !== null
        spacing: 6
        Label { text: "地图位置"; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        ComboBox {
            objectName: "mapPlacementChoice"
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            model: root.placementNames
            enabled: root.activeIndex === 0
            currentIndex: (root.activeIndex === 0 && root.currentBox)
                          ? Math.max(0, root.placementValues.indexOf(Number(root.currentBox.mode))) : 2
            onActivated: {
                if (root.activeIndex === 0) root.setField("mode", root.placementValues[currentIndex], false)
            }
        }
    }
    // 贴边宽度（只有贴边模式用）。
    RowLayout {
        width: parent.width
        visible: root.currentBox !== null && Number(root.currentBox.mode) !== 4
        spacing: 6
        Label { text: "地图宽度 (%)"; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapTextField {
            objectName: "mapStripShare"
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            enabled: root.hasVideo
            text: ((root.hasVideo ? currentShare() : 1) * 100).toFixed(1)
            validator: DoubleValidator { bottom: 0; top: 100; locale: "C" }
            onEditingFinished: {
                if (!root.hasVideo) return
                var n = Number(text)
                if (isFinite(n)) root.setField("share", Math.max(0, Math.min(1, n / 100)), false)
            }
            ToolTip.visible: hovered
            ToolTip.text: "没有视频时地图铺满整帧，画面占比固定 100%"
            ToolTip.delay: 400
        }
    }
    function currentShare() {
        return (root.currentBox && isFinite(Number(root.currentBox.share))) ? Number(root.currentBox.share) : 0.25
    }

    // 自由摆放外观。
    RowLayout {
        width: parent.width
        visible: root.currentBox !== null && Number(root.currentBox.mode) === 4
        spacing: 6
        Label { text: "地图形状"; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        ComboBox {
            objectName: "mapShapeChoice"
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            model: ["矩形", "圆形"]
            currentIndex: root.currentBox ? (Number(root.currentBox.shape) === 1 ? 1 : 0) : 0
            onActivated: root.setField("shape", currentIndex, false)
        }
    }
    Repeater {
        model: [{key: "opacity", label: "不透明度 (%)", factor: 100, min: 0, max: 100},
                {key: "feather", label: "边框羽化 (%)", factor: 100, min: 0, max: 50},
                {key: "corner", label: "圆角 (%)", factor: 100, min: 0, max: 50}]
        delegate: PercentSlider {
            required property var modelData
            field: modelData
            visible: root.currentBox !== null && Number(root.currentBox.mode) === 4
                     && (modelData.key !== "corner" || Number(root.currentBox.shape) === 0)
        }
    }

    RowLayout {
        width: parent.width
        visible: root.currentBox !== null
        spacing: 12
        MapCheckBox {
            text: "吸附对齐"
            checked: root.snapping
            onToggled: root.snapping = checked
        }
    }

    // 自由摆放的位置/尺寸/旋转。
    Repeater {
        model: [{key: "x", label: "水平位置 (%)", factor: 100, min: -100, max: 100},
                {key: "y", label: "垂直位置 (%)", factor: 100, min: -100, max: 100},
                {key: "w", label: "地图框宽度 (%)", factor: 100, min: 2, max: 100},
                {key: "h", label: "地图框高度 (%)", factor: 100, min: 2, max: 100},
                {key: "rotation", label: "地图框旋转 (°)", factor: 1, min: -360, max: 360}]
        delegate: FrameField {
            required property var modelData
            field: modelData
            visible: root.currentBox !== null && Number(root.currentBox.mode) === 4
        }
    }

    // ---------------------------------------------------------------- 取景

    Label {
        width: parent.width
        visible: root.currentBox !== null
        topPadding: 6
        bottomPadding: 1
        font.pixelSize: 14
        font.bold: true
        color: root.textColor
        text: "取景"
    }

    // 视野模式（本框独立）。
    RowLayout {
        width: parent.width
        visible: root.currentBox !== null
        spacing: 6
        Label { text: "视野模式"; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        ComboBox {
            objectName: "mapViewModeChoice"
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            model: root.viewModeNames
            currentIndex: root.currentBox ? Math.max(0, root.viewModeNames.indexOf(root.viewModeName(root.currentBox))) : 0
            onActivated: root.setField("viewMode", currentIndex, false)
        }
    }
    ViewNumber {
        objectName: "mapViewLrMargin"
        label: "左右边距"
        stored: root.currentBox ? Number(root.currentBox.lrMargin) : 20
        visible: root.currentBox !== null && Number(root.currentBox.viewMode) === 0
        onCommitted: function (v) { root.setField("lrMargin", v, false) }
    }
    ViewNumber {
        objectName: "mapViewTbMargin"
        label: "上下边距"
        stored: root.currentBox ? Number(root.currentBox.tbMargin) : 20
        visible: root.currentBox !== null && Number(root.currentBox.viewMode) === 0
        onCommitted: function (v) { root.setField("tbMargin", v, false) }
    }
    ViewSlider {
        objectName: "mapViewZoom"
        label: "取景缩放 (%)"
        from: 2; to: 300; decimals: 0; displayScale: 100; suffix: "%"
        stored: root.currentBox ? Number(root.currentBox.viewZoom) : 1.0
        visible: root.currentBox !== null && Number(root.currentBox.viewMode) !== 0
        hint: "地图的放大倍数（跟跑者居中模式）"
        onCommitted: function (v, continuous) { root.setField("viewZoom", v, continuous) }
    }
    ViewSlider {
        objectName: "mapViewSensitivity"
        label: "灵敏度"
        from: 0.1; to: 5; decimals: 2; displayScale: 1
        stored: root.currentBox ? Number(root.currentBox.sensitivity) : 0.5
        visible: root.currentBox !== null && Number(root.currentBox.viewMode) !== 0
        hint: "镜头灵敏度：越小画面越稳，但跑者移动时镜头会更慢跟上。"
        onCommitted: function (v, continuous) { root.setField("sensitivity", v, continuous) }
    }

    // ---------------------------------------------------------------- 轨迹显示
    //
    // 路线 / 尾巴 / 控制点 / 运动员 四组样式逐框独立；改动走 `setStyle()` →
    // `renderer.apply_map_frames` 就地生效（框数不变，不触发完整重载）。
    // 颜色用本文件内的 HudColorDialog；色块按「颜色 + 透明度」合成显示。

    Label {
        width: parent.width
        visible: root.currentBox !== null
        topPadding: 10
        bottomPadding: 1
        font.pixelSize: 14
        font.bold: true
        color: root.textColor
        text: "轨迹显示"
    }

    StyleSubSection { title: "路线"; visible: root.currentBox !== null }
    StyleCombo {
        objectName: "boxStyle_renderMode"
        label: "路线渲染模式"; key: "renderMode"
        items: ["无", "简洁", "配速"]; values: [0, 1, 3]
        value: Number(root.currentStyle.renderMode)
        visible: root.currentBox !== null
    }
    StyleCombo {
        objectName: "boxStyle_renderScope"
        label: "渲染范围"; key: "renderScope"
        items: ["完整渲染", "单段渲染"]; values: [0, 1]
        value: Number(root.currentStyle.renderScope)
        hint: "完整渲染 = 整条路线；单段渲染 = 只画当前所在的那一段（判定与取景一致）。"
        visible: root.currentBox !== null && Number(root.currentStyle.renderMode) !== 0
    }
    StyleSpin {
        objectName: "boxStyle_width"
        label: "路线宽度 (像素)"; key: "width"
        value: Number(root.currentStyle.width)
        visible: root.currentBox !== null && Number(root.currentStyle.renderMode) !== 0
    }
    StyleColor {
        objectName: "boxStyle_routeColor"
        label: "路线颜色"; colorKey: "routeColor"; opacityKey: "routeOpacity"
        colorValue: String(root.currentStyle.routeColor)
        opacityValue: Number(root.currentStyle.routeOpacity)
        visible: root.currentBox !== null && Number(root.currentStyle.renderMode) !== 0
                 && Number(root.currentStyle.renderMode) !== 3
    }
    StyleSlider {
        objectName: "boxStyle_routeOpacity"
        label: "路线透明度 (%)"; key: "routeOpacity"
        alphaPercent: true; from: 0; to: 100; decimals: 0
        value: Number(root.currentStyle.routeOpacity)
        hint: "0% = 完全透明，100% = 完全不透明（配速模式下逐段线也用这个不透明度）"
        visible: root.currentBox !== null && Number(root.currentStyle.renderMode) !== 0
    }
    StyleCheck {
        objectName: "boxStyle_paceAuto"
        label: "配速色带自适应"; key: "paceAuto"
        value: !!root.currentStyle.paceAuto
        hint: "按实际轨迹的配速分布自动定色带上下限（掐掉最快/最慢各 10% 的极值）"
        visible: root.currentBox !== null && Number(root.currentStyle.renderMode) === 3
    }
    StyleSpin {
        objectName: "boxStyle_lowPace"
        label: "低配速 (分钟/公里)"; key: "lowPace"
        value: Number(root.currentStyle.lowPace)
        hint: "配速达到这个值（更慢）画成红色"
        visible: root.currentBox !== null && Number(root.currentStyle.renderMode) === 3
                 && !root.currentStyle.paceAuto
    }
    StyleSpin {
        objectName: "boxStyle_highPace"
        label: "高配速 (分钟/公里)"; key: "highPace"
        value: Number(root.currentStyle.highPace)
        hint: "配速达到这个值（更快）画成绿色"
        visible: root.currentBox !== null && Number(root.currentStyle.renderMode) === 3
                 && !root.currentStyle.paceAuto
    }

    StyleSubSection { title: "尾巴"; visible: root.currentBox !== null }
    StyleCombo {
        objectName: "boxStyle_tailRenderMode"
        label: "尾巴渲染模式"; key: "tailRenderMode"
        items: ["无", "简洁", "配速"]; values: [0, 1, 3]
        value: Number(root.currentStyle.tailRenderMode)
        hint: "无 = 不画尾巴；简洁 = 单色拖尾；配速 = 按配速渐变着色"
        visible: root.currentBox !== null
    }
    StyleSpin {
        objectName: "boxStyle_tailWidth"
        label: "尾巴宽度 (像素)"; key: "tailWidth"
        value: Number(root.currentStyle.tailWidth)
        visible: root.currentBox !== null && Number(root.currentStyle.tailRenderMode) !== 0
    }
    StyleSpin {
        objectName: "boxStyle_tailLength"
        label: "尾巴长度 (秒)"; key: "tailLength"
        value: Number(root.currentStyle.tailLength)
        visible: root.currentBox !== null && Number(root.currentStyle.tailRenderMode) !== 0
    }
    StyleColor {
        objectName: "boxStyle_tailColor"
        label: "尾巴颜色"; colorKey: "tailColor"; opacityKey: "tailOpacity"
        colorValue: String(root.currentStyle.tailColor)
        opacityValue: Number(root.currentStyle.tailOpacity)
        visible: root.currentBox !== null && Number(root.currentStyle.tailRenderMode) !== 0
                 && Number(root.currentStyle.tailRenderMode) !== 3
    }
    StyleSlider {
        objectName: "boxStyle_tailOpacity"
        label: "尾巴透明度 (%)"; key: "tailOpacity"
        alphaPercent: true; from: 0; to: 100; decimals: 0
        value: Number(root.currentStyle.tailOpacity)
        hint: "0% = 完全透明，100% = 完全不透明"
        visible: root.currentBox !== null && Number(root.currentStyle.tailRenderMode) !== 0
    }
    StyleCheck {
        objectName: "boxStyle_tailPaceAuto"
        label: "尾巴配速色带自适应"; key: "tailPaceAuto"
        value: !!root.currentStyle.tailPaceAuto
        hint: "按实际轨迹的配速分布自动定色带上下限（掐掉最快/最慢各 10% 的极值）"
        visible: root.currentBox !== null && Number(root.currentStyle.tailRenderMode) === 3
    }
    StyleSpin {
        objectName: "boxStyle_tailLowPace"
        label: "尾巴低配速 (分钟/公里)"; key: "tailLowPace"
        value: Number(root.currentStyle.tailLowPace)
        hint: "配速达到这个值（更慢）画成红色"
        visible: root.currentBox !== null && Number(root.currentStyle.tailRenderMode) === 3
                 && !root.currentStyle.tailPaceAuto
    }
    StyleSpin {
        objectName: "boxStyle_tailHighPace"
        label: "尾巴高配速 (分钟/公里)"; key: "tailHighPace"
        value: Number(root.currentStyle.tailHighPace)
        hint: "配速达到这个值（更快）画成绿色"
        visible: root.currentBox !== null && Number(root.currentStyle.tailRenderMode) === 3
                 && !root.currentStyle.tailPaceAuto
    }

    StyleSubSection { title: "控制点"; visible: root.currentBox !== null }
    StyleCheck {
        objectName: "boxStyle_showControls"
        label: "显示控制点"; key: "showControls"
        value: !!root.currentStyle.showControls
        visible: root.currentBox !== null
    }
    StyleSpin {
        objectName: "boxStyle_controlRadius"
        label: "控制点半径"; key: "controlRadius"
        value: Number(root.currentStyle.controlRadius)
        visible: root.currentBox !== null && !!root.currentStyle.showControls
    }
    StyleColor {
        objectName: "boxStyle_controlColor"
        label: "控制点颜色"; colorKey: "controlColor"; opacityKey: "controlOpacity"
        colorValue: String(root.currentStyle.controlColor)
        opacityValue: Number(root.currentStyle.controlOpacity)
        visible: root.currentBox !== null && !!root.currentStyle.showControls
    }
    StyleSlider {
        objectName: "boxStyle_controlOpacity"
        label: "控制点透明度 (%)"; key: "controlOpacity"
        alphaPercent: true; from: 0; to: 100; decimals: 0
        value: Number(root.currentStyle.controlOpacity)
        hint: "0% = 完全透明，100% = 完全不透明"
        visible: root.currentBox !== null && !!root.currentStyle.showControls
    }

    StyleSubSection { title: "运动员"; visible: root.currentBox !== null }
    StyleCheck {
        objectName: "boxStyle_showRunner"
        label: "显示运动员"; key: "showRunner"
        value: !!root.currentStyle.showRunner
        visible: root.currentBox !== null
    }
    StyleCombo {
        objectName: "boxStyle_runnerStyle"
        label: "运动员样式"; key: "runnerStyle"
        items: ["圆形", "箭头"]; values: [0, 1]
        value: Number(root.currentStyle.runnerStyle)
        visible: root.currentBox !== null && !!root.currentStyle.showRunner
    }
    StyleSpin {
        objectName: "boxStyle_runnerRadius"
        label: "运动员半径"; key: "runnerRadius"
        value: Number(root.currentStyle.runnerRadius)
        visible: root.currentBox !== null && !!root.currentStyle.showRunner
    }
    StyleColor {
        objectName: "boxStyle_runnerColor"
        label: "运动员颜色"; colorKey: "runnerColor"; opacityKey: "runnerOpacity"
        colorValue: String(root.currentStyle.runnerColor)
        opacityValue: Number(root.currentStyle.runnerOpacity)
        visible: root.currentBox !== null && !!root.currentStyle.showRunner
    }
    StyleSlider {
        objectName: "boxStyle_runnerOpacity"
        label: "运动员透明度 (%)"; key: "runnerOpacity"
        alphaPercent: true; from: 0; to: 100; decimals: 0
        value: Number(root.currentStyle.runnerOpacity)
        hint: "0% = 完全透明，100% = 完全不透明"
        visible: root.currentBox !== null && !!root.currentStyle.showRunner
    }

    // 取色对话框：与 gui.qml 的全局取色同款（HudColorDialog 就是 Qt ColorDialog 的薄封装）。
    HudColorDialog {
        id: stylePicker
        objectName: "boxStyleColorPicker"
        property string colorKey: ""
        property string opacityKey: ""
        onAccepted: {
            if (root.currentBox && stylePicker.colorKey.length > 0) {
                root.setStyle(stylePicker.colorKey, root.colorHex(selectedColor), false)
            }
        }
    }

    // ---------------------------------------------------------------- 组件

    component MapTextField: TextField {
        id: fieldControl
        color: root.textColor
        selectByMouse: true
        leftPadding: 10; rightPadding: 10
        background: Rectangle {
            color: root.fieldColor
            radius: 5
            border.width: fieldControl.activeFocus ? 2 : 1
            border.color: fieldControl.activeFocus ? root.accentColor : root.borderColor
        }
    }
    component MapSlider: Slider {
        id: sliderControl
        implicitHeight: 28
        background: Rectangle {
            x: sliderControl.leftPadding + 9
            y: sliderControl.topPadding + sliderControl.availableHeight / 2 - 2
            width: Math.max(0, sliderControl.availableWidth - 18)
            height: 4; radius: 2; color: root.borderColor
            Rectangle { width: parent.width * sliderControl.visualPosition; height: 4; radius: 2; color: root.accentColor }
        }
        handle: Rectangle {
            x: sliderControl.leftPadding + sliderControl.visualPosition * (sliderControl.availableWidth - width)
            y: sliderControl.topPadding + (sliderControl.availableHeight - height) / 2
            width: 18; height: 18; radius: 9
            color: root.accentColor
            border.width: sliderControl.activeFocus ? 2 : 0
            border.color: root.textColor
            scale: sliderControl.pressed ? 1.1 : 1
            Behavior on scale { NumberAnimation { duration: 120 } }
        }
    }

    component FrameField: RowLayout {
        id: ff
        required property var field
        width: root.width
        spacing: 6
        Label { text: ff.field.label; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapTextField {
            objectName: "mapFrame_" + ff.field.key
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            text: {
                root.revision
                var v = root.currentBox ? Number(root.currentBox[ff.field.key]) : 0
                return ((isFinite(v) ? v : 0) * ff.field.factor).toFixed(1)
            }
            validator: DoubleValidator { bottom: ff.field.min; top: ff.field.max; locale: "C" }
            onEditingFinished: {
                var n = Number(text)
                if (isFinite(n)) {
                    root.setField(ff.field.key, Math.max(ff.field.min, Math.min(ff.field.max, n)) / ff.field.factor, false)
                }
            }
        }
    }

    // 外观类数值（不透明度 / 羽化 / 圆角）：滑杆 + 数值框，拖动即时生效、一次拖动一步撤销。
    component PercentSlider: GridLayout {
        id: ps
        required property var field
        width: root.width
        columns: 2
        columnSpacing: 10
        rowSpacing: 0
        objectName: "mapFrame_" + field.key
        function displayValue() {
            var v = root.currentBox ? Number(root.currentBox[ps.field.key]) : 0
            return (isFinite(v) ? v : 0) * ps.field.factor
        }
        function sync() {
            var v = Math.max(ps.field.min, Math.min(ps.field.max, ps.displayValue()))
            if (Math.abs(slider.value - v) > 1e-9) slider.value = v
        }
        function commit(display) {
            var d = Math.max(ps.field.min, Math.min(ps.field.max, display))
            root.setField(ps.field.key, d / ps.field.factor, true)
        }
        Label { text: ps.field.label; Layout.fillWidth: true; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapSlider {
            id: slider
            objectName: "mapFrame_" + ps.field.key + "Slider"
            Layout.fillWidth: true
            Layout.row: 1; Layout.column: 0; Layout.columnSpan: 2
            from: ps.field.min; to: ps.field.max
            stepSize: 1
            live: true
            onMoved: ps.commit(value)
            onPressedChanged: { if (pressed) root.begin(); else root.end() }
        }
        MapTextField {
            objectName: "mapFrame_" + ps.field.key + "Field"
            Layout.row: 0; Layout.column: 1
            Layout.preferredWidth: 64; Layout.preferredHeight: 28; font.pixelSize: 12
            text: { root.revision; return ps.displayValue().toFixed(1) }
            validator: DoubleValidator { bottom: ps.field.min; top: ps.field.max; locale: "C" }
            onEditingFinished: {
                var n = Number(text)
                if (isFinite(n)) { root.begin(); ps.commit(n); root.end() }
            }
        }
        Component.onCompleted: sync()
        Connections {
            target: root
            function onRevisionChanged() { ps.sync() }
            function onSelectedChanged() { ps.sync() }
            function onCurrentBoxChanged() { ps.sync() }
        }
    }

    // 取景数值框（左右/上下边距）：按轨迹地图单位计，不做百分比换算。
    component ViewNumber: RowLayout {
        id: vn
        property string label: ""
        property real stored: 0
        signal committed(real value)
        width: root.width
        spacing: 6
        Label { text: vn.label; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapTextField {
            objectName: (vn.objectName || "") + "Field"
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            text: { root.revision; return (isFinite(vn.stored) ? vn.stored : 0).toFixed(1) }
            validator: DoubleValidator { bottom: 0; top: 100000; locale: "C" }
            onEditingFinished: {
                var n = Number(text)
                if (isFinite(n)) vn.committed(n)
            }
        }
    }

    // 取景滑杆 + 数值框（缩放 %、灵敏度）。
    component ViewSlider: GridLayout {
        id: vs
        property string label: ""
        property string hint: ""
        property real stored: 0
        property real from: 0
        property real to: 100
        property int decimals: 0
        // 显示单位换算（取景缩放按 % 显示：displayScale=100）。刻意不叫 `scale`：
        // Item 自带 scale 是绘制变换，同名会遮蔽基类属性、把整行放大。
        property real displayScale: 1
        property string suffix: ""
        signal committed(real value, bool continuous)
        width: root.width
        columns: 2
        columnSpacing: 10
        rowSpacing: 0
        function displayValue() {
            var v = Number(vs.stored)
            return (isFinite(v) ? v : 0) * vs.displayScale
        }
        function sync() {
            var v = Math.max(vs.from, Math.min(vs.to, vs.displayValue()))
            if (Math.abs(slider.value - v) > 1e-9) slider.value = v
        }
        function commit(display, continuous) {
            var d = Math.max(vs.from, Math.min(vs.to, display))
            vs.committed(d / vs.displayScale, continuous)
        }
        Label { text: vs.label; Layout.fillWidth: true; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapSlider {
            id: slider
            objectName: (vs.objectName || "") + "Slider"
            Layout.fillWidth: true
            Layout.row: 1; Layout.column: 0; Layout.columnSpan: 2
            from: vs.from; to: vs.to
            stepSize: vs.decimals > 0 ? Math.pow(10, -vs.decimals) : 1
            live: true
            onMoved: vs.commit(value, true)
            onPressedChanged: { if (pressed) root.begin(); else root.end() }
        }
        MapTextField {
            objectName: (vs.objectName || "") + "Field"
            Layout.row: 0; Layout.column: 1
            Layout.preferredWidth: 70; Layout.preferredHeight: 28; font.pixelSize: 12
            text: { root.revision; return vs.displayValue().toFixed(vs.decimals) }
            validator: DoubleValidator { bottom: vs.from; top: vs.to; locale: "C" }
            onEditingFinished: {
                var n = Number(text)
                if (isFinite(n)) { root.begin(); vs.commit(n, false); root.end() }
            }
        }
        Label { visible: vs.suffix.length > 0; text: vs.suffix; color: root.mutedColor; font.pixelSize: 11; Layout.preferredWidth: 24 }
        HoverHandler { id: vsHover }
        ToolTip.visible: vsHover.hovered && vs.hint.length > 0
        ToolTip.text: vs.hint
        ToolTip.delay: 300
        Component.onCompleted: sync()
        Connections {
            target: root
            function onRevisionChanged() { vs.sync() }
            function onSelectedChanged() { vs.sync() }
            function onCurrentBoxChanged() { vs.sync() }
        }
    }

    // ---- 逐框轨迹样式控件（路线/尾巴/控制点/运动员） ----
    // 统一 objectName：boxStyle_<key>（行）+ boxStyle_<key>Combo/Slider/Field/Swatch/Pick/Check。
    // 写值一律走 root.setStyle()/root.setStyleCheck()，框数不变 → apply_map_frames 即时生效。

    component StyleSubSection: Column {
        id: sss
        property string title: ""
        width: root.width
        topPadding: 4
        Row {
            spacing: 6
            Rectangle {
                width: 3
                height: titleLabel.height
                radius: 1.5
                color: root.accentColor
            }
            Label {
                id: titleLabel
                text: sss.title
                color: root.textColor
                font.pixelSize: 13
                font.bold: true
            }
        }
    }

    component StyleCombo: RowLayout {
        id: scm
        property string label: ""
        property string key: ""
        property var items: []
        property var values: null
        property int value: 0
        property string hint: ""
        width: root.width
        spacing: 6
        Label { text: scm.label; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        ComboBox {
            id: cb
            objectName: (scm.objectName || "") + "Combo"
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            model: scm.items
            currentIndex: {
                var vals = scm.values || []
                var i = vals.indexOf(Number(scm.value))
                return i < 0 ? 0 : i
            }
            onActivated: root.setStyle(scm.key, scm.values ? scm.values[currentIndex] : currentIndex, false)
        }
        HoverHandler { id: scmHover }
        ToolTip.visible: scmHover.hovered && scm.hint.length > 0
        ToolTip.text: scm.hint
        ToolTip.delay: 400
        Connections {
            target: root
            function onRevisionChanged() {
                var vals = scm.values || []
                var i = vals.indexOf(Number(scm.value))
                if (i >= 0 && cb.currentIndex !== i) cb.currentIndex = i
            }
        }
    }

    component StyleSpin: RowLayout {
        id: ssp
        property string label: ""
        property string key: ""
        property real value: 0
        property int decimals: 2
        property string hint: ""
        width: root.width
        spacing: 6
        Label { text: ssp.label; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapTextField {
            objectName: (ssp.objectName || "") + "Field"
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            font.pixelSize: 13
            text: { root.revision; return (isFinite(ssp.value) ? ssp.value : 0).toFixed(ssp.decimals) }
            validator: DoubleValidator { bottom: 0; top: 100000; locale: "C" }
            onEditingFinished: {
                var n = Number(text)
                if (isFinite(n)) root.setStyle(ssp.key, n, false)
            }
        }
        HoverHandler { id: sspHover }
        ToolTip.visible: sspHover.hovered && ssp.hint.length > 0
        ToolTip.text: ssp.hint
        ToolTip.delay: 400
    }

    component StyleSlider: GridLayout {
        id: ssl
        property string label: ""
        property string key: ""
        property real value: 0
        property real from: 0
        property real to: 100
        property int decimals: 0
        /// 透明度：存储 0..255，界面显示 0..100%。
        property bool alphaPercent: false
        property string hint: ""
        width: root.width
        columns: 2
        columnSpacing: 10
        rowSpacing: 0
        function displayValue() {
            var v = Number(ssl.value)
            if (!isFinite(v)) v = 0
            return ssl.alphaPercent ? v * 100 / 255 : v
        }
        function storedValue(display) {
            return ssl.alphaPercent ? Math.round(display * 255 / 100) : display
        }
        function sync() {
            var v = Math.max(ssl.from, Math.min(ssl.to, ssl.displayValue()))
            if (Math.abs(slider.value - v) > 1e-9) slider.value = v
        }
        function commit(display, continuous) {
            var d = Math.max(ssl.from, Math.min(ssl.to, display))
            root.setStyle(ssl.key, ssl.storedValue(d), continuous)
        }
        Label { text: ssl.label; Layout.fillWidth: true; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapSlider {
            id: slider
            objectName: (ssl.objectName || "") + "Slider"
            Layout.fillWidth: true
            Layout.row: 1; Layout.column: 0; Layout.columnSpan: 2
            from: ssl.from; to: ssl.to
            stepSize: ssl.decimals > 0 ? Math.pow(10, -ssl.decimals) : 1
            live: true
            onMoved: ssl.commit(value, true)
            onPressedChanged: { if (pressed) root.begin(); else root.end() }
        }
        MapTextField {
            objectName: (ssl.objectName || "") + "Field"
            Layout.row: 0; Layout.column: 1
            Layout.preferredWidth: 70; Layout.preferredHeight: 28; font.pixelSize: 12
            text: { root.revision; return ssl.displayValue().toFixed(ssl.decimals) }
            validator: DoubleValidator { bottom: ssl.from; top: ssl.to; locale: "C" }
            onEditingFinished: {
                var n = Number(text)
                if (isFinite(n)) { root.begin(); ssl.commit(n, false); root.end() }
            }
        }
        HoverHandler { id: sslHover }
        ToolTip.visible: sslHover.hovered && ssl.hint.length > 0
        ToolTip.text: ssl.hint
        ToolTip.delay: 300
        Component.onCompleted: sync()
        Connections {
            target: root
            function onRevisionChanged() { ssl.sync() }
            function onSelectedChanged() { ssl.sync() }
            function onCurrentBoxChanged() { ssl.sync() }
        }
    }

    component StyleColor: RowLayout {
        id: scc
        property string label: ""
        property string colorKey: ""
        property string opacityKey: ""
        property string colorValue: "#000000"
        property real opacityValue: 255
        width: root.width
        spacing: 6
        Label { text: scc.label; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        Rectangle {
            objectName: (scc.objectName || "") + "Swatch"
            Layout.preferredWidth: 44; Layout.preferredHeight: 24
            color: { root.revision; return root.qtColor(scc.colorValue, scc.opacityValue) }
            border.color: root.mutedColor; border.width: 1; radius: 2
        }
        Button {
            objectName: (scc.objectName || "") + "Pick"
            text: "拾取"
            font.pixelSize: 13
            Layout.preferredHeight: 28
            Layout.preferredWidth: 64
            onClicked: root.pickStyleColor(scc.colorKey, scc.opacityKey)
        }
        Item { Layout.fillWidth: true; Layout.fillHeight: true }
    }

    // 布尔配置使用开关，保留调用方的设置写回。
    component MapCheckBox: Switch {
        id: mcb
        font.pixelSize: 13
        spacing: 6
        leftPadding: 0
        rightPadding: 0
        contentItem: Text {
            text: mcb.text
            font: mcb.font
            color: root.textColor
            leftPadding: mcb.indicator.width + mcb.spacing
            verticalAlignment: Text.AlignVCenter
        }
        indicator: Rectangle {
            implicitWidth: 38 * root.uiScale
            implicitHeight: 22 * root.uiScale
            y: (mcb.height - height) / 2
            radius: height / 2
            color: mcb.checked ? root.accentColor : root.surfaceHoverColor
            border.width: mcb.activeFocus ? 2 : 1
            border.color: mcb.checked || mcb.activeFocus ? root.accentColor : root.borderColor
            opacity: mcb.enabled ? 1 : 0.45
            Behavior on color { ColorAnimation { duration: 140 } }
            Rectangle {
                x: mcb.checked ? parent.width - width - 3 * root.uiScale : 3 * root.uiScale
                anchors.verticalCenter: parent.verticalCenter
                width: 16 * root.uiScale; height: width; radius: width / 2
                color: "#ffffff"
                Behavior on x { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
            }
        }
    }

    component StyleCheck: RowLayout {
        id: sck
        property string label: ""
        property string key: ""
        property bool value: false
        property string hint: ""
        width: root.width
        spacing: 6
        Label { text: sck.label; Layout.preferredWidth: 150; color: root.textColor; elide: Text.ElideRight; font.pixelSize: 13 }
        MapCheckBox {
            id: cb
            objectName: (sck.objectName || "") + "Check"
            checked: sck.value
            onToggled: {
                if (sck.key === "paceAuto" || sck.key === "tailPaceAuto") root.setStyleCheck(sck.key, checked)
                else root.setStyle(sck.key, checked, false)
            }
        }
        Item { Layout.fillWidth: true; Layout.fillHeight: true }
        HoverHandler { id: sckHover }
        ToolTip.visible: sckHover.hovered && sck.hint.length > 0
        ToolTip.text: sck.hint
        ToolTip.delay: 400
        Connections {
            target: root
            function onRevisionChanged() { if (cb.checked !== sck.value) cb.checked = sck.value }
            function onSelectedChanged() { if (cb.checked !== sck.value) cb.checked = sck.value }
        }
    }
}
