"""The main window: a source-list sidebar, a unified toolbar with per-page actions and search, and the pages."""

from collections.abc import Callable

from PySide6.QtCore import QEvent, QRect, QSize, Qt
from PySide6.QtGui import QAction, QColor, QFont, QGuiApplication, QKeySequence, QPainter, QPalette
from PySide6.QtWidgets import (
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QMessageBox,
    QPushButton,
    QSizePolicy,
    QStackedWidget,
    QStyle,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QToolBar,
    QVBoxLayout,
    QWidget,
)

from blankey.core import Blankey
from blankey.ui import macos, platform
from blankey.ui.pages import ActivityPage, DocumentsPage, FillSetsPage, TemplatesPage
from blankey.ui.profiles import ProfilesPage
from blankey.ui.widgets import Page, accent_color, secondary_color

SECTIONS: list[tuple[str, list[type[Page]]]] = [
    ("Claude", [ActivityPage]),
    ("Library", [FillSetsPage, DocumentsPage, TemplatesPage]),
    ("Vault", [ProfilesPage]),
]
PAGE_ROLE = Qt.ItemDataRole.UserRole
BADGE_ROLE = Qt.ItemDataRole.UserRole + 1


def _sidebar_color(palette: QPalette) -> QColor:
    window = palette.color(QPalette.ColorRole.Window)
    return window.darker(106) if window.lightness() > 128 else window.lighter(125)


class SidebarDelegate(QStyledItemDelegate):
    """Source-list rows: rounded gray selection, accent-tinted icon, count badge; small caps-free section headers."""

    def sizeHint(self, option: QStyleOptionViewItem, index) -> QSize:
        header = index.data(PAGE_ROLE) is None
        return QSize(option.rect.width(), 28 if header else 30)

    def paint(self, painter: QPainter, option: QStyleOptionViewItem, index) -> None:
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        rect = option.rect.adjusted(10, 1, -10, -1)
        palette = option.palette
        text_color = palette.color(QPalette.ColorRole.Text)
        font = QFont(option.font)
        if index.data(PAGE_ROLE) is None:
            font.setPointSizeF(font.pointSizeF() - 2)
            font.setWeight(QFont.Weight.DemiBold)
            painter.setFont(font)
            painter.setPen(secondary_color())
            align = Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter
            painter.drawText(rect.adjusted(10, 8, 0, 0), align, index.data())
            painter.restore()
            return
        if option.state & QStyle.StateFlag.State_Selected:
            fill = QColor(text_color)
            fill.setAlphaF(0.1 if text_color.lightness() < 128 else 0.16)
            painter.setPen(Qt.PenStyle.NoPen)
            painter.setBrush(fill)
            painter.drawRoundedRect(rect, 6, 6)
            font.setWeight(QFont.Weight.Medium)
        icon = index.data(Qt.ItemDataRole.DecorationRole)
        x = rect.left() + 10
        if icon is not None and not icon.isNull():
            icon.paint(painter, QRect(x, rect.center().y() - 8, 16, 16))
            x += 24
        painter.setFont(font)
        painter.setPen(text_color)
        text_rect = QRect(x, rect.top(), rect.right() - x - 36, rect.height())
        painter.drawText(text_rect, Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, index.data())
        badge = index.data(BADGE_ROLE) or 0
        if badge:
            small = QFont(option.font)
            small.setPointSizeF(small.pointSizeF() - 2)
            small.setWeight(QFont.Weight.DemiBold)
            text = str(badge)
            width = max(18, painter.fontMetrics().horizontalAdvance(text) + 12)
            pill = QRect(rect.right() - width - 6, rect.center().y() - 9, width, 18)
            pill_fill = QColor(text_color)
            pill_fill.setAlphaF(0.16)
            painter.setPen(Qt.PenStyle.NoPen)
            painter.setBrush(pill_fill)
            painter.drawRoundedRect(pill, 9, 9)
            painter.setFont(small)
            painter.setPen(text_color)
            painter.drawText(pill, Qt.AlignmentFlag.AlignCenter, text)
        painter.restore()


class MainWindow(QMainWindow):
    def __init__(self, app: Blankey, on_lock: Callable[[], None], on_quit: Callable[[], None]):
        super().__init__()
        self.app = app
        self.on_lock = on_lock
        self.setWindowTitle("Blankey")
        self.setUnifiedTitleAndToolBarOnMac(True)
        # zoomed: fills the screen outside the menu bar and Dock, title bar included; not a full-screen Space
        self.resize((self.screen() or QGuiApplication.primaryScreen()).availableGeometry().size())
        self.setWindowState(Qt.WindowState.WindowMaximized)

        self.pages: list[Page] = []
        self.sidebar = QListWidget()
        self.sidebar.setItemDelegate(SidebarDelegate(self.sidebar))
        self.sidebar.setFrameShape(QFrame.Shape.NoFrame)
        self.sidebar.setAttribute(Qt.WidgetAttribute.WA_MacShowFocusRect, False)
        self.sidebar.setFixedWidth(220)
        self.stack = QStackedWidget()
        for section, page_types in SECTIONS:
            header = QListWidgetItem(section)
            header.setFlags(Qt.ItemFlag.NoItemFlags)
            self.sidebar.addItem(header)
            for page_type in page_types:
                page = page_type(app)
                item = QListWidgetItem(page.title)
                item.setData(PAGE_ROLE, len(self.pages))
                self.sidebar.addItem(item)
                self.pages.append(page)
                self.stack.addWidget(page)
                page.changed.connect(self._update_page_info)

        self.lock_label = QLabel()
        lock_button = QPushButton("Lock")
        lock_button.clicked.connect(on_lock)
        footer = QHBoxLayout()
        footer.setContentsMargins(20, 10, 14, 12)
        footer.addWidget(self.lock_label, 1)
        footer.addWidget(lock_button)
        side = QWidget()
        side.setAutoFillBackground(True)
        side_layout = QVBoxLayout(side)
        side_layout.setContentsMargins(0, 8, 0, 0)
        side_layout.setSpacing(0)
        side_layout.addWidget(self.sidebar, 1)
        side_layout.addLayout(footer)
        self.side = side

        divider = QFrame()
        divider.setFrameShape(QFrame.Shape.VLine)
        divider.setFrameShadow(QFrame.Shadow.Plain)
        divider.setForegroundRole(QPalette.ColorRole.Mid)
        central = QWidget()
        layout = QHBoxLayout(central)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(side)
        layout.addWidget(divider)
        layout.addWidget(self.stack, 1)
        self.setCentralWidget(central)

        self.toolbar = QToolBar("Toolbar")
        self.toolbar.setMovable(False)
        self.toolbar.setFloatable(False)
        self.toolbar.setIconSize(QSize(18, 18))
        self.toolbar.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        self.toolbar.toggleViewAction().setVisible(False)
        self.title_label = QLabel()
        self.subtitle_label = QLabel()
        titles = QWidget()
        titles_layout = QVBoxLayout(titles)
        titles_layout.setContentsMargins(8, 0, 16, 0)
        titles_layout.setSpacing(0)
        titles_layout.addWidget(self.title_label)
        titles_layout.addWidget(self.subtitle_label)
        self.toolbar.addWidget(titles)
        spacer = QWidget()
        spacer.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        self.toolbar.addWidget(spacer)
        self.search = QLineEdit()
        self.search.setPlaceholderText("Search")
        self.search.setClearButtonEnabled(True)
        self.search.setFixedWidth(220)
        self.search.setAttribute(Qt.WidgetAttribute.WA_MacShowFocusRect, False)
        self.search.textChanged.connect(lambda text: self.current_page().filter(text))
        self.search_action = self.toolbar.addWidget(self.search)
        self.addToolBar(self.toolbar)
        self._page_actions: list[QAction] = []

        self._build_menu_bar(on_quit)
        self._apply_theme()
        self.sidebar.currentItemChanged.connect(self._on_sidebar)
        self.show_page(FillSetsPage)

    # -- pages -------------------------------------------------------------------

    def current_page(self) -> Page:
        return self.stack.currentWidget()

    def show_page(self, page_type: type[Page]) -> None:
        for row in range(self.sidebar.count()):
            index = self.sidebar.item(row).data(PAGE_ROLE)
            if index is not None and isinstance(self.pages[index], page_type):
                self.sidebar.setCurrentRow(row)
                return

    def _on_sidebar(self, current: QListWidgetItem | None, previous: QListWidgetItem | None) -> None:
        if current is None or current.data(PAGE_ROLE) is None:
            return
        page = self.pages[current.data(PAGE_ROLE)]
        if not page.activate():
            self.sidebar.blockSignals(True)
            self.sidebar.setCurrentItem(previous)
            self.sidebar.blockSignals(False)
            return
        self.stack.setCurrentWidget(page)
        for action in self._page_actions:
            self.toolbar.removeAction(action)
        self._page_actions = page.toolbar_actions
        self.toolbar.insertActions(self.search_action, self._page_actions)
        self.search.clear()
        self._update_page_info()

    def _update_page_info(self) -> None:
        page = self.current_page()
        self.title_label.setText(page.title)
        self.subtitle_label.setText(page.subtitle())
        for row in range(self.sidebar.count()):
            item = self.sidebar.item(row)
            if item.data(PAGE_ROLE) is not None:
                item.setData(BADGE_ROLE, self.pages[item.data(PAGE_ROLE)].badge())
        self.lock_label.setText("Vault unlocked" if self.app.vault.unlocked else "Vault locked")

    def refresh(self) -> None:
        for page in self.pages:
            page.refresh()
        self._update_page_info()

    # -- look ------------------------------------------------------------------

    def _apply_theme(self) -> None:
        palette = self.side.palette()
        sidebar = _sidebar_color(self.palette())
        palette.setColor(QPalette.ColorRole.Window, sidebar)
        palette.setColor(QPalette.ColorRole.Base, sidebar)
        self.side.setPalette(palette)
        self.sidebar.setPalette(palette)
        accent = accent_color(self)
        for row in range(self.sidebar.count()):
            item = self.sidebar.item(row)
            if item.data(PAGE_ROLE) is not None:
                item.setIcon(macos.symbol_icon(self.pages[item.data(PAGE_ROLE)].symbol, accent))
        ink = self.palette().color(QPalette.ColorRole.ButtonText)
        for page in self.pages:
            for action in page.toolbar_actions:
                action.setIcon(macos.symbol_icon(action.data(), ink))
        title_font = self.title_label.font()
        title_font.setPointSizeF(QLabel().font().pointSizeF() + 1)
        title_font.setWeight(QFont.Weight.DemiBold)
        self.title_label.setFont(title_font)
        for label in (self.subtitle_label, self.lock_label):
            muted = label.palette()
            muted.setColor(QPalette.ColorRole.WindowText, secondary_color(self))
            label.setPalette(muted)
            small = label.font()
            small.setPointSizeF(QLabel().font().pointSizeF() - 2)
            label.setFont(small)
        glass = macos.symbol_icon("magnifyingglass", secondary_color(self), 12)
        self.search.addAction(glass, QLineEdit.ActionPosition.LeadingPosition)

    def changeEvent(self, event: QEvent) -> None:
        if event.type() == QEvent.Type.PaletteChange and hasattr(self, "side"):
            for action in self.search.actions():
                self.search.removeAction(action)
            self._apply_theme()
        super().changeEvent(event)

    # -- menu bar & window lifecycle ---------------------------------------------

    def _build_menu_bar(self, on_quit: Callable[[], None]) -> None:
        bar = self.menuBar()
        app_menu = bar.addMenu("Blankey")
        about = app_menu.addAction("About Blankey", self._about)
        about.setMenuRole(QAction.MenuRole.AboutRole)
        quit_action = app_menu.addAction("Quit Blankey", on_quit)
        quit_action.setMenuRole(QAction.MenuRole.QuitRole)
        quit_action.setShortcut(QKeySequence.StandardKey.Quit)

        file_menu = bar.addMenu("File")
        file_menu.addAction("Lock Vault", QKeySequence("Ctrl+L"), self.on_lock)
        file_menu.addSeparator()
        file_menu.addAction("Close Window", QKeySequence.StandardKey.Close, self.close)

        edit_menu = bar.addMenu("Edit")
        edit_menu.addAction("Find", QKeySequence.StandardKey.Find, self._focus_search)

        view_menu = bar.addMenu("View")
        for number, page in enumerate(self.pages, start=1):
            view_menu.addAction(page.title, QKeySequence(f"Ctrl+{number}"), lambda p=page: self.show_page(type(p)))

        window_menu = bar.addMenu("Window")
        window_menu.addAction("Minimize", QKeySequence("Ctrl+M"), self.showMinimized)

    def _focus_search(self) -> None:
        self.search.setFocus()
        self.search.selectAll()

    def _about(self) -> None:
        QMessageBox.about(
            self,
            "Blankey",
            "Blankey keeps personal data encrypted on this Mac and fills document templates for Claude.",
        )

    def showEvent(self, event) -> None:
        platform.set_dock_visible(True)
        platform.bring_to_front()
        super().showEvent(event)

    def closeEvent(self, event) -> None:
        platform.set_dock_visible(False)
        super().closeEvent(event)
