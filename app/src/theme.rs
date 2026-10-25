//! 界面主题：专业剪辑暗色（默认）与 Fluent 浅色。
//!
//! `Theme` 是 QML 单例，所有窗口与编辑器共享同一套语义令牌。除颜色外，
//! 圆角与间距也在这里定义，避免 QML 继续散落风格常量。切换主题只刷新视觉，
//! 不触发工程修改、视频重载或预览重载。

use qmetaobject::*;

/// 一套完整的界面语义令牌。
struct Palette {
    background: &'static str,
    background2: &'static str,
    surface: &'static str,
    surface_hover: &'static str,
    field: &'static str,
    button: &'static str,
    text: &'static str,
    accent: &'static str,
    muted: &'static str,
    disabled: &'static str,
    video_border: &'static str,
    text_on_accent: &'static str,
    hr: &'static str,
    popup_border: &'static str,
    focus: &'static str,
    slider_handle: &'static str,
    slider_background: &'static str,
    highlight: &'static str,
    danger: &'static str,
    warning: &'static str,
    success: &'static str,
    overlay: &'static str,
    radius_control: f64,
    radius_panel: f64,
}

/// 专业剪辑暗色：中性石墨层级，蓝色只用于焦点、当前状态和主要操作。
const DARK: Palette = Palette {
    background: "#18191c",
    background2: "#202226",
    surface: "#282b30",
    surface_hover: "#32363d",
    field: "#151619",
    button: "#282b30",
    text: "#e8eaed",
    accent: "#4c9aff",
    muted: "#9aa0a6",
    disabled: "#6f757d",
    video_border: "#2b2f34",
    text_on_accent: "#0b0d10",
    hr: "#2b2f34",
    popup_border: "#3a3e45",
    focus: "#6cb4ff",
    slider_handle: "#4c9aff",
    slider_background: "#454a52",
    highlight: "#29455f",
    danger: "#f26d6d",
    warning: "#e3b341",
    success: "#63c78c",
    overlay: "#b3101316",
    radius_control: 4.0,
    radius_panel: 6.0,
};

/// Fluent 浅色：浅灰工作区、白色输入与面板，保持紧凑的桌面工具密度。
const LIGHT: Palette = Palette {
    background: "#f3f3f3",
    background2: "#f9f9f9",
    surface: "#ffffff",
    surface_hover: "#f4f5f7",
    field: "#ffffff",
    button: "#ffffff",
    text: "#242424",
    accent: "#0f6cbd",
    muted: "#616161",
    disabled: "#9e9e9e",
    video_border: "#d1d1d1",
    text_on_accent: "#ffffff",
    hr: "#e5e5e5",
    popup_border: "#d1d1d1",
    focus: "#0f6cbd",
    slider_handle: "#ffffff",
    slider_background: "#d1d1d1",
    highlight: "#eaf3fb",
    danger: "#c42b1c",
    warning: "#9d5d00",
    success: "#0f7b0f",
    overlay: "#33000000",
    radius_control: 4.0,
    radius_panel: 8.0,
};

#[derive(QObject, Default)]
pub struct Theme {
    base: qt_base_class!(trait QObject),

    /// true = 专业剪辑暗色，false = Fluent 浅色。
    dark: qt_property!(bool; NOTIFY theme_changed),
    theme_changed: qt_signal!(),
    /// Application preference, independent of every project.
    preference: qt_property!(QString; NOTIFY theme_changed),

    background: qt_property!(QString; NOTIFY theme_changed),
    background2: qt_property!(QString; NOTIFY theme_changed),
    surface: qt_property!(QString; NOTIFY theme_changed),
    surface_hover: qt_property!(QString; NOTIFY theme_changed),
    field: qt_property!(QString; NOTIFY theme_changed),
    button: qt_property!(QString; NOTIFY theme_changed),
    text: qt_property!(QString; NOTIFY theme_changed),
    accent: qt_property!(QString; NOTIFY theme_changed),
    muted: qt_property!(QString; NOTIFY theme_changed),
    disabled: qt_property!(QString; NOTIFY theme_changed),
    video_border: qt_property!(QString; NOTIFY theme_changed),
    text_on_accent: qt_property!(QString; NOTIFY theme_changed),
    hr: qt_property!(QString; NOTIFY theme_changed),
    popup_border: qt_property!(QString; NOTIFY theme_changed),
    focus: qt_property!(QString; NOTIFY theme_changed),
    slider_handle: qt_property!(QString; NOTIFY theme_changed),
    slider_background: qt_property!(QString; NOTIFY theme_changed),
    highlight: qt_property!(QString; NOTIFY theme_changed),
    danger: qt_property!(QString; NOTIFY theme_changed),
    warning: qt_property!(QString; NOTIFY theme_changed),
    success: qt_property!(QString; NOTIFY theme_changed),
    overlay: qt_property!(QString; NOTIFY theme_changed),
    radius_control: qt_property!(f64; NOTIFY theme_changed),
    radius_panel: qt_property!(f64; NOTIFY theme_changed),

    /// 中文界面字体；由操作系统提供中文回退。
    font_family: qt_property!(QString; NOTIFY theme_changed),
    /// 缩放系数：屏幕 DPI / 96。
    dpi_scale: qt_property!(f64; NOTIFY theme_changed),

    /// 切换主题：`dark` / `light`（system 由 QML 解析后传入）。
    apply_theme: qt_method!(
        fn apply_theme(&mut self, name: QString) {
            let dark = name.to_string() != "light";
            self.set_palette(dark);
        }
    ),
    set_preference: qt_method!(
        fn set_preference(&mut self, name: QString) {
            if !matches!(name.to_string().as_str(), "light" | "dark" | "system") {
                return;
            }
            if self.preference != name {
                if !crate::qtwrap::save_theme_preference(&name) {
                    ::log::warn!("无法保存当前用户的外观偏好；本次会话仍会应用所选主题");
                }
                self.preference = name;
                self.theme_changed();
            }
        }
    ),
}

impl Theme {
    fn set_palette(&mut self, dark: bool) {
        let p = if dark { &DARK } else { &LIGHT };
        self.dark = dark;
        self.background = QString::from(p.background);
        self.background2 = QString::from(p.background2);
        self.surface = QString::from(p.surface);
        self.surface_hover = QString::from(p.surface_hover);
        self.field = QString::from(p.field);
        self.button = QString::from(p.button);
        self.text = QString::from(p.text);
        self.accent = QString::from(p.accent);
        self.muted = QString::from(p.muted);
        self.disabled = QString::from(p.disabled);
        self.video_border = QString::from(p.video_border);
        self.text_on_accent = QString::from(p.text_on_accent);
        self.hr = QString::from(p.hr);
        self.popup_border = QString::from(p.popup_border);
        self.focus = QString::from(p.focus);
        self.slider_handle = QString::from(p.slider_handle);
        self.slider_background = QString::from(p.slider_background);
        self.highlight = QString::from(p.highlight);
        self.danger = QString::from(p.danger);
        self.warning = QString::from(p.warning);
        self.success = QString::from(p.success);
        self.overlay = QString::from(p.overlay);
        self.radius_control = p.radius_control;
        self.radius_panel = p.radius_panel;
        self.theme_changed();
    }
}

impl QSingletonInit for Theme {
    fn init(&mut self) {
        let saved = crate::qtwrap::load_theme_preference();
        self.preference = if matches!(saved.to_string().as_str(), "light" | "dark" | "system") {
            saved
        } else {
            QString::from("dark")
        };
        self.font_family = QString::from("Microsoft YaHei UI");
        let scale = std::env::var("QT_SCALE_FACTOR")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0);
        self.dpi_scale = scale;
        self.set_palette(self.preference.to_string() != "light");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn professional_dark_and_fluent_light_keep_distinct_field_layers() {
        assert_eq!(DARK.background, "#18191c");
        assert_eq!(DARK.field, "#151619");
        assert_eq!(DARK.accent, "#4c9aff");
        assert_eq!(LIGHT.background, "#f3f3f3");
        assert_eq!(LIGHT.field, "#ffffff");
        assert_eq!(LIGHT.accent, "#0f6cbd");
        assert!(DARK.radius_panel <= 6.0);
        assert_eq!(LIGHT.radius_panel, 8.0);
    }
}
