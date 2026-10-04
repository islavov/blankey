"""Vector-drawn icons: a document with a padlock (open when the vault is unlocked)."""

import sys

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPainterPath, QPen, QPixmap

ACCENT = QColor("#2f7d5b")
ACCENT_LOCKED = QColor("#56606b")
BADGE = QColor("#e5484d")
PAPER = QColor("#ffffff")


def _document_path(size: float) -> QPainterPath:
    """Sheet with a folded top-right corner, in a size x size box (left part of the canvas)."""
    left, top = size * 0.14, size * 0.06
    right, bottom = size * 0.70, size * 0.86
    fold = size * 0.18
    radius = size * 0.06
    path = QPainterPath()
    path.moveTo(left + radius, top)
    path.lineTo(right - fold, top)
    path.lineTo(right, top + fold)
    path.lineTo(right, bottom - radius)
    path.quadTo(right, bottom, right - radius, bottom)
    path.lineTo(left + radius, bottom)
    path.quadTo(left, bottom, left, bottom - radius)
    path.lineTo(left, top + radius)
    path.quadTo(left, top, left + radius, top)
    return path


def _fold_path(size: float) -> QPainterPath:
    right, top, fold = size * 0.70, size * 0.06, size * 0.18
    path = QPainterPath()
    path.moveTo(right - fold, top)
    path.lineTo(right - fold, top + fold)
    path.lineTo(right, top + fold)
    return path


def _lock_paths(size: float, locked: bool) -> tuple[QPainterPath, QPainterPath]:
    body = QRectF(size * 0.50, size * 0.56, size * 0.42, size * 0.34)
    body_path = QPainterPath()
    body_path.addRoundedRect(body, size * 0.06, size * 0.06)
    shackle_w = body.width() * 0.62
    shackle_left = body.center().x() - shackle_w / 2
    shackle_top = body.top() - size * 0.20
    arc = QRectF(shackle_left, shackle_top, shackle_w, shackle_w)
    shackle = QPainterPath()
    if locked:
        shackle.moveTo(shackle_left, body.top())
        shackle.lineTo(shackle_left, arc.center().y())
        shackle.arcTo(arc, 180, -180)
        shackle.lineTo(shackle_left + shackle_w, body.top())
    else:
        # open: the shackle is lifted, the right leg stays in the body, the left leg is free
        lifted = arc.translated(0, -size * 0.09)
        shackle.moveTo(shackle_left, lifted.center().y() + size * 0.05)
        shackle.lineTo(shackle_left, lifted.center().y())
        shackle.arcTo(lifted, 180, -180)
        shackle.lineTo(shackle_left + shackle_w, body.top())
    return body_path, shackle


def _draw(size: int, locked: bool, pending: bool, mono: bool) -> QPixmap:
    pixmap = QPixmap(size, size)
    pixmap.fill(Qt.GlobalColor.transparent)
    p = QPainter(pixmap)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    ink = QColor("black") if mono else (ACCENT_LOCKED if locked else ACCENT)
    stroke = max(1.5, size * 0.075)

    doc = _document_path(size)
    if mono:
        p.setPen(QPen(ink, stroke, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap, Qt.PenJoinStyle.RoundJoin))
        p.setBrush(Qt.BrushStyle.NoBrush)
    else:
        p.setPen(QPen(ink, stroke, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap, Qt.PenJoinStyle.RoundJoin))
        p.setBrush(PAPER)
    p.drawPath(doc)
    p.setBrush(Qt.BrushStyle.NoBrush)
    p.drawPath(_fold_path(size))

    # text lines on the sheet
    line_pen = QPen(ink, stroke * 0.8, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap)
    p.setPen(line_pen)
    for i, end in enumerate((0.52, 0.44, 0.36)):
        y = size * (0.36 + i * 0.13)
        p.drawLine(QPointF(size * 0.26, y), QPointF(size * end, y))

    body, shackle = _lock_paths(size, locked)
    # knock out the area under the lock so it reads clearly over the sheet
    p.setCompositionMode(QPainter.CompositionMode.CompositionMode_Clear)
    p.setPen(QPen(Qt.GlobalColor.transparent, stroke * 2.2))
    p.setBrush(Qt.GlobalColor.transparent)
    p.drawPath(body)
    p.drawPath(shackle)
    p.setCompositionMode(QPainter.CompositionMode.CompositionMode_SourceOver)
    p.setPen(QPen(ink, stroke, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap, Qt.PenJoinStyle.RoundJoin))
    p.setBrush(Qt.BrushStyle.NoBrush)
    p.drawPath(shackle)
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(ink)
    p.drawPath(body)

    if pending:
        radius = size * 0.13
        center = QPointF(size - radius - 1, radius + 1)
        p.setCompositionMode(QPainter.CompositionMode.CompositionMode_Clear)
        p.drawEllipse(center, radius + stroke, radius + stroke)
        p.setCompositionMode(QPainter.CompositionMode.CompositionMode_SourceOver)
        p.setBrush(QColor("black") if mono else BADGE)
        p.drawEllipse(center, radius, radius)
    p.end()
    return pixmap


def tray_icon(locked: bool, pending: bool) -> QIcon:
    """Template (monochrome) icon on macOS so the menu bar tints it; coloured elsewhere."""
    mono = sys.platform == "darwin"
    icon = QIcon()
    for size in (16, 22, 32, 44, 64):
        icon.addPixmap(_draw(size, locked, pending, mono))
    icon.setIsMask(mono)
    return icon


def app_icon() -> QIcon:
    icon = QIcon()
    for size in (32, 64, 128, 256):
        icon.addPixmap(_draw(size, locked=False, pending=False, mono=False))
    return icon
