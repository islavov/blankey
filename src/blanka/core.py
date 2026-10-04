"""Application service shared by the MCP server (metadata + examples) and the UI (real values)."""

import datetime
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from blanka.config import Config
from blanka.output import protect
from blanka.render.preview import page_count
from blanka.templates import TemplateStore, engine
from blanka.templates.bindings import profile_context
from blanka.vault import FieldInfo, Vault


class RequestKind:
    PROFILE_INPUT = "profile_input"
    GENERATE = "generate"


@dataclass(slots=True)
class GenerateOptions:
    sign: str = "none"  # none | self-signed | qes
    password: str | None = None
    pkcs11_pin: str | None = None
    signer_name: str = ""


class Blanka:
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

    def render_real(self, template_id: str, profiles: dict[str, int]) -> engine.Rendered:
        return engine.render(self.templates.get(template_id), self.real_context(profiles))

    def self_signed_identity(self, common_name: str) -> protect.SelfSigned:
        raw = self.vault.get_secret(protect.SELF_SIGNED_SECRET)
        if raw is not None:
            return protect.SelfSigned.load(raw)
        identity = protect.create_self_signed(common_name or "Blanka")
        self.vault.put_secret(protect.SELF_SIGNED_SECRET, identity.dump())
        return identity

    def generate(self, template_id: str, profiles: dict[str, int], options: GenerateOptions, title: str = "") -> int:
        template = self.templates.get(template_id)
        rendered = self.render_real(template_id, profiles)
        content, signed = rendered.content, ""
        if rendered.is_pdf:
            content, signed = protect.protect(
                content,
                password=options.password,
                self_signed=self.self_signed_identity(options.signer_name) if options.sign == "self-signed" else None,
                pkcs11_lib=self.config.pkcs11_lib if options.sign == "qes" else None,
                pkcs11_pin=options.pkcs11_pin,
                reason=title or template.name,
            )
        stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M")
        doc_id = self.vault.store_document(
            template_id=template_id,
            title=title or template.name,
            profiles=profiles,
            pages=page_count(rendered.content) if rendered.is_pdf else 0,
            signed=signed,
            encrypted=bool(options.password) and rendered.is_pdf,
            filename=f"{template_id}-{stamp}.{rendered.extension}",
            content=content,
        )
        self.vault.audit("user", "generate", f"template={template_id} document={doc_id}")
        return doc_id
