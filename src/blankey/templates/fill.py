"""Fill sets: per-variable choice of source (vault path, literal value or template default) across templates."""

from dataclasses import dataclass, field
from typing import Any

from blankey.render import pdf_form, preview
from blankey.templates import docx
from blankey.templates.bindings import as_text, evaluate, referenced_paths
from blankey.templates.store import Template, TemplateKind

PATH = "path"
VALUE = "value"
DEFAULT = "default"


@dataclass(slots=True)
class FillVar:
    name: str
    label: str = ""
    contexts: list[tuple[str, str]] = field(default_factory=list)  # (template name, context)
    defaults: dict[str, str] = field(default_factory=dict)  # template id -> expression


def roles(templates: list[Template]) -> dict[str, str]:
    out: dict[str, str] = {}
    for template in templates:
        for role, description in template.roles.items():
            out.setdefault(role, description)
    return out


def variables(templates: list[Template]) -> dict[str, FillVar]:
    """Union of variables in template order; a variable shared by several templates appears once."""
    out: dict[str, FillVar] = {}
    for template in templates:
        printed: dict[str, str] = {}
        if template.kind == TemplateKind.DOCX:
            names = docx.variables(template.source_path)
            contexts = docx.blank_contexts(template.source_path)
        elif template.kind == TemplateKind.PDF_FORM:
            names = list(template.fields)
            form_fields = [f for f in pdf_form.inspect_form(template.source) if f.name in template.fields]
            printed = preview.field_labels(template.source, form_fields)
            contexts = {name: [text] for name, text in printed.items()}
        else:
            names, contexts = list(template.fields), {}
        for name in [*names, *(f for f in template.fields if f not in names)]:
            var = out.setdefault(name, FillVar(name))
            var.label = var.label or template.labels.get(name, "") or printed.get(name, "")
            var.contexts += [(template.name, c) for c in contexts.get(name, [])]
            if name in template.fields:
                var.defaults[template.id] = template.fields[name]
    return out


def default_binding(var: FillVar, role_names: set[str]) -> dict[str, Any]:
    """A plain `{{ role.key }}` default becomes a path binding; anything else stays the template default."""
    expressions = set(var.defaults.values())
    if len(expressions) == 1:
        expression = next(iter(expressions))
        paths = referenced_paths(expression, role_names)
        if len(paths) == 1 and expression.replace(" ", "") == "{{" + next(iter(paths)) + "}}":
            return {PATH: next(iter(paths))}
    return {DEFAULT: True} if expressions else {VALUE: ""}


def evaluate_binding(binding: dict[str, Any], context: dict[str, Any], expression: str | None) -> Any:
    if PATH in binding:
        return evaluate("{{ " + binding[PATH] + " }}", context)
    if VALUE in binding:
        return binding[VALUE]
    return evaluate(expression, context) if expression else None


def resolve(
    templates: list[Template], bindings: dict[str, dict[str, Any]], context: dict[str, Any]
) -> dict[str, tuple[dict[str, Any], list[str]]]:
    """Values per template id, plus errors."""
    out = {}
    for template in templates:
        values, errors = {}, []
        names = docx.variables(template.source_path) if template.kind == TemplateKind.DOCX else []
        for name in dict.fromkeys([*names, *template.fields]):
            binding = bindings.get(name) or {DEFAULT: True}
            try:
                values[name] = evaluate_binding(binding, context, template.fields.get(name))
            except Exception as exc:
                errors.append(f"{name}: {type(exc).__name__}: {exc}")
        out[template.id] = (values, errors)
    return out


def describe_binding(binding: dict[str, Any]) -> str:
    """Metadata-only description (safe for MCP)."""
    if PATH in binding:
        return f"path:{binding[PATH]}"
    return VALUE if VALUE in binding else DEFAULT


def is_empty(value: Any) -> bool:
    return not as_text(value).strip()
