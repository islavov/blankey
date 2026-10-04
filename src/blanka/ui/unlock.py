from PySide6.QtGui import QGuiApplication
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QLabel,
    QLineEdit,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from blanka.vault import Vault, WrongSecret

MIN_PASSWORD = 8


def _password_edit() -> QLineEdit:
    edit = QLineEdit()
    edit.setEchoMode(QLineEdit.EchoMode.Password)
    return edit


class SetupDialog(QDialog):
    """First run: choose the master password, then show the recovery key once."""

    def __init__(self, vault: Vault, parent: QWidget | None = None):
        super().__init__(parent)
        self.vault = vault
        self.setWindowTitle("Blanka - new vault")
        self.password = _password_edit()
        self.repeat = _password_edit()
        self.use_keyring = QCheckBox("Unlock automatically with the system keychain")
        form = QFormLayout()
        form.addRow("Master password", self.password)
        form.addRow("Repeat", self.repeat)
        form.addRow(self.use_keyring)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self._create)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addWidget(
            QLabel(
                "Personal data is encrypted with this password. Without it or the recovery key\n"
                "the data cannot be read."
            )
        )
        layout.addLayout(form)
        layout.addWidget(buttons)

    def _create(self) -> None:
        if len(self.password.text()) < MIN_PASSWORD:
            QMessageBox.warning(self, "Blanka", f"The password must be at least {MIN_PASSWORD} characters.")
            return
        if self.password.text() != self.repeat.text():
            QMessageBox.warning(self, "Blanka", "The passwords do not match.")
            return
        recovery = self.vault.initialize(self.password.text(), self.use_keyring.isChecked())
        RecoveryKeyDialog(recovery, self).exec()
        self.accept()


class RecoveryKeyDialog(QDialog):
    def __init__(self, recovery_key: str, parent: QWidget | None = None):
        super().__init__(parent)
        self.setWindowTitle("Recovery key")
        key = QLineEdit(recovery_key)
        key.setReadOnly(True)
        copy = QPushButton("Copy")
        copy.clicked.connect(lambda: QGuiApplication.clipboard().setText(recovery_key))
        ok = QPushButton("I saved it")
        ok.clicked.connect(self.accept)
        layout = QVBoxLayout(self)
        layout.addWidget(
            QLabel(
                "Store this key somewhere safe. It is shown only once.\n"
                "It lets you set a new password if you forget the current one."
            )
        )
        layout.addWidget(key)
        layout.addWidget(copy)
        layout.addWidget(ok)


class UnlockDialog(QDialog):
    def __init__(self, vault: Vault, reason: str = "", parent: QWidget | None = None):
        super().__init__(parent)
        self.vault = vault
        self.setWindowTitle("Blanka - unlock")
        self.password = _password_edit()
        recover = QPushButton("Forgot password…")
        recover.setFlat(True)
        recover.clicked.connect(self._recover)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self._unlock)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        if reason:
            layout.addWidget(QLabel(reason))
        form = QFormLayout()
        form.addRow("Password", self.password)
        layout.addLayout(form)
        layout.addWidget(recover)
        layout.addWidget(buttons)

    def _unlock(self) -> None:
        try:
            self.vault.unlock(self.password.text())
        except WrongSecret:
            QMessageBox.warning(self, "Blanka", "Wrong password.")
            self.password.selectAll()
            return
        self.accept()

    def _recover(self) -> None:
        dialog = QDialog(self)
        dialog.setWindowTitle("Recovery")
        key = QLineEdit()
        new_password = _password_edit()
        form = QFormLayout(dialog)
        form.addRow("Recovery key", key)
        form.addRow("New password", new_password)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        if len(new_password.text()) < MIN_PASSWORD:
            QMessageBox.warning(self, "Blanka", f"The password must be at least {MIN_PASSWORD} characters.")
            return
        try:
            self.vault.unlock_with_recovery(key.text(), new_password.text())
        except WrongSecret:
            QMessageBox.warning(self, "Blanka", "Invalid recovery key.")
            return
        self.accept()


def ensure_unlocked(vault: Vault, parent: QWidget | None = None, reason: str = "") -> bool:
    if vault.unlocked or vault.unlock_with_keyring():
        return True
    return UnlockDialog(vault, reason, parent).exec() == QDialog.DialogCode.Accepted
