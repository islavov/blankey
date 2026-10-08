import re
import shutil
from dataclasses import dataclass, field
from enum import StrEnum
from pathlib import Path
from typing import Any

import yaml

ID_RE = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")
SOURCE_NAMES = {"pdf_form": "form.pdf", "typst": "main.typ", "docx": "template.docx"}


class TemplateKind(StrEnum):
    PDF_FORM = "pdf_form"
    TYPST = "typst"
    DOCX = "docx"


@dataclass(slots=True)
class Template:
    id: str
    name: str
    kind: TemplateKind
    directory: Path
    roles: dict[str, str] = field(default_factory=dict)  # role -> description
    fields: dict[str, str] = field(default_factory=dict)  # field / variable -> Jinja expression
    description: str = ""
    labels: dict[str, str] = field(default_factory=dict)  # variable -> human label
    boxes: list[dict[str, Any]] = field(default_factory=list)  # fields drawn on a flat PDF (see flat.pdf)

    @property
    def source_path(self) -> Path:
        return self.directory / SOURCE_NAMES[self.kind]

    @property
    def source(self) -> bytes:
        return self.source_path.read_bytes()

    def manifest(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "kind": str(self.kind),
            "description": self.description,
            "roles": self.roles,
            "fields": self.fields,
            "labels": self.labels,
        } | ({"boxes": self.boxes} if self.boxes else {})


class TemplateStore:
    def __init__(self, root: Path):
        self.root = root

    def all(self) -> list[Template]:
        return [self.get(p.name) for p in sorted(self.root.iterdir()) if (p / "manifest.yaml").exists()]

    def get(self, template_id: str) -> Template:
        directory = self.root / template_id
        manifest_path = directory / "manifest.yaml"
        if not ID_RE.match(template_id) or not manifest_path.exists():
            raise KeyError(f"Unknown template {template_id!r}")
        data = yaml.safe_load(manifest_path.read_text(encoding="utf-8"))
        return Template(
            id=template_id,
            name=data.get("name", template_id),
            kind=TemplateKind(data["kind"]),
            directory=directory,
            roles=data.get("roles") or {},
            fields={str(k): str(v) for k, v in (data.get("fields") or {}).items()},
            description=data.get("description", ""),
            labels={str(k): str(v) for k, v in (data.get("labels") or {}).items()},
            boxes=data.get("boxes") or [],
        )

    def save(
        self,
        template_id: str,
        name: str,
        kind: TemplateKind,
        roles: dict[str, str],
        fields: dict[str, str],
        description: str = "",
        source_file: Path | None = None,
        source_text: str | None = None,
        labels: dict[str, str] | None = None,
        boxes: list[dict[str, Any]] | None = None,
    ) -> Template:
        if not ID_RE.match(template_id):
            raise ValueError("Template id must be lowercase letters, digits, '-' or '_'")
        template = Template(
            template_id,
            name,
            TemplateKind(kind),
            self.root / template_id,
            roles,
            fields,
            description,
            labels or {},
            boxes or [],
        )
        template.directory.mkdir(parents=True, exist_ok=True)
        if source_file is not None:
            shutil.copyfile(source_file, template.source_path)
        elif source_text is not None:
            template.source_path.write_text(source_text, encoding="utf-8")
        if not template.source_path.exists():
            raise ValueError(f"Template {template_id!r} has no source; pass source_file or source_text")
        self._write_manifest(template)
        return template

    def _write_manifest(self, template: Template) -> None:
        text = yaml.safe_dump(template.manifest(), allow_unicode=True, sort_keys=False, width=120)
        (template.directory / "manifest.yaml").write_text(text, encoding="utf-8")

    def delete(self, template_id: str) -> None:
        shutil.rmtree(self.get(template_id).directory)

    # examples are plaintext, non-PII datasets keyed by role
    def examples(self, template_id: str) -> list[str]:
        directory = self.get(template_id).directory / "examples"
        return sorted(p.stem for p in directory.glob("*.yaml")) if directory.exists() else []

    def get_example(self, template_id: str, name: str) -> dict[str, Any]:
        path = self.get(template_id).directory / "examples" / f"{name}.yaml"
        if not ID_RE.match(name) or not path.exists():
            raise KeyError(f"Unknown example {name!r} for template {template_id!r}")
        return yaml.safe_load(path.read_text(encoding="utf-8")) or {}

    def save_example(self, template_id: str, name: str, data: dict[str, Any]) -> None:
        if not ID_RE.match(name):
            raise ValueError("Example name must be lowercase letters, digits, '-' or '_'")
        directory = self.get(template_id).directory / "examples"
        directory.mkdir(exist_ok=True)
        text = yaml.safe_dump(data, allow_unicode=True, sort_keys=False, width=120)
        (directory / f"{name}.yaml").write_text(text, encoding="utf-8")
