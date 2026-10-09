"""The documents being filled, shown whole next to the fill form with the chosen values in place.

The selected field is highlighted; clicking a value in the preview selects its field in the form.
"""

import contextlib
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QRectF, QSize, Qt, Signal
from PySide6.QtGui import QColor, QMouseEvent, QPainter, QPaintEvent, QPixmap
from PySide6.QtWidgets import QScrollArea, QSizePolicy, QStackedWidget, QTabBar, QVBoxLayout, QWidget

from blankey.render import pdf_form, preview
from blankey.templates import Template, TemplateKind, docx, engine, fill
from blankey.templates.bindings import as_text
from blankey.ui.widgets import PAPER_CSS, accent_color, paper_browser, secondary_label

PDF_DPI = 90
Boxes = dict[str, list[tuple[int, tuple]]]  # name -> [(1-based page, (x0, y0, x1, y1) in points, origin bottom-left)]


class DocxPage(QWidget):
    def __init__(self, template: Template, on_click: Callable[[str], None]):
        super().__init__()
        self.template = template
        self.names = set(docx.variables(template.source_path))
        self.active: str | None = None
        self.browser = paper_browser()
        self.browser.anchorClicked.connect(lambda url: on_click(url.path()) if url.scheme() == "field" else None)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.browser)
        self.set_values(None)

    def set_values(self, values: dict[str, Any] | None) -> None:
        texts = None if values is None else {k: as_text(v) for k, v in values.items()}
        self.html = docx.to_html(self.template.source_path, self.template.labels, texts)
        self._show(scroll_to_active=False)

    def highlight(self, var: str) -> None:
        self.active = var
        self._show(scroll_to_active=True)

    def _show(self, scroll_to_active: bool) -> None:
        rule = ""
        if self.active:
            rule = f".b-{self.active} {{ background-color: {accent_color(self).name()}; color: white; }}"
        self.browser.document().setDefaultStyleSheet(PAPER_CSS + rule)
        position = self.browser.verticalScrollBar().value()
        self.browser.setHtml(self.html)  # the stylesheet applies when the HTML is set
        if scroll_to_active and self.active:
            self.browser.scrollToAnchor(f"blank-{self.active}")
        else:
            self.browser.verticalScrollBar().setValue(position)


class PageView(QWidget):
    """One PDF page, scaled to the width, with the active field's rectangles highlighted. Fields are clickable."""

    def __init__(self, png: bytes, size: tuple[float, float], rects: dict[str, list[tuple]], on_click):
        super().__init__()
        self.pixmap = QPixmap()
        self.pixmap.loadFromData(png, "PNG")
        self.page_width, self.page_height = size
        self.rects = rects  # field name -> [(x0, y0, x1, y1)] in PDF points, origin bottom-left
        self.on_click = on_click
        self.active: str | None = None
        policy = QSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Preferred)
        policy.setHeightForWidth(True)
        self.setSizePolicy(policy)
        self.setMouseTracking(True)

    def hasHeightForWidth(self) -> bool:
        return True

    def heightForWidth(self, width: int) -> int:
        return round(width * self.page_height / self.page_width)

    def sizeHint(self) -> QSize:
        return QSize(600, self.heightForWidth(600))

    def field_rects(self, name: str) -> list[QRectF]:
        scale = self.width() / self.page_width
        return [
            QRectF(x0 * scale, (self.page_height - y1) * scale, (x1 - x0) * scale, (y1 - y0) * scale)
            for x0, y0, x1, y1 in self.rects.get(name, [])
        ]

    def field_at(self, x: float, y: float) -> str | None:
        for name in self.rects:
            if any(rect.adjusted(-2, -2, 2, 2).contains(x, y) for rect in self.field_rects(name)):
                return name
        return None

    def mouseMoveEvent(self, event: QMouseEvent) -> None:
        hit = self.field_at(event.position().x(), event.position().y())
        self.setCursor(Qt.CursorShape.PointingHandCursor if hit else Qt.CursorShape.ArrowCursor)

    def mousePressEvent(self, event: QMouseEvent) -> None:
        if name := self.field_at(event.position().x(), event.position().y()):
            self.on_click(name)

    def paintEvent(self, event: QPaintEvent) -> None:
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        painter.drawPixmap(self.rect(), self.pixmap)
        if self.active:
            accent = accent_color(self)
            tint = QColor(accent)
            tint.setAlphaF(0.3)
            painter.setPen(accent)
            painter.setBrush(tint)
            for rect in self.field_rects(self.active):
                painter.drawRoundedRect(rect.adjusted(-2, -2, 2, 2), 3, 3)
        painter.end()


class PdfPage(QScrollArea):
    """A PDF form or Typst template rendered with the values, one PageView per page."""

    def __init__(self, template: Template, on_click: Callable[[str], None]):
        super().__init__()
        self.template = template
        self.on_click = on_click
        self.active: str | None = None
        self.views: list[PageView] = []
        self.setWidgetResizable(True)
        self.setFrameShape(QScrollArea.Shape.NoFrame)
        if template.kind == TemplateKind.PDF_FORM:
            self.form_boxes: Boxes = {}
            for f in pdf_form.inspect_form(template.source):
                self.form_boxes.setdefault(f.name, []).extend((f.page, rect) for rect in f.rects)
            self.names = set(self.form_boxes)
        else:
            self.markers = {name: f"[{name}]" for name in template.fields}
            pdf = engine.render_values(template, self.markers).content
            self.marker_boxes = preview.find_text(pdf, self.markers)
            self.names = set(self.marker_boxes)
        self.set_values(None)

    def set_values(self, values: dict[str, Any] | None) -> None:
        values = values or {}
        if self.template.kind == TemplateKind.PDF_FORM:
            pdf = engine.render_values(self.template, values).content if values else self.template.source
            self._show(pdf, self.form_boxes)
            return
        # Typst: empty values keep their marker; filled values are found near where their marker was
        shown = {name: as_text(values.get(name)) for name in self.markers}
        shown = {name: self.markers[name] if fill.is_empty(text) else text for name, text in shown.items()}
        pdf = engine.render_values(self.template, shown).content
        found = preview.find_text(pdf, {name: " ".join(text.split()) for name, text in shown.items()})
        boxes = {name: _nearest(found.get(name, []), anchors) for name, anchors in self.marker_boxes.items()}
        self._show(pdf, boxes)

    def _show(self, pdf: bytes, boxes: Boxes) -> None:
        position = self.verticalScrollBar().value()
        sizes = preview.page_sizes(pdf)
        container = QWidget()
        layout = QVBoxLayout(container)
        layout.setContentsMargins(16, 16, 16, 16)
        layout.setSpacing(16)
        self.views = []
        for number, png in enumerate(preview.render_pages(pdf, dpi=PDF_DPI), start=1):
            rects: dict[str, list[tuple]] = {}
            for name, places in boxes.items():
                for page, rect in places:
                    if page == number:
                        rects.setdefault(name, []).append(rect)
            view = PageView(png, sizes[number - 1], rects, self.on_click)
            view.active = self.active
            self.views.append(view)
            layout.addWidget(view)
        layout.addStretch()
        self.setWidget(container)
        self.verticalScrollBar().setValue(position)

    def highlight(self, var: str) -> None:
        self.active = var
        for view in self.views:
            view.active = var
            view.update()
        view = next((v for v in self.views if var in v.rects), None)
        if view is not None:
            rect = view.field_rects(var)[0]
            center = view.mapTo(self.widget(), rect.center().toPoint())
            self.ensureVisible(center.x(), center.y(), 40, int(self.viewport().height() * 0.4))


def _nearest(candidates: list[tuple[int, tuple]], anchors: list[tuple[int, tuple]]) -> list[tuple[int, tuple]]:
    """For each anchor (where the marker was), the candidate on the same page closest to it; the anchor itself when
    there is none (a value broken across lines is not found by text search)."""
    picks = []
    for page, (x0, y0, x1, y1) in anchors:
        same_page = [c for c in candidates if c[0] == page]
        if not same_page:
            picks.append((page, (x0, y0, x1, y1)))
            continue
        best = min(same_page, key=lambda c: abs(c[1][0] - x0) + abs(c[1][3] - y1) * 3)
        if best not in picks:
            picks.append(best)
    return picks


class FillPreview(QWidget):
    """One page per template, with tabs when there are several."""

    field_clicked = Signal(str)

    def __init__(self, templates: list[Template]):
        super().__init__()
        self.tabs = QTabBar()
        self.tabs.setDocumentMode(True)
        self.tabs.setExpanding(False)
        self.stack = QStackedWidget()
        self.templates = templates
        self.pages: list[QWidget] = []
        for template in templates:
            self.tabs.addTab(template.name)
            page = self._page(template)
            self.pages.append(page)
            self.stack.addWidget(page)
        self.tabs.setVisible(len(templates) > 1)
        self.tabs.currentChanged.connect(self.stack.setCurrentIndex)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(self.tabs)
        layout.addWidget(self.stack, 1)

    def _page(self, template: Template) -> QWidget:
        try:
            if template.kind == TemplateKind.DOCX:
                return DocxPage(template, self.field_clicked.emit)
            return PdfPage(template, self.field_clicked.emit)
        except Exception as exc:
            label = secondary_label(f"No preview: {type(exc).__name__}: {exc}", smaller=False)
            label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            label.names = set()
            return label

    def set_values(self, values: dict[str, dict[str, Any]]) -> None:
        """Values per template id, as fill.resolve returns them (without the errors)."""
        for template, page in zip(self.templates, self.pages, strict=True):
            if hasattr(page, "set_values"):
                # a value the template cannot render keeps the previous preview
                with contextlib.suppress(Exception):
                    page.set_values(values.get(template.id, {}))

    def highlight(self, var: str) -> None:
        """Show the variable in the current document if it is there, else in the first one that has it."""
        current = self.stack.currentIndex()
        if var not in self.pages[current].names:
            current = next((i for i, page in enumerate(self.pages) if var in page.names), current)
            self.tabs.setCurrentIndex(current)
            self.stack.setCurrentIndex(current)
        page = self.pages[current]
        if var in page.names:
            page.highlight(var)
