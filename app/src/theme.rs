//! 主题（配色令牌）—— 对齐原版 Gyroflow 的调色板与尺寸约定。
//!
//! 原版在 `gyroflow-src/src/ui/ui_tools.rs::set_theme()` 里把十来个 `style*` 变量注入 QML 根上下文
//! （浅色/深色两套），所有组件都只引用这些变量。这里做成 **QML 单例** `Theme`：
//! 好处是 `gui.qml`、`HudEditor.qml`、`HudCanvas.qml`、`HudColorDialog.qml` 都能直接 `Theme.accent` 引用，
//! 且带 NOTIFY，切换主题时绑定会即时刷新（上下文属性做不到这点）。
//!
//! 用法（QML）：`import TraceReel 1.0` 后直接 `Theme.accent`、`Theme.background`、`Theme.dpiScale`。
//! 取值与 `ui_tools.rs:94-125` 逐字一致。

use qmetaobject::*;

/// 一套调色板（原版 12 个令牌）。
struct Palette {
    background: &'static str,
    background2: &'static str,
    button: &'static str,
    text: &'static str,
    accent: &'static str,
    /// 次要文字（原版用 styleTextColor + 透明度；这里给个明确的灰阶便于直接用）。
    muted: &'static str,
    video_border: &'static str,
    text_on_accent: &'static str,
    hr: &'static str,
    popup_border: &'static str,
    slider_handle: &'static str,
    slider_background: &'static str,
    highlight: &'static str,
}

/// 浅色（原版 `ui_tools.rs:110-124`）。
const LIGHT: Palette = Palette {
    background: "#f9f9f9",
    background2: "#f3f3f3",
    button: "#fbfbfb",
    text: "#111111",
    accent: "#116cad",
    muted: "#6b6b6b",
    video_border: "#d5d5d5",
    text_on_accent: "#ffffff",
    hr: "#e5e5e5",
    popup_border: "#d5d5d5",
    slider_handle: "#c2c2c2",
    slider_background: "#cdcdcd",
    highlight: "#10000000",
};

/// 深色（原版 `ui_tools.rs:95-108`）。
const DARK: Palette = Palette {
    background: "#1e1e1e",
    background2: "#191919",
    button: "#282828",
    text: "#ffffff",
    accent: "#76baed",
    muted: "#a8a8a8",
    video_border: "#2b2b2b",
    text_on_accent: "#000000",
    hr: "#2e2e2e",
    popup_border: "#0f0f0f",
    slider_handle: "#454545",
    slider_background: "#949494",
    highlight: "#10ffffff",
};

#[derive(QObject, Default)]
pub struct Theme {
    base: qt_base_class!(trait QObject),

    /// true = 深色主题。
    dark: qt_property!(bool; NOTIFY theme_changed),
    /// 「跟随系统」解析后的实际主题名（"light" / "dark"）。
    theme_changed: qt_signal!(),

    background: qt_property!(QString; NOTIFY theme_changed),
    background2: qt_property!(QString; NOTIFY theme_changed),
    button: qt_property!(QString; NOTIFY theme_changed),
    text: qt_property!(QString; NOTIFY theme_changed),
    accent: qt_property!(QString; NOTIFY theme_changed),
    muted: qt_property!(QString; NOTIFY theme_changed),
    video_border: qt_property!(QString; NOTIFY theme_changed),
    text_on_accent: qt_property!(QString; NOTIFY theme_changed),
    hr: qt_property!(QString; NOTIFY theme_changed),
    popup_border: qt_property!(QString; NOTIFY theme_changed),
    slider_handle: qt_property!(QString; NOTIFY theme_changed),
    slider_background: qt_property!(QString; NOTIFY theme_changed),
    highlight: qt_property!(QString; NOTIFY theme_changed),

    /// 原版字体族（`set_theme()` 里固定 Arial）。
    font_family: qt_property!(QString; NOTIFY theme_changed),
    /// 缩放系数：屏幕 DPI / 96（原版所有尺寸都乘它）。
    dpi_scale: qt_property!(f64; NOTIFY theme_changed),

    /// 切换主题：`dark` / `light` / `system`（system 由 QML 用 `Qt.styleHints` 解析后传 light|dark）。
    apply_theme: qt_method!(
        fn apply_theme(&mut self, name: QString) {
            let dark = name.to_string() == "dark";
            self.set_palette(dark);
        }
    ),
}

impl Theme {
    fn set_palette(&mut self, dark: bool) {
        let p = if dark { &DARK } else { &LIGHT };
        self.dark = dark;
        self.background = QString::from(p.background);
        self.background2 = QString::from(p.background2);
        self.button = QString::from(p.button);
        self.text = QString::from(p.text);
        self.accent = QString::from(p.accent);
        self.muted = QString::from(p.muted);
        self.video_border = QString::from(p.video_border);
        self.text_on_accent = QString::from(p.text_on_accent);
        self.hr = QString::from(p.hr);
        self.popup_border = QString::from(p.popup_border);
        self.slider_handle = QString::from(p.slider_handle);
        self.slider_background = QString::from(p.slider_background);
        self.highlight = QString::from(p.highlight);
        self.theme_changed();
    }
}

impl QSingletonInit for Theme {
    fn init(&mut self) {
        self.font_family = QString::from("Arial");
        // DPI 缩放：和 main.rs / run.ps1 的环境（-platform windows）一致取屏幕逻辑 DPI。
        // 拿不到时退回 1.0（QML 侧也只用它乘尺寸，1.0 即原样）。
        let scale = std::env::var("QT_SCALE_FACTOR")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0);
        self.dpi_scale = scale;
        self.set_palette(false);
    }
}
