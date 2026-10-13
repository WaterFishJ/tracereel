import QtQuick 2.15
import QtQuick.Layouts 1.15
import QtQuick.Controls 2.15
import QtQuick.Dialogs
import TraceReel 1.0

ApplicationWindow {
    id: win
    visible: true
    width: 1440
    height: 900
    title: "TraceReel · 迹映"
    font.pixelSize: 13
    font.family: Theme.font_family
    color: Theme.background
    ReceiptImport {
        id: receiptImport
        parent: Overlay.overlay
        onImported: function(times, codes, relative) {
            settings.split_is_relative = relative
            settings.split_times = times
            settings.split_point_codes = codes
            win.applyNow("split_times")
            splitRows.syncFromSettings()
            win.appendLog("INFO", "从成绩条导入 " + times.split("|").length + " 个分段（" + (relative ? "分段用时" : "累计时间") + "）")
        }
    }
    // Qt Quick Controls 的取色来自窗口调色板：这一处套用原版令牌后，Fusion 控件的默认外观
    // （文本框/下拉框/菜单/弹层/滚动条/工具提示/进度条/数字框…）就都跟着主题走了。
    palette.window: Theme.background
    palette.windowText: Theme.text
    palette.base: Theme.background2
    palette.alternateBase: Theme.background
    palette.text: Theme.text
    palette.button: Theme.button
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.text_on_accent
    palette.mid: Theme.popup_border
    palette.dark: Theme.hr

    // ---------- 复用控件：外观全部对齐原版 Gyroflow（Theme 令牌 + 原版尺寸/动画） ----------
    // 分区：原版设置面板是「加粗标题 + 1px 分隔线」，没有灰底标题条。
    component GroupBox: Column {
        property string title
        width: parent ? (parent.width - 2 * (parent.padding ? parent.padding : 0)) : 0
        spacing: 6
        Label {
            width: parent.width
            text: title
            font.bold: true
            font.pixelSize: 14
            color: Theme.text
        }
        Rectangle { width: parent.width; height: 1; color: Theme.hr }
    }
    // 按钮（对齐原版 components/Button.qml：圆角 6、浅色描边、hover 提亮、按下 0.97 缩放动画）
    component TButton: Button {
        id: tb
        property bool small: false
        implicitHeight: small ? 24 * Theme.dpi_scale : 32 * Theme.dpi_scale
        leftPadding: (small ? 8 : 14) * Theme.dpi_scale
        rightPadding: (small ? 8 : 14) * Theme.dpi_scale
        font.pixelSize: small ? 12 : 13
        background: Rectangle {
            color: (tb.flat && !tb.hovered && !tb.down) ? "transparent"
                 : ((tb.hovered || tb.down || tb.activeFocus) ? Qt.lighter(Theme.button, 1.2) : Theme.button)
            radius: 6 * Theme.dpi_scale
            border.width: Theme.dark ? 0 : 1
            border.color: Theme.popup_border
            Behavior on color { ColorAnimation { duration: 100; easing.type: Easing.OutExpo } }
        }
        contentItem: Text {
            text: tb.text
            font: tb.font
            color: Theme.text
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            opacity: tb.enabled ? 1.0 : 0.6
        }
        scale: tb.down ? 0.97 : 1.0
        Behavior on scale { NumberAnimation { duration: 100; easing.type: Easing.OutExpo } }
    }
    // 滑杆（对齐原版 components/Slider.qml：4·dpi 圆角轨道 + 强调色已选段 + 圆形手柄，
    // 手柄内圈 hover/按下时 0.9/1.1 缩放，200ms OutExpo）
    component TSlider: Slider {
        id: ts
        implicitHeight: 22 * Theme.dpi_scale
        opacity: enabled ? 1.0 : 0.5
        background: Rectangle {
            x: ts.leftPadding
            y: ts.topPadding + ts.availableHeight / 2 - height / 2
            width: ts.availableWidth
            height: 4 * Theme.dpi_scale
            radius: height
            color: Theme.slider_background
            Rectangle {
                width: ts.visualPosition * parent.width
                height: parent.height
                radius: parent.radius
                color: Theme.accent
            }
        }
        handle: Rectangle {
            x: ts.leftPadding + ts.visualPosition * ts.availableWidth - width / 2
            y: ts.topPadding + ts.availableHeight / 2 - height / 2
            width: height
            height: Math.max(12, ts.height * 0.85)
            radius: width
            color: Theme.slider_handle
            Rectangle {
                anchors.centerIn: parent
                width: parent.width * 0.7
                height: width
                radius: width
                color: Theme.accent
                scale: ts.pressed ? 1.1 : (ts.hovered ? 0.9 : 1.0)
                Behavior on scale { NumberAnimation { duration: 200; easing.type: Easing.OutExpo } }
            }
        }
    }
    // 文件路径输入框：末尾内置一个 "…" 按钮，点它弹文件选择框（不再单独占一列放「浏览」按钮）。
    // settingProp 非空 → 选中的路径写回 settings[settingProp] 并触发应用；
    // 为空 → 发出 picked(value)，由父级（列表行）自己写回。
    component FileInput: TextField {
        id: fileInput
        property string settingProp: ""
        property string filter: "*.*"
        property bool saveMode: false
        property bool browse: true
        signal picked(string value)
        Layout.preferredHeight: 28
        font.pixelSize: 13
        rightPadding: browse ? 26 : 8
        TButton {
            visible: fileInput.browse
            anchors.right: parent.right
            anchors.rightMargin: 2
            anchors.verticalCenter: parent.verticalCenter
            width: 22
            height: Math.max(18, fileInput.height - 8)
            text: "…"
            flat: true
            padding: 0
            font.pixelSize: 14
            ToolTip.visible: hovered
            ToolTip.text: "选择文件"
            ToolTip.delay: 300
            onClicked: win.browseFor(fileInput)
        }
    }
    component Field: RowLayout {
        id: field
        property string label
        property string prop
        property string filter: "*.*"
        property bool browse: true
        property bool saveMode: false
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        FileInput {
            Layout.fillWidth: true
            settingProp: field.prop
            filter: field.filter
            browse: field.browse
            saveMode: field.saveMode
            text: { win.refreshTick; return settings[field.prop] || "" }
            onEditingFinished: {
                settings[field.prop] = text
                // 预设文件：顺手把它的 stabilization 段填进「稳定化」页。
                if (field.prop === "gyro_preset") win.loadPresetFields(text)
                win.applyChange(field.prop, field.label + " = " + (text.length ? text : "(空)"))
            }
        }
    }
    // DSpin / ISpin 的 hint：鼠标悬停在输入框上直接弹出提示（不再用「?」图标占一列）。
    component DSpin: RowLayout {
        property string label
        property string prop
        property string hint: ""
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        TextField {
            Layout.fillWidth: true; Layout.preferredHeight: 28
            text: { win.refreshTick; return settings[prop] === undefined ? "" : settings[prop].toFixed(2) }
            onEditingFinished: {
                settings[prop] = parseFloat(text) || 0
                win.applyChange(prop, label + " = " + settings[prop])
            }
            font.pixelSize: 13
            HoverHandler { id: dspinHover }
            ToolTip.visible: dspinHover.hovered && hint.length > 0
            ToolTip.text: hint
            ToolTip.delay: 300
        }
    }
    // Integer spinner (segment indices etc.): underlying value is a whole number.
    component ISpin: RowLayout {
        property string label
        property string prop
        property string hint: ""
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        TextField {
            Layout.fillWidth: true; Layout.preferredHeight: 28
            validator: IntValidator { bottom: 0 }
            text: { win.refreshTick; var v = settings[prop]; return (v === undefined ? 0 : Math.round(v)).toString() }
            // Commit live while focused: clicking the 导出 button may not blur the field,
            // so relying on onEditingFinished alone leaves the last-typed value uncommitted.
            onTextChanged: if (activeFocus) { var n = parseInt(text); if (!isNaN(n) && n !== settings[prop]) { settings[prop] = n; win.applyChange(prop) } }
            onEditingFinished: {
                settings[prop] = parseInt(text) || 0
                win.applyChange(prop, label + " = " + settings[prop])
            }
            font.pixelSize: 13
            HoverHandler { id: ispinHover }
            ToolTip.visible: ispinHover.hovered && hint.length > 0
            ToolTip.text: hint
            ToolTip.delay: 300
        }
    }
    // A field whose underlying value is a 0..1 fraction but is edited as 0-100% (画面占比).
    component PercentSpin: RowLayout {
        property string label
        property string prop
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        TextField {
            Layout.fillWidth: true; Layout.preferredHeight: 28
            text: { win.refreshTick; var v = settings[prop]; return v === undefined ? "" : (v * 100).toFixed(1) }
            onEditingFinished: {
                var v = (parseFloat(text) || 0)
                if (v < 0) v = 0
                if (v > 100) v = 100
                settings[prop] = v / 100.0
                win.applyChange(prop, label + " = " + v + "%")
            }
            font.pixelSize: 13
        }
    }
    component Spin: RowLayout {
        property string label
        property string prop
        property int min: 0
        property int max: 99999
        property int step: 1
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        SpinBox {
            from: min; to: max; stepSize: step
            value: { win.refreshTick; return settings[prop] }
            onValueModified: { settings[prop] = value; win.applyChange(prop, label + " = " + value) }
            Layout.fillWidth: true; Layout.preferredHeight: 28; font.pixelSize: 13
        }
    }
    // 勾选框行：标签在左（150px），方块对齐输入框的左边缘（156）。
    // 末尾的 Item 吸收 RowLayout 的剩余宽度 —— 没有可拉伸项时 RowLayout 会把控件推向右边，
    // 导致勾选框/色块「不左不右」；加上这个填充项后控件就固定贴着标签列右端。
    component Check: RowLayout {
        id: chk
        property string label
        property string prop
        property string hint: ""
        /// 勾选 / 取消时写回的值：默认 true / false（开关型设置）；给 0/1 这类整数型设置
        /// （例如「显示控制点」这类真的只有开/关两态的量）传 `valueOn: 1; valueOff: 0`，
        /// 一个勾选框就能直接控制它，不必再多一个只有单项可选的模式下拉。
        property var valueOn: true
        property var valueOff: false
        /// 取消勾选时的额外处理（例如关掉「配速色带自适应」时，要把手动滑杆对齐到当前
        /// 实际生效的色带）。用信号而不是 `onXxx` 属性，免得和 QML 的处理器命名规则打架。
        signal uncheckedAction()
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        CheckBox {
            id: cb
            checked: { win.refreshTick; return settings[prop] === chk.valueOn }
            onToggled: {
                settings[prop] = checked ? chk.valueOn : chk.valueOff
                if (!checked) chk.uncheckedAction()
                win.refreshFields()
                win.applyChange(prop, label + " = " + (checked ? "勾选" : "取消"))
            }
            font.pixelSize: 13
            HoverHandler { id: cbHover }
            ToolTip.visible: cbHover.hovered && parent.hint.length > 0
            ToolTip.text: parent.hint
            ToolTip.delay: 400
            indicator: Rectangle {
                implicitWidth: 20 * Theme.dpi_scale; implicitHeight: 20 * Theme.dpi_scale
                x: 0; y: (parent.height - height) / 2
                radius: 5 * Theme.dpi_scale
                color: cb.checked ? Theme.accent : "transparent"
                border.color: cb.checked ? Theme.accent : "#999999"
                border.width: 1
                opacity: cb.down || cb.activeFocus ? 0.8 : 1.0
                Behavior on color { ColorAnimation { duration: 300; easing.type: Easing.OutExpo } }
                Behavior on opacity { NumberAnimation { duration: 100; easing.type: Easing.OutExpo } }
                Text {
                    anchors.centerIn: parent
                    text: cb.checked ? "\u2713" : ""
                    color: Theme.text_on_accent
                    font.pixelSize: 13
                    font.bold: true
                    scale: cb.checked ? 1.0 : 0.0
                    Behavior on scale { NumberAnimation { duration: 200; easing.type: Easing.InOutCubic } }
                }
            }
        }
        Item { Layout.fillWidth: true; Layout.fillHeight: true }
    }
    // Edit split times as rows, always keeping one empty row at the end.
    component SplitList: Column {
        id: splitList
        width: parent.width
        spacing: 4
        // The string the current rows encode (only non-empty values).
        function encoded() {
            var parts = []
            for (var i = 0; i < splitModel.count; ++i) {
                var s = splitModel.get(i).str.trim()
                if (s.length) parts.push(s)
            }
            return parts.join("|")
        }
        function encodedCodes() {
            var parts = []
            for (var i = 0; i < splitModel.count; ++i) {
                if (splitModel.get(i).str.trim().length) parts.push(splitModel.get(i).code.trim())
            }
            return parts.some(function(code) { return code.length > 0 }) ? parts.join("|") : ""
        }
        // Rebuild the rows from settings.split_times.
        function syncFromSettings() {
            splitModel.clear()
            var raw = (settings.split_times || "").trim()
            var codes = (settings.split_point_codes || "").split("|")
            if (raw.length) {
                var parts = raw.split("|")
                for (var i = 0; i < parts.length; ++i) splitModel.append({ str: parts[i].trim(), code: codes[i] || "" })
            }
            ensureEmptyTail()
        }
        // Only collapse trailing blanks: keep the edited row and any later filled rows.
        function ensureEmptyTail() {
            while (splitModel.count > 1
                   && !splitModel.get(splitModel.count - 1).str.trim().length
                   && !splitModel.get(splitModel.count - 1).code.trim().length
                   && !splitModel.get(splitModel.count - 2).str.trim().length
                   && !splitModel.get(splitModel.count - 2).code.trim().length) {
                splitModel.remove(splitModel.count - 1)
            }
            if (!splitModel.count || splitModel.get(splitModel.count - 1).str.trim().length || splitModel.get(splitModel.count - 1).code.trim().length) {
                splitModel.append({ str: "", code: "" })
            }
        }
        function focusRow(i) {
            var row = splitRepeater.itemAt(i)
            if (row) row.focusTime()
        }
        function focusNextRow(i) {
            ensureEmptyTail()
            focusRow(i + 1)
        }
        // Persist the current rows back to settings.split_times.
        function writeBack() {
            settings["split_times"] = encoded()
            settings["split_point_codes"] = encodedCodes()
            win.applyChange("split_times")
        }
        // 立即应用（增删行用）：防抖窗口里预览还停在旧数据上，容易被当成"没生效"。
        function writeBackNow() {
            settings["split_times"] = encoded()
            settings["split_point_codes"] = encodedCodes()
            win.applyNow("split_times")
        }
        function rowCount() { return splitModel.count }
        // 删除一行：✕ 按钮与自动排查都走这里，只有一处逻辑。
        function removeRow(i) {
            if (i < 0 || i >= splitModel.count) return
            splitModel.remove(i)
            ensureEmptyTail()
            writeBack()
            win.appendLog("INFO", "删除分段行 " + (i + 1) + " · " + win.listCounts())
        }
        function appendRow() {
            ensureEmptyTail()
            focusRow(splitModel.count - 1)
        }
        function setRowValue(i, s) {
            if (i < 0 || i >= splitModel.count) return
            splitModel.setProperty(i, "str", s)
            ensureEmptyTail()
            writeBack()
        }
        function setPointCode(i, value) {
            if (i < 0 || i >= splitModel.count) return
            splitModel.setProperty(i, "code", value)
            ensureEmptyTail()
            writeBack()
        }
        ListModel { id: splitModel }
        // The proxy may load settings asynchronously after this component is created, so
        // sync on completion and re-sync a couple of delayed ticks to catch that load.
        Component.onCompleted: {
            syncFromSettings()
            syncTimer1.restart()
            syncTimer2.restart()
        }
        Timer { id: syncTimer1; interval: 300; running: false; repeat: false; onTriggered: splitList.syncFromSettings() }
        Timer { id: syncTimer2; interval: 900; running: false; repeat: false; onTriggered: splitList.syncFromSettings() }
        // Re-sync only when an external change (project load / reset) alters the string;
        // skip when the change was produced by our own writeBack so the model isn't reset.
        Connections {
            target: win
            function onRefreshTickChanged() {
                if ((settings.split_times || "").trim() !== splitList.encoded()
                    || (settings.split_point_codes || "") !== splitList.encodedCodes()) splitList.syncFromSettings()
            }
        }
        Repeater {
            id: splitRepeater
            model: splitModel
            delegate: RowLayout {
                function focusTime() { timeInput.forceActiveFocus(); timeInput.selectAll() }
                width: splitList.width
                spacing: 6
                Label { text: "分段 " + (index + 1); Layout.preferredWidth: 65; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
                TextField {
                    objectName: "splitPointCodeInput"
                    Layout.preferredWidth: 85; Layout.preferredHeight: 28
                    text: code
                    placeholderText: "点号（可选）"
                    validator: RegularExpressionValidator { regularExpression: /[^|;\r\n]*/ }
                    onTextEdited: splitList.setPointCode(index, text)
                    onAccepted: timeInput.forceActiveFocus()
                    font.pixelSize: 13
                }
                TextField {
                    id: timeInput
                    objectName: "splitTimeInput"
                    Layout.fillWidth: true; Layout.preferredWidth: 160; Layout.minimumWidth: 90; Layout.preferredHeight: 28
                    text: str
                    placeholderText: "mm:ss / h:mm:ss / 秒"
                    HoverHandler { id: splitTimeHover }
                    ToolTip.visible: splitTimeHover.hovered
                    ToolTip.delay: 400
                    ToolTip.text: "时间格式：分:秒（1:30）、时:分:秒（1:02:30）或纯秒数（90）。\n也支持 1.30 表示 1 分 30 秒；小数秒请使用冒号格式，如 1:30.5。\n绝对时间填从出发起的累计用时，相对时间填本段用时。"
                    onTextEdited: splitList.setRowValue(index, text)
                    onAccepted: splitList.focusNextRow(index)
                    font.pixelSize: 13
                }
                TButton {
                    text: "跳转"; small: true
                    Layout.preferredHeight: 28; Layout.preferredWidth: 52; font.pixelSize: 12
                    ToolTip.visible: hovered
                    ToolTip.text: "把视频定位到该分段的起点（按视频时间轴计算，与导出的边界一致）"
                    ToolTip.delay: 400
                    onClicked: win.jumpToSplit(index)
                }
                TButton {
                    text: "\u2715"; small: true
                    Layout.preferredHeight: 28; Layout.preferredWidth: 30; font.pixelSize: 13
                    onClicked: splitList.removeRow(index)
                }
            }
        }
    }
    // 同一场定向的多段视频（运动相机自动分段）：行 0 = 主文件（video_path），其余行 =
    // 续接分段（video_extras，'|' 拼接）。每行有路径输入框 + 浏览 + 删除；「添加视频」
    // 追加空行。预览时间轴 = 各段按录制顺序首尾相接。
    component VideoList: Column {
        id: videoList
        width: parent.width
        spacing: 4
        property string fileFilter: "*.mp4 *.mov *.mkv *.avi"
        function firstPath() {
            return videoModel.count ? videoModel.get(0).path.trim() : ""
        }
        // 续接分段（不含行 0）编码为 '|' 拼接串。
        function encodedExtras() {
            var parts = []
            for (var i = 1; i < videoModel.count; ++i) {
                var s = videoModel.get(i).path.trim()
                if (s.length) parts.push(s)
            }
            return parts.join("|")
        }
        // 从 settings.video_path / settings.video_extras 重建行。
        function syncFromSettings() {
            videoModel.clear()
            videoModel.append({ path: settings.video_path || "" })
            var raw = (settings.video_extras || "").trim()
            if (raw.length) {
                var parts = raw.split("|")
                for (var i = 0; i < parts.length; ++i) videoModel.append({ path: parts[i].trim() })
            }
        }
        // 把当前行写回 settings 并触发整链重载（video_path 属于需重载 gyro 的属性）。
        function writeBack() {
            settings.video_path = firstPath()
            settings.video_extras = encodedExtras()
            win.applyChange("video_path")
        }
        // 立即应用（增删行 / 选择文件用）：防抖窗口里预览还停在旧视频上。
        function writeBackNow() {
            settings.video_path = firstPath()
            settings.video_extras = encodedExtras()
            win.applyNow("video_path")
        }
        function rowCount() { return videoModel.count }
        // 拖入的文件**进模型**（而不是去拼 settings 字符串）：列表行是唯一事实来源，
        // 于是"删掉的行"不可能被下一次拖入又拼回来。
        // 最后一个非空行之后都是"待填"空行（点过「添加」还没选文件）：填第一个、
        // 多余的删掉，否则追加。不这样做的话，空行 + 一次拖入就会多出一行，
        // 看起来就是"文件越拖越多"。
        function addDroppedPath(p) {
            if (videoModel.count === 0) { videoModel.append({ path: p }); writeBack(); return }
            var lastFilled = -1
            for (var i = videoModel.count - 1; i >= 0; --i) {
                if (String(videoModel.get(i).path).trim().length) { lastFilled = i; break }
            }
            var hole = lastFilled + 1
            if (hole < videoModel.count) {
                videoModel.setProperty(hole, "path", p)
                var extra = videoModel.count - 1 - hole
                if (extra > 0) videoModel.remove(hole + 1, extra)
            } else {
                videoModel.append({ path: p })
            }
            writeBack()
        }
        function removeRow(i) {
            if (i < 0 || i >= videoModel.count) return
            videoModel.remove(i)
            writeBackNow()
            win.appendLog("INFO", "删除视频行 " + (i + 1) + " · " + win.listCounts())
        }
        function appendRow() {
            videoModel.append({ path: "" })
            win.appendLog("INFO", "添加视频行 · " + win.listCounts())
        }
        // 等价于「浏览…」选中：与 FileInput.onPicked 同一条路径。
        function setRowPath(i, p) {
            if (i < 0 || i >= videoModel.count) return
            videoModel.setProperty(i, "path", p)
            writeBackNow()
        }
        ListModel { id: videoModel }
        // 同 SplitList：代理可能异步载入设置，完成后同步并补两次延迟同步。
        Component.onCompleted: {
            syncFromSettings()
            vsyncTimer1.restart()
            vsyncTimer2.restart()
        }
        Timer { id: vsyncTimer1; interval: 300; running: false; repeat: false; onTriggered: videoList.syncFromSettings() }
        Timer { id: vsyncTimer2; interval: 900; running: false; repeat: false; onTriggered: videoList.syncFromSettings() }
        // 仅在外部变更（加载项目/重置）与当前行不一致时重建，避免打断正在编辑的行。
        Connections {
            target: win
            function onRefreshTickChanged() {
                if ((settings.video_path || "") !== videoList.firstPath()
                    || (settings.video_extras || "").trim() !== videoList.encodedExtras())
                    videoList.syncFromSettings()
            }
        }
        Repeater {
            model: videoModel
            delegate: RowLayout {
                width: videoList.width
                spacing: 6
                Label { text: index === 0 ? "主文件" : "分段 " + (index + 1); Layout.preferredWidth: 60; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
                FileInput {
                    Layout.fillWidth: true
                    filter: videoList.fileFilter
                    text: path
                    onEditingFinished: { videoModel.setProperty(index, "path", text); videoList.writeBackNow() }
                    onPicked: function (value) { videoModel.setProperty(index, "path", value); videoList.writeBackNow() }
                }
                TButton {
                    text: "\u2715"; Layout.preferredHeight: 28; Layout.preferredWidth: 30; font.pixelSize: 13
                    onClicked: videoList.removeRow(index)
                }
            }
        }
        TButton {
            text: "添加视频"; Layout.preferredHeight: 28; Layout.preferredWidth: 120; font.pixelSize: 13
            onClicked: videoList.appendRow()
        }
    }
    // 轨迹图片列表：QuickRoute JPEG = 地图 + 轨迹合一，每个文件自带轨迹数据。
    //   .qrt 工程文件同样可以放在这里：地图（内嵌）+ 轨迹一次性拿到，
    //   省掉「先在 QuickRoute 里导出 gpx.jpg」这一步。
    //   行 0 = 主图（route_path + map_path，从出发时刻开始显示，没有换图时间）
    //   行 1+ = 额外地图（map_extras / map_change_times，到换图时间切换底图）
    // 初始没有任何行、只有「添加轨迹图片」按钮——完全不用地图（只做防抖/HUD）也是合法配置。
    // 换图时间格式同「分段时间」：h:mm:ss / mm:ss / 纯秒数，从出发时刻算起。
    component TrackList: Column {
        id: trackList
        width: parent.width
        spacing: 4
        property string fileFilter: "*.jpg *.jpeg *.qrt"
        // .qrt 识别到分段时，暂存待确认的目标行与路径。
        property int pendingQrtIndex: -1
        property string pendingQrtPath: ""
        // 「按分段设置换图时间」暂存目标行号。
        property int pendingMapRow: -1
        property string timeTip: "从出发时刻算起的换图时间；格式 h:mm:ss、mm:ss 或纯秒数（同「分段时间」）"
        // 没有任何轨迹图片时「画面占比」无意义（渲染层会让视频铺满整帧）。
        property bool hasMap: trackModel.count > 0
        // 行 0 = 主图路径。
        function mainPath() {
            return trackModel.count ? String(trackModel.get(0).path).trim() : ""
        }
        // 行 1+ 原样编码（空行保留占位，保证路径与换图时间按索引对齐）。
        function encodedExtras() {
            var parts = []
            for (var i = 1; i < trackModel.count; ++i) parts.push(String(trackModel.get(i).path).trim())
            return parts.join("|")
        }
        function encodedTimes() {
            var parts = []
            for (var i = 1; i < trackModel.count; ++i) parts.push(String(trackModel.get(i).time).trim())
            return parts.join("|")
        }
        // 从 settings 重建行：主图 = route_path（空则 map_path），其余 = map_extras。
        function syncFromSettings() {
            trackModel.clear()
            var main = String(settings.route_path || "").trim()
            if (!main.length) main = String(settings.map_path || "").trim()
            var raw = (settings.map_extras || "").trim()
            var paths = raw.length ? raw.split("|") : []
            var timesRaw = (settings.map_change_times || "").trim()
            var times = timesRaw.length ? timesRaw.split("|") : []
            if (main.length || paths.length) trackModel.append({ path: main, time: "" })
            for (var i = 0; i < paths.length; ++i) trackModel.append({ path: paths[i].trim(), time: i < times.length ? times[i].trim() : "" })
        }
        // 写回 settings 并触发整链重载。主图同时写 route_path 与 map_path：同一个
        // QuickRoute JPEG 既是底图又是轨迹来源。
        function writeBack() {
            var main = mainPath()
            settings.route_path = main
            settings.map_path = main
            settings.map_extras = encodedExtras()
            settings.map_change_times = encodedTimes()
            win.applyChange("map_path")
        }
        // 增删行 / 按分段选时刻：立即应用，别让预览停在旧地图上。
        function writeBackNow() {
            var main = mainPath()
            settings.route_path = main
            settings.map_path = main
            settings.map_extras = encodedExtras()
            settings.map_change_times = encodedTimes()
            win.applyNow("map_path")
        }
        // 把某一行的换图时间设成 `t`（`h:mm:ss` / `m:ss`，与「分段时间」同格式）。
        function setRowTime(i, t) {
            if (i < 0 || i >= trackModel.count) return
            trackModel.setProperty(i, "time", t)
            writeBackNow()
            // `win.` 限定：`component TrackList` 内部看不到文档级的裸函数名。
            win.appendLog("INFO", "第 " + (i + 1) + " 张地图的换图时间已设为 " + t)
        }
        function rowCount() { return trackModel.count }
        // 拖入的文件进模型（同 VideoList）：列表行是唯一事实来源，
        // "删掉的行"不会被下一次拖入又拼回来；结尾的待填空行先填掉、多余的删掉。
        function addDroppedPath(p) {
            if (trackModel.count === 0) { trackModel.append({ path: p, time: "" }); writeBack(); return }
            var lastFilled = -1
            for (var i = trackModel.count - 1; i >= 0; --i) {
                if (String(trackModel.get(i).path).trim().length) { lastFilled = i; break }
            }
            var hole = lastFilled + 1
            if (hole < trackModel.count) {
                trackModel.setProperty(hole, "path", p)
                var extra = trackModel.count - 1 - hole
                if (extra > 0) trackModel.remove(hole + 1, extra)
            } else {
                trackModel.append({ path: p, time: "" })
            }
            writeBack()
        }
        function removeRow(i) {
            if (i < 0 || i >= trackModel.count) return
            var p = String(trackModel.get(i).path)
            trackModel.remove(i)
            writeBackNow()
            win.appendLog("INFO", "删除轨迹行 " + (i + 1) + ": " + p + " · " + win.listCounts())
        }
        function appendRow() {
            trackModel.append({ path: "", time: "" })
            win.appendLog("INFO", "添加轨迹图片行 · " + win.listCounts())
        }
        // 等价于「浏览…」选中：与 FileInput.onPicked 同一条路径。
        function setRowPath(i, p) {
            if (i < 0 || i >= trackModel.count) return
            trackModel.setProperty(i, "path", p)
            writeBackNow()
        }
        ListModel { id: trackModel }
        // 同 VideoList：代理可能异步载入设置，完成后同步并补两次延迟同步。
        Component.onCompleted: {
            syncFromSettings()
            tracksyncTimer1.restart()
            tracksyncTimer2.restart()
        }
        Timer { id: tracksyncTimer1; interval: 300; running: false; repeat: false; onTriggered: trackList.syncFromSettings() }
        Timer { id: tracksyncTimer2; interval: 900; running: false; repeat: false; onTriggered: trackList.syncFromSettings() }
        // 仅在外部变更（加载项目/重置）与当前行不一致时重建，避免打断正在编辑的行。
        Connections {
            target: win
            function onRefreshTickChanged() {
                var effMain = String(settings.route_path || "").trim()
                if (!effMain.length) effMain = String(settings.map_path || "").trim()
                if (effMain !== trackList.mainPath()
                    || (settings.map_extras || "").trim() !== trackList.encodedExtras()
                    || (settings.map_change_times || "").trim() !== trackList.encodedTimes())
                    trackList.syncFromSettings()
            }
        }
        Repeater {
            model: trackModel
            delegate: RowLayout {
                width: trackList.width
                spacing: 6
                Label { text: "文件"; Layout.preferredWidth: 34; color: Theme.text; font.pixelSize: 13 }
                FileInput {
                    Layout.fillWidth: true
                    filter: trackList.fileFilter
                    text: path
                    onEditingFinished: { trackModel.setProperty(index, "path", text); trackList.writeBack() }
                    onPicked: function (value) {
                        trackModel.setProperty(index, "path", value)
                        // .qrt 工程文件带分段（Handle）数据时，先问是否导入，再落地路径。
                        if (item.qrt_has_splits(value)) {
                            win.appendLog("INFO", "检测到 .qrt 分段，等待确认: " + value)
                            trackList.pendingQrtIndex = index
                            trackList.pendingQrtPath = value
                            qrtSplitDialog.open()
                            return
                        }
                        trackList.writeBack()
                    }
                }
                Label { text: "换图时间"; Layout.preferredWidth: 60; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
                Item {
                    Layout.preferredWidth: 88; Layout.preferredHeight: 28
                    TextField {
                        anchors.fill: parent
                        // 行 0 是主图，从出发时刻开始显示，换图时间恒为 0。
                        enabled: index > 0
                        text: index > 0 ? time : ""
                        // 空 = 未设置：这张图暂不参与播放（不会退化成 0 秒抢整条时间轴）。
                        placeholderText: index > 0 ? "未设置" : "0:00:00"
                        onEditingFinished: { trackModel.setProperty(index, "time", text); trackList.writeBackNow() }
                        font.pixelSize: 13
                    }
                    HoverHandler { id: timeHover }
                    ToolTip.visible: timeHover.hovered
                    ToolTip.text: index > 0
                        ? trackList.timeTip + "\n留空 = 这张图暂不参与播放；也可以点右边「按段」用分段起点。"
                        : "主图从出发时刻开始显示，不需要换图时间"
                    ToolTip.delay: 300
                }
                TButton {
                    // 按分段设置：第 N 段的起点时刻直接填进来。
                    visible: index > 0
                    text: "按段"
                    small: true
                    Layout.preferredHeight: 28; Layout.preferredWidth: 44; font.pixelSize: 12
                    enabled: { win.refreshTick; return settings.split_starts().length > 0 }
                    onClicked: { trackList.pendingMapRow = index; mapSplitDialog.open() }
                }
                TButton {
                    text: "\u2715"; Layout.preferredHeight: 28; Layout.preferredWidth: 30; font.pixelSize: 13
                    onClicked: trackList.removeRow(index)
                }
            }
        }
        TButton {
            text: "添加轨迹图片"; Layout.preferredHeight: 28; Layout.preferredWidth: 120; font.pixelSize: 13
            onClicked: trackList.appendRow()
        }
    }
    component Combo: RowLayout {
        id: combo
        property string label
        property string prop
        property var items
        property string hint: ""
        /// 可选的「项 → 存储值」映射。不填时下标就是存储值；填了就用它换算，
        /// 从而能表达「界面只给三项、但存储值沿用原版编号（例如 0/1/3）」这种菜单。
        property var values: null
        width: parent.width
        spacing: 6
        /// 选中后额外通知（例如「缩放方式」变化时要联动补默认的缩放速度）。
        signal picked(int index)
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        ComboBox {
            model: items
            currentIndex: {
                win.refreshTick
                if (combo.values === null) return settings[prop]
                var i = combo.values.indexOf(Number(settings[prop]))
                return i < 0 ? 0 : i
            }
            onActivated: {
                settings[prop] = combo.values === null ? currentIndex : combo.values[currentIndex]
                win.applyChange(prop, label + " = " + items[currentIndex]); win.refreshFields(); combo.picked(currentIndex)
            }
            Layout.fillWidth: true; Layout.preferredHeight: 28; font.pixelSize: 13
            HoverHandler { id: comboHover }
            ToolTip.visible: comboHover.hovered && combo.hint.length > 0
            ToolTip.text: combo.hint
            ToolTip.delay: 400
        }
    }
    component ComboV: RowLayout {
        id: comboV
        property string label
        property string prop
        property var values
        property var labels
        /// 选中后额外通知（例如「分段导出」要联动底层开关、分辨率要刷新预览宽高比）。
        signal picked(int index)
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        ComboBox {
            Layout.fillWidth: true; Layout.preferredHeight: 28
            model: labels
            currentIndex: { win.refreshTick; var v = String(settings[prop]); var i = values.indexOf(v); return i < 0 ? 0 : i }
            onActivated: { settings[prop] = values[currentIndex]; win.applyChange(prop, label + " = " + labels[currentIndex]); comboV.picked(currentIndex) }
            font.pixelSize: 13
        }
    }
    // 滑杆 + 数值框（对应原版 Gyroflow 的 SliderWithField）。
    // 拖动松手（live:false）即写值并应用；`scale` 用于「显示单位 ≠ 存储单位」
    // （例如存储 1.0 显示 100%），`unit` 只做展示。
    // `alphaPercent`：透明度专用。存储里透明度仍是原版的 0..255（QColor alpha），
    // 界面上按百分比显示与输入（0% = 完全透明，100% = 完全不透明）；不能用 `scale: 2.55`
    // 糊弄，因为那样滑杆的取值网格是 2.55 的倍数、数值框还会露出小数。
    component SliderField: RowLayout {
        id: sf
        property string label
        property string prop
        property real from: 0
        property real to: 100
        property int decimals: 2
        property real scale: 1
        property bool alphaPercent: false
        property string unit: ""
        property string hint: ""
        width: parent.width
        spacing: 6
        /// 初始化完成前不写回：Slider 建立时会把 value 夹到 [from,to]（比如 from=-1000），
        /// 那次 valueChanged 若写回就会把设置改成 from，必须在 onCompleted 之后才允许写。
        property bool ready: false
        function dv() {
            var v = Number(settings[sf.prop])
            if (!isFinite(v)) return 0
            return sf.alphaPercent ? v * 100 / 255 : v * sf.scale
        }
        function store(display) {
            if (!sf.ready) return
            // 透明度：百分比 → 原版 0..255（四舍五入，255/100 = 2.55 不是整数）。
            var v = sf.alphaPercent ? Math.round(display * 255 / 100) : display / sf.scale
            var cur = Number(settings[sf.prop])
            // 值没变就直接返回：绑定回写/重复输入不会触发应用，也避免递归。
            if (isFinite(cur) && Math.abs(cur - v) < 1e-9) return
            settings[sf.prop] = v
            // 不要在这里调 refreshFields()：那会让 `value` 绑定依赖的 refreshTick 反过来依赖
            // value 本身，Qt 会报 “Binding loop detected for property value”。
            // applyChange 自带 350ms 防抖，随后的 doApply 会 refreshFields，数值框那时刷新。
            win.applyChange(sf.prop, sf.label + " = " + display.toFixed(sf.decimals) + (sf.alphaPercent ? "%" : sf.unit))
        }
        Label { text: sf.label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        TSlider {
            id: sfSlider
            Layout.fillWidth: true
            from: sf.from; to: sf.to
            live: false
            // 刻意**不写** `value: 绑定`：绑定读 settings[prop]，而 onValueChanged 又写回它，
            // Qt 会报 “Binding loop detected for property value”。改成命令式同步（见 sliderSync）。
            onValueChanged: sf.store(value)
        }
        Connections {
            target: win
            // 外部变化（加载工程 / 导入预设 / 重置）时把滑杆位置同步过来。
            function onRefreshTickChanged() { sf.sliderSync() }
        }
        function sliderSync() {
            var v = Math.max(sf.from, Math.min(sf.to, sf.dv()))
            if (Math.abs(sfSlider.value - v) > 1e-9) sfSlider.value = v
        }
        Component.onCompleted: { sliderSync(); sf.ready = true }
        TextField {
            Layout.preferredWidth: 74; Layout.preferredHeight: 28; font.pixelSize: 12
            text: { win.refreshTick; return sf.dv().toFixed(sf.decimals) }
            onEditingFinished: sf.store(parseFloat(text) || 0)
        }
        Label { visible: sf.unit.length > 0; text: sf.unit; color: Theme.muted; font.pixelSize: 11; Layout.preferredWidth: 30 }
        HoverHandler { id: sfHover }
        ToolTip.visible: sfHover.hovered && sf.hint.length > 0
        ToolTip.text: sf.hint
        ToolTip.delay: 300
    }
    // 折叠区「高级选项」（对齐原版 components/AdvancedSection.qml）：
    // 居中的强调色按钮 + 高度/透明度 700ms OutExpo 动画；展开后关掉 height 动画避免常驻开销。
    component AdvancedSection: Column {
        id: adv
        property string label: "高级选项"
        property bool opened: false
        default property alias content: inner.data
        width: parent.width
        spacing: parent.spacing !== undefined ? parent.spacing : 6
        TButton {
            anchors.horizontalCenter: parent.horizontalCenter
            text: (adv.opened ? "▴  " : "▾  ") + adv.label
            flat: true
            font.pixelSize: 12
            font.bold: adv.opened
            Layout.preferredHeight: 26
            onClicked: adv.opened = !adv.opened
        }
        Column {
            id: inner
            width: parent.width
            spacing: parent.spacing
            visible: opacity > 0
            opacity: adv.opened ? 1 : 0
            height: adv.opened ? implicitHeight : 0
            clip: true
            Behavior on opacity { NumberAnimation { duration: 700; easing.type: Easing.OutExpo } }
            Behavior on height { NumberAnimation { duration: 700; easing.type: Easing.OutExpo } }
        }
    }
    // 1px 分隔线（原版 components/Hr.qml）
    component Hr: Rectangle {
        width: parent.width
        height: 1
        color: Theme.hr
        Layout.fillWidth: true
    }
    // 分区小标题（原版设置面板用「加粗文字 + 分隔线」，而不是灰底标题条）
    component SectionHeader: Column {
        property string text
        width: parent.width
        spacing: 4
        Label { text: parent.text; font.bold: true; font.pixelSize: 13; color: Theme.text }
        Hr { }
    }
    // 二级小标题：「显示」里的每一片区域（路线 / 尾巴 / 控制点 / 运动员）用它，刻意做得
    // 比 GroupBox 的标题低一级 —— 小一号字、弱化色、左侧一根竖条，而且**不画整条分隔线**，
    // 免得跟「显示」这样的分组标题在视觉上撞成同级。
    component SubSection: Column {
        id: sub
        property string title
        width: parent.width
        spacing: 6
        topPadding: 6
        Row {
            spacing: 6
            Rectangle {
                width: 3 * Theme.dpi_scale
                height: subLabel.height
                radius: 1.5 * Theme.dpi_scale
                color: Theme.accent
            }
            Label {
                id: subLabel
                text: sub.title
                font.pixelSize: 12
                color: Theme.muted
            }
        }
    }
    // 平滑参数一行（标签 + 滑杆 + 数值）：基础/高级两个列表共用，spRef 指向 SmoothParams。
    // 两个坑同 SliderField：①初始化时 Slider 会把 value 夹到 from（0.001…）→ 委托 ready 之前不写回；
    // ②不给 value 写绑定，改命令式同步。
    component SmoothRow: RowLayout {
        id: prow
        property var spRef
        property bool adv: false
        width: prow.spRef ? prow.spRef.width : 0
        spacing: 6
        property bool ready: false
        property bool syncing: false
        function sliderSync() {
            var v = Math.max(model.from, Math.min(model.to, Number(model.val)))
            if (Math.abs(paramSlider.value - v) > 1e-9) {
                syncing = true
                paramSlider.value = v
                syncing = false
            }
        }
        Component.onCompleted: { sliderSync(); ready = true }
        Label { text: model.label; Layout.preferredWidth: 150; color: Theme.text; font.pixelSize: 12; elide: Text.ElideRight }
        TSlider {
            id: paramSlider
            Layout.fillWidth: true
            from: model.from; to: model.to
            live: false
            onValueChanged: {
                if (!prow.ready || prow.syncing || prow.spRef.reloading) return
                if (Math.abs(Number(model.val) - value) < 1e-9) return
                prow.spRef.setRow(prow.adv, index, value)
            }
        }
        Label {
            text: Number(model.val).toFixed(model.decimals === undefined ? 3 : model.decimals)
            color: Theme.muted; font.pixelSize: 11; Layout.preferredWidth: 46
        }
        Label {
            visible: (model.unit || "").length > 0
            text: model.unit || ""
            color: Theme.muted; font.pixelSize: 11; Layout.preferredWidth: 30
        }
    }
    // ---------- 平滑算法参数（随「平滑方式」变化） ----------
    // 参数定义来自 gyroflow_core 的 `SmoothingAlgorithm::get_parameters_json()`
    // （name/description/from/to/value/default/advanced/unit/precision）。
    // 与原版一致：标了 `advanced` 的参数（如 max_smoothness、alpha_0_1s、per_axis、trim_range_only）
    // 收进「高级选项」；`Default` 方式的四个 smoothness 按百分比显示（×100、单位 %、精度 1）。
    component SmoothParams: Column {
        id: sp
        property int tick: 0
        property bool reloading: false
        /// 「高级选项」是否展开（由本组件的 AdvancedSection 驱动）。
        property alias advancedOpen: spAdv.opened
        width: parent.width
        spacing: 3
        ListModel { id: spModel }
        ListModel { id: spAdvModel }

        // 参数名 → 中文标签（Rust 只给英文 description）
        function paramLabel(name) {
            var m = {
                "smoothness": "平滑度", "smoothness_pitch": "俯仰平滑度",
                "smoothness_yaw": "偏航平滑度", "smoothness_roll": "横滚平滑度",
                "per_axis": "按轴", "trim_range_only": "仅限修剪范围内",
                "max_smoothness": "最大平滑度", "alpha_0_1s": "高速最大平滑度",
                "second_pass": "二阶平滑过程",
                "time_constant": "时间常数", "roll": "横滚角", "pitch": "俯仰角", "yaw": "偏航角"
            }
            return m[name] !== undefined ? m[name] : name
        }
        function methodLabel(name) {
            var m = { "No smoothing": "无平滑", "Default": "默认", "Plain 3D": "纯 3D", "Fixed camera": "固定摄像头" }
            return m[name] !== undefined ? m[name] : name
        }
        property var methodNames: { win.refreshTick; var l = []; try { l = JSON.parse(settings.smoothing_methods()) } catch (e) {} return l }
        property var methodLabels: { var out = []; for (var i = 0; i < methodNames.length; ++i) out.push(methodLabel(methodNames[i])); return out }
        function storedNames() {
            var out = {}
            try {
                var a = JSON.parse(String(settings.gyro_smoothing_params || "[]"))
                for (var i = 0; i < a.length; ++i) out[a[i].name] = true
            } catch (e) {}
            return out
        }
        function reload() {
            if (sp.reloading) return
            sp.reloading = true
            var list = []
            try { list = JSON.parse(settings.smoothing_params()) } catch (e) { list = [] }
            var stored = sp.storedNames()
            var needWrite = false
            spModel.clear()
            spAdvModel.clear()
            for (var i = 0; i < list.length; ++i) {
                var p = list[i]
                if (!stored[p.name]) needWrite = true
                // 原版：Default 方式的四个平滑度按百分比显示并乘 100 存回
                var isPercent = (String(settings.gyro_smoothing_method) === "Default")
                        && (p.name === "smoothness" || p.name === "smoothness_pitch"
                            || p.name === "smoothness_yaw" || p.name === "smoothness_roll")
                var row = {
                    name: p.name,
                    label: sp.paramLabel(p.name),
                    from: (p.from === undefined ? 0 : p.from) * (isPercent ? 100 : 1),
                    to: (p.to === undefined ? 1 : p.to) * (isPercent ? 100 : 1),
                    val: (p.value === undefined ? (p.default === undefined ? 0 : p.default) : p.value) * (isPercent ? 100 : 1),
                    unit: isPercent ? "%" : (p.unit || ""),
                    decimals: isPercent ? 1 : (p.precision === undefined ? 3 : Math.min(3, p.precision))
                }
                if (p.advanced) spAdvModel.append(row)
                else spModel.append(row)
            }
            sp.tick++
            // 换了平滑方式（存的参数名对不上）→ 用新方式的默认值覆盖，不留旧参数。
            if (needWrite && (spModel.count + spAdvModel.count) > 0) sp.writeRaw()
            sp.reloading = false
        }
        /// 滑杆回调写回（重建期间忽略）。adv 决定写哪个列表（都要写，顺序不影响核心）。
        function setRow(adv, idx, v) {
            if (sp.reloading) return
            if (adv) spAdvModel.setProperty(idx, "val", v)
            else spModel.setProperty(idx, "val", v)
            sp.writeBack()
        }
        function writeBack() {
            if (sp.reloading) return
            sp.writeRaw()
        }
        function writeRaw() {
            var list = []
            var i
            for (i = 0; i < spModel.count; ++i) list.push(sp.jsonOf(spModel.get(i)))
            for (i = 0; i < spAdvModel.count; ++i) list.push(sp.jsonOf(spAdvModel.get(i)))
            settings.gyro_smoothing_params = JSON.stringify(list)
            win.applyChange("gyro_smoothing_params")
        }
        /// 百分比显示的参数要除回原值再存（与核心/预设的 0..1 语义一致）。
        function jsonOf(r) {
            var v = r.val
            if (r.unit === "%") v = v / 100
            return { name: r.name, value: v }
        }
        Component.onCompleted: reload()
        Connections {
            target: win
            function onRefreshTickChanged() { sp.reload() }
        }
        ComboV {
            label: "平滑方式"; prop: "gyro_smoothing_method"
            values: sp.methodNames
            labels: sp.methodLabels
        }
        Repeater {
            model: spModel
            delegate: SmoothRow { spRef: sp; adv: false }
        }
        AdvancedSection {
            id: spAdv
            Repeater {
                model: spAdvModel
                delegate: SmoothRow { spRef: sp; adv: true }
            }
        }
    }
    // ---------- 镜头标定搜索框 ----------
    // 原版 Gyroflow 的镜头预设就是这个交互：一个「输入即搜」的框 + 结果弹窗
    // （SearchField + LensProfileSearchDelegate）—— 库里一万多条，全列出来不现实。
    // 这里只列**镜头标定**（Gyroflow 的 profiles.cbor.gz，实测 12409 条）；`.gyroflow` 预设
    // 改回上面那个「预设文件」输入框（手输/浏览都行，选完会把它的 stabilization 段填进「稳定化」页）。
    // 匹配规则与 Gyroflow 一致：空格分隔的每个词都要命中；结果最多 60 条。
    component ProfileSearch: Column {
        id: ps
        property string label
        property int tick: 0
        property bool dbReady: false
        property int dbTotal: 0
        property string dbPath: ""
        property string dbDir: ""
        width: parent.width
        spacing: 4

        ListModel { id: hits }

        function search(text) {
            var data = null
            try { data = JSON.parse(settings.search_profiles(text, 60)) } catch (e) { data = null }
            hits.clear()
            if (data) {
                ps.dbReady = !!data.ready
                ps.dbTotal = data.total || 0
                ps.dbPath = data.db || ""
                ps.dbDir = data.dir || ""
                var items = data.items || []
                for (var i = 0; i < items.length; ++i)
                    if (items[i].kind === "lens") hits.append(items[i])   // 这里只列镜头标定；预设用「预设文件」输入框
            }
            ps.tick++
            // 有结果就自动弹出（原版 Gyroflow 的 SearchField 也是这个行为：
            // 输入即搜、有命中就开列表；这里原来只在按方向键时才开，所以「输入没反应」）。
            if (box.text.length > 0) {
                if (hits.count > 0) { pop.open(); lv.currentIndex = 0 }
                else pop.close()
            }
        }
        function pick(index) {
            if (index < 0 || index >= hits.count) return
            var it = hits.get(index)
            if (it.kind === "preset") {
                settings.gyro_preset = it.id
                win.loadPresetFields(it.id)
                win.refreshFields()
                win.applyChange("gyro_preset")
                win.appendLog("INFO", "防抖预设: " + it.name)
            } else {
                settings.gyro_lens_profile = it.id
                settings.gyro_lens_profile_name = it.name
                // 标定自带的相机属性（帧读出时间/方向）填进「稳定化」页 —— 设置页才是真源，
                // 只把它塞进 stabilizer 会被设置页的值覆盖。
                try {
                    var lf = JSON.parse(settings.lens_fields(it.id))
                    for (var k in lf) if (lf.hasOwnProperty(k)) settings[k] = lf[k]
                } catch (e) {}
                win.refreshFields()
                win.applyChange("gyro_lens_profile")
                win.appendLog("INFO", "镜头标定: " + it.name)
            }
            box.text = ""
            hits.clear()
            pop.close()
        }
        function clearPreset() {
            settings.gyro_preset = ""
            win.refreshFields()
            win.applyChange("gyro_preset")
        }
        function clearLens() {
            settings.gyro_lens_profile = ""
            settings.gyro_lens_profile_name = ""
            win.refreshFields()
            win.applyChange("gyro_lens_profile")
        }
        // 空查询 = 只取镜头库状态（条数/路径），用于启动时显示。
        Component.onCompleted: search("")

        RowLayout {
            width: parent.width
            spacing: 6
            Label { text: ps.label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
            TextField {
                id: box
                Layout.fillWidth: true
                Layout.preferredHeight: 28
                font.pixelSize: 13
                placeholderText: "搜索..."     // 没有输入时的灰色占位
                onTextChanged: {
                    if (text.length) { debounce.restart() }
                    else { hits.clear(); pop.close() }
                }
                onAccepted: { if (pop.opened && lv.currentIndex >= 0) ps.pick(lv.currentIndex) }
                Keys.onDownPressed: {
                    if (!pop.opened && hits.count > 0) pop.open()
                    lv.currentIndex = Math.min(hits.count - 1, lv.currentIndex + 1)
                }
                Keys.onUpPressed: lv.currentIndex = Math.max(0, lv.currentIndex - 1)
                Keys.onEscapePressed: pop.close()
            }
        }
        Timer { id: debounce; interval: 180; repeat: false; onTriggered: ps.search(box.text) }
        // 镜头库加载完（约 1.4s）自动把状态提示从「加载中」换成条数/路径。
        Timer {
            interval: 1200
            running: !ps.dbReady
            repeat: true
            onTriggered: ps.search(box.text)
        }

        Popup {
            id: pop
            x: box.x
            y: box.y + box.height + 2
            width: box.width
            height: Math.min(320, hits.count * 24 + 6)
            padding: 2
            closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
            ListView {
                id: lv
                anchors.fill: parent
                clip: true
                model: hits
                currentIndex: -1
                delegate: Rectangle {
                    width: lv.width
                    height: 24
                    color: lv.currentIndex === index ? Theme.highlight : (mouse.containsMouse ? Theme.highlight : Theme.button)
                    Row {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8
                        Text {
                            width: parent.width - 76
                            anchors.verticalCenter: parent.verticalCenter
                            text: model.name
                            color: "#222"
                            font.pixelSize: 12
                            elide: Text.ElideRight
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: model.note
                            color: model.kind === "preset" ? Theme.accent : "#8a8a8a"
                            font.pixelSize: 11
                        }
                    }
                    MouseArea {
                        id: mouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: ps.pick(index)
                    }
                }
            }
        }

        // 当前镜头标定（带 × 清除；预设用上面的「预设文件」输入框）。
        RowLayout {
            width: parent.width
            spacing: 6
            visible: { win.refreshTick; return String(settings.gyro_lens_profile || "").length > 0 }
            Label { text: "镜头标定"; Layout.preferredWidth: 150; color: Theme.muted; font.pixelSize: 12 }
            Label {
                Layout.fillWidth: true
                text: { win.refreshTick; return String(settings.gyro_lens_profile_name || settings.gyro_lens_profile) }
                color: Theme.accent; font.pixelSize: 12; elide: Text.ElideMiddle
                HoverHandler { id: lensHover }
                ToolTip.visible: lensHover.hovered
                ToolTip.text: { win.refreshTick; return String(settings.gyro_lens_profile || "") }
                ToolTip.delay: 400
            }
            TButton {
                text: "\u2715"; Layout.preferredHeight: 22; Layout.preferredWidth: 26; font.pixelSize: 11
                ToolTip.visible: hovered
                ToolTip.text: "清除镜头标定（回到预设自带的标定）"
                ToolTip.delay: 300
                onClicked: ps.clearLens()
            }
        }
    }
    // 颜色行：色块同样对齐输入框左边缘（156），末尾填充项吸收剩余宽度（同 Check）。
    component ColorField: RowLayout {
        property string label
        property string prop
        /// 配套的「透明度」设置项（0..255）。填了色块就按它半透明显示，
        /// 让用户在调滑杆时立刻看到效果，而不是只看到纯色块。
        property string opacityProp: ""
        width: parent.width
        spacing: 6
        Label { text: label; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
        Rectangle {
            Layout.preferredWidth: 44; Layout.preferredHeight: 24
            color: {
                win.refreshTick
                return win.qtColor(settings[prop], opacityProp === "" ? -1 : Number(settings[opacityProp]))
            }
            border.color: "#999"; border.width: 1; radius: 2
        }
        TButton { text: "拾取"; onClicked: win.pickColor(prop, opacityProp); Layout.preferredHeight: 28; Layout.preferredWidth: 64; font.pixelSize: 13 }
        Item { Layout.fillWidth: true; Layout.fillHeight: true }
    }

    // ---------- state + helpers ----------
    property var logModel: ListModel {}
    property string currentPath: settings.file_path
    property string colorTarget: ""
    property var browseInput: null
    // 「浏览…」直写某个 settings 属性（不经过 FileInput）：owner.reload() 让下拉重建。
    property string browseSettingProp: ""
    property var browseSettingOwner: null
    property int refreshTick: 0
    property string pendingProp: ""
    // 已排队的改动对应的可读描述（如「画面占比 = 30.0」），日志里用它代替裸属性名。
    property string pendingNote: ""
    property bool scrubbing: false
    property string gyroStatusText: ""

    // 主题：settings.theme ∈ light|dark|system；system 用 Qt 的配色方案提示解析。
    function applyTheme() {
        var t = String(settings.theme || "light")
        if (t === "system") {
            var dark = false
            try { dark = (Qt.styleHints.colorScheme === Qt.ColorScheme.Dark) } catch (e) { dark = false }
            t = dark ? "dark" : "light"
        }
        Theme.apply_theme(t)
    }
    // 「分段」页的跳转：把视频定位到该分段的**开始**时刻（视频时间轴，与导出边界同一套换算）。
    // 每行填的时间是「该分段的结束」= 下一个分段点，所以本段的开始 = 上一个分段点：
    // 行 k（0 基）→ 分段点 k。分段点 0 = 出发时刻，因此第 1 段跳到视频开头。
    function jumpToSplit(rowIndex) {
        var ms = settings.split_video_ms(rowIndex)
        if (!isFinite(ms)) return
        item.seek(ms)
        win.refreshFields()
        appendLog("INFO", "跳转到分段 " + (rowIndex + 1) + " 的起点（" + win.fmtTime(ms) + "）")
    }
    // 「分段范围」输入框（xx-xx）→ 起始/结束分段点。
    function setSegRange(text) {
        var parts = String(text || "").split("-")
        var a = parseInt(parts[0])
        var b = parts.length > 1 ? parseInt(parts[1]) : 0
        if (isNaN(a) || a < 0) a = 0
        if (isNaN(b) || b < 0) b = 0
        settings.encode_start_seg = a
        settings.encode_end_seg = b
        settings.encode_seg_mode = "custom"
        settings.encode_seg_enabled = true
        settings.encode_all_seg = false
        win.refreshFields()
        win.applyChange("encode_seg_mode")
    }
    function setTheme(name) {
        settings.theme = name
        win.applyTheme()
        appendLog("INFO", "已切换外观：" + (name === "light" ? "浅色" : name === "dark" ? "深色" : "跟随系统"))
    }
    function refreshFields() { refreshTick++; win.applyTheme(); if(hudEditor)hudEditor.sync() }
    function isGyroProp(p) {
        // 「硬」防抖输入：只有这些变了才需要真正重载（重探测视频 + 重读运动数据）。
        var g = ["gyro_enabled", "gyro_preset", "gyro_lens_profile", "video_path"]
        return g.indexOf(p) >= 0
    }
    // 「软」防抖参数：把新参数应用到已加载的稳定化器上重算即可（快），不必重读运动数据
    // —— 原版 Gyroflow 拖 Stabilization 滑杆就是这样。
    function isGyroParam(p) {
        var g = ["fov", "max_zoom", "max_zoom_iterations", "gyro_zoom_mode", "gyro_zoom_window",
                 "gyro_zoom_center_x", "gyro_zoom_center_y", "gyro_lens", "gyro_horizon",
                 "gyro_horizon_roll", "gyro_use_gravity", "frame_readout_time", "gyro_readout_dir",
                 "gyro_smoothing_method", "gyro_smoothing_params", "gyro_video_speed",
                 "gyro_vs_smoothing", "gyro_vs_zooming", "gyro_vs_zoom_limit",
                 "gyro_rot_x", "gyro_rot_y", "gyro_rot_z", "gyro_tr_x", "gyro_tr_y", "gyro_tr_z",
                 "gyro_offset", "gyro_offset_frames",
                 // 对齐原版 Stabilization 面板新增的项
                 "gyro_horizon_enabled", "gyro_auto_lock", "gyro_horizon_pitch_enabled",
                 "gyro_horizon_pitch", "gyro_integration_method", "gyro_turn_threshold",
                 "gyro_turn_smoothing", "gyro_turn_multiplier", "gyro_limit_tilt_accel",
                 "gyro_tilt_accel_limit", "frame_readout_enabled",
                 "gyro_fl_smoothing", "gyro_fl_strength"]
        return g.indexOf(p) >= 0
    }
    function appendLog(level, msg) {
        logModel.append({ time: Qt.formatTime(new Date(), "HH:mm:ss"), level: level, message: msg })
        // 日志窗口滚动到底部；同时把模型限制在最近 3000 条，拖动滑杆也不会把它撑爆。
        if (logModel.count > 3000) logModel.remove(0, logModel.count - 3000)
        logList.positionViewAtEnd()
    }
    // 整个日志的纯文本（「复制全部」用，方便直接贴出来排查）。
    function logText() {
        var lines = []
        for (var i = 0; i < logModel.count; ++i) {
            var it = logModel.get(i)
            lines.push(it.time + " [" + it.level + "] " + it.message)
        }
        return lines.join("\n")
    }
    // 各列表当前行数：日志与自动排查都用它，一眼看出"怎么变成三行了"。
    function listCounts() {
        return "视频 " + videoRows.rowCount() + " 行 · 轨迹图 " + trackRows.rowCount()
             + " 行 · 分段 " + splitRows.rowCount() + " 行"
    }
    // 自动排查入口（`qmlprobe` 用）：等价于点界面上的操作，走的是同一条函数，
    // 方便复现"拖入 → 删除 → 再拖入"这类时序问题。
    function debugRemoveRow(which, i) {
        if (which === "video") videoRows.removeRow(i)
        else if (which === "track") trackRows.removeRow(i)
        else if (which === "split") splitRows.removeRow(i)
        appendLog("INFO", "debugRemoveRow(" + which + ", " + i + ") · " + listCounts())
    }
    function debugAppendRow(which) {
        if (which === "video") videoRows.appendRow()
        else if (which === "track") trackRows.appendRow()
        else if (which === "split") splitRows.appendRow()
    }
    // 等价于在某一行的文件框里「浏览…」选中一个文件（走同一条 writeBack 路径）。
    function debugSetRow(which, i, path) {
        if (which === "video") videoRows.setRowPath(i, path)
        else if (which === "track") trackRows.setRowPath(i, path)
        else if (which === "split") splitRows.setRowValue(i, path)
        appendLog("INFO", "debugSetRow(" + which + ", " + i + ", " + path + ") · " + listCounts())
    }
    // 当前设置里的原始字符串（排查用：行数不对时先看这几个值）。
    function debugState() {
        return "video_path=[" + settings.video_path + "] video_extras=[" + settings.video_extras
             + "] map_path=[" + settings.map_path + "] map_extras=[" + settings.map_extras
             + "] change_times=[" + settings.map_change_times + "] · " + listCounts()
    }
    // 预览状态串（排查 / 自动化回归用）：位置、是否在播放、是否在加载、总时长、已画帧数、
    // 最后一条日志。`frames` 是"新帧还在来"的硬判据 —— 黑屏时它停在原地，而 t_ms 照样按
    // 墙钟前进（这正是用户看到"只有进度条在走"的原因）。
    function debugPreviewState() {
        var last = logModel.count > 0 ? logModel.get(logModel.count - 1).message : ""
        return "t_ms=" + Math.round(item.t_ms) + " playing=" + item.playing
             + " busy=" + item.busy + " dur=" + Math.round(item.duration_ms)
             + " frames=" + item.debug_frames + " kind=" + item.debug_apply_kind
             + " landed=" + item.debug_full_landed
             + " log=[" + last + "]"
    }
    // 播放/暂停预览（等价于点播放键）。
    function debugPlay(on) {
        item.playing = on
        appendLog("INFO", "debugPlay(" + (on ? "播放" : "暂停") + ")")
        return debugPreviewState()
    }
    // 读一个代理属性（自动化用）：从外部（harness 里的 `win`）拿不到 `settings` 这个 id，
    // 必须在这里转一手。
    function debugGet(prop) { return settings[prop] }
    // 等价于「拖动某个设置控件」：写代理值 → 立即应用（跳过 350ms 防抖）→ 返回状态串。
    // 用来复现"播放中改轨迹显示 → 预览重载 → 黑屏 / 地图与 HUD 冻住"。
    function debugApplyProp(prop, value, note) {
        settings[prop] = value
        applyNow(prop, note === undefined ? (prop + " = " + value) : note)
        return debugPreviewState()
    }
    // 设置值的可读描述：布尔→开/关，长字符串截断，空值→(空)。
    function describeValue(prop) {
        var v = settings[prop]
        if (v === undefined) return "(无此属性)"
        if (typeof v === "boolean") return v ? "开" : "关"
        if (typeof v === "number") return String(v)
        var s = String(v)
        if (!s.length) return "(空)"
        if (s.length > 160) s = s.slice(0, 157) + "..."
        return s
    }
    // 「应用设置」是异步的（后台线程探测视频 / 解码地图 / 建路线管理器）：
    // 开始、落地、失败都写进日志，这样"导入到底完成没有、加载到了什么"看得见。
    Connections {
        target: item
        function onApplySummaryChanged() {
            var s = String(item.apply_summary || "")
            if (!s.length) return
            appendLog(s.indexOf("失败") === 0 ? "ERROR" : "INFO", "加载完成 · " + s)
        }
        function onBusyChanged() {
            if (item.busy) appendLog("INFO", "加载中 · " + String(item.busy_msg || ""))
        }
        // 导出（编码）结束也写日志：成功 / 中止 / 失败一眼可见。
        function onEncodeDoneChanged() {
            if (!item.encode_done) return
            if (item.encode_error.length)
                appendLog(item.encode_aborted ? "WARN" : "ERROR",
                          (item.encode_aborted ? "导出已中止: " : "导出失败: ") + item.encode_error)
            else
                appendLog("INFO", "导出完成: " + item.encode_output)
        }
        // 自动回退到 CPU 之类的非致命提示：写进日志，完整报错在输出视频旁的日志文件里。
        function onEncodeStateChanged() {
            if (item.encode_notice.length)
                appendLog("WARN", item.encode_notice)
            // 编码/解码组合只记一次（值变化时才进这里）。
            if (item.encode_encoder_info.length && item.encode_encoder_info !== lastEncoderInfo) {
                lastEncoderInfo = item.encode_encoder_info
                appendLog("INFO", item.encode_encoder_info)
            }
        }
        property string lastEncoderInfo: ""
    }
    function filePath(url) {
        var s = url.toString()
        s = s.replace(/^file:\/\//, "")
        if (s.length > 2 && s[0] === "/" && s[2] === ":") s = s.substring(1)
        try { s = decodeURIComponent(s) } catch (e) {}
        return s
    }
    function fmtTime(ms) {
        var s = Math.floor(ms / 1000)
        var h = Math.floor(s / 3600); s -= h * 3600
        var m = Math.floor(s / 60); s -= m * 60
        function p(n) { return (n < 10 ? "0" : "") + n }
        return h > 0 ? h + ":" + p(m) + ":" + p(s) : m + ":" + p(s)
    }
    function browseFor(input) {
        browseInput = input
        if (input.saveMode) { browseSaveDialog.open(); return }
        browseDialog.nameFilters = [input.filter + " (*)", "所有文件 (*)"]
        browseDialog.open()
    }
    // 直接给某个 settings 属性挑文件（预设下拉的「浏览…」用）。
    function browseSetting(prop, filter, owner) {
        browseInput = null
        browseSettingProp = prop
        browseSettingOwner = owner || null
        browseDialog.nameFilters = [filter + " (*)", "所有文件 (*)"]
        browseDialog.open()
    }
    // 只取文件名（去掉目录与扩展名），用于显示路径类设置。
    function baseName(p) {
        p = String(p || "").replace(/\\/g, "/")
        var i = p.lastIndexOf("/")
        var n = i >= 0 ? p.substring(i + 1) : p
        return n.replace(/\.[^.]*$/, "")
    }
    // 选中/拖入 .gyroflow：把它的 stabilization 段填进设置页（之后可直接手改，不再依赖该文件）。
    function loadPresetFields(p) {
        var fields = null
        try { fields = JSON.parse(settings.preset_fields(p)) } catch (e) { fields = null }
        if (!fields) return
        for (var k in fields) if (fields.hasOwnProperty(k)) settings[k] = fields[k]
        win.refreshFields()
    }
    function extOf(p) {
        p = String(p || "")
        var i = p.lastIndexOf(".")
        return i >= 0 ? p.substring(i + 1).toLowerCase() : ""
    }
    // ---------- 拖放到预览区：按扩展名导入 ----------
    // .trr / .orv → 打开工程；视频 → 主视频；图片 → 地图/轨迹图（QuickRoute JPEG 自带轨迹）；
    // .gpx → HUD 轨迹；.gyroflow → 防抖预设；.json → 镜头标定。
    function handleDrop(urls) {
        var videoExt = ["mp4", "mov", "mkv", "avi", "m4v", "mts", "m2ts", "insv", "mpg", "mpeg", "webm", "wmv"]
        var imageExt = ["jpg", "jpeg", "png", "bmp", "tif", "tiff", "webp", "qrt"]
        for (var i = 0; i < urls.length; ++i) {
            var p = win.filePath(urls[i])
            if (!p.length) continue
            var e = win.extOf(p)
            if (e === "trr" || e === "orv") {
                win.loadProject(urls[i])
                appendLog("INFO", "拖入工程: " + p)
            } else if (videoExt.indexOf(e) >= 0) {
                // 第一个视频 = 主文件；之后再拖 = 追加续接分段（不替换主视频）。
                // 判断与实际写入都以**列表行**为准（见 `VideoList.addDroppedPath`），
                // 不再去拼 settings 字符串——两处都改字符串的话，"删掉的行"会被
                // 下一次拖入又拼回来（越拖越多）。
                var wasMain = !videoRows.firstPath().length
                videoRows.addDroppedPath(p)
                appendLog("INFO", (wasMain ? "拖入视频（主文件）: " : "拖入视频（续接分段）: ") + p
                          + " · " + win.listCounts())
            } else if (imageExt.indexOf(e) >= 0) {
                // 与「轨迹」页主图行一致：同一个文件既是底图也是轨迹来源。
                // 第一个图片 = 主图；之后再拖 = 追加额外地图（换图时间留空，列表里可改）。
                var asMain = !trackRows.mainPath().length
                trackRows.addDroppedPath(p)
                appendLog("INFO", (asMain ? "拖入地图/轨迹图（主图）: " : "拖入额外地图: ") + p
                          + " · " + win.listCounts())
                // .qrt 自带分段时同样要问，否则拖进来就只剩地图 + 轨迹。
                // 注意用文档级 id `trackRows`：`trackList` 是 TrackList 组件内部的 id，
                // 在这个作用域里解析不到。
                // 路径已经进模型，pendingQrtIndex 记 -1 表示弹窗不需要再回写一次。
                if (e === "qrt" && item.qrt_has_splits(p)) {
                    appendLog("INFO", "检测到 .qrt 分段，等待确认: " + p)
                    trackRows.pendingQrtIndex = -1
                    trackRows.pendingQrtPath = p
                    qrtSplitDialog.open()
                }
            } else if (e === "gpx") {
                settings.hud_gpx_path = p
                win.refreshFields()
                win.applyChange("hud_gpx_path")
                appendLog("INFO", "拖入 GPX: " + p)
            } else if (e === "gyroflow") {
                settings.gyro_preset = p
                win.loadPresetFields(p)
                win.refreshFields()
                win.applyChange("gyro_preset")
                appendLog("INFO", "拖入防抖预设: " + p)
            } else if (e === "json") {
                settings.gyro_lens_profile = p
                settings.gyro_lens_profile_name = win.baseName(p)
                win.refreshFields()
                win.applyChange("gyro_lens_profile")
                appendLog("INFO", "拖入镜头标定: " + p)
            } else {
                appendLog("WARN", "不认识的文件类型，已忽略: " + p)
            }
        }
    }
    // 文件选择框确认后回填：settingProp 非空 → 写回 settings；否则交给列表行（picked）。
    function acceptBrowsed(value) {
        if (browseSettingProp.length) {
            var prop = browseSettingProp
            var owner = browseSettingOwner
            browseSettingProp = ""
            browseSettingOwner = null
            settings[prop] = value
            if (prop === "gyro_preset") win.loadPresetFields(value)
            win.refreshFields()
            win.applyChange(prop)
            appendLog("INFO", "选择文件 " + prop + ": " + value)
            if (owner && owner.reload) owner.reload()
            return
        }
        var input = browseInput
        browseInput = null
        if (!input) return
        if (input.settingProp.length) {
            settings[input.settingProp] = value
            if (input.settingProp === "gyro_preset") win.loadPresetFields(value)
            win.refreshFields()
            win.applyChange(input.settingProp)
            appendLog("INFO", "选择文件 " + input.settingProp + ": " + value)
        } else {
            input.picked(value)
        }
    }
    function pickColor(prop, opacityProp) {
        colorTarget = prop
        var alpha = (opacityProp === undefined || opacityProp === "") ? 255 : Number(settings[opacityProp])
        try { colorDialog.selectedColor = win.qtRgba(settings[prop], alpha) } catch (e) { }
        colorDialog.open()
    }
    // 颜色存储约定：设置里只存 `#rrggbb`，不透明度是**独立**的设置项（0..255）——它由每个
    // 颜色项旁边的「透明度」滑杆控制，不再塞进颜色串里。QML 的颜色字面量是 `#aarrggbb`
    // （不透明度在开头），跨这一步要换序，否则 8 位色会被 QML 当 a=0x00 解读成全透明。
    // 老工程里带 alpha 的 9 字符串仍然兼容（末尾两位被忽略，alpha 由设置项决定）。
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
    // "#rrggbb" + 0..255 的不透明度 → QML 的 color（给取色对话框做初始值，好让它和界面一致）。
    function qtRgba(v, opacity) {
        var c = win.qtColor(v, opacity)
        if (c.length !== 9 || c.charAt(0) !== "#") return "transparent"
        return Qt.rgba(
            parseInt(c.substr(3, 2), 16) / 255,
            parseInt(c.substr(5, 2), 16) / 255,
            parseInt(c.substr(7, 2), 16) / 255,
            parseInt(c.substr(1, 2), 16) / 255
        )
    }
    // 反向：QColorDialog 给回的 color → 存储用的 "#rrggbb"。不透明度**不在这里**，
    // 它由每个颜色项配套的「透明度」设置项负责（对话框里的 alpha 不参与存储）。
    function colorHex(c) {
        function hx(x) {
            var n = Math.round(Math.max(0, Math.min(1, Number(x))) * 255)
            var s = n.toString(16)
            return s.length < 2 ? "0" + s : s
        }
        return "#" + hx(c.r) + hx(c.g) + hx(c.b)
    }
    // Debounced auto-apply: every control change goes through here. If a DIFFERENT field
    // is touched, the previous pending edit is applied SYNCHRONOUSLY first — so moving to
    // the next setting makes the last one take effect immediately instead of being silently
    // coalesced away by the shared debounce timer.
    function applyChange(prop, note) {
        if (pendingProp !== "" && pendingProp !== prop) { applyTimer.stop(); doApply(pendingProp, pendingNote) }
        pendingProp = prop; pendingNote = (note === undefined ? "" : note); applyTimer.restart()
    }
    // 结构变化（增删行、按分段选时刻）立即生效：防抖窗口里预览还停在旧地图上，
    // 会让人以为"删掉了但画面没变"。
    function applyNow(prop, note) {
        applyTimer.stop()
        pendingProp = ""
        doApply(prop, note === undefined ? "" : note)
        pendingNote = ""
    }
    function isEncodeProp(prop) { return prop.indexOf("encode_") === 0 }
    function doApply(prop, note) {
        // Apply changes to the preview via a temp file — NEVER auto-save to the loaded
        // project. Only 保存设置 / 保存配置 writes the project file.
        var p = settings.temp_apply_path()
        if (!p.length) return
        // 每条改动都记一行：可读描述（组件传进来的 label）优先，否则退化成"属性 = 值"。
        // 这样"勾了什么、改了什么值"在日志里能对上。
        var what = (note !== undefined && note !== "") ? note : (prop + " = " + describeValue(prop))
        // Encode-page changes only affect the export, never the preview: refresh the
        // in-memory settings lightly (no re-probe / no seek / no overlay). Still bump
        // refreshTick so field bindings (no NOTIFY on the proxy) re-read their values —
        // e.g. the 按分段导出 checkbox's `visible` gating and the browse-filled output path.
        if (prop === "hud_preview") {
            item.apply_hud_preview(!!settings.hud_preview)
            refreshFields()
            appendLog("INFO", settings.hud_preview ? "HUD 预览已开启" : "HUD 预览已关闭")
            return
        }
        if (isEncodeProp(prop)) {
            item.apply_encode_only(p)
            refreshFields()
            appendLog("INFO", "编码设置 " + what + "（不影响预览）")
            return
        }
        // 防抖相关属性（参数或硬输入）都走这条轻量路径：
        // 参数改动 = 只重算（毫秒~秒级）；视频没变的预设/镜头/开关改动 = 只重建稳定化器；
        // 只有视频真的换了才退回完整 apply_settings。
        if (isGyroParam(prop) || isGyroProp(prop)) {
            item.apply_gyro_params(p, prop)
            refreshFields()
            appendLog("INFO", "防抖设置 " + what)
            return
        }
        // 轨迹/地图的"显示"类改动（线宽、颜色、透明度、尾巴、控制点、跑者、视野模式、
        // 取景边距、画面占比…）先走就地更新：Rust 侧会逐键比对设置，确认只有显示项变化时
        // 就地刷新样式与取景参数（毫秒级、预览不重载、播放不中断）；只要有任何一项
        // 视频/地图/轨迹文件、分段时间、GPX、防抖或编码相关的变化，它自己会退回完整重载
        // —— 所以这里不再逐个控件判断 prop，统一交给它。
        item.apply_route_display(p)
        refreshFields()
        seek.to = item.duration_ms
        appendLog("INFO", item.busy ? ("开始加载 · " + what) : ("已更新 · " + what))
    }
    function loadProject(url) {
        var p = filePath(url)
        appendLog("INFO", "打开项目: " + p)
        settings.load_settings_from(p)
        item.reload(p)
        currentPath = p
        refreshFields()
        hudEditor.reload()
        seek.to = item.duration_ms
        appendLog("INFO", "已加载项目: " + p)
    }
    function saveProject(url) {
        var p = filePath(url)
        settings.save_to(p)
        currentPath = p
        appendLog("INFO", "已保存设置: " + p)
    }

    Component.onCompleted: {
        appendLog("INFO", "TraceReel 已启动")
        win.applyTheme()
        item.reload(currentPath)
        item.load_queue()
    }

    // ---------- menu ----------
    menuBar: MenuBar {
        Menu {
            title: "加载"
            // 单击直接弹出工程文件选择框：在 aboutToShow 里 close() 会阻止
            // 空子菜单弹出（Popup.open() 发信号后检查 allowOpen），零闪烁。
            onAboutToShow: {
                close()
                if (!openDialog.visible)
                    openDialog.open()
            }
        }
        Menu {
            title: "外观"
            MenuItem { text: "浅色";   onTriggered: win.setTheme("light") }
            MenuItem { text: "深色";   onTriggered: win.setTheme("dark") }
            MenuItem { text: "跟随系统"; onTriggered: win.setTheme("system") }
        }
        Menu {
            title: "日志(&L)"
            onAboutToShow: {
                close()
                logWindow.show()
                logWindow.raise()
                logWindow.requestActivate()
            }
        }
        Menu {
            title: "导出队列"
            onAboutToShow: {
                close()
                queueWindow.show()
                queueWindow.raise()
                queueWindow.requestActivate()
            }
        }
        Menu {
            title: "帮助(&H)"
            onAboutToShow: {
                close()
                if (!aboutDialog.visible) aboutDialog.open()
            }
        }
    }

    // ---------- settings proxy ----------
    SettingsProxy {
        id: settings
        file_path: orvPath
        // SettingsProxy 的 Component.onCompleted 晚于 HudEditor 的：HUD 编辑器首次
        // reload 时 hud_layout 还空着，所以这里加载完设置后再刷新一次字段 + 同步 HUD 布局。
        Component.onCompleted: { reload_settings(); win.refreshFields(); profiles_warmup() }
    }

    function addExportQueue() {
        applyTimer.stop()
        pendingProp = ""
        item.add_to_queue(settings.export_snapshot(), currentPath.length ? currentPath : "未保存项目")
        queueWindow.show()
    }
    Window {
        id: queueWindow
        title: "导出队列"
        width: 900; height: 560
        minimumWidth: 700; minimumHeight: 360
        visible: false
        color: Theme.background
        property var entries: {
            try { return JSON.parse(item.queue_json || "[]") } catch (e) { return [] }
        }
        function durationText(seconds) {
            var ms = Math.round(seconds * 1000)
            return Math.floor(ms / 3600000) + ":" + ("0" + Math.floor(ms / 60000) % 60).slice(-2)
                + ":" + ("0" + Math.floor(ms / 1000) % 60).slice(-2) + "." + ("00" + ms % 1000).slice(-3)
        }
        function lengthsText(entry) {
            if (!entry.durations) return entry.durationError ? "成片时长无法计算：" + entry.durationError : "成片时长：计算中…"
            if (entry.durations.length === 1) return "成片时长：" + durationText(entry.durations[0])
            var total = 0
            var parts = entry.durations.map(function(d, i) { total += d; return "[" + i + "-" + (i+1) + "] " + durationText(d) })
            return "各段成片时长：" + parts.join(" · ") + "\n合计：" + durationText(total)
        }
        ColumnLayout {
            anchors.fill: parent; anchors.margins: 12; spacing: 10
            RowLayout {
                TButton {
                    text: "开始 / 重试队列"
                    enabled: !item.encoding && !item.queue_running && queueWindow.entries.some(function(e) { return e.status !== "已完成" })
                    onClicked: { win.encodeTotalWall = 0; item.start_queue() }
                }
                TButton { text: item.encode_paused ? "继续" : "暂停"; enabled: item.queue_running && item.encoding; onClicked: item.pause_encode(!item.encode_paused) }
                TButton { text: "中止队列"; enabled: item.queue_running || queueWindow.entries.some(function(e) { return e.status === "导出中" }); onClicked: item.stop_queue() }
                Item { Layout.fillWidth: true }
                Label { text: queueWindow.entries.length + " 项"; color: Theme.text }
            }
            Label {
                text: "每项保存添加时的配置，按顺序导出。成功后自动移除；失败或中止后保留任务。"
                color: Theme.text; wrapMode: Text.Wrap; Layout.fillWidth: true
            }
            Label { text: item.queue_error; visible: text.length > 0; color: "#d9534f"; wrapMode: Text.Wrap; Layout.fillWidth: true }
            ScrollView {
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                ListView {
                    id: queueList
                    model: queueWindow.entries
                    spacing: 10
                    delegate: Rectangle {
                        required property var modelData
                        required property int index
                        width: queueList.width; height: queueRow.implicitHeight + 20
                        color: "transparent"; border.color: "#888"; radius: 4
                        ColumnLayout {
                            id: queueRow
                            anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top; anchors.margins: 10
                            RowLayout {
                                Label { text: (index + 1) + ". " + modelData.project; elide: Text.ElideMiddle; Layout.fillWidth: true; color: Theme.text }
                                Label { text: modelData.status; color: Theme.text; font.bold: true }
                                TButton { text: "移除"; enabled: !item.queue_running && !item.encoding; onClicked: item.remove_queue_entry(index) }
                            }
                            Repeater {
                                model: modelData.configuration
                                delegate: Label {
                                    required property string modelData
                                    text: modelData; color: Theme.text; wrapMode: Text.Wrap; Layout.fillWidth: true
                                }
                            }
                            Label { text: queueWindow.lengthsText(modelData); color: Theme.text; wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Label { text: modelData.warning; visible: text.length > 0; color: "#b07800"; wrapMode: Text.Wrap; Layout.fillWidth: true }
                            GridLayout {
                                Layout.fillWidth: true
                                columns: 2
                                columnSpacing: 5; rowSpacing: 5
                                TextField { id: queuedOutput; text: modelData.output; Layout.fillWidth: true; enabled: !item.queue_running && !item.encoding; selectByMouse: true }
                                TButton { text: "修改输出"; enabled: !item.queue_running && !item.encoding; onClicked: item.change_queue_output(index, queuedOutput.text) }
                                ProgressBar {
                                    objectName: "queueEntryProgress"
                                    Layout.fillWidth: true; from: 0; to: 1
                                    value: modelData.active ? item.queue_progress : modelData.progress
                                    indeterminate: modelData.active && win.num(item.encode_total_frames) === 0
                                }
                                Label {
                                    text: ((modelData.active ? item.queue_progress : modelData.progress) * 100).toFixed(1) + "%"
                                    color: Theme.text
                                }
                            }
                            Label { text: modelData.error; visible: text.length > 0; color: "#d9534f"; wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Label {
                                visible: modelData.active
                                text: item.encode_paused ? "已暂停" : (item.encode_segment.length ? item.encode_segment : "导出中…")
                                color: Theme.text; wrapMode: Text.Wrap; Layout.fillWidth: true
                            }
                        }
                    }
                }
            }
        }
    }

    // ---------- log window (opened from the 日志 menu) ----------
    Window {
        id: logWindow
        title: "日志"
        width: 680
        height: 420
        visible: false
        Column {
            anchors.fill: parent
            anchors.margins: 6
            Row {
                spacing: 6
                Label { text: "日志"; color: Theme.text; font.bold: true; anchors.verticalCenter: parent.verticalCenter }
                TButton {
                    text: "复制全部"
                    onClicked: {
                        // 用隐藏的 TextArea 走剪贴板（QML 的 TextEdit.copy 复制的是选中内容）。
                        logCopyArea.text = win.logText()
                        logCopyArea.selectAll()
                        logCopyArea.copy()
                        appendLog("INFO", "已复制 " + logModel.count + " 行日志到剪贴板")
                    }
                }
                TButton { text: "清空"; onClicked: logModel.clear() }
                Label {
                    text: logModel.count + " 行"
                    color: Theme.muted; font.pixelSize: 11
                    anchors.verticalCenter: parent.verticalCenter
                }
            }
            // 1×1 透明，仅用于剪贴板（不能用 visible:false —— 那会让它拿不到选区，
            // copy() 复制不到东西）。
            TextArea {
                id: logCopyArea
                width: 1; height: 1; opacity: 0
                activeFocusOnTab: false
            }
            ListView {
                id: logList
                width: parent.width
                height: parent.height - 46
                model: logModel
                clip: true
                delegate: Row {
                    spacing: 8
                    width: logList.width
                    Label { text: model.time; color: Theme.muted; font.pixelSize: 11; width: 70 }
                    Label {
                        text: "[" + model.level + "]"
                        color: model.level === "WARN" ? "#a86e00" : (model.level === "ERROR" ? "#b00020" : "#2e7d32")
                        font.pixelSize: 11; width: 70
                    }
                    Label { text: model.message; color: "#222"; font.pixelSize: 11; elide: Text.ElideRight; width: logList.width - 160 }
                }
            }
        }
    }

    // ---------- settings panel (right) ----------
    Rectangle {
        id: settingsPanel
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        width: 500
        color: Theme.background2

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            TabBar {
                id: tabs
                Layout.fillWidth: true
                background: Rectangle {
                    color: Theme.background2
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.popup_border }
                }
                TabButton {
                    text: "轨迹"
                    font.pixelSize: 13
                    background: Rectangle { color: "transparent" }
                    contentItem: Text {
                        text: parent.text
                        font: parent.font
                        color: parent.checked ? Theme.accent : Theme.text
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.OutExpo } }
                    }
                }
                TabButton {
                    text: "分段"
                    font.pixelSize: 13
                    background: Rectangle { color: "transparent" }
                    contentItem: Text {
                        text: parent.text
                        font: parent.font
                        color: parent.checked ? Theme.accent : Theme.text
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.OutExpo } }
                    }
                }
                TabButton {
                    text: "视频"
                    font.pixelSize: 13
                    background: Rectangle { color: "transparent" }
                    contentItem: Text {
                        text: parent.text
                        font: parent.font
                        color: parent.checked ? Theme.accent : Theme.text
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.OutExpo } }
                    }
                }
                TabButton {
                    text: "HUD"
                    font.pixelSize: 13
                    background: Rectangle { color: "transparent" }
                    contentItem: Text {
                        text: parent.text
                        font: parent.font
                        color: parent.checked ? Theme.accent : Theme.text
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.OutExpo } }
                    }
                }
                TabButton {
                    text: "导出"
                    font.pixelSize: 13
                    background: Rectangle { color: "transparent" }
                    contentItem: Text {
                        text: parent.text
                        font: parent.font
                        color: parent.checked ? Theme.accent : Theme.text
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.OutExpo } }
                    }
                }
                // 选中指示条（跟随选中页滑动）
                Rectangle {
                    height: 2 * Theme.dpi_scale
                    width: tabs.currentItem ? tabs.currentItem.width : 0
                    y: tabs.height - height
                    color: Theme.accent
                    x: tabs.currentItem ? tabs.currentItem.x : 0
                    Behavior on x { NumberAnimation { duration: 250; easing.type: Easing.OutExpo } }
                    Behavior on width { NumberAnimation { duration: 250; easing.type: Easing.OutExpo } }
                }
            }

            StackLayout {
                currentIndex: tabs.currentIndex
                Layout.fillWidth: true
                Layout.fillHeight: true

                // ================= 轨迹 (地图 + 路线) =================
                ScrollView {
                    id: svTrack
                    contentWidth: availableWidth
                    contentHeight: colTrack.height
                    Column {
                        id: colTrack
                        width: svTrack.availableWidth
                        padding: 10
                        spacing: 10
                        GroupBox {
                            title: "轨迹文件"
                            TrackList { id: trackRows }
                        }
                        GroupBox {
                            title: "校准"
                            DSpin { label: "按表-出发时间"; prop: "route_control_time_offset"; hint: "出发时刻在轨迹时间轴上的秒数：出发前几秒开始按表则为正值（例如 3）。分段时间的 0 点就是出发时刻。" }
                        }
                        GroupBox {
                            title: "显示"
                            // 每片区域一个二级小标题（SubSection）：路线 / 尾巴 / 控制点 / 运动员，
                            // 每块自己一个颜色 + 一个独立的「透明度」滑杆。参数项只在真正参与绘制时
                            // 显示（模式=「无」或没勾选时，宽度/长度/颜色都没有意义——原版是常显）。
                            // visible 里读 refreshTick 是为了改了模式/勾选后立刻重算绑定，
                            // 不用等防抖的 applyChange。
                            // 画面占比：无轨迹图片时视频自动铺满整帧，这个值没有意义 → 隐藏。
                            PercentSpin { label: "画面占比 (%)"; prop: "relative_width"; visible: trackRows.hasMap }
                            SubSection {
                                title: "路线"
                                // 只留原版的 0=无 / 1=简洁 / 3=配速；2 是原版的「高亮」，本程序不提供，
                                // 但编号不挪位，好让同一个 .orv 在原版里打开时模式仍然对得上。
                                Combo {
                                    label: "路线渲染模式"; prop: "route_render_mode"
                                    items: ["无", "简洁", "配速"]; values: [0, 1, 3]
                                }
                                // 渲染范围：完整 = 整条路线（默认）；单段 = 只画当前所在的那一段。
                                // 「当前段」的判定与取景用的完全一致 —— 画面框住哪一段，就只画哪一段。
                                Combo {
                                    label: "渲染范围"; prop: "route_render_scope"
                                    items: ["完整渲染", "单段渲染"]; values: [0, 1]
                                    hint: "完整渲染 = 整条路线；单段渲染 = 只画当前所在的那一段（判定与取景一致：画面框住哪一段就只画哪一段）。预览与导出都生效。"
                                    visible: { win.refreshTick; return settings.route_render_mode !== 0 }
                                }
                                DSpin {
                                    label: "路线宽度 (像素)"; prop: "route_width"
                                    visible: { win.refreshTick; return settings.route_render_mode !== 0 }
                                }
                                // 配速模式的颜色由「低/高配速」色带决定，这时给一个不生效的取色器
                                // 只会让人以为「改了没反应」——所以只在单色（简洁）模式下显示。
                                // 判断写成「不是无、也不是配速」：老工程里的 2（原版高亮）也是按简洁画的，
                                // 读盘时虽已归一成 1，这里再兜一层，免得出现「画了却没得改」。
                                ColorField {
                                    label: "路线颜色"; prop: "route_color"; opacityProp: "route_opacity"
                                    visible: { win.refreshTick; return settings.route_render_mode !== 0 && settings.route_render_mode !== 3 }
                                }
                                // 透明度三种模式都有效：配速模式下逐段线也用这个不透明度。
                                SliderField {
                                    label: "路线透明度 (%)"; prop: "route_opacity"; from: 0; to: 100; decimals: 0; alphaPercent: true
                                    hint: "0% = 完全透明，100% = 完全不透明"
                                    visible: { win.refreshTick; return settings.route_render_mode !== 0 }
                                }
                                Check {
                                    label: "配速色带自适应"; prop: "route_pace_auto"
                                    hint: "按实际轨迹的配速分布自动定色带上下限（掐掉最快/最慢各 10% 的极值）"
                                    visible: { win.refreshTick; return settings.route_render_mode === 3 }
                                    // 关掉自适应时，先把手动值对齐到当前**实际生效**的色带，
                                    // 否则滑杆会跳回工程里存的旧值、画面颜色跟着突变一下。
                                    onUncheckedAction: {
                                        if (item.pace_auto_low > 0 && item.pace_auto_high > 0) {
                                            settings.route_low_pace = item.pace_auto_low
                                            settings.route_high_pace = item.pace_auto_high
                                        }
                                    }
                                }
                                // 自适应开着时这两个值由轨迹算出来，手填没有意义 → 不显示。
                                DSpin {
                                    label: "低配速 (分钟/公里)"; prop: "route_low_pace"
                                    hint: "配速达到这个值（更慢）画成红色"
                                    visible: { win.refreshTick; return (settings.route_render_mode === 3) && !settings.route_pace_auto }
                                }
                                DSpin {
                                    label: "高配速 (分钟/公里)"; prop: "route_high_pace"
                                    hint: "配速达到这个值（更快）画成绿色"
                                    visible: { win.refreshTick; return (settings.route_render_mode === 3) && !settings.route_pace_auto }
                                }
                            }
                            SubSection {
                                title: "尾巴"
                                // 尾巴和路线一样有「无 / 简洁 / 配速」三种渲染模式，编号同样沿用原版
                                // 的 0/1/3（2 是原版尾巴的「高亮」，本程序按简洁渲染）。
                                Combo {
                                    label: "尾巴渲染模式"; prop: "route_tail_render_mode"
                                    items: ["无", "简洁", "配速"]; values: [0, 1, 3]
                                    hint: "无 = 不画尾巴；简洁 = 单色拖尾；配速 = 和路线一样按配速渐变着色"
                                }
                                DSpin {
                                    label: "尾巴宽度 (像素)"; prop: "route_tail_width"
                                    visible: { win.refreshTick; return settings.route_tail_render_mode !== 0 }
                                }
                                DSpin {
                                    label: "尾巴长度 (秒)"; prop: "route_tail_length"
                                    visible: { win.refreshTick; return settings.route_tail_render_mode !== 0 }
                                }
                                // 配速模式的颜色由「低/高配速」色带决定，取色器同样只在单色（简洁）下显示。
                                ColorField {
                                    label: "尾巴颜色"; prop: "route_tail_color"; opacityProp: "route_tail_opacity"
                                    visible: { win.refreshTick; return settings.route_tail_render_mode !== 0 && settings.route_tail_render_mode !== 3 }
                                }
                                SliderField {
                                    label: "尾巴透明度 (%)"; prop: "route_tail_opacity"; from: 0; to: 100; decimals: 0; alphaPercent: true
                                    hint: "0% = 完全透明，100% = 完全不透明"
                                    visible: { win.refreshTick; return settings.route_tail_render_mode !== 0 }
                                }
                                Check {
                                    label: "尾巴配速色带自适应"; prop: "route_tail_pace_auto"
                                    hint: "按实际轨迹的配速分布自动定色带上下限（掐掉最快/最慢各 10% 的极值）"
                                    visible: { win.refreshTick; return settings.route_tail_render_mode === 3 }
                                    // 关掉自适应时，先把手动值对齐到当前**实际生效**的色带，
                                    // 否则滑杆会跳回工程里存的旧值、画面颜色跟着突变一下。
                                    onUncheckedAction: {
                                        if (item.pace_auto_low > 0 && item.pace_auto_high > 0) {
                                            settings.route_tail_low_pace = item.pace_auto_low
                                            settings.route_tail_high_pace = item.pace_auto_high
                                        }
                                    }
                                }
                                // 自适应开着时这两个值由轨迹算出来，手填没有意义 → 不显示。
                                DSpin {
                                    label: "尾巴低配速 (分钟/公里)"; prop: "route_tail_low_pace"
                                    hint: "配速达到这个值（更慢）画成红色"
                                    visible: { win.refreshTick; return (settings.route_tail_render_mode === 3) && !settings.route_tail_pace_auto }
                                }
                                DSpin {
                                    label: "尾巴高配速 (分钟/公里)"; prop: "route_tail_high_pace"
                                    hint: "配速达到这个值（更快）画成绿色"
                                    visible: { win.refreshTick; return (settings.route_tail_render_mode === 3) && !settings.route_tail_pace_auto }
                                }
                            }
                            SubSection {
                                title: "控制点"
                                Check { label: "显示控制点"; prop: "route_show_controls" }
                                DSpin {
                                    label: "控制点半径"; prop: "route_control_radius"
                                    visible: { win.refreshTick; return settings.route_show_controls }
                                }
                                ColorField {
                                    label: "控制点颜色"; prop: "route_control_color"; opacityProp: "route_control_opacity"
                                    visible: { win.refreshTick; return settings.route_show_controls }
                                }
                                SliderField {
                                    label: "控制点透明度 (%)"; prop: "route_control_opacity"; from: 0; to: 100; decimals: 0; alphaPercent: true
                                    hint: "0% = 完全透明，100% = 完全不透明"
                                    visible: { win.refreshTick; return settings.route_show_controls }
                                }
                            }
                            SubSection {
                                title: "运动员"
                                Check { label: "显示运动员"; prop: "route_show_runner" }
                                // 箭头会顺着运动方向转；圆形就是原版的行为。
                                Combo {
                                    label: "运动员样式"; prop: "route_runner_style"
                                    items: ["圆形", "箭头"]; values: [0, 1]
                                    visible: { win.refreshTick; return settings.route_show_runner }
                                }
                                DSpin {
                                    label: "运动员半径"; prop: "route_runner_radius"
                                    visible: { win.refreshTick; return settings.route_show_runner }
                                }
                                ColorField {
                                    label: "运动员颜色"; prop: "route_runner_color"; opacityProp: "route_runner_opacity"
                                    visible: { win.refreshTick; return settings.route_show_runner }
                                }
                                SliderField {
                                    label: "运动员透明度 (%)"; prop: "route_runner_opacity"; from: 0; to: 100; decimals: 0; alphaPercent: true
                                    hint: "0% = 完全透明，100% = 完全不透明"
                                    visible: { win.refreshTick; return settings.route_show_runner }
                                }
                            }
                        }
                        GroupBox {
                            title: "取景"
                            Combo { label: "视野模式"; prop: "route_view_mode"; items: ["固定分段", "跟跑者居中", "跑者居中+分段定向"] }
                            DSpin { label: "左右边距"; prop: "route_lr_margin" }
                            DSpin { label: "上下边距"; prop: "route_tb_margin" }
                        }
                    }
                }

                // ================= 分段 =================
                ScrollView {
                    id: svSplit
                    contentWidth: availableWidth
                    contentHeight: colSplit.height
                    Column {
                        id: colSplit
                        width: svSplit.availableWidth
                        padding: 10
                        spacing: 10
                        GroupBox {
                            title: "分段"
                            TButton { text: "从成绩条图片导入…"; onClicked: receiptImport.open() }
                            Combo { label: "时间类型"; prop: "split_is_relative"; items: ["绝对时间", "相对时间"] }
                            SplitList { id: splitRows }
                        }
                    }
                }

                // ================= 视频 =================
                ScrollView {
                    id: svVid
                    contentWidth: availableWidth
                    contentHeight: colVid.height
                    Column {
                        id: colVid
                        width: svVid.availableWidth
                        padding: 10
                        spacing: 10
                        GroupBox {
                            title: "视频文件"
                            VideoList { id: videoRows }
                        }
                        GroupBox {
                            title: "校准"
                            DSpin { label: "录像-出发时间"; prop: "video_start_offset"; hint: "出发时刻在视频时间轴上的秒数：出发前几十秒开始录像则为正值（例如 40）。运动员偏移 = 按表-出发时间 − 录像-出发时间 自动换算。" }
                        }
                        GroupBox {
                            title: "视频防抖 (Gyroflow)"
                            Check { label: "启用视频防抖"; prop: "gyro_enabled" }
                            Field { label: "预设文件"; prop: "gyro_preset"; filter: "*.gyroflow" }
                            ProfileSearch { label: "镜头标定" }
                            Check { label: "预览关闭防抖"; prop: "preview_perf" }
                            Label { text: gyroStatusText; color: Theme.accent; wrapMode: Text.Wrap; width: parent.width - 20 }
                        }
                        GroupBox {
                            title: "稳定化"
                            SmoothParams { id: smoothParams }

                            // ---------- 地平线锁定 ----------
                            Check { label: "锁定地平线"; prop: "gyro_horizon_enabled" }
                            SliderField {
                                label: "锁定量"; prop: "gyro_horizon"; from: 0; to: 100; decimals: 0; scale: 100; unit: "%"
                                enabled: { win.refreshTick; return !!settings.gyro_horizon_enabled && !settings.gyro_auto_lock }
                                hint: "地平线锁定的强度：0 = 不锁，100 = 完全锁定水平"
                            }
                            SliderField {
                                label: "横滚角度校正"; prop: "gyro_horizon_roll"; from: -180; to: 180; decimals: 1; unit: "°"
                                enabled: { win.refreshTick; return !!settings.gyro_horizon_enabled }
                            }
                            Check {
                                label: "锁定俯仰角"; prop: "gyro_horizon_pitch_enabled"
                                enabled: { win.refreshTick; return !!settings.gyro_horizon_enabled }
                            }
                            SliderField {
                                label: "俯仰角度校正"; prop: "gyro_horizon_pitch"; from: -90; to: 90; decimals: 1; unit: "°"
                                enabled: { win.refreshTick; return !!settings.gyro_horizon_enabled && !!settings.gyro_horizon_pitch_enabled }
                            }
                            Check { label: "使用重力矢量"; prop: "gyro_use_gravity" }
                            Combo {
                                label: "积分方法"; prop: "gyro_integration_method"
                                items: ["互补滤波", "VQF", "陀螺 + 加速度计", "Mahony", "Madgwick"]
                            }
                            Check { label: "自动锁定"; prop: "gyro_auto_lock" }
                            Column {
                                width: parent.width
                                spacing: 3
                                visible: { win.refreshTick; return !!settings.gyro_auto_lock }
                                SliderField {
                                    label: "转弯阈值"; prop: "gyro_turn_threshold"; from: 0; to: 50; decimals: 1; unit: "°/秒"
                                    hint: "横滚角速度超过该阈值时，地平线按角速度成比例倾斜"
                                }
                                SliderField {
                                    label: "转弯平滑"; prop: "gyro_turn_smoothing"; from: 0; to: 2000; decimals: 0; unit: "毫秒"
                                    hint: "转弯检测的平滑时间常数"
                                }
                                SliderField {
                                    label: "转弯倍率"; prop: "gyro_turn_multiplier"; from: 0; to: 2; decimals: 2; unit: "倍"
                                    hint: "动态倾斜倍率：倾斜角 = 倍率 × 横滚角速度"
                                }
                                Check { label: "限制倾斜加速度"; prop: "gyro_limit_tilt_accel" }
                                SliderField {
                                    label: "倾斜加速度上限"; prop: "gyro_tilt_accel_limit"; from: 0.1; to: 30; decimals: 1; unit: "°/秒²"
                                    visible: { win.refreshTick; return !!settings.gyro_limit_tilt_accel }
                                    hint: "限制倾斜的变化快慢，避免地平线动作发顿"
                                }
                            }

                            // ---------- 缩放 / 镜头 ----------
                            SliderField {
                                label: "缩放限额"; prop: "max_zoom"; from: 110; to: 300; decimals: 0; unit: "%"
                                hint: "缩放上限：100 = 不放大，130 = 最多放大到 130%（原版：限额为近似值，要更准就加迭代次数）"
                            }
                            SliderField {
                                label: "缩放速度"; prop: "gyro_zoom_window"; from: 0.1; to: 15; decimals: 2; unit: "秒"
                                enabled: { win.refreshTick; return Number(settings.gyro_zoom_mode) === 1 }
                                hint: "动态缩放的时间窗口；只有「动态缩放」时有效（原版默认 4 秒）"
                            }
                            SliderField {
                                label: "镜头校正"; prop: "gyro_lens"; from: 0; to: 100; decimals: 0; scale: 100; unit: "%"
                                hint: "鱼眼去畸变的强度（需要镜头标定）；0 = 不校正"
                            }

                            // ---------- 稳定焦距（只有逐帧焦距的视频才显示，与原版一致） ----------
                            Check {
                                label: "稳定焦距"; prop: "gyro_fl_smoothing"
                                visible: item.gyro_has_focal_length
                                hint: "对逐帧焦距做平滑（需要视频自带镜头焦距数据）"
                            }
                            SliderField {
                                label: "焦距平滑度"; prop: "gyro_fl_strength"; from: 1; to: 100; decimals: 0; unit: "%"
                                visible: item.gyro_has_focal_length && !!settings.gyro_fl_smoothing
                            }

                            // ---------- 只读状态（原版 Max rotation / Max zoom） ----------
                            Label {
                                width: parent.width - 20
                                color: Theme.muted; font.pixelSize: 11; wrapMode: Text.Wrap
                                text: {
                                    win.refreshTick
                                    var st = null
                                    try { st = JSON.parse(String(item.gyro_stats || "{}")) } catch (e) { st = null }
                                    if (!st || st.pitch === undefined) return "最大旋转：—"
                                    return "最大旋转：俯仰 " + Number(st.pitch).toFixed(2) + "°，偏航 " + Number(st.yaw).toFixed(2)
                                         + "°，横滚 " + Number(st.roll).toFixed(2) + "°"
                                         + "　·　最大缩放：" + Number(st.zoom).toFixed(1) + "%"
                                }
                            }

                            // ---------- 高级选项（对齐原版 AdvancedSection） ----------
                            AdvancedSection {
                                SliderField {
                                    label: "视场角"; prop: "fov"; from: 0.1; to: 3; decimals: 3
                                    hint: "1 = 原始视野；小于 1 裁切放大（更稳），大于 1 可能看到黑边"
                                }
                                Check { label: "卷帘快门校正"; prop: "frame_readout_enabled" }
                                SliderField {
                                    label: "帧读出时间"; prop: "frame_readout_time"; from: 0; to: 100; decimals: 2; unit: "毫秒"
                                    enabled: { win.refreshTick; return !!settings.frame_readout_enabled }
                                    hint: "传感器逐行读出的时间，可从镜头标定得到（DJI Action 4 约 11.09 毫秒）"
                                }
                                ComboV {
                                    label: "帧读出方向"; prop: "gyro_readout_dir"
                                    values: ["TopToBottom", "BottomToTop"]; labels: ["从上到下", "从下到上"]
                                    enabled: { win.refreshTick; return !!settings.frame_readout_enabled }
                                }
                                SliderField {
                                    label: "视频速度"; prop: "gyro_video_speed"; from: 10; to: 1000; decimals: 0; scale: 100; unit: "%"
                                    hint: "100% = 原速；50% = 慢放 2 倍（影响平滑/缩放的等效时间尺度）"
                                }
                                Check { label: "链接平滑"; prop: "gyro_vs_smoothing" }
                                Check { label: "链接缩放速度"; prop: "gyro_vs_zooming" }
                                Check { label: "链接缩放限额"; prop: "gyro_vs_zoom_limit" }
                                Combo {
                                    label: "缩放方式"; prop: "gyro_zoom_mode"
                                    items: ["无缩放", "动态缩放", "静态缩放"]
                                    // 联动：切到「动态缩放」时若速度还是 0（核心会当成关闭）就补默认 4 秒；
                                    // 切回「无缩放」时把中心偏移归零（与 gyroflow 一致）。
                                    onPicked: function (idx) {
                                        if (idx === 1 && Number(settings.gyro_zoom_window) < 0.1) settings.gyro_zoom_window = 4.0
                                        if (idx === 0) { settings.gyro_zoom_center_x = 0; settings.gyro_zoom_center_y = 0 }
                                        win.refreshFields()
                                    }
                                }
                                SliderField {
                                    label: "缩放中心偏移 X"; prop: "gyro_zoom_center_x"
                                    from: -100; to: 100; decimals: 0; scale: 100; unit: "%"
                                    enabled: { win.refreshTick; return Number(settings.gyro_zoom_mode) > 0 }
                                }
                                SliderField {
                                    label: "缩放中心偏移 Y"; prop: "gyro_zoom_center_y"
                                    from: -100; to: 100; decimals: 0; scale: 100; unit: "%"
                                    enabled: { win.refreshTick; return Number(settings.gyro_zoom_mode) > 0 }
                                }
                                SectionHeader { text: "附加 3D 旋转" }
                                SliderField { label: "俯仰"; prop: "gyro_rot_y"; from: -180; to: 180; decimals: 2; unit: "°" }
                                SliderField { label: "偏航"; prop: "gyro_rot_x"; from: -180; to: 180; decimals: 2; unit: "°" }
                                SliderField { label: "横滚"; prop: "gyro_rot_z"; from: -180; to: 180; decimals: 2; unit: "°" }
                                SectionHeader { text: "附加 3D 平移" }
                                SliderField { label: "X"; prop: "gyro_tr_x"; from: -1000; to: 1000; decimals: 0; unit: "像素" }
                                SliderField { label: "Y"; prop: "gyro_tr_y"; from: -1000; to: 1000; decimals: 0; unit: "像素" }
                                SliderField { label: "Z"; prop: "gyro_tr_z"; from: -1000; to: 1000; decimals: 0; unit: "像素" }
                                ISpin {
                                    label: "缩放限额迭代次数"; prop: "max_zoom_iterations"
                                    hint: "更多的迭代 = 更准确的限额数，但计算速度也更慢（原版默认 5）"
                                }
                                SliderField {
                                    label: "陀螺偏移"; prop: "gyro_offset"; from: -1000; to: 1000; decimals: 1; unit: "毫秒"
                                    hint: "陀螺与视频的时间差；DJI 内嵌同步数据约 0"
                                }
                                ISpin { label: "陀螺偏移帧数"; prop: "gyro_offset_frames"; hint: "非 0 时优先于上面的毫秒偏移" }
                            }
                        }
                    }
                }


                // ================= HUD =================
                ScrollView {
                    id: svHud
                    contentWidth: availableWidth
                    contentHeight: colHud.height
                    Column {
                        id: colHud
                        width: svHud.availableWidth
                        padding: 10
                        spacing: 10
                        GroupBox {
                            title: "HUD"
                            Check { label: "预览"; prop: "hud_preview" }
                            Field { label: "GPX 文件"; prop: "hud_gpx_path"; filter: "*.gpx" }
                            // 时间基准是「比赛出发时刻」：出发时刻在视频时间轴上的位置由「录像」
                            // 页的「录像-出发时间」自动换算；这一栏填的是**这条 GPX 自己**相对
                            // 出发时刻的偏移（出发后 8 秒才搜到 GPS → −8）。
                            DSpin {
                                label: "GPX 时间偏移（秒）"; prop: "hud_gpx_offset"
                                hint: "以比赛出发时刻为基准：出发后 8 秒手表才开始记录就填 −8（HUD 用的就是轨迹文件同一条记录时，填成与「按表-出发时间」相同的值）；视频那一段（出发前几秒开录）由「视频」页的「录像-出发时间」自动换算，这里不用重复"
                            }
                            HudEditor {id:hudEditor;width:parent.width;source:settings;renderer:item;enabled:!item.busy && !item.encoding}
                        }
                    }
                }

                // ================= 导出 =================
                ScrollView {
                    id: svEnc
                    contentWidth: availableWidth
                    contentHeight: colEnc.height
                    Column {
                        id: colEnc
                        width: svEnc.availableWidth
                        padding: 10
                        spacing: 10
                        GroupBox {
                            title: "编码"
                            ComboV { label: "编码器"; prop: "encode_backend"; values: ["software", "qsv", "nvenc", "amf", "videotoolbox"]; labels: ["CPU", "QSV", "NVENC", "AMF", "VideoToolbox"] }
                            ComboV { label: "格式"; prop: "encode_format"; values: ["h264", "hevc", "av1"]; labels: ["H264", "HEVC", "AV1"] }
                            ComboV { label: "码率控制"; prop: "encode_rate_control"; values: ["cbr", "vbr", "crf"]; labels: ["CBR恒定码率", "VBR可变码率", "CRF恒定质量"] }
                            DSpin { label: "质量"; prop: "encode_quality"; visible: { win.refreshTick; return String(settings.encode_rate_control) === "crf" } }
                            Field { label: "码率（Mbps）"; prop: "encode_bitrate"; browse: false; visible: { win.refreshTick; var rc = String(settings.encode_rate_control); return rc === "cbr" || rc === "vbr" } }
                            ComboV { label: "音频"; prop: "encode_audio_codec"; values: ["copy", "none", "aac", "mp3", "opus"]; labels: ["复制原声", "无音频", "aac", "mp3", "opus"] }
                        }
                        GroupBox {
                            title: "输出"
                            Check { label: "地图"; prop: "encode_include_map" }
                            Check { label: "视频"; prop: "encode_include_video" }
                            Check { label: "HUD"; prop: "encode_include_hud" }
                            ComboV {
                                label: "分段导出"; prop: "encode_seg_mode"
                                values: ["none", "all", "custom"]
                                labels: ["不分段", "逐段导出", "自定义"]
                                // 联动底层开关（落盘时由模式推导；这里同步是为了其它绑定立即刷新）
                                onPicked: function (idx) {
                                    settings.encode_seg_enabled = (idx !== 0)
                                    settings.encode_all_seg = (idx === 1)
                                    win.refreshFields()
                                }
                            }
                            RowLayout {
                                width: parent.width
                                spacing: 6
                                visible: { win.refreshTick; return String(settings.encode_seg_mode) === "custom" }
                                Label { text: "分段范围"; Layout.preferredWidth: 150; color: Theme.text; elide: Text.ElideRight; font.pixelSize: 13 }
                                TextField {
                                    Layout.fillWidth: true; Layout.preferredHeight: 28; font.pixelSize: 13
                                    placeholderText: "如 0-1"
                                    text: { win.refreshTick; return String(settings.encode_start_seg) + "-" + String(settings.encode_end_seg) }
                                    validator: RegularExpressionValidator { regularExpression: /^[0-9]{0,3}-[0-9]{0,3}$/ }
                                    onEditingFinished: win.setSegRange(text)
                                    HoverHandler { id: segRangeHover }
                                    ToolTip.visible: segRangeHover.hovered
                                    ToolTip.text: "起始-结束分段点编号（0 = 出发时刻；结束填 0 = 到视频末尾）"
                                    ToolTip.delay: 400
                                }
                            }
                            ComboV {
                                label: "分辨率"; prop: "encode_resolution"
                                // 「与原始视频一致」= 不缩放，直接用源视频尺寸（encode.rs 的 "source"）；
                                // 其余是显式尺寸：先 16:9（4K → 1080p → 720p），再 4:3（同样由大到小）。
                                values: ["source", "3840x2160", "1920x1080", "1280x720", "2880x2160", "1920x1440", "1440x1080"]
                                labels: ["与原始视频一致", "3840x2160 (4K 16:9)", "1920x1080 (1080p 16:9)", "1280x720 (720p 16:9)",
                                         "2880x2160 (4K 4:3)", "1920x1440 (1440p 4:3)", "1440x1080 (1080p 4:3)"]
                            }
                            ComboV { label: "帧率"; prop: "encode_fps"; values: ["source", "60", "30", "24"]; labels: ["与原始视频一致", "60 fps", "30 fps", "24 fps"] }
                            ComboV { label: "导出倍速"; prop: "encode_speed"; values: ["0.8", "1", "1.5", "2", "3", "4"]; labels: ["0.8x", "1x", "1.5x", "2x", "3x", "4x"] }
                            Field { label: "输出文件"; prop: "encode_output_path"; filter: "*.mp4"; saveMode: true }
                        }
                    }
                }
            }

            // ---------- footer actions ----------
            Row {
                id: footerActions
                Layout.fillWidth: true
                Layout.margins: 8
                spacing: 8
                TButton { text: "导出"; visible: tabs.currentIndex === 4; onClicked: win.startExport() }
                TButton { text: "添加到导出队列"; visible: tabs.currentIndex === 4; onClicked: win.addExportQueue() }
                TButton { text: "保存配置"; visible: tabs.currentIndex !== 4; onClicked: saveDialog.open() }
            }
        }
    }

    // ---------- composite preview ----------
    Rectangle {
        id: previewArea
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.right: settingsPanel.left
        anchors.bottom: parent.bottom
        color: Theme.dark ? "#101010" : "#111111"
        border.width: 1
        border.color: Theme.video_border
    }
    TraceReelItem {
        id: item
        property real outputAspect: {
            win.refreshTick
            var a = win.aspectOf(settings.encode_resolution)
            // 「与原始视频一致」：预览画布跟随源视频宽高比（item.source_size 由真实视频探测得到），
            // 否则预览会退回 16:9，地图/HUD 面板比例与真实导出的画布对不上。
            if (a <= 0 && String(settings.encode_resolution).toLowerCase() === "source") a = win.aspectOf(item.source_size)
            return a > 0 ? a : 16/9
        }
        width: Math.min(previewArea.width, Math.max(1,previewArea.height-48)*outputAspect)
        height: width/outputAspect
        x: (previewArea.width-width)/2
        y: (previewArea.height-48-height)/2
        settings_path: currentPath
        playing: false
    }
    HudCanvas {
        property var videoRect: {
            win.refreshTick; item.busy; item.apply_summary
            return JSON.parse(String(item.hud_viewport(item.width,item.height)))
        }
        x: item.x+videoRect.x; y: item.y+videoRect.y
        width: videoRect.width; height: videoRect.height
        editor: hudEditor
        visible: tabs.currentIndex === 3 && !!settings.hud_preview
        enabled: visible && hudEditor.enabled
    }

    // ---------- 拖放导入：把文件拖到预览区 ----------
    // 放在 item/HudCanvas 之后 = 最上层，拖放事件才落在这里（它不处理鼠标事件，不影响预览交互）。
    DropArea {
        id: dropZone
        anchors.fill: previewArea
        onDropped: function (drop) {
            if (!drop.hasUrls) return
            win.handleDrop(drop.urls)
            drop.acceptProposedAction()
        }
        Rectangle {
            anchors.fill: parent
            visible: dropZone.containsDrag
            color: "#401565c0"
            border.color: Theme.accent
            border.width: 2
            Text {
                anchors.centerIn: parent
                text: "松开即导入：" + "\n.trr 工程（兼容 .orv） / 视频 / 图片(地图) / .gpx / .gyroflow 预设 / .json 镜头标定"
                color: Theme.button
                font.pixelSize: 15
                horizontalAlignment: Text.AlignHCenter
            }
        }
    }

    // Loading overlay across the whole preview (apply / gyro load).
    Item {
        id: loadOverlay
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.right: settingsPanel.left
        anchors.bottom: parent.bottom
        visible: item.busy || item.gyro_loading
        // The apply phase has no progress signal (busy_msg only); the gyro load reports a
        // real 0..1 fraction. -1 means "unknown" → keep the animated indeterminate bar.
        readonly property bool indeterminate: item.busy || item.gyro_progress < 0 || item.gyro_progress > 1
        readonly property int pct: Math.round(Math.max(0, Math.min(1, item.gyro_progress)) * 100)
        readonly property string phase: item.gyro_phase.length > 0 ? item.gyro_phase : "读取运动数据"
        Rectangle {
            anchors.fill: parent
            color: "#b0404040"
        }
        Column {
            anchors.centerIn: parent
            spacing: 14
            BusyIndicator { width: 48; height: 48; running: loadOverlay.indeterminate }
            Label {
                text: item.busy
                      ? item.busy_msg
                      : (loadOverlay.indeterminate ? "正在加载防抖数据..." : loadOverlay.phase + " " + loadOverlay.pct + "%")
                color: "white"; font.pixelSize: 16; horizontalAlignment: Text.AlignHCenter
            }
            ProgressBar {
                width: 280
                // Real progress: the loader reports how much of the video container has been
                // scanned for motion data, then the post-passes (see gyro.rs phase bounds).
                indeterminate: loadOverlay.indeterminate
                from: 0; to: 1
                value: Math.max(0, Math.min(1, item.gyro_progress))
            }
        }
    }

    // Composite seek toolbar (bottom of the preview): icon play/pause + progress only.
    RowLayout {
        anchors.bottom: previewArea.bottom
        anchors.left: item.left
        anchors.right: item.right
        anchors.margins: 8
        spacing: 6
        TButton {
            id: playButton
            text: item.playing ? "\u23F8" : "\u25B6"
            onClicked: item.set_playing(!item.playing)
            Layout.preferredHeight: 28
            Layout.preferredWidth: 40
            font.pixelSize: 14
        }
        TSlider {
            id: seek
            Layout.fillWidth: true
            from: 0
            to: item.duration_ms
            // 拖动时在手柄上方显示目标时间（对齐原版 Slider 拖动时的数值气泡）
            ToolTip {
                parent: seek.handle
                visible: seek.pressed
                text: win.fmtTime(seek.value)
                bottomMargin: 6
            }
            // Do not restart decoding for every mouse movement. Submit only the final
            // position when the user releases the handle.
            onPressedChanged: {
                scrubbing = pressed
                if (!pressed)
                    item.seek(value)
            }
        }
        Label { id: timeLabel; text: "00:00 / 00:00"; color: "white"; Layout.alignment: Qt.AlignVCenter; Layout.preferredHeight: 28 }
        ComboBox {
            id: speedSelect
            model: ["0.8x", "1x", "1.5x", "2x", "3x", "4x"]
            currentIndex: 1
            Layout.preferredHeight: 28
            Layout.alignment: Qt.AlignVCenter
            // 选择后立即应用到当前播放（含音频保音调变速）。
            onActivated: {
                var rates = [0.8, 1.0, 1.5, 2.0, 3.0, 4.0]
                item.set_speed(rates[currentIndex])
            }
        }
    }

    // ---------- dialogs ----------
    FileDialog {
        id: openDialog
        title: "加载 TraceReel 设置"
        nameFilters: ["TraceReel 工程 (*.trr)", "旧版工程 (*.orv)", "所有文件 (*)"]
        onAccepted: loadProject(selectedFile)
    }
    FileDialog {
        id: saveDialog
        title: "保存 TraceReel 设置"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "trr"
        nameFilters: ["TraceReel 工程 (*.trr)"]
        onAccepted: saveProject(selectedFile)
    }
    FileDialog {
        id: browseDialog
        title: "选择文件"
        onAccepted: win.acceptBrowsed(win.filePath(selectedFile))
    }
    // Save-mode picker: lets the user type a NEW filename (used for the encode output path).
    FileDialog {
        id: browseSaveDialog
        title: "选择输出文件"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "mp4"
        nameFilters: ["MP4 视频 (*.mp4)", "所有文件 (*)"]
        onAccepted: win.acceptBrowsed(win.filePath(selectedFile))
    }
    ColorDialog {
        id: colorDialog
        onAccepted: { settings[colorTarget] = win.colorHex(selectedColor); win.applyChange(colorTarget) }
    }
    // ---------- encode progress window (port of EncodeWindow) ----------
    function fmtClock(s) {
        s = Number(s)
        if (!isFinite(s) || s < 0) s = 0
        s = Math.floor(s)
        var h = Math.floor(s / 3600); var m = Math.floor(s / 60) % 60; var ss = s % 60
        function p(n) { return (n < 10 ? "0" : "") + n }
        return p(h) + ":" + p(m) + ":" + p(ss)
    }
    function num(v) { v = Number(v); return isFinite(v) ? v : 0 }
    // 「宽x高」token → 宽高比；不是显式尺寸（如 "source"）时返回 0。
    function aspectOf(token) {
        var r = String(token).toLowerCase().split("x")
        return r.length === 2 && Number(r[0]) > 0 && Number(r[1]) > 0 ? Number(r[0]) / Number(r[1]) : 0
    }
    function fmtSize(b) {
        b = Number(b) || 0
        if (b < 1024) return b + " B"
        if (b < 1024 * 1024) return (b / 1024).toFixed(1) + " KB"
        if (b < 1024 * 1024 * 1024) return (b / (1024 * 1024)).toFixed(1) + " MB"
        return (b / (1024 * 1024 * 1024)).toFixed(2) + " GB"
    }
    function startExport() {
        var out = (settings.encode_output_path || "").trim()
        if (!out.length) { appendLog("ERROR", "请先设置输出视频文件路径（导出页）"); return }
        if (item.encoding) { appendLog("WARN", "已有编码任务在进行中"); encodeWin.visible = true; return }
        // Flush any pending edit + reload the in-memory settings SYNCHRONOUSLY so the export
        // uses the values just typed. Without this the debounced apply (~350 ms) may not have
        // run yet, so start_encode would read stale settings (the "wrong length until I click
        // elsewhere and re-export" bug).
        applyTimer.stop()
        var p = settings.temp_apply_path()
        if (p.length) item.apply_encode_only(p)
        encodeTotalWall = 0
        encodeWin.visible = true
        item.start_encode(out)
        appendLog("INFO", "开始编码: " + out)
    }
    property real encodeTotalWall: 0

    Window {
        id: encodeWin
        title: "TraceReel - 编码"
        width: 760
        height: 360
        visible: false
        color: Theme.background

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 12
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                spacing: 10
                ProgressBar {
                    Layout.fillWidth: true
                    from: 0; to: 1000
                    value: win.num(item.encode_total_frames) > 0 ? win.num(item.encode_frame) / win.num(item.encode_total_frames) * 1000 : 0
                }
                Label {
                    // 逐段导出时显示当前分段（普通导出时 encode_segment 为空，不占位）
                    visible: item.encode_segment.length > 0
                    text: item.encode_segment
                    color: Theme.accent; font.pixelSize: 12; font.bold: true
                }
                Label {
                    text: item.encoding ? (item.encode_paused ? "已暂停" : "编码中…")
                        : (item.encode_done ? (item.encode_aborted ? "已中止" : (item.encode_error.length ? "失败" : "完成")) : "待开始")
                    color: Theme.muted; font.pixelSize: 12
                    Layout.preferredWidth: 64
                    horizontalAlignment: Text.AlignRight
                }
            }

            // 本次实际使用的编码/解码组合（如「编码：中央处理器（AV1 · libsvtav1）｜解码：显卡硬解」）。
            // CPU 和 QSV 都靠显卡解码，任务管理器里 GPU 一直在动属正常，这行字说清楚。
            Label {
                Layout.fillWidth: true
                visible: item.encode_encoder_info.length > 0
                text: item.encode_encoder_info
                color: Theme.muted
                wrapMode: Text.Wrap
                font.pixelSize: 12
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 10

                component StatRow: RowLayout {
                    property string k
                    property string v
                    Layout.fillWidth: true
                    spacing: 8
                    Label { text: k; color: Theme.muted; font.pixelSize: 12 }
                    Item { Layout.fillWidth: true }
                    Label { text: v; color: "#222"; font.pixelSize: 12 }
                }
                GroupBox {
                    title: "时间"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 0
                    Layout.minimumWidth: 0
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        StatRow { k: "已用:"; v: win.fmtClock(item.encode_elapsed_s) }
                        StatRow { k: "剩余:"; v: item.encoding ? win.fmtClock(item.encode_remaining_s) : "--:--:--" }
                        StatRow { k: "总用时:"; v: win.fmtClock(win.encodeTotalWall) }
                        StatRow { k: "视频当前:"; v: win.fmtClock(win.num(item.encode_current_t_ms) / 1000) }
                        StatRow { k: "视频总长:"; v: win.fmtClock(win.num(item.encode_duration_ms) / 1000) }
                    }
                }
                GroupBox {
                    title: "帧"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 0
                    Layout.minimumWidth: 0
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        StatRow { k: "当前帧:"; v: "" + win.num(item.encode_frame) }
                        StatRow { k: "总帧数:"; v: "" + win.num(item.encode_total_frames) }
                        StatRow { k: "帧/秒:"; v: win.num(item.encode_fps).toFixed(1) }
                    }
                }
                GroupBox {
                    title: "大小"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 0
                    Layout.minimumWidth: 0
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        StatRow { k: "当前大小:"; v: win.fmtSize(item.encode_out_size) }
                        StatRow { k: "总大小:"; v: item.encode_done ? win.fmtSize(item.encode_out_size) : "--" }
                    }
                }
            }

            Label {
                Layout.fillWidth: true
                visible: item.encode_done && item.encode_error.length > 0
                text: item.encode_error
                color: item.encode_aborted ? "#a86e00" : "#b00020"
                wrapMode: Text.Wrap
                font.pixelSize: 12
            }

            Label {
                Layout.fillWidth: true
                visible: item.encode_warning.length > 0
                text: item.encode_warning
                color: "#b07800"
                wrapMode: Text.Wrap
            }

            // 非致命提示（例如「显卡不支持该编码格式，已自动改用 CPU 重新导出」）。
            // 导出仍然成功，所以用警告色而不是错误色。
            Label {
                Layout.fillWidth: true
                visible: item.encode_notice.length > 0
                text: "⚠ " + item.encode_notice
                color: "#b07800"
                wrapMode: Text.Wrap
                font.pixelSize: 12
            }

            // Three action buttons, evenly stretched to fill the full width.
            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                TButton {
                    id: btnPause
                    text: item.encode_paused ? "继续" : "暂停"
                    enabled: item.encoding
                    Layout.fillWidth: true
                    onClicked: item.pause_encode(!item.encode_paused)
                }
                TButton {
                    id: btnStop
                    text: item.encoding ? "中止" : "关闭"
                    Layout.fillWidth: true
                    onClicked: {
                        if (item.encoding) { item.abort_encode(); appendLog("WARN", "正在中止编码…") }
                        else { encodeWin.visible = false }
                    }
                }
                TButton {
                    id: btnOpen
                    text: "打开"
                    enabled: item.encode_done && item.encode_out_size > 0 && item.encode_output.length > 0
                    Layout.fillWidth: true
                    onClicked: Qt.openUrlExternally("file:///" + item.encode_output.replace(/\\/g, "/"))
                }
            }
        }

        // Drive the wall-clock total while encoding (see the 16ms playback timer).
        // Closing the window mid-encode aborts it (mirrors the PyQt closeEvent).
        onVisibleChanged: if (!visible && item.encoding) item.abort_encode()
    }
    // 弹窗居中：ApplicationWindow 本身是 Window 而不是 Item，`Overlay.overlay`
    // 只对 Item 生效（在 Window 上取值恒为 null），popup 的 parent 于是为 null ——
    // Qt 兜底把父项设成 contentItem，但**不会**自动居中，弹窗就停在窗口左上角 (0,0)。
    // 所以这里显式指定 parent 为 contentItem，再用 anchors.centerIn 居中。
    // 内容留白走 Dialog 自己的 padding，**不要**给内容 Column 加 anchors.fill + anchors.margins：
    // Dialog 的高度预算正好等于内容 Column 的 implicitHeight，margins 又把 Column 压小
    // （每边 12 像素），内容就会越过弹窗底边被裁掉——按钮只剩上半截就是这个原因。
    Dialog {
        id: aboutDialog
        objectName: "aboutDialog"
        title: "关于 TraceReel · 迹映"
        parent: win.contentItem
        anchors.centerIn: parent
        width: 380
        modal: true
        padding: 12
        Column {
            spacing: 8
            Row {
                spacing: 12
                Image {
                    objectName: "aboutBrandMark"
                    source: "qrc:/brand/mark.png"
                    width: 76; height: 76
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 6
                    Label { text: "TraceReel · 迹映"; font.bold: true; font.pixelSize: 22; color: Theme.text }
                    Label { text: "定向视频 · 地图 · 轨迹"; color: Theme.muted }
                }
            }
            Label { text: "Qt6 + qmetaobject + PaintedItem 渲染\n带 Gyroflow 运动防抖与 HUD 功能。"; wrapMode: Text.Wrap; width: 340 }
            TButton { text: "确定"; onClicked: aboutDialog.close() }
        }
    }

    // .qrt 工程文件带分段数据时的确认框。
    // 「导入分段」把 QuickRoute 的 lap 时间写进 settings.split_times（绝对时间），
    // 路径本身已经落地，所以地图 + 轨迹立刻可用。
    Dialog {
        id: qrtSplitDialog
        title: "检测到分段数据"
        parent: win.contentItem
        anchors.centerIn: parent
        width: 400
        modal: true
        padding: 12
        Column {
            spacing: 10
            Label {
                text: "这个 .qrt 工程里带了分段数据（QuickRoute 的 lap 时间）。"
                wrapMode: Text.Wrap
                width: 360
            }
            Label {
                text: "地图和轨迹会立刻导入。是否同时导入分段？"
                wrapMode: Text.Wrap
                width: 360
                color: "#555"
            }
            Row {
                spacing: 8
                TButton {
                    text: "导入分段"
                    onClicked: {
                        // `trackRows`（文档级 id）而非组件内的 `trackList`。
                        var path = trackRows.pendingQrtPath
                        appendLog("INFO", "读取 .qrt 分段: " + path)
                        var encoded = item.qrt_split_times(path)
                        var count = encoded.length ? encoded.split("|").length : 0
                        if (count > 0) {
                            settings["split_times"] = encoded
                            settings["split_point_codes"] = ""
                            // QuickRoute laps are absolute times measured from the
                            // start of the recording, so pin the time type no matter
                            // what the page was set to before.
                            settings["split_is_relative"] = false
                            win.applyChange("split_times", "分段时间 = 从 .qrt 导入的 " + count + " 个分段")
                            win.applyChange("split_is_relative", "时间类型 = 绝对时间")
                            win.refreshFields()
                            appendLog("INFO", "已从 .qrt 导入 " + count + " 个分段: " + encoded)
                        } else {
                            appendLog("WARN", "该 .qrt 未解析出可用分段，仅导入地图与轨迹")
                        }
                        qrtSplitDialog.close()
                        // 拖入的路径已经落地，只有从轨迹列表行进来时才需要回写。
                        if (trackRows.pendingQrtIndex >= 0) trackRows.writeBack()
                    }
                }
                TButton {
                    text: "仅地图 + 轨迹"
                    onClicked: {
                        appendLog("INFO", "仅导入地图与轨迹: " + trackRows.pendingQrtPath)
                        qrtSplitDialog.close()
                        if (trackRows.pendingQrtIndex >= 0) trackRows.writeBack()
                    }
                }            }
        }
    }

    // 按分段设置换图时间：列出每一段的起点时刻，点一下就填进那一行的「换图时间」。
    // 第 N 段的起点 = 第 N-1 个分段的结束时刻（第 1 段起点恒为 0:00）。
    Dialog {
        id: mapSplitDialog
        title: "按分段设置换图时间"
        parent: win.contentItem
        anchors.centerIn: parent
        width: 340
        height: 420
        modal: true
        property var starts: []
        onOpened: {
            var raw = String(settings.split_starts() || "")
            starts = raw.length ? raw.split("|") : []
        }
        Column {
            anchors.fill: parent
            anchors.margins: 12
            spacing: 8
            Label {
                width: parent.width
                wrapMode: Text.Wrap
                color: Theme.text
                text: mapSplitDialog.starts.length
                    ? "选择换图时刻：第 N 段的起点就是第 N-1 个分段的结束时刻。"
                    : "还没有分段时间。先在「分段时间」里添加分段，或拖入带分段的 .qrt。"
            }
            Flickable {
                width: parent.width
                height: parent.height - 60
                clip: true
                contentHeight: splitStartColumn.height
                Column {
                    id: splitStartColumn
                    width: parent.width
                    spacing: 4
                    Repeater {
                        model: mapSplitDialog.starts
                        delegate: TButton {
                            width: splitStartColumn.width
                            height: 30 * Theme.dpi_scale
                            text: "第 " + (index + 1) + " 段起点  ·  " + modelData
                            onClicked: {
                                trackRows.setRowTime(trackRows.pendingMapRow, modelData)
                                mapSplitDialog.close()
                            }
                        }
                    }
                }
            }
        }
    }

    // ---------- debounce for auto-apply ----------
    Timer {
        id: applyTimer
        interval: 350
        repeat: false
        onTriggered: { if (pendingProp.length) { doApply(pendingProp, pendingNote); pendingProp = ""; pendingNote = "" } }
    }

    // ---------- playback timer (also drives the seek bar + time label) ----------
    Timer {
        interval: 16
        repeat: true
        running: true
        onTriggered: {
            item.tick()
            seek.to = item.duration_ms
            // 拖动进度条时显示拖动位置的时间（否则显示当前播放时间）
            timeLabel.text = (scrubbing ? fmtTime(seek.value) : fmtTime(item.t_ms)) + " / " + fmtTime(item.duration_ms)
            playButton.text = item.playing ? "\u23F8" : "\u25B6"
            gyroStatusText = item.gyro_status
            if (item.encoding && !item.encode_paused) win.encodeTotalWall += 0.016
            if (!scrubbing) seek.value = item.t_ms
        }
    }
}
