//! `profiles` and `fields`.

use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};

use super::{collect, encrypted, Repo};
use crate::vault::encrypt::Encrypted;
use crate::vault::error::Result;
use crate::vault::{now, FieldInfo, Profile};

/// A field as written: metadata, the length of the plaintext, the value encrypted (None when empty).
pub struct FieldRow<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub field_type: &'a str,
    pub length: i64,
    pub value: Option<Encrypted>,
}

impl Repo {
    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self.conn.prepare("SELECT id, name, kind FROM profiles ORDER BY name")?;
        let rows = stmt.query_map([], |r| Ok(Profile { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)? }))?;
        collect(rows)
    }

    pub fn profile(&self, id: i64) -> Result<Option<Profile>> {
        Ok(self
            .conn
            .query_row("SELECT id, name, kind FROM profiles WHERE id = ?1", [id], |r| {
                Ok(Profile { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)? })
            })
            .optional()?)
    }

    pub fn insert_profile(&self, name: &str, kind: &str) -> Result<i64> {
        self.conn
            .execute("INSERT INTO profiles (name, kind, created_at) VALUES (?1, ?2, ?3)", params![name, kind, now()])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_profile(&self, id: i64, name: &str, kind: &str) -> Result<()> {
        self.conn.execute("UPDATE profiles SET name = ?1, kind = ?2 WHERE id = ?3", params![name, kind, id])?;
        Ok(())
    }

    /// Its fields go with it.
    pub fn delete_profile(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn field_infos(&self, profile_id: i64) -> Result<Vec<FieldInfo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, label, type, value_length FROM fields WHERE profile_id = ?1 ORDER BY key")?;
        let rows = stmt.query_map([profile_id], |r| {
            Ok(FieldInfo { key: r.get(0)?, label: r.get(1)?, field_type: r.get(2)?, length: r.get(3)? })
        })?;
        collect(rows)
    }

    /// Field metadata of every profile at once.
    pub fn field_index(&self) -> Result<HashMap<i64, Vec<FieldInfo>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT profile_id, key, label, type, value_length FROM fields ORDER BY profile_id, key")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                FieldInfo { key: r.get(1)?, label: r.get(2)?, field_type: r.get(3)?, length: r.get(4)? },
            ))
        })?;
        let mut out: HashMap<i64, Vec<FieldInfo>> = HashMap::new();
        for row in rows {
            let (id, info) = row?;
            out.entry(id).or_default().push(info);
        }
        Ok(out)
    }

    /// key, type and the encrypted value (None when empty) of each field, by key.
    pub fn encrypted_fields(&self, profile_id: i64) -> Result<Vec<(String, String, Option<Encrypted>)>> {
        let mut stmt =
            self.conn.prepare("SELECT key, type, nonce, ciphertext FROM fields WHERE profile_id = ?1 ORDER BY key")?;
        let rows = stmt.query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?, encrypted(r.get(2)?, r.get(3)?))))?;
        collect(rows)
    }

    /// Adds field definitions (key, label, type) that are not there yet; existing ones and values stay.
    pub fn define_fields(&self, profile_id: i64, fields: &[(String, String, String)]) -> Result<()> {
        for (key, label, field_type) in fields {
            self.conn.execute(
                "INSERT INTO fields (profile_id, key, label, type, value_length, updated_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)
                 ON CONFLICT (profile_id, key) DO NOTHING",
                params![profile_id, key, label, field_type, now()],
            )?;
        }
        Ok(())
    }

    /// Writes the fields in one transaction, replacing rows with the same key.
    pub fn replace_fields(&mut self, profile_id: i64, fields: &[FieldRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for field in fields {
            let (nonce, ciphertext) = match &field.value {
                Some(e) => (Some(&e.nonce), Some(&e.ciphertext)),
                None => (None, None),
            };
            tx.execute(
                "REPLACE INTO fields (profile_id, key, label, type, value_length, nonce, ciphertext, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![profile_id, field.key, field.label, field.field_type, field.length, nonce, ciphertext, now()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_field(&self, profile_id: i64, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM fields WHERE profile_id = ?1 AND key = ?2", params![profile_id, key])?;
        Ok(())
    }
}
