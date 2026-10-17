//! Same primitives and byte layouts as the Python vault, so existing vault.db files open unchanged.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

pub const KEY_SIZE: usize = 32;
pub const NONCE_SIZE: usize = 12;
const WRAP_AAD: &[u8] = b"blanka:dek";
const SEAL_INFO: &[u8] = b"blankey:seal";

pub type Key = Zeroizing<[u8; KEY_SIZE]>;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("decryption failed")]
    Decrypt,
    #[error("key derivation failed: {0}")]
    Kdf(String),
    #[error("malformed data")]
    Malformed,
}

#[derive(Serialize, Deserialize)]
pub struct KdfParams {
    #[serde(with = "b64")]
    pub salt: Vec<u8>,
    pub time_cost: u32,
    pub memory_cost: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self::new()
    }
}

impl KdfParams {
    pub fn new() -> Self {
        Self { salt: random_bytes(16), time_cost: 3, memory_cost: 64 * 1024, parallelism: 4 }
    }
}

mod b64 {
    use super::B64;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&B64.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        B64.decode(text).map_err(serde::de::Error::custom)
    }
}

pub fn encode_b64(data: &[u8]) -> String {
    B64.encode(data)
}

pub fn decode_b64(text: &str) -> Result<Vec<u8>, CryptoError> {
    B64.decode(text.trim()).map_err(|_| CryptoError::Malformed)
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    OsRng.fill_bytes(&mut out);
    out
}

pub fn new_key() -> Key {
    let mut key = Zeroizing::new([0u8; KEY_SIZE]);
    OsRng.fill_bytes(key.as_mut());
    key
}

pub fn key_from(bytes: &[u8]) -> Result<Key, CryptoError> {
    let array: [u8; KEY_SIZE] = bytes.try_into().map_err(|_| CryptoError::Malformed)?;
    Ok(Zeroizing::new(array))
}

pub fn derive_key(secret: &str, params: &KdfParams) -> Result<Key, CryptoError> {
    let argon_params = Params::new(params.memory_cost, params.time_cost, params.parallelism, Some(KEY_SIZE))
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
    let mut key = Zeroizing::new([0u8; KEY_SIZE]);
    argon
        .hash_password_into(secret.as_bytes(), &params.salt, key.as_mut())
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    Ok(key)
}

/// Returns (nonce, ciphertext with tag).
pub fn encrypt(key: &[u8], plaintext: &[u8], aad: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let nonce = random_bytes(NONCE_SIZE);
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad }).expect("AES-GCM encryption");
    (nonce, ct)
}

pub fn decrypt(key: &[u8], nonce: &[u8], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if nonce.len() != NONCE_SIZE {
        return Err(CryptoError::Malformed);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::Malformed)?;
    cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad }).map_err(|_| CryptoError::Decrypt)
}

/// nonce + ciphertext in one blob
pub fn encrypt_joined(key: &[u8], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let (mut nonce, ct) = encrypt(key, plaintext, aad);
    nonce.extend(ct);
    nonce
}

pub fn decrypt_joined(key: &[u8], blob: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if blob.len() < NONCE_SIZE {
        return Err(CryptoError::Malformed);
    }
    decrypt(key, &blob[..NONCE_SIZE], &blob[NONCE_SIZE..], aad)
}

pub fn wrap_key(kek: &[u8], dek: &[u8]) -> Vec<u8> {
    encrypt_joined(kek, dek, WRAP_AAD)
}

pub fn unwrap_key(kek: &[u8], wrapped: &[u8]) -> Result<Key, CryptoError> {
    let plain = Zeroizing::new(decrypt_joined(kek, wrapped, WRAP_AAD)?);
    key_from(&plain)
}

pub fn new_recovery_key() -> String {
    let raw = data_encoding::BASE32_NOPAD.encode(&random_bytes(20));
    group4(&raw)
}

pub fn normalize_recovery_key(value: &str) -> String {
    let raw: String = value.to_uppercase().chars().filter(|c| c.is_alphanumeric()).collect();
    group4(&raw)
}

fn group4(raw: &str) -> String {
    raw.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

// -- sealing: encrypt to a public key while the vault is locked, open only when unlocked --------

/// (private, public) raw X25519 keys
pub fn new_seal_keypair() -> (Key, [u8; 32]) {
    let secret = StaticSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&secret);
    (Zeroizing::new(secret.to_bytes()), public.to_bytes())
}

fn seal_key(shared: &[u8], ephemeral: &[u8], public: &[u8]) -> Key {
    let info = [SEAL_INFO, ephemeral, public].concat();
    let mut key = Zeroizing::new([0u8; KEY_SIZE]);
    Hkdf::<Sha256>::new(None, shared).expand(&info, key.as_mut()).expect("HKDF length");
    key
}

/// `ephemeral_pub(32) || nonce(12) || ciphertext`; the relay's collect page (seal.js) builds the same bytes.
pub fn seal(public: &[u8], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let ephemeral = StaticSecret::random_from_rng(OsRng);
    let nonce: [u8; NONCE_SIZE] = random_bytes(NONCE_SIZE).try_into().unwrap();
    seal_with(&ephemeral, &nonce, public, plaintext, aad)
}

fn seal_with(
    ephemeral: &StaticSecret,
    nonce: &[u8; NONCE_SIZE],
    public: &[u8],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let public: [u8; 32] = public.try_into().map_err(|_| CryptoError::Malformed)?;
    let ephemeral_public = PublicKey::from(ephemeral).to_bytes();
    let shared = ephemeral.diffie_hellman(&PublicKey::from(public));
    let key = seal_key(shared.as_bytes(), &ephemeral_public, &public);
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).expect("32-byte key");
    let ct = cipher.encrypt(Nonce::from_slice(nonce), Payload { msg: plaintext, aad }).expect("AES-GCM encryption");
    Ok([&ephemeral_public[..], nonce, &ct].concat())
}

/// What a collection link carries in its fragment: base64url(SHA-256(public)[..12]), 16 characters.
pub fn fingerprint(public: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&Sha256::digest(public)[..12])
}

pub fn unseal(private: &[u8], sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let private: [u8; 32] = private.try_into().map_err(|_| CryptoError::Malformed)?;
    if sealed.len() < 32 + NONCE_SIZE {
        return Err(CryptoError::Malformed);
    }
    let secret = StaticSecret::from(private);
    let public = PublicKey::from(&secret).to_bytes();
    let ephemeral: [u8; 32] = sealed[..32].try_into().unwrap();
    let shared = secret.diffie_hellman(&PublicKey::from(ephemeral));
    let key = seal_key(shared.as_bytes(), &ephemeral, &public);
    decrypt_joined(key.as_ref(), &sealed[32..], aad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_roundtrip() {
        let kek = new_key();
        let dek = new_key();
        let wrapped = wrap_key(kek.as_ref(), dek.as_ref());
        assert_eq!(unwrap_key(kek.as_ref(), &wrapped).unwrap()[..], dek[..]);
        assert!(unwrap_key(new_key().as_ref(), &wrapped).is_err());
    }

    #[test]
    fn seal_roundtrip() {
        let (private, public) = new_seal_keypair();
        let sealed = seal(&public, b"hello", b"aad").unwrap();
        assert_eq!(unseal(private.as_ref(), &sealed, b"aad").unwrap(), b"hello");
        assert!(unseal(private.as_ref(), &sealed, b"other").is_err());
    }

    /// The relay copies tests/data/seal_vectors.json into its own tests, so seal.js must produce the same bytes.
    #[test]
    fn seal_vectors() {
        let hex = data_encoding::HEXLOWER;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/seal_vectors.json");
        let cases = [
            (
                "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
                "c8c7c6c5c4c3c2c1c0bfbebdbcbbbab9b8b7b6b5b4b3b2b1b0afaeadacabaaa9",
                "00070e151c232a31383f464d",
                "link:XchaBj2",
                b"hello".to_vec(),
            ),
            (
                "05121f2c394653606d7a8794a1aebbc8d5e2effc091623303d4a5764717e8b98",
                "0928476685a4c3e201203f5e7d9cbbdaf91837567594b3d2f1102f4e6d8cabca",
                "fffefdfcfbfaf9f8f7f6f5f4",
                "link:Ab3dE9z",
                vector_plaintext(),
            ),
        ];
        let vectors: Vec<serde_json::Value> = cases
            .iter()
            .map(|(private, ephemeral, nonce, aad, plaintext)| {
                let private: [u8; 32] = hex.decode(private.as_bytes()).unwrap().try_into().unwrap();
                let ephemeral: [u8; 32] = hex.decode(ephemeral.as_bytes()).unwrap().try_into().unwrap();
                let nonce: [u8; NONCE_SIZE] = hex.decode(nonce.as_bytes()).unwrap().try_into().unwrap();
                let public = PublicKey::from(&StaticSecret::from(private)).to_bytes();
                let sealed =
                    seal_with(&StaticSecret::from(ephemeral), &nonce, &public, plaintext, aad.as_bytes()).unwrap();
                assert_eq!(unseal(&private, &sealed, aad.as_bytes()).unwrap(), *plaintext);
                serde_json::json!({
                    "recipient_private": hex.encode(&private),
                    "recipient_public": hex.encode(&public),
                    "ephemeral_private": hex.encode(&ephemeral),
                    "nonce": hex.encode(&nonce),
                    "aad": aad,
                    "plaintext": hex.encode(plaintext),
                    "sealed": hex.encode(&sealed),
                    "fingerprint": fingerprint(&public),
                })
            })
            .collect();
        let expected = serde_json::to_string_pretty(&serde_json::json!({"vectors": vectors})).unwrap() + "\n";
        match std::fs::read_to_string(&path) {
            Ok(existing) => assert_eq!(existing, expected, "seal output changed; seal.js must change with it"),
            Err(_) => std::fs::write(&path, expected).unwrap(),
        }
    }

    /// A small submission in the collect page's layout: u32 BE header length, header JSON, file bytes.
    fn vector_plaintext() -> Vec<u8> {
        let header = r#"{"v":1,"values":{"name":"Иван"},"files":[{"name":"a.bin","type":"application/octet-stream","size":4}],"submitted_at":"2026-10-17T12:00:00.000Z"}"#;
        let mut out = (header.len() as u32).to_be_bytes().to_vec();
        out.extend(header.as_bytes());
        out.extend([0, 1, 2, 0xff]);
        out
    }

    #[test]
    fn recovery_key_format() {
        let key = new_recovery_key();
        assert_eq!(key.len(), 32 + 7);
        assert_eq!(normalize_recovery_key(&key.to_lowercase().replace('-', " ")), key);
    }
}
