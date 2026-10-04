from PySide6.QtGui import QPixmap
from PySide6.QtWidgets import QCheckBox, QLabel, QLineEdit, QScrollArea, QVBoxLayout, QWidget

from blanka.vault.fieldtypes import TRUE_VALUES, FieldType

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
