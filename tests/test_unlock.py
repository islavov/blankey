import pytest

from blankey import biometric
from blankey.ui.unlock import UnlockDialog, ensure_unlocked, quick_unlock


@pytest.fixture
def fingerprint(monkeypatch):
    state = {"available": True, "ok": True, "prompts": 0}

    def authenticate(reason):
        state["prompts"] += 1
        return state["ok"]

    monkeypatch.setattr(biometric, "available", lambda: state["available"])
    monkeypatch.setattr(biometric, "authenticate", authenticate)
    return state


def test_touch_id_required_for_keychain_unlock(app, fingerprint):
    app.vault.enable_keyring()
    app.vault.lock()
    fingerprint["ok"] = False
    assert not quick_unlock(app.vault)
    assert not app.vault.unlocked
    fingerprint["ok"] = True
    assert quick_unlock(app.vault)
    assert app.vault.unlocked
    assert fingerprint["prompts"] == 2


def test_no_prompt_without_keychain_key(app, fingerprint):
    app.vault.lock()
    assert not quick_unlock(app.vault)
    assert fingerprint["prompts"] == 0


def test_silent_keychain_without_biometrics(app, fingerprint):
    fingerprint["available"] = False
    app.vault.enable_keyring()
    app.vault.lock()
    assert quick_unlock(app.vault)
    assert fingerprint["prompts"] == 0


def test_failed_scan_falls_back_to_password_dialog(qtbot, app, fingerprint, monkeypatch):
    app.vault.enable_keyring()
    app.vault.lock()
    fingerprint["ok"] = False
    shown = []
    monkeypatch.setattr(UnlockDialog, "exec", lambda self: shown.append(self) or 0)
    assert not ensure_unlocked(app)
    assert len(shown) == 1
