"""Render marketing screenshots of the main window and the dialogs from a freshly seeded demo vault.

    uv run python scripts/screenshots.py [output-dir]

Uses the native platform (SF Symbols need AppKit), so windows briefly appear on screen. Every screen is captured
in light and dark mode at the screen's device pixel ratio. The real data dir and keychain are never touched.
"""

import argparse
import dataclasses
import sys
import tempfile
from pathlib import Path

import keyring
from PySide6.QtCore import QPointF, QRectF, QSize, Qt
from PySide6.QtGui import QColor, QDesktopServices, QGuiApplication, QPainter, QPainterPath, QPalette, QPixmap
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication, QPushButton, QWidget

sys.path.insert(0, str(Path(__file__).parent))
from demo_seed import PASSWORD, seed

from blankey.config import load_config
from blankey.core import Blankey, RequestKind
from blankey.ui import icons
from blankey.ui.fill import FillDialog
from blankey.ui.main_window import MainWindow
from blankey.ui.pages import ID_ROLE, ActivityPage, DocumentsPage, FillSetsPage, TemplatesPage
from blankey.ui.profiles import ProfilesPage
from blankey.ui.requests import ProfileInputDialog

DEFAULT_OUT = Path(__file__).resolve().parents[2] / "blankey-io" / "assets" / "img" / "screens"
WINDOW = QSize(1280, 800)
TITLE_BAR = 28
CORNER = 10
LIGHTS = ("#ff5f57", "#febc2e", "#28c840")
SCHEMES = {"light": Qt.ColorScheme.Light, "dark": Qt.ColorScheme.Dark}
PROFILE_FIELDS = [
    {"key": "passport_no", "label": "Passport number", "required": True},
    {"key": "tax_id", "label": "Tax ID"},
    {"key": "phone", "label": "Mobile", "type": "phone"},
    {"key": "iban", "label": "IBAN", "type": "iban"},
]


def isolate_keyring() -> None:
    store: dict[tuple[str, str], str] = {}
    keyring.get_password = lambda s, u: store.get((s, u))
    keyring.set_password = lambda s, u, p: store.__setitem__((s, u), p)
    keyring.delete_password = lambda s, u: store.pop((s, u), None)


def settle(ms: int = 600) -> None:
    QApplication.processEvents()
    QTest.qWait(ms)


def framed(widget: QWidget, chrome_height: int = 0) -> QPixmap:
    """The grab inside a drawn macOS frame. Native parts (the unified toolbar's backdrop, the window background
    behind transparent views) come out transparent in a grab, so the window color is painted underneath."""
    shot = widget.grab()
    ratio = shot.devicePixelRatio()
    width, height = widget.width(), widget.height() + TITLE_BAR
    out = QPixmap(QSize(width, height) * ratio)
    out.setDevicePixelRatio(ratio)
    out.fill(Qt.GlobalColor.transparent)
    window = widget.palette().color(QPalette.ColorRole.Window)
    dark = window.lightness() < 128
    bar = window.lighter(112) if dark else window.lighter(104)
    painter = QPainter(out)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    outline = QPainterPath()
    outline.addRoundedRect(QRectF(0, 0, width, height), CORNER, CORNER)
    painter.setClipPath(outline)
    painter.fillRect(QRectF(0, 0, width, height), window)
    painter.fillRect(QRectF(0, 0, width, TITLE_BAR + chrome_height), bar)
    for number, color in enumerate(LIGHTS):
        painter.setPen(QColor(0, 0, 0, 40))
        painter.setBrush(QColor(color))
        painter.drawEllipse(QPointF(20 + number * 20, TITLE_BAR / 2), 6, 6)
    if not chrome_height and widget.windowTitle():
        painter.setPen(widget.palette().color(QPalette.ColorRole.WindowText))
        painter.drawText(QRectF(0, 0, width, TITLE_BAR), Qt.AlignmentFlag.AlignCenter, widget.windowTitle())
    painter.setPen(QColor(0, 0, 0, 60) if not dark else QColor(0, 0, 0, 160))
    painter.drawLine(QPointF(0, TITLE_BAR + chrome_height - 0.5), QPointF(width, TITLE_BAR + chrome_height - 0.5))
    painter.drawPixmap(0, TITLE_BAR, shot)
    painter.setClipping(False)
    painter.setBrush(Qt.BrushStyle.NoBrush)
    painter.setPen(QColor(255, 255, 255, 40) if dark else QColor(0, 0, 0, 50))
    painter.drawRoundedRect(QRectF(0.5, 0.5, width - 1, height - 1), CORNER, CORNER)
    painter.end()
    return out


def save(widget: QWidget, path: Path, chrome_height: int = 0) -> None:
    settle()
    pixmap = framed(widget, chrome_height)
    if not pixmap.save(str(path), "PNG"):
        raise RuntimeError(f"could not write {path}")
    print(f"{path}  {pixmap.width()}x{pixmap.height()}")


def show_fixed(widget: QWidget, size: QSize) -> None:
    widget.setWindowState(Qt.WindowState.WindowNoState)
    widget.setFixedSize(size)
    widget.move(40, 60)
    widget.show()
    widget.raise_()
    settle()


def chrome(window: MainWindow) -> int:
    return window.centralWidget().geometry().top()


def capture_window(app: Blankey, scheme: str, out: Path) -> None:
    window = MainWindow(app, on_lock=lambda: None, on_quit=lambda: None)
    show_fixed(window, WINDOW)
    window.show_page(TemplatesPage)
    page = window.current_page()
    page.list.setCurrentRow(
        next(r for r in range(page.list.count()) if page.list.item(r).data(ID_ROLE) == "service-agreement")
    )
    save(window, out / f"templates-{scheme}.png", chrome(window))

    window.show_page(ProfilesPage)
    page = window.current_page()
    page.profiles.setCurrentRow(
        next(r for r in range(page.profiles.count()) if page.profiles.item(r).text().startswith("Jane"))
    )
    save(window, out / f"profiles-{scheme}.png", chrome(window))

    for page_type, name in ((FillSetsPage, "fill-sets"), (DocumentsPage, "documents"), (ActivityPage, "activity")):
        window.show_page(page_type)
        save(window, out / f"{name}-{scheme}.png", chrome(window))
    window.hide()
    window.deleteLater()


def capture_fill(app: Blankey, scheme: str, out: Path) -> None:
    request = next(r for r in app.vault.list_requests("pending") if r.kind == RequestKind.FILL)
    payload = request.payload
    dialog = FillDialog(
        app,
        payload["templates"],
        {role: int(pid) for role, pid in payload["profiles"].items()},
        payload["name"],
        payload["values"],
        payload["paths"],
        request=request,
    )
    show_fixed(dialog, WINDOW)
    dialog.table.clearSelection()
    generate = next(b for b in dialog.findChildren(QPushButton) if b.text() == "Save and Generate")
    generate.setFocus()
    settle(1200)
    save(dialog, out / f"fill-dialog-{scheme}.png")
    dialog.hide()  # not reject(): that would resolve the request
    dialog.deleteLater()


def capture_profile_input(app: Blankey, scheme: str, out: Path) -> None:
    template = next(r for r in app.vault.list_requests("pending") if r.kind == RequestKind.FILL)
    jane = next(p.id for p in app.vault.list_profiles() if p.name == "Jane Example")
    request = dataclasses.replace(
        template,
        kind=RequestKind.PROFILE_INPUT,
        payload={"profile_id": jane, "fields": PROFILE_FIELDS, "reason": "Passport details for the visa application"},
    )
    dialog = ProfileInputDialog(app, request)
    dialog.inputs[0][2].setText("533812947")
    show_fixed(dialog, QSize(720, dialog.sizeHint().height()))
    save(dialog, out / f"profile-input-{scheme}.png")
    dialog.hide()
    dialog.deleteLater()


def set_scheme(scheme: str) -> None:
    QGuiApplication.styleHints().setColorScheme(SCHEMES[scheme])
    settle(800)
    window = QGuiApplication.palette().color(QPalette.ColorRole.Window)
    dark = window.lightness() < 128
    if dark != (scheme == "dark"):
        raise RuntimeError(f"palette did not switch to {scheme}: window color {window.name()}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out", type=Path, nargs="?", default=DEFAULT_OUT)
    args = parser.parse_args()
    out = args.out.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)

    isolate_keyring()
    qt_app = QApplication(sys.argv)
    qt_app.setApplicationName("Blankey")
    qt_app.setWindowIcon(icons.app_icon())
    QDesktopServices.openUrl = lambda url: True
    with tempfile.TemporaryDirectory(prefix="blankey-shots-") as tmp:
        root = Path(tmp) / "demo"
        seed(root, full=True)
        app = Blankey(load_config(root / "data"))
        app.vault.unlock(PASSWORD)
        try:
            for scheme in SCHEMES:
                set_scheme(scheme)
                capture_window(app, scheme, out)
                capture_fill(app, scheme, out)
                capture_profile_input(app, scheme, out)
        finally:
            QGuiApplication.styleHints().unsetColorScheme()
            app.vault.close()


if __name__ == "__main__":
    main()
