import QtQuick
import QtQuick.Controls

Rectangle {
    id: root
    width: 1440
    height: 960
    color: background

    // 由渲染脚本传入：0 专业剪辑，1 Fluent，2 现代极简。
    property int concept: typeof conceptIndex !== "undefined" ? conceptIndex : 0
    property bool light: concept === 1
    property color background: concept === 0 ? "#15171A" : concept === 1 ? "#F3F4F6" : "#0E1014"
    property color panel: concept === 0 ? "#1D2024" : concept === 1 ? "#FAFAFA" : "#14171C"
    property color surface: concept === 0 ? "#25292E" : concept === 1 ? "#FFFFFF" : "#1B1F26"
    property color surfaceHover: concept === 0 ? "#2C3137" : concept === 1 ? "#F5F7F9" : "#232832"
    property color border: concept === 0 ? "#34393F" : concept === 1 ? "#DFE2E6" : "#292F39"
    property color textMain: concept === 0 ? "#F1F3F5" : concept === 1 ? "#202124" : "#E9EBF2"
    property color textMuted: concept === 0 ? "#9BA3AC" : concept === 1 ? "#69707A" : "#9299A8"
    property color accent: concept === 0 ? "#49A6E9" : concept === 1 ? "#0F6CBD" : "#8B7CF6"
    property color accentSoft: concept === 0 ? "#243B4C" : concept === 1 ? "#EAF3FB" : "#29243F"
    property color accentText: "#FFFFFF"
    property int radius: concept === 0 ? 5 : concept === 1 ? 7 : 11
    property string uiFont: "Microsoft YaHei UI"

    component LabelText: Text {
        color: root.textMain
        font.family: root.uiFont
        font.pixelSize: 14
        verticalAlignment: Text.AlignVCenter
    }

    component IconButton: Rectangle {
        property string label: ""
        property bool active: false
        width: 38
        height: 38
        radius: root.radius
        color: active ? root.accentSoft : "transparent"
        border.width: active && root.concept === 0 ? 1 : 0
        border.color: root.accent
        LabelText {
            anchors.centerIn: parent
            text: parent.label
            color: parent.active ? root.accent : root.textMuted
            font.pixelSize: 17
            font.bold: parent.active
        }
    }

    component MockButton: Rectangle {
        property string label: ""
        property bool primary: false
        property bool quiet: false
        height: 36
        radius: root.concept === 2 ? 8 : root.radius
        color: primary ? root.accent : quiet ? "transparent" : root.surfaceHover
        border.width: primary || quiet ? 0 : 1
        border.color: root.border
        LabelText {
            anchors.centerIn: parent
            text: parent.label
            color: parent.primary ? root.accentText : parent.quiet ? root.textMuted : root.textMain
            font.pixelSize: 13
            font.bold: parent.primary
        }
    }

    component SectionTitle: Item {
        property string title: ""
        property string detail: ""
        height: 28
        LabelText {
            text: parent.title
            font.pixelSize: 15
            font.bold: true
        }
        LabelText {
            anchors.right: parent.right
            text: parent.detail
            color: root.textMuted
            font.pixelSize: 12
        }
    }

    component Toggle: Item {
        property bool checked: true
        width: 38
        height: 22
        Rectangle {
            anchors.fill: parent
            radius: 11
            color: parent.checked ? root.accent : root.surfaceHover
            border.width: parent.checked ? 0 : 1
            border.color: root.border
        }
        Rectangle {
            x: parent.checked ? 19 : 3
            y: 3
            width: 16
            height: 16
            radius: 8
            color: parent.checked ? "#FFFFFF" : root.textMuted
        }
    }

    component InputField: Item {
        property string title: ""
        property string value: ""
        property string suffix: ""
        height: 66
        LabelText {
            text: parent.title
            anchors.left: parent.left
            anchors.top: parent.top
            height: 24
            color: root.textMuted
            font.pixelSize: 13
        }
        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 36
            radius: root.concept === 2 ? 8 : root.radius
            color: root.light ? "#FFFFFF" : root.background
            border.width: 1
            border.color: root.border
            LabelText {
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.right: suffixLabel.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                text: value
                font.pixelSize: 13
                elide: Text.ElideMiddle
            }
            LabelText {
                id: suffixLabel
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                text: suffix
                color: root.textMuted
                font.pixelSize: 16
            }
        }
    }

    component SelectField: Item {
        property string title: ""
        property string value: ""
        height: 66
        LabelText {
            text: parent.title
            anchors.left: parent.left
            anchors.top: parent.top
            height: 24
            color: root.textMuted
            font.pixelSize: 13
        }
        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 36
            radius: root.concept === 2 ? 8 : root.radius
            color: root.light ? "#FFFFFF" : root.surfaceHover
            border.width: 1
            border.color: root.border
            LabelText {
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: value
                font.pixelSize: 13
            }
            LabelText {
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: "∨"
                color: root.textMuted
                font.pixelSize: 17
            }
        }
    }

    component SliderField: Item {
        property string title: ""
        property string value: ""
        property real progress: 0.5
        height: 72
        LabelText {
            text: parent.title
            anchors.left: parent.left
            anchors.top: parent.top
            height: 24
            color: root.textMuted
            font.pixelSize: 13
        }
        Rectangle {
            anchors.right: parent.right
            anchors.top: parent.top
            width: 72
            height: 30
            radius: root.concept === 2 ? 7 : root.radius
            color: root.light ? "#FFFFFF" : root.background
            border.width: 1
            border.color: root.border
            LabelText {
                anchors.centerIn: parent
                text: value
                font.pixelSize: 12
            }
        }
        Item {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 28
            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                height: root.concept === 0 ? 4 : 5
                radius: 3
                color: root.border
            }
            Rectangle {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width * progress
                height: root.concept === 0 ? 4 : 5
                radius: 3
                color: root.accent
            }
            Rectangle {
                x: parent.width * progress - width / 2
                anchors.verticalCenter: parent.verticalCenter
                width: root.concept === 0 ? 16 : 18
                height: width
                radius: width / 2
                color: root.light ? "#FFFFFF" : root.accent
                border.width: root.light ? 2 : root.concept === 2 ? 4 : 0
                border.color: root.light ? root.accent : root.panel
            }
        }
    }

    // 顶部标题栏
    Rectangle {
        id: topBar
        width: parent.width
        height: 64
        color: root.panel
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: root.border }
        Rectangle {
            x: 20; y: 15; width: 34; height: 34; radius: root.concept === 2 ? 10 : 8
            color: root.accent
            Canvas {
                anchors.fill: parent
                onPaint: {
                    var ctx = getContext("2d")
                    ctx.clearRect(0, 0, width, height)
                    ctx.strokeStyle = "#FFFFFF"
                    ctx.lineWidth = 2.4
                    ctx.lineCap = "round"
                    ctx.lineJoin = "round"
                    ctx.beginPath()
                    ctx.moveTo(8, 24)
                    ctx.lineTo(13, 16)
                    ctx.lineTo(19, 20)
                    ctx.lineTo(26, 9)
                    ctx.stroke()
                    ctx.fillStyle = "#FFFFFF"
                    ctx.beginPath(); ctx.arc(8, 24, 2.3, 0, Math.PI * 2); ctx.fill()
                    ctx.beginPath(); ctx.arc(26, 9, 2.3, 0, Math.PI * 2); ctx.fill()
                }
            }
        }
        LabelText { x: 66; y: 12; height: 24; text: "迹映"; font.pixelSize: 18; font.bold: true }
        LabelText { x: 66; y: 35; height: 16; text: "TRACEREEL"; color: root.textMuted; font.pixelSize: 9; font.letterSpacing: 1.6 }
        Rectangle { x: 160; y: 18; width: 1; height: 28; color: root.border }
        LabelText { x: 180; y: 18; height: 28; text: "狮山定向赛"; font.pixelSize: 14; font.bold: true }
        LabelText { x: 278; y: 20; height: 24; text: "已自动保存"; color: root.textMuted; font.pixelSize: 12 }
        Rectangle {
            anchors.right: parent.right; anchors.rightMargin: 22; anchors.verticalCenter: parent.verticalCenter
            width: 84; height: 30; radius: 15; color: root.accentSoft
            Rectangle { x: 12; anchors.verticalCenter: parent.verticalCenter; width: 7; height: 7; radius: 4; color: root.accent }
            LabelText { x: 27; anchors.verticalCenter: parent.verticalCenter; text: "准备就绪"; color: root.accent; font.pixelSize: 12 }
        }
    }

    // 左侧功能导航
    Rectangle {
        id: navigation
        x: 0; y: 64; width: 80; height: parent.height - 64
        color: root.panel
        Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: root.border }
        Column {
            x: 10; y: 18; width: 60; spacing: 10
            Repeater {
                model: ["轨迹", "分段", "视频", "仪表", "导出"]
                delegate: Rectangle {
                    width: 60; height: 58; radius: root.concept === 2 ? 10 : root.radius
                    color: index === 2 ? root.accentSoft : "transparent"
                    border.width: index === 2 && root.concept === 0 ? 1 : 0
                    border.color: root.accent
                    Rectangle {
                        visible: index === 2 && root.concept === 0
                        anchors.left: parent.left; anchors.leftMargin: -10; anchors.verticalCenter: parent.verticalCenter
                        width: 3; height: 24; radius: 2; color: root.accent
                    }
                    LabelText {
                        anchors.top: parent.top; anchors.topMargin: 8; anchors.horizontalCenter: parent.horizontalCenter
                        text: ["迹", "段", "影", "表", "出"][index]
                        color: index === 2 ? root.accent : root.textMuted
                        font.pixelSize: 14
                        font.bold: index === 2
                    }
                    LabelText {
                        anchors.bottom: parent.bottom; anchors.bottomMargin: 6; anchors.horizontalCenter: parent.horizontalCenter
                        text: modelData
                        color: index === 2 ? root.accent : root.textMuted
                        font.pixelSize: 11
                        font.bold: index === 2
                    }
                }
            }
        }
        IconButton { anchors.bottom: parent.bottom; anchors.bottomMargin: 18; anchors.horizontalCenter: parent.horizontalCenter; label: "?" }
    }

    // 中央预览工作区
    Rectangle {
        id: workspace
        x: 80; y: 64; width: 888; height: parent.height - 64
        color: root.background
        LabelText { x: 28; y: 24; height: 28; text: "视频预览"; font.pixelSize: 18; font.bold: true }
        LabelText { x: 28; y: 53; height: 20; text: "所有调整都会即时反映到画面"; color: root.textMuted; font.pixelSize: 12 }
        Rectangle {
            anchors.right: parent.right; anchors.rightMargin: 26; anchors.top: parent.top; anchors.topMargin: 26
            width: 132; height: 30; radius: root.concept === 2 ? 15 : root.radius
            color: root.surface; border.width: 1; border.color: root.border
            LabelText { anchors.centerIn: parent; text: "1920 × 1080 · 60"; color: root.textMuted; font.pixelSize: 11 }
        }

        // 预览画布：示意地图 + 路线 + 视频 + 运动仪表
        Rectangle {
            id: preview
            x: 28; y: 94; width: parent.width - 56; height: 470
            color: "#101316"
            radius: root.concept === 2 ? 13 : root.radius
            border.width: 1
            border.color: root.border
            clip: true
            Rectangle {
                x: 0; y: 0; width: parent.width * 0.56; height: parent.height
                color: root.light ? "#DDE4DF" : "#20282A"
                Canvas {
                    anchors.fill: parent
                    onPaint: {
                        var ctx = getContext("2d")
                        ctx.clearRect(0, 0, width, height)
                        ctx.lineWidth = 1
                        ctx.strokeStyle = root.light ? "#CFD8D1" : "#2A3536"
                        for (var i = -height; i < width + height; i += 42) {
                            ctx.beginPath(); ctx.moveTo(i, 0); ctx.lineTo(i + height, height); ctx.stroke()
                        }
                        ctx.strokeStyle = root.light ? "#F7F8F5" : "#354143"
                        ctx.lineWidth = 11
                        ctx.lineCap = "round"
                        ctx.beginPath(); ctx.moveTo(-20, 110); ctx.bezierCurveTo(130, 80, 220, 190, 500, 110); ctx.stroke()
                        ctx.beginPath(); ctx.moveTo(105, -20); ctx.bezierCurveTo(120, 130, 360, 260, 280, 500); ctx.stroke()
                        ctx.strokeStyle = root.light ? "#C5D0C8" : "#303C3D"
                        ctx.lineWidth = 4
                        ctx.beginPath(); ctx.moveTo(-10, 370); ctx.bezierCurveTo(150, 290, 300, 410, 490, 300); ctx.stroke()
                        ctx.strokeStyle = root.light ? "#167C80" : "#52D3C7"
                        ctx.lineWidth = 5
                        ctx.shadowColor = root.light ? "#40167C80" : "#8052D3C7"
                        ctx.shadowBlur = 8
                        ctx.beginPath()
                        ctx.moveTo(68, 390)
                        ctx.bezierCurveTo(82, 300, 190, 350, 190, 260)
                        ctx.bezierCurveTo(190, 165, 320, 260, 365, 160)
                        ctx.bezierCurveTo(390, 105, 350, 80, 405, 58)
                        ctx.stroke()
                        ctx.shadowBlur = 0
                        ctx.fillStyle = root.light ? "#167C80" : "#52D3C7"
                        ctx.beginPath(); ctx.arc(68, 390, 7, 0, Math.PI * 2); ctx.fill()
                        ctx.fillStyle = "#FFFFFF"
                        ctx.beginPath(); ctx.arc(68, 390, 3, 0, Math.PI * 2); ctx.fill()
                        ctx.fillStyle = root.accent
                        ctx.beginPath(); ctx.arc(259, 218, 9, 0, Math.PI * 2); ctx.fill()
                        ctx.fillStyle = "#FFFFFF"
                        ctx.beginPath(); ctx.arc(259, 218, 4, 0, Math.PI * 2); ctx.fill()
                    }
                }
                Rectangle {
                    x: 18; y: 18; width: 90; height: 28; radius: 6; color: "#B0000000"
                    LabelText { anchors.centerIn: parent; text: "轨迹地图"; color: "#FFFFFF"; font.pixelSize: 11 }
                }
                Rectangle {
                    x: 18; anchors.bottom: parent.bottom; anchors.bottomMargin: 18; width: 150; height: 58; radius: 8; color: "#C0101416"
                    LabelText { x: 12; y: 7; text: "当前分段"; color: "#B9C2C7"; font.pixelSize: 10 }
                    LabelText { x: 12; y: 24; text: "第 3 段 · 02:18"; color: "#FFFFFF"; font.pixelSize: 15; font.bold: true }
                }
            }
            Rectangle {
                x: parent.width * 0.56; y: 0; width: parent.width * 0.44; height: parent.height
                gradient: Gradient {
                    GradientStop { position: 0; color: root.light ? "#879EAD" : "#354958" }
                    GradientStop { position: 0.58; color: root.light ? "#617C86" : "#25373E" }
                    GradientStop { position: 1; color: "#182429" }
                }
                // 抽象跑道与远景，避免依赖外部素材
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 170; color: "#70352F2A" }
                Canvas {
                    anchors.fill: parent
                    onPaint: {
                        var ctx = getContext("2d")
                        ctx.clearRect(0, 0, width, height)
                        ctx.fillStyle = "#40312E2B"
                        ctx.beginPath(); ctx.moveTo(0, 220); ctx.lineTo(60, 160); ctx.lineTo(110, 220); ctx.lineTo(180, 135); ctx.lineTo(260, 220); ctx.lineTo(370, 150); ctx.lineTo(370, 300); ctx.lineTo(0, 300); ctx.fill()
                        ctx.strokeStyle = "#70D6C6A0"; ctx.lineWidth = 2
                        for (var i = 0; i < 5; ++i) {
                            ctx.beginPath(); ctx.moveTo(130 + i * 30, 470); ctx.lineTo(174 + i * 8, 278); ctx.stroke()
                        }
                        // 跑者剪影
                        ctx.strokeStyle = "#101416"; ctx.fillStyle = "#101416"; ctx.lineWidth = 12; ctx.lineCap = "round"
                        ctx.beginPath(); ctx.arc(195, 223, 15, 0, Math.PI * 2); ctx.fill()
                        ctx.beginPath(); ctx.moveTo(190, 245); ctx.lineTo(177, 312); ctx.lineTo(146, 371); ctx.stroke()
                        ctx.beginPath(); ctx.moveTo(181, 288); ctx.lineTo(222, 332); ctx.lineTo(235, 390); ctx.stroke()
                        ctx.beginPath(); ctx.moveTo(187, 258); ctx.lineTo(145, 278); ctx.lineTo(120, 250); ctx.stroke()
                        ctx.beginPath(); ctx.moveTo(188, 260); ctx.lineTo(228, 264); ctx.lineTo(248, 237); ctx.stroke()
                    }
                }
                Rectangle {
                    x: 18; y: 18; width: 90; height: 28; radius: 6; color: "#B0000000"
                    LabelText { anchors.centerIn: parent; text: "防抖视频"; color: "#FFFFFF"; font.pixelSize: 11 }
                }
                // 仪表数据
                Column {
                    x: 20; anchors.bottom: parent.bottom; anchors.bottomMargin: 20; spacing: 4
                    LabelText { text: "04:32"; color: "#FFFFFF"; font.pixelSize: 38; font.bold: true; font.letterSpacing: -1 }
                    LabelText { text: "每公里配速"; color: "#D6DDE1"; font.pixelSize: 11 }
                }
                Column {
                    anchors.right: parent.right; anchors.rightMargin: 20; anchors.bottom: parent.bottom; anchors.bottomMargin: 22; spacing: 5
                    LabelText { text: "8.42 公里"; color: "#FFFFFF"; font.pixelSize: 16; font.bold: true; horizontalAlignment: Text.AlignRight; anchors.right: parent.right }
                    LabelText { text: "142 次/分 · 心率"; color: "#D6DDE1"; font.pixelSize: 11; anchors.right: parent.right }
                }
            }
        }

        // 播放控制
        Rectangle {
            x: 28; y: 582; width: parent.width - 56; height: 86
            radius: root.concept === 2 ? 12 : root.radius
            color: root.surface
            border.width: 1; border.color: root.border
            Rectangle {
                x: 18; anchors.verticalCenter: parent.verticalCenter; width: 42; height: 42; radius: 21; color: root.accent
                LabelText { anchors.centerIn: parent; anchors.horizontalCenterOffset: 2; text: "播"; color: "#FFFFFF"; font.pixelSize: 16 }
            }
            LabelText { x: 76; y: 17; text: "02:18.42"; font.pixelSize: 14; font.bold: true }
            LabelText { x: 150; y: 19; text: "/  05:36.00"; color: root.textMuted; font.pixelSize: 12 }
            Item {
                x: 76; y: 48; width: parent.width - 166; height: 20
                Rectangle { y: 8; width: parent.width; height: 4; radius: 2; color: root.border }
                Rectangle { y: 8; width: parent.width * 0.42; height: 4; radius: 2; color: root.accent }
                Rectangle { x: parent.width * 0.42 - 5; y: 4; width: 12; height: 12; radius: 6; color: root.accent }
                Rectangle { x: parent.width * 0.22; y: 5; width: 2; height: 10; color: root.textMuted }
                Rectangle { x: parent.width * 0.68; y: 5; width: 2; height: 10; color: root.textMuted }
            }
            LabelText { anchors.right: parent.right; anchors.rightMargin: 18; anchors.verticalCenter: parent.verticalCenter; text: "1×"; color: root.textMuted; font.pixelSize: 13 }
        }

        LabelText { x: 30; y: 696; text: "当前片段"; font.pixelSize: 15; font.bold: true }
        Row {
            x: 28; y: 732; width: parent.width - 56; height: 96; spacing: 12
            Repeater {
                model: ["出发与检录", "第 1 段", "第 2 段", "第 3 段"]
                delegate: Rectangle {
                    width: 194; height: 92; radius: root.concept === 2 ? 10 : root.radius
                    color: index === 3 ? root.accentSoft : root.surface
                    border.width: 1; border.color: index === 3 ? root.accent : root.border
                    Rectangle {
                        x: 10; y: 10; width: 74; height: 46; radius: 5
                        color: ["#34434A", "#495449", "#394B55", "#4D4341"][index]
                        Canvas {
                            anchors.fill: parent
                            property int segment: index
                            onPaint: {
                                var ctx = getContext("2d"); ctx.strokeStyle = "#B7D7D1"; ctx.lineWidth = 2
                                ctx.beginPath(); ctx.moveTo(6, 38); ctx.lineTo(23, 25); ctx.lineTo(39, 30); ctx.lineTo(62, 9); ctx.stroke()
                            }
                        }
                    }
                    LabelText { x: 94; y: 11; text: modelData; font.pixelSize: 12; font.bold: index === 3 }
                    LabelText { x: 94; y: 34; text: ["00:00", "00:42", "01:26", "02:18"][index]; color: root.textMuted; font.pixelSize: 11 }
                    LabelText { x: 10; anchors.bottom: parent.bottom; anchors.bottomMargin: 8; text: index === 3 ? "正在预览" : "分段 " + (index + 1); color: index === 3 ? root.accent : root.textMuted; font.pixelSize: 10 }
                }
            }
        }
    }

    // 右侧设置面板
    Rectangle {
        id: inspector
        x: 968; y: 64; width: parent.width - 968; height: parent.height - 64
        color: root.panel
        Rectangle { anchors.left: parent.left; width: 1; height: parent.height; color: root.border }

        // 页签
        Item {
            width: parent.width; height: 58
            Row {
                anchors.horizontalCenter: parent.horizontalCenter; anchors.verticalCenter: parent.verticalCenter
                spacing: root.concept === 2 ? 5 : 0
                Repeater {
                    model: ["轨迹", "分段", "视频", "仪表", "导出"]
                    delegate: Rectangle {
                        width: 82; height: root.concept === 2 ? 34 : 58
                        radius: root.concept === 2 ? 8 : 0
                        color: index === 2 && root.concept !== 0 ? root.accentSoft : "transparent"
                        LabelText {
                            anchors.centerIn: parent; text: modelData
                            color: index === 2 ? root.accent : root.textMuted
                            font.pixelSize: 13; font.bold: index === 2
                        }
                        Rectangle {
                            visible: index === 2 && root.concept !== 2
                            anchors.bottom: parent.bottom; anchors.horizontalCenter: parent.horizontalCenter
                            width: root.concept === 0 ? parent.width : 24; height: 2; color: root.accent
                        }
                    }
                }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: root.border }
        }

        LabelText { x: 24; y: 80; text: "视频与防抖"; font.pixelSize: 20; font.bold: true }
        LabelText { x: 24; y: 111; text: "让运动画面更平稳、更清晰"; color: root.textMuted; font.pixelSize: 12 }

        // 视频源卡片
        Rectangle {
            x: 20; y: 150; width: parent.width - 40; height: 154
            color: root.concept === 0 ? "transparent" : root.surface
            radius: root.concept === 2 ? 12 : root.radius
            border.width: root.concept === 0 ? 0 : 1; border.color: root.border
            SectionTitle { x: root.concept === 0 ? 4 : 16; y: 12; width: parent.width - (root.concept === 0 ? 8 : 32); title: "视频源"; detail: "1 个文件" }
            InputField {
                x: root.concept === 0 ? 4 : 16; y: 48; width: parent.width - (root.concept === 0 ? 8 : 32)
                title: "文件路径"; value: "D:\\比赛\\狮山\\主视频.mp4"; suffix: "…"
            }
            LabelText { x: root.concept === 0 ? 4 : 16; anchors.bottom: parent.bottom; anchors.bottomMargin: 12; text: "2688 × 2016 · 60 帧/秒 · 02:18:36"; color: root.textMuted; font.pixelSize: 11 }
            Rectangle { visible: root.concept === 0; anchors.bottom: parent.bottom; width: parent.width; height: 1; color: root.border }
        }

        // 防抖卡片
        Rectangle {
            x: 20; y: 320; width: parent.width - 40; height: 358
            color: root.concept === 0 ? "transparent" : root.surface
            radius: root.concept === 2 ? 12 : root.radius
            border.width: root.concept === 0 ? 0 : 1; border.color: root.border
            SectionTitle { x: root.concept === 0 ? 4 : 16; y: 12; width: parent.width - (root.concept === 0 ? 8 : 32); title: "视频防抖"; detail: "GYROFLOW" }
            LabelText { x: root.concept === 0 ? 4 : 16; y: 50; text: "启用防抖"; font.pixelSize: 13 }
            Toggle { anchors.right: parent.right; anchors.rightMargin: root.concept === 0 ? 4 : 16; y: 49; checked: true }
            SelectField {
                x: root.concept === 0 ? 4 : 16; y: 86; width: parent.width - (root.concept === 0 ? 8 : 32)
                title: "平滑方式"; value: "默认平滑（推荐）"
            }
            SliderField {
                x: root.concept === 0 ? 4 : 16; y: 166; width: parent.width - (root.concept === 0 ? 8 : 32)
                title: "平滑度"; value: "65%"; progress: 0.65
            }
            SliderField {
                x: root.concept === 0 ? 4 : 16; y: 248; width: parent.width - (root.concept === 0 ? 8 : 32)
                title: "缩放限额"; value: "130%"; progress: 0.30
            }
            LabelText { x: root.concept === 0 ? 4 : 16; anchors.bottom: parent.bottom; anchors.bottomMargin: 12; text: "＋  高级选项"; color: root.textMuted; font.pixelSize: 12 }
            Rectangle { visible: root.concept === 0; anchors.bottom: parent.bottom; width: parent.width; height: 1; color: root.border }
        }

        // 状态提示与主按钮
        Rectangle {
            x: 24; y: 700; width: parent.width - 48; height: 48; radius: root.concept === 2 ? 9 : root.radius
            color: root.accentSoft
            Rectangle { x: 14; anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: root.accent }
            LabelText { x: 32; anchors.verticalCenter: parent.verticalCenter; text: "镜头标定已匹配 · 可以预览"; color: root.accent; font.pixelSize: 12 }
        }
        Item {
            x: 24; anchors.bottom: parent.bottom; anchors.bottomMargin: 24; width: parent.width - 48; height: 40
            MockButton { anchors.left: parent.left; width: 128; height: 40; label: "重置参数"; quiet: root.concept !== 0 }
            MockButton { anchors.right: parent.right; width: 174; height: 40; label: "导出视频"; primary: true }
        }
    }
}
