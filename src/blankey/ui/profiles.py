from PySide6.QtCore import Qt
from PySide6.QtGui import QKeySequence, QPalette
from PySide6.QtWidgets import (
    QComboBox,
    QFrame,
    QHBoxLayout,
    QInputDialog,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from blankey.core import Blankey
from blankey.ui.unlock import ensure_unlocked
from blankey.ui.widgets import Page, confirm, filter_table, style_list, style_table
from blankey.vault import FieldInput, FieldType
from blankey.vault.fieldtypes import validate

COLUMNS = ["Key", "Label", "Type", "Value"]


class ProfilesPage(Page):
    title = "Profiles"
    symbol = "person.crop.circle"

    def __init__(self, app: Blankey, parent: QWidget | None = None):
        super().__init__(app, parent)
        self.profiles = QListWidget()
        style_list(self.profiles)
        self.profiles.setAlternatingRowColors(False)
        # long names wrap onto more lines instead of being cut off
        self.profiles.setWordWrap(True)
        self.profiles.setTextElideMode(Qt.TextElideMode.ElideNone)
        self.profiles.setUniformItemSizes(False)
        self.profiles.setResizeMode(QListWidget.ResizeMode.Adjust)
        self.profiles.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.profiles.setSpacing(3)
        self.profiles.currentItemChanged.connect(lambda *_: self._load_fields())
        self.profile_search = QLineEdit()
        self.profile_search.setPlaceholderText("Search profiles")
        self.profile_search.setClearButtonEnabled(True)
        self.profile_search.textChanged.connect(self._filter_profiles)
        sidebar = QWidget()
        sidebar.setFixedWidth(240)
        sidebar_layout = QVBoxLayout(sidebar)
        sidebar_layout.setContentsMargins(0, 8, 0, 0)
        sidebar_layout.setSpacing(6)
        search_row = QHBoxLayout()
        search_row.setContentsMargins(8, 0, 8, 0)
        search_row.addWidget(self.profile_search)
        sidebar_layout.addLayout(search_row)
        sidebar_layout.addWidget(self.profiles, 1)

        self.table = QTableWidget(0, len(COLUMNS))
        self.table.setHorizontalHeaderLabels(COLUMNS)
        style_table(self.table, editable=True)
        self.table.setSelectionMode(QTableWidget.SelectionMode.ExtendedSelection)
        for column, width in enumerate((260, 240, 120)):
            self.table.setColumnWidth(column, width)

        add_profile = self.action("New profile", "person.badge.plus", self._add_profile)
        rename = self.action("Rename", "pencil", self._rename_profile)
        delete_profile = self.action("Delete profile", "person.badge.minus", self._delete_profile)
        add_field = self.action("Add field", "plus", lambda: self._append_row("", "", FieldType.TEXT, ""))
        remove_fields = self.action("Remove fields", "minus", self._remove_rows, QKeySequence("Ctrl+Backspace"))
        self.profiles.setContextMenuPolicy(Qt.ContextMenuPolicy.ActionsContextMenu)
        self.profiles.addActions([add_profile, rename, delete_profile])
        self.table.setContextMenuPolicy(Qt.ContextMenuPolicy.ActionsContextMenu)
        self.table.addActions([add_field, remove_fields])
        self.action("Save", "checkmark.circle", self._save, QKeySequence.StandardKey.Save)

        divider = QFrame()
        divider.setFrameShape(QFrame.Shape.VLine)
        divider.setFrameShadow(QFrame.Shadow.Plain)
        divider.setForegroundRole(QPalette.ColorRole.Mid)
        layout = QHBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(sidebar)
        layout.addWidget(divider)
        layout.addWidget(self.table, 1)
        self._removed: set[str] = set()
        self.refresh()

    def activate(self) -> bool:
        if not ensure_unlocked(self.app, self.window()):
            return False
        self._load_fields()
        return True

    def subtitle(self) -> str:
        return f"{self.profiles.count()} profiles"

    def filter(self, text: str) -> None:
        filter_table(self.table, text)

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
        self._filter_profiles(self.profile_search.text())
        self.changed.emit()

    def _filter_profiles(self, text: str) -> None:
        needle = text.strip().casefold()
        for row in range(self.profiles.count()):
            item = self.profiles.item(row)
            item.setHidden(bool(needle) and needle not in item.text().casefold())

    def _profile_id(self) -> int | None:
        item = self.profiles.currentItem()
        return item.data(Qt.ItemDataRole.UserRole) if item else None

    def _load_fields(self) -> None:
        self.table.setRowCount(0)
        self._removed.clear()
        profile_id = self._profile_id()
        if profile_id is None or not self.app.vault.unlocked:
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

    def _remove_rows(self) -> None:
        """Removes the selected rows (Cmd-A selects all); the vault changes on Save."""
        rows = sorted({index.row() for index in self.table.selectionModel().selectedRows()}, reverse=True)
        if not rows and self.table.currentRow() >= 0:
            rows = [self.table.currentRow()]
        for row in rows:
            if key := self.table.item(row, 0).text().strip():
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
        informative = "All its fields are removed from the vault."
        if confirm(self.window(), f'Delete the profile "{profile.name}"?', informative):
            self.app.vault.delete_profile(profile_id)
            self.app.vault.audit("user", "delete_profile", str(profile_id))
            self.refresh()
