"""Fill form: every blank of one or more templates with its context and a choice of source."""

import html
import json
from typing import Any

from PySide6.QtCore import QModelIndex, QRect, QSize, Qt, QTimer
from PySide6.QtGui import QColor, QFont, QFontMetrics, QPainter, QPalette, QTextDocument
from PySide6.QtWidgets import (
    QAbstractItemView,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QHeaderView,
    QLineEdit,
    QMessageBox,
    QStyle,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from blankey.core import Blankey
from blankey.templates import fill
from blankey.templates.bindings import as_text
from blankey.templates.docx import BLANK
from blankey.ui.widgets import (
    HeaderStrip,
    accent_color,
    fit_to_screen,
    footer_note,
    open_documents,
    secondary_color,
    secondary_label,
    separator,
    style_table,
)
from blankey.vault import Request

BINDING_ROLE = Qt.ItemDataRole.UserRole
OPTIONS_ROLE = Qt.ItemDataRole.UserRole + 1
VAR_ROLE = Qt.ItemDataRole.UserRole + 2
FIELD, CONTEXT, SOURCE = range(3)
CHOOSE = "Choose a source…"


def _dark(widget: QWidget) -> bool:
    return widget.palette().color(QPalette.ColorRole.Window).lightness() < 128


def warn_color(widget: QWidget) -> QColor:
    return QColor("#FF9F0A") if _dark(widget) else QColor("#B24A00")


def _tint(color: QColor, alpha: float) -> QColor:
    tint = QColor(color)
    tint.setAlphaF(alpha)
    return tint


class FieldDelegate(QStyledItemDelegate):
    """Label, the template variable underneath, and a warning dot when the value would be empty."""

    def __init__(self, dialog: "FillDialog"):
        super().__init__(dialog.table)
        self.dialog = dialog

    def paint(self, painter: QPainter, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        self.initStyleOption(option, index)
        option.text = ""
        option.widget.style().drawControl(QStyle.ControlElement.CE_ItemViewItem, option, painter, option.widget)
        empty = self.dialog.is_empty_row(index.row())
        rect = option.rect.adjusted(12, 8, -8, -8)
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        font = QFont(option.font)
        font.setWeight(QFont.Weight.Medium)
        painter.setFont(font)
        x = rect.left()
        if empty:
            painter.setPen(Qt.PenStyle.NoPen)
            painter.setBrush(warn_color(self.dialog))
            painter.drawEllipse(x, rect.top() + 5, 7, 7)
            x += 13
        painter.setPen(warn_color(self.dialog) if empty else option.palette.color(QPalette.ColorRole.Text))
        line = QFontMetrics(font).height()
        painter.drawText(QRect(x, rect.top(), rect.right() - x, line), Qt.AlignmentFlag.AlignLeft, index.data())
        mono = QFont("Menlo")
        mono.setPointSizeF(option.font.pointSizeF() - 2)
        painter.setFont(mono)
        painter.setPen(secondary_color(self.dialog))
        painter.drawText(QRect(rect.left(), rect.top() + line + 2, rect.width(), line), 0, index.data(VAR_ROLE))
        painter.restore()

    def sizeHint(self, option: QStyleOptionViewItem, index: QModelIndex) -> QSize:
        return QSize(200, QFontMetrics(option.font).height() * 2 + 20)


class ContextDelegate(QStyledItemDelegate):
    """The paragraph around the blank, wrapped, with the blank itself highlighted."""

    def __init__(self, dialog: "FillDialog"):
        super().__init__(dialog.table)
        self.dialog = dialog

    def _document(self, option: QStyleOptionViewItem, index: QModelIndex, width: int) -> QTextDocument:
        empty = self.dialog.is_empty_row(index.row())
        color = warn_color(self.dialog) if empty else accent_color(self.dialog)
        muted = secondary_color(self.dialog).name()
        blank = (
            f'<span style="color:{color.name()}; background-color:{_tint(color, 0.14).name(QColor.NameFormat.HexArgb)};'
            f' font-weight:600">&nbsp;____&nbsp;</span>'
        )
        text = html.escape(index.data() or "").replace(html.escape(BLANK), blank)
        doc = QTextDocument()
        doc.setDefaultFont(option.font)
        doc.setDocumentMargin(0)
        doc.setHtml(f'<div style="color:{muted}">{text}</div>')
        doc.setTextWidth(width)
        return doc

    def paint(self, painter: QPainter, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        self.initStyleOption(option, index)
        option.text = ""
        option.widget.style().drawControl(QStyle.ControlElement.CE_ItemViewItem, option, painter, option.widget)
        rect = option.rect.adjusted(10, 8, -10, -8)
        painter.save()
        painter.translate(rect.topLeft())
        self._document(option, index, rect.width()).drawContents(painter)
        painter.restore()

    def sizeHint(self, option: QStyleOptionViewItem, index: QModelIndex) -> QSize:
        width = max(200, self.dialog.table.columnWidth(CONTEXT) - 20)
        return QSize(width, int(self._document(option, index, width).size().height()) + 16)


class SourceDelegate(QStyledItemDelegate):
    """A pop-up button look: a source chip, the vault path and the value. Edits through a combo box."""

    def __init__(self, dialog: "FillDialog"):
        super().__init__(dialog.table)
        self.dialog = dialog

    def paint(self, painter: QPainter, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        self.initStyleOption(option, index)
        option.text = ""
        option.widget.style().drawControl(QStyle.ControlElement.CE_ItemViewItem, option, painter, option.widget)
        kind, path, value, empty = self.dialog.describe_row(index.row())
        box = QRect(option.rect.left() + 8, option.rect.top() + 6, option.rect.width() - 16, 26)
        palette = option.palette
        warn = warn_color(self.dialog)
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        border = warn if empty else palette.color(QPalette.ColorRole.Mid)
        painter.setPen(Qt.PenStyle.DashLine if empty else Qt.PenStyle.SolidLine)
        pen = painter.pen()
        pen.setColor(border)
        painter.setPen(pen)
        painter.setBrush(_tint(warn, 0.08) if empty else palette.color(QPalette.ColorRole.Base))
        painter.drawRoundedRect(box.adjusted(0, 0, -1, -1), 6, 6)

        chip_colors = {
            "Vault": accent_color(self.dialog),
            "Claude": QColor("#8E5CD9"),
            "Value": QColor("#2A9D8F"),
            "Default": secondary_color(self.dialog),
            "Empty": warn,
        }
        small = QFont(option.font)
        small.setPointSizeF(option.font.pointSizeF() - 2)
        small.setWeight(QFont.Weight.DemiBold)
        metrics = QFontMetrics(small)
        chip = QRect(box.left() + 6, box.center().y() - 9, metrics.horizontalAdvance(kind) + 12, 18)
        painter.setPen(Qt.PenStyle.NoPen)
        painter.setBrush(_tint(chip_colors[kind], 0.2))
        painter.drawRoundedRect(chip, 4, 4)
        painter.setFont(small)
        painter.setPen(palette.color(QPalette.ColorRole.Text))
        painter.drawText(chip, Qt.AlignmentFlag.AlignCenter, kind)
        x = chip.right() + 8
        if path:
            mono = QFont("Menlo")
            mono.setPointSizeF(option.font.pointSizeF() - 2)
            painter.setFont(mono)
            painter.setPen(secondary_color(self.dialog))
            width = QFontMetrics(mono).horizontalAdvance(path)
            painter.drawText(QRect(x, box.top(), width, box.height()), Qt.AlignmentFlag.AlignVCenter, path)
            x += width + 10
        painter.setFont(option.font)
        painter.setPen(warn if empty else palette.color(QPalette.ColorRole.Text))
        text_rect = QRect(x, box.top(), box.right() - x - 22, box.height())
        shown = QFontMetrics(option.font).elidedText(value, Qt.TextElideMode.ElideRight, text_rect.width())
        painter.drawText(text_rect, Qt.AlignmentFlag.AlignVCenter, shown)
        painter.setPen(secondary_color(self.dialog))
        painter.drawText(QRect(box.right() - 20, box.top(), 14, box.height()), Qt.AlignmentFlag.AlignCenter, "⌄")
        painter.restore()

    def sizeHint(self, option: QStyleOptionViewItem, index: QModelIndex) -> QSize:
        return QSize(420, 38)

    def createEditor(self, parent: QWidget, option: QStyleOptionViewItem, index: QModelIndex) -> QWidget:
        combo = QComboBox(parent)
        combo.setEditable(True)
        combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        for text, key in index.data(OPTIONS_ROLE):
            combo.addItem(text, key)
        view = combo.view()
        view.setTextElideMode(Qt.TextElideMode.ElideNone)
        view.setMinimumWidth(min(view.sizeHintForColumn(0) + 40, self.dialog.width()))
        combo.activated.connect(lambda *_: self.commitData.emit(combo))
        combo.activated.connect(lambda *_: self.closeEditor.emit(combo))
        QTimer.singleShot(0, combo.showPopup)
        return combo

    def setEditorData(self, editor: QComboBox, index: QModelIndex) -> None:
        key = index.data(BINDING_ROLE)
        position = editor.findData(key)
        if position >= 0:
            editor.setCurrentIndex(position)
        else:
            editor.setEditText(json.loads(key).get(fill.VALUE, ""))
        editor.lineEdit().setCursorPosition(0)

    def setModelData(self, editor: QComboBox, model, index: QModelIndex) -> None:
        text = editor.currentText()
        position = editor.findText(text)
        key = editor.itemData(position) if position >= 0 else _key({fill.VALUE: text.strip()})
        model.setData(index, key, BINDING_ROLE)

    def updateEditorGeometry(self, editor: QWidget, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        editor.setGeometry(option.rect.adjusted(6, 4, -6, -4))


class FillDialog(QDialog):
    def __init__(
        self,
        app: Blankey,
        templates: list[str],
        profiles: dict[str, int],
        name: str = "",
        values: dict[str, str] | None = None,
        paths: dict[str, str] | None = None,
        fill_set_id: int | None = None,
        request: Request | None = None,
        parent: QWidget | None = None,
    ):
        super().__init__(parent)
        self.app = app
        self.request = request
        self.saved = False
        self.fill_set_id = fill_set_id if fill_set_id is not None else app.vault.find_fill_set(name)
        self.bindings: dict[str, dict[str, Any]] = {}
        if self.fill_set_id is not None:
            info, self.bindings = app.vault.get_fill_set(self.fill_set_id)
            templates = templates or info.templates
            profiles = info.profiles | profiles
            name = name or info.name
        self.claude_values = values or {}
        self.bindings |= {var: {fill.VALUE: value} for var, value in self.claude_values.items()}
        self.bindings |= {var: {fill.PATH: path} for var, path in (paths or {}).items()}

        self.templates = app.fill_templates(templates)
        self.roles = fill.roles(self.templates)
        self.vars = fill.variables(self.templates)
        self.var_list = list(self.vars.values())
        for var in self.var_list:
            self.bindings.setdefault(var.name, fill.default_binding(var, set(self.roles)))
        self.context: dict[str, Any] = {}
        self.vault_values: dict[str, str] = {}

        names = " · ".join(t.name for t in self.templates)
        count = len(self.templates)
        self.setWindowTitle("Fill documents")
        fit_to_screen(self, 0.9, 0.85, 1200, 760)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        title = f"Claude wants to fill {count} document{'s' if count != 1 else ''}" if request else "Fill documents"
        reason = (request.payload.get("reason") if request else "") or ""
        explanation = f"{names}. Check each blank and pick where its value comes from."
        header = QHBoxLayout()
        text = f"{reason}. {explanation}" if reason else explanation
        header.addWidget(HeaderStrip("doc.text.fill", title, text), 1)

        form = QFormLayout()
        form.setContentsMargins(0, 14, 20, 14)
        form.setLabelAlignment(Qt.AlignmentFlag.AlignRight)
        form.setHorizontalSpacing(10)
        self.name_edit = QLineEdit(name)
        self.name_edit.setMinimumWidth(320)
        form.addRow("Fill set", self.name_edit)
        self.profile_combos: dict[str, QComboBox] = {}
        all_profiles = app.vault.list_profiles()
        for role, description in self.roles.items():
            combo = QComboBox()
            for profile in all_profiles:
                combo.addItem(f"{profile.name}  ({profile.kind})" if profile.kind else profile.name, profile.id)
            if role in profiles:
                combo.setCurrentIndex(combo.findData(profiles[role]))
            combo.setToolTip(description)
            combo.currentIndexChanged.connect(self._rebuild_sources)
            form.addRow(role, combo)
            self.profile_combos[role] = combo
        header.addLayout(form)
        layout.addLayout(header)
        layout.addWidget(separator())

        self.summary = secondary_label()
        self.summary.setContentsMargins(20, 6, 20, 6)
        layout.addWidget(self.summary)
        layout.addWidget(separator())

        self.table = QTableWidget(len(self.var_list), 3)
        self.table.setHorizontalHeaderLabels(["Field", "In the document", "Source and value"])
        style_table(self.table, editable=True)
        self.table.setWordWrap(True)
        self.table.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.table.setEditTriggers(
            QAbstractItemView.EditTrigger.SelectedClicked
            | QAbstractItemView.EditTrigger.DoubleClicked
            | QAbstractItemView.EditTrigger.EditKeyPressed
        )
        self.table.clicked.connect(lambda index: index.column() == SOURCE and self.table.edit(index))
        self.table.setItemDelegateForColumn(FIELD, FieldDelegate(self))
        self.table.setItemDelegateForColumn(CONTEXT, ContextDelegate(self))
        self.table.setItemDelegateForColumn(SOURCE, SourceDelegate(self))
        header_view = self.table.horizontalHeader()
        header_view.setStretchLastSection(False)
        header_view.setSectionResizeMode(FIELD, QHeaderView.ResizeMode.Interactive)
        header_view.setSectionResizeMode(CONTEXT, QHeaderView.ResizeMode.Stretch)
        header_view.setSectionResizeMode(SOURCE, QHeaderView.ResizeMode.Interactive)
        self.table.setColumnWidth(FIELD, 220)
        self.table.setColumnWidth(SOURCE, int(self.width() * 0.38))
        header_view.sectionResized.connect(lambda *_: QTimer.singleShot(0, self.table.resizeRowsToContents))
        for row, var in enumerate(self.var_list):
            label = QTableWidgetItem(var.label or var.name)
            label.setData(VAR_ROLE, var.name)
            context = QTableWidgetItem(var.contexts[0][1] if var.contexts else "")
            context.setToolTip("\n\n".join(f"{t}: {c}" for t, c in var.contexts))
            for item in (label, context):
                item.setFlags(item.flags() & ~Qt.ItemFlag.ItemIsEditable)
            self.table.setItem(row, FIELD, label)
            self.table.setItem(row, CONTEXT, context)
            self.table.setItem(row, SOURCE, QTableWidgetItem())
        self.table.itemChanged.connect(lambda item: item.column() == SOURCE and self._changed())
        layout.addWidget(self.table, 1)
        layout.addWidget(separator())

        footer = QHBoxLayout()
        footer.setContentsMargins(20, 12, 20, 14)
        footer.addWidget(footer_note("Values stay in Blankey. Claude only learns which source each field uses."), 1)
        buttons = QDialogButtonBox()
        save = buttons.addButton("Save", QDialogButtonBox.ButtonRole.ApplyRole)
        save.clicked.connect(lambda: self._save(False))
        generate = buttons.addButton("Save and Generate", QDialogButtonBox.ButtonRole.AcceptRole)
        generate.setDefault(True)
        buttons.addButton(QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(lambda: self._save(True))
        buttons.rejected.connect(self.reject)
        footer.addWidget(buttons)
        layout.addLayout(footer)
        self._rebuild_sources()

    # -- sources -------------------------------------------------------------

    def profiles(self) -> dict[str, int]:
        return {role: combo.currentData() for role, combo in self.profile_combos.items() if combo.currentData()}

    def _rebuild_sources(self) -> None:
        if self.table.item(0, SOURCE) is not None and self.table.item(0, SOURCE).data(BINDING_ROLE):
            self.bindings = self.current_bindings()
        profiles = self.profiles()
        self.context = self.app.real_context(profiles)
        self.vault_values = {}
        path_items = []
        for role, profile_id in profiles.items():
            for key, (_, value) in sorted(self.app.vault.get_values(profile_id).items()):
                self.vault_values[f"{role}.{key}"] = value
                path_items.append((f"{role}.{key}  —  {value or '(empty)'}", _key({fill.PATH: f"{role}.{key}"})))
        self.table.blockSignals(True)
        for row, var in enumerate(self.var_list):
            options = []
            expressions = set(var.defaults.values())
            if expressions:
                expression = next(iter(expressions)) if len(expressions) == 1 else "per template"
                value = self._value({fill.DEFAULT: True}, var)
                options.append((f"Default  {expression}  —  {as_text(value)}", _key({fill.DEFAULT: True})))
            literals = dict.fromkeys(
                v for v in (self.claude_values.get(var.name), self.bindings[var.name].get(fill.VALUE)) if v
            )
            options += [(literal, _key({fill.VALUE: literal})) for literal in literals]
            options += path_items
            item = self.table.item(row, SOURCE)
            item.setData(OPTIONS_ROLE, options)
            item.setData(BINDING_ROLE, _key(self.bindings[var.name]))
        self.table.blockSignals(False)
        self._changed()
        QTimer.singleShot(0, self.table.resizeRowsToContents)

    def _changed(self) -> None:
        empty = len(self.empty_vars())
        total = len(self.var_list)
        self.summary.setText(f"{total} fields · {empty} empty" if empty else f"{total} fields · all filled")
        self.table.viewport().update()

    def _binding(self, row: int) -> dict[str, Any]:
        return json.loads(self.table.item(row, SOURCE).data(BINDING_ROLE))

    def set_binding(self, row: int, binding: dict[str, Any]) -> None:
        self.table.item(row, SOURCE).setData(BINDING_ROLE, _key(binding))

    def _value(self, binding: dict[str, Any], var: fill.FillVar) -> Any:
        expression = next(iter(var.defaults.values()), None)
        try:
            return fill.evaluate_binding(binding, self.context, expression)
        except Exception as exc:
            return f"<{type(exc).__name__}>"

    def describe_row(self, row: int) -> tuple[str, str, str, bool]:
        """(kind, path, shown value, empty) for painting the source cell."""
        var = self.var_list[row]
        binding = self._binding(row)
        value = as_text(self._value(binding, var))
        empty = fill.is_empty(value)
        if fill.PATH in binding:
            kind, path = "Vault", binding[fill.PATH]
        elif fill.VALUE in binding:
            kind = "Claude" if binding[fill.VALUE] == self.claude_values.get(var.name) else "Value"
            path = ""
        else:
            expressions = set(var.defaults.values())
            kind, path = "Default", next(iter(expressions)) if len(expressions) == 1 else "per template"
            path = path.replace("{{", "").replace("}}", "").strip()
        if not empty:
            return kind, path, value, False
        if kind == "Vault":
            return kind, path, "(empty in profile)", True
        return "Empty", path, CHOOSE, True

    def is_empty_row(self, row: int) -> bool:
        return fill.is_empty(self._value(self._binding(row), self.var_list[row]))

    def current_bindings(self) -> dict[str, dict[str, Any]]:
        return {var.name: self._binding(row) for row, var in enumerate(self.var_list)}

    def empty_vars(self) -> list[str]:
        return [var.name for row, var in enumerate(self.var_list) if self.is_empty_row(row)]

    # -- actions ---------------------------------------------------------------

    def _save(self, generate: bool) -> None:
        name = self.name_edit.text().strip()
        problems = [] if name else ["Enter a fill set name"]
        if missing := sorted(set(self.roles) - set(self.profiles())):
            problems.append(f"Choose profiles for: {', '.join(missing)}")
        existing = self.app.vault.find_fill_set(name) if name else None
        if existing is not None and existing != self.fill_set_id:
            problems.append(f"A fill set named {name!r} already exists")
        if problems:
            QMessageBox.warning(self, "Blankey", "\n".join(problems))
            return
        templates = [t.id for t in self.templates]
        self.fill_set_id = self.app.vault.save_fill_set(
            name, templates, self.profiles(), self.current_bindings(), self.fill_set_id
        )
        self.app.vault.audit("user", "save_fill_set", str(self.fill_set_id))
        self.saved = True
        if not generate:
            return
        try:
            document_ids = self.app.generate_fill(self.fill_set_id)
        except Exception as exc:
            QMessageBox.critical(self, "Blankey", f"Generation failed: {exc}")
            return
        self._resolve("generated", document_ids)
        open_documents(self.app.vault, document_ids)
        self.accept()

    def _resolve(self, outcome: str, document_ids: list[int] | None = None) -> None:
        if self.request is None:
            return
        result = {"outcome": outcome}
        if outcome != "cancelled":
            result |= {"fill_set_id": self.fill_set_id, "document_ids": document_ids or [], "empty": self.empty_vars()}
        self.app.vault.resolve_request(self.request.id, "cancelled" if outcome == "cancelled" else "done", result)
        self.request = None

    def reject(self) -> None:
        self._resolve("saved" if self.saved else "cancelled")
        super().reject()


def _key(binding: dict[str, Any]) -> str:
    return json.dumps(binding, sort_keys=True, ensure_ascii=False)


def open_fill_request(app: Blankey, request: Request, parent: QWidget | None = None) -> None:
    payload = request.payload
    FillDialog(
        app,
        payload["templates"],
        {role: int(pid) for role, pid in payload["profiles"].items()},
        payload.get("name", ""),
        payload.get("values"),
        payload.get("paths"),
        payload.get("fill_set_id"),
        request,
        parent,
    ).exec()
