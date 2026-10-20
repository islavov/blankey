//! The data key (DEK) and the keys that wrap it: the password, the one-time recovery key and the OS keychain. Also
//! the request key pair and secrets kept in `meta` under the DEK.

use std::sync::Arc;

use super::encrypt;
use super::error::{Result, VaultError};
use super::repo::SealedRequest;
use super::Vault;
use crate::crypto::{self, KdfParams, Key};
use crate::keychain::SecretStore;

pub(super) const REQUEST_PUBLIC_KEY: &str = "request_public_key";
const REQUEST_PRIVATE_KEY: &str = "request_private_key";

impl Vault {
    pub fn initialized(&self) -> bool {
        self.repo.meta("kdf").ok().flatten().is_some()
    }

    pub fn unlocked(&self) -> bool {
        self.dek.is_some()
    }

    /// Creates the data key and returns the one-time recovery key.
    pub fn initialize(&mut self, password: &str, use_keyring: bool) -> Result<String> {
        if self.initialized() {
            return Err(VaultError::AlreadyInitialized);
        }
        let dek = crypto::new_key();
        let recovery_key = crypto::new_recovery_key();
        self.set_password(&dek, password)?;
        let params = KdfParams::new();
        self.repo.set_meta("recovery_kdf", &serde_json::to_vec(&params)?)?;
        let kek = crypto::derive_key(&recovery_key, &params)?;
        self.repo.set_meta("wrapped_recovery", &crypto::wrap_key(kek.as_ref(), dek.as_ref()))?;
        self.dek = Some(dek);
        self.after_unlock()?;
        if use_keyring {
            self.enable_keyring()?;
        }
        Ok(recovery_key)
    }

    pub fn unlock(&mut self, password: &str) -> Result<()> {
        let params = self.load_kdf("kdf")?;
        let kek = crypto::derive_key(password, &params)?;
        self.unwrap_dek(&kek, "wrapped_password")
    }

    pub fn unlock_with_recovery(&mut self, recovery_key: &str, new_password: &str) -> Result<()> {
        let params = self.load_kdf("recovery_kdf")?;
        let kek = crypto::derive_key(&crypto::normalize_recovery_key(recovery_key), &params)?;
        self.unwrap_dek(&kek, "wrapped_recovery")?;
        self.change_password(new_password)
    }

    /// False when there is no keychain key or it does not open the vault.
    pub fn unlock_with_keyring(&mut self) -> bool {
        let Ok(Some(stored)) = self.secrets.get() else { return false };
        self.unlock_with_keyring_secret(&stored)
    }

    /// The keychain, to read outside the vault lock: a keychain prompt can wait on the user for minutes.
    pub fn secrets(&self) -> Arc<dyn SecretStore> {
        self.secrets.clone()
    }

    /// `stored` is the keychain entry, read with `secrets()`.
    pub fn unlock_with_keyring_secret(&mut self, stored: &str) -> bool {
        if self.repo.meta("wrapped_keyring").ok().flatten().is_none() {
            return false;
        }
        let Ok(kek) = crypto::decode_b64(stored) else { return false };
        let Ok(kek) = crypto::key_from(&kek) else { return false };
        self.unwrap_dek(&kek, "wrapped_keyring").is_ok()
    }

    pub fn enable_keyring(&mut self) -> Result<()> {
        let dek = self.require_dek()?.clone();
        let kek = crypto::new_key();
        self.secrets.set(&crypto::encode_b64(kek.as_ref())).map_err(VaultError::Keychain)?;
        self.repo.set_meta("wrapped_keyring", &crypto::wrap_key(kek.as_ref(), dek.as_ref()))
    }

    pub fn disable_keyring(&mut self) -> Result<()> {
        self.repo.delete_meta("wrapped_keyring")?;
        let _ = self.secrets.delete();
        Ok(())
    }

    pub fn keyring_enabled(&self) -> bool {
        self.repo.meta("wrapped_keyring").ok().flatten().is_some()
    }

    pub fn change_password(&mut self, new_password: &str) -> Result<()> {
        let dek = self.require_dek()?.clone();
        self.set_password(&dek, new_password)
    }

    pub fn lock(&mut self) {
        self.dek = None;
        self.seal_private = None;
    }

    fn set_password(&self, dek: &Key, password: &str) -> Result<()> {
        let params = KdfParams::new();
        let kek = crypto::derive_key(password, &params)?;
        self.repo.set_meta("kdf", &serde_json::to_vec(&params)?)?;
        self.repo.set_meta("wrapped_password", &crypto::wrap_key(kek.as_ref(), dek.as_ref()))
    }

    fn unwrap_dek(&mut self, kek: &Key, meta_key: &str) -> Result<()> {
        let wrapped = self.repo.meta(meta_key)?.ok_or(VaultError::WrongSecret)?;
        let dek = crypto::unwrap_key(kek.as_ref(), &wrapped).map_err(|_| VaultError::WrongSecret)?;
        self.dek = Some(dek);
        self.after_unlock()
    }

    pub(super) fn require_dek(&self) -> Result<&Key> {
        self.dek.as_ref().ok_or(VaultError::Locked)
    }

    fn load_kdf(&self, key: &str) -> Result<KdfParams> {
        let raw = self.repo.meta(key)?.ok_or(VaultError::NotInitialized)?;
        Ok(serde_json::from_slice(&raw)?)
    }

    pub(super) fn get_secret(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(blob) = self.repo.meta(&encrypt::secret_key(name))? else { return Ok(None) };
        Ok(Some(encrypt::open_secret(self.require_dek()?, name, &blob)?))
    }

    pub(super) fn put_secret(&self, name: &str, value: &[u8]) -> Result<()> {
        let blob = encrypt::secret(self.require_dek()?, name, value);
        self.repo.set_meta(&encrypt::secret_key(name), &blob)
    }

    pub(super) fn request_public_key(&self) -> Result<Vec<u8>> {
        self.repo.meta(REQUEST_PUBLIC_KEY)?.ok_or(VaultError::NoRequestKey)
    }

    /// Creates the request key pair on first unlock, then seals requests stored before sealing existed.
    fn after_unlock(&mut self) -> Result<()> {
        let private = match self.get_secret(REQUEST_PRIVATE_KEY)? {
            Some(bytes) => crypto::key_from(&bytes)?,
            None => {
                let (private, public) = crypto::new_seal_keypair();
                self.put_secret(REQUEST_PRIVATE_KEY, private.as_ref())?;
                self.repo.set_meta(REQUEST_PUBLIC_KEY, &public)?;
                private
            }
        };
        self.seal_private = Some(private);
        let public = self.request_public_key()?;
        let plaintext = self.repo.plaintext_requests()?;
        if plaintext.is_empty() {
            return Ok(());
        }
        let mut sealed = Vec::new();
        for (id, payload, result) in plaintext {
            let payload = encrypt::seal_request(&public, id, "payload", payload.as_bytes())?;
            let result = match result {
                Some(r) if !r.is_empty() => Some(encrypt::seal_request(&public, id, "result", r.as_bytes())?),
                _ => None,
            };
            sealed.push(SealedRequest { id, payload, result });
        }
        self.repo.seal_requests(&sealed)
    }
}
