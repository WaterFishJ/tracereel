import QtQuick 2.15
import QtQuick.Controls 2.15
import "../src/bin"

ApplicationWindow {
    id: window
    visible: true
    width: 1200; height: 900
    palette.window: "#202020"
    palette.windowText: "#eeeeee"
    palette.text: "#eeeeee"
    palette.base: "#303030"
    palette.button: "#404040"
    palette.buttonText: "#eeeeee"
    ReceiptImport {
        id: importer
        parent: Overlay.overlay
    }
    property int elapsed: 0
    property bool prepared: false
    function debugState() { return JSON.stringify({elapsed:elapsed, rows:importer.rows.length, message:importer.message, output:importer.output}) }
    function cellText(item, name) {
        if (item.objectName === name) return item.text
        var children=item.children || []
        for (var i=0; i<children.length; i++) {
            var result=cellText(children[i],name)
            if (result !== undefined) return result
        }
        return undefined
    }
    function checkDuplicateActions() {
        var original = importer.rows
        function row(i,code,time) { return {cells:[String(i),code,time,time], scores:[1,1,1,1],include:true,mergedInto:-1} }
        importer.rows=[row(1,"50","0:01"),row(2,"50","0:02"),row(3,"50","0:03"),row(4,"120","0:20")]
        if (importer.suspectedCount !== 3) return false
        importer.mergeRow(1,-1)
        if (cellText(importer.contentItem,"receiptCell0_2") !== "0:02") return false
        importer.mergeRow(1,-1)
        if (importer.rows.length !== 2 || importer.rows[0].cells[2] !== "0:03"
                || importer.output.times.join("|") !== "0:03|0:20" || importer.suspectedCount !== 0) return false
        importer.undoMerge()
        if (importer.rows.length !== 3 || importer.rows[0].cells[2] !== "0:02" || importer.suspectedCount !== 2) return false
        if (cellText(importer.contentItem,"receiptCell0_2") !== "0:02") return false
        importer.rows=[row(1,"50","0:01"),row(2,"50","0:02"),row(3,"50","0:03"),row(4,"120","0:20")]
        importer.timeKindIndex=1
        importer.mergeRow(0,1)
        importer.mergeRow(0,1)
        if (importer.rows.length !== 2 || importer.rows[0].cells[2] !== "0:06"
                || importer.output.times.join("|") !== "0:06|0:20" || importer.output.finish !== "0:26") return false
        importer.codeColumnIndex=0
        importer.mergeRow(1,-1)
        if (importer.rows.length !== 1 || importer.rows[0].cells[2] !== "0:26") return false
        importer.undoMerge()
        if (importer.rows.length !== 2 || importer.rows[0].cells[2] !== "0:06") return false
        importer.codeColumnIndex=2
        importer.rows=original; importer.timeKindIndex=0; importer.undoRows=[]; importer.message="重复打卡提示、向上／向下合并和撤销检查通过。"
        return true
    }
    Timer {
        interval: 200; repeat: true; running: true
        onTriggered: {
            window.elapsed += 200
            if (!window.prepared) {
                window.prepared = true
                importer.selectImage(ocrTestImage)
                importer.corners = [[.335,.485],[.676,.483],[.639,.863],[.335,.85]]
                importer.open()
                importer.recognizeImage()
                return
            }
            if (window.elapsed > 30000) { console.error("RECEIPT_TEST_FAIL timeout: " + importer.message); Qt.quit() }
            if (!importer.recognizing && importer.rows.length) {
                if (importer.rows[0].include) {
                    importer.timeColumnIndex = 3; importer.codeColumnIndex = 2; importer.ordinalColumnIndex = 1
                    importer.rows[0].include = false; importer.changedRows()
                    return
                }
                stop()
                if (importer.output.error || importer.output.times.length !== 19 || importer.output.times[18] !== "22:03.1"
                        || importer.output.times[0] !== "1:21" || importer.suspectedCount !== 0
                        || importer.output.codes[11] !== "120" || importer.output.codes[17] !== "120"
                        || importer.rows[13].cells[1] !== "120" || importer.rows[19].cells[1] !== "120") {
                    console.error("RECEIPT_TEST_FAIL " + JSON.stringify(importer.output))
                    Qt.quit(); return
                }
                console.log("RECEIPT_TEST_PASS: actual OCR -> 4 columns, 19 splits, repeated control 120 retained, finish 1323.1")
                if (!window.checkDuplicateActions()) {
                    console.error("RECEIPT_TEST_FAIL duplicate merge controls"); Qt.quit(); return
                }
                importer.contentItem.grabToImage(function(result) {
                    result.saveToFile("target/receipt-import-preview.png")
                    Qt.quit()
                })
            }
        }
    }
}
