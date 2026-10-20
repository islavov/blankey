//! Collection links, their submissions and the files collected through them. A link and its submissions are
//! plaintext; each link's private key is a secret under the DEK. Collected files keep their bytes in
//! `files/<id>.bin` under a per-file key, and their name, type and key in the row under the DEK.

use std::path::PathBuf;

use serde_json::Value;

use super::encrypt;
use super::error::{Result, VaultError};
use super::repo::StoredCollected;
use super::{CollectedFile, Link, SubmissionInfo, Vault};
use crate::crypto::{self, Key};

impl Vault {
    // -- links ---------------------------------------------------------------

    /// Stores the link and its private key; needs the vault unlocked.
    pub fn add_link(&self, link: &Link, private: &[u8]) -> Result<()> {
        self.require_dek()?;
        self.repo.insert_link(link)?;
        self.put_secret(&encrypt::link_secret(&link.link_id), private)
    }

    /// Newest first.
    pub fn list_links(&self) -> Result<Vec<Link>> {
        self.repo.links()
    }

    pub fn get_link(&self, link_id: &str) -> Result<Link> {
        self.repo.link(link_id)?.ok_or_else(|| VaultError::NotFound(format!("Unknown link {link_id}")))
    }

    pub fn count_received(&self, link_id: &str) -> Result<()> {
        self.repo.count_received(link_id)
    }

    /// The link's private key; None once the link is revoked.
    pub fn link_key(&self, link_id: &str) -> Result<Option<Key>> {
        self.get_secret(&encrypt::link_secret(link_id))?.map(|bytes| Ok(crypto::key_from(&bytes)?)).transpose()
    }

    /// The key stays, so submissions already received still open; anything pulled for the link later is dropped.
    pub fn revoke_link(&self, link_id: &str) -> Result<()> {
        self.repo.revoke_link(link_id)
    }

    /// Removes the link, its key and its submission rows; files already kept stay. The caller deletes the blobs.
    pub fn delete_link(&mut self, link_id: &str) -> Result<()> {
        self.repo.delete_link(link_id, &encrypt::secret_key(&encrypt::link_secret(link_id)))
    }

    // -- submissions -------------------------------------------------------------

    pub fn add_submission(&self, sid: &str, link_id: &str, request_id: i64, size: i64) -> Result<()> {
        self.repo.insert_submission(sid, link_id, request_id, size)
    }

    /// Newest first; all links when `link_id` is None.
    pub fn list_submissions(&self, link_id: Option<&str>) -> Result<Vec<SubmissionInfo>> {
        self.repo.submissions(link_id)
    }

    pub fn get_submission(&self, sid: &str) -> Result<SubmissionInfo> {
        self.repo.submission(sid)?.ok_or_else(|| VaultError::NotFound(format!("Unknown submission {sid}")))
    }

    pub fn set_submission_status(&self, sid: &str, status: &str, profile_id: Option<i64>) -> Result<()> {
        self.repo.set_submission_status(sid, status, profile_id)
    }

    // -- collected files -------------------------------------------------------------

    fn files_dir(&self) -> PathBuf {
        self.path.with_file_name("files")
    }

    fn collected_path(&self, id: i64) -> PathBuf {
        self.files_dir().join(format!("{id}.bin"))
    }

    /// Encrypts the bytes under a fresh key into `files/<id>.bin`; the name, type and key go in the row.
    pub fn add_collected_file(
        &mut self,
        profile_id: Option<i64>,
        link_id: &str,
        name: &str,
        file_type: &str,
        content: &[u8],
    ) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let dir = self.files_dir();
        std::fs::create_dir_all(&dir)?;
        self.repo.insert_collected_file(profile_id, link_id, content.len() as i64, |id| {
            let key = crypto::new_key();
            let meta = serde_json::json!({"name": name, "type": file_type, "key": crypto::encode_b64(key.as_ref())});
            let sealed = encrypt::collected_meta(&dek, id, &serde_json::to_vec(&meta)?);
            std::fs::write(dir.join(format!("{id}.bin")), encrypt::collected_data(&key, id, content))?;
            Ok(sealed)
        })
    }

    /// Newest first; needs the vault unlocked for the names.
    pub fn list_collected_files(&self) -> Result<Vec<CollectedFile>> {
        self.require_dek()?;
        self.repo.collected_files()?.into_iter().map(|row| Ok(self.open_collected(row)?.0)).collect()
    }

    pub fn load_collected_file(&self, id: i64) -> Result<(CollectedFile, Vec<u8>)> {
        let row = self.repo.collected_file(id)?.ok_or_else(|| VaultError::NotFound(format!("Unknown file {id}")))?;
        let (file, key) = self.open_collected(row)?;
        let blob = std::fs::read(self.collected_path(id))?;
        Ok((file, encrypt::open_collected_data(&key, id, &blob)?))
    }

    pub fn delete_collected_file(&self, id: i64) -> Result<()> {
        self.repo.delete_collected_file(id)?;
        let _ = std::fs::remove_file(self.collected_path(id));
        Ok(())
    }

    /// The file's metadata and its key.
    fn open_collected(&self, row: StoredCollected) -> Result<(CollectedFile, Key)> {
        let dek = self.require_dek()?;
        let plain = encrypt::open_collected_meta(dek, row.id, &row.meta.nonce, &row.meta.ciphertext)?;
        let meta: Value = serde_json::from_slice(&plain)?;
        let text = |name: &str| meta.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
        let key = crypto::key_from(&crypto::decode_b64(&text("key"))?)?;
        let file = CollectedFile {
            id: row.id,
            profile_id: row.profile_id,
            link_id: row.link_id,
            size: row.size,
            created_at: row.created_at,
            name: text("name"),
            file_type: text("type"),
        };
        Ok((file, key))
    }
}
