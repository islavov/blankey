"""Dialogs for requests coming from Claude: profile input and document generation."""

from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMessageBox,
    QVBoxLayout,
    QWidget,
)

from blanka.core import Blanka, GenerateOptions
from blanka.render.preview import render_pages
from blanka.ui.unlock import ensure_unlocked
from blanka.ui.widgets import PagePreview, input_value, value_input
from blanka.vault import FieldInput, FieldType, Request
from blanka.vault.fieldtypes import validate

SIGN_CHOICES = {
    "none": "No signature",
    "self-signed": "Self-signed certificate",
    "qes": "Qualified e-signature (smart card)",
}


class ProfileInputDialog(QDialog):
    def __init__(self, app: Blanka, request: Request, parent: QWidget | None = None):
        super().__init__(parent)
        self.app = app
        self.request = request
        payload = request.payload
        self.profile_id: int | None = payload.get("profile_id")
        self.setWindowTitle("Claude asks for data")
        self.resize(560, 0)

        layout = QVBoxLayout(self)
        reason = QLabel(payload.get("reason") or "")
        reason.setWordWrap(True)
        layout.addWidget(reason)

        form = QFormLayout()
        existing: dict[str, tuple[FieldType, str]] = {}
        self.name_edit: QLineEdit | None = None
        if self.profile_id is not None:
            profile = app.vault.get_profile(self.profile_id)
            form.addRow("Profile", QLabel(f"<b>{profile.name}</b>"))
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
        layout.addWidget(QLabel("<small>Values stay in Blanka. Claude only sees the keys and lengths.</small>"))

        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Save | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self._save)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)

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
            QMessageBox.warning(self, "Blanka", "\n".join(errors))
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


class GenerateDialog(QDialog):
    """Preview the real document (visible only here) and choose signing and encryption."""

    def __init__(
        self,
        app: Blanka,
        template_id: str,
        profiles: dict[str, int],
        defaults: dict | None = None,
        request: Request | None = None,
        parent: QWidget | None = None,
    ):
        super().__init__(parent)
        self.app = app
        self.template_id = template_id
        self.profiles = profiles
        self.request = request
        defaults = defaults or {}
        template = app.templates.get(template_id)
        self.setWindowTitle(f"Generate: {template.name}")

        rendered = app.render_real(template_id, profiles)
        layout = QHBoxLayout(self)
        if rendered.is_pdf:
            layout.addWidget(PagePreview(render_pages(rendered.content, dpi=90)), 3)

        side = QVBoxLayout()
        names = ", ".join(f"{role}: {app.vault.get_profile(pid).name}" for role, pid in profiles.items())
        info = QLabel(f"<b>{template.name}</b><br>{names}")
        info.setWordWrap(True)
        side.addWidget(info)
        if rendered.warnings:
            warning = QLabel("Warnings:\n" + "\n".join(rendered.warnings))
            warning.setWordWrap(True)
            side.addWidget(warning)

        self.title = QLineEdit(defaults.get("title") or template.name)
        self.sign = QComboBox()
        for key, label in SIGN_CHOICES.items():
            self.sign.addItem(label, key)
        self.sign.setCurrentIndex(list(SIGN_CHOICES).index(defaults.get("sign", "none")))
        self.signer = QLineEdit()
        self.signer.setPlaceholderText("Name in the certificate")
        self.pin = QLineEdit()
        self.pin.setEchoMode(QLineEdit.EchoMode.Password)
        self.pin.setPlaceholderText("Card PIN")
        self.encrypt = QCheckBox("Protect with a password")
        self.encrypt.setChecked(bool(defaults.get("encrypt")))
        self.password = QLineEdit()
        self.password.setEchoMode(QLineEdit.EchoMode.Password)
        self.password_repeat = QLineEdit()
        self.password_repeat.setEchoMode(QLineEdit.EchoMode.Password)

        options = QGroupBox("Output")
        form = QFormLayout(options)
        form.addRow("Title", self.title)
        form.addRow("Signature", self.sign)
        form.addRow("Signer", self.signer)
        form.addRow("PIN", self.pin)
        form.addRow(self.encrypt)
        form.addRow("Password", self.password)
        form.addRow("Repeat", self.password_repeat)
        side.addWidget(options)
        side.addStretch()
        buttons = QDialogButtonBox()
        buttons.addButton("Generate", QDialogButtonBox.ButtonRole.AcceptRole)
        buttons.addButton("Decline", QDialogButtonBox.ButtonRole.RejectRole)
        buttons.accepted.connect(self._generate)
        buttons.rejected.connect(self.reject)
        side.addWidget(buttons)
        layout.addLayout(side, 1)

        self.sign.currentIndexChanged.connect(self._sync)
        self.encrypt.toggled.connect(self._sync)
        self._sync()
        if not rendered.is_pdf:
            for widget in (self.sign, self.encrypt):
                widget.setEnabled(False)

    def _sync(self) -> None:
        sign = self.sign.currentData()
        self.signer.setEnabled(sign == "self-signed")
        self.pin.setEnabled(sign == "qes")
        for widget in (self.password, self.password_repeat):
            widget.setEnabled(self.encrypt.isChecked())

    def _generate(self) -> None:
        sign = self.sign.currentData()
        password = None
        if self.encrypt.isChecked():
            if not self.password.text() or self.password.text() != self.password_repeat.text():
                QMessageBox.warning(self, "Blanka", "The passwords do not match.")
                return
            password = self.password.text()
        if sign == "qes" and not self.app.config.pkcs11_lib:
            QMessageBox.warning(self, "Blanka", f"Set pkcs11_lib in {self.app.config.config_path} (the card driver).")
            return
        options = GenerateOptions(
            sign=sign, password=password, pkcs11_pin=self.pin.text(), signer_name=self.signer.text()
        )
        try:
            doc_id = self.app.generate(self.template_id, self.profiles, options, self.title.text().strip())
        except Exception as exc:
            QMessageBox.critical(self, "Blanka", f"Generation failed: {exc}")
            return
        if self.request is not None:
            doc = next(d for d in self.app.vault.list_documents() if d.id == doc_id)
            self.app.vault.resolve_request(
                self.request.id,
                "done",
                {
                    "outcome": "generated",
                    "document_id": doc_id,
                    "pages": doc.pages,
                    "signed": doc.signed,
                    "encrypted": doc.encrypted,
                },
            )
        self.accept()

    def reject(self) -> None:
        if self.request is not None:
            self.app.vault.resolve_request(self.request.id, "cancelled", {"outcome": "cancelled"})
        super().reject()


def open_request(app: Blanka, request: Request, parent: QWidget | None = None) -> None:
    if request.status != "pending":
        return
    if not ensure_unlocked(app.vault, parent, "Claude sent a request. Unlock the vault to continue."):
        return
    match request.kind:
        case "profile_input":
            ProfileInputDialog(app, request, parent).exec()
        case "generate":
            payload = request.payload
            profiles = {role: int(pid) for role, pid in payload["profiles"].items()}
            GenerateDialog(app, payload["template_id"], profiles, payload, request, parent).exec()
