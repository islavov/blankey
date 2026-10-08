"""Dialogs for requests coming from Claude."""

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLineEdit,
    QMessageBox,
    QVBoxLayout,
    QWidget,
)

from blankey.core import Blankey
from blankey.ui.fill import open_fill_request
from blankey.ui.unlock import ensure_unlocked
from blankey.ui.widgets import (
    HeaderStrip,
    fit_to_screen,
    footer_note,
    input_value,
    separator,
    value_input,
)
from blankey.vault import FieldInput, FieldType, Request
from blankey.vault.fieldtypes import validate


class ProfileInputDialog(QDialog):
    def __init__(self, app: Blankey, request: Request, parent: QWidget | None = None):
        super().__init__(parent)
        self.app = app
        self.request = request
        payload = request.payload
        self.profile_id: int | None = payload.get("profile_id")
        self.setWindowTitle("Claude asks for data")
        fit_to_screen(self, 0.45, 0, 720)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        count = len(payload.get("fields", []))
        target = app.vault.get_profile(self.profile_id).name if self.profile_id is not None else "a new profile"
        title = f"{count} field{'s' if count != 1 else ''} for {target}"
        layout.addWidget(HeaderStrip("person.fill", title, payload.get("reason") or ""))
        layout.addWidget(separator())

        form = QFormLayout()
        form.setContentsMargins(28, 20, 28, 20)
        form.setVerticalSpacing(12)
        form.setHorizontalSpacing(12)
        form.setLabelAlignment(Qt.AlignmentFlag.AlignRight)
        form.setFieldGrowthPolicy(QFormLayout.FieldGrowthPolicy.ExpandingFieldsGrow)
        existing: dict[str, tuple[FieldType, str]] = {}
        self.name_edit: QLineEdit | None = None
        if self.profile_id is not None:
            existing = app.vault.get_values(self.profile_id)
        else:
            new_profile = payload.get("new_profile") or {}
            self.name_edit = QLineEdit(new_profile.get("name") or "")
            form.addRow("New profile", self.name_edit)

        self.inputs: list[tuple[dict, FieldType, QWidget]] = []
        for spec in payload.get("fields", []):
            field_type = FieldType(spec.get("type", "text"))
            current = existing.get(spec["key"], (field_type, ""))[1]
            widget = value_input(field_type, current, spec.get("hint", ""))
            label = spec.get("label") or spec["key"]
            if spec.get("required"):
                label += " *"
            widget.setToolTip(spec["key"])
            form.addRow(label, widget)
            self.inputs.append((spec, field_type, widget))
        layout.addLayout(form)
        layout.addStretch()
        layout.addWidget(separator())

        footer = QHBoxLayout()
        footer.setContentsMargins(20, 12, 20, 14)
        footer.addWidget(footer_note("Values stay in Blankey. Claude only sees the field names and lengths."), 1)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Save | QDialogButtonBox.StandardButton.Cancel)
        buttons.button(QDialogButtonBox.StandardButton.Save).setDefault(True)
        buttons.accepted.connect(self._save)
        buttons.rejected.connect(self.reject)
        footer.addWidget(buttons)
        layout.addLayout(footer)

    def _save(self) -> None:
        errors, items, statuses = [], [], []
        for spec, field_type, widget in self.inputs:
            value = input_value(widget)
            label = spec.get("label") or spec["key"]
            if spec.get("required") and not value:
                errors.append(f"{label}: required")
            elif error := validate(field_type, value):
                errors.append(f"{label}: {error}")
            items.append(FieldInput(spec["key"], value, spec.get("label", ""), field_type))
        if self.name_edit is not None and not self.name_edit.text().strip():
            errors.append("Enter a profile name")
        if errors:
            QMessageBox.warning(self, "Blankey", "\n".join(errors))
            return
        if self.profile_id is None:
            kind = (self.request.payload.get("new_profile") or {}).get("kind", "")
            self.profile_id = self.app.vault.create_profile(self.name_edit.text().strip(), kind)
        self.app.vault.set_values(self.profile_id, items)
        lengths = {f.key: f.length for f in self.app.vault.describe(self.profile_id)}
        for item in items:
            statuses.append(
                {"key": item.key, "length": lengths.get(item.key, 0), "status": "saved" if item.value else "skipped"}
            )
        self.app.vault.resolve_request(
            self.request.id, "done", {"outcome": "saved", "profile_id": self.profile_id, "fields": statuses}
        )
        self.app.vault.audit("user", "profile_input", f"request={self.request.id} profile={self.profile_id}")
        self.accept()

    def reject(self) -> None:
        self.app.vault.resolve_request(self.request.id, "cancelled", {"outcome": "cancelled"})
        super().reject()


def open_request(app: Blankey, request: Request, parent: QWidget | None = None) -> None:
    if request.status != "pending":
        return
    if not ensure_unlocked(app, parent, "Claude sent a request. Unlock the vault to continue."):
        return
    request = app.vault.get_request(request.id)  # the payload is readable only now
    match request.kind:
        case "profile_input":
            ProfileInputDialog(app, request, parent).exec()
        case "generate":  # retired: documents are generated from fill sets only
            app.vault.resolve_request(request.id, "cancelled", {"outcome": "cancelled"})
        case "fill":
            open_fill_request(app, request, parent)
