import base64
import os
import secrets
from dataclasses import dataclass
from typing import Self

from argon2.low_level import Type, hash_secret_raw
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

KEY_SIZE = 32
NONCE_SIZE = 12
WRAP_AAD = b"blanka:dek"


@dataclass(slots=True, frozen=True)
class KdfParams:
    salt: bytes
    time_cost: int = 3
    memory_cost: int = 64 * 1024
    parallelism: int = 4

    @classmethod
    def new(cls) -> Self:
        return cls(salt=os.urandom(16))


def derive_key(secret: str, params: KdfParams) -> bytes:
    return hash_secret_raw(
        secret.encode("utf-8"),
        params.salt,
        time_cost=params.time_cost,
        memory_cost=params.memory_cost,
        parallelism=params.parallelism,
        hash_len=KEY_SIZE,
        type=Type.ID,
    )


def new_key() -> bytes:
    return AESGCM.generate_key(bit_length=KEY_SIZE * 8)


def encrypt(key: bytes, plaintext: bytes, aad: bytes) -> tuple[bytes, bytes]:
    nonce = os.urandom(NONCE_SIZE)
    return nonce, AESGCM(key).encrypt(nonce, plaintext, aad)


def decrypt(key: bytes, nonce: bytes, ciphertext: bytes, aad: bytes) -> bytes:
    return AESGCM(key).decrypt(nonce, ciphertext, aad)


def wrap_key(kek: bytes, dek: bytes) -> bytes:
    nonce, ct = encrypt(kek, dek, WRAP_AAD)
    return nonce + ct


def unwrap_key(kek: bytes, wrapped: bytes) -> bytes:
    return decrypt(kek, wrapped[:NONCE_SIZE], wrapped[NONCE_SIZE:], WRAP_AAD)


def new_recovery_key() -> str:
    raw = base64.b32encode(secrets.token_bytes(20)).decode().rstrip("=")
    return "-".join(raw[i : i + 4] for i in range(0, len(raw), 4))


def normalize_recovery_key(value: str) -> str:
    raw = "".join(ch for ch in value.upper() if ch.isalnum())
    return "-".join(raw[i : i + 4] for i in range(0, len(raw), 4))


# -- sealing: encrypt with a public key while the vault is locked, open only when unlocked --------

SEAL_INFO = b"blankey:seal"


def new_seal_keypair() -> tuple[bytes, bytes]:
    """(private, public) X25519 raw keys."""
    private = X25519PrivateKey.generate()
    return private.private_bytes_raw(), private.public_key().public_bytes_raw()


def _seal_key(shared: bytes, ephemeral: bytes, public: bytes) -> bytes:
    return HKDF(algorithm=hashes.SHA256(), length=KEY_SIZE, salt=None, info=SEAL_INFO + ephemeral + public).derive(
        shared
    )


def seal(public: bytes, plaintext: bytes, aad: bytes) -> bytes:
    ephemeral = X25519PrivateKey.generate()
    ephemeral_public = ephemeral.public_key().public_bytes_raw()
    key = _seal_key(ephemeral.exchange(X25519PublicKey.from_public_bytes(public)), ephemeral_public, public)
    nonce, ct = encrypt(key, plaintext, aad)
    return ephemeral_public + nonce + ct


def unseal(private: bytes, sealed: bytes, aad: bytes) -> bytes:
    secret = X25519PrivateKey.from_private_bytes(private)
    ephemeral_public, nonce, ct = sealed[:32], sealed[32 : 32 + NONCE_SIZE], sealed[32 + NONCE_SIZE :]
    public = secret.public_key().public_bytes_raw()
    key = _seal_key(secret.exchange(X25519PublicKey.from_public_bytes(ephemeral_public)), ephemeral_public, public)
    return decrypt(key, nonce, ct, aad)
