"""PDF encryption (pypdf, AES-256) and signing (pyHanko: self-signed or PKCS#11 / QES)."""

import datetime
import io
import secrets
from dataclasses import dataclass
from typing import Self

from asn1crypto import keys, x509
from cryptography import x509 as cx509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID
from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.sign import pkcs11, signers
from pyhanko_certvalidator.registry import SimpleCertificateStore
from pypdf import PdfReader, PdfWriter

SELF_SIGNED_SECRET = "selfsigned"


@dataclass(slots=True)
class SelfSigned:
    cert_der: bytes
    key_der: bytes  # PKCS#8

    def dump(self) -> bytes:
        return len(self.cert_der).to_bytes(4, "big") + self.cert_der + self.key_der

    @classmethod
    def load(cls, raw: bytes) -> Self:
        size = int.from_bytes(raw[:4], "big")
        return cls(raw[4 : 4 + size], raw[4 + size :])


def create_self_signed(common_name: str, years: int = 10) -> SelfSigned:
    key = ec.generate_private_key(ec.SECP256R1())
    name = cx509.Name([cx509.NameAttribute(NameOID.COMMON_NAME, common_name)])
    now = datetime.datetime.now(datetime.UTC)
    cert = (
        cx509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(cx509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=365 * years))
        .add_extension(cx509.KeyUsage(True, True, False, False, False, False, False, False, False), critical=True)
        .sign(key, hashes.SHA256())
    )
    return SelfSigned(
        cert.public_bytes(serialization.Encoding.DER),
        key.private_bytes(serialization.Encoding.DER, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()),
    )


def encrypt(pdf: bytes, password: str) -> bytes:
    writer = PdfWriter(clone_from=PdfReader(io.BytesIO(pdf)))
    writer.encrypt(user_password=password, owner_password=secrets.token_urlsafe(24), algorithm="AES-256")
    out = io.BytesIO()
    writer.write(out)
    return out.getvalue()


def _sign(pdf: bytes, signer: signers.Signer, password: str | None, reason: str) -> bytes:
    writer = IncrementalPdfFileWriter(io.BytesIO(pdf))
    if password:
        writer.encrypt(password)
    meta = signers.PdfSignatureMetadata(field_name="Signature1", reason=reason, md_algorithm="sha256")
    out = io.BytesIO()
    signers.sign_pdf(writer, meta, signer=signer, output=out)
    return out.getvalue()


def sign_self_signed(pdf: bytes, identity: SelfSigned, password: str | None = None, reason: str = "") -> bytes:
    cert = x509.Certificate.load(identity.cert_der)
    signer = signers.SimpleSigner(
        signing_cert=cert,
        signing_key=keys.PrivateKeyInfo.load(identity.key_der),
        cert_registry=SimpleCertificateStore.from_certs([cert]),
    )
    return _sign(pdf, signer, password, reason)


def sign_pkcs11(
    pdf: bytes, lib_path: str, pin: str, cert_label: str | None = None, password: str | None = None, reason: str = ""
) -> bytes:
    with pkcs11.open_pkcs11_session(lib_path, user_pin=pin) as session:
        signer = pkcs11.PKCS11Signer(session, cert_label=cert_label)
        return _sign(pdf, signer, password, reason)


def protect(
    pdf: bytes,
    *,
    password: str | None = None,
    self_signed: SelfSigned | None = None,
    pkcs11_lib: str | None = None,
    pkcs11_pin: str | None = None,
    reason: str = "",
) -> tuple[bytes, str]:
    """Encrypt first, then sign incrementally so the signature stays valid. Returns (pdf, signed_kind)."""
    if password:
        pdf = encrypt(pdf, password)
    if pkcs11_lib:
        return sign_pkcs11(pdf, pkcs11_lib, pkcs11_pin or "", password=password, reason=reason), "qes"
    if self_signed is not None:
        return sign_self_signed(pdf, self_signed, password, reason), "self-signed"
    return pdf, ""
