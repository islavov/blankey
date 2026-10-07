import asyncio
import io
import sqlite3
import zipfile
from pathlib import Path

import docx
import pytest
import yaml

from blankey.core import RequestKind
from blankey.mcp_server import build_server
from blankey.templates import TemplateKind, engine, fill
from blankey.templates import docx as docx_template
from blankey.ui.fill import FillDialog
from blankey.vault import FieldInput, FieldType

REPLACEMENTS = [  # applied in order: longer dot blanks first
    {"find": "...........2026", "var": "contract_date"},
    {"find": "„ACME HOLDINGS“ ЕООД", "var": "company_name"},
    {"find": "999000111", "var": "company_eik"},
    {"find": "John Smith", "var": "manager_name"},
    {"find": "..........", "var": "manager_egn", "occurrence": 1},
    {"find": "..........", "var": "manager_id_card", "occurrence": 1},
    {"find": "34 000", "var": "loan_amount"},
    {"find": ".........", "var": "contract_no"},
]
FIELDS = {
    "company_name": "{{ company.name }}",
    "company_eik": "{{ company.eik }}",
    "manager_name": "{{ manager.name }}",
    "manager_egn": "{{ manager.egn }}",
    "contract_date": "{{ today() }}",
}
ROLES = {"company": "lender", "manager": "borrower"}


def text_of(content: bytes) -> str:
    document = docx.Document(io.BytesIO(content))
    cells = [c.text for t in document.tables for r in t.rows for c in r.cells]
    return "\n".join([p.text for p in document.paragraphs] + cells)


@pytest.fixture
def template(app, filled_docx, tmp_path):
    target = tmp_path / "template.docx"
    docx_template.tokenize(filled_docx, target, REPLACEMENTS)
    return app.templates.save(
        "loan", "Loan", TemplateKind.DOCX, ROLES, FIELDS, source_file=target, labels={"loan_amount": "Сума"}
    )


@pytest.fixture
def profiles(app):
    company = app.vault.create_profile("Delta", "company")
    app.vault.set_values(company, [FieldInput("name", "„ACME“ ЕООД"), FieldInput("eik", "999000111")])
    manager = app.vault.create_profile("John", "person")
    app.vault.set_values(
        manager, [FieldInput("name", "Тест Тестов"), FieldInput("egn", "7501020018", type=FieldType.EGN)]
    )
    return {"company": company, "manager": manager}


def test_tokenize_and_variables(template):
    text = text_of(template.source)
    assert "{{ company_name }}, ЕИК {{ company_eik }}" in text
    assert "ЕГН {{ manager_egn }}, л.к. № {{ manager_id_card }}" in text
    assert "Договор № {{ contract_no }} / {{ contract_date }} г." in text
    assert docx_template.variables(template.source_path) == [
        "contract_no",
        "contract_date",
        "company_name",
        "company_eik",
        "manager_name",
        "manager_egn",
        "manager_id_card",
        "loan_amount",
    ]
    contexts = docx_template.blank_contexts(template.source_path)
    assert contexts["loan_amount"] == ["Заем в размер на [____] евро."]
    assert len(contexts["manager_name"]) == 2  # paragraph and table cell


def test_tokenize_errors(filled_docx, tmp_path):
    with pytest.raises(ValueError, match="spans several runs"):
        docx_template.tokenize(filled_docx, tmp_path / "x.docx", [{"find": "ЕООД, ЕИК", "var": "x"}])
    with pytest.raises(ValueError, match="not found"):
        docx_template.tokenize(filled_docx, tmp_path / "x.docx", [{"find": "nope", "var": "x"}])
    with pytest.raises(ValueError, match="no occurrence 9"):
        docx_template.tokenize(filled_docx, tmp_path / "x.docx", [{"find": "John", "var": "x", "occurrence": 9}])


def test_describe_warns_about_unbound_tags(template):
    warnings = engine.describe_template(template)["warnings"]
    assert "Tag without binding: loan_amount" in warnings


def test_fill_set_encrypted_round_trip(app):
    bindings = {"manager_egn": {"path": "manager.egn"}, "secret": {"value": "Тайна Стойност"}}
    fill_set_id = app.vault.save_fill_set("s", ["loan"], {"manager": 1}, bindings)
    raw = sqlite3.connect(app.config.db_path).execute("SELECT ciphertext FROM fill_sets").fetchone()[0]
    assert "Тайна".encode() not in raw
    info, loaded = app.vault.get_fill_set(fill_set_id)
    assert (info.name, info.templates, info.profiles, loaded) == ("s", ["loan"], {"manager": 1}, bindings)
    app.vault.save_fill_set("s2", ["loan"], {}, {}, fill_set_id)
    assert [f.name for f in app.vault.list_fill_sets()] == ["s2"]


def test_generate_fill_mixes_paths_values_and_defaults(app, template, profiles):
    bindings = {
        "manager_name": {"path": "manager.name"},
        "loan_amount": {"value": "34 000"},
        "contract_no": {"value": ""},
        "manager_id_card": {"path": "manager.egn"},
    }
    fill_set_id = app.vault.save_fill_set("delta", ["loan"], profiles, bindings)
    [doc_id] = app.generate_fill(fill_set_id)
    text = text_of(app.vault.load_document(doc_id))
    assert "„ACME“ ЕООД, ЕИК 999000111, представлявано от Тест Тестов" in text
    assert "ЕГН 7501020018, л.к. № 7501020018" in text
    assert "34 000 евро" in text
    assert "None" not in text and "{{" not in text


def test_export_import_and_bundle(app, template, profiles, tmp_path):
    fill_set_id = app.vault.save_fill_set("delta", ["loan"], profiles, {"loan_amount": {"value": "1"}})
    exported = yaml.safe_load(app.export_fill_set(fill_set_id))
    assert exported["bindings"] == {"loan_amount": {"value": "1"}}
    exported["bindings"]["loan_amount"]["value"] = "2"
    assert app.import_fill_set(yaml.safe_dump(exported, allow_unicode=True)) == fill_set_id
    assert app.vault.get_fill_set(fill_set_id)[1]["loan_amount"] == {"value": "2"}
    app.export_fill_bundle(fill_set_id, tmp_path / "b.zip")
    assert sorted(zipfile.ZipFile(tmp_path / "b.zip").namelist()) == [
        "fill.yaml",
        "loan/manifest.yaml",
        "loan/template.docx",
    ]


def test_default_binding():
    assert fill.default_binding(fill.FillVar("x", defaults={"t": "{{ manager.egn }}"}), {"manager"}) == {
        "path": "manager.egn"
    }
    assert fill.default_binding(fill.FillVar("x", defaults={"t": "{{ manager.name | upper }}"}), {"manager"}) == {
        "default": True
    }
    assert fill.default_binding(fill.FillVar("x"), {"manager"}) == {"value": ""}


def test_request_fill_validates_and_hides_values(qtbot, app, template, profiles, filled_docx, opened_urls):
    mcp = build_server(app)

    def call(tool, **arguments):
        return asyncio.run(mcp.call_tool(tool, arguments))

    assert "ACME" in call("inspect_docx", path=str(filled_docx)).model_dump_json(ensure_ascii=False)
    with pytest.raises(Exception, match="Missing profiles"):
        call("request_fill", templates=["loan"], profiles={"company": profiles["company"]}, name="x")
    with pytest.raises(Exception, match="no vault field"):
        call("request_fill", templates=["loan"], profiles=profiles, name="x", paths={"manager_egn": "manager.nope"})
    with pytest.raises(Exception, match="Unknown variable"):
        call("request_fill", templates=["loan"], profiles=profiles, name="x", values={"nope": "1"})
    call(
        "request_fill",
        templates=["loan"],
        profiles=profiles,
        name="delta",
        values={"loan_amount": "34 000"},
        paths={"manager_id_card": "manager.egn"},
    )
    request = app.vault.list_requests("pending")[0]
    assert request.kind == RequestKind.FILL

    dialog = FillDialog(
        app,
        request.payload["templates"],
        request.payload["profiles"],
        request.payload["name"],
        request.payload["values"],
        request.payload["paths"],
        request=request,
    )
    qtbot.addWidget(dialog)
    bindings = dialog.current_bindings()
    assert bindings["loan_amount"] == {"value": "34 000"}
    assert bindings["manager_id_card"] == {"path": "manager.egn"}
    assert bindings["manager_egn"] == {"path": "manager.egn"}
    assert bindings["contract_date"] == {"default": True}
    row = list(dialog.vars).index("contract_no")
    dialog._combo(row).setEditText("17")
    dialog._save(generate=True)

    result = app.vault.get_request(request.id).result
    assert result["outcome"] == "generated" and len(result["document_ids"]) == 1
    assert result["empty"] == []
    assert len(opened_urls) == 1 and opened_urls[0].endswith(".docx")
    assert "Договор № 17" in text_of(Path(opened_urls[0]).read_bytes())
    assert "Договор № 17" in text_of(app.vault.load_document(result["document_ids"][0]))
    listed = call("list_fill_sets").model_dump_json()
    assert "7501020018" not in listed and "Тест" not in listed and '"loan_amount":"value"' in listed.replace(" ", "")


def test_fill_dialog_cancel_and_save(qtbot, app, template, profiles):
    request_id = app.create_request(
        RequestKind.FILL, {"templates": ["loan"], "profiles": profiles, "name": "n", "values": {}, "paths": {}}
    )
    dialog = FillDialog(app, ["loan"], profiles, "n", request=app.vault.get_request(request_id))
    qtbot.addWidget(dialog)
    dialog._save(generate=False)
    dialog.reject()
    result = app.vault.get_request(request_id).result
    assert result["outcome"] == "saved"
    assert "loan_amount" in result["empty"]
    reopened = FillDialog(app, [], {}, "n")
    qtbot.addWidget(reopened)
    assert reopened.fill_set_id == result["fill_set_id"]
