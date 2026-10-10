//! Qt wrapper helpers: construct Qt types that qttypes does not bind, using `cpp!`.
//!
//! qttypes exposes QPainter/QImage/QPixmap/QPen/QBrush/QRectF/QPointF/QColor but
//! lacks a QImage constructor from raw pixels and an image size accessor. These
//! helpers fill that gap.

use cpp::cpp;
use qmetaobject::qttypes::{QPainter, QPen};
use qmetaobject::QImage;
use std::ops::{Deref, DerefMut};

cpp! {{
    #include <QtGui/QImage>
    #include <QtGui/QPainter>
    #include <QtGui/QPen>
    #include <QtGui/QBrush>
    #include <QtGui/QTransform>
    #include <QtQuick/QQuickItem>
    #include <QtQuick/QQuickWindow>
    #include <QtGui/QGuiApplication>
    #include <QtGui/QIcon>
    #include <QtCore/QCoreApplication>
    #include <cstdlib>
    #include <cmath>
    #include <crtdbg.h>
    #include <windows.h>
    #include <cstdint>
    #include <cstring>

    // A QPainter that lives at a STABLE heap address. qttypes' by-value return memcpy's
    // the object; text rendering (QPainter::drawText) dereferences internal pointers set
    // up at construction time, so a moved-into QPainter faults. Heap allocation keeps the
    // address fixed for the painter's whole life.
    static QPainter *ov_painter_new(QImage *img) { return new QPainter(img); }
    static void ov_painter_delete(QPainter *p) { delete p; }

    #include "src/asserts_silent.h"
}}

/// Select the customizable controls style before creating the QML engine.
pub fn configure_gui_style() {
    crate::branding::register_resources();
    // Qt reads the C runtime environment on Windows; Rust's set_var updates the
    // Win32 environment, which may leave Qt's runtime copy unchanged.
    unsafe {
        cpp!([] { qputenv("QT_QUICK_CONTROLS_STYLE", QByteArray("Fusion")); });
    }
}

/// Set native application identity and icon after QmlEngine creates the app.
pub fn configure_application_icon() {
    unsafe {
        cpp!([] {
            QCoreApplication::setApplicationName(QStringLiteral("TraceReel"));
            QGuiApplication::setApplicationDisplayName(
                QStringLiteral("TraceReel ") + QChar(0x00b7) + QChar(' ') + QChar(0x8ff9) + QChar(0x6620));
            QGuiApplication::setWindowIcon(QIcon(QStringLiteral(":/brand/icon.png")));
        });
    }
}

/// Pump Qt events for offscreen smoke tests without entering the permanent app loop.
pub fn process_events() {
    unsafe {
        cpp!([] { QCoreApplication::processEvents(); });
    }
}

// Qt::PenCapStyle / Qt::PenJoinStyle 的原始数值（qttypes 0.2.12 只绑定了 QPen 的
// from_color/from_style/set_color/set_style/set_width，**没有** setCapStyle/setJoinStyle）。
pub const PEN_CAP_FLAT: i32 = 0x00;
pub const PEN_CAP_SQUARE: i32 = 0x10;
pub const PEN_CAP_ROUND: i32 = 0x20;
pub const PEN_JOIN_MITER: i32 = 0x00;
pub const PEN_JOIN_BEVEL: i32 = 0x40;
pub const PEN_JOIN_ROUND: i32 = 0x80;

/// 设置画笔的端头/拐角样式（等价于原版 `pen.setCapStyle(cap); pen.setJoinStyle(join);`）。
///
/// 原版 ov 的路线与尾巴笔画都用 `Qt::RoundCap` + `Qt::RoundJoin`（Renderer.cpp 的
/// routeRenderMode / tailRenderMode 分支），线宽 10px 时端头形状肉眼可见，必须跟着设，
/// 否则两端是平头、折点是尖角，跟原版对不上。
pub fn pen_set_cap_join(pen: &mut QPen, cap_style: i32, join_style: i32) {
    unsafe {
        cpp!([pen as "QPen *", cap_style as "int", join_style as "int"] {
            pen->setCapStyle(static_cast<Qt::PenCapStyle>(cap_style));
            pen->setJoinStyle(static_cast<Qt::PenJoinStyle>(join_style));
        });
    }
}

/// 把画笔的刷子换成「起点色 → 终点色」的线性渐变（两点都是逻辑坐标）。
///
/// Qt 的线性渐变默认用 LogicalMode，也就是绘制时的世界坐标；所以只要把渐变端点给成这一
/// 段的两个端点，描边出来的颜色就沿这一段线性过渡，相邻两段的端点色相同 → 整条带子的
/// 颜色连续。`qttypes` 没有暴露 `QLinearGradient`，只能 cpp!。
pub fn pen_set_linear_gradient(
    pen: &mut QPen,
    from: (f64, f64),
    to: (f64, f64),
    from_rgb: (u8, u8, u8),
    to_rgb: (u8, u8, u8),
) {
    let (x1, y1) = from;
    let (x2, y2) = to;
    let (r1, g1, b1) = (from_rgb.0 as i32, from_rgb.1 as i32, from_rgb.2 as i32);
    let (r2, g2, b2) = (to_rgb.0 as i32, to_rgb.1 as i32, to_rgb.2 as i32);
    unsafe {
        cpp!([pen as "QPen *", x1 as "double", y1 as "double", x2 as "double", y2 as "double",
              r1 as "int", g1 as "int", b1 as "int", r2 as "int", g2 as "int", b2 as "int"] {
            QLinearGradient gradient(x1, y1, x2, y2);
            gradient.setColorAt(0.0, QColor(r1, g1, b1));
            gradient.setColorAt(1.0, QColor(r2, g2, b2));
            pen->setBrush(QBrush(gradient));
        });
    }
}

/// 画家当前世界变换的等效缩放（设备像素 / 用户单位）。
///
/// `qttypes` 没有暴露 `QPainter::worldTransform()`，所以这里用 cpp! 取：把 (0,0) 和 (1,1)
/// 两个用户点映到设备空间，取对角线长度的均值。用来给离屏位图选分辨率 —— 位图的 1 个像素
/// 对应 1 个设备像素时，画出来的线宽与直接画在画布上完全一致，最后合成只是 1:1 拷贝。
pub fn painter_scale(p: &QPainter) -> f64 {
    unsafe {
        cpp!([p as "QPainter *"] -> f64 as "double" {
            const QTransform t = p->worldTransform();
            const QPointF o = t.map(QPointF(0, 0));
            const QPointF u = t.map(QPointF(1, 1));
            const double dx = u.x() - o.x();
            const double dy = u.y() - o.y();
            return std::sqrt((dx * dx + dy * dy) / 2.0);
        })
    }
}

/// 让 Qt / MSVC 调试运行时的断言**只打一行日志，永不弹模态对话框**。
///
/// 为什么需要它：Qt 的 debug 运行库（`Qt6Cored.dll`）里 `Q_ASSERT` 失败默认会弹一个
/// 断言对话框（标题栏写着模块名，例如 `Qt6Cored.dll`，正文是
/// `ASSERT: "..." in file ...\qqmlengine.cpp, line 1441`）。在无人值守/自动化跑的时候
/// 这个框会把进程一直挂住，等有人点「忽略」才继续 —— 有弹窗就没人能测。
///
/// `_set_error_mode` 只管普通运行时错误，**断言框由 CRT 的报告模式决定**，所以还要显式
/// 把 `_CRT_ASSERT` / `_CRT_ERROR` / `_CRT_WARN` 改成写文件（stderr）。这些符号只在
/// debug CRT 里存在，因此用 `LoadLibrary + GetProcAddress` 动态取，release 构建下自动跳过。
///
/// 可反复调用，无副作用。
/// 让 MSVC/Qt 调试运行时的断言走 stderr（`_set_error_mode` + 关掉 abort 弹框）。
///
/// 注意：这**不足以**消掉 Qt 的断言弹框，只是把消息写进 stderr；`Q_ASSERT` 失败本身
/// 仍然会终止进程（除非有人在弹框上点「忽略」）。真正的解法是把触发断言的那处 QML
/// 修掉 —— 见 `app/src/bin/gui.qml` 里 `TrackList` / `VideoList` / `SplitList` 的注释。
pub fn test_runtime_errors_to_stderr() {
    unsafe {
        cpp!([] {
            _set_error_mode(_OUT_TO_STDERR);
            _set_abort_behavior(0, _WRITE_ABORT_MSG | _CALL_REPORTFAULT);
        });
    }
}/// 让 CRT 断言**不弹框、且让进程继续**（等价于自动点「忽略」），断言原文仍打到 stderr。
///
/// GUI 与所有无头工具都调用它；这就是「无人值守也不会被断言框卡住」的机制。
pub fn asserts_silent_continue() {
    unsafe {
        cpp!([] { ov_asserts_silent_continue(); });
    }
}
pub fn save_first_window(path: &str) -> bool {
    let path = qmetaobject::QString::from(path);
    unsafe {
        cpp!([path as "QString"] -> bool as "bool" {
            for(auto w:QGuiApplication::topLevelWindows()) {
                if(auto quick=qobject_cast<QQuickWindow *>(w)) {
                    auto img=quick->grabWindow();
                    if(!img.isNull()) return img.save(path);
                }
            }
            return false;
        })
    }
}

/// A heap-allocated QPainter bound to a QImage. Deref-mut to `QPainter` so callers use
/// it transparently; dropping it ends painting and frees the object.
pub struct PainterHandle {
    ptr: *mut QPainter,
}

impl PainterHandle {
    /// Construct a QPainter drawing onto `img`. The image must outlive the handle.
    pub fn on_image(img: &mut QImage) -> Self {
        let ptr = unsafe {
            cpp!([img as "QImage *"] -> *mut std::ffi::c_void as "QPainter *" {
                return ov_painter_new(img);
            })
        };
        PainterHandle {
            ptr: ptr as *mut QPainter,
        }
    }
}

impl Deref for PainterHandle {
    type Target = QPainter;
    fn deref(&self) -> &QPainter {
        unsafe { &*self.ptr }
    }
}

impl DerefMut for PainterHandle {
    fn deref_mut(&mut self) -> &mut QPainter {
        unsafe { &mut *self.ptr }
    }
}

impl Drop for PainterHandle {
    fn drop(&mut self) {
        let ptr = self.ptr as *mut std::ffi::c_void;
        unsafe {
            cpp!([ptr as "QPainter *"] {
                ov_painter_delete(ptr);
            });
        }
    }
}

/// Build a QImage that owns a copy of RGBA8 data (width×height×4).
pub fn qimage_from_rgba(width: i32, height: i32, data: &[u8]) -> QImage {
    let mut d = data.to_vec();
    let p = d.as_mut_ptr() as *mut u8;
    unsafe {
        cpp!([p as "unsigned char *", width as "int", height as "int"] -> QImage as "QImage" {
            QImage img(static_cast<const uchar *>(p), width, height, width * 4, QImage::Format_RGBA8888);
            // copy so the QImage owns its pixels (the Rust buffer is dropped after).
            return img.copy();
        })
    }
}

/// Build an empty ARGB32 QImage (a standard format that supports text rendering).
pub fn qimage_new_argb32(width: i32, height: i32) -> QImage {
    unsafe {
        cpp!([width as "int", height as "int"] -> QImage as "QImage" {
            return QImage(width, height, QImage::Format_ARGB32);
        })
    }
}

/// Build a fully transparent ARGB32 QImage. `QImage(w,h,Format_ARGB32)` leaves the buffer
/// uninitialised, so an offscreen layer needs this explicit transparent fill (painting a
/// transparent color with SourceOver would not clear anything).
pub fn qimage_new_transparent(width: i32, height: i32) -> QImage {
    unsafe {
        cpp!([width as "int", height as "int"] -> QImage as "QImage" {
            QImage img(width, height, QImage::Format_ARGB32);
            img.fill(QColor(0, 0, 0, 0));
            return img;
        })
    }
}

/// Shallow (implicitly shared) copy of a QImage: Qt 的 QImage 拷贝构造只加引用计数，
/// 不复制像素 —— 用来既把离屏位图留在缓存里，又能把它交给画家合成。
pub fn qimage_clone(img: &QImage) -> QImage {
    unsafe {
        cpp!([img as "QImage *"] -> QImage as "QImage" {
            return QImage(*img);
        })
    }
}

/// Build a QImage that owns a copy of RGB24 data (width×height×3).
pub fn qimage_from_rgb(width: i32, height: i32, data: &[u8]) -> QImage {
    let mut d = data.to_vec();
    let p = d.as_mut_ptr() as *mut u8;
    unsafe {
        cpp!([p as "unsigned char *", width as "int", height as "int"] -> QImage as "QImage" {
            QImage img(static_cast<const uchar *>(p), width, height, width * 3, QImage::Format_RGB888);
            return img.copy();
        })
    }
}

/// Return a QImage's width.
pub fn qimage_width(img: &QImage) -> i32 {
    unsafe {
        cpp!([img as "QImage *"] -> i32 as "int" {
            return img->width();
        })
    }
}

/// Return a QImage's height.
pub fn qimage_height(img: &QImage) -> i32 {
    unsafe {
        cpp!([img as "QImage *"] -> i32 as "int" {
            return img->height();
        })
    }
}

/// Return a QQuickItem's width (from the underlying C++ object pointer).
pub fn qquick_item_width(obj: *mut std::ffi::c_void) -> f64 {
    unsafe {
        cpp!([obj as "QQuickItem *"] -> f64 as "double" {
            QQuickItem *item = static_cast<QQuickItem*>(obj);
            return item ? item->width() : 0.0;
        })
    }
}

/// Return a QQuickItem's height (from the underlying C++ object pointer).
pub fn qquick_item_height(obj: *mut std::ffi::c_void) -> f64 {
    unsafe {
        cpp!([obj as "QQuickItem *"] -> f64 as "double" {
            QQuickItem *item = static_cast<QQuickItem*>(obj);
            return item ? item->height() : 0.0;
        })
    }
}

/// Create a QPainter drawing onto a QImage. Returns a heap-stable [`PainterHandle`]
/// (deref-mut to `QPainter`); drop it to end painting before reading the image back.
pub fn qpainter_on_image(img: &mut QImage) -> PainterHandle {
    PainterHandle::on_image(img)
}

/// Save a QImage to a file path; returns true on success.
pub fn qimage_save(img: &QImage, path: &str) -> bool {
    let p = std::ffi::CString::new(path).unwrap_or_default();
    let ptr = p.as_ptr();
    unsafe {
        cpp!([img as "QImage *", ptr as "char *"] -> bool as "bool" {
            return img->save(QString::fromUtf8(ptr));
        })
    }
}

/// Copy a QImage's native ARGB32 bytes straight out as BGRA (width*height*4), with NO
/// format conversion. The encode canvas is `Format_ARGB32`; on little-endian its memory
/// layout is B,G,R,A per pixel — exactly ffmpeg's `bgra` rawvideo input. Because draw_scene
/// fills the whole canvas with opaque pixels, premultiplied vs straight is a non-issue
/// (alpha is 255 everywhere). This replaces the old ARGB→RGBA swizzle + a 3-byte pack loop
/// (the single biggest per-frame CPU cost in the export pipeline).
pub fn qimage_to_bgra(img: &QImage) -> Vec<u8> {
    let w = qimage_width(img) as i32;
    let h = qimage_height(img) as i32;
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    let ptr = out.as_mut_ptr();
    unsafe {
        cpp!([img as "QImage *", ptr as "unsigned char *", w as "int", h as "int"] {
            // Ensure ARGB32 (native BGRA). If the paint device left it premultiplied or
            // another format, a single convertToFormat normalizes it; for the already-ARGB32
            // canvas this is a cheap path. Then copy each row honouring bytesPerLine padding.
            QImage c = (img->format() == QImage::Format_ARGB32 || img->format() == QImage::Format_ARGB32_Premultiplied)
                           ? *img : img->convertToFormat(QImage::Format_ARGB32);
            const int bpl = c.bytesPerLine();
            for (int y = 0; y < h; ++y) {
                std::memcpy(ptr + (size_t)y * w * 4, c.constBits() + (size_t)y * bpl, (size_t)w * 4);
            }
        });
    }
    out
}

/// Copy a QImage's pixels as RGBA8 into a Vec (width*height*4).
pub fn qimage_to_rgba(img: &QImage) -> Vec<u8> {
    let w = qimage_width(img) as i32;
    let h = qimage_height(img) as i32;
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    let ptr = out.as_mut_ptr();
    unsafe {
        cpp!([img as "QImage *", ptr as "unsigned char *", w as "int", h as "int"] {
            QImage c = img->convertToFormat(QImage::Format_RGBA8888);
            for (int y = 0; y < h; ++y) {
                std::memcpy(ptr + y * w * 4, c.constBits() + y * c.bytesPerLine(), (size_t)w * 4);
            }
        });
    }
    out
}
