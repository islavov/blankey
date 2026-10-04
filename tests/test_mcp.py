import asyncio
import base64
import io
import json

import pikepdf
import pytest
from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign.validation import validate_pdf_signature

from blankey.core import GenerateOptions, RequestKind
from blankey.mcp_server import build_server
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


def test_no_tool_leaks_vault_values(app, mcp, setup):
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
        call(mcp, "request_generate", template_id="t", profiles={"applicant": pid}, sign="self-signed"),
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
    doc_id = app.generate("t", {"applicant": pid}, GenerateOptions(sign="self-signed", signer_name=SENTINEL_NAME))
    app.vault.resolve_request(1, "done", {"document_id": doc_id})
    outputs += [call(mcp, "list_documents"), call(mcp, "get_request", request_id=1)]
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


def test_request_generate_validates_roles(mcp, setup):
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
        asyncio.run(mcp.call_tool("request_generate", {"template_id": "t", "profiles": {}}))


def test_generate_signed_and_encrypted(app, setup):
    app.templates.save(
        "t",
        "T",
        "pdf_form",
        {"applicant": "person"},
        {"name": "{{ applicant.name }}"},
        source_file=__import__("pathlib").Path(setup["source"]),
    )
    doc_id = app.generate(
        "t", {"applicant": setup["pid"]}, GenerateOptions(sign="self-signed", password="pdf-pass", signer_name="Me")
    )
    info = app.vault.list_documents()[0]
    assert (info.signed, info.encrypted, info.pages) == ("self-signed", True, 1)
    content = app.vault.load_document(doc_id)
    with pytest.raises(pikepdf.PasswordError):
        pikepdf.open(io.BytesIO(content))
    with pikepdf.open(io.BytesIO(content), password="pdf-pass") as pdf:
        assert len(pdf.pages) == 1
    reader = PdfFileReader(io.BytesIO(content))
    reader.decrypt("pdf-pass")
    status = validate_pdf_signature(reader.embedded_signatures[0])
    assert status.intact and status.valid  # self-signed: integrity ok, not trusted


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
