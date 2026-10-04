"""Fingerprint check before using the keychain-wrapped vault key (Touch ID on macOS)."""

import sys
import threading

if sys.platform == "darwin":
    import LocalAuthentication

PROMPT_TIMEOUT_S = 60


def available() -> bool:
    if sys.platform != "darwin":
        return False
    context = LocalAuthentication.LAContext.new()
    ok, _ = context.canEvaluatePolicy_error_(LocalAuthentication.LAPolicyDeviceOwnerAuthenticationWithBiometrics, None)
    return bool(ok)


def authenticate(reason: str) -> bool:
    """Show the system Touch ID prompt. False on failure, cancel or "use password"."""
    if not available():
        return False
    context = LocalAuthentication.LAContext.new()
    context.setLocalizedFallbackTitle_("Use password")
    done = threading.Event()
    result = {"ok": False}

    def reply(success, _error):
        result["ok"] = bool(success)
        done.set()

    context.evaluatePolicy_localizedReason_reply_(
        LocalAuthentication.LAPolicyDeviceOwnerAuthenticationWithBiometrics, reason, reply
    )
    done.wait(PROMPT_TIMEOUT_S)
    return result["ok"]
