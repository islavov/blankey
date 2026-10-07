"""Main window pages: Claude activity, fill sets, documents and templates."""

from pathlib import Path

from PySide6.QtCore import Qt, QUrl
from PySide6.QtGui import QColor, QDesktopServices, QFont, QKeySequence, QPalette
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QFrame,
    QHeaderView,
    QMessageBox,
    QSplitter,
    QStackedLayout,
    QTableWidget,
    QTableWidgetItem,
    QTextBrowser,
    QVBoxLayout,
    QWidget,
)

from blankey.core import Blankey
from blankey.render import pdf_form, preview
from blankey.templates import Template, TemplateKind, docx, engine
from blankey.templates.bindings import example_context
from blankey.ui.fill import FillDialog
from blankey.ui.requests import GenerateDialog, open_request
from blankey.ui.unlock import ensure_unlocked
from blankey.ui.widgets import (
    Page,
    PagePreview,
    confirm,
    filter_table,
    heading_label,
    open_documents,
    secondary_label,
    style_table,
)
from blankey.vault import Request

REQUEST_LABELS = {"profile_input": "Profile data", "generate": "Generate document", "fill": "Fill documents"}


def _when(timestamp: str) -> str:
    return timestamp.replace("T", " ")[:16]


class TablePage(Page):
    """A page holding one Finder-style table, with an empty-state message when it has no rows."""

    columns: tuple[str, ...] = ()
    empty_text = ""

    def __init__(self, app: Blankey, parent: QWidget | None = None):
        super().__init__(app, parent)
        self.table = QTableWidget(0, len(self.columns))
        self.table.setHorizontalHeaderLabels(list(self.columns))
        style_table(self.table)
        self.table.doubleClicked.connect(lambda *_: self.open())
        empty = secondary_label(self.empty_text, smaller=False)
        empty.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.list_area = QWidget()
        self.stack = QStackedLayout(self.list_area)
        self.stack.addWidget(self.table)
        self.stack.addWidget(empty)
        self.page_layout = QVBoxLayout(self)
        self.page_layout.setContentsMargins(0, 0, 0, 0)
        self.page_layout.addWidget(self.list_area)

    def set_rows(self, rows: list[tuple[int, list[str]]]) -> None:
        self.table.setRowCount(len(rows))
        for row, (row_id, cells) in enumerate(rows):
            for col, text in enumerate(cells):
                item = QTableWidgetItem(text)
                item.setData(Qt.ItemDataRole.UserRole, row_id)
                item.setToolTip(text)
                self.table.setItem(row, col, item)
        self.table.resizeColumnsToContents()
        for col in range(self.table.columnCount() - 1):
            self.table.setColumnWidth(col, self.table.columnWidth(col) + 32)
        self.stack.setCurrentIndex(0 if rows else 1)
        self.changed.emit()

    def current_id(self) -> int | None:
        row = self.table.currentRow()
        return self.table.item(row, 0).data(Qt.ItemDataRole.UserRole) if row >= 0 else None

    def selected_ids(self) -> list[int]:
        rows = sorted({index.row() for index in self.table.selectionModel().selectedRows()})
        return [self.table.item(row, 0).data(Qt.ItemDataRole.UserRole) for row in rows]

    def filter(self, text: str) -> None:
        filter_table(self.table, text)

    def open(self) -> None:
        pass


def request_outcome(request: Request) -> str:
    """One-line outcome of a request, from its metadata-only result."""
    result = request.result or {}
    if request.status == "pending":
        return "Waiting for you"
    if request.status == "cancelled" or result.get("outcome") == "cancelled":
        return "Cancelled"
    match request.kind:
        case "profile_input":
            fields = result.get("fields", [])
            saved = sum(f.get("status") == "saved" for f in fields)
            skipped = len(fields) - saved
            return f"Saved {saved} field{'s' if saved != 1 else ''}" + (f", {skipped} skipped" if skipped else "")
        case "generate":
            extras = [e for e in (result.get("signed") and "signed", result.get("encrypted") and "password") if e]
            return "Document generated" + (f" ({', '.join(extras)})" if extras else "")
        case "fill":
            documents = len(result.get("document_ids", []))
            text = f"{documents} document{'s' if documents != 1 else ''} generated" if documents else "Fill set saved"
            empty = len(result.get("empty", []))
            return text + (f", {empty} empty" if empty else "")
    return str(result.get("outcome", request.status))


def request_details(request: Request, template_names: dict[str, str]) -> str:
    payload = request.payload
    if payload.get("reason"):
        return payload["reason"]
    match request.kind:
        case "profile_input":
            count = len(payload.get("fields", []))
            return f"{count} field{'s' if count != 1 else ''}"
        case "generate":
            return template_names.get(payload.get("template_id", ""), payload.get("template_id", ""))
        case "fill":
            names = ", ".join(template_names.get(t, t) for t in payload.get("templates", []))
            return f"{names} · {payload.get('name', '')}"
    return ""


class ActivityPage(TablePage):
    """Everything Claude asked for, waiting requests first. Metadata only: no values are shown."""

    title = "Activity"
    symbol = "clock.arrow.circlepath"
    columns = ("Request", "Details", "Outcome", "Time")
    empty_text = "Nothing from Claude yet"

    def __init__(self, app: Blankey, parent: QWidget | None = None):
        super().__init__(app, parent)
        self.action("Open", "arrow.up.forward.app", self.open)
        self.pending = 0
        self.refresh()

    def refresh(self) -> None:
        requests = sorted(self.app.vault.list_requests(), key=lambda r: (r.status != "pending", -r.id))
        names = {t.id: t.name for t in self.app.templates.all()}
        self.pending = sum(r.status == "pending" for r in requests)
        self.set_rows(
            [
                (
                    r.id,
                    [
                        REQUEST_LABELS.get(r.kind, r.kind),
                        request_details(r, names),
                        request_outcome(r),
                        _when(r.resolved_at or r.created_at),
                    ],
                )
                for r in requests
            ]
        )

    def subtitle(self) -> str:
        return f"{self.pending} waiting" if self.pending else "Nothing waiting"

    def badge(self) -> int:
        return self.pending

    def open(self) -> None:
        request_id = self.current_id()
        if request_id is None:
            return
        request = self.app.vault.get_request(request_id)
        result = request.result or {}
        if request.status == "pending":
            open_request(self.app, request, self.window())
        elif not ensure_unlocked(self.app, self):
            return
        elif request.kind == "fill" and result.get("fill_set_id") in {f.id for f in self.app.vault.list_fill_sets()}:
            info, _ = self.app.vault.get_fill_set(result["fill_set_id"])
            FillDialog(
                self.app, info.templates, info.profiles, info.name, fill_set_id=info.id, parent=self.window()
            ).exec()
        elif result.get("document_id") in {d.id for d in self.app.vault.list_documents()}:
            open_documents(self.app.vault, [result["document_id"]])
        self.refresh()


class FillSetsPage(TablePage):
    title = "Fill sets"
    symbol = "list.bullet.rectangle"
    columns = ("Name", "Templates", "Profiles", "Updated")
    empty_text = "No fill sets yet. Ask Claude to fill a template."

    def __init__(self, app: Blankey, parent: QWidget | None = None):
        super().__init__(app, parent)
        self.action("Open", "square.and.pencil", self.open)
        self.action("Generate", "play", self._generate)
        self.action("Export", "square.and.arrow.up", self._export_yaml)
        self.action("Export bundle", "shippingbox", self._export_bundle)
        self.action("Import", "square.and.arrow.down", self._import)
        self.action("Delete", "trash", self._delete, QKeySequence("Ctrl+Backspace"))
        self.refresh()

    def refresh(self) -> None:
        names = {p.id: p.name for p in self.app.vault.list_profiles()}
        self.set_rows(
            [
                (
                    info.id,
                    [
                        info.name,
                        ", ".join(info.templates),
                        ", ".join(names.get(pid, f"#{pid}") for pid in info.profiles.values()),
                        _when(info.updated_at),
                    ],
                )
                for info in self.app.vault.list_fill_sets()
            ]
        )

    def subtitle(self) -> str:
        return f"{self.table.rowCount()} sets"

    def _selected(self):
        fill_set_id = self.current_id()
        if fill_set_id is None or not ensure_unlocked(self.app, self):
            return None
        return next(f for f in self.app.vault.list_fill_sets() if f.id == fill_set_id)

    def open(self) -> None:
        if info := self._selected():
            FillDialog(
                self.app, info.templates, info.profiles, info.name, fill_set_id=info.id, parent=self.window()
            ).exec()
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
        if info and confirm(self.window(), f'Delete the fill set "{info.name}"?', "Generated documents are kept."):
            self.app.vault.delete_fill_set(info.id)
            self.refresh()


class DocumentsPage(TablePage):
    title = "Documents"
    symbol = "doc.text"
    columns = ("Title", "Template", "Kind", "Protection", "Created")
    empty_text = "No documents yet"

    def __init__(self, app: Blankey, parent: QWidget | None = None):
        super().__init__(app, parent)
        self.table.setSelectionMode(QTableWidget.SelectionMode.ExtendedSelection)
        self.action("Open", "arrow.up.forward.app", self.open)
        self.action("Export", "square.and.arrow.up", self._export)
        self.action("Delete", "trash", self._delete, QKeySequence("Ctrl+Backspace"))
        self.refresh()

    def refresh(self) -> None:
        rows = []
        for doc in self.app.vault.list_documents():
            flags = (doc.signed and f"signed ({doc.signed})", doc.encrypted and "password")
            protection = ", ".join(f for f in flags if f)
            kind = "PDF" if doc.filename.endswith(".pdf") else "Word"
            rows.append((doc.id, [doc.title, doc.template_id, kind, protection or "-", _when(doc.created_at)]))
        self.set_rows(rows)

    def subtitle(self) -> str:
        return "Encrypted in the vault"

    def _selected_all(self):
        ids = set(self.selected_ids())
        return [d for d in self.app.vault.list_documents() if d.id in ids]

    def open(self) -> None:
        docs = self._selected_all()
        if docs and ensure_unlocked(self.app, self):
            open_documents(self.app.vault, [d.id for d in docs])

    def _export(self) -> None:
        doc_id = self.current_id()
        if doc_id is None or not ensure_unlocked(self.app, self):
            return
        doc = next(d for d in self.app.vault.list_documents() if d.id == doc_id)
        target, _ = QFileDialog.getSaveFileName(self, "Export", str(Path.home() / doc.filename))
        if target:
            Path(target).write_bytes(self.app.vault.load_document(doc.id))
            self.app.vault.audit("user", "export_document", str(doc.id))

    def _delete(self) -> None:
        docs = self._selected_all()
        if not docs:
            return
        text = f'Delete "{docs[0].title}"?' if len(docs) == 1 else f"Delete {len(docs)} documents?"
        informative = "They are removed from the vault. Copies you exported stay where you saved them."
        if confirm(self.window(), text, informative):
            for doc in docs:
                self.app.vault.delete_document(doc.id)
            self.refresh()


class TemplatePreview(QWidget):
    """The selected template, read-only: a docx as a white page with its blanks as chips, PDFs as page images."""

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.setAutoFillBackground(True)
        self.heading = heading_label("")
        self.info = secondary_label()
        self.browser = QTextBrowser()
        self.browser.setFrameShape(QFrame.Shape.NoFrame)
        self.browser.setOpenLinks(False)
        paper = self.browser.palette()
        paper.setColor(QPalette.ColorRole.Base, QColor("white"))
        paper.setColor(QPalette.ColorRole.Text, QColor("#1d1d1f"))
        self.browser.setPalette(paper)
        self.browser.document().setDocumentMargin(56)
        self.browser.document().setDefaultFont(QFont("Times New Roman", 12))
        self.browser.document().setDefaultStyleSheet(
            ".blank { background-color: #dcebff; color: #0b4fa8; font-weight: 600; }"
            "p { margin-top: 0; margin-bottom: 8px; }"
        )
        self.pages = PagePreview([])
        self.pages.setFrameShape(QFrame.Shape.NoFrame)
        self.message = secondary_label("Select a template to see it", smaller=False)
        self.message.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.stack = QStackedLayout()
        for widget in (self.message, self.browser, self.pages):
            self.stack.addWidget(widget)
        backdrop = QWidget()
        backdrop.setLayout(self.stack)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(24, 16, 24, 16)
        layout.setSpacing(4)
        layout.addWidget(self.heading)
        layout.addWidget(self.info)
        layout.addSpacing(10)
        layout.addWidget(backdrop, 1)

    def clear(self) -> None:
        self.heading.setText("")
        self.info.setText("")
        self.stack.setCurrentWidget(self.message)

    def show_template(self, template: Template, examples: list[dict], used_by: list[str]) -> None:
        blanks = docx.variables(template.source_path) if template.kind == TemplateKind.DOCX else list(template.fields)
        parts = [f"{len(blanks)} blanks", f"roles: {', '.join(template.roles) or '-'}"]
        if used_by:
            parts.append(f"used by: {', '.join(used_by)}")
        self.heading.setText(template.name)
        self.info.setText("  ·  ".join(parts))
        if template.kind == TemplateKind.DOCX:
            self.browser.setHtml(docx.to_html(template.source_path, template.labels))
            self.stack.setCurrentWidget(self.browser)
            return
        try:
            pngs = self._page_images(template, examples)
        except Exception as exc:
            self.message.setText(f"No preview: {type(exc).__name__}: {exc}")
            self.stack.setCurrentWidget(self.message)
            return
        pages = PagePreview(pngs)
        pages.setFrameShape(QFrame.Shape.NoFrame)
        self.stack.replaceWidget(self.pages, pages)
        self.pages.deleteLater()
        self.pages = pages
        self.stack.setCurrentWidget(self.pages)

    @staticmethod
    def _page_images(template: Template, examples: list[dict]) -> list[bytes]:
        if template.kind == TemplateKind.PDF_FORM:
            fields = pdf_form.inspect_form(template.source)
            return [
                preview.annotate_fields(template.source, fields, page)
                for page in range(1, preview.page_count(template.source) + 1)
            ]
        rendered = engine.render(template, example_context(examples[0] if examples else {}))
        return preview.render_pages(rendered.content) if rendered.is_pdf else []


class TemplatesPage(TablePage):
    title = "Templates"
    symbol = "doc.on.doc"
    columns = ("Name", "Kind", "Blanks")
    empty_text = "No templates yet. Ask Claude to make one from a document."

    def __init__(self, app: Blankey, parent: QWidget | None = None):
        super().__init__(app, parent)
        self._ids: list[str] = []
        self.preview = TemplatePreview()
        self.page_layout.removeWidget(self.list_area)
        splitter = QSplitter()
        splitter.addWidget(self.list_area)
        splitter.addWidget(self.preview)
        splitter.setStretchFactor(1, 1)
        splitter.setSizes([380, 900])
        splitter.setChildrenCollapsible(False)
        self.page_layout.addWidget(splitter)
        self.table.setSelectionMode(QTableWidget.SelectionMode.SingleSelection)
        self.table.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.table.itemSelectionChanged.connect(self._show_selected)
        self.generate_action = self.action("Generate", "play", self.open)
        self.action("Show in Finder", "folder", self._open_folder)
        self.action("Delete", "trash", self._delete)
        self.refresh()

    def refresh(self) -> None:
        index = self.current_id()
        current = self._ids[index] if index is not None and index < len(self._ids) else None
        templates = self.app.templates.all()
        self._ids = [t.id for t in templates]
        rows = []
        for i, t in enumerate(templates):
            blanks = len(docx.variables(t.source_path)) if t.kind == TemplateKind.DOCX else len(t.fields)
            rows.append((i, [t.name, "Word" if t.kind == TemplateKind.DOCX else str(t.kind), str(blanks)]))
        self.set_rows(rows)
        header = self.table.horizontalHeader()
        header.setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        for col, width in ((1, 80), (2, 70)):
            header.setSectionResizeMode(col, QHeaderView.ResizeMode.Fixed)
            self.table.setColumnWidth(col, width)
        if rows:
            self.table.selectRow(self._ids.index(current) if current in self._ids else 0)
        self._show_selected()

    def subtitle(self) -> str:
        return f"{self.table.rowCount()} templates"

    def _current(self) -> Template | None:
        index = self.current_id()
        if index is None or index >= len(self._ids):
            return None
        return self.app.templates.get(self._ids[index])

    def _used_by(self, template_id: str) -> list[str]:
        return [f.name for f in self.app.vault.list_fill_sets() if template_id in f.templates]

    def _show_selected(self) -> None:
        template = self._current()
        self.generate_action.setVisible(template is not None and template.kind != TemplateKind.DOCX)
        if template is None:
            self.preview.clear()
            return
        examples = [
            self.app.templates.get_example(template.id, name) for name in self.app.templates.examples(template.id)
        ]
        self.preview.show_template(template, examples, self._used_by(template.id))

    def _open_folder(self) -> None:
        template = self._current()
        path = template.directory if template else self.app.templates.root
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(path)))

    def _delete(self) -> None:
        template = self._current()
        if template is None:
            return
        used_by = self._used_by(template.id)
        informative = "Its folder and examples are removed. Generated documents are kept."
        if used_by:
            informative += f" Fill sets using it will no longer open: {', '.join(used_by)}."
        if confirm(self.window(), f'Delete the template "{template.name}"?', informative):
            self.app.templates.delete(template.id)
            self.app.vault.audit("user", "delete_template", template.id)
            self.refresh()

    def open(self) -> None:
        """Generate a PDF template with the signing / encryption options (docx is filled through fill sets)."""
        template = self._current()
        if template is None or template.kind == TemplateKind.DOCX or not ensure_unlocked(self.app, self):
            return
        dialog = QDialog(self.window())
        dialog.setWindowTitle("Profiles")
        dialog.setWindowModality(Qt.WindowModality.WindowModal)
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
            GenerateDialog(self.app, template.id, profiles, parent=self.window()).exec()
