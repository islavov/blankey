import sqlite3

import pytest
from cryptography.exceptions import InvalidTag

from blanka.vault import FieldInput, FieldType, Vault, VaultLocked, WrongSecret
from blanka.vault.fieldtypes import valid_egn, valid_iban, validate
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
    doc_id = app.vault.store_document("t", "Title", {"applicant": 1}, 1, "", False, "t.pdf", b"%PDF-data")
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
