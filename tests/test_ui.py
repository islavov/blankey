import pytest
from PySide6.QtWidgets import QCheckBox, QMessageBox

from blanka.core import RequestKind
from blanka.templates import TemplateKind
from blanka.ui.profiles import ProfilesWindow
from blanka.ui.requests import GenerateDialog, ProfileInputDialog
from blanka.vault import FieldInput


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


def test_profiles_window_round_trip(qtbot, app):
    pid = app.vault.create_profile("Иван", "person")
    app.vault.set_values(pid, [FieldInput("city", "София", "Град")])
    window = ProfilesWindow(app)
    qtbot.addWidget(window)
    assert window.table.item(0, 3).text() == "София"
    window.table.item(0, 3).setText("Пловдив")
    window._save()
    assert app.vault.get_values(pid)["city"][1] == "Пловдив"
