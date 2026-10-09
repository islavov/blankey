import base64
import contextlib
import datetime
import json
import threading
import unicodedata
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import keyring
from cryptography.exceptions import InvalidTag
from keyring.errors import PasswordDeleteError

from blankey.vault import crypto, db
from blankey.vault.fieldtypes import FieldType
from blankey.vault.models import AuditRow, DocumentRow, FieldRow, FillSetRow, ProfileRow, RequestRow, Setting

KEYRING_SERVICE = "blankey"
KEYRING_USER = "vault-kek"
REQUEST_PUBLIC_KEY = "request_public_key"
REQUEST_PRIVATE_KEY = "request_private_key"


class VaultLocked(Exception):
    pass


class WrongSecret(Exception):
    pass


@dataclass(slots=True, frozen=True)
class Profile:
    id: int
    name: str
    kind: str


@dataclass(slots=True, frozen=True)
class FieldInfo:
    key: str
    label: str
    type: FieldType
    length: int


@dataclass(slots=True, frozen=True)
class FieldInput:
    key: str
    value: str
    label: str = ""
    type: FieldType = FieldType.TEXT


@dataclass(slots=True, frozen=True)
class DocumentInfo:
    id: int
    template_id: str
    title: str
    profiles: dict[str, int]
    pages: int
    filename: str
    created_at: str


@dataclass(slots=True, frozen=True)
class FillSetInfo:
    id: int
    name: str
    templates: list[str]
    profiles: dict[str, int]
    updated_at: str


@dataclass(slots=True, frozen=True)
class Request:
    id: int
    kind: str
    payload: dict[str, Any]
    status: str
    result: dict[str, Any] | None
    created_at: str
    resolved_at: str | None
    hidden: bool = False  # payload sealed and the vault is locked


def _request_aad(request_id: int, part: str = "payload") -> bytes:
    return f"request:{request_id}|{part}".encode()


def now() -> str:
    return datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds")


def normalize(value: str) -> str:
    return unicodedata.normalize("NFC", value)


def _b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def _unb64(data: str) -> bytes:
    return base64.b64decode(data)


class Vault:
    """Encrypted profile store. Values are AES-GCM encrypted per field; metadata stays readable."""

    def __init__(self, path: Path):
        self.path = path
        self.db = db.connect(path)
        self._lock = threading.RLock()
        self._dek: bytes | None = None
        self._seal_private: bytes | None = None
        self._results: dict[int, dict[str, Any]] = {}  # resolved results for Claude, kept for this process

    # -- key management ----------------------------------------------------

    @property
    def initialized(self) -> bool:
        return self._meta("kdf") is not None

    @property
    def unlocked(self) -> bool:
        return self._dek is not None

    def initialize(self, password: str, use_keyring: bool = False) -> str:
        """Create the data key and return the one-time recovery key."""
        if self.initialized:
            raise RuntimeError("Vault is already initialized")
        dek = crypto.new_key()
        recovery_key = crypto.new_recovery_key()
        with self._lock:
            self._set_password(dek, password)
            params = crypto.KdfParams.new()
            self._set_meta("recovery_kdf", self._dump_kdf(params))
            kek = crypto.derive_key(recovery_key, params)
            self._set_meta("wrapped_recovery", crypto.wrap_key(kek, dek))
            self._dek = dek
            self._after_unlock()
            if use_keyring:
                self.enable_keyring()
        return recovery_key

    def unlock(self, password: str) -> None:
        params = self._load_kdf("kdf")
        self._unwrap(crypto.derive_key(password, params), "wrapped_password")

    def unlock_with_recovery(self, recovery_key: str, new_password: str) -> None:
        params = self._load_kdf("recovery_kdf")
        self._unwrap(crypto.derive_key(crypto.normalize_recovery_key(recovery_key), params), "wrapped_recovery")
        self.change_password(new_password)

    def unlock_with_keyring(self) -> bool:
        stored = keyring.get_password(KEYRING_SERVICE, KEYRING_USER)
        if not stored or self._meta("wrapped_keyring") is None:
            return False
        try:
            self._unwrap(_unb64(stored), "wrapped_keyring")
        except WrongSecret:
            return False
        return True

    def enable_keyring(self) -> None:
        dek = self._require_dek()
        kek = crypto.new_key()
        keyring.set_password(KEYRING_SERVICE, KEYRING_USER, _b64(kek))
        self._set_meta("wrapped_keyring", crypto.wrap_key(kek, dek))

    def disable_keyring(self) -> None:
        Setting.delete_by_id("wrapped_keyring")
        with contextlib.suppress(PasswordDeleteError):
            keyring.delete_password(KEYRING_SERVICE, KEYRING_USER)

    @property
    def keyring_enabled(self) -> bool:
        return self._meta("wrapped_keyring") is not None

    def change_password(self, new_password: str) -> None:
        self._set_password(self._require_dek(), new_password)

    def lock(self) -> None:
        self._dek = None
        self._seal_private = None

    def _set_password(self, dek: bytes, password: str) -> None:
        params = crypto.KdfParams.new()
        self._set_meta("kdf", self._dump_kdf(params))
        self._set_meta("wrapped_password", crypto.wrap_key(crypto.derive_key(password, params), dek))

    def _unwrap(self, kek: bytes, meta_key: str) -> None:
        wrapped = self._meta(meta_key)
        if wrapped is None:
            raise WrongSecret(meta_key)
        try:
            self._dek = crypto.unwrap_key(kek, wrapped)
        except InvalidTag as exc:
            raise WrongSecret(meta_key) from exc
        self._after_unlock()

    def _require_dek(self) -> bytes:
        if self._dek is None:
            raise VaultLocked("Vault is locked")
        return self._dek

    @staticmethod
    def _dump_kdf(params: crypto.KdfParams) -> bytes:
        return json.dumps(
            {
                "salt": _b64(params.salt),
                "time_cost": params.time_cost,
                "memory_cost": params.memory_cost,
                "parallelism": params.parallelism,
            }
        ).encode()

    def _load_kdf(self, key: str) -> crypto.KdfParams:
        raw = self._meta(key)
        if raw is None:
            raise RuntimeError("Vault is not initialized")
        data = json.loads(raw)
        return crypto.KdfParams(
            salt=_unb64(data["salt"]),
            time_cost=data["time_cost"],
            memory_cost=data["memory_cost"],
            parallelism=data["parallelism"],
        )

    def _meta(self, key: str) -> bytes | None:
        with self._lock:
            row = Setting.get_or_none(Setting.key == key)
        return bytes(row.value) if row else None

    def _set_meta(self, key: str, value: bytes) -> None:
        with self._lock:
            Setting.replace(key=key, value=value).execute()

    def get_secret(self, name: str) -> bytes | None:
        raw = self._meta(f"secret:{name}")
        if raw is None:
            return None
        aad = f"secret:{name}".encode()
        return crypto.decrypt(self._require_dek(), raw[: crypto.NONCE_SIZE], raw[crypto.NONCE_SIZE :], aad)

    def put_secret(self, name: str, value: bytes) -> None:
        nonce, ct = crypto.encrypt(self._require_dek(), value, f"secret:{name}".encode())
        self._set_meta(f"secret:{name}", nonce + ct)

    # -- profiles ----------------------------------------------------------

    def list_profiles(self) -> list[Profile]:
        with self._lock:
            return [_profile(r) for r in ProfileRow.select().order_by(ProfileRow.name)]

    def get_profile(self, profile_id: int) -> Profile:
        with self._lock:
            row = ProfileRow.get_or_none(ProfileRow.id == profile_id)
        if row is None:
            raise KeyError(f"Unknown profile {profile_id}")
        return _profile(row)

    def create_profile(self, name: str, kind: str = "") -> int:
        with self._lock:
            return ProfileRow.insert(name=name, kind=kind, created_at=now()).execute()

    def update_profile(self, profile_id: int, name: str, kind: str) -> None:
        with self._lock:
            ProfileRow.update(name=name, kind=kind).where(ProfileRow.id == profile_id).execute()

    def delete_profile(self, profile_id: int) -> None:
        with self._lock:
            ProfileRow.delete_by_id(profile_id)

    def describe(self, profile_id: int) -> list[FieldInfo]:
        self.get_profile(profile_id)
        with self._lock:
            rows = FieldRow.select().where(FieldRow.profile == profile_id).order_by(FieldRow.key)
            return [FieldInfo(r.key, r.label, FieldType(r.type), r.value_length) for r in rows]

    def define_fields(self, profile_id: int, fields: Iterable[tuple[str, str, FieldType]]) -> None:
        """Upsert field definitions (key, label, type) without touching values."""
        with self._lock, self.db.atomic():
            for key, label, field_type in fields:
                FieldRow.insert(
                    profile=profile_id, key=key, label=label, type=str(field_type), updated_at=now()
                ).on_conflict(
                    conflict_target=[FieldRow.profile, FieldRow.key],
                    preserve=[FieldRow.label, FieldRow.type],
                ).execute()

    def set_values(self, profile_id: int, values: Iterable[FieldInput]) -> None:
        dek = self._require_dek()
        with self._lock, self.db.atomic():
            for item in values:
                value = normalize(item.value)
                if value:
                    nonce, ct = crypto.encrypt(dek, value.encode("utf-8"), self._aad(profile_id, item.key))
                else:
                    nonce = ct = None
                FieldRow.insert(
                    profile=profile_id,
                    key=item.key,
                    label=item.label,
                    type=str(item.type),
                    value_length=len(value),
                    nonce=nonce,
                    ciphertext=ct,
                    updated_at=now(),
                ).on_conflict(
                    conflict_target=[FieldRow.profile, FieldRow.key],
                    preserve=[
                        FieldRow.label,
                        FieldRow.type,
                        FieldRow.value_length,
                        FieldRow.nonce,
                        FieldRow.ciphertext,
                        FieldRow.updated_at,
                    ],
                ).execute()
        self.backup()

    def delete_field(self, profile_id: int, key: str) -> None:
        with self._lock:
            FieldRow.delete().where((FieldRow.profile == profile_id) & (FieldRow.key == key)).execute()

    def get_values(self, profile_id: int) -> dict[str, tuple[FieldType, str]]:
        dek = self._require_dek()
        with self._lock:
            rows = list(FieldRow.select().where(FieldRow.profile == profile_id))
        values = {}
        for r in rows:
            plain = ""
            if r.ciphertext is not None:
                plain = crypto.decrypt(dek, bytes(r.nonce), bytes(r.ciphertext), self._aad(profile_id, r.key)).decode()
            values[r.key] = (FieldType(r.type), plain)
        return values

    @staticmethod
    def _aad(profile_id: int, key: str) -> bytes:
        return f"field:{profile_id}|{key}".encode()

    # -- documents ---------------------------------------------------------

    def store_document(
        self,
        template_id: str,
        title: str,
        profiles: dict[str, int],
        pages: int,
        filename: str,
        content: bytes,
    ) -> int:
        dek = self._require_dek()
        with self._lock, self.db.atomic():
            doc_id = DocumentRow.insert(
                template_id=template_id,
                title=title,
                profiles=profiles,
                pages=pages,
                filename=filename,
                created_at=now(),
            ).execute()
            nonce, ct = crypto.encrypt(dek, content, f"document:{doc_id}".encode())
            DocumentRow.update(nonce=nonce, ciphertext=ct).where(DocumentRow.id == doc_id).execute()
        return doc_id

    def list_documents(self) -> list[DocumentInfo]:
        with self._lock:
            rows = DocumentRow.select(
                DocumentRow.id,
                DocumentRow.template_id,
                DocumentRow.title,
                DocumentRow.profiles,
                DocumentRow.pages,
                DocumentRow.filename,
                DocumentRow.created_at,
            ).order_by(DocumentRow.id.desc())
            return [
                DocumentInfo(r.id, r.template_id, r.title, r.profiles, r.pages, r.filename, r.created_at) for r in rows
            ]

    def load_document(self, doc_id: int) -> bytes:
        dek = self._require_dek()
        with self._lock:
            row = DocumentRow.get_or_none(DocumentRow.id == doc_id)
        if row is None:
            raise KeyError(f"Unknown document {doc_id}")
        return crypto.decrypt(dek, bytes(row.nonce), bytes(row.ciphertext), f"document:{doc_id}".encode())

    def delete_document(self, doc_id: int) -> None:
        with self._lock:
            DocumentRow.delete_by_id(doc_id)

    # -- fill sets ---------------------------------------------------------
    # bindings: {var: {"path": "role.key"} | {"value": "..."} | {"default": True}}; encrypted because
    # literal values may be personal data

    def save_fill_set(
        self,
        name: str,
        templates: list[str],
        profiles: dict[str, int],
        bindings: dict[str, dict[str, Any]],
        fill_set_id: int | None = None,
    ) -> int:
        dek = self._require_dek()
        meta = {"name": name, "templates": templates, "profiles": profiles, "updated_at": now()}
        with self._lock, self.db.atomic():
            if fill_set_id is None:
                fill_set_id = FillSetRow.insert(**meta, created_at=now()).execute()
            elif not FillSetRow.update(**meta).where(FillSetRow.id == fill_set_id).execute():
                raise KeyError(f"Unknown fill set {fill_set_id}")
            plain = json.dumps(bindings, ensure_ascii=False).encode()
            nonce, ct = crypto.encrypt(dek, plain, f"fillset:{fill_set_id}".encode())
            FillSetRow.update(nonce=nonce, ciphertext=ct).where(FillSetRow.id == fill_set_id).execute()
        return fill_set_id

    def list_fill_sets(self) -> list[FillSetInfo]:
        with self._lock:
            return [_fill_set_info(r) for r in FillSetRow.select().order_by(FillSetRow.id.desc())]

    def get_fill_set(self, fill_set_id: int) -> tuple[FillSetInfo, dict[str, dict[str, Any]]]:
        dek = self._require_dek()
        with self._lock:
            row = FillSetRow.get_or_none(FillSetRow.id == fill_set_id)
        if row is None:
            raise KeyError(f"Unknown fill set {fill_set_id}")
        plain = crypto.decrypt(dek, bytes(row.nonce), bytes(row.ciphertext), f"fillset:{fill_set_id}".encode())
        return _fill_set_info(row), json.loads(plain)

    def find_fill_set(self, name: str) -> int | None:
        with self._lock:
            row = FillSetRow.get_or_none(FillSetRow.name == name)
        return row.id if row else None

    def delete_fill_set(self, fill_set_id: int) -> None:
        with self._lock:
            FillSetRow.delete_by_id(fill_set_id)

    # -- requests ----------------------------------------------------------
    # Payloads and results are sealed to the request public key, so the MCP thread can store a request while
    # the vault is locked, and only an unlocked vault can read it. Kind, status and timestamps stay readable.

    def create_request(self, kind: str, payload: dict[str, Any]) -> int:
        public = self._meta(REQUEST_PUBLIC_KEY)
        if public is None:
            raise VaultLocked("Unlock Blankey once to enable requests from Claude")
        with self._lock, self.db.atomic():
            request_id = RequestRow.insert(kind=kind, created_at=now()).execute()
            sealed = crypto.seal(public, json.dumps(payload, ensure_ascii=False).encode(), _request_aad(request_id))
            RequestRow.update(sealed_payload=sealed).where(RequestRow.id == request_id).execute()
        return request_id

    def get_request(self, request_id: int) -> Request:
        with self._lock:
            row = RequestRow.get_or_none(RequestRow.id == request_id)
        if row is None:
            raise KeyError(f"Unknown request {request_id}")
        return self._request(row)

    def list_requests(self, status: str | None = None) -> list[Request]:
        query = RequestRow.select()
        if status:
            query = query.where(RequestRow.status == status).order_by(RequestRow.id)
        else:
            query = query.order_by(RequestRow.id.desc()).limit(100)
        with self._lock:
            rows = list(query)
        return [self._request(r) for r in rows]

    def count_requests(self, status: str) -> int:
        with self._lock:
            return RequestRow.select().where(RequestRow.status == status).count()

    def resolve_request(self, request_id: int, status: str, result: dict[str, Any]) -> None:
        public = self._meta(REQUEST_PUBLIC_KEY)
        plain = json.dumps(result, ensure_ascii=False).encode()
        sealed = crypto.seal(public, plain, _request_aad(request_id, "result"))
        with self._lock:
            RequestRow.update(status=status, result=None, sealed_result=sealed, resolved_at=now()).where(
                RequestRow.id == request_id
            ).execute()
        self._results[request_id] = result

    def _request(self, row: RequestRow) -> Request:
        payload: dict[str, Any] = {}
        result = self._results.get(row.id)
        hidden = False
        if row.sealed_payload is not None:
            if self._seal_private is None:
                hidden = True
            else:
                payload = json.loads(crypto.unseal(self._seal_private, bytes(row.sealed_payload), _request_aad(row.id)))
        else:
            payload = json.loads(row.payload)
        if result is None and row.sealed_result is not None and self._seal_private is not None:
            aad = _request_aad(row.id, "result")
            result = json.loads(crypto.unseal(self._seal_private, bytes(row.sealed_result), aad))
        elif result is None and row.result:
            result = json.loads(row.result)
        return Request(row.id, row.kind, payload, row.status, result, row.created_at, row.resolved_at, hidden)

    def _after_unlock(self) -> None:
        """Create the request key pair on first unlock, then seal requests stored before sealing existed."""
        private = self.get_secret(REQUEST_PRIVATE_KEY)
        if private is None:
            private, public = crypto.new_seal_keypair()
            self.put_secret(REQUEST_PRIVATE_KEY, private)
            self._set_meta(REQUEST_PUBLIC_KEY, public)
        self._seal_private = private
        public = self._meta(REQUEST_PUBLIC_KEY)
        with self._lock, self.db.atomic():
            for row in RequestRow.select().where(RequestRow.sealed_payload.is_null()):
                sealed_result = (
                    crypto.seal(public, row.result.encode(), _request_aad(row.id, "result")) if row.result else None
                )
                RequestRow.update(
                    payload="{}",
                    result=None,
                    sealed_payload=crypto.seal(public, row.payload.encode(), _request_aad(row.id)),
                    sealed_result=sealed_result,
                ).where(RequestRow.id == row.id).execute()

    # -- audit & maintenance -----------------------------------------------

    def audit(self, actor: str, action: str, target: str = "") -> None:
        with self._lock:
            AuditRow.insert(ts=now(), actor=actor, action=action, target=target).execute()

    def backup(self) -> None:
        with self._lock:
            db.backup(self.db, self.path)

    def close(self) -> None:
        self.lock()
        self.db.close()


def _profile(row: ProfileRow) -> Profile:
    return Profile(row.id, row.name, row.kind)


def _fill_set_info(row: FillSetRow) -> FillSetInfo:
    profiles = {role: int(pid) for role, pid in row.profiles.items()}
    return FillSetInfo(row.id, row.name, row.templates, profiles, row.updated_at)
