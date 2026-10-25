//! Profiles and their fields. Keys, labels, types and value lengths are plaintext metadata; values are encrypted per
//! field.

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

use super::encrypt;
use super::error::{Result, VaultError};
use super::repo::FieldRow;
use super::{Field, FieldInfo, FieldInput, Profile, Vault};

impl Vault {
    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        self.repo.profiles()
    }

    pub fn get_profile(&self, id: i64) -> Result<Profile> {
        self.repo.profile(id)?.ok_or_else(|| VaultError::NotFound(format!("Unknown profile {id}")))
    }

    pub fn create_profile(&self, name: &str, kind: &str) -> Result<i64> {
        self.repo.insert_profile(name, kind)
    }

    pub fn update_profile(&self, id: i64, name: &str, kind: &str) -> Result<()> {
        self.repo.update_profile(id, name, kind)
    }

    /// The profile, its fields and the files kept with it.
    pub fn delete_profile(&self, id: i64) -> Result<()> {
        for file in self.repo.collected_files()?.into_iter().filter(|f| f.profile_id == Some(id)) {
            self.delete_collected_file(file.id)?;
        }
        self.repo.delete_profile(id)
    }

    pub fn describe(&self, profile_id: i64) -> Result<Vec<FieldInfo>> {
        self.get_profile(profile_id)?;
        self.repo.field_infos(profile_id)
    }

    /// Field metadata of every profile at once (no values), for searching.
    pub fn field_index(&self) -> Result<HashMap<i64, Vec<FieldInfo>>> {
        self.repo.field_index()
    }

    /// Field metadata, plus decrypted values when unlocked.
    pub fn fields(&self, profile_id: i64) -> Result<Vec<Field>> {
        let values = if self.unlocked() { Some(self.get_values(profile_id)?) } else { None };
        Ok(self
            .describe(profile_id)?
            .into_iter()
            .map(|info| {
                let value =
                    values.as_ref().map(|v| v.get(&info.key).map(|(_, value)| value.clone()).unwrap_or_default());
                Field { key: info.key, label: info.label, field_type: info.field_type, length: info.length, value }
            })
            .collect())
    }

    /// key -> (type, plaintext)
    pub fn get_values(&self, profile_id: i64) -> Result<indexmap::IndexMap<String, (String, String)>> {
        let dek = self.require_dek()?;
        let mut out = indexmap::IndexMap::new();
        for (key, field_type, value) in self.repo.encrypted_fields(profile_id)? {
            let plain = match value {
                Some(e) => encrypt::open_field(dek, profile_id, &key, &e.nonce, &e.ciphertext)?,
                None => String::new(),
            };
            out.insert(key, (field_type, plain));
        }
        Ok(out)
    }

    /// Upserts field definitions (key, label, type) without touching values.
    pub fn define_fields(&self, profile_id: i64, fields: &[(String, String, String)]) -> Result<()> {
        self.repo.define_fields(profile_id, fields)
    }

    /// Values are stored NFC-normalized; the length is counted in characters.
    pub fn set_values(&mut self, profile_id: i64, values: &[FieldInput]) -> Result<()> {
        let dek = self.require_dek()?.clone();
        let normalized: Vec<String> = values.iter().map(|item| item.value.nfc().collect()).collect();
        let rows: Vec<FieldRow> = values
            .iter()
            .zip(&normalized)
            .map(|(item, value)| FieldRow {
                key: &item.key,
                label: &item.label,
                field_type: &item.field_type,
                length: value.chars().count() as i64,
                value: encrypt::field(&dek, profile_id, &item.key, value),
            })
            .collect();
        self.repo.replace_fields(profile_id, &rows)?;
        self.backup()
    }

    pub fn delete_field(&self, profile_id: i64, key: &str) -> Result<()> {
        self.repo.delete_field(profile_id, key)
    }
}
