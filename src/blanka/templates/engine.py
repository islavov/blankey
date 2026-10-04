"""Render templates to documents and check bindings against profiles."""

import json
import shutil
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import typst
from docxtpl import DocxTemplate

from blanka.config import FONTS_DIR
from blanka.render import pdf_form
from blanka.templates.bindings import as_text, evaluate, referenced_paths, to_json
from blanka.templates.store import Template, TemplateKind
from blanka.vault import FieldInfo


@dataclass(slots=True)
class Rendered:
    content: bytes
    extension: str  # "pdf" or "docx"
    warnings: list[str] = field(default_factory=list)

    @property
    def is_pdf(self) -> bool:
        return self.extension == "pdf"


@dataclass(slots=True)
class BindingReport:
    field: str
    status: str  # ok | missing | empty | error | overflow | glyphs
    missing: list[str] = field(default_factory=list)
    empty: list[str] = field(default_factory=list)
    fit_ratio: float | None = None
    missing_glyphs: list[str] = field(default_factory=list)
    error: str | None = None


def resolve(template: Template, context: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    values, errors = {}, []
    for name, expression in template.fields.items():
        try:
            values[name] = evaluate(expression, context)
        except Exception as exc:
            errors.append(f"{name}: {type(exc).__name__}: {exc}")
    return values, errors


def render(template: Template, context: dict[str, Any]) -> Rendered:
    values, errors = resolve(template, context)
    match template.kind:
        case TemplateKind.PDF_FORM:
            known = {f.name for f in pdf_form.inspect_form(template.source)}
            unknown = sorted(set(values) - known)
            if unknown:
                errors.append(f"Fields not in the PDF form: {', '.join(unknown)}")
            text_values = {k: as_text(v) for k, v in values.items() if k in known}
            return Rendered(pdf_form.fill_form(template.source, text_values), "pdf", errors)
        case TemplateKind.TYPST:
            pdf = typst.compile(
                str(template.source_path),
                root=str(template.directory),
                font_paths=[str(FONTS_DIR)],
                sys_inputs={"data": to_json(values)},
            )
            return Rendered(pdf, "pdf", errors)
        case TemplateKind.DOCX:
            doc = DocxTemplate(str(template.source_path))
            doc.render(values, autoescape=True)
            with tempfile.TemporaryDirectory() as tmp:
                docx_path = Path(tmp) / "document.docx"
                doc.save(str(docx_path))
                pdf = docx_to_pdf(docx_path)
                if pdf is None:
                    errors.append("LibreOffice not found: output is .docx and cannot be previewed or signed")
                    return Rendered(docx_path.read_bytes(), "docx", errors)
                return Rendered(pdf, "pdf", errors)
    raise ValueError(f"Unsupported template kind {template.kind}")


def docx_to_pdf(docx_path: Path) -> bytes | None:
    soffice = shutil.which("soffice") or shutil.which("libreoffice")
    mac_app = Path("/Applications/LibreOffice.app/Contents/MacOS/soffice")
    if soffice is None and mac_app.exists():
        soffice = str(mac_app)
    if soffice is None:
        return None
    subprocess.run(
        [soffice, "--headless", "--convert-to", "pdf", "--outdir", str(docx_path.parent), str(docx_path)],
        check=True,
        capture_output=True,
        timeout=120,
    )
    return docx_path.with_suffix(".pdf").read_bytes()


def check(
    template: Template,
    profile_fields: dict[str, list[FieldInfo]],
    context: dict[str, Any] | None,
) -> list[BindingReport]:
    """Check each binding. `profile_fields` is metadata per role; `context` holds real values
    when the vault is unlocked and enables fit / glyph checks. Reports never include values."""
    roles = set(template.roles) | set(profile_fields)
    form_fields = (
        {f.name: f for f in pdf_form.inspect_form(template.source)} if template.kind == TemplateKind.PDF_FORM else {}
    )
    reports = []
    for name, expression in template.fields.items():
        report = BindingReport(field=name, status="ok")
        try:
            paths = referenced_paths(expression, roles)
        except Exception as exc:
            reports.append(BindingReport(field=name, status="error", error=str(exc)))
            continue
        for path in sorted(paths):
            role, _, key = path.partition(".")
            infos = profile_fields.get(role)
            if infos is None:
                report.missing.append(path)
                continue
            matches = [i for i in infos if key and (i.key == key or i.key.startswith(key + "."))]
            if not matches:
                report.missing.append(path)
            elif not any(i.length for i in matches):
                report.empty.append(path)
        if report.missing:
            report.status = "missing"
        elif report.empty:
            report.status = "empty"

        if context is not None:
            try:
                text = as_text(evaluate(expression, context))
            except Exception as exc:
                report.status, report.error = "error", f"{type(exc).__name__} with real values"
                reports.append(report)
                continue
            report.missing_glyphs = pdf_form.missing_glyphs(text)
            form_field = form_fields.get(name)
            if form_field and form_field.type == "text" and text:
                comb_len = (form_field.max_len or 0) if form_field.comb else 0
                fit = pdf_form.measure(form_field.rects[0], text, comb_len)
                report.fit_ratio = fit.ratio
                if not fit.fits and report.status == "ok":
                    report.status = "overflow"
            if report.missing_glyphs and report.status == "ok":
                report.status = "glyphs"
        reports.append(report)
    return reports


def describe_template(template: Template) -> dict[str, Any]:
    return template.manifest() | {"source": template.source_path.name}


def dump(data: Any) -> str:
    return json.dumps(data, ensure_ascii=False, indent=1, default=str)
