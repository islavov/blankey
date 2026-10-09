"""Peewee models over the tables created by the migrations in db.py (the schema itself lives there)."""

import json

from peewee import (
    AutoField,
    BlobField,
    CompositeKey,
    ForeignKeyField,
    IntegerField,
    Model,
    TextField,
)


class JSONField(TextField):
    def db_value(self, value):
        return None if value is None else json.dumps(value, ensure_ascii=False)

    def python_value(self, value):
        return None if value is None else json.loads(value)


class BaseModel(Model):
    class Meta:
        database = None  # bound per vault by db.connect


class Setting(BaseModel):
    key = TextField(primary_key=True)
    value = BlobField()

    class Meta:
        table_name = "meta"


class ProfileRow(BaseModel):
    id = AutoField()
    name = TextField()
    kind = TextField(default="")
    created_at = TextField()

    class Meta:
        table_name = "profiles"


class FieldRow(BaseModel):
    profile = ForeignKeyField(ProfileRow, column_name="profile_id", on_delete="CASCADE")
    key = TextField()
    label = TextField(default="")
    type = TextField(default="text")
    value_length = IntegerField(default=0)
    nonce = BlobField(null=True)
    ciphertext = BlobField(null=True)
    updated_at = TextField()

    class Meta:
        table_name = "fields"
        primary_key = CompositeKey("profile", "key")


class DocumentRow(BaseModel):
    id = AutoField()
    template_id = TextField()
    title = TextField()
    profiles = JSONField()
    pages = IntegerField()
    filename = TextField()
    created_at = TextField()
    nonce = BlobField(default=b"")
    ciphertext = BlobField(default=b"")

    class Meta:
        table_name = "documents"


class FillSetRow(BaseModel):
    id = AutoField()
    name = TextField(unique=True)
    templates = JSONField()
    profiles = JSONField()
    created_at = TextField()
    updated_at = TextField()
    nonce = BlobField(default=b"")
    ciphertext = BlobField(default=b"")

    class Meta:
        table_name = "fill_sets"


class RequestRow(BaseModel):
    id = AutoField()
    kind = TextField()
    payload = TextField(default="{}")  # legacy plaintext, sealed into sealed_payload on unlock
    status = TextField(default="pending")
    result = TextField(null=True)  # legacy plaintext
    created_at = TextField()
    resolved_at = TextField(null=True)
    sealed_payload = BlobField(null=True)
    sealed_result = BlobField(null=True)

    class Meta:
        table_name = "requests"


class AuditRow(BaseModel):
    id = AutoField()
    ts = TextField()
    actor = TextField()
    action = TextField()
    target = TextField(default="")

    class Meta:
        table_name = "audit"


MODELS = [Setting, ProfileRow, FieldRow, DocumentRow, FillSetRow, RequestRow, AuditRow]
