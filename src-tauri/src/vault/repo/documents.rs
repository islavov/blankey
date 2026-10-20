//! `documents` and `fill_sets`. Their ciphertext is bound to the row id, so inserts encrypt inside the transaction.

use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

use super::{collect, json, Repo};
use crate::vault::encrypt::Encrypted;
use crate::vault::error::{Result, VaultError};
use crate::vault::{now, DocumentInfo, FillSetInfo, Profiles};

/// A document to store; its content goes in through `insert_document`'s closure.
pub struct NewDocument<'a> {
    pub template_id: &'a str,
    pub title: &'a str,
    pub profiles: &'a Profiles,
    pub pages: i64,
    pub filename: &'a str,
}

fn parse_profiles(text: &str) -> Profiles {
    let raw: Map<String, Value> = serde_json::from_str(text).unwrap_or_default();
    raw.into_iter()
        .filter_map(|(role, id)| {
            let id = id.as_i64().or_else(|| id.as_str().and_then(|s| s.parse().ok()))?;
            Some((role, id))
        })
        .collect()
}

impl Repo {
    /// Inserts the row, then stores what `encrypt` makes of the new id, in one transaction.
    pub fn insert_document(&mut self, doc: &NewDocument, encrypt: impl FnOnce(i64) -> Encrypted) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO documents (template_id, title, profiles, pages, filename, created_at, nonce, ciphertext)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, x'', x'')",
            params![doc.template_id, doc.title, serde_json::to_string(doc.profiles)?, doc.pages, doc.filename, now()],
        )?;
        let id = tx.last_insert_rowid();
        let sealed = encrypt(id);
        tx.execute(
            "UPDATE documents SET nonce = ?1, ciphertext = ?2 WHERE id = ?3",
            params![sealed.nonce, sealed.ciphertext, id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Newest first.
    pub fn documents(&self) -> Result<Vec<DocumentInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, template_id, title, profiles, pages, filename, created_at FROM documents ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DocumentInfo {
                id: r.get(0)?,
                template_id: r.get(1)?,
                title: r.get(2)?,
                profiles: json(r, 3)?,
                pages: r.get(4)?,
                filename: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?;
        collect(rows)
    }

    /// (filename, encrypted content)
    pub fn document_content(&self, id: i64) -> Result<Option<(String, Encrypted)>> {
        Ok(self
            .conn
            .query_row("SELECT filename, nonce, ciphertext FROM documents WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, Encrypted { nonce: r.get(1)?, ciphertext: r.get(2)? }))
            })
            .optional()?)
    }

    pub fn delete_document(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM documents WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Inserts a fill set (`id` None) or updates one, then stores what `encrypt` makes of its id, in one
    /// transaction.
    pub fn save_fill_set(
        &mut self,
        id: Option<i64>,
        name: &str,
        templates: &[String],
        profiles: &Profiles,
        encrypt: impl FnOnce(i64) -> Encrypted,
    ) -> Result<i64> {
        let stamp = now();
        let templates = serde_json::to_string(templates)?;
        let profiles = serde_json::to_string(profiles)?;
        let tx = self.conn.transaction()?;
        let id = match id {
            None => {
                tx.execute(
                    "INSERT INTO fill_sets (name, templates, profiles, created_at, updated_at, nonce, ciphertext)
                     VALUES (?1, ?2, ?3, ?4, ?4, x'', x'')",
                    params![name, templates, profiles, stamp],
                )?;
                tx.last_insert_rowid()
            }
            Some(id) => {
                let changed = tx.execute(
                    "UPDATE fill_sets SET name = ?1, templates = ?2, profiles = ?3, updated_at = ?4 WHERE id = ?5",
                    params![name, templates, profiles, stamp, id],
                )?;
                if changed == 0 {
                    return Err(VaultError::NotFound(format!("Unknown fill set {id}")));
                }
                id
            }
        };
        let sealed = encrypt(id);
        tx.execute(
            "UPDATE fill_sets SET nonce = ?1, ciphertext = ?2 WHERE id = ?3",
            params![sealed.nonce, sealed.ciphertext, id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Newest first.
    pub fn fill_sets(&self) -> Result<Vec<FillSetInfo>> {
        let mut stmt =
            self.conn.prepare("SELECT id, name, templates, profiles, updated_at FROM fill_sets ORDER BY id DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(FillSetInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                templates: json(r, 2)?,
                profiles: parse_profiles(&r.get::<_, String>(3)?),
                updated_at: r.get(4)?,
            })
        })?;
        collect(rows)
    }

    pub fn fill_set_bindings(&self, id: i64) -> Result<Option<Encrypted>> {
        Ok(self
            .conn
            .query_row("SELECT nonce, ciphertext FROM fill_sets WHERE id = ?1", [id], |r| {
                Ok(Encrypted { nonce: r.get(0)?, ciphertext: r.get(1)? })
            })
            .optional()?)
    }

    pub fn fill_set_id(&self, name: &str) -> Result<Option<i64>> {
        Ok(self.conn.query_row("SELECT id FROM fill_sets WHERE name = ?1", [name], |r| r.get(0)).optional()?)
    }

    pub fn delete_fill_set(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM fill_sets WHERE id = ?1", [id])?;
        Ok(())
    }
}
