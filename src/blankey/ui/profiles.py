from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QComboBox,
    QHBoxLayout,
    QInputDialog,
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
from blankey.ui.unlock import ensure_unlocked
from blankey.ui.widgets import fit_to_screen
from blankey.vault import FieldInput, FieldType
from blankey.vault.fieldtypes import validate

COLUMNS = ["Key", "Label", "Type", "Value"]


class ProfilesWindow(QWidget):
    def __init__(self, app: Blankey):
        super().__init__()
        self.app = app
        self.setWindowTitle("Blankey - profiles")
        fit_to_screen(self, 0.7, 0.7, 1000, 620)

        self.profiles = QListWidget()
        self.profiles.currentItemChanged.connect(lambda *_: self._load_fields())
        add_profile = QPushButton("New profile")
        add_profile.clicked.connect(self._add_profile)
        rename_profile = QPushButton("Rename")
        rename_profile.clicked.connect(self._rename_profile)
        delete_profile = QPushButton("Delete")
        delete_profile.clicked.connect(self._delete_profile)

        self.table = QTableWidget(0, len(COLUMNS))
        self.table.setHorizontalHeaderLabels(COLUMNS)
        self.table.horizontalHeader().setStretchLastSection(True)
        for column, width in enumerate((260, 240, 120)):
            self.table.setColumnWidth(column, width)
        add_field = QPushButton("Add field")
        add_field.clicked.connect(lambda: self._append_row("", "", FieldType.TEXT, ""))
        remove_field = QPushButton("Remove field")
        remove_field.clicked.connect(self._remove_row)
        save = QPushButton("Save")
        save.clicked.connect(self._save)

        left = QVBoxLayout()
        left.addWidget(self.profiles)
        for button in (add_profile, rename_profile, delete_profile):
            left.addWidget(button)
        right = QVBoxLayout()
        right.addWidget(self.table)
        row = QHBoxLayout()
        for button in (add_field, remove_field):
            row.addWidget(button)
        row.addStretch()
        row.addWidget(save)
        right.addLayout(row)
        layout = QHBoxLayout(self)
        layout.addLayout(left, 1)
        layout.addLayout(right, 3)
        self._removed: set[str] = set()
        self.refresh()

    def refresh(self) -> None:
        current = self._profile_id()
        self.profiles.clear()
        for profile in self.app.vault.list_profiles():
            item = QListWidgetItem(f"{profile.name}  ({profile.kind})" if profile.kind else profile.name)
            item.setData(Qt.ItemDataRole.UserRole, profile.id)
            self.profiles.addItem(item)
            if profile.id == current:
                self.profiles.setCurrentItem(item)
        if self.profiles.currentItem() is None and self.profiles.count():
            self.profiles.setCurrentRow(0)

    def _profile_id(self) -> int | None:
        item = self.profiles.currentItem()
        return item.data(Qt.ItemDataRole.UserRole) if item else None

    def _load_fields(self) -> None:
        self.table.setRowCount(0)
        self._removed.clear()
        profile_id = self._profile_id()
        if profile_id is None or not ensure_unlocked(self.app, self):
            return
        values = self.app.vault.get_values(profile_id)
        for info in self.app.vault.describe(profile_id):
            self._append_row(info.key, info.label, info.type, values.get(info.key, (info.type, ""))[1])

    def _append_row(self, key: str, label: str, field_type: FieldType, value: str) -> None:
        row = self.table.rowCount()
        self.table.insertRow(row)
        self.table.setItem(row, 0, QTableWidgetItem(key))
        self.table.setItem(row, 1, QTableWidgetItem(label))
        combo = QComboBox()
        combo.addItems([str(t) for t in FieldType])
        combo.setCurrentText(str(field_type))
        self.table.setCellWidget(row, 2, combo)
        self.table.setItem(row, 3, QTableWidgetItem(value))

    def _remove_row(self) -> None:
        row = self.table.currentRow()
        if row < 0:
            return
        key = self.table.item(row, 0).text().strip()
        if key:
            self._removed.add(key)
        self.table.removeRow(row)

    def _save(self) -> None:
        profile_id = self._profile_id()
        if profile_id is None or not ensure_unlocked(self.app, self):
            return
        inputs, errors, seen = [], [], set()
        for row in range(self.table.rowCount()):
            key = self.table.item(row, 0).text().strip()
            if not key:
                continue
            if key in seen:
                errors.append(f"{key}: duplicate key")
            seen.add(key)
            field_type = FieldType(self.table.cellWidget(row, 2).currentText())
            value = self.table.item(row, 3).text().strip()
            if error := validate(field_type, value):
                errors.append(f"{key}: {error}")
            inputs.append(FieldInput(key, value, self.table.item(row, 1).text().strip(), field_type))
        if errors:
            QMessageBox.warning(self, "Blankey", "\n".join(errors))
            return
        for key in self._removed - seen:
            self.app.vault.delete_field(profile_id, key)
        self.app.vault.set_values(profile_id, inputs)
        self.app.vault.audit("user", "save_profile", str(profile_id))
        self._load_fields()

    def _add_profile(self) -> None:
        name, ok = QInputDialog.getText(self, "New profile", "Name")
        if not ok or not name.strip():
            return
        kind, _ = QInputDialog.getText(self, "New profile", "Kind (person, loan, company…)")
        profile_id = self.app.vault.create_profile(name.strip(), kind.strip())
        self.refresh()
        for i in range(self.profiles.count()):
            if self.profiles.item(i).data(Qt.ItemDataRole.UserRole) == profile_id:
                self.profiles.setCurrentRow(i)

    def _rename_profile(self) -> None:
        profile_id = self._profile_id()
        if profile_id is None:
            return
        profile = self.app.vault.get_profile(profile_id)
        name, ok = QInputDialog.getText(self, "Rename", "Name", text=profile.name)
        if not ok:
            return
        kind, ok = QInputDialog.getText(self, "Rename", "Kind", text=profile.kind)
        if ok:
            self.app.vault.update_profile(profile_id, name.strip() or profile.name, kind.strip())
            self.refresh()

    def _delete_profile(self) -> None:
        profile_id = self._profile_id()
        if profile_id is None:
            return
        profile = self.app.vault.get_profile(profile_id)
        answer = QMessageBox.question(self, "Blankey", f'Delete profile "{profile.name}" and all its data?')
        if answer == QMessageBox.StandardButton.Yes:
            self.app.vault.delete_profile(profile_id)
            self.app.vault.audit("user", "delete_profile", str(profile_id))
            self.refresh()
