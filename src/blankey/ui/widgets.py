import tempfile
from pathlib import Path

from PySide6.QtCore import QEvent, QRectF, Qt, QUrl, Signal
from PySide6.QtGui import QAction, QColor, QDesktopServices, QFont, QGuiApplication, QPainter, QPalette, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QMessageBox,
    QScrollArea,
    QStyle,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QTableWidget,
    QVBoxLayout,
    QWidget,
)

from blankey.ui import macos
from blankey.vault import Vault
from blankey.vault.fieldtypes import TRUE_VALUES, FieldType

OPEN_DIR = Path(tempfile.gettempdir()) / "blankey-open"

PLACEHOLDERS = {
    FieldType.DATE: "DD.MM.YYYY",
    FieldType.EGN: "10 digits",
    FieldType.IBAN: "BG00 XXXX 0000 0000 0000 00",
    FieldType.NUMBER: "1 500,00",
    FieldType.EMAIL: "name@example.com",
}


def value_input(field_type: FieldType, value: str = "", hint: str = "") -> QWidget:
    if field_type == FieldType.BOOL:
        box = QCheckBox(hint)
        box.setChecked(value.strip().lower() in TRUE_VALUES)
        return box
    edit = QLineEdit(value)
    edit.setPlaceholderText(hint or PLACEHOLDERS.get(field_type, ""))
    return edit


def input_value(widget: QWidget) -> str:
    if isinstance(widget, QCheckBox):
        return "yes" if widget.isChecked() else ""
    return widget.text().strip()


def open_documents(vault: Vault, doc_ids: list[int]) -> None:
    """Decrypt into a private temp folder (cleared on lock and quit) and open with the default app."""
    OPEN_DIR.mkdir(mode=0o700, exist_ok=True)
    filenames = {d.id: d.filename for d in vault.list_documents()}
    for doc_id in doc_ids:
        path = OPEN_DIR / filenames[doc_id]
        path.write_bytes(vault.load_document(doc_id))
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(path)))


def clear_opened_documents() -> None:
    for path in OPEN_DIR.glob("*"):
        path.unlink(missing_ok=True)


def fit_to_screen(widget: QWidget, width: float, height: float, min_width: int = 0, min_height: int = 0) -> None:
    """Size a window as a fraction of the available screen, never smaller than the minimum or larger than the screen."""
    area = (widget.screen() or QGuiApplication.primaryScreen()).availableGeometry()
    widget.resize(
        min(area.width(), max(min_width, int(area.width() * width))),
        min(area.height(), max(min_height, int(area.height() * height))),
    )


class PagePreview(QScrollArea):
    def __init__(self, pngs: list[bytes], parent: QWidget | None = None):
        super().__init__(parent)
        container = QWidget()
        layout = QVBoxLayout(container)
        for png in pngs:
            pixmap = QPixmap()
            pixmap.loadFromData(png, "PNG")
            label = QLabel()
            label.setPixmap(pixmap)
            layout.addWidget(label)
        self.setWidget(container)
        self.setMinimumSize(720, 600)


# -- native-looking building blocks ------------------------------------------------


def accent_color(widget: QWidget | None = None) -> QColor:
    palette = widget.palette() if widget is not None else QGuiApplication.palette()
    return palette.color(QPalette.ColorRole.Accent)


def blend(color: QColor, ground: QColor, weight: float) -> QColor:
    """Opaque mix: `weight` of `color` over `ground`."""
    pairs = zip(color.getRgbF()[:3], ground.getRgbF()[:3], strict=True)
    return QColor.fromRgbF(*(c * weight + g * (1 - weight) for c, g in pairs))


def secondary_color(widget: QWidget | None = None) -> QColor:
    """Opaque secondary text color with at least 4.5:1 contrast on the window and list backgrounds.
    The palette's PlaceholderText is a 25% alpha label color on macOS: far too faint for content."""
    palette = widget.palette() if widget is not None else QGuiApplication.palette()
    return blend(palette.color(QPalette.ColorRole.Text), palette.color(QPalette.ColorRole.Base), 0.62)


class SecondaryLabel(QLabel):
    """A label in the secondary color that follows light / dark switches."""

    def __init__(self, text: str = "", smaller: bool = True):
        super().__init__(text)
        self.setWordWrap(True)
        if smaller:
            font = self.font()
            font.setPointSizeF(font.pointSizeF() - 1)
            self.setFont(font)
        self._apply()

    def _apply(self) -> None:
        palette = self.palette()
        palette.setColor(QPalette.ColorRole.WindowText, secondary_color())
        self.setPalette(palette)

    def event(self, event) -> bool:
        if event.type() == QEvent.Type.ApplicationPaletteChange:
            self._apply()
        return super().event(event)


def secondary_label(text: str = "", smaller: bool = True) -> QLabel:
    return SecondaryLabel(text, smaller)


def heading_label(text: str, delta: float = 2) -> QLabel:
    label = QLabel(text)
    font = label.font()
    font.setPointSizeF(font.pointSizeF() + delta)
    font.setWeight(QFont.Weight.DemiBold)
    label.setFont(font)
    return label


def separator() -> QFrame:
    line = QFrame()
    line.setFrameShape(QFrame.Shape.HLine)
    line.setFrameShadow(QFrame.Shadow.Plain)
    line.setForegroundRole(QPalette.ColorRole.Mid)
    return line


def icon_tile(symbol: str, color: QColor, size: int = 36) -> QPixmap:
    """A rounded colored square with a white SF Symbol, like the icons in macOS settings panes."""
    ratio = 2
    pixmap = QPixmap(size * ratio, size * ratio)
    pixmap.setDevicePixelRatio(ratio)
    pixmap.fill(Qt.GlobalColor.transparent)
    painter = QPainter(pixmap)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    painter.setPen(Qt.PenStyle.NoPen)
    painter.setBrush(color)
    painter.drawRoundedRect(QRectF(0, 0, size, size), size * 0.24, size * 0.24)
    glyph = macos.symbol_pixmap(symbol, QColor("white"), size * 0.5)
    if glyph is not None:
        w, h = glyph.width() / glyph.devicePixelRatio(), glyph.height() / glyph.devicePixelRatio()
        painter.drawPixmap(QRectF((size - w) / 2, (size - h) / 2, w, h), glyph, QRectF(glyph.rect()))
    painter.end()
    return pixmap


class HeaderStrip(QWidget):
    """Icon tile, title and explanation at the top of a dialog."""

    def __init__(self, symbol: str, title: str, text: str = "", parent: QWidget | None = None):
        super().__init__(parent)
        icon = QLabel()
        icon.setPixmap(icon_tile(symbol, accent_color(self)))
        icon.setAlignment(Qt.AlignmentFlag.AlignTop)
        texts = QVBoxLayout()
        texts.setSpacing(3)
        texts.addWidget(heading_label(title))
        self.text = secondary_label(text, smaller=False)
        self.text.setVisible(bool(text))
        texts.addWidget(self.text)
        layout = QHBoxLayout(self)
        layout.setContentsMargins(20, 14, 20, 14)
        layout.setSpacing(14)
        layout.addWidget(icon)
        layout.addLayout(texts, 1)


def footer_note(text: str) -> QWidget:
    """Lock glyph and a short privacy note, shown left of a dialog's buttons."""
    widget = QWidget()
    layout = QHBoxLayout(widget)
    layout.setContentsMargins(0, 0, 0, 0)
    layout.setSpacing(6)
    glyph = macos.symbol_pixmap("lock", secondary_color(widget), 11)
    if glyph is not None:
        icon = QLabel()
        icon.setPixmap(glyph)
        layout.addWidget(icon)
    layout.addWidget(secondary_label(text), 1)
    return widget


class RowDelegate(QStyledItemDelegate):
    """Cell text inset to line up with the header label; columns after the first in the secondary color."""

    def paint(self, painter: QPainter, option: QStyleOptionViewItem, index) -> None:
        self.initStyleOption(option, index)
        text, option.text = option.text, ""
        style = option.widget.style()
        style.drawControl(QStyle.ControlElement.CE_ItemViewItem, option, painter, option.widget)
        selected = bool(option.state & QStyle.StateFlag.State_Selected)
        if selected:
            color = option.palette.color(QPalette.ColorRole.HighlightedText)
        elif index.column() == 0:
            color = option.palette.color(QPalette.ColorRole.Text)
        else:
            color = secondary_color(option.widget)
        margin = style.pixelMetric(QStyle.PixelMetric.PM_HeaderMargin) + 2
        rect = option.rect.adjusted(margin, 0, -margin, 0)
        painter.save()
        painter.setFont(option.font)
        painter.setPen(color)
        shown = option.fontMetrics.elidedText(text, Qt.TextElideMode.ElideRight, rect.width())
        painter.drawText(rect, Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, shown)
        painter.restore()


def style_table(table: QTableWidget, editable: bool = False) -> None:
    """Finder-like list: no grid or frame, alternating rows, full-row selection, small left-aligned header."""
    table.setShowGrid(False)
    table.setAlternatingRowColors(True)
    table.setFrameShape(QFrame.Shape.NoFrame)
    table.setWordWrap(False)
    table.setAttribute(Qt.WidgetAttribute.WA_MacShowFocusRect, False)
    table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
    if not editable:
        table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
    table.verticalHeader().setVisible(False)
    table.verticalHeader().setDefaultSectionSize(28)
    table.setItemDelegate(RowDelegate(table))
    header = table.horizontalHeader()
    header.setHighlightSections(False)
    header.setStretchLastSection(True)
    header.setDefaultAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
    font = header.font()
    font.setPointSizeF(font.pointSizeF() - 1)
    font.setWeight(QFont.Weight.DemiBold)
    header.setFont(font)


def style_list(widget: QListWidget) -> None:
    widget.setFrameShape(QFrame.Shape.NoFrame)
    widget.setAlternatingRowColors(True)
    widget.setAttribute(Qt.WidgetAttribute.WA_MacShowFocusRect, False)


def filter_table(table: QTableWidget, text: str) -> None:
    needle = text.strip().casefold()
    for row in range(table.rowCount()):
        cells = (table.item(row, col) for col in range(table.columnCount()))
        table.setRowHidden(row, bool(needle) and not any(c and needle in c.text().casefold() for c in cells))


def filter_list(widget: QListWidget, text: str) -> None:
    needle = text.strip().casefold()
    for row in range(widget.count()):
        item = widget.item(row)
        item.setHidden(bool(needle) and needle not in item.text().casefold())


def confirm(parent: QWidget | None, text: str, informative: str = "", action: str = "Delete") -> bool:
    """Destructive confirmation; a sheet on macOS when it has a parent window."""
    box = QMessageBox(parent)
    box.setIcon(QMessageBox.Icon.Warning)
    box.setText(text)
    box.setInformativeText(informative)
    if parent is not None:
        box.setWindowModality(Qt.WindowModality.WindowModal)
    ok = box.addButton(action, QMessageBox.ButtonRole.DestructiveRole)
    cancel = box.addButton(QMessageBox.StandardButton.Cancel)
    box.setDefaultButton(cancel)
    box.setEscapeButton(cancel)
    box.exec()
    return box.clickedButton() is ok


class Page(QWidget):
    """A section of the main window: its toolbar actions, subtitle and search filter."""

    title = ""
    symbol = ""
    changed = Signal()

    def __init__(self, app, parent: QWidget | None = None):
        super().__init__(parent)
        self.app = app
        self.toolbar_actions: list[QAction] = []

    def action(self, text: str, symbol: str, slot, shortcut=None) -> QAction:
        action = QAction(macos.symbol_icon(symbol), text, self)
        action.setData(symbol)
        action.triggered.connect(slot)
        if shortcut is not None:
            action.setShortcut(shortcut)
        self.toolbar_actions.append(action)
        return action

    def subtitle(self) -> str:
        return ""

    def badge(self) -> int:
        return 0

    def refresh(self) -> None:
        pass

    def filter(self, text: str) -> None:
        pass

    def activate(self) -> bool:
        """Called when the page is selected; return False to stay on the previous page."""
        return True
