import signal
import sys
import time

from PySide6.QtCore import QEvent, QObject, QTimer, Signal
from PySide6.QtGui import QAction, QGuiApplication
from PySide6.QtWidgets import QApplication, QDialog, QMenu, QMessageBox, QSystemTrayIcon, QWidget

from blankey import biometric, bridge
from blankey.config import load_config
from blankey.core import Blankey
from blankey.mcp_server import McpThread
from blankey.ui import icons, platform
from blankey.ui.profiles import ProfilesWindow
from blankey.ui.requests import open_request
from blankey.ui.unlock import SetupDialog, ensure_unlocked
from blankey.ui.windows import DocumentsWindow, RequestsWindow, TemplatesWindow, clear_opened_documents

USER_INPUT_EVENTS = {QEvent.Type.KeyPress, QEvent.Type.MouseButtonPress, QEvent.Type.Wheel}


class Bridge(QObject):
    """Carries request notifications from the MCP thread to the Qt thread."""

    request_created = Signal(int)


class Tray(QObject):
    def __init__(self, qt_app: QApplication, app: Blankey, mcp: McpThread):
        super().__init__(qt_app)
        self.qt_app = qt_app
        self.app = app
        self.mcp = mcp
        self.windows: dict[str, QWidget] = {}
        self.last_activity = time.monotonic()
        self.handling_requests = False

        self.bridge = Bridge()
        self.bridge.request_created.connect(self._on_request)
        app.on_request = self.bridge.request_created.emit

        self.tray = QSystemTrayIcon()
        self.tray.setToolTip("Blankey")
        self.tray.messageClicked.connect(self._process_requests)
        self.menu = QMenu()
        self.menu.aboutToShow.connect(self._build_menu)
        self.tray.setContextMenu(self.menu)
        self._refresh_icon()
        self.tray.show()

        qt_app.installEventFilter(self)
        self.lock_timer = QTimer(self)
        self.lock_timer.timeout.connect(self._auto_lock)
        self.lock_timer.start(30_000)

    # -- menu ----------------------------------------------------------------

    def _build_menu(self) -> None:
        self.menu.clear()
        vault = self.app.vault
        state = QAction("Unlocked" if vault.unlocked else "Locked", self.menu)
        state.setEnabled(False)
        self.menu.addAction(state)
        if vault.unlocked:
            self.menu.addAction("Lock now", self._lock)
        else:
            self.menu.addAction("Unlock…", self._unlock)
        self.menu.addSeparator()
        pending = len(vault.list_requests("pending"))
        requests = self.menu.addAction(f"Requests from Claude ({pending})", lambda: self._show("requests"))
        requests.setEnabled(pending > 0)
        self.menu.addAction("Profiles…", lambda: self._show("profiles"))
        self.menu.addAction("Documents…", lambda: self._show("documents"))
        self.menu.addAction("Templates…", lambda: self._show("templates"))
        self.menu.addSeparator()
        running = self.mcp.is_alive() and self.mcp.server.started
        status = QAction(f"MCP: {self.app.config.mcp_url}" if running else "MCP: not running", self.menu)
        status.setEnabled(False)
        self.menu.addAction(status)
        self.menu.addAction("Connect Claude Desktop", self._connect_claude_desktop)
        self.menu.addAction("Copy Claude Code command", self._copy_mcp_command)
        label = "Unlock with Touch ID" if biometric.available() else "Unlock with keychain"
        keyring_action = QAction(label, self.menu, checkable=True)
        keyring_action.setChecked(vault.keyring_enabled)
        keyring_action.setEnabled(vault.unlocked)
        keyring_action.toggled.connect(self._toggle_keyring)
        self.menu.addAction(keyring_action)
        autostart = QAction("Start at login", self.menu, checkable=True)
        autostart.setChecked(platform.autostart_enabled())
        autostart.toggled.connect(platform.set_autostart)
        self.menu.addAction(autostart)
        self.menu.addSeparator()
        self.menu.addAction("Quit", self.quit)

    def _show(self, name: str) -> None:
        factories = {
            "profiles": ProfilesWindow,
            "documents": DocumentsWindow,
            "templates": TemplatesWindow,
            "requests": RequestsWindow,
        }
        if name == "profiles" and not ensure_unlocked(self.app):
            return
        window = self.windows.get(name)
        if window is None:
            window = self.windows[name] = factories[name](self.app)
        else:
            window.refresh()
        platform.bring_to_front()
        window.show()
        window.raise_()
        window.activateWindow()
        self._refresh_icon()

    def _copy_mcp_command(self) -> None:
        QGuiApplication.clipboard().setText(f"claude mcp add --transport http blankey {self.app.config.mcp_url}")
        self.tray.showMessage("Blankey", "Command copied.", QSystemTrayIcon.MessageIcon.Information, 3000)

    def _connect_claude_desktop(self) -> None:
        try:
            path = platform.connect_claude_desktop()
        except (OSError, ValueError) as exc:
            QMessageBox.warning(None, "Blankey", f"Could not write the config: {exc}")
            return
        message = f"Added to {path.name}. Restart Claude Desktop."
        self.tray.showMessage("Blankey", message, QSystemTrayIcon.MessageIcon.Information, 5000)

    def _toggle_keyring(self, enabled: bool) -> None:
        if enabled:
            self.app.vault.enable_keyring()
        else:
            self.app.vault.disable_keyring()

    # -- lock state ------------------------------------------------------------

    def _unlock(self) -> None:
        platform.bring_to_front()
        ensure_unlocked(self.app)
        self._refresh_icon()

    def _lock(self) -> None:
        for window in self.windows.values():
            window.close()
        self.app.vault.lock()
        clear_opened_documents()
        self._refresh_icon()

    def _auto_lock(self) -> None:
        idle = time.monotonic() - self.last_activity
        if self.app.vault.unlocked and idle > self.app.config.auto_lock_minutes * 60 and not self.handling_requests:
            self._lock()

    def eventFilter(self, watched: QObject, event: QEvent) -> bool:
        if event.type() in USER_INPUT_EVENTS:
            self.last_activity = time.monotonic()
        return False

    def _refresh_icon(self) -> None:
        pending = len(self.app.vault.list_requests("pending"))
        self.tray.setIcon(icons.tray_icon(locked=not self.app.vault.unlocked, pending=pending > 0))

    # -- requests from Claude ----------------------------------------------------

    def _on_request(self, request_id: int) -> None:
        self._refresh_icon()
        request = self.app.vault.get_request(request_id)
        message = request.payload.get("reason") or "Claude asks you to approve a document."
        self.tray.showMessage("Request from Claude", message, QSystemTrayIcon.MessageIcon.Information, 8000)
        QTimer.singleShot(0, self._process_requests)

    def _process_requests(self) -> None:
        if self.handling_requests:
            return
        self.handling_requests = True
        try:
            for request in self.app.vault.list_requests("pending"):
                platform.bring_to_front()
                open_request(self.app, request)
                if self.app.vault.get_request(request.id).status == "pending":
                    break  # user dismissed the unlock prompt; keep the rest queued
        finally:
            self.handling_requests = False
            self.last_activity = time.monotonic()
            self._refresh_icon()
            if window := self.windows.get("documents"):
                window.refresh()

    def quit(self) -> None:
        self.mcp.stop()
        self.app.vault.close()
        clear_opened_documents()
        self.qt_app.quit()


def main() -> None:
    if sys.argv[1:] == ["mcp"]:
        bridge.main()
        return
    qt_app = QApplication(sys.argv)
    qt_app.setApplicationName("Blankey")
    qt_app.setQuitOnLastWindowClosed(False)
    qt_app.setWindowIcon(icons.app_icon())
    if not QSystemTrayIcon.isSystemTrayAvailable():
        QMessageBox.critical(None, "Blankey", "No system tray available (on GNOME install the AppIndicator extension).")
        sys.exit(1)
    config = load_config()
    if bridge.server_running(config.mcp_url):
        QMessageBox.information(None, "Blankey", "Blankey is already running. Look for the B icon in the menu bar.")
        sys.exit(0)
    platform.hide_dock_icon()

    app = Blankey(config)
    if not app.vault.initialized:
        platform.bring_to_front()
        if SetupDialog(app.vault).exec() != QDialog.DialogCode.Accepted:
            sys.exit(0)
    elif not biometric.available():
        app.vault.unlock_with_keyring()  # with Touch ID the vault stays locked until first use

    mcp = McpThread(app)
    mcp.start()
    tray = Tray(qt_app, app, mcp)
    signal.signal(signal.SIGINT, lambda *_: tray.quit())
    # Python only runs signal handlers between bytecodes; wake it up periodically while Qt idles.
    wakeup = QTimer()
    wakeup.timeout.connect(lambda: None)
    wakeup.start(250)
    sys.exit(qt_app.exec())
