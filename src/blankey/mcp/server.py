"""MCP tools. Nothing here may return a vault value or real-data document bytes."""

import asyncio
import functools
import inspect
import tempfile
import time
from dataclasses import asdict
from pathlib import Path
from typing import Any

import uvicorn
from mcp.server.mcpserver import Image, MCPServer
from mcp.server.mcpserver.exceptions import ToolError
from starlette.responses import PlainTextResponse

from blankey.config import HEALTH_PATH, HEALTH_TEXT
from blankey.core import Blankey, RequestKind
from blankey.render import pdf_form, preview
from blankey.templates import TemplateKind, docx, engine, fill
from blankey.templates.bindings import example_context
from blankey.templates.store import ID_RE
from blankey.vault import FieldType, VaultLocked

INSTRUCTIONS = """\
Blankey keeps personal data (PII) encrypted in a local vault and renders document templates.
You never see real values. You see profiles, their field keys, labels, types and value lengths.

Workflow:
1. inspect_pdf_form on a blank PDF form (or write a Typst template) to learn its fields.
2. list_profiles / describe_profile to see which data keys exist.
3. save_template with Jinja bindings per field, e.g. "{{ applicant.address.city }}",
   "{{ 'X' if applicant.citizenship == 'българско' }}", "{{ loan.amount | money }}".
   Checkboxes: any truthy value checks them. Radio groups: return the state name, e.g. "/Choice2".
4. save_example + render_example to fill the template with invented example data and look at the result.
5. check_bindings against real profiles: reports missing/empty keys, overflow (fit_ratio > 1) and
   missing glyphs, without revealing values.
6. request_profile_input to ask the user to type missing data into the app, then wait_request.
7. request_fill to have the user fill the template into a named fill set and generate the real document.
   Documents are always generated from a fill set.
The user exports real documents from the app; you only get their metadata.

Filled .docx documents (contracts, decisions...):
1. inspect_docx to read the paragraphs and their runs.
2. save_docx_template with replacements [{find, var, occurrence?}] that turn the variable spans
   (names, ids, addresses, amounts, dates, dot blanks like "......") into {{ var }} tags. Replace only the
   value, not its label ("ЕГН ......" -> find the dots with occurrence). A span must sit inside one run.
   Use the same var names across related documents so one fill covers them all.
   fields gives a default per var: "{{ manager.egn }}", "{{ manager.name | upper }}", "{{ today() }}".
3. request_fill for one or more templates: the user sees every blank with its context and picks the source
   (vault path, your literal, or the template default). Pass non-personal data you know (amounts, rates,
   terms, dates, city) in values, and vault paths in paths. Never put personal data in values: ask for it
   with request_profile_input instead. wait_request returns the fill set id and document ids.
4. list_fill_sets shows saved fill sets (sources only); request_fill with fill_set_id reopens one.

Flat PDFs (no form fields, e.g. exports or scans):
1. inspect_pdf_layout: pages with a coordinate grid (PDF points, origin bottom-left) and the printed text lines
   with their boxes.
2. save_pdf_overlay_template with boxes [{name, page, rect: [x0, y0, x1, y1], type: text|check, max_len?, comb?}]
   placed where values go (right of / below their printed labels), plus fields (name -> Jinja expression) and labels.
   It returns the pages with the boxes outlined: check them and call again with corrected boxes (source_path can
   be omitted then; the original flat PDF is kept). A check box gets "X" when its expression is truthy text.
3. save_example + render_example, then request_fill as for any PDF form.
"""

TOOL_TIMEOUT_MAX = 1800
EXPECTED_ERRORS = (KeyError, ValueError, FileNotFoundError, VaultLocked)


def _expected_errors_as_tool_errors(fn):
    """Report bad input to the client instead of an opaque crash."""
    if inspect.iscoroutinefunction(fn):

        @functools.wraps(fn)
        async def async_wrapper(*args, **kwargs):
            try:
                return await fn(*args, **kwargs)
            except EXPECTED_ERRORS as exc:
                raise ToolError(str(exc)) from exc

        return async_wrapper

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        try:
            return fn(*args, **kwargs)
        except EXPECTED_ERRORS as exc:
            raise ToolError(str(exc)) from exc

    return wrapper


def build_server(app: Blankey) -> MCPServer:
    mcp = MCPServer(name="blankey", instructions=INSTRUCTIONS)

    def tool(**options):
        def register(fn):
            return mcp.tool(**options)(_expected_errors_as_tool_errors(fn))

        return register

    def audit(action: str, target: str = "") -> None:
        app.vault.audit("mcp", action, target)

    @tool()
    def vault_status() -> dict[str, Any]:
        """Whether the vault exists and is unlocked. Real-value checks need it unlocked."""
        return {"initialized": app.vault.initialized, "unlocked": app.vault.unlocked}

    @tool()
    def list_profiles() -> list[dict[str, Any]]:
        """List profiles (people, loans, companies...) stored in the vault."""
        audit("list_profiles")
        return [asdict(p) for p in app.vault.list_profiles()]

    @tool()
    def describe_profile(profile_id: int) -> dict[str, Any]:
        """Field keys of a profile with label, type and value length in characters (0 = empty). No values."""
        audit("describe_profile", str(profile_id))
        profile = app.vault.get_profile(profile_id)
        return asdict(profile) | {
            "fields": [
                {"key": f.key, "label": f.label, "type": str(f.type), "length": f.length}
                for f in app.vault.describe(profile_id)
            ]
        }

    @tool()
    def list_templates() -> list[dict[str, Any]]:
        """List saved templates."""
        return [
            {"id": t.id, "name": t.name, "kind": str(t.kind), "roles": t.roles, "fields": len(t.fields)}
            for t in app.templates.all()
        ]

    @tool()
    def get_template(template_id: str) -> dict[str, Any]:
        """Full template manifest (roles, field bindings) and its example names."""
        template = app.templates.get(template_id)
        return engine.describe_template(template) | {"examples": app.templates.examples(template_id)}

    @tool(structured_output=False)
    def inspect_pdf_form(path: str, annotate_pages: list[int] | None = None) -> list[Any]:
        """List the AcroForm fields of a local PDF (name, type, page, rects in PDF points, comb length,
        button states). annotate_pages renders those 1-based pages with every field outlined and named."""
        pdf = Path(path).expanduser().read_bytes()
        fields = pdf_form.inspect_form(pdf)
        out: list[Any] = [engine.dump([asdict(f) for f in fields])]
        for page in annotate_pages or []:
            out.append(Image(data=preview.annotate_fields(pdf, fields, page), format="png"))
        return out

    @tool(structured_output=False)
    def inspect_pdf_layout(path: str, pages: list[int] | None = None, dpi: int = 80) -> list[Any]:
        """Read a local (flat or scanned) PDF to place form boxes: per page its size in points, printed text lines
        with [x0, y0, x1, y1] boxes (PDF points, origin bottom-left) and existing form fields, plus each page as an
        image with a coordinate grid (lines every 50 pt, labels every 100 pt). Default pages: all."""
        pdf = Path(path).expanduser().read_bytes()
        sizes = preview.page_sizes(pdf)
        selected = pages or list(range(1, len(sizes) + 1))
        for page in selected:
            if not 1 <= page <= len(sizes):
                raise ValueError(f"page must be 1..{len(sizes)}")
        info = {
            "pages": [
                {
                    "page": page,
                    "width": round(sizes[page - 1][0], 1),
                    "height": round(sizes[page - 1][1], 1),
                    "lines": preview.layout_lines(pdf, page),
                }
                for page in selected
            ],
            "form_fields": [asdict(f) for f in pdf_form.inspect_form(pdf)],
        }
        return [engine.dump(info)] + [Image(data=preview.grid_page(pdf, page, dpi), format="png") for page in selected]

    @tool(structured_output=False)
    def save_pdf_overlay_template(
        template_id: str,
        name: str,
        boxes: list[dict[str, Any]],
        roles: dict[str, str],
        fields: dict[str, str],
        source_path: str | None = None,
        labels: dict[str, str] | None = None,
        description: str = "",
    ) -> list[Any]:
        """Make a PDF form template from a flat PDF: each box {name, page, rect: [x0, y0, x1, y1], type: text|check,
        max_len?, comb?} (PDF points, origin bottom-left) becomes a form field named `name`. fields maps those names
        to Jinja expressions over roles; labels maps them to human labels for the fill form. Omit source_path to
        rebuild from the flat PDF saved earlier (replaces all boxes). Returns the template and its pages with the
        boxes outlined, to check placement."""
        if not ID_RE.match(template_id):
            raise ValueError("Template id must be lowercase letters, digits, '-' or '_'")
        if source_path:
            flat = Path(source_path).expanduser().read_bytes()
        else:
            saved = app.templates.root / template_id / "flat.pdf"
            if not saved.exists():
                raise ValueError("Pass source_path: no flat PDF saved for this template yet")
            flat = saved.read_bytes()
        form = pdf_form.add_fields(flat, boxes)
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "form.pdf"
            target.write_bytes(form)
            template = app.templates.save(
                template_id,
                name,
                TemplateKind.PDF_FORM,
                roles,
                fields,
                description,
                source_file=target,
                labels=labels,
                boxes=boxes,
            )
        (template.directory / "flat.pdf").write_bytes(flat)
        audit("save_template", template_id)
        names = [b["name"] for b in boxes]
        warnings = [f"Box without binding: {n}" for n in names if n not in fields]
        known = set(names) | {f.name for f in pdf_form.inspect_form(flat)}
        warnings += [f"Binding without field: {f}" for f in fields if f not in known]
        summary = engine.describe_template(template) | {"warnings": warnings}
        all_fields = pdf_form.inspect_form(form)
        images = [
            Image(data=preview.annotate_fields(form, all_fields, page), format="png")
            for page in sorted({b["page"] for b in boxes})
        ]
        return [engine.dump(summary), *images]

    @tool()
    def save_template(
        template_id: str,
        name: str,
        kind: TemplateKind,
        fields: dict[str, str],
        roles: dict[str, str],
        description: str = "",
        source_path: str | None = None,
        source_text: str | None = None,
    ) -> dict[str, Any]:
        """Create or replace a template. kind: pdf_form (source_path = blank PDF form), typst (source_text,
        reads `json(bytes(sys.inputs.data))` for the field values) or docx (source_path, docxtpl Jinja tags).
        fields maps PDF field names / template variables to Jinja expressions over the roles.
        roles maps role names (e.g. applicant, loan) to a short description. Omit the source to keep
        the existing one."""
        template = app.templates.save(
            template_id,
            name,
            kind,
            roles,
            fields,
            description,
            source_file=Path(source_path).expanduser() if source_path else None,
            source_text=source_text,
        )
        audit("save_template", template_id)
        return engine.describe_template(template)

    @tool()
    def inspect_docx(path: str) -> list[dict[str, Any]]:
        """Paragraphs of a local .docx (body and table cells) with their run texts and existing {{ }} tags.
        Use the run texts to pick spans for save_docx_template: a span cannot cross a run boundary."""
        return [asdict(p) for p in docx.inspect(Path(path).expanduser())]

    @tool()
    def save_docx_template(
        template_id: str,
        name: str,
        source_path: str,
        replacements: list[dict[str, Any]],
        roles: dict[str, str],
        fields: dict[str, str],
        labels: dict[str, str] | None = None,
        description: str = "",
    ) -> dict[str, Any]:
        """Create a docx template from a filled .docx: each replacement {find, var, occurrence?} replaces the
        text with {{ var }} (occurrence is 1-based across the document; omit it to replace all). Replacements
        apply in order, so put longer spans first (".......2026" before "......"). fields maps
        var -> default Jinja expression over roles; labels maps var -> human label shown in the fill form."""
        if not ID_RE.match(template_id):
            raise ValueError("Template id must be lowercase letters, digits, '-' or '_'")
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "template.docx"
            report = docx.tokenize(Path(source_path).expanduser(), target, replacements)
            template = app.templates.save(
                template_id, name, TemplateKind.DOCX, roles, fields, description, source_file=target, labels=labels
            )
        audit("save_template", template_id)
        return engine.describe_template(template) | {"replaced": report}

    @tool()
    def save_example(template_id: str, name: str, data: dict[str, Any]) -> str:
        """Store an invented example dataset keyed by role, e.g. {"applicant": {"name": "Иван Иванов"}}."""
        app.templates.save_example(template_id, name, data)
        return f"Saved example {name!r}"

    @tool(structured_output=False)
    def render_example(
        template_id: str,
        example: str | dict[str, Any],
        pages: list[int] | None = None,
        dpi: int = 80,
    ) -> list[Any]:
        """Render a template with example data (a saved example name or an inline dict keyed by role).
        Returns warnings, the preview PDF path and PNGs of the selected pages (default: all)."""
        template = app.templates.get(template_id)
        data = app.templates.get_example(template_id, example) if isinstance(example, str) else example
        rendered = engine.render(template, example_context(data))
        out_path = app.config.previews_dir / f"{template_id}-example.{rendered.extension}"
        out_path.write_bytes(rendered.content)
        summary = {"path": str(out_path), "warnings": rendered.warnings}
        out: list[Any] = [engine.dump(summary)]
        if rendered.is_pdf:
            out += [Image(data=png, format="png") for png in preview.render_pages(rendered.content, pages, dpi)]
        return out

    @tool()
    def check_bindings(template_id: str, profiles: dict[str, int]) -> dict[str, Any]:
        """Check a template against real profiles ({role: profile_id}) without revealing values:
        missing/empty keys always; fit_ratio (> 1 = text too wide) and missing glyphs when the vault is unlocked."""
        audit("check_bindings", f"template={template_id} profiles={profiles}")
        reports = app.check_bindings(template_id, profiles)
        problems = [asdict(r) for r in reports if r.status != "ok"]
        return {
            "vault_unlocked": app.vault.unlocked,
            "checked": len(reports),
            "ok": len(reports) - len(problems),
            "problems": problems,
        }

    @tool()
    def request_profile_input(
        fields: list[dict[str, Any]],
        reason: str,
        profile_id: int | None = None,
        new_profile_name: str | None = None,
        new_profile_kind: str = "",
    ) -> dict[str, Any]:
        """Ask the user to type data into the app. fields: [{key, label, type, required, hint}] with type one of
        text, number, date, bool, egn, iban, email, phone. Give profile_id to extend a profile, or
        new_profile_name to create one. Returns a request id; call wait_request for the outcome."""
        if profile_id is None and not new_profile_name:
            raise ValueError("Pass profile_id or new_profile_name")
        if profile_id is not None:
            app.vault.get_profile(profile_id)
        for item in fields:
            if "key" not in item:
                raise ValueError("Every field needs a key")
            FieldType(item.get("type", "text"))
        payload = {
            "profile_id": profile_id,
            "new_profile": {"name": new_profile_name, "kind": new_profile_kind} if profile_id is None else None,
            "fields": fields,
            "reason": reason,
        }
        request_id = app.create_request(RequestKind.PROFILE_INPUT, payload)
        return {"request_id": request_id, "status": "pending"}

    @tool()
    def request_fill(
        templates: list[str],
        profiles: dict[str, int],
        name: str,
        values: dict[str, str] | None = None,
        paths: dict[str, str] | None = None,
        fill_set_id: int | None = None,
        reason: str = "",
    ) -> dict[str, Any]:
        """Open the fill form for one or more templates. profiles maps every role to a profile id. values are
        non-personal literals per var (e.g. {"loan_amount": "34 000"}); paths pick vault fields per var
        (e.g. {"borrower_egn": "manager.egn"}). Other vars start from the template default, or from the saved
        fill set when fill_set_id is given. name names the fill set. Returns a request id; call wait_request."""
        values, paths = values or {}, paths or {}
        loaded = app.fill_templates(templates)
        if not loaded or not name.strip():
            raise ValueError("Pass at least one template and a name")
        missing_roles = set(fill.roles(loaded)) - set(profiles)
        if missing_roles:
            raise ValueError(f"Missing profiles for roles: {', '.join(sorted(missing_roles))}")
        fields = app.profile_fields(profiles)
        known = fill.variables(loaded)
        for var in [*values, *paths]:
            if var not in known:
                raise ValueError(f"Unknown variable {var!r}; known: {', '.join(known)}")
        for var, path in paths.items():
            role, _, key = path.partition(".")
            if role not in fields or not any(i.key == key or i.key.startswith(key + ".") for i in fields[role]):
                raise ValueError(f"{var}: no vault field {path!r}")
        if fill_set_id is not None and fill_set_id not in {f.id for f in app.vault.list_fill_sets()}:
            raise KeyError(f"Unknown fill set {fill_set_id}")
        payload = {
            "templates": templates,
            "profiles": profiles,
            "name": name.strip(),
            "values": values,
            "paths": paths,
            "fill_set_id": fill_set_id,
            "reason": reason or f"Fill {', '.join(t.name for t in loaded)}",
        }
        request_id = app.create_request(RequestKind.FILL, payload)
        return {"request_id": request_id, "status": "pending"}

    @tool()
    def list_fill_sets() -> list[dict[str, Any]]:
        """Saved fill sets with the source kind per var (path:role.key, value or default). No values."""
        if not app.vault.unlocked:
            return [asdict(f) for f in app.vault.list_fill_sets()]
        out = []
        for info in app.vault.list_fill_sets():
            _, bindings = app.vault.get_fill_set(info.id)
            out.append(asdict(info) | {"vars": {v: fill.describe_binding(b) for v, b in bindings.items()}})
        return out

    @tool()
    async def wait_request(request_id: int, timeout_s: int = 300) -> dict[str, Any]:
        """Wait until the user resolves a request (or the timeout passes). Returns status and metadata only."""
        deadline = time.monotonic() + min(timeout_s, TOOL_TIMEOUT_MAX)
        while True:
            request = app.vault.get_request(request_id)
            if request.status != "pending" or time.monotonic() >= deadline:
                return _request_view(request)
            await asyncio.sleep(0.5)

    @tool()
    def get_request(request_id: int) -> dict[str, Any]:
        """Current status of a request."""
        return _request_view(app.vault.get_request(request_id))

    @tool()
    def list_requests(status: str | None = None) -> list[dict[str, Any]]:
        """List requests, optionally filtered by status (pending, done, cancelled)."""
        return [_request_view(r) for r in app.vault.list_requests(status)]

    @tool()
    def list_documents() -> list[dict[str, Any]]:
        """Generated real documents (metadata only; the user exports them from the app)."""
        return [asdict(d) for d in app.vault.list_documents()]

    return mcp


def _request_view(request) -> dict[str, Any]:
    return {
        "id": request.id,
        "kind": request.kind,
        "status": request.status,
        "result": request.result,
        "created_at": request.created_at,
        "resolved_at": request.resolved_at,
    }


def create_server(app: Blankey) -> uvicorn.Server:
    # localhost only; the SDK adds Host/Origin checks against DNS rebinding. JSON responses keep the
    # stdio bridge (blankey mcp) a plain request/response proxy.
    asgi = build_server(app).streamable_http_app(json_response=True)
    asgi.add_route(HEALTH_PATH, lambda request: PlainTextResponse(HEALTH_TEXT))
    return uvicorn.Server(
        uvicorn.Config(asgi, host="127.0.0.1", port=app.config.port, log_level="warning", lifespan="on")
    )
