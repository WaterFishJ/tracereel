import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import QtQuick.Dialogs
import TraceReel 1.0
import "ReceiptImport.js" as Logic

Dialog {
    id: dialog
    title: "成绩条图片导入（本地 OCR）"
    modal: true
    width: Math.min(parent.width - 40, 1320)
    height: Math.min(parent.height - 40, 900)
    anchors.centerIn: parent
    closePolicy: Popup.NoAutoClose
    signal imported(string times, string codes, bool relative)
    property string imageUrl: ""
    property string imagePath: ""
    property var corners: [[0.05, 0.05], [0.95, 0.05], [0.95, 0.95], [0.05, 0.95]]
    property var rows: []
    property int columns: 0
    property string correctedUrl: ""
    property string message: ""
    property color warningColor: "#e79832"
    property var undoRows: []
    property alias timeColumnIndex: timeColumn.currentIndex
    property alias codeColumnIndex: codeColumn.currentIndex
    property alias ordinalColumnIndex: ordinalColumn.currentIndex
    property alias timeKindIndex: timeKind.currentIndex
    property alias recognizing: engine.busy
    property var output: Logic.build(rows, timeColumn.currentIndex - 1, timeKind.currentIndex === 1, ordinalColumn.currentIndex - 1, codeColumn.currentIndex - 1)
    property var suspected: Logic.suspects(rows, codeColumn.currentIndex - 1)
    property int suspectedCount: suspected.filter(function(value) { return value }).length
    /// 预览的旋转角（度，正值＝顺时针）：只是把照片转过来方便看清、框准，四角仍然
    /// 量在原图上，所以转多少都不改变识别结果，也不用重新识别。
    property real fineAngle: 0
    readonly property bool showingCorrected: showCorrected.checked && dialog.correctedUrl.length > 0
    /// 预览要转的角度：转的是源图照片；校正图已经是摆正的样子，不用再转。
    readonly property real previewRotation: dialog.showingCorrected ? 0 : dialog.fineAngle
    function setFineAngle(value) {
        var v = Math.round(Math.max(-180, Math.min(180, value)) * 10) / 10
        if (dialog.fineAngle !== v) dialog.fineAngle = v
        if (!angleField.activeFocus) angleField.text = v.toFixed(1)
    }
    function columnNames(optional) {
        var names = [optional ? "不指定" : "请选择"]
        for (var i = 0; i < columns; i++) names.push("第 " + (i + 1) + " 列")
        return names
    }
    function changedRows() { rows = rows.slice() }
    function mergeRow(index, direction) {
        var snapshot = JSON.parse(JSON.stringify(rows))
        var error = Logic.mergeRow(rows,index,direction,timeColumn.currentIndex-1,timeKind.currentIndex === 1)
        if (error.length) { message=error; return }
        undoRows = snapshot
        changedRows()
        message = "已将第 " + (index+1) + " 行合并到" + (direction<0 ? "上" : "下") + "方并删除原行，所选时间列已更新。"
    }
    function undoMerge() {
        if (!undoRows.length) return
        rows=undoRows; undoRows=[]
        message="已撤销最近一次合并。"
    }
    function recognizeImage() {
        rows = []; undoRows=[]; correctedUrl = ""; message = "正在校正图片并识别…"
        engine.start(dialog.requestJson())
    }
    /// 送给引擎的请求。单拎出来是为了能在测试里核对四角真的传出去了 ——
    /// 角度只影响预览，不参与识别，所以这里没有它。
    function requestJson() {
        return JSON.stringify({path:imagePath, corners:corners})
    }
    function selectImage(url) {
        imageUrl = String(url)
        imagePath = decodeURIComponent(imageUrl.replace(/^file:\/\//, ""))
        if (/^\/[A-Za-z]:/.test(imagePath)) imagePath = imagePath.substring(1)
        rows = []; undoRows=[]; columns = 0; correctedUrl = ""; message = ""
        corners = [[0.05,0.05],[0.95,0.05],[0.95,0.95],[0.05,0.95]]
        dialog.setFineAngle(0)
    }
    ReceiptOcr {
        id: engine
        onChanged: {
            if (busy) return
            if (error.length) { dialog.message = error; return }
            if (!result.length) return
            var data = JSON.parse(result)
            dialog.columns = data.columns
            dialog.rows = data.rows.map(function(r) { return {cells:r.cells, scores:r.scores, include:true} })
            dialog.undoRows = []
            dialog.correctedUrl = data.preview
            dialog.message = data.rows.length ? "请核对列和行，取消勾选表头、起点等无关行；低置信度文字以橙色显示。" : "未识别出文字，请调整四角后重试。"
            timeColumn.currentIndex = 0; codeColumn.currentIndex = 0; ordinalColumn.currentIndex = 0
        }
    }
    Timer { interval: 100; repeat: true; running: engine.busy; onTriggered: engine.poll() }
    FileDialog {
        id: picker
        title: "选择成绩条照片"
        nameFilters: ["图片 (*.jpg *.jpeg *.png *.bmp)"]
        onAccepted: dialog.selectImage(selectedFile)
    }
    contentItem: ColumnLayout {
        spacing: 8
        RowLayout {
            Button { text: "选择图片"; enabled: !engine.busy; onClicked: picker.open() }
            CheckBox { id: showCorrected; text: "查看校正图"; enabled: dialog.correctedUrl.length > 0; checked: false }
            Button {
                text: engine.busy ? "正在本地识别…" : "校正并识别"
                enabled: dialog.imagePath.length > 0 && !engine.busy
                onClicked: dialog.recognizeImage()
            }
            BusyIndicator { running: engine.busy; visible: running; Layout.preferredWidth: 28; Layout.preferredHeight: 28 }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "旋转角" }
            TextField {
                id: angleField
                objectName: "receiptAngleField"
                Layout.preferredWidth: 68
                enabled: !engine.busy
                text: dialog.fineAngle.toFixed(1)
                // 编辑时不要抢用户的输入，但焦点一离开就按真实角度回填，
                // 免得点「归零」这类按钮时框里还留着旧数字。
                onActiveFocusChanged: if (!activeFocus) text = dialog.fineAngle.toFixed(1)
                onEditingFinished: dialog.setFineAngle(parseFloat(text) || 0)
            }
            Label { text: "°" }
            // 不加 flat：扁平按钮在深色主题下边框和文字都看不出来，用户只看见一片空。
            // 也不用 fineAngle 去禁用：0° 时就该能按（按了没变化），禁用反而让按钮消失。
            Button { text: "归零"; enabled: !engine.busy; onClicked: dialog.setFineAngle(0) }
            Label {
                text: "拖预览图上方圆钮把照片转正（转的时候出网格标，把表格的行对到横线上），再拖四角框住成绩表"
                color: palette.mid
                elide: Text.ElideRight
            }
            Item { Layout.fillWidth: true }
        }
        Label { text: "请使用清晰照片。先拖预览图上方的圆钮把照片转正（网格标对着表格的行），再拖动四角框住成绩表：①左上 ②右上 ③右下 ④左下，然后点「校正并识别」。"; wrapMode: Text.Wrap; Layout.fillWidth: true }
        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            SplitView {
                orientation: Qt.Vertical
                // 图片是主要操作区：给它比表格更大的一块，四角/旋转才拖得准。
                SplitView.preferredWidth: Math.max(420, dialog.width * 0.56)
                SplitView.minimumWidth: 340
                ReceiptImageView {
                    id: imageView
                    SplitView.fillHeight: true
                    SplitView.minimumHeight: 200
                    source: dialog.showingCorrected ? dialog.correctedUrl : dialog.imageUrl
                    corners: dialog.showingCorrected ? [] : dialog.corners
                    editable: !engine.busy
                    previewRotation: dialog.previewRotation
                    rotatable: !dialog.showingCorrected && dialog.imageUrl.length > 0 && !engine.busy
                    onAngleDragged: function(angle) { dialog.setFineAngle(angle) }
                    onCornerMoved: function(index, canvasX, canvasY) {
                        var updated = dialog.corners.slice()
                        updated[index] = [canvasX, canvasY]
                        dialog.corners = updated
                    }
                }
                GroupBox {
                    title: "导入预览"
                    visible: dialog.rows.length > 0
                    SplitView.preferredHeight: 170
                    SplitView.minimumHeight: 90
                    ColumnLayout {
                        anchors.fill: parent
                        Label {
                            text: dialog.output.error || ("共 " + dialog.output.times.length + " 段 · 终点 " + dialog.output.finish)
                            wrapMode: Text.Wrap
                            Layout.fillWidth: true
                        }
                        ScrollView {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            TextArea {
                                readOnly: true
                                text: (dialog.output.display || []).join("\n")
                                wrapMode: Text.Wrap
                            }
                        }
                    }
                }
            }
            ColumnLayout {
                SplitView.fillWidth: true
                SplitView.minimumWidth: 340
                RowLayout {
                    Label { text: "时间列" }
                    ComboBox { id: timeColumn; model: dialog.columnNames(false); Layout.fillWidth: true }
                    ComboBox { id: timeKind; model: ["累计用时", "分段用时"] }
                }
                RowLayout {
                    Label { text: "分段号" }
                    ComboBox { id: ordinalColumn; model: dialog.columnNames(true); Layout.fillWidth: true }
                    Label { text: "点号" }
                    ComboBox { id: codeColumn; model: dialog.columnNames(true); Layout.fillWidth: true }
                }
                Label {
                    visible: dialog.suspectedCount > 0
                    text: dialog.suspectedCount + " 行疑似重复打卡"
                    color: dialog.warningColor
                    wrapMode: Text.Wrap; Layout.fillWidth: true
                }
                ScrollView {
                    Layout.fillHeight: true; Layout.fillWidth: true
                    contentWidth: grid.width
                    Column {
                        id: grid
                        spacing: 3
                        Row {
                            spacing: 4
                            Label { text: "行 / 导入"; width: 60 }
                            Repeater { model: dialog.columns; Label { text: "第 " + (index+1) + " 列"; width: 96 } }
                            Label { text: "合并相邻行"; width: 190 }
                        }
                        Repeater {
                            model: dialog.rows.length
                            Row {
                                id: gridRow
                                property int rowIndex: index
                                property var record: dialog.rows[rowIndex] || {cells:[], scores:[], include:false}
                                spacing: 4
                                CheckBox {
                                    width: 60; text: String(gridRow.rowIndex+1)
                                    checked: gridRow.record.include
                                    onToggled: { gridRow.record.include=checked; dialog.changedRows() }
                                }
                                Repeater {
                                    model: dialog.columns
                                    TextField {
                                        objectName: "receiptCell" + gridRow.rowIndex + "_" + index
                                        width: 96
                                        text: gridRow.record.cells[index] || ""
                                        color: gridRow.record.scores[index] < 0.85 ? dialog.warningColor : palette.text
                                        background: Rectangle {
                                            color: dialog.suspected[gridRow.rowIndex] ? Qt.tint(palette.base, Qt.rgba(dialog.warningColor.r, dialog.warningColor.g, dialog.warningColor.b, 0.21)) : palette.base
                                            border.color: dialog.suspected[gridRow.rowIndex] ? dialog.warningColor : palette.mid
                                            border.width: 1; radius: 3
                                        }
                                        onTextEdited: { gridRow.record.cells[index]=text; dialog.changedRows() }
                                    }
                                }
                                Button {
                                    objectName: "receiptMergeUp" + gridRow.rowIndex
                                    text: "↑合并"; width: 64
                                    enabled: timeColumn.currentIndex > 0 && Logic.neighbor(dialog.rows,gridRow.rowIndex,-1) >= 0
                                    onClicked: dialog.mergeRow(gridRow.rowIndex,-1)
                                    ToolTip.visible: hovered
                                    ToolTip.text: "并入上行并删除当前行：分段用时相加，累计用时取当前行时刻"
                                }
                                Button {
                                    objectName: "receiptMergeDown" + gridRow.rowIndex
                                    text: "↓合并"; width: 64
                                    enabled: timeColumn.currentIndex > 0 && Logic.neighbor(dialog.rows,gridRow.rowIndex,1) >= 0
                                    onClicked: dialog.mergeRow(gridRow.rowIndex,1)
                                    ToolTip.visible: hovered
                                    ToolTip.text: "并入下行并删除当前行：分段用时相加，累计用时取下行时刻"
                                }
                                Label {
                                    width: 110
                                    text: dialog.suspected[gridRow.rowIndex] ? "疑似重复打卡" : ""
                                    color: dialog.warningColor
                                }
                            }
                        }
                    }
                }
                RowLayout {
                    Button { text: "撤销合并"; enabled: dialog.undoRows.length>0; onClicked: dialog.undoMerge() }
                }
            }
        }
        Label { text: dialog.message; wrapMode: Text.Wrap; Layout.fillWidth: true }
        RowLayout {
            Layout.alignment: Qt.AlignRight
            Button { text: "关闭"; enabled: !engine.busy; onClicked: dialog.close() }
            Button {
                text: "确认替换分段列表"; enabled: !engine.busy && !dialog.output.error && dialog.output.times.length > 0
                onClicked: {
                    var codes = dialog.output.codes.some(function(code) { return code.length }) ? dialog.output.codes.join("|") : ""
                    dialog.imported(dialog.output.times.join("|"),codes,dialog.output.relative); dialog.close()
                }
            }
        }
    }
}
