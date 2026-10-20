//! Generated documents and fill sets, both encrypted under the DEK (fill set bindings can hold personal data).

use super::encrypt;
use super::error::{Result, VaultError};
use super::repo::NewDocument;
use super::{Bindings, DocumentInfo, FillSetInfo, Profiles, Vault};

impl Vault {
    // -- documents -----------------------------------------------------------

    pub fn store_document(
        &mut self,
        template_id: &str,
        title: &str,
        profiles: &Profiles,
        pages: i64,
        filename: &str,
        content: &[u8],
    ) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let doc = NewDocument { template_id, title, profiles, pages, filename };
        self.repo.insert_document(&doc, |id| encrypt::document(&dek, id, content))
    }

    pub fn list_documents(&self) -> Result<Vec<DocumentInfo>> {
        self.repo.documents()
    }

    /// (filename, content)
    pub fn load_document(&self, id: i64) -> Result<(String, Vec<u8>)> {
        let dek = self.require_dek()?;
        let (filename, e) =
            self.repo.document_content(id)?.ok_or_else(|| VaultError::NotFound(format!("Unknown document {id}")))?;
        Ok((filename, encrypt::open_document(dek, id, &e.nonce, &e.ciphertext)?))
    }

    pub fn delete_document(&self, id: i64) -> Result<()> {
        self.repo.delete_document(id)
    }

    // -- fill sets -----------------------------------------------------------

    pub fn save_fill_set(
        &mut self,
        name: &str,
        templates: &[String],
        profiles: &Profiles,
        bindings: &Bindings,
        fill_set_id: Option<i64>,
    ) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let plain = serde_json::to_vec(bindings)?;
        self.repo.save_fill_set(fill_set_id, name, templates, profiles, |id| encrypt::fill_set(&dek, id, &plain))
    }

    pub fn list_fill_sets(&self) -> Result<Vec<FillSetInfo>> {
        self.repo.fill_sets()
    }

    pub fn get_fill_set(&self, id: i64) -> Result<(FillSetInfo, Bindings)> {
        let dek = self.require_dek()?;
        let unknown = || VaultError::NotFound(format!("Unknown fill set {id}"));
        let info = self.list_fill_sets()?.into_iter().find(|f| f.id == id).ok_or_else(unknown)?;
        let e = self.repo.fill_set_bindings(id)?.ok_or_else(unknown)?;
        let plain = encrypt::open_fill_set(dek, id, &e.nonce, &e.ciphertext)?;
        Ok((info, serde_json::from_slice(&plain)?))
    }

    pub fn find_fill_set(&self, name: &str) -> Result<Option<i64>> {
        self.repo.fill_set_id(name)
    }

    pub fn delete_fill_set(&self, id: i64) -> Result<()> {
        self.repo.delete_fill_set(id)
    }
}
