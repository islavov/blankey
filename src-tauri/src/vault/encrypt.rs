//! What the vault encrypts and under which associated data. The AAD binds each ciphertext to its row, so a
//! ciphertext copied to another field, document or request does not open. Changing a string here makes existing
//! vaults unreadable.

use super::error::Result;
use crate::crypto::{self, Key};

/// AES-GCM output stored as two columns.
pub struct Encrypted {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

fn encrypt(key: &Key, plaintext: &[u8], aad: &[u8]) -> Encrypted {
    let (nonce, ciphertext) = crypto::encrypt(key.as_ref(), plaintext, aad);
    Encrypted { nonce, ciphertext }
}

fn field_aad(profile_id: i64, key: &str) -> Vec<u8> {
    format!("field:{profile_id}|{key}").into_bytes()
}

fn request_aad(request_id: i64, part: &str) -> Vec<u8> {
    format!("request:{request_id}|{part}").into_bytes()
}

fn collected_aad(id: i64) -> Vec<u8> {
    format!("collected:{id}").into_bytes()
}

fn collected_data_aad(id: i64) -> Vec<u8> {
    format!("collected:{id}|data").into_bytes()
}

/// The meta key a secret is stored under, also its AAD.
pub fn secret_key(name: &str) -> String {
    format!("secret:{name}")
}

/// The secret name of a collection link's private key.
pub fn link_secret(link_id: &str) -> String {
    format!("link:{link_id}")
}

// -- profile fields ----------------------------------------------------------

/// None for an empty value: it is stored as NULL columns.
pub fn field(dek: &Key, profile_id: i64, key: &str, value: &str) -> Option<Encrypted> {
    (!value.is_empty()).then(|| encrypt(dek, value.as_bytes(), &field_aad(profile_id, key)))
}

pub fn open_field(dek: &Key, profile_id: i64, key: &str, nonce: &[u8], ciphertext: &[u8]) -> Result<String> {
    let bytes = crypto::decrypt(dek.as_ref(), nonce, ciphertext, &field_aad(profile_id, key))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// -- documents and fill sets --------------------------------------------------

pub fn document(dek: &Key, id: i64, content: &[u8]) -> Encrypted {
    encrypt(dek, content, format!("document:{id}").as_bytes())
}

pub fn open_document(dek: &Key, id: i64, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto::decrypt(dek.as_ref(), nonce, ciphertext, format!("document:{id}").as_bytes())?)
}

/// Fill set bindings are encrypted because literal values may be personal data.
pub fn fill_set(dek: &Key, id: i64, bindings: &[u8]) -> Encrypted {
    encrypt(dek, bindings, format!("fillset:{id}").as_bytes())
}

pub fn open_fill_set(dek: &Key, id: i64, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto::decrypt(dek.as_ref(), nonce, ciphertext, format!("fillset:{id}").as_bytes())?)
}

// -- requests ------------------------------------------------------------------
// Sealed to the request public key, so a request can be stored while the vault is locked.

/// `part`: "payload" or "result".
pub fn seal_request(public: &[u8], id: i64, part: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto::seal(public, plaintext, &request_aad(id, part))?)
}

pub fn unseal_request(private: &Key, id: i64, part: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto::unseal(private.as_ref(), sealed, &request_aad(id, part))?)
}

// -- secrets in meta ---------------------------------------------------------------

pub fn secret(dek: &Key, name: &str, value: &[u8]) -> Vec<u8> {
    crypto::encrypt_joined(dek.as_ref(), value, secret_key(name).as_bytes())
}

pub fn open_secret(dek: &Key, name: &str, blob: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto::decrypt_joined(dek.as_ref(), blob, secret_key(name).as_bytes())?)
}

// -- collected files ------------------------------------------------------------------
// The row holds the name, type and a per-file key under the DEK; the bytes sit in files/<id>.bin under that key.

pub fn collected_meta(dek: &Key, id: i64, meta: &[u8]) -> Encrypted {
    encrypt(dek, meta, &collected_aad(id))
}

pub fn open_collected_meta(dek: &Key, id: i64, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto::decrypt(dek.as_ref(), nonce, ciphertext, &collected_aad(id))?)
}

pub fn collected_data(file_key: &Key, id: i64, content: &[u8]) -> Vec<u8> {
    crypto::encrypt_joined(file_key.as_ref(), content, &collected_data_aad(id))
}

pub fn open_collected_data(file_key: &Key, id: i64, blob: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto::decrypt_joined(file_key.as_ref(), blob, &collected_data_aad(id))?)
}
