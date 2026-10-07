"""Fill form: every blank of one or more templates with its context and a choice of source."""

import json
from typing import Any

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QBrush, QColor
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QMessageBox,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from blankey.core import Blankey
from blankey.templates import fill
from blankey.templates.bindings import as_text
from blankey.vault import Request

EMPTY = QBrush(QColor("#c0392b"))


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
        for var in self.vars.values():
            self.bindings.setdefault(var.name, fill.default_binding(var, set(self.roles)))

        self.setWindowTitle("Fill: " + ", ".join(t.name for t in self.templates))
        self.resize(1100, 720)
        layout = QVBoxLayout(self)
        if request is not None and request.payload.get("reason"):
            reason = QLabel(request.payload["reason"])
            reason.setWordWrap(True)
            layout.addWidget(reason)

        form = QFormLayout()
        self.name_edit = QLineEdit(name)
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
        layout.addLayout(form)

        self.table = QTableWidget(len(self.vars), 3)
        self.table.setHorizontalHeaderLabels(["Field", "Context", "Source / value"])
        self.table.setWordWrap(True)
        self.table.verticalHeader().setVisible(False)
        header = self.table.horizontalHeader()
        header.setSectionResizeMode(0, QHeaderView.ResizeMode.ResizeToContents)
        header.setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        header.setSectionResizeMode(2, QHeaderView.ResizeMode.Interactive)
        self.table.setColumnWidth(2, 380)
        for row, var in enumerate(self.vars.values()):
            label = QTableWidgetItem(var.label or var.name)
            label.setToolTip(var.name)
            context = QTableWidgetItem(var.contexts[0][1] if var.contexts else "")
            context.setToolTip("\n\n".join(f"{t}: {c}" for t, c in var.contexts))
            for item in (label, context):
                item.setFlags(item.flags() & ~Qt.ItemFlag.ItemIsEditable)
            self.table.setItem(row, 0, label)
            self.table.setItem(row, 1, context)
        layout.addWidget(self.table)
        layout.addWidget(
            QLabel(
                "<small>Pick a vault field or type a value. Values stay in Blankey; "
                "Claude only learns which source each field uses.</small>"
            )
        )

        buttons = QDialogButtonBox()
        buttons.addButton("Save", QDialogButtonBox.ButtonRole.ApplyRole).clicked.connect(lambda: self._save(False))
        buttons.addButton("Save && Generate", QDialogButtonBox.ButtonRole.AcceptRole)
        buttons.addButton(QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(lambda: self._save(True))
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)
        self._rebuild_sources()

    # -- sources -------------------------------------------------------------

    def profiles(self) -> dict[str, int]:
        return {role: combo.currentData() for role, combo in self.profile_combos.items() if combo.currentData()}

    def _rebuild_sources(self) -> None:
        if self.table.cellWidget(0, 2) is not None:
            self.bindings = self.current_bindings()
        profiles = self.profiles()
        self.context = self.app.real_context(profiles)
        path_items = []
        for role, profile_id in profiles.items():
            for key, (_, value) in sorted(self.app.vault.get_values(profile_id).items()):
                path_items.append((f"{role}.{key}  —  {value}", {fill.PATH: f"{role}.{key}"}))
        for row, var in enumerate(self.vars.values()):
            combo = QComboBox()
            combo.setEditable(True)
            combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
            expressions = set(var.defaults.values())
            if expressions:
                expression = expressions.pop() if len(expressions) == 1 else "per template"
                value = self._value({fill.DEFAULT: True}, var)
                combo.addItem(f"= {expression}  —  {as_text(value)}", _key({fill.DEFAULT: True}))
            literals = dict.fromkeys(
                v for v in (self.claude_values.get(var.name), self.bindings[var.name].get(fill.VALUE)) if v
            )
            for literal in literals:
                combo.addItem(literal, _key({fill.VALUE: literal}))
            for text, binding in path_items:
                combo.addItem(text, _key(binding))
            index = combo.findData(_key(self.bindings[var.name]))
            if index >= 0:
                combo.setCurrentIndex(index)
            else:
                combo.setEditText(self.bindings[var.name].get(fill.VALUE, ""))
            combo.currentTextChanged.connect(lambda _, r=row: self._mark(r))
            self.table.setCellWidget(row, 2, combo)
            self._mark(row)
        QTimer.singleShot(0, self.table.resizeRowsToContents)  # after the stretched column has its width

    def _combo(self, row: int) -> QComboBox:
        return self.table.cellWidget(row, 2)

    def _binding(self, row: int) -> dict[str, Any]:
        combo = self._combo(row)
        text = combo.currentText()
        index = combo.findText(text)
        return json.loads(combo.itemData(index)) if index >= 0 else {fill.VALUE: text.strip()}

    def _value(self, binding: dict[str, Any], var: fill.FillVar) -> Any:
        expression = next(iter(var.defaults.values()), None)
        try:
            return fill.evaluate_binding(binding, self.context, expression)
        except Exception as exc:
            return f"<{type(exc).__name__}>"

    def _mark(self, row: int) -> None:
        var = list(self.vars.values())[row]
        empty = fill.is_empty(self._value(self._binding(row), var))
        self.table.item(row, 0).setForeground(EMPTY if empty else QBrush())

    def current_bindings(self) -> dict[str, dict[str, Any]]:
        return {var: self._binding(row) for row, var in enumerate(self.vars)}

    def empty_vars(self) -> list[str]:
        return [
            var.name
            for row, var in enumerate(self.vars.values())
            if fill.is_empty(self._value(self._binding(row), var))
        ]

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
