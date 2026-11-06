//! Generated documents and fill sets, both encrypted under the DEK (fill set bindings can hold personal data).

use super::encrypt;
use super::error::{Result, VaultError};
use super::repo::NewDocument;
use super::{Bindings, DocumentInfo, FillSetInfo, Profiles, Vault};

impl Vault {
    // -- documents -----------------------------------------------------------

    /// In a case: the case moves to the top of the list.
    pub fn store_document(&mut self, doc: &NewDocument, content: &[u8]) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let id = self.repo.insert_document(doc, |id| encrypt::document(&dek, id, content))?;
        self.touch_case(doc.case_id)?;
        Ok(id)
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

    /// `case_id` applies to a new fill set; an existing one stays in its case.
    pub fn save_fill_set(
        &mut self,
        name: &str,
        templates: &[String],
        profiles: &Profiles,
        bindings: &Bindings,
        fill_set_id: Option<i64>,
        case_id: Option<i64>,
    ) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let plain = serde_json::to_vec(bindings)?;
        let id = self
            .repo
            .save_fill_set(fill_set_id, case_id, name, templates, profiles, |id| encrypt::fill_set(&dek, id, &plain))?;
        let case_id = self.list_fill_sets()?.into_iter().find(|f| f.id == id).and_then(|f| f.case_id);
        self.touch_case(case_id)?;
        Ok(id)
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

    /// By name within a case, or among the fill sets without one.
    pub fn find_fill_set(&self, case_id: Option<i64>, name: &str) -> Result<Option<i64>> {
        self.repo.fill_set_id(case_id, name)
    }

    pub fn delete_fill_set(&self, id: i64) -> Result<()> {
        self.repo.delete_fill_set(id)
    }
}
