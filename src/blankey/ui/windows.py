"""Documents, templates and pending-requests windows."""

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

from blankey.core import Blankey
from blankey.ui.fill import FillDialog
from blankey.ui.requests import GenerateDialog, open_request
from blankey.ui.unlock import ensure_unlocked
from blankey.ui.widgets import fit_to_screen, open_documents

REQUEST_LABELS = {"profile_input": "Profile data", "generate": "Generate document", "fill": "Fill documents"}


class DocumentsWindow(QWidget):
    def __init__(self, app: Blankey):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blankey - documents")
        fit_to_screen(self, 0.6, 0.55, 960, 520)
        self.table = QTableWidget(0, 6)
        self.table.setHorizontalHeaderLabels(["#", "Title", "Template", "Signature", "Password", "Created"])
        self.table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.table.setSelectionMode(QTableWidget.SelectionMode.ExtendedSelection)
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
        self.table.resizeColumnsToContents()

    def _selected(self):
        row = self.table.currentRow()
        if row < 0:
            return None
        doc_id = self.table.item(row, 0).data(Qt.ItemDataRole.UserRole)
        return next(d for d in self.app.vault.list_documents() if d.id == doc_id)

    def _selected_all(self):
        rows = self.table.selectionModel().selectedRows()
        doc_ids = {self.table.item(index.row(), 0).data(Qt.ItemDataRole.UserRole) for index in rows}
        return [d for d in self.app.vault.list_documents() if d.id in doc_ids]

    def _open(self) -> None:
        docs = self._selected_all()
        if not docs or not ensure_unlocked(self.app, self):
            return
        open_documents(self.app.vault, [d.id for d in docs])

    def _export(self) -> None:
        doc = self._selected()
        if doc is None or not ensure_unlocked(self.app, self):
            return
        target, _ = QFileDialog.getSaveFileName(self, "Export", str(Path.home() / doc.filename))
        if target:
            Path(target).write_bytes(self.app.vault.load_document(doc.id))
            self.app.vault.audit("user", "export_document", str(doc.id))

    def _delete(self) -> None:
        docs = self._selected_all()
        if not docs:
            return
        question = f'Delete "{docs[0].title}"?' if len(docs) == 1 else f"Delete {len(docs)} documents?"
        if QMessageBox.question(self, "Blankey", question) == QMessageBox.StandardButton.Yes:
            for doc in docs:
                self.app.vault.delete_document(doc.id)
            self.refresh()


class TemplatesWindow(QWidget):
    def __init__(self, app: Blankey):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blankey - templates")
        fit_to_screen(self, 0.45, 0.5, 720, 460)
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
        if item is None or not ensure_unlocked(self.app, self):
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
    def __init__(self, app: Blankey):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blankey - requests from Claude")
        fit_to_screen(self, 0.45, 0.45, 720, 420)
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
            kind = REQUEST_LABELS.get(request.kind, request.kind)
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


class FillSetsWindow(QWidget):
    def __init__(self, app: Blankey):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blankey - fill sets")
        fit_to_screen(self, 0.6, 0.55, 960, 520)
        self.table = QTableWidget(0, 4)
        self.table.setHorizontalHeaderLabels(["#", "Name", "Templates", "Updated"])
        self.table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.table.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.table.horizontalHeader().setStretchLastSection(True)
        self.table.doubleClicked.connect(lambda *_: self._open())
        buttons = QHBoxLayout()
        for label, slot in (
            ("Open", self._open),
            ("Generate", self._generate),
            ("Export YAML…", self._export_yaml),
            ("Export bundle…", self._export_bundle),
            ("Import…", self._import),
            ("Delete", self._delete),
        ):
            button = QPushButton(label)
            button.clicked.connect(slot)
            buttons.addWidget(button)
        buttons.addStretch()
        layout = QVBoxLayout(self)
        layout.addWidget(self.table)
        layout.addLayout(buttons)
        self.refresh()

    def refresh(self) -> None:
        sets = self.app.vault.list_fill_sets()
        self.table.setRowCount(len(sets))
        for row, info in enumerate(sets):
            cells = [str(info.id), info.name, ", ".join(info.templates), info.updated_at.replace("T", " ")[:16]]
            for col, text in enumerate(cells):
                item = QTableWidgetItem(text)
                item.setData(Qt.ItemDataRole.UserRole, info.id)
                self.table.setItem(row, col, item)
        self.table.resizeColumnsToContents()

    def _selected(self):
        row = self.table.currentRow()
        if row < 0 or not ensure_unlocked(self.app, self):
            return None
        fill_set_id = self.table.item(row, 0).data(Qt.ItemDataRole.UserRole)
        return next(f for f in self.app.vault.list_fill_sets() if f.id == fill_set_id)

    def _open(self) -> None:
        if info := self._selected():
            FillDialog(self.app, info.templates, info.profiles, info.name, fill_set_id=info.id, parent=self).exec()
            self.refresh()

    def _generate(self) -> None:
        if info := self._selected():
            try:
                document_ids = self.app.generate_fill(info.id)
            except Exception as exc:
                QMessageBox.critical(self, "Blankey", f"Generation failed: {exc}")
                return
            open_documents(self.app.vault, document_ids)

    def _export_yaml(self) -> None:
        if info := self._selected():
            target, _ = QFileDialog.getSaveFileName(self, "Export", str(Path.home() / f"{info.name}.yaml"))
            if target:
                Path(target).write_text(self.app.export_fill_set(info.id), encoding="utf-8")

    def _export_bundle(self) -> None:
        if info := self._selected():
            target, _ = QFileDialog.getSaveFileName(self, "Export", str(Path.home() / f"{info.name}.zip"))
            if target:
                self.app.export_fill_bundle(info.id, Path(target))

    def _import(self) -> None:
        if not ensure_unlocked(self.app, self):
            return
        source, _ = QFileDialog.getOpenFileName(self, "Import fill set", str(Path.home()), "YAML (*.yaml *.yml)")
        if not source:
            return
        try:
            self.app.import_fill_set(Path(source).read_text(encoding="utf-8"))
        except (KeyError, ValueError) as exc:
            QMessageBox.warning(self, "Blankey", f"Import failed: {exc}")
        self.refresh()

    def _delete(self) -> None:
        info = self._selected()
        if info and QMessageBox.question(self, "Blankey", f'Delete "{info.name}"?') == QMessageBox.StandardButton.Yes:
            self.app.vault.delete_fill_set(info.id)
            self.refresh()
