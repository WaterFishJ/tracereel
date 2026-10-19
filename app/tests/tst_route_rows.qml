import QtQuick
import QtTest

// 「显示」配置组的行显隐测试。
// 被测文本是从 app/src/bin/gui.qml 里**原样切出来**的那个 GroupBox（见 test_route_rows.ps1），
// 组件本身（Combo/DSpin/Check/ColorField/SliderField/SubSection/PercentSpin）在 harness 里
// 用同名桩替代，只保留 label/prop/opacityProp/items/values/from/to/visible 这些被测文本真的用到的接口。
Item {
    width: 600; height: 800
    RouteRowsHarness { id: host; anchors.fill: parent }

    TestCase {
        name: "RouteRows"
        when: windowShown

        function initTestCase() { wait(200) }

        function row(label) {
            function walk(item) {
                for (var i = 0; i < item.children.length; ++i) {
                    var c = item.children[i]
                    if (c.label === label) return c
                    var hit = walk(c)
                    if (hit) return hit
                }
                return null
            }
            return walk(host)
        }

        function vis(label) {
            var r = row(label)
            verify(r !== null, "找不到配置行: " + label)
            return r.visible
        }

        function setRoute(mode, tail, showControls, showRunner) {
            host.settings.route_render_mode = mode
            host.settings.route_tail_render_mode = tail
            host.settings.route_show_controls = showControls
            host.settings.route_show_runner = showRunner
            host.refreshTick++
        }

        function setPaceAuto(v) {
            host.settings.route_pace_auto = v
            host.refreshTick++
        }

        // 路线渲染模式 = 无 → 隐藏「路线宽度」「路线颜色」「路线透明度」
        // 配速模式（3）的颜色由色带决定 → 不显示路线颜色，但**透明度仍然有效**
        // （逐段线的不透明度用的就是它），只显示自适应开关/低/高配速。
        function test_route_rows_follow_render_mode() {
            setRoute(0, 1, true, true)
            verify(!vis("路线宽度 (像素)"), "路线渲染模式=无 时必须隐藏路线宽度")
            verify(!vis("路线颜色"), "路线渲染模式=无 时必须隐藏路线颜色")
            verify(!vis("路线透明度 (%)"), "路线渲染模式=无 时必须隐藏路线透明度")
            setRoute(1, 1, true, true)
            verify(vis("路线宽度 (像素)"), "简洁模式要显示路线宽度")
            verify(vis("路线颜色"), "简洁模式要显示路线颜色")
            verify(vis("路线透明度 (%)"), "简洁模式要显示路线透明度")
            // 老工程里存的是原版的 2（高亮）：本程序把它当简洁渲染，所以这些项也要显示。
            setRoute(2, 1, true, true)
            verify(vis("路线宽度 (像素)"), "老工程的 2（高亮）要显示路线宽度")
            verify(vis("路线颜色"), "老工程的 2 按简洁渲染，要显示路线颜色")
            verify(!vis("配速色带自适应"), "老工程的 2 不是配速模式")
            setRoute(3, 1, true, true)
            verify(vis("路线宽度 (像素)"), "配速模式要显示路线宽度")
            verify(!vis("路线颜色"), "配速模式的颜色来自色带，不该显示路线颜色")
            verify(vis("路线透明度 (%)"), "配速模式的逐段线也要用路线透明度，必须显示")
            verify(vis("配速色带自适应"), "配速模式要显示自适应开关")
        }

        // 低/高配速只在配速模式、且关掉「配速色带自适应」时才有意义（自适应时由轨迹算）
        function test_pace_rows_follow_pace_mode() {
            setPaceAuto(false)
            setRoute(0, 1, true, true)
            verify(!vis("低配速 (分钟/公里)"), "非配速模式不该显示低配速")
            verify(!vis("高配速 (分钟/公里)"), "非配速模式不该显示高配速")
            setRoute(1, 1, true, true)
            verify(!vis("低配速 (分钟/公里)"), "简洁模式不该显示低配速")
            setRoute(3, 1, true, true)
            verify(vis("低配速 (分钟/公里)"), "配速模式+手动 要显示低配速")
            verify(vis("高配速 (分钟/公里)"), "配速模式+手动 要显示高配速")
            setPaceAuto(true)
            verify(vis("配速色带自适应"), "配速模式要显示自适应开关")
            verify(!vis("低配速 (分钟/公里)"), "自适应开着时低配速由轨迹算，不该显示")
            verify(!vis("高配速 (分钟/公里)"), "自适应开着时高配速由轨迹算，不该显示")
            setPaceAuto(false)
            verify(vis("低配速 (分钟/公里)"), "关掉自适应后要能手动改低配速")
        }

        // 尾巴渲染模式：无 → 尾巴各项全不显示；简洁 → 显示宽度/长度/颜色/透明度；
        // 配速 → 显示尾巴独立色带设置，路线设置不受影响。
        function test_tail_rows_follow_tail_mode() {
            setPaceAuto(false)
            setRoute(1, 0, true, true)
            verify(!vis("尾巴宽度 (像素)"), "尾巴渲染模式=无 时必须隐藏尾巴宽度")
            verify(!vis("尾巴长度 (秒)"), "尾巴渲染模式=无 时必须隐藏尾巴长度")
            verify(!vis("尾巴颜色"), "尾巴渲染模式=无 时必须隐藏尾巴颜色")
            verify(!vis("尾巴透明度 (%)"), "尾巴渲染模式=无 时必须隐藏尾巴透明度")
            verify(vis("路线宽度 (像素)"), "路线模式=简洁 时路线宽度仍要显示")
            setRoute(1, 1, true, true)
            verify(vis("尾巴宽度 (像素)"), "尾巴模式=简洁 要显示尾巴宽度")
            verify(vis("尾巴长度 (秒)"), "尾巴模式=简洁 要显示尾巴长度")
            verify(vis("尾巴颜色"), "尾巴模式=简洁 要显示尾巴颜色")
            verify(vis("尾巴透明度 (%)"), "尾巴模式=简洁 要显示尾巴透明度")
            verify(!vis("配速色带自适应"), "路线与尾巴都不是配速时不该显示色带设置")
            // 老工程里尾巴模式可能存的是 2（高亮），读进内存时归一成 1（简洁）。
            setRoute(1, 2, true, true)
            verify(vis("尾巴宽度 (像素)"), "尾巴=2（老工程高亮）要显示尾巴宽度")
            verify(vis("尾巴颜色"), "尾巴=2 按简洁渲染，要显示尾巴颜色")
            host.settings.route_tail_pace_auto = false
            setRoute(0, 3, true, true)
            verify(vis("尾巴宽度 (像素)"), "尾巴模式=配速 要显示尾巴宽度")
            verify(vis("尾巴长度 (秒)"), "尾巴模式=配速 要显示尾巴长度")
            verify(!vis("尾巴颜色"), "尾巴模式=配速 的颜色来自色带，不该显示尾巴颜色")
            verify(vis("尾巴透明度 (%)"), "尾巴模式=配速 的色带也要用尾巴透明度，必须显示")
            verify(vis("尾巴配速色带自适应"), "尾巴模式=配速 要显示色带自适应开关")
            verify(vis("尾巴低配速 (分钟/公里)"), "尾巴模式=配速+手动 要显示低配速")
            verify(vis("尾巴高配速 (分钟/公里)"), "尾巴模式=配速+手动 要显示高配速")
            host.settings.route_tail_pace_auto = true
            host.refreshTick++
            verify(!vis("尾巴低配速 (分钟/公里)"), "自适应开着时低配速由轨迹算，不该显示")
            verify(!vis("尾巴高配速 (分钟/公里)"), "自适应开着时高配速由轨迹算，不该显示")
            host.settings.route_tail_pace_auto = false
            host.refreshTick++
            verify(!vis("配速色带自适应"), "尾巴配速不能显示路线自适应开关")
            setRoute(3, 3, true, true)
            setPaceAuto(true)
            verify(vis("尾巴低配速 (分钟/公里)"), "路线自适应不影响尾巴手动范围")
            verify(!vis("低配速 (分钟/公里)"), "路线自适应时隐藏路线手动范围")
            host.settings.route_tail_pace_auto = true
            setPaceAuto(false)
            verify(vis("低配速 (分钟/公里)"), "尾巴自适应不影响路线手动范围")
            verify(!vis("尾巴低配速 (分钟/公里)"), "尾巴自适应时隐藏尾巴手动范围")
            row("尾巴配速色带自适应").uncheckedAction()
            compare(host.settings.route_tail_low_pace, 6.5)
            compare(host.settings.route_tail_high_pace, 4.25)
            compare(host.settings.route_low_pace, 15, "取消尾巴自适应不得修改路线端点")
            setRoute(1, 1, true, true)
        }

        // 显示控制点 = 关 → 隐藏控制点半径/颜色/透明度
        function test_control_rows_follow_switch() {
            setRoute(1, 1, true, true)
            verify(vis("控制点半径"), "显示控制点=开 要显示控制点半径")
            verify(vis("控制点颜色"), "显示控制点=开 要显示控制点颜色")
            verify(vis("控制点透明度 (%)"), "显示控制点=开 要显示控制点透明度")
            setRoute(1, 1, false, true)
            verify(!vis("控制点半径"), "显示控制点=关 必须隐藏控制点半径")
            verify(!vis("控制点颜色"), "显示控制点=关 必须隐藏控制点颜色")
            verify(!vis("控制点透明度 (%)"), "显示控制点=关 必须隐藏控制点透明度")
            verify(vis("运动员半径"), "只关了控制点，运动员半径仍要显示")
        }

        // 显示运动员 = 关 → 隐藏运动员半径/颜色/透明度（颜色也要跟着显隐）
        function test_runner_rows_follow_switch() {
            setRoute(1, 1, true, true)
            verify(vis("运动员半径"), "显示运动员=开 要显示运动员半径")
            verify(vis("运动员颜色"), "显示运动员=开 要显示运动员颜色")
            verify(vis("运动员透明度 (%)"), "显示运动员=开 要显示运动员透明度")
            setRoute(1, 1, true, false)
            verify(!vis("运动员半径"), "显示运动员=关 必须隐藏运动员半径")
            verify(!vis("运动员颜色"), "显示运动员=关 必须隐藏运动员颜色")
            verify(!vis("运动员透明度 (%)"), "显示运动员=关 必须隐藏运动员透明度")
            verify(vis("控制点半径"), "只关了运动员，控制点半径仍要显示")
        }

        // 运动员样式：圆形/箭头两项；只有「显示运动员」打开时才需要选
        function test_runner_style_menu() {
            var s = row("运动员样式")
            verify(s !== null, "缺少运动员样式选择器")
            compare(s.items.length, 2, "运动员样式只留两项")
            compare(s.items[0], "圆形")
            compare(s.items[1], "箭头")
            compare(s.values[0], 0)
            compare(s.values[1], 1)
            setRoute(1, 1, true, true)
            verify(vis("运动员样式"), "显示运动员=开 要显示运动员样式")
            setRoute(1, 1, true, false)
            verify(!vis("运动员样式"), "显示运动员=关 必须隐藏运动员样式")
        }

        // 「画面占比」原本在「轨迹文件」组，现在归到「显示」里：必须是显示组的第一行，
        // 且仍然只在有轨迹图片（地图）时出现。
        function test_canvas_share_row_is_first_in_display_group() {
            var share = row("画面占比 (%)")
            verify(share !== null, "画面占比 应该已经挪进「显示」组")
            compare(share.prop, "relative_width", "画面占比绑定的应是 relative_width")
            var labels = []
            function walk(item) {
                for (var i = 0; i < item.children.length; ++i) {
                    var c = item.children[i]
                    if (c.label) labels.push(c.label)
                    walk(c)
                }
            }
            walk(host)
            verify(labels.length > 0, "显示组里应该能收集到配置行")
            compare(labels[0], "画面占比 (%)", "「画面占比」应是显示组的第一行")
            host.trackHasMap = true
            verify(share.visible, "有轨迹图片时画面占比要显示")
            host.trackHasMap = false
            verify(!share.visible, "没有轨迹图片（视频铺满整帧）时画面占比要隐藏")
            host.trackHasMap = true
        }

        // 两个开关与两个模式选择器本身永远可见（它们不依赖别的模式）
        function test_always_visible_rows() {
            setRoute(0, 0, false, false)
            verify(vis("显示控制点"), "显示控制点开关本身要一直可见")
            verify(vis("显示运动员"), "显示运动员开关本身要一直可见")
            verify(vis("尾巴渲染模式"), "尾巴渲染模式选择器本身要一直可见")
            verify(vis("路线渲染模式"), "路线渲染模式选择器要一直可见")
        }

        // 渲染模式菜单只给三项，但存储值沿用原版编号 0/1/3（2 留给原版的高亮）
        function test_route_mode_menu_values() {
            var c = row("路线渲染模式")
            compare(c.items.length, 3, "路线渲染模式只留三项")
            compare(c.items[0], "无")
            compare(c.items[1], "简洁")
            compare(c.items[2], "配速")
            compare(c.values[0], 0)
            compare(c.values[1], 1)
            compare(c.values[2], 3)
        }

        // 尾巴现在是「无 / 简洁 / 配速」三态（用户在路线配速之后要求尾巴也支持配速，
        // 编号同样沿用原版 0/1/3）；不再是一个勾选框。
        function test_tail_mode_menu_and_values() {
            var c = row("尾巴渲染模式")
            verify(c !== null, "缺少「尾巴渲染模式」选择器")
            compare(c.items.length, 3, "尾巴渲染模式只留三项")
            compare(c.items[0], "无")
            compare(c.items[1], "简洁")
            compare(c.items[2], "配速")
            compare(c.values[0], 0)
            compare(c.values[1], 1)
            compare(c.values[2], 3)
            verify(row("显示尾巴") === null, "尾巴有三种模式，不该再是一个勾选框")
        }

        // 新增「渲染范围」：完整渲染（整条路线，默认）/ 单段渲染（只画当前所在的那一段）。
        // 存储值 0/1；老工程没有这个键、或写了别的值时按 0（完整）读，见 settings.rs。
        function test_render_scope_menu_and_visibility() {
            var c = row("渲染范围")
            verify(c !== null, "缺少「渲染范围」选择器")
            compare(c.items.length, 2, "渲染范围只留两项")
            compare(c.items[0], "完整渲染")
            compare(c.items[1], "单段渲染")
            compare(c.values[0], 0)
            compare(c.values[1], 1)
            // 路线不画的时候（模式=无）这个选项没有意义 → 跟着隐藏。
            setRoute(0, 1, true, true)
            verify(!vis("渲染范围"), "路线渲染模式=无 时必须隐藏渲染范围")
            setRoute(1, 1, true, true)
            verify(vis("渲染范围"), "简洁模式要显示渲染范围")
            setRoute(3, 1, true, true)
            verify(vis("渲染范围"), "配速模式要显示渲染范围")
            setRoute(1, 1, true, true)
        }

        // 用户要求「相关的配置放到一起」：按文档顺序收集所有行的标签，
        // 断言 路线 → 尾巴 → 控制点 → 运动员 四块依次出现，且每块内部顺序固定。
        function test_rows_are_grouped_in_order() {
            var labels = []
            function walk(item) {
                for (var i = 0; i < item.children.length; ++i) {
                    var c = item.children[i]
                    if (c.label) labels.push(c.label)
                    walk(c)
                }
            }
            function before(a, b, where) {
                verify(labels.indexOf(a) >= 0, where + ": 缺少「" + a + "」")
                verify(labels.indexOf(b) >= 0, where + ": 缺少「" + b + "」")
                verify(labels.indexOf(a) < labels.indexOf(b),
                       where + ": 「" + a + "」应排在「" + b + "」前面")
            }
            walk(host)
            // 组内第一行是「画面占比」（它从「轨迹文件」组挪进来的）
            before("画面占比 (%)", "路线渲染模式", "分组")
            // 四块的主题顺序
            before("路线渲染模式", "尾巴渲染模式", "分组")
            before("尾巴渲染模式", "显示控制点", "分组")
            before("显示控制点", "显示运动员", "分组")
            // 路线块内部
            before("路线渲染模式", "渲染范围", "路线")
            before("渲染范围", "路线宽度 (像素)", "路线")
            before("路线宽度 (像素)", "路线颜色", "路线")
            before("路线颜色", "路线透明度 (%)", "路线")
            before("路线透明度 (%)", "配速色带自适应", "路线")
            before("配速色带自适应", "低配速 (分钟/公里)", "路线")
            before("低配速 (分钟/公里)", "高配速 (分钟/公里)", "路线")
            // 尾巴块内部
            before("尾巴渲染模式", "尾巴宽度 (像素)", "尾巴")
            before("尾巴宽度 (像素)", "尾巴长度 (秒)", "尾巴")
            before("尾巴长度 (秒)", "尾巴颜色", "尾巴")
            before("尾巴颜色", "尾巴透明度 (%)", "尾巴")
            // 控制点块内部
            before("显示控制点", "控制点半径", "控制点")
            before("控制点半径", "控制点颜色", "控制点")
            before("控制点颜色", "控制点透明度 (%)", "控制点")
            // 运动员块内部
            before("显示运动员", "运动员样式", "运动员")
            before("运动员样式", "运动员半径", "运动员")
            before("运动员半径", "运动员颜色", "运动员")
            before("运动员颜色", "运动员透明度 (%)", "运动员")
            // 跨块：路线块整体在尾巴块之前
            before("路线透明度 (%)", "尾巴颜色", "分组")
            before("尾巴透明度 (%)", "控制点半径", "分组")
            before("控制点透明度 (%)", "运动员半径", "分组")
        }

        // 「显示」组里用二级小标题（SubSection）分区：四个区域的标题都在，且按 路线→尾巴→
        // 控制点→运动员 排列（用户要求「相关的配置放到一起」，并抱怨过原来的小标题太大）。
        function test_subsection_titles() {
            var titles = []
            function walk(item) {
                for (var i = 0; i < item.children.length; ++i) {
                    var c = item.children[i]
                    if (c.title !== undefined && c.title !== "") titles.push(c.title)
                    walk(c)
                }
            }
            walk(host)
            var want = ["显示", "路线", "尾巴", "控制点", "运动员"]
            for (var i = 0; i < want.length; ++i) {
                verify(titles.indexOf(want[i]) >= 0, "缺少标题: " + want[i])
            }
            verify(titles.indexOf("路线") < titles.indexOf("尾巴"), "路线区应在尾巴区之前")
            verify(titles.indexOf("尾巴") < titles.indexOf("控制点"), "尾巴区应在控制点区之前")
            verify(titles.indexOf("控制点") < titles.indexOf("运动员"), "控制点区应在运动员区之前")
        }

        // 每个颜色项都有配套的「透明度」设置项（用户要求：每一个都要加透明度），
        // 且透明度一律按**百分比**显示（存储仍是原版 0..255，见 SliderField.alphaPercent）。
        function test_every_color_has_opacity_row() {
            var pairs = [
                ["路线颜色", "路线透明度 (%)"],
                ["尾巴颜色", "尾巴透明度 (%)"],
                ["控制点颜色", "控制点透明度 (%)"],
                ["运动员颜色", "运动员透明度 (%)"]
            ]
            for (var i = 0; i < pairs.length; ++i) {
                var color = row(pairs[i][0])
                var opacity = row(pairs[i][1])
                verify(color !== null, "缺少颜色项: " + pairs[i][0])
                verify(opacity !== null, "缺少透明度项: " + pairs[i][1])
                compare(color.opacityProp, opacity.prop, pairs[i][0] + " 的色块要按 " + pairs[i][1] + " 显示")
                compare(opacity.from, 0, pairs[i][1] + " 的下限是 0")
                compare(opacity.to, 100, pairs[i][1] + " 的上限是 100（百分比）")
                verify(opacity.alphaPercent === true, pairs[i][1] + " 必须按百分比换算（alphaPercent）")
            }
            // 百分比是「显示单位」：设置里存的仍是原版 0..255 的 alpha，界面换算由 alphaPercent 负责。
            compare(row("路线透明度 (%)").decimals, 0, "百分比透明度不要小数")
        }

        // 「画面占比」在没有视频时不起作用（地图铺满整帧）：这一行要灰显只读、恒显 100%，
        // 并在悬停时说明原因。「没有视频」有两种来源：没选视频文件，或「导出」页没勾视频。
        function test_map_share_is_forced_full_without_video() {
            var share = row("画面占比 (%)")
            verify(share !== null, "缺少「画面占比 (%)」这一行")
            verify(share.forcedFull === false, "有视频时不该灰显")
            verify(String(share.hint).length > 0, "灰显时要有悬停提示文案")

            // 没选视频文件 → 灰显。
            host.settings.video_path = ""
            host.refreshTick++
            verify(share.forcedFull === true, "没有视频文件时「画面占比」要灰显")

            // 有视频文件，但「导出」页没勾视频 → 同样灰显。
            host.settings.video_path = "clip.mp4"
            host.settings.encode_include_video = false
            host.refreshTick++
            verify(share.forcedFull === true, "没勾「导出 → 视频」时「画面占比」要灰显")

            // 恢复：值本身没被改过，仍然是用户原先填的 0.75。
            host.settings.encode_include_video = true
            host.refreshTick++
            compare(share.forcedFull, false, "勾回视频后要能继续调")
            compare(host.settings.relative_width, 0.75, "灰显只是显示态，不许悄悄改掉存着的占比")
        }
    }
}
