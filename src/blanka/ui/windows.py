"""Documents, templates and pending-requests windows."""

import tempfile
from pathlib import Path

from PySide6.QtCore import Qt, QUrl
from PySide6.QtGui import QDesktopServices
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPushButton,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from blanka.core import Blanka
from blanka.ui.requests import GenerateDialog, open_request
from blanka.ui.unlock import ensure_unlocked

OPEN_DIR = Path(tempfile.gettempdir()) / "blanka-open"


def clear_opened_documents() -> None:
    for path in OPEN_DIR.glob("*"):
        path.unlink(missing_ok=True)


class DocumentsWindow(QWidget):
    def __init__(self, app: Blanka):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blanka - documents")
        self.resize(820, 420)
        self.table = QTableWidget(0, 6)
        self.table.setHorizontalHeaderLabels(["#", "Title", "Template", "Signature", "Password", "Created"])
        self.table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.table.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.table.horizontalHeader().setStretchLastSection(True)
        self.table.doubleClicked.connect(lambda *_: self._open())
        buttons = QHBoxLayout()
        for label, slot in (("Open", self._open), ("Export…", self._export), ("Delete", self._delete)):
            button = QPushButton(label)
            button.clicked.connect(slot)
            buttons.addWidget(button)
        buttons.addStretch()
        layout = QVBoxLayout(self)
        layout.addWidget(self.table)
        layout.addLayout(buttons)
        self.refresh()

    def refresh(self) -> None:
        docs = self.app.vault.list_documents()
        self.table.setRowCount(len(docs))
        for row, doc in enumerate(docs):
            cells = [
                str(doc.id),
                doc.title,
                doc.template_id,
                doc.signed or "-",
                "yes" if doc.encrypted else "-",
                doc.created_at.replace("T", " ")[:16],
            ]
            for col, text in enumerate(cells):
                item = QTableWidgetItem(text)
                item.setData(Qt.ItemDataRole.UserRole, doc.id)
                self.table.setItem(row, col, item)

    def _selected(self):
        row = self.table.currentRow()
        if row < 0:
            return None
        doc_id = self.table.item(row, 0).data(Qt.ItemDataRole.UserRole)
        return next(d for d in self.app.vault.list_documents() if d.id == doc_id)

    def _open(self) -> None:
        doc = self._selected()
        if doc is None or not ensure_unlocked(self.app.vault, self):
            return
        OPEN_DIR.mkdir(mode=0o700, exist_ok=True)
        path = OPEN_DIR / doc.filename
        path.write_bytes(self.app.vault.load_document(doc.id))
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(path)))

    def _export(self) -> None:
        doc = self._selected()
        if doc is None or not ensure_unlocked(self.app.vault, self):
            return
        target, _ = QFileDialog.getSaveFileName(self, "Export", str(Path.home() / doc.filename))
        if target:
            Path(target).write_bytes(self.app.vault.load_document(doc.id))
            self.app.vault.audit("user", "export_document", str(doc.id))

    def _delete(self) -> None:
        doc = self._selected()
        if doc is None:
            return
        if QMessageBox.question(self, "Blanka", f'Delete "{doc.title}"?') == QMessageBox.StandardButton.Yes:
            self.app.vault.delete_document(doc.id)
            self.refresh()


class TemplatesWindow(QWidget):
    def __init__(self, app: Blanka):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blanka - templates")
        self.resize(560, 380)
        self.list = QListWidget()
        generate = QPushButton("Generate…")
        generate.clicked.connect(self._generate)
        folder = QPushButton("Open folder")
        folder.clicked.connect(self._open_folder)
        buttons = QHBoxLayout()
        buttons.addWidget(generate)
        buttons.addWidget(folder)
        buttons.addStretch()
        layout = QVBoxLayout(self)
        layout.addWidget(self.list)
        layout.addLayout(buttons)
        self.refresh()

    def refresh(self) -> None:
        self.list.clear()
        for template in self.app.templates.all():
            item = QListWidgetItem(f"{template.name}  [{template.kind}]")
            item.setData(Qt.ItemDataRole.UserRole, template.id)
            self.list.addItem(item)

    def _open_folder(self) -> None:
        item = self.list.currentItem()
        path = (
            self.app.templates.get(item.data(Qt.ItemDataRole.UserRole)).directory if item else self.app.templates.root
        )
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(path)))

    def _generate(self) -> None:
        item = self.list.currentItem()
        if item is None or not ensure_unlocked(self.app.vault, self):
            return
        template = self.app.templates.get(item.data(Qt.ItemDataRole.UserRole))
        dialog = QDialog(self)
        dialog.setWindowTitle("Profiles")
        form = QFormLayout(dialog)
        combos = {}
        for role, description in template.roles.items():
            combo = QComboBox()
            for profile in self.app.vault.list_profiles():
                combo.addItem(profile.name, profile.id)
            combo.setToolTip(description)
            form.addRow(role, combo)
            combos[role] = combo
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() == QDialog.DialogCode.Accepted:
            profiles = {role: combo.currentData() for role, combo in combos.items()}
            GenerateDialog(self.app, template.id, profiles, parent=self).exec()


class RequestsWindow(QWidget):
    def __init__(self, app: Blanka):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blanka - requests from Claude")
        self.resize(520, 320)
        self.list = QListWidget()
        self.list.itemDoubleClicked.connect(lambda *_: self._open())
        open_button = QPushButton("Open")
        open_button.clicked.connect(self._open)
        layout = QVBoxLayout(self)
        layout.addWidget(self.list)
        layout.addWidget(open_button)
        self.refresh()

    def refresh(self) -> None:
        self.list.clear()
        for request in self.app.vault.list_requests("pending"):
            kind = "Profile data" if request.kind == "profile_input" else "Generate document"
            item = QListWidgetItem(
                f"#{request.id} {kind} - {request.payload.get('reason') or request.payload.get('template_id', '')}"
            )
            item.setData(Qt.ItemDataRole.UserRole, request.id)
            self.list.addItem(item)

    def _open(self) -> None:
        item = self.list.currentItem()
        if item:
            open_request(self.app, self.app.vault.get_request(item.data(Qt.ItemDataRole.UserRole)), self)
            self.refresh()
