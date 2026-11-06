//! Cases and the profiles linked to them. A case's title and the roles are plaintext metadata, like profile names;
//! its notes are encrypted under the DEK.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::encrypt;
use super::error::{Result, VaultError};
use super::{CaseCounts, CaseInfo, CaseLink, Vault};

impl Vault {
    pub fn create_case(&self, title: &str) -> Result<i64> {
        self.repo.insert_case(&uuid::Uuid::new_v4().to_string(), title)
    }

    /// Latest change first.
    pub fn list_cases(&self) -> Result<Vec<CaseInfo>> {
        self.repo.cases()
    }

    pub fn get_case(&self, id: i64) -> Result<CaseInfo> {
        self.repo.case(id)?.ok_or_else(|| VaultError::NotFound(format!("Unknown case {id}")))
    }

    /// Needs the vault unlocked.
    pub fn case_notes(&self, id: i64) -> Result<String> {
        let dek = self.require_dek()?;
        let Some(e) = self.repo.case_private(id)? else { return Ok(String::new()) };
        let private: Value = serde_json::from_slice(&encrypt::open_case_private(dek, id, &e.nonce, &e.ciphertext)?)?;
        Ok(private.get("notes").and_then(Value::as_str).unwrap_or_default().to_string())
    }

    /// Changes what is given; either one moves the case to the top of the list.
    pub fn update_case(&self, id: i64, title: Option<&str>, notes: Option<&str>) -> Result<()> {
        self.get_case(id)?;
        if let Some(notes) = notes {
            let dek = self.require_dek()?;
            let private = serde_json::to_vec(&json!({"notes": notes}))?;
            self.repo.set_case_private(id, &encrypt::case_private(dek, id, &private))?;
        }
        if let Some(title) = title {
            self.repo.set_case_title(id, title)?;
        }
        Ok(())
    }

    pub fn touch_case(&self, case_id: Option<i64>) -> Result<()> {
        match case_id {
            Some(id) => self.repo.touch_case(id),
            None => Ok(()),
        }
    }

    /// Removes the case with its links, files (and their blobs), documents and fill sets. Its requests stay in the
    /// activity, without the case. Profiles and templates are the caller's.
    pub fn delete_case(&mut self, id: i64) -> Result<()> {
        self.get_case(id)?;
        for file in self.repo.delete_case(id)? {
            let _ = std::fs::remove_file(self.collected_path(file));
        }
        Ok(())
    }

    pub fn case_counts(&self, id: i64) -> Result<CaseCounts> {
        self.repo.case_counts(id)
    }

    /// case id -> number of files, for the cases that have any.
    pub fn case_file_counts(&self) -> Result<HashMap<i64, i64>> {
        self.repo.case_file_counts()
    }

    // -- profiles in cases ---------------------------------------------------------

    /// Every link, by case and in the order the profiles were added.
    pub fn case_links(&self) -> Result<Vec<CaseLink>> {
        self.repo.case_links()
    }

    /// Links the profile, or changes its role when it is linked already.
    pub fn link_profile(&self, case_id: i64, profile_id: i64, role: &str) -> Result<()> {
        self.get_case(case_id)?;
        self.get_profile(profile_id)?;
        self.repo.link_profile(case_id, profile_id, role)?;
        self.repo.touch_case(case_id)
    }

    pub fn unlink_profile(&self, case_id: i64, profile_id: i64) -> Result<()> {
        self.repo.unlink_profile(case_id, profile_id)?;
        self.repo.touch_case(case_id)
    }

    /// A profile that shows only in this case (and those it is linked to later), until it is made global.
    pub fn create_local_profile(&mut self, name: &str, kind: &str, case_id: i64, role: &str) -> Result<i64> {
        self.get_case(case_id)?;
        self.repo.insert_local_profile(name, kind, case_id, role)
    }

    pub fn make_profile_global(&self, id: i64) -> Result<()> {
        self.get_profile(id)?;
        self.repo.set_profile_global(id)
    }
}
