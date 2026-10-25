"""用 PyQt6 离屏渲染三套 QML 视觉概念图。"""
from __future__ import annotations

import os
import sys
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
os.environ.setdefault("QT_QUICK_BACKEND", "software")

from PyQt6.QtCore import QTimer, QUrl
from PyQt6.QtGui import QFontDatabase, QGuiApplication
from PyQt6.QtQml import QQmlEngine
from PyQt6.QtQuick import QQuickView

HERE = Path(__file__).resolve().parent
OUT = HERE / "renders"
CONCEPTS = [
    (0, "01-professional-dark.png"),
    (1, "02-fluent-light.png"),
    (2, "03-linear-modern.png"),
]


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    app = QGuiApplication(sys.argv[:1])
    app.setApplicationName("TraceReel UI Concepts")
    # PyQt 的离屏平台不会自动枚举 Windows 字体目录，显式注册中文字体。
    for font_file in ("msyh.ttc", "msyhbd.ttc", "segoeui.ttf", "segoeuib.ttf"):
        font_path = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts" / font_file
        if font_path.exists():
            font_id = QFontDatabase.addApplicationFont(str(font_path))
            if font_id < 0:
                print(f"字体注册失败：{font_path}", file=sys.stderr)
    views: list[QQuickView] = []
    pending = len(CONCEPTS)
    failures: list[str] = []

    def render_next(index: int) -> None:
        nonlocal pending
        if index >= len(CONCEPTS):
            app.quit()
            return
        concept, filename = CONCEPTS[index]
        view = QQuickView()
        views.append(view)
        view.setResizeMode(QQuickView.ResizeMode.SizeRootObjectToView)
        view.resize(1440, 960)
        view.rootContext().setContextProperty("conceptIndex", concept)
        view.setSource(QUrl.fromLocalFile(str(HERE / "ConceptShowcase.qml")))
        if view.status() == QQuickView.Status.Error:
            for error in view.errors():
                failures.append(error.toString())
            pending -= 1
            QTimer.singleShot(0, lambda: render_next(index + 1))
            return
        view.show()

        def capture() -> None:
            nonlocal pending
            image = view.grabWindow()
            target = OUT / filename
            if image.isNull() or not image.save(str(target)):
                failures.append(f"无法保存 {target}")
            else:
                print(f"已生成 {target}（{image.width()}×{image.height()}）")
            view.hide()
            pending -= 1
            QTimer.singleShot(50, lambda: render_next(index + 1))

        QTimer.singleShot(400, capture)

    render_next(0)
    app.exec()
    for failure in failures:
        print(failure, file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
