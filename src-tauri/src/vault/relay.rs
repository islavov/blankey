//! Blankey Online: accepted agents and the app's relay settings. Plaintext metadata, so the relay client runs while
//! the vault is locked.

use super::error::Result;
use super::{RelayGrant, Vault};

const RELAY_DEVICE_ID: &str = "relay_device_id";
const RELAY_CLIENT: &str = "relay_client";
const RELAY_ACCOUNT: &str = "relay_account";
const RELAY_LANGUAGE: &str = "relay_language";

impl Vault {
    pub fn add_relay_grant(&self, grant: &RelayGrant) -> Result<()> {
        self.repo.insert_relay_grant(grant)
    }

    /// Grants on this relay that are not revoked, newest first.
    pub fn relay_grants(&self, relay_url: &str) -> Result<Vec<RelayGrant>> {
        self.repo.relay_grants(relay_url)
    }

    pub fn relay_grant_allowed(&self, grant_id: &str, relay_url: &str) -> Result<bool> {
        self.repo.relay_grant_allowed(grant_id, relay_url)
    }

    pub fn revoke_relay_grant(&self, grant_id: &str) -> Result<()> {
        self.repo.revoke_relay_grant(grant_id)
    }

    pub fn clear_relay_grants(&self, relay_url: &str) -> Result<()> {
        self.repo.clear_relay_grants(relay_url)
    }

    pub fn relay_device_id(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_DEVICE_ID)
    }

    pub fn set_relay_device_id(&self, device_id: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_DEVICE_ID, device_id)
    }

    /// The account the app is signed in to on the relay, as the relay names it (an email address).
    pub fn relay_account(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_ACCOUNT)
    }

    pub fn set_relay_account(&self, account: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_ACCOUNT, account)
    }

    /// The UI language of the relay account, as its device profile last named it.
    pub fn relay_language(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_LANGUAGE)
    }

    pub fn set_relay_language(&self, language: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_LANGUAGE, language)
    }

    /// The app's own OAuth client registration on the relay (JSON).
    pub fn relay_client(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_CLIENT)
    }

    pub fn set_relay_client(&self, client: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_CLIENT, client)
    }

    fn text_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.repo.meta(key)?.map(|v| String::from_utf8_lossy(&v).into_owned()))
    }

    fn set_text_meta(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => self.repo.set_meta(key, value.as_bytes()),
            None => self.repo.delete_meta(key),
        }
    }
}
