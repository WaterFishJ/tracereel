import QtQuick
import QtQuick.Controls
import QtTest
import "../src/bin" as UI

// 多地图框编辑器的无头交互回归：
// - 框列表（添加/复制/删除/切换/数量提示）与「结构变化才完整重载」的调用纪律；
// - 逐框字段互不影响、每框独立取景控件显隐；
// - 预览里点选 / 拖动自由框 / 贴边框锁定 / 跨框撤销；
// - 外部（打开工程）重载。
//
// stub 只提供 MapEditor 用到的接口：source.map_boxes（JSON 字符串）与
// renderer.apply_map_frames（记录调用）；不依赖 Rust 编译。
Item {
    width: 1280; height: 1800

    QtObject {
        id: source
        property string map_boxes: ""
    }
    QtObject {
        id: renderer
        property int applyCount: 0
        property int paceRangeCalls: 0
        property string lastJson: ""
        function apply_map_frames(json) { lastJson = json; applyCount++ }
        /// 模拟 gui.rs 的 map_box_pace_range(index)：返回该框“当前实际生效”的色带端点。
        function map_box_pace_range(index) {
            paceRangeCalls++
            return JSON.stringify({low: 7.0, high: 3.0, tailLow: 8.0, tailHigh: 2.5})
        }
    }
    UI.MapEditor { id: editor; x: 930; width: 340; source: source; renderer: renderer; hasVideo: true }
    UI.HudCanvas { id: canvas; width: 900; height: 600; editor: editor; allowDelete: false; allowRedo: false }

    TestCase {
        name: "MapLayout"
        when: windowShown

        SignalSpy { id: structureSpy; target: editor; signalName: "structureChanged" }

        function initTestCase() { wait(150) }

        function defaultBox() {
            return {mode: 0, share: 0.25, x: 0.03, y: 0.05, w: 0.3, h: 0.6,
                    rotation: 0, opacity: 1, shape: 0, feather: 0, corner: 0,
                    viewMode: 0, viewZoom: 1.0, lrMargin: 20.0, tbMargin: 20.0, sensitivity: 0.5,
                    style: defaultStyle()}
        }
        /// 与 MapEditor.defaultStyle()/Rust MapBoxStyle::default() 一致。
        function defaultStyle() {
            return {renderMode: 0, renderScope: 0, width: 10.0,
                    tailRenderMode: 3, tailWidth: 10.0, tailLength: 30.0,
                    controlRadius: 15.0, showControls: false,
                    routeColor: "#000000", routeOpacity: 80.0,
                    tailColor: "#000000", tailOpacity: 204.0,
                    controlColor: "#000000", controlOpacity: 255.0,
                    runnerRadius: 15.0, showRunner: true,
                    runnerColor: "#ff5d00", runnerOpacity: 255.0, runnerStyle: 1,
                    lowPace: 15.0, highPace: 5.0, paceAuto: true,
                    tailLowPace: 16.204162961116385, tailHighPace: 5.117530921453715,
                    tailPaceAuto: true}
        }
        function styleWith(overrides) {
            var s = defaultStyle()
            for (var k in overrides) s[k] = overrides[k]
            return s
        }
        function makeBox(overrides) {
            var b = defaultBox()
            for (var k in overrides) {
                if (k === "style") {
                    var s = defaultStyle()
                    for (var sk in overrides.style) s[sk] = overrides.style[sk]
                    b.style = s
                } else b[k] = overrides[k]
            }
            return b
        }
        function oneBoxJson(overrides) {
            return JSON.stringify({version: 1, boxes: [makeBox(overrides || {})]})
        }
        function twoBoxJson(b0, b1) {
            return JSON.stringify({version: 1, boxes: [makeBox(b0), makeBox(b1)]})
        }
        function load(json) {
            source.map_boxes = json
            editor.sync()
            wait(0)
        }
        function resetOne() {
            structureSpy.clear()
            renderer.applyCount = 0
            renderer.lastJson = ""
            load(oneBoxJson({}))
            editor.snapping = false
            editor.selected = 0
            editor.undoStack = []
            wait(0)
        }
        function init() { resetOne() }

        function test_initial_single_box() {
            compare(editor.boxCount, 1)
            compare(editor.selected, 0)
            compare(Number(editor.currentBox.mode), 0)
            var tab0 = findChild(editor, "mapFrameTab_0")
            verify(tab0 !== null, "缺少框标签 mapFrameTab_0")
            verify(String(tab0.text).indexOf("1·") === 0, "标签应显示序号 1: " + tab0.text)
            verify(String(tab0.text).indexOf("固定分段") >= 0, "标签应显示取景方式: " + tab0.text)
            // 数量提示行已按用户要求移除（数量看标签即可）。
            verify(findChild(editor, "mapFrameCountLabel") === null, "数量提示行应已移除")
            // 贴边单框：显示「地图宽度」，不显示自由摆放的位置项。
            verify(findChild(editor, "mapStripShare") !== null)
            verify(!findChild(editor, "mapFrame_x").visible, "贴边模式不该显示水平位置")
            // 初始化不应触发结构重载 / 就地更新。
            compare(structureSpy.count, 0)
            compare(renderer.applyCount, 0)
        }

        function test_add_duplicate_remove_and_structure_signal() {
            var oldCount = editor.boxCount
            findChild(editor, "mapFrameAdd").clicked()
            wait(0)
            compare(editor.boxCount, oldCount + 1, "添加后应多一个框")
            compare(structureSpy.count, 1, "添加框必须发 structureChanged（触发完整重载）")
            compare(renderer.applyCount, 0, "框数变化时不能走 apply_map_frames 就地更新")
            compare(Number(editor.currentBox.mode), 4, "新增框必须自由摆放")
            compare(Number(editor.currentBox.viewMode), 1, "新增框取景方式应与第 1 框（固定分段）相反")
            // 新框的取景参数继承第 1 框。
            compare(Number(editor.currentBox.lrMargin), 20)
            // 写回代理的 JSON 必须已经有 2 个框。
            var saved = JSON.parse(String(source.map_boxes))
            compare(saved.boxes.length, 2, "source.map_boxes 应已包含新框")
            compare(Number(saved.boxes[1].viewMode), 1)

            // 复制当前选中的第 2 框（界面上已无「复制」按钮，保留 duplicateBox() 行为）。
            editor.duplicateBox()
            wait(0)
            compare(editor.boxCount, 3)
            compare(structureSpy.count, 2)
            compare(Number(editor.currentBox.viewMode), 1, "复制框应保留取景参数")
            verify(editor.currentBox.x !== saved.boxes[1].x, "复制框位置应偏移，避免完全重叠")

            // 删除两个，回到 1 个框；只剩 1 个时删除按钮禁用。
            findChild(editor, "mapFrameRemove").clicked()
            wait(0)
            findChild(editor, "mapFrameRemove").clicked()
            wait(0)
            compare(editor.boxCount, 1)
            verify(!findChild(editor, "mapFrameRemove").enabled, "只剩 1 个框时删除按钮应禁用")
        }

        function test_tabs_switch_selection() {
            load(twoBoxJson({mode: 0, share: 0.3}, {mode: 4, x: 0.6, y: 0.1, w: 0.3, h: 0.3, viewMode: 1}))
            compare(editor.selected, 0)
            findChild(editor, "mapFrameTab_1").clicked()
            wait(0)
            compare(editor.selected, 1, "点第 2 个标签应选中第 2 框")
            compare(Number(editor.currentBox.mode), 4)
            findChild(editor, "mapFrameTab_0").clicked()
            wait(0)
            compare(editor.selected, 0)
            // 第 2+ 框不能贴边：位置下拉禁用、当前显示「自由摆放」，切回第 1 框后可用。
            findChild(editor, "mapFrameTab_1").clicked()
            wait(0)
            verify(!findChild(editor, "mapPlacementChoice").enabled, "第 2+ 框不能选贴边")
            compare(findChild(editor, "mapPlacementChoice").currentIndex, 2)
            findChild(editor, "mapFrameTab_0").clicked()
            wait(0)
            verify(findChild(editor, "mapPlacementChoice").enabled, "第 1 框可以选择贴边")
        }

        function test_per_box_fields_are_independent() {
            load(twoBoxJson({mode: 0, share: 0.3, lrMargin: 12.5},
                             {mode: 4, x: 0.6, y: 0.1, w: 0.3, h: 0.3, viewMode: 1, viewZoom: 1.0}))
            // 第 1 框：固定分段 → 改左右边距。
            compare(editor.selected, 0)
            editor.setField("lrMargin", 33.0, false)
            wait(0)
            compare(Number(editor.currentBox.lrMargin), 33.0)
            // 第 2 框：跟跑者居中 → 改取景缩放；第 1 框的值不受影响。
            editor.selected = 1
            editor.setField("viewZoom", 2.5, false)
            wait(0)
            compare(Number(editor.currentBox.viewZoom), 2.5)
            editor.selected = 0
            wait(0)
            compare(Number(editor.currentBox.viewZoom), 1.0, "逐框取景参数互不影响")
            compare(Number(editor.currentBox.lrMargin), 33.0)
        }

        function test_view_mode_controls_follow_selected_box() {
            load(oneBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.4, h: 0.4, viewMode: 0}))
            var modeCombo = findChild(editor, "mapViewModeChoice")
            verify(modeCombo !== null, "缺少视野模式下拉 mapViewModeChoice")
            var lr = findChild(editor, "mapViewLrMargin")
            var tb = findChild(editor, "mapViewTbMargin")
            var zoom = findChild(editor, "mapViewZoom")
            var sens = findChild(editor, "mapViewSensitivity")
            verify(lr !== null && tb !== null && zoom !== null && sens !== null,
                   "缺少逐框取景控件（边距/缩放/灵敏度）")
            verify(lr.visible && tb.visible, "固定分段要显示左右/上下边距")
            verify(!zoom.visible && !sens.visible, "固定分段不该显示取景缩放/灵敏度")

            compare(modeCombo.currentIndex, 0)
            editor.setField("viewMode", 1, false)
            wait(0)
            compare(Number(editor.currentBox.viewMode), 1)
            compare(modeCombo.currentIndex, 1, "下拉应跟随当前框的取景方式")
            verify(!lr.visible && !tb.visible, "跟跑者居中不该显示边距")
            verify(zoom.visible && sens.visible, "跟跑者居中要显示取景缩放/灵敏度")
            compare(zoom.from, 2)
            compare(zoom.to, 300)
            compare(zoom.displayScale, 100, "取景缩放按 % 显示")
            compare(sens.decimals, 2)
        }

        function test_free_box_appearance_and_shape() {
            load(oneBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.4, h: 0.4}))
            var shape = findChild(editor, "mapShapeChoice")
            verify(shape !== null, "缺少地图形状下拉 mapShapeChoice")
            verify(findChild(editor, "mapFrame_opacity") !== null, "缺少不透明度滑杆")
            verify(findChild(editor, "mapFrame_opacitySlider") !== null)
            verify(findChild(editor, "mapFrame_opacityField") !== null)
            compare(findChild(editor, "mapFrame_corner").visible, true)
            editor.setField("shape", 1, false)
            wait(0)
            compare(Number(editor.currentBox.shape), 1)
            compare(findChild(editor, "mapFrame_corner").visible, false, "圆形框没有圆角项")
            editor.setField("shape", 0, false)
            editor.setField("opacity", 0.5, false)
            wait(0)
            fuzzyCompare(Number(editor.currentBox.opacity), 0.5, 0.0001)
            // 数值框精确输入羽化 15%。
            var field = findChild(editor, "mapFrame_featherField")
            field.forceActiveFocus(); field.selectAll()
            keyClick(Qt.Key_1); keyClick(Qt.Key_5); keyClick(Qt.Key_Return)
            wait(0)
            fuzzyCompare(Number(editor.currentBox.feather), 0.15, 0.0001)
        }

        function test_preview_click_selects_and_drag_moves_free_box() {
            load(twoBoxJson({mode: 0, share: 0.3},
                            {mode: 4, x: 0.6, y: 0.1, w: 0.3, h: 0.3}))
            editor.snapping = false
            // 第 2 框中心（画布 900x600，自由框 .6/.1/.3/.3 → 675,150）。
            mouseClick(canvas, 675, 150)
            wait(0)
            compare(editor.selected, 1, "点预览里的自由框应选中它")
            var beforeX = Number(editor.currentBox.x)
            mouseDrag(canvas, 675, 150, 45, 30, Qt.LeftButton)
            wait(0)
            compare(editor.selected, 1)
            verify(Number(editor.currentBox.x) > beforeX + 0.02, "拖动自由框应改变水平位置")
            // 撤销拖动（一次拖动 = 一步）。
            editor.undo()
            wait(0)
            fuzzyCompare(Number(editor.currentBox.x), beforeX, 0.0001)

            // 贴边第 1 框：可点选但不可拖动（locked）。
            mouseClick(canvas, 135, 300)
            wait(0)
            compare(editor.selected, 0, "点预览里的贴边框应选中它")
            var dockedX = Number(editor.currentBox.x)
            mouseDrag(canvas, 135, 300, 60, 40, Qt.LeftButton)
            wait(0)
            fuzzyCompare(Number(editor.currentBox.x), dockedX, 0.0001, "贴边框不应被拖动")
        }

        function test_drag_free_box_updates_renderer_in_place() {
            load(twoBoxJson({mode: 0, share: 0.3},
                            {mode: 4, x: 0.6, y: 0.1, w: 0.3, h: 0.3}))
            editor.snapping = false
            renderer.applyCount = 0
            renderer.lastJson = ""
            // 改取景参数：框数不变 → 必须走 apply_map_frames。
            editor.setField("viewMode", 1, false)
            wait(0)
            compare(renderer.applyCount, 1, "改取景参数应调 apply_map_frames 就地更新")
            var payload = JSON.parse(String(renderer.lastJson))
            compare(payload.boxes.length, 2)
            compare(Number(payload.boxes[0].viewMode), 1)
            editor.setField("viewZoom", 2.0, false)
            wait(0)
            compare(renderer.applyCount, 2)
            // 结构操作不发 apply_map_frames（必须完整重载）。
            findChild(editor, "mapFrameAdd").clicked()
            wait(0)
            compare(renderer.applyCount, 2, "加框不能走就地更新")
            compare(structureSpy.count, 1)
        }

        function test_external_reload() {
            editor.setField("viewMode", 2, false)
            wait(0)
            verify(editor.undoStack.length > 0, "改动后应有撤销历史")
            var json = twoBoxJson({mode: 4, x: 0.1, y: 0.2, w: 0.3, h: 0.3},
                                  {mode: 4, x: 0.5, y: 0.5, w: 0.3, h: 0.3, viewMode: 1})
            source.map_boxes = json
            editor.sync()
            wait(0)
            compare(editor.boxCount, 2, "外部重载后应读到 2 个框")
            compare(editor.undoStack.length, 0, "外部重载应清空撤销历史")
            verify(editor.selected >= 0 && editor.selected < editor.boxCount, "选中下标不能越界")
            fuzzyCompare(Number(editor.currentBox.y), 0.2, 0.0001)
        }

        function test_undo_across_boxes() {
            load(twoBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.3, h: 0.3},
                            {mode: 4, x: 0.6, y: 0.1, w: 0.3, h: 0.3, viewMode: 1}))
            editor.selected = 0
            editor.setField("opacity", 0.4, false)
            wait(0)
            editor.selected = 1
            editor.setField("viewZoom", 3.0, false)
            wait(0)
            fuzzyCompare(Number(editor.currentBox.viewZoom), 3.0, 0.0001)
            editor.undo()
            wait(0)
            fuzzyCompare(Number(editor.currentBox.viewZoom), 1.0, 0.0001, "撤销应回到该框改动前")
            editor.undo()
            wait(0)
            editor.selected = 0
            wait(0)
            fuzzyCompare(Number(editor.currentBox.opacity), 1.0, 0.0001, "撤销栈跨框共用")
        }

        // ------------------------------------------------------------ 逐框轨迹样式

        /// 代理写回的 JSON：样式嵌套在每框的 "style" 里、camelCase 键名。
        function test_box_style_present_in_json_and_camel_case() {
            var j0 = JSON.parse(String(source.map_boxes))
            var st0 = j0.boxes[0].style
            verify(st0 !== undefined, "代理 JSON 的框里应带 style")
            compare(Number(st0.renderMode), 0)
            compare(Number(st0.tailRenderMode), 3)
            compare(String(st0.runnerColor), "#ff5d00")
            verify(st0.render_mode === undefined, "style 必须是 camelCase: " + JSON.stringify(st0))

            editor.setStyle("width", 12.5, false)
            wait(0)
            var j = JSON.parse(String(source.map_boxes))
            compare(Number(j.boxes[0].style.width), 12.5, "改样式要写回代理 map_boxes")
            compare(renderer.applyCount, 1, "框数不变时样式改动走 apply_map_frames 就地更新")
            var payload = JSON.parse(String(renderer.lastJson))
            compare(Number(payload.boxes[0].style.width), 12.5)
        }

        /// 样式控件随模式/勾选显隐（与原 gui.qml「显示」组一致的行显隐规则）。
        function test_style_controls_visibility() {
            load(oneBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.4, h: 0.4,
                             style: styleWith({renderMode: 0, tailRenderMode: 0,
                                               showControls: false, showRunner: false})}))
            function row(name) { return findChild(editor, name) }
            verify(row("boxStyle_renderMode") !== null, "缺少路线渲染模式行")
            compare(row("boxStyle_width").visible, false, "路线=无时不显示宽度")
            compare(row("boxStyle_routeColor").visible, false)
            compare(row("boxStyle_routeOpacity").visible, false)
            compare(row("boxStyle_paceAuto").visible, false, "非配速模式不显示配速自适应")

            editor.setStyle("renderMode", 1, false)
            wait(0)
            compare(row("boxStyle_width").visible, true)
            compare(row("boxStyle_routeColor").visible, true, "简洁模式显示颜色")
            compare(row("boxStyle_routeOpacity").visible, true)
            compare(row("boxStyle_paceAuto").visible, false)

            editor.setStyle("renderMode", 3, false)
            wait(0)
            compare(row("boxStyle_routeColor").visible, false, "配速模式不显示单色取色器")
            compare(row("boxStyle_routeOpacity").visible, true, "透明度三种模式都有效")
            compare(row("boxStyle_paceAuto").visible, true)
            compare(row("boxStyle_lowPace").visible, false, "自适应开着时不显示手填配速")
            editor.setStyle("paceAuto", false, false)
            wait(0)
            compare(row("boxStyle_lowPace").visible, true)
            compare(row("boxStyle_highPace").visible, true)

            // 尾巴同理。
            editor.setStyle("tailRenderMode", 1, false)
            wait(0)
            compare(row("boxStyle_tailWidth").visible, true)
            compare(row("boxStyle_tailLength").visible, true)
            compare(row("boxStyle_tailColor").visible, true)
            compare(row("boxStyle_tailOpacity").visible, true)
            compare(row("boxStyle_tailPaceAuto").visible, false)
            editor.setStyle("tailRenderMode", 3, false)
            wait(0)
            compare(row("boxStyle_tailColor").visible, false)
            compare(row("boxStyle_tailPaceAuto").visible, true)

            // 控制点 / 运动员：开关控制同组参数。
            compare(row("boxStyle_controlRadius").visible, false)
            editor.setStyle("showControls", true, false)
            wait(0)
            compare(row("boxStyle_controlRadius").visible, true)
            compare(row("boxStyle_controlColor").visible, true)
            compare(row("boxStyle_controlOpacity").visible, true)
            compare(row("boxStyle_runnerStyle").visible, false)
            editor.setStyle("showRunner", true, false)
            wait(0)
            compare(row("boxStyle_runnerStyle").visible, true)
            compare(row("boxStyle_runnerRadius").visible, true)
            compare(row("boxStyle_runnerColor").visible, true)
            compare(row("boxStyle_runnerOpacity").visible, true)
        }

        /// 逐框样式互不影响：改第 2 框的渲染模式/颜色不改第 1 框。
        function test_style_edit_is_per_box() {
            load(twoBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.3, h: 0.3,
                             style: styleWith({renderMode: 1, width: 18.0, routeColor: "#112233"})},
                            {mode: 4, x: 0.6, y: 0.1, w: 0.3, h: 0.3,
                             style: styleWith({renderMode: 3, width: 22.0})}))
            editor.selected = 0
            editor.setStyle("width", 30.0, false)
            wait(0)
            var j = JSON.parse(String(source.map_boxes))
            compare(Number(j.boxes[0].style.width), 30.0, "第 1 框样式应被改")
            compare(Number(j.boxes[1].style.width), 22.0, "第 2 框样式不能被连累")
            editor.selected = 1
            editor.setStyle("routeColor", "#abcdef", false)
            wait(0)
            j = JSON.parse(String(source.map_boxes))
            compare(String(j.boxes[1].style.routeColor), "#abcdef")
            compare(String(j.boxes[0].style.routeColor), "#112233", "第 1 框颜色不能被连累")
            compare(Number(j.boxes[1].style.width), 22.0, "同框其它字段保持不变")
        }

        /// 滑杆/数值框/勾选框交互（拖动一步撤销、透明度按 0..255 存储）。
        function test_style_controls_interaction() {
            load(oneBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.4, h: 0.4,
                             style: styleWith({renderMode: 1})}))
            // 数值框：路线宽度精确输入。
            var widthField = findChild(editor, "boxStyle_widthField")
            verify(widthField !== null, "缺少路线宽度输入框")
            widthField.forceActiveFocus(); widthField.selectAll()
            keyClick(Qt.Key_2); keyClick(Qt.Key_0); keyClick(Qt.Key_Return)
            wait(0)
            fuzzyCompare(Number(editor.currentStyle.width), 20.0, 0.001)

            // 滑杆：路线透明度（显示 %，存储 0..255）。拖到 ~50%。
            var opacitySlider = findChild(editor, "boxStyle_routeOpacitySlider")
            verify(opacitySlider !== null, "缺少路线透明度滑杆")
            wait(80)
            verify(opacitySlider.width > 50, "滑杆还没完成布局")
            var undoBefore = editor.undoStack.length
            // 存储 80/255 ≈ 31%，滑杆手柄也在 31%：从手柄位置往左拖一点。
            var startFrac = (Number(editor.currentStyle.routeOpacity) / 255)
            mouseDrag(opacitySlider, opacitySlider.width * startFrac, opacitySlider.height / 2,
                      -opacitySlider.width * 0.12, 0)
            wait(0)
            verify(Number(editor.currentStyle.routeOpacity) < 80.0, "拖滑杆应改小不透明度")
            verify(Number(editor.currentStyle.routeOpacity) >= 0
                   && Number(editor.currentStyle.routeOpacity) <= 255.0)
            compare(editor.undoStack.length, undoBefore + 1, "一次拖动只算一步撤销")
            editor.undo()
            wait(0)
            fuzzyCompare(Number(editor.currentStyle.routeOpacity), 80.0, 0.5)

            // 勾选框：显示控制点 → 控制点参数行出现。
            var showCb = findChild(editor, "boxStyle_showControlsCheck")
            verify(showCb !== null, "缺少显示控制点勾选框")
            compare(showCb.checked, false)
            mouseClick(showCb, showCb.width / 2, showCb.height / 2)
            wait(0)
            compare(editor.currentStyle.showControls, true)
            compare(findChild(editor, "boxStyle_controlRadius").visible, true)
        }

        /// 关「配速色带自适应」时用 renderer.map_box_pace_range(selected) 填当前色带。
        function test_pace_auto_off_uses_renderer_range() {
            load(oneBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.4, h: 0.4,
                             style: styleWith({renderMode: 3, tailRenderMode: 3,
                                               paceAuto: true, lowPace: 15, highPace: 5,
                                               tailPaceAuto: true,
                                               tailLowPace: 16.2, tailHighPace: 5.1})}))
            renderer.paceRangeCalls = 0
            var routeCb = findChild(editor, "boxStyle_paceAutoCheck")
            verify(routeCb !== null)
            mouseClick(routeCb, routeCb.width / 2, routeCb.height / 2)
            wait(0)
            compare(renderer.paceRangeCalls, 1, "关自适应应查询该框实际生效的色带")
            compare(editor.currentStyle.paceAuto, false)
            fuzzyCompare(Number(editor.currentStyle.lowPace), 7.0, 0.001)
            fuzzyCompare(Number(editor.currentStyle.highPace), 3.0, 0.001)
            compare(findChild(editor, "boxStyle_lowPace").visible, true)
            compare(findChild(editor, "boxStyle_highPace").visible, true)
            editor.undo()
            wait(0)
            compare(editor.currentStyle.paceAuto, true, "关自适应只算一步撤销")
            fuzzyCompare(Number(editor.currentStyle.lowPace), 15.0, 0.001)

            var tailCb = findChild(editor, "boxStyle_tailPaceAutoCheck")
            verify(tailCb !== null)
            mouseClick(tailCb, tailCb.width / 2, tailCb.height / 2)
            wait(0)
            compare(renderer.paceRangeCalls, 2)
            compare(editor.currentStyle.tailPaceAuto, false)
            fuzzyCompare(Number(editor.currentStyle.tailLowPace), 8.0, 0.001)
            fuzzyCompare(Number(editor.currentStyle.tailHighPace), 2.5, 0.001)
        }

        /// 颜色：色块按颜色+透明度合成显示；取色对话框回写 rgb（透明度独立）。
        /// 放在最后执行：真实打开 ColorDialog 会让离屏窗口失活，影响后续键盘聚焦测试。
        function test_zz_style_color_swatch_and_picker() {
            load(oneBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.4, h: 0.4,
                             style: styleWith({renderMode: 1, routeColor: "#112233",
                                               routeOpacity: 128})}))
            var swatch = findChild(editor, "boxStyle_routeColorSwatch")
            verify(swatch !== null, "缺少路线颜色色块")
            compare(String(swatch.color), String(editor.qtColor("#112233", 128)),
                    "色块要按颜色+不透明度显示")
            var pick = findChild(editor, "boxStyle_routeColorPick")
            verify(pick !== null, "缺少取色按钮")
            editor.pickStyleColor("routeColor", "routeOpacity")
            var picker = findChild(editor, "boxStyleColorPicker")
            verify(picker !== null, "缺少取色对话框")
            picker.selectedColor = "#00ff00"
            picker.accepted()
            wait(0)
            compare(editor.currentStyle.routeColor, "#00ff00", "取色回写 #rrggbb")
            fuzzyCompare(Number(editor.currentStyle.routeOpacity), 128, 0.001,
                         "取色不应改透明度")
            try { picker.close() } catch (e) { }
        }

        /// 样式改动也进同一套跨框撤销。
        function test_style_undo_across_boxes() {
            load(twoBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.3, h: 0.3},
                            {mode: 4, x: 0.6, y: 0.1, w: 0.3, h: 0.3}))
            editor.selected = 0
            editor.setStyle("width", 30.0, false)
            wait(0)
            editor.selected = 1
            editor.setStyle("renderMode", 3, false)
            wait(0)
            compare(Number(editor.currentStyle.renderMode), 3)
            editor.undo()
            wait(0)
            compare(Number(editor.currentStyle.renderMode), 0, "撤销应回到该框改动前")
            editor.undo()
            wait(0)
            editor.selected = 0
            wait(0)
            fuzzyCompare(Number(editor.currentStyle.width), 10.0, 0.001, "撤销栈跨框共用")
        }

        /// 框标签很多时「添加/复制/删除」仍在编辑器宽度内可点（用户反馈的 bug）。
        function test_many_frames_keep_action_buttons_visible() {
            var boxes = []
            for (var i = 0; i < 10; ++i) {
                boxes.push(makeBox({mode: i === 0 ? 0 : 4,
                                    share: 0.25,
                                    x: 0.05 + 0.02 * i, y: 0.05 + 0.15 * (i % 4),
                                    w: 0.2, h: 0.2}))
            }
            load(JSON.stringify({version: 1, boxes: boxes}))
            compare(editor.boxCount, 10)
            wait(80)
            var ew = Number(editor.width)
            verify(ew > 0)
            var actions = ["mapFrameAdd", "mapFrameRemove"]
            for (var a = 0; a < actions.length; ++a) {
                var btn = findChild(editor, actions[a])
                verify(btn !== null, "缺少按钮 " + actions[a])
                verify(btn.visible && btn.width > 0 && btn.height > 0, actions[a] + " 不可见")
                verify(Number(btn.x) + Number(btn.width) <= ew + 0.5,
                       actions[a] + " 被挤出编辑区: x=" + btn.x + " w=" + btn.width + " editor=" + ew)
            }
            compare(findChild(editor, "mapFrameAdd").enabled, true)
            var tab9 = findChild(editor, "mapFrameTab_9")
            verify(tab9 !== null, "10 框时应能找到第 10 个标签（Flow 换行）")
            tab9.clicked()
            wait(0)
            compare(editor.selected, 9, "多框时标签仍能选中任意框")
        }

        /// ---- 从旧 test_route_rows（guib.qml「显示」组）迁移过来的结构/映射断言 ----
        /// 四组样式已搬进 MapEditor 的逐框子面板，这里直接对编辑器结构断言：
        /// 分组顺序、每组的行顺序、菜单「显示项 → 存储值」映射、颜色/透明度配对。
        function test_style_rows_grouped_and_mapped() {
            load(oneBoxJson({mode: 4, x: 0.1, y: 0.1, w: 0.4, h: 0.4,
                             style: styleWith({renderMode: 3, tailRenderMode: 3})}))
            function styleNames() {
                var names = []
                for (var i = 0; i < editor.children.length; ++i) {
                    var c = editor.children[i]
                    if (c && c.objectName && String(c.objectName).indexOf("boxStyle_") === 0
                            && String(c.objectName) !== "boxStyleColorPicker") {
                        names.push(String(c.objectName))
                    }
                }
                return names
            }
            function subsectionTitles() {
                var titles = []
                for (var i = 0; i < editor.children.length; ++i) {
                    var c = editor.children[i]
                    if (c && typeof c.title === "string" && c.title.length > 0) titles.push(c.title)
                }
                return titles
            }
            // 四组的二级标题与顺序。
            compare(subsectionTitles().join("|"), "路线|尾巴|控制点|运动员",
                    "逐框样式的四组顺序/标题与原「显示」组一致")
            // 每组的行顺序（与原 gui.qml 显示组一一对应）。
            var routeRows = ["boxStyle_renderMode", "boxStyle_renderScope", "boxStyle_width",
                             "boxStyle_routeColor", "boxStyle_routeOpacity", "boxStyle_paceAuto",
                             "boxStyle_lowPace", "boxStyle_highPace"]
            var tailRows = ["boxStyle_tailRenderMode", "boxStyle_tailWidth", "boxStyle_tailLength",
                            "boxStyle_tailColor", "boxStyle_tailOpacity", "boxStyle_tailPaceAuto",
                            "boxStyle_tailLowPace", "boxStyle_tailHighPace"]
            var controlRows = ["boxStyle_showControls", "boxStyle_controlRadius",
                               "boxStyle_controlColor", "boxStyle_controlOpacity"]
            var runnerRows = ["boxStyle_showRunner", "boxStyle_runnerStyle", "boxStyle_runnerRadius",
                              "boxStyle_runnerColor", "boxStyle_runnerOpacity"]
            var expected = routeRows.concat(tailRows).concat(controlRows).concat(runnerRows)
            compare(styleNames().join("|"), expected.join("|"), "样式行的分组与顺序")
            // 菜单：显示项与存储值的映射（编号沿用原版 0/1/3）。
            var renderMode = findChild(editor, "boxStyle_renderMode")
            compare(renderMode.items.length, 3)
            compare(renderMode.items[0], "无")
            compare(renderMode.items[1], "简洁")
            compare(renderMode.items[2], "配速")
            compare(renderMode.values.join(","), "0,1,3", "路线渲染模式沿用原版编号")
            var tailMode = findChild(editor, "boxStyle_tailRenderMode")
            compare(tailMode.values.join(","), "0,1,3")
            compare(tailMode.items[2], "配速")
            var scope = findChild(editor, "boxStyle_renderScope")
            compare(scope.values.join(","), "0,1")
            compare(scope.items[0], "完整渲染")
            compare(scope.items[1], "单段渲染")
            var runnerStyle = findChild(editor, "boxStyle_runnerStyle")
            compare(runnerStyle.values.join(","), "0,1")
            compare(runnerStyle.items[0], "圆形")
            compare(runnerStyle.items[1], "箭头")
            // 颜色 + 独立透明度：每个色块都绑着同一片区域的透明度项（0..255 存储）。
            var pairs = [["boxStyle_routeColor", "boxStyle_routeOpacity", "routeColor", "routeOpacity"],
                         ["boxStyle_tailColor", "boxStyle_tailOpacity", "tailColor", "tailOpacity"],
                         ["boxStyle_controlColor", "boxStyle_controlOpacity", "controlColor", "controlOpacity"],
                         ["boxStyle_runnerColor", "boxStyle_runnerOpacity", "runnerColor", "runnerOpacity"]]
            for (var p = 0; p < pairs.length; ++p) {
                var colorRow = findChild(editor, pairs[p][0])
                var opacityRow = findChild(editor, pairs[p][1])
                verify(colorRow !== null, "缺少颜色行 " + pairs[p][0])
                verify(opacityRow !== null, "缺少透明度行 " + pairs[p][1])
                compare(colorRow.colorKey, pairs[p][2])
                compare(colorRow.opacityKey, pairs[p][3])
                compare(opacityRow.key, pairs[p][3], "透明度行要绑到同一片区域")
                compare(opacityRow.alphaPercent, true, "透明度按 0..255 存储、0..100% 显示")
                compare(opacityRow.from, 0)
                compare(opacityRow.to, 100)
                compare(opacityRow.decimals, 0, "百分比透明度不要小数")
            }
            // 四个「开关型」行永远可见（不依赖别的模式）。
            var always = ["boxStyle_renderMode", "boxStyle_tailRenderMode",
                          "boxStyle_showControls", "boxStyle_showRunner"]
            for (var a = 0; a < always.length; ++a) {
                verify(findChild(editor, always[a]).visible, always[a] + " 应常显")
            }
        }

        function test_required_object_names() {
            load(oneBoxJson({mode: 4, x: 0.2, y: 0.2, w: 0.3, h: 0.3,
                             style: styleWith({renderMode: 3, tailRenderMode: 3,
                                               showControls: true, showRunner: true,
                                               paceAuto: false, tailPaceAuto: false})}))
            var names = ["mapFrameTab_0", "mapFrameAdd", "mapFrameRemove",
                         "mapPlacementChoice", "mapStripShare", "mapShapeChoice",
                         "mapFrame_x", "mapFrame_y", "mapFrame_w", "mapFrame_h", "mapFrame_rotation",
                         "mapFrame_opacity", "mapFrame_opacitySlider", "mapFrame_opacityField",
                         "mapFrame_feather", "mapFrame_corner",
                         "mapViewModeChoice", "mapViewLrMargin", "mapViewTbMargin",
                         "mapViewZoom", "mapViewSensitivity",
                         // 逐框轨迹样式（camelCase 键名对应的 objectName）。
                         "boxStyle_renderMode", "boxStyle_renderModeCombo",
                         "boxStyle_renderScope", "boxStyle_renderScopeCombo",
                         "boxStyle_width", "boxStyle_widthField",
                         "boxStyle_routeColor", "boxStyle_routeColorSwatch", "boxStyle_routeColorPick",
                         "boxStyle_routeOpacity", "boxStyle_routeOpacitySlider", "boxStyle_routeOpacityField",
                         "boxStyle_paceAuto", "boxStyle_paceAutoCheck",
                         "boxStyle_lowPace", "boxStyle_highPace",
                         "boxStyle_tailRenderMode", "boxStyle_tailRenderModeCombo",
                         "boxStyle_tailWidth", "boxStyle_tailLength",
                         "boxStyle_tailColor", "boxStyle_tailOpacity",
                         "boxStyle_tailPaceAuto", "boxStyle_tailLowPace", "boxStyle_tailHighPace",
                         "boxStyle_showControls", "boxStyle_showControlsCheck",
                         "boxStyle_controlRadius", "boxStyle_controlColor", "boxStyle_controlOpacity",
                         "boxStyle_showRunner", "boxStyle_showRunnerCheck",
                         "boxStyle_runnerStyle", "boxStyle_runnerStyleCombo",
                         "boxStyle_runnerRadius", "boxStyle_runnerColor", "boxStyle_runnerOpacity",
                         "boxStyleColorPicker"]
            for (var i = 0; i < names.length; ++i) {
                verify(findChild(editor, names[i]) !== null, "缺少 objectName: " + names[i])
            }
        }

        // 在预览空白处取消选中（HudCanvas 写 selected=-1）后：面板继续显示/编辑
        // 最后一个选中的框（和选中时一样），只是预览里不再高亮。
        function test_deselect_keeps_last_selected_info() {
            load(twoBoxJson({mode: 0}, {}))
            editor.selected = 1
            wait(0)
            compare(editor.activeIndex, 1)
            var box1 = JSON.stringify(editor.currentBox)
            editor.selected = -1
            wait(0)
            compare(editor.selected, -1)
            compare(editor.activeIndex, 1, "取消选中后应回退到最后一个选中的框")
            verify(editor.currentBox !== null, "取消选中后仍要显示最后选中框的信息")
            compare(JSON.stringify(editor.currentBox), box1, "显示的信息应就是最后选中的那个框")
            verify(editor.current === null, "取消选中后预览里不应再高亮")
            verify(findChild(editor, "mapPlacementChoice").visible, "取消选中后子面板应仍然显示")
            // 编辑仍然作用在最后选中的框上，不会悄悄改到别的框。
            editor.setField("viewZoom", 1.25, false)
            compare(Number(editor.currentBox.viewZoom), 1.25)
            compare(Number(editor.layout.boxes[1].viewZoom), 1.25)
            compare(editor.selected, -1, "编辑不应把取消选中状态改回去")
        }
    }
}
