import QtQuick
import QtTest

// .qrt 分段导入的决策测试。
// 被测函数（hasSplitTimes / adoptQrtSplits / offerQrtSplits）是从 app/src/bin/gui.qml 里
// **原样切出来**的那一段（见 test_qrt_splits.ps1），settings / trackRows / qrtSplitDialog /
// item 在 harness 里用桩替代。
//
// 用户要求：「使用 qrt 文件导入时，如果 qrt 文件有分段信息，并且当前没有填写分段信息，
// 则直接导入，不用弹窗确认。」——所以这里盯两条：没有分段时必须直接落地、不弹窗；
// 已经有分段时仍然要问，不能默默覆盖。
Item {
    width: 400; height: 300
    QrtSplitsHarness { id: host; anchors.fill: parent }

    TestCase {
        name: "QrtSplits"
        when: windowShown

        function init() {
            host.settings.split_times = ""
            host.settings.split_point_codes = "1|2"
            host.settings.split_is_relative = true
            host.rows.pendingQrtIndex = -9
            host.rows.pendingQrtPath = ""
            host.rows.writeBacks = 0
            host.dlg.opens = 0
            host.preview.hasSplits = true
            host.preview.encoded = "0:01:00|0:02:00"
            host.applied = []
            host.logs = []
        }

        // 没有分段：直接导入，不弹窗。
        function test_imports_directly_when_no_splits_yet() {
            host.offerQrtSplits("C:/m.qrt", 0)
            compare(host.dlg.opens, 0, "没有分段时不该弹窗")
            compare(host.settings.split_times, "0:01:00|0:02:00")
            compare(host.settings.split_point_codes, "", "点号要清空（.qrt 里没有点号）")
            compare(host.settings.split_is_relative, false, "QuickRoute 的 lap 是绝对时间")
            compare(host.rows.writeBacks, 1, "列表行进来时要回写路径")
            compare(host.applied.length, 2, "分段时间与时间类型都要应用")
        }

        // 已经有分段：仍然弹窗问，不能覆盖。
        function test_asks_before_overwriting_existing_splits() {
            host.settings.split_times = "0:00:30"
            host.offerQrtSplits("D:/m.qrt", 2)
            compare(host.dlg.opens, 1)
            compare(host.rows.pendingQrtIndex, 2)
            compare(host.rows.pendingQrtPath, "D:/m.qrt")
            compare(host.settings.split_times, "0:00:30", "确认前不动已有分段")
            compare(host.rows.writeBacks, 0, "等用户确认后再回写")
        }

        // 拖放进来的路径已经在模型里：不弹窗、也不回写，但分段照样导入。
        function test_dropped_path_needs_no_write_back() {
            host.offerQrtSplits("C:/drop.qrt", -1)
            compare(host.dlg.opens, 0)
            compare(host.settings.split_times, "0:01:00|0:02:00")
            compare(host.rows.writeBacks, 0)
        }

        // 不是 .qrt（没有分段）：只回写路径。
        function test_plain_image_only_writes_back() {
            host.preview.hasSplits = false
            host.offerQrtSplits("C:/map.jpg", 1)
            compare(host.dlg.opens, 0)
            compare(host.settings.split_times, "")
            compare(host.rows.writeBacks, 1)
        }

        // 只有 Start/Stop 的工程解析不出分段：不写空值、不弹窗。
        function test_empty_split_set_is_not_written() {
            host.preview.encoded = ""
            host.offerQrtSplits("C:/empty.qrt", 0)
            compare(host.dlg.opens, 0)
            compare(host.settings.split_times, "")
            compare(host.applied.length, 0)
        }
    }
}
