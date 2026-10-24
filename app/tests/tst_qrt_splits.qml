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
//
// 另一条铁律（0.3.0 回归）：**地图路径必须先落地，再碰分段**。
// adoptQrtSplits() 会调 refreshFields()，真机上那会让轨迹列表按 settings 重建，
// 还没写进 settings 的行会被清掉 → 分段导入成功、地图却没导入。
// 所以 harness 的 refreshFields() 会真的做一次 syncFromSettings()（见 ps1 里的说明）。
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
            host.settings.route_path = ""
            host.settings.map_path = ""
            host.rows.mainPath = ""
            host.rows.pendingQrtPath = ""
            host.rows.writeBacks = 0
            host.rows.syncs = 0
            host.rows.wiped = false
            host.dlg.opens = 0
            host.preview.hasSplits = true
            host.preview.encoded = "0:01:00|0:02:00"
            host.applied = []
            host.logs = []
        }

        // 没有分段：直接导入，不弹窗，**地图路径也必须落地**。
        function test_imports_directly_when_no_splits_yet() {
            host.rows.mainPath = "C:/m.qrt"          // 用户在列表行里选中了 .qrt
            host.offerQrtSplits("C:/m.qrt", 0)
            compare(host.dlg.opens, 0, "没有分段时不该弹窗")
            compare(host.settings.split_times, "0:01:00|0:02:00")
            compare(host.settings.split_point_codes, "", "点号要清空（.qrt 里没有点号）")
            compare(host.settings.split_is_relative, false, "QuickRoute 的 lap 是绝对时间")
            compare(host.rows.writeBacks, 1, "列表行进来时要回写路径")
            compare(host.applied.length, 2, "分段时间与时间类型都要应用")
        }

        // 0.3.0 回归：.qrt 带分段 + 当前没有分段 → 直接导入分段，
        // 但 adoptQrtSplits() 里的 refreshFields() 会把尚未落地的行清掉，
        // 于是 writeBack() 写了个空路径 → 地图没导入。路径必须先落地。
        function test_map_row_survives_split_import() {
            host.rows.mainPath = "C:/maps/AM 男子短距 第一面.qrt"
            host.offerQrtSplits("C:/maps/AM 男子短距 第一面.qrt", 0)
            compare(host.rows.wiped, false, "还没落地的行不能被 refreshFields 清掉")
            compare(host.settings.route_path, "C:/maps/AM 男子短距 第一面.qrt",
                    "地图路径必须留在 settings 里（分段导入不能把它冲掉）")
            compare(host.settings.map_path, "C:/maps/AM 男子短距 第一面.qrt")
            compare(host.settings.split_times, "0:01:00|0:02:00", "分段照样导入")
            compare(host.rows.writeBacks, 1)
        }

        // 已经有分段：仍然弹窗问，不能覆盖 —— 但路径（地图）照旧先落地。
        function test_asks_before_overwriting_existing_splits() {
            host.settings.split_times = "0:00:30"
            host.rows.mainPath = "D:/m.qrt"
            host.offerQrtSplits("D:/m.qrt", 2)
            compare(host.dlg.opens, 1)
            compare(host.rows.pendingQrtPath, "D:/m.qrt")
            compare(host.settings.split_times, "0:00:30", "确认前不动已有分段")
            // 路径先落地：弹窗还没回答，地图与轨迹也应该已经导入（弹窗文案就是这么写的）。
            compare(host.rows.writeBacks, 1, "路径先落地，地图立刻可用")
            compare(host.settings.route_path, "D:/m.qrt")
        }

        // 拖放进来的路径已经在模型里（addDroppedPath 已经写过 settings）：不重复回写。
        function test_dropped_path_needs_no_write_back() {
            host.rows.mainPath = "C:/drop.qrt"
            host.settings.route_path = "C:/drop.qrt"
            host.offerQrtSplits("C:/drop.qrt", -1)
            compare(host.dlg.opens, 0)
            compare(host.settings.split_times, "0:01:00|0:02:00")
            compare(host.rows.writeBacks, 0)
            compare(host.settings.route_path, "C:/drop.qrt")
        }

        // 不是 .qrt（没有分段）：只回写路径。
        function test_plain_image_only_writes_back() {
            host.preview.hasSplits = false
            host.rows.mainPath = "C:/map.jpg"
            host.offerQrtSplits("C:/map.jpg", 1)
            compare(host.dlg.opens, 0)
            compare(host.settings.split_times, "")
            compare(host.rows.writeBacks, 1)
            compare(host.settings.route_path, "C:/map.jpg")
        }

        // 只有 Start/Stop 的工程解析不出分段：不写空值、不弹窗，但地图照常导入。
        function test_empty_split_set_is_not_written() {
            host.preview.encoded = ""
            host.rows.mainPath = "C:/empty.qrt"
            host.offerQrtSplits("C:/empty.qrt", 0)
            compare(host.dlg.opens, 0)
            compare(host.settings.split_times, "")
            compare(host.applied.length, 0)
            compare(host.settings.route_path, "C:/empty.qrt", "地图照样导入")
        }
    }
}
