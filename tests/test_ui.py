import pytest
from PySide6.QtCore import QItemSelectionModel, Qt
from PySide6.QtWidgets import QCheckBox, QMessageBox

from blankey.core import RequestKind
from blankey.templates import TemplateKind
from blankey.ui import pages
from blankey.ui.main_window import MainWindow
from blankey.ui.profiles import ProfilesPage
from blankey.ui.requests import GenerateDialog, ProfileInputDialog
from blankey.vault import FieldInput


@pytest.fixture(autouse=True)
def no_modal_boxes(monkeypatch):
    shown = []
    for name in ("warning", "critical", "information"):
        monkeypatch.setattr(QMessageBox, name, lambda *args, **kw: shown.append(args))
    return shown


@pytest.fixture
def template(app, form_pdf, tmp_path):
    source = tmp_path / "form.pdf"
    source.write_bytes(form_pdf)
    return app.templates.save(
        "t", "T", TemplateKind.PDF_FORM, {"applicant": "person"}, {"name": "{{ applicant.name }}"}, source_file=source
    )


def test_profile_input_creates_profile_and_reports_lengths(qtbot, app, no_modal_boxes):
    request_id = app.create_request(
        RequestKind.PROFILE_INPUT,
        {
            "profile_id": None,
            "new_profile": {"name": "Мария", "kind": "person"},
            "fields": [
                {"key": "egn", "label": "ЕГН", "type": "egn", "required": True},
                {"key": "married", "label": "Семейна", "type": "bool"},
                {"key": "phone", "label": "Телефон", "type": "phone"},
            ],
            "reason": "test",
        },
    )
    dialog = ProfileInputDialog(app, app.vault.get_request(request_id))
    qtbot.addWidget(dialog)
    widgets = [w for _, _, w in dialog.inputs]
    widgets[0].setText("7501020019")  # bad checksum
    dialog._save()
    assert app.vault.get_request(request_id).status == "pending"
    assert "Invalid EGN" in no_modal_boxes[0][2]

    widgets[0].setText("7501020018")
    assert isinstance(widgets[1], QCheckBox)
    widgets[1].setChecked(True)
    dialog._save()
    request = app.vault.get_request(request_id)
    assert request.status == "done"
    assert request.result["fields"] == [
        {"key": "egn", "length": 10, "status": "saved"},
        {"key": "married", "length": 3, "status": "saved"},
        {"key": "phone", "length": 0, "status": "skipped"},
    ]
    profile = app.vault.get_profile(request.result["profile_id"])
    assert profile.name == "Мария"


def test_profile_input_cancel(qtbot, app):
    pid = app.vault.create_profile("p")
    request_id = app.create_request(
        RequestKind.PROFILE_INPUT, {"profile_id": pid, "fields": [{"key": "x"}], "reason": ""}
    )
    dialog = ProfileInputDialog(app, app.vault.get_request(request_id))
    qtbot.addWidget(dialog)
    dialog.reject()
    assert app.vault.get_request(request_id).result == {"outcome": "cancelled"}


def test_generate_dialog_resolves_request(qtbot, app, template):
    pid = app.vault.create_profile("Иван", "person")
    app.vault.set_values(pid, [FieldInput("name", "Иван Иванов")])
    payload = {"template_id": "t", "profiles": {"applicant": pid}, "sign": "self-signed", "encrypt": True}
    request_id = app.create_request(RequestKind.GENERATE, payload)
    dialog = GenerateDialog(app, "t", {"applicant": pid}, payload, app.vault.get_request(request_id))
    qtbot.addWidget(dialog)
    assert dialog.encrypt.isChecked() and dialog.sign.currentData() == "self-signed"
    dialog.password.setText("secret")
    dialog.password_repeat.setText("secret")
    dialog._generate()
    result = app.vault.get_request(request_id).result
    assert result["outcome"] == "generated"
    assert (result["signed"], result["encrypted"], result["pages"]) == ("self-signed", True, 1)


def test_profiles_page_round_trip(qtbot, app):
    pid = app.vault.create_profile("Иван", "person")
    app.vault.set_values(pid, [FieldInput("city", "София", "Град")])
    window = ProfilesPage(app)
    qtbot.addWidget(window)
    assert window.activate()
    assert window.table.item(0, 3).text() == "София"
    window.table.item(0, 3).setText("Пловдив")
    window._save()
    assert app.vault.get_values(pid)["city"][1] == "Пловдив"


def test_documents_page_deletes_all_selected(qtbot, app, monkeypatch):
    ids = [app.vault.store_document("t", f"Doc {i}", {}, 0, "", False, f"d{i}.docx", b"x") for i in range(3)]
    window = pages.DocumentsPage(app)
    qtbot.addWidget(window)
    selection = window.table.selectionModel()
    for row in (0, 2):
        selection.select(
            window.table.model().index(row, 0),
            QItemSelectionModel.SelectionFlag.Select | QItemSelectionModel.SelectionFlag.Rows,
        )
    asked = []
    monkeypatch.setattr(pages, "confirm", lambda parent, text, *args: asked.append(text) or True)
    window._delete()
    assert asked == ["Delete 2 documents?"]
    assert [d.id for d in app.vault.list_documents()] == [ids[1]]
    assert window.table.rowCount() == 1


def test_main_window_switches_pages_and_filters(qtbot, app):
    app.vault.store_document("t", "Договор", {}, 0, "", False, "a.docx", b"x")
    app.vault.store_document("t", "Решение", {}, 0, "", False, "b.docx", b"x")
    window = MainWindow(app, on_lock=lambda: None, on_quit=lambda: None)
    qtbot.addWidget(window)
    assert isinstance(window.current_page(), pages.FillSetsPage)
    window.show_page(pages.DocumentsPage)
    page = window.current_page()
    assert isinstance(page, pages.DocumentsPage)
    assert window.title_label.text() == "Documents"
    assert [a.text() for a in window.toolbar.actions() if a in page.toolbar_actions] == ["Open", "Export", "Delete"]
    window.search.setText("решение")
    visible = [page.table.item(r, 0).text() for r in range(page.table.rowCount()) if not page.table.isRowHidden(r)]
    assert visible == ["Решение"]


def test_main_window_badges_pending_requests(qtbot, app):
    app.create_request(RequestKind.PROFILE_INPUT, {"profile_id": None, "fields": [], "reason": "need data"})
    window = MainWindow(app, on_lock=lambda: None, on_quit=lambda: None)
    qtbot.addWidget(window)
    window.show_page(pages.RequestsPage)
    assert window.subtitle_label.text() == "1 waiting"
    item = next(
        window.sidebar.item(r) for r in range(window.sidebar.count()) if window.sidebar.item(r).text() == "Requests"
    )
    assert item.data(Qt.ItemDataRole.UserRole + 1) == 1
