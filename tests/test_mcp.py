import asyncio
import base64
import json

import pytest

from blankey.core import RequestKind
from blankey.mcp.server import build_server
from blankey.render import pdf_form
from blankey.vault import FieldInput, FieldType
from tests.conftest import PASSWORD

SENTINEL_NAME = "Сентинела Тайнова"
SENTINEL_EGN = "7501020018"


@pytest.fixture
def mcp(app):
    return build_server(app)


@pytest.fixture
def setup(app, form_pdf, tmp_path):
    source = tmp_path / "form.pdf"
    source.write_bytes(form_pdf)
    pid = app.vault.create_profile("Applicant", "person")
    app.vault.set_values(
        pid,
        [
            FieldInput("name", SENTINEL_NAME, "Име", FieldType.TEXT),
            FieldInput("egn", SENTINEL_EGN, "ЕГН", FieldType.EGN),
        ],
    )
    return {"pid": pid, "source": str(source)}


def call(mcp, tool, /, **arguments) -> str:
    result = asyncio.run(mcp.call_tool(tool, arguments))
    payload = result.model_dump_json()
    for content in result.content:
        if content.type == "text":
            payload += content.text
    return payload


def images(mcp, tool, /, **arguments) -> list[bytes]:
    result = asyncio.run(mcp.call_tool(tool, arguments))
    return [base64.b64decode(c.data) for c in result.content if c.type == "image"]


def assert_no_pii(text: str) -> None:
    for secret in (SENTINEL_NAME, SENTINEL_EGN, *SENTINEL_NAME.split()):
        assert secret not in text
        assert json.dumps(secret)[1:-1] not in text  # escaped unicode


def test_no_tool_leaks_vault_values(app, mcp, setup, filled_docx):
    pid = setup["pid"]
    fields = {"name": "{{ applicant.name }}", "egn": "{{ applicant.egn }}"}
    outputs = [
        call(mcp, "vault_status"),
        call(mcp, "list_profiles"),
        call(mcp, "describe_profile", profile_id=pid),
        call(mcp, "inspect_pdf_form", path=setup["source"]),
        call(
            mcp,
            "save_template",
            template_id="t",
            name="T",
            kind="pdf_form",
            fields=fields,
            roles={"applicant": "person"},
            source_path=setup["source"],
        ),
        call(mcp, "list_templates"),
        call(mcp, "get_template", template_id="t"),
        call(mcp, "save_example", template_id="t", name="ex", data={"applicant": {"name": "Пример"}}),
        call(mcp, "render_example", template_id="t", example="ex"),
        call(mcp, "check_bindings", template_id="t", profiles={"applicant": pid}),
        call(mcp, "request_fill", templates=["t"], profiles={"applicant": pid}, name="leak"),
        call(
            mcp,
            "request_profile_input",
            fields=[{"key": "email", "type": "email"}],
            reason="r",
            profile_id=pid,
        ),
        call(mcp, "list_requests"),
        call(mcp, "get_request", request_id=1),
        call(mcp, "wait_request", request_id=1, timeout_s=0),
    ]
    fill_set_id = app.vault.save_fill_set("leak", ["t"], {"applicant": pid}, {})
    [doc_id] = app.generate_fill(fill_set_id)
    app.vault.resolve_request(1, "done", {"fill_set_id": fill_set_id, "document_ids": [doc_id]})
    outputs += [call(mcp, "list_documents"), call(mcp, "get_request", request_id=1)]
    outputs += [
        call(mcp, "inspect_docx", path=str(filled_docx)),
        call(mcp, "inspect_pdf_layout", path=setup["source"]),
        call(
            mcp,
            "save_docx_template",
            template_id="d",
            name="D",
            source_path=str(filled_docx),
            replacements=[{"find": "John Smith", "var": "who"}, {"find": "34 000", "var": "amount"}],
            roles={"applicant": "person"},
            fields={"who": "{{ applicant.name }}"},
        ),
        call(mcp, "request_fill", templates=["d"], profiles={"applicant": pid}, name="f", values={"amount": "1"}),
    ]
    app.vault.save_fill_set("f", ["d"], {"applicant": pid}, {"amount": {"value": SENTINEL_NAME}})
    outputs.append(call(mcp, "list_fill_sets"))
    for output in outputs:
        assert_no_pii(output)
    assert SENTINEL_NAME not in app.config.previews_dir.joinpath("t-example.pdf").read_bytes().decode("latin-1")
    for row in app.vault._conn.execute("SELECT action, target FROM audit"):
        assert_no_pii(row["action"] + row["target"])


def test_describe_profile_reports_lengths(mcp, setup):
    data = json.loads(call(mcp, "describe_profile", profile_id=setup["pid"]).split("}{", 1)[0] + "}")
    lengths = {f["key"]: f["length"] for f in data["structured_content"]["fields"]}
    assert lengths == {"egn": 10, "name": len(SENTINEL_NAME)}


def test_inspect_annotates_pages(mcp, setup):
    [png] = images(mcp, "inspect_pdf_form", path=setup["source"], annotate_pages=[1])
    assert png.startswith(b"\x89PNG")


def test_render_example_returns_pages(mcp, setup):
    call(
        mcp,
        "save_template",
        template_id="t",
        name="T",
        kind="pdf_form",
        fields={"name": "{{ applicant.name }}"},
        roles={"applicant": "person"},
        source_path=setup["source"],
    )
    pngs = images(mcp, "render_example", template_id="t", example={"applicant": {"name": "Пример"}})
    assert len(pngs) == 1


def test_wait_request_returns_profile_input_result(app, mcp, setup):
    created = call(
        mcp,
        "request_profile_input",
        fields=[{"key": "email", "label": "Имейл", "type": "email"}],
        reason="Нужно за ДСК",
        new_profile_name="Мария",
    )
    assert '"pending"' in created
    request = app.vault.list_requests("pending")[0]
    assert request.kind == RequestKind.PROFILE_INPUT
    app.vault.resolve_request(request.id, "done", {"outcome": "saved", "fields": [{"key": "email", "length": 9}]})
    result = call(mcp, "wait_request", request_id=request.id, timeout_s=5)
    assert '"saved"' in result and '"length":9' in result.replace(" ", "")


def test_request_fill_validates_roles(mcp, setup):
    call(
        mcp,
        "save_template",
        template_id="t",
        name="T",
        kind="pdf_form",
        fields={"name": "{{ applicant.name }}"},
        roles={"applicant": "person"},
        source_path=setup["source"],
    )
    with pytest.raises(Exception, match="Missing profiles"):
        asyncio.run(mcp.call_tool("request_fill", {"templates": ["t"], "profiles": {}, "name": "n"}))


def test_vault_locked_check_still_reports_metadata(app, mcp, setup):
    call(
        mcp,
        "save_template",
        template_id="t",
        name="T",
        kind="pdf_form",
        fields={"name": "{{ applicant.name }}", "email": "{{ applicant.email }}"},
        roles={"applicant": "person"},
        source_path=setup["source"],
    )
    app.vault.lock()
    result = call(mcp, "check_bindings", template_id="t", profiles={"applicant": setup["pid"]})
    assert '"vault_unlocked":false' in result.replace(" ", "")
    assert "applicant.email" in result
    app.vault.unlock(PASSWORD)


def test_flat_pdf_becomes_a_template(app, mcp, flat_pdf, tmp_path):
    source = tmp_path / "flat.pdf"
    source.write_bytes(flat_pdf)
    layout = call(mcp, "inspect_pdf_layout", path=str(source))
    assert '"Name:"' in layout and '"height": 842' in layout
    assert len(images(mcp, "inspect_pdf_layout", path=str(source))) == 1

    boxes = [
        {"name": "name", "page": 1, "rect": [90, 785, 400, 803]},
        {"name": "married", "page": 1, "rect": [100, 706, 112, 718], "type": "check"},
    ]
    arguments = {
        "template_id": "flat",
        "name": "Flat",
        "boxes": boxes,
        "roles": {"applicant": "person"},
        "fields": {"name": "{{ applicant.name }}", "married": "{{ 'X' if applicant.married }}", "ghost": "x"},
        "source_path": str(source),
    }
    summary = call(mcp, "save_pdf_overlay_template", **arguments)
    assert "Binding without field: ghost" in summary
    assert len(images(mcp, "save_pdf_overlay_template", **arguments)) == 1
    template = app.templates.get("flat")
    assert (template.directory / "flat.pdf").read_bytes() == flat_pdf
    assert [b["name"] for b in template.boxes] == ["name", "married"]

    moved = [{"name": "name", "page": 1, "rect": [95, 785, 400, 803]}]
    call(mcp, "save_pdf_overlay_template", **(arguments | {"boxes": moved, "source_path": None}))
    fields = pdf_form.inspect_form(app.templates.get("flat").source)
    assert [(f.name, f.rects) for f in fields] == [("name", [[95.0, 785.0, 400.0, 803.0]])]  # rebuilt, not stacked

    rendered = call(mcp, "render_example", template_id="flat", example={"applicant": {"name": "Пример Примеров"}})
    assert "warnings" in rendered
    with pytest.raises(Exception, match="page must be"):
        asyncio.run(
            mcp.call_tool(
                "save_pdf_overlay_template", arguments | {"boxes": [{"name": "x", "page": 9, "rect": [0, 0, 1, 1]}]}
            )
        )


def test_bridge_starts_without_qt_or_the_server():
    import subprocess
    import sys

    code = (
        "import sys, blankey.__main__, blankey.mcp.bridge; "
        "print([m for m in ('PySide6', 'mcp', 'docxtpl', 'typst', 'uvicorn') if m in sys.modules])"
    )
    assert subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, check=True).stdout == "[]\n"
