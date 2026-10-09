import shutil
import sqlite3
from pathlib import Path

import pytest
from cryptography.exceptions import InvalidTag

from blankey.vault import FieldInput, FieldType, Vault, VaultLocked, WrongSecret
from blankey.vault.fieldtypes import valid_egn, valid_iban, validate
from blankey.vault.migrations import VERSION
from blankey.vault.models import MODELS
from tests.conftest import PASSWORD


def test_values_are_encrypted_at_rest(app):
    pid = app.vault.create_profile("Иван", "person")
    app.vault.set_values(pid, [FieldInput("name", "Иван Петров Иванов", "Име", FieldType.TEXT)])
    raw = sqlite3.connect(app.config.db_path).execute("SELECT ciphertext, value_length FROM fields").fetchone()
    assert "Иван".encode() not in raw[0]
    assert raw[1] == len("Иван Петров Иванов")


def test_lock_unlock_and_wrong_password(app):
    pid = app.vault.create_profile("p")
    app.vault.set_values(pid, [FieldInput("egn", "8501011234", type=FieldType.EGN)])
    app.vault.lock()
    with pytest.raises(VaultLocked):
        app.vault.get_values(pid)
    with pytest.raises(WrongSecret):
        app.vault.unlock("wrong")
    app.vault.unlock(PASSWORD)
    assert app.vault.get_values(pid)["egn"] == (FieldType.EGN, "8501011234")


def test_metadata_readable_while_locked(app):
    pid = app.vault.create_profile("p")
    app.vault.set_values(pid, [FieldInput("city", "София", "Град")])
    app.vault.lock()
    [info] = app.vault.describe(pid)
    assert (info.key, info.label, info.length) == ("city", "Град", 5)


def test_length_counts_nfc_characters(app):
    pid = app.vault.create_profile("p")
    decomposed = "й"  # й as two code points
    app.vault.set_values(pid, [FieldInput("x", decomposed)])
    assert app.vault.describe(pid)[0].length == 1


def test_recovery_key_resets_password(tmp_path):
    vault = Vault(tmp_path / "v.db")
    recovery = vault.initialize(PASSWORD)
    vault.lock()
    vault.unlock_with_recovery(recovery.lower().replace("-", " "), "new password")
    vault.lock()
    vault.unlock("new password")
    assert vault.unlocked


def test_keyring_unlock(app):
    app.vault.enable_keyring()
    app.vault.lock()
    assert app.vault.unlock_with_keyring()
    app.vault.disable_keyring()
    app.vault.lock()
    assert not app.vault.unlock_with_keyring()


def test_ciphertext_cannot_be_moved_between_fields(app):
    pid = app.vault.create_profile("p")
    app.vault.set_values(pid, [FieldInput("a", "secret-a"), FieldInput("b", "secret-b")])
    conn = sqlite3.connect(app.config.db_path)
    nonce, ct = conn.execute("SELECT nonce, ciphertext FROM fields WHERE key = 'a'").fetchone()
    conn.execute("UPDATE fields SET nonce = ?, ciphertext = ? WHERE key = 'b'", (nonce, ct))
    conn.commit()
    with pytest.raises(InvalidTag):
        app.vault.get_values(pid)


def test_documents_round_trip(app):
    doc_id = app.vault.store_document("t", "Title", {"applicant": 1}, 1, "t.pdf", b"%PDF-data")
    assert app.vault.load_document(doc_id) == b"%PDF-data"
    assert app.vault.list_documents()[0].filename == "t.pdf"


def test_backup_rotation(app):
    pid = app.vault.create_profile("p")
    for i in range(5):
        app.vault.set_values(pid, [FieldInput("n", str(i))])
    names = sorted(p.name for p in app.config.data_dir.glob("vault.db.*"))
    assert names == ["vault.db.1", "vault.db.2", "vault.db.3"]


@pytest.mark.parametrize(
    ("field_type", "value", "ok"),
    [
        (FieldType.EGN, "7501020018", True),
        (FieldType.EGN, "7501020019", False),
        (FieldType.IBAN, "BG80 BNBG 9661 1020 3456 78", True),
        (FieldType.IBAN, "BG81 BNBG 9661 1020 3456 78", False),
        (FieldType.DATE, "03.10.2026", True),
        (FieldType.DATE, "2026-10-03", False),
        (FieldType.EMAIL, "a@b.bg", True),
        (FieldType.NUMBER, "1 500,50", True),
    ],
)
def test_validation(field_type, value, ok):
    assert (validate(field_type, value) is None) is ok


def test_validators_direct():
    assert valid_egn("7501020018")
    assert valid_iban("BG80BNBG96611020345678")


def test_requests_are_sealed_and_need_unlock(app):
    vault = app.vault
    request_id = vault.create_request("fill", {"reason": "Договор за Тайнов", "values": {"x": "Тайна"}})
    row = (
        sqlite3.connect(app.config.db_path)
        .execute("SELECT payload, sealed_payload FROM requests WHERE id = ?", (request_id,))
        .fetchone()
    )
    assert row[0] == "{}" and "Тайн".encode() not in row[1]

    vault.lock()
    hidden = vault.get_request(request_id)
    assert hidden.hidden and hidden.payload == {} and hidden.kind == "fill"
    assert vault.count_requests("pending") == 1
    vault.unlock(PASSWORD)
    assert vault.get_request(request_id).payload["reason"] == "Договор за Тайнов"

    vault.resolve_request(request_id, "done", {"outcome": "saved", "fill_set_id": 3})
    raw = sqlite3.connect(app.config.db_path).execute("SELECT result FROM requests").fetchone()[0]
    assert raw is None
    vault.lock()
    assert vault.get_request(request_id).result == {"outcome": "saved", "fill_set_id": 3}  # kept for Claude

    fresh = Vault(app.config.db_path)
    assert fresh.get_request(request_id).result is None
    fresh.unlock(PASSWORD)
    assert fresh.get_request(request_id).result["fill_set_id"] == 3
    fresh.close()


def test_plaintext_requests_from_before_sealing_are_sealed_on_unlock(app):
    conn = sqlite3.connect(app.config.db_path)
    conn.execute(
        "INSERT INTO requests (kind, payload, status, result, created_at) VALUES "
        "('generate', '{\"reason\": \"Стар\"}', 'done', '{\"outcome\": \"generated\"}', '2026-01-01')"
    )
    conn.commit()
    app.vault.lock()
    app.vault.unlock(PASSWORD)
    payload, result, sealed = conn.execute("SELECT payload, result, sealed_payload FROM requests").fetchone()
    assert payload == "{}" and result is None and sealed is not None
    request = app.vault.list_requests()[0]
    assert request.payload == {"reason": "Стар"} and request.result == {"outcome": "generated"}


def test_requests_need_a_key_pair(app):
    with sqlite3.connect(app.config.db_path) as conn:
        conn.execute("DELETE FROM meta WHERE key = 'request_public_key'")
    with pytest.raises(VaultLocked, match="Unlock Blankey once"):
        app.vault.create_request("fill", {})


DATA = Path(__file__).parent / "data"


def _v1_database(path: Path) -> Path:
    conn = sqlite3.connect(path)
    conn.executescript((DATA / "schema_v1.sql").read_text() + "PRAGMA user_version = 1;")
    conn.execute(
        "INSERT INTO requests (kind, payload, status, created_at) VALUES ('fill', ?, 'pending', 'x')",
        ('{"reason": "Стар"}',),
    )
    conn.commit()
    conn.close()
    return path


@pytest.mark.parametrize("origin", ["new", "v1", "v3"])
def test_models_match_the_schema(tmp_path, origin):
    path = tmp_path / "vault.db"
    if origin == "v1":
        _v1_database(path)
    elif origin == "v3":
        shutil.copy(DATA / "vault_v3.db", path)
    Vault(path).close()
    conn = sqlite3.connect(path)
    assert conn.execute("PRAGMA user_version").fetchone()[0] == VERSION
    for model in MODELS:
        columns = {row[1]: row for row in conn.execute(f"PRAGMA table_info({model._meta.table_name})")}
        declared = {field.column_name for field in model._meta.sorted_fields}
        assert declared <= set(columns), model.__name__
        required = {name for name, (_, _, _, notnull, default, pk) in columns.items() if notnull and default is None}
        assert required <= declared, model.__name__


def test_upgrades_a_first_version_vault(tmp_path):
    vault = Vault(_v1_database(tmp_path / "vault.db"))
    vault.initialize(PASSWORD)
    assert vault.list_requests()[0].payload == {"reason": "Стар"}
    fill_set_id = vault.save_fill_set("delta", ["loan"], {}, {"amount": {"value": "1"}})
    assert vault.get_fill_set(fill_set_id)[1] == {"amount": {"value": "1"}}
    vault.close()


def test_opens_a_vault_written_by_the_previous_db_layer(tmp_path):
    path = tmp_path / "vault.db"
    shutil.copy(DATA / "vault_v3.db", path)
    vault = Vault(path)
    assert [(p.name, p.kind) for p in vault.list_profiles()] == [("Иван", "person")]
    assert [(f.key, f.label, f.length) for f in vault.describe(1)] == [("egn", "ЕГН", 0), ("name", "Име", 11)]
    [request] = vault.list_requests()
    assert request.hidden and request.status == "done"
    vault.unlock("legacy-pw")
    assert vault.get_values(1)["name"] == (FieldType.TEXT, "Иван Иванов")
    info, bindings = vault.get_fill_set(vault.find_fill_set("delta"))
    assert (info.templates, info.profiles, bindings) == (["loan"], {"manager": 1}, {"amount": {"value": "34 000"}})
    [doc] = vault.list_documents()
    assert (doc.title, doc.profiles, vault.load_document(doc.id)) == ("Договор", {"manager": 1}, b"%PDF-legacy")
    request = vault.get_request(request.id)
    assert (request.payload, request.result) == ({"reason": "Попълни"}, {"outcome": "saved"})
    vault.close()
