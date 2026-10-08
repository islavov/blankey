"""Application service shared by the MCP server (metadata + examples) and the UI (real values)."""

import datetime
import zipfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

from blankey.config import Config
from blankey.output import protect
from blankey.render.preview import page_count
from blankey.templates import Template, TemplateStore, engine, fill
from blankey.templates.bindings import profile_context
from blankey.vault import FieldInfo, Vault


class RequestKind:
    PROFILE_INPUT = "profile_input"
    FILL = "fill"


@dataclass(slots=True)
class GenerateOptions:
    sign: str = "none"  # none | self-signed | qes
    password: str | None = None
    pkcs11_pin: str | None = None
    signer_name: str = ""


class Blankey:
    def __init__(self, config: Config):
        self.config = config
        self.vault = Vault(config.db_path)
        self.templates = TemplateStore(config.templates_dir)
        self.on_request: Callable[[int], None] = lambda request_id: None

    # -- requests ----------------------------------------------------------

    def create_request(self, kind: str, payload: dict[str, Any]) -> int:
        request_id = self.vault.create_request(kind, payload)
        self.vault.audit("mcp", f"request:{kind}", str(request_id))
        self.on_request(request_id)
        return request_id

    # -- metadata (safe for MCP) -------------------------------------------

    def profile_fields(self, profiles: dict[str, int]) -> dict[str, list[FieldInfo]]:
        return {role: self.vault.describe(profile_id) for role, profile_id in profiles.items()}

    def check_bindings(self, template_id: str, profiles: dict[str, int]) -> list[engine.BindingReport]:
        template = self.templates.get(template_id)
        context = self.real_context(profiles) if self.vault.unlocked else None
        return engine.check(template, self.profile_fields(profiles), context)

    # -- real values (UI only, never returned through MCP) -----------------

    def real_context(self, profiles: dict[str, int]) -> dict[str, Any]:
        return {role: profile_context(self.vault.get_values(pid)) for role, pid in profiles.items()}

    def self_signed_identity(self, common_name: str) -> protect.SelfSigned:
        raw = self.vault.get_secret(protect.SELF_SIGNED_SECRET)
        if raw is not None:
            return protect.SelfSigned.load(raw)
        identity = protect.create_self_signed(common_name or "Blankey")
        self.vault.put_secret(protect.SELF_SIGNED_SECRET, identity.dump())
        return identity

    def fill_templates(self, template_ids: list[str]) -> list[Template]:
        return [self.templates.get(t) for t in template_ids]

    def generate_fill(self, fill_set_id: int, options: GenerateOptions | None = None) -> list[int]:
        """Render every template of a fill set and store the documents. PDFs are signed / encrypted per options."""
        options = options or GenerateOptions()
        info, bindings = self.vault.get_fill_set(fill_set_id)
        templates = self.fill_templates(info.templates)
        resolved = fill.resolve(templates, bindings, self.real_context(info.profiles))
        doc_ids = []
        for template in templates:
            values, errors = resolved[template.id]
            if errors:
                raise ValueError(f"{template.name}: " + "; ".join(errors))
            rendered = engine.render_values(template, values)
            title = f"{template.name} ({info.name})"
            content, signed = rendered.content, ""
            if rendered.is_pdf:
                content, signed = protect.protect(
                    content,
                    password=options.password,
                    self_signed=self.self_signed_identity(options.signer_name)
                    if options.sign == "self-signed"
                    else None,
                    pkcs11_lib=self.config.pkcs11_lib if options.sign == "qes" else None,
                    pkcs11_pin=options.pkcs11_pin,
                    reason=title,
                )
            encrypted = bool(options.password)
            doc_ids.append(self._store(template, rendered, content, signed, encrypted, info.profiles, title))
        return doc_ids

    def export_fill_set(self, fill_set_id: int) -> str:
        info, bindings = self.vault.get_fill_set(fill_set_id)
        data = {"name": info.name, "templates": info.templates, "profiles": info.profiles, "bindings": bindings}
        self.vault.audit("user", "export_fill_set", str(fill_set_id))
        return yaml.safe_dump(data, allow_unicode=True, sort_keys=False, width=120)

    def export_fill_bundle(self, fill_set_id: int, target: Path) -> None:
        """Zip with each template's manifest + source and the fill set as fill.yaml."""
        info, _ = self.vault.get_fill_set(fill_set_id)
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as bundle:
            for template in self.fill_templates(info.templates):
                for path in (template.directory / "manifest.yaml", template.source_path):
                    bundle.write(path, f"{template.id}/{path.name}")
            bundle.writestr("fill.yaml", self.export_fill_set(fill_set_id))

    def import_fill_set(self, text: str) -> int:
        data = yaml.safe_load(text) or {}
        name, templates = data.get("name"), data.get("templates") or []
        if not name or not templates:
            raise ValueError("A fill set needs a name and templates")
        self.fill_templates(templates)
        profiles = {role: int(pid) for role, pid in (data.get("profiles") or {}).items()}
        for profile_id in profiles.values():
            self.vault.get_profile(profile_id)
        bindings = data.get("bindings") or {}
        fill_set_id = self.vault.save_fill_set(name, templates, profiles, bindings, self.vault.find_fill_set(name))
        self.vault.audit("user", "import_fill_set", str(fill_set_id))
        return fill_set_id

    def _store(
        self,
        template: Template,
        rendered: engine.Rendered,
        content: bytes,
        signed: str,
        encrypted: bool,
        profiles: dict[str, int],
        title: str,
    ) -> int:
        template_id = template.id
        stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M")
        doc_id = self.vault.store_document(
            template_id=template_id,
            title=title or template.name,
            profiles=profiles,
            pages=page_count(rendered.content) if rendered.is_pdf else 0,
            signed=signed,
            encrypted=encrypted and rendered.is_pdf,
            filename=f"{template_id}-{stamp}.{rendered.extension}",
            content=content,
        )
        self.vault.audit("user", "generate", f"template={template_id} document={doc_id}")
        return doc_id
