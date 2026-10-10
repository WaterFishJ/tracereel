import QtQuick
import QtQuick.Controls
import QtTest

Item {
    width: 600; height: 500
    SplitHarness { id: host; anchors.fill: parent }
    TestCase {
        name: "SplitInput"
        when: windowShown
        function initTestCase() { wait(1000) }
        function init() { host.times = ""; host.codes = ""; host.list.syncFromSettings() }
        function input(index) {
            var rows = []
            for (var i = 0; i < host.list.children.length; ++i) {
                var child = host.list.children[i]
                if (typeof child.focusTime === "function") rows.push(child)
            }
            return findChild(rows[index], "splitTimeInput")
        }
        function typeTime(index, text) {
            var field = input(index)
            field.forceActiveFocus()
            field.selectAll()
            for (var i = 0; i < text.length; ++i) keyClick(text[i])
        }
        function test_typing_appends_and_enter_focuses() {
            compare(host.list.rowCount(), 1)
            typeTime(0, "1:23")
            compare(host.list.rowCount(), 2)
            compare(host.times, "1:23")
            verify(input(0).activeFocus)
            keyClick(Qt.Key_Return)
            verify(input(1).activeFocus)
            typeTime(1, "45")
            compare(host.list.rowCount(), 3)
            compare(host.times, "1:23|45")
            keyClick(Qt.Key_Enter)
            verify(input(2).activeFocus)
        }
        function test_clear_time_removes_automatic_tail() {
            typeTime(0, "12")
            keyClick(Qt.Key_Return)
            typeTime(1, "34")
            input(1).selectAll()
            keyClick(Qt.Key_Backspace)
            compare(host.list.rowCount(), 2)
            compare(host.times, "12")
            verify(input(1).activeFocus)
            typeTime(1, "56")
            compare(host.list.rowCount(), 3)
            compare(host.times, "12|56")
        }
        function test_clear_middle_keeps_later_times() {
            host.times = "12|34|56"
            host.list.syncFromSettings()
            input(1).forceActiveFocus()
            input(1).selectAll()
            keyClick(Qt.Key_Delete)
            compare(host.list.rowCount(), 4)
            compare(input(2).text, "56")
            compare(host.times, "12|56")
            verify(input(1).activeFocus)
        }
        function test_load_delete_and_empty_enter() {
            host.times = "12|34"
            host.refreshTick++
            compare(host.list.rowCount(), 3)
            compare(input(2).text, "")
            host.list.removeRow(1)
            compare(host.list.rowCount(), 2)
            compare(host.times, "12")
            host.list.removeRow(0)
            compare(host.list.rowCount(), 1)
            compare(host.times, "")
            host.list.appendRow()
            keyClick(Qt.Key_Return)
            compare(host.list.rowCount(), 1)
            verify(input(0).activeFocus)
            host.list.removeRow(0)
            compare(host.list.rowCount(), 1)
        }
        function test_whitespace_is_empty() {
            typeTime(0, " ")
            compare(host.list.rowCount(), 1)
            compare(host.times, "")
            typeTime(0, "12")
            compare(host.list.rowCount(), 2)
        }
        function test_optional_codes_remain_aligned_after_clear_delete_and_reload() {
            host.times = "1:23|1:25|2:00"
            host.codes = "120||120"
            host.list.syncFromSettings()
            compare(host.list.encodedCodes(), "120||120")
            host.list.setPointCode(1,"50")
            compare(host.codes,"120|50|120")
            host.list.setRowValue(1,"")
            compare(host.codes,"120|120")
            host.list.syncFromSettings()
            compare(host.list.encoded(),"1:23|2:00")
            compare(host.list.encodedCodes(),"120|120")
            host.list.removeRow(0)
            compare(host.times,"2:00")
            compare(host.codes,"120")
        }
        function test_code_can_be_entered_before_time() {
            host.list.setPointCode(0,"120")
            compare(host.list.rowCount(),2)
            compare(host.times,"")
            host.list.setRowValue(0,"1:23")
            compare(host.codes,"120")
            host.list.syncFromSettings()
            compare(host.list.encodedCodes(),"120")
        }
    }
}
