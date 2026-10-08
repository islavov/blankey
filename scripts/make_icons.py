"""Render the app icon (document with padlock) to resources/blankey.icns and blankey.png."""

import os
import subprocess
import tempfile
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QGuiApplication, QPainter, QPixmap

from blankey.ui.icons import _draw

RESOURCES = Path(__file__).resolve().parent.parent / "resources"
SIZES = [16, 32, 64, 128, 256, 512, 1024]


def tile(size: int) -> QPixmap:
    """macOS-style rounded square with the document icon centered."""
    pixmap = QPixmap(size, size)
    pixmap.fill(Qt.GlobalColor.transparent)
    painter = QPainter(pixmap)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    margin = size * 0.1
    painter.setPen(Qt.PenStyle.NoPen)
    painter.setBrush(QColor("#f4f1ea"))
    painter.drawRoundedRect(margin, margin, size - 2 * margin, size - 2 * margin, size * 0.18, size * 0.18)
    inner = int(size * 0.62)
    painter.drawPixmap((size - inner) // 2 + int(size * 0.03), (size - inner) // 2, _draw(inner, False, False, False))
    painter.end()
    return pixmap


def main() -> None:
    QGuiApplication([])
    RESOURCES.mkdir(exist_ok=True)
    tile(1024).save(str(RESOURCES / "blankey.png"))
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "blankey.iconset"
        iconset.mkdir()
        for size in SIZES[:-1]:
            tile(size).save(str(iconset / f"icon_{size}x{size}.png"))
            tile(size * 2).save(str(iconset / f"icon_{size}x{size}@2x.png"))
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(RESOURCES / "blankey.icns")], check=True)


if __name__ == "__main__":
    main()
