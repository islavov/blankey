"""macOS-native touches: SF Symbols as Qt icons. Everything degrades to plain Qt elsewhere."""

import sys
from functools import cache

from PySide6.QtCore import QByteArray, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPixmap

MAC = sys.platform == "darwin"


@cache
def _symbol_pixmap(name: str, point_size: float) -> QPixmap | None:
    if not MAC:
        return None
    import AppKit

    image = AppKit.NSImage.imageWithSystemSymbolName_accessibilityDescription_(name, None)
    if image is None:
        return None
    config = AppKit.NSImageSymbolConfiguration.configurationWithPointSize_weight_(
        point_size * 2, AppKit.NSFontWeightRegular
    )
    rep = AppKit.NSBitmapImageRep.imageRepWithData_(image.imageWithSymbolConfiguration_(config).TIFFRepresentation())
    png = rep.representationUsingType_properties_(AppKit.NSBitmapImageFileTypePNG, {})
    pixmap = QPixmap()
    if not pixmap.loadFromData(QByteArray(bytes(png))):
        return None
    pixmap.setDevicePixelRatio(2)
    return pixmap


def tinted(pixmap: QPixmap, color: QColor) -> QPixmap:
    out = QPixmap(pixmap.size())
    out.setDevicePixelRatio(pixmap.devicePixelRatio())
    out.fill(Qt.GlobalColor.transparent)
    painter = QPainter(out)
    painter.drawPixmap(0, 0, pixmap)
    painter.setCompositionMode(QPainter.CompositionMode.CompositionMode_SourceIn)
    painter.fillRect(out.rect(), color)
    painter.end()
    return out


def symbol_icon(name: str, color: QColor | None = None, point_size: float = 15) -> QIcon:
    """An SF Symbol as an icon; a template (mask) icon unless a color is given. Empty off macOS."""
    pixmap = _symbol_pixmap(name, point_size)
    if pixmap is None:
        return QIcon()
    if color is not None:
        return QIcon(tinted(pixmap, color))
    icon = QIcon(pixmap)
    icon.setIsMask(True)
    return icon


def symbol_pixmap(name: str, color: QColor, point_size: float = 15) -> QPixmap | None:
    pixmap = _symbol_pixmap(name, point_size)
    return tinted(pixmap, color) if pixmap is not None else None
