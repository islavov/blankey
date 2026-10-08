"""Fill form: every blank of one or more templates with its context and a choice of source."""

import html
import json
from typing import Any

from PySide6.QtCore import QEvent, QModelIndex, QRect, QRectF, QSize, Qt, QTimer
from PySide6.QtGui import QColor, QFont, QFontMetrics, QPainter, QPainterPath, QPalette, QPixmap, QTextDocument
from PySide6.QtWidgets import (
    QAbstractItemView,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QHeaderView,
    QLineEdit,
    QMenu,
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
FIELD, SOURCE, CONTEXT = range(3)
CHOOSE = "Choose a source…"
CROP_MAX_HEIGHT = 110


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

    def _crop_size(self, pixmap: QPixmap, width: int) -> QSize:
        """Scaled to the column, never enlarged, at most CROP_MAX_HEIGHT high."""
        w, h = pixmap.width() / pixmap.devicePixelRatio(), pixmap.height() / pixmap.devicePixelRatio()
        scale = min(1, width / w, CROP_MAX_HEIGHT / h)
        return QSize(round(w * scale), round(h * scale))

    def paint(self, painter: QPainter, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        self.initStyleOption(option, index)
        option.text = ""
        option.widget.style().drawControl(QStyle.ControlElement.CE_ItemViewItem, option, painter, option.widget)
        rect = option.rect.adjusted(10, 8, -10, -8)
        painter.save()
        pixmap = self.dialog.crops.get(index.row())
        if pixmap is not None:
            size = self._crop_size(pixmap, rect.width())
            target = QRect(rect.topLeft(), size)
            painter.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
            painter.setRenderHint(QPainter.RenderHint.Antialiasing)
            clip = QPainterPath()
            clip.addRoundedRect(QRectF(target), 6, 6)
            painter.setClipPath(clip)
            painter.drawPixmap(target, pixmap)
            painter.setClipping(False)
            painter.setPen(option.palette.color(QPalette.ColorRole.Mid))
            painter.setBrush(Qt.BrushStyle.NoBrush)
            painter.drawRoundedRect(QRectF(target).adjusted(0.5, 0.5, -0.5, -0.5), 6, 6)
        else:
            painter.translate(rect.topLeft())
            self._document(option, index, rect.width()).drawContents(painter)
        painter.restore()

    def sizeHint(self, option: QStyleOptionViewItem, index: QModelIndex) -> QSize:
        width = max(200, self.dialog.table.columnWidth(CONTEXT) - 20)
        pixmap = self.dialog.crops.get(index.row())
        if pixmap is not None:
            return QSize(width, self._crop_size(pixmap, width).height() + 16)
        return QSize(width, int(self._document(option, index, width).size().height()) + 16)


class SourceDelegate(QStyledItemDelegate):
    """A pop-up button look: a source chip, the vault path and the value. Typed values edit in place."""

    def __init__(self, dialog: "FillDialog"):
        super().__init__(dialog.table)
        self.dialog = dialog

    def paint(self, painter: QPainter, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        self.initStyleOption(option, index)
        option.text = ""
        option.widget.style().drawControl(QStyle.ControlElement.CE_ItemViewItem, option, painter, option.widget)
        kind, path, value, empty = self.dialog.describe_row(index.row())
        box = source_box(option.rect)
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
        """Only typed values use an editor; picking a source goes through FillDialog.source_menu."""
        edit = QLineEdit(parent)
        edit.setPlaceholderText("Type a value")
        edit.setAttribute(Qt.WidgetAttribute.WA_MacShowFocusRect, False)
        return edit

    def setEditorData(self, editor: QLineEdit, index: QModelIndex) -> None:
        editor.setText(json.loads(index.data(BINDING_ROLE)).get(fill.VALUE, ""))
        editor.selectAll()

    def setModelData(self, editor: QLineEdit, model, index: QModelIndex) -> None:
        model.setData(index, _key({fill.VALUE: editor.text().strip()}), BINDING_ROLE)

    def updateEditorGeometry(self, editor: QWidget, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        editor.setGeometry(source_box(option.rect))


def source_box(rect: QRect) -> QRect:
    """The pop-up button drawn inside a source cell."""
    return QRect(rect.left() + 8, rect.top() + 6, rect.width() - 16, 26)


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
        self.crops: dict[int, QPixmap] = {}
        for row, var in enumerate(self.var_list):
            if var.crops:
                pixmap = QPixmap()
                pixmap.loadFromData(var.crops[0][1], "PNG")
                self.crops[row] = pixmap
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
        self.table.setHorizontalHeaderLabels(["Field", "Source and value", "In the document"])
        style_table(self.table, editable=True)
        self.table.setWordWrap(True)
        self.table.setSelectionMode(QAbstractItemView.SelectionMode.NoSelection)
        self.table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.table.clicked.connect(lambda index: index.column() == SOURCE and self.show_source_menu(index.row()))
        self.table.installEventFilter(self)
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
        path_sections = []
        for role, profile_id in profiles.items():
            section = []
            for key, (_, value) in sorted(self.app.vault.get_values(profile_id).items()):
                self.vault_values[f"{role}.{key}"] = value
                section.append((f"{role}.{key}  —  {value or '(empty)'}", _key({fill.PATH: f"{role}.{key}"})))
            if section:
                path_sections.append(section)
        self.table.blockSignals(True)
        for row, var in enumerate(self.var_list):
            first = []
            expressions = set(var.defaults.values())
            if expressions:
                expression = next(iter(expressions)) if len(expressions) == 1 else "per template"
                value = self._value({fill.DEFAULT: True}, var)
                first.append((f"Template default  {expression}  —  {as_text(value)}", _key({fill.DEFAULT: True})))
            claude = self.claude_values.get(var.name)
            typed = self.bindings[var.name].get(fill.VALUE)
            if claude:
                first.append((f"Claude  —  {claude}", _key({fill.VALUE: claude})))
            if typed and typed != claude:
                first.append((f"Typed  —  {typed}", _key({fill.VALUE: typed})))
            options = [first, *path_sections] if first else path_sections
            item = self.table.item(row, SOURCE)
            item.setData(OPTIONS_ROLE, options)
            item.setData(BINDING_ROLE, _key(self.bindings[var.name]))
        self.table.blockSignals(False)
        self._changed()
        QTimer.singleShot(0, self.table.resizeRowsToContents)

    def source_menu(self, row: int) -> QMenu:
        """Native pop-up menu of sources for a row: defaults and literals, one section per role, then typing."""
        menu = QMenu(self)
        current = self.table.item(row, SOURCE).data(BINDING_ROLE)
        for number, section in enumerate(self.table.item(row, SOURCE).data(OPTIONS_ROLE)):
            if number:
                menu.addSeparator()
            for text, key in section:
                action = menu.addAction(text)
                action.setCheckable(True)
                action.setChecked(key == current)
                action.triggered.connect(lambda _=False, k=key: self.table.item(row, SOURCE).setData(BINDING_ROLE, k))
        menu.addSeparator()
        menu.addAction("Type a value…", lambda: self.start_typing(row))
        return menu

    def show_source_menu(self, row: int) -> None:
        index = self.table.model().index(row, SOURCE)
        box = source_box(self.table.visualRect(index))
        menu = self.source_menu(row)
        menu.setMinimumWidth(box.width())
        menu.exec(self.table.viewport().mapToGlobal(box.bottomLeft()))

    def start_typing(self, row: int) -> None:
        index = self.table.model().index(row, SOURCE)
        self.table.setCurrentIndex(index)
        self.table.edit(index)

    def eventFilter(self, watched, event) -> bool:
        # only when the table itself has focus: a Return that finishes typing propagates here too
        if watched is self.table and event.type() == QEvent.Type.KeyPress and self.table.hasFocus():
            index = self.table.currentIndex()
            if index.isValid() and event.key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter, Qt.Key.Key_Space):
                self.show_source_menu(index.row())
                return True
        return super().eventFilter(watched, event)

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
