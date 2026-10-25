import QtQuick

// 本文件已废弃：旧「显示」GroupBox 已删除，四组轨迹样式（路线/尾巴/控制点/运动员）
// 迁进「地图布局」的逐框子面板（app/src/bin/MapEditor.qml）。
// 对应的结构/映射/显隐/交互断言已整体迁进 tst_map_layout.qml；
// test_route_rows.ps1 现在直接运行那份测试（保留旧文件名兼容 run-all 与文档）。
//
// 保留空 Item 只是为了外部若仍按文件名加载时不报错；不要在这里加断言（会重复维护）。
Item { objectName: "obsoleteRouteRowsTest" }
