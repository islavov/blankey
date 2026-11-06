//! `cases` and `case_profiles`, plus the `case_id` of the rows a case holds. Titles and roles are plaintext; the
//! notes are encrypted in the case row.

use std::collections::HashMap;

use rusqlite::{params, OptionalExtension, Row};

use super::{collect, encrypted, Repo};
use crate::vault::encrypt::Encrypted;
use crate::vault::error::Result;
use crate::vault::{now, CaseInfo, CaseLink};

const CASE_COLUMNS: &str = "id, uuid, title, created_at, updated_at";

fn case_row(r: &Row) -> rusqlite::Result<CaseInfo> {
    Ok(CaseInfo { id: r.get(0)?, uuid: r.get(1)?, title: r.get(2)?, created_at: r.get(3)?, updated_at: r.get(4)? })
}

/// How many rows of each kind a case holds.
pub struct CaseCounts {
    pub files: i64,
    pub fill_sets: i64,
    pub documents: i64,
    pub requests: i64,
}

impl Repo {
    pub fn insert_case(&self, uuid: &str, title: &str) -> Result<i64> {
        let stamp = now();
        self.conn.execute(
            "INSERT INTO cases (uuid, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![uuid, title, stamp],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Latest change first.
    pub fn cases(&self) -> Result<Vec<CaseInfo>> {
        let sql = format!("SELECT {CASE_COLUMNS} FROM cases ORDER BY updated_at DESC, id DESC");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], case_row)?;
        collect(rows)
    }

    pub fn case(&self, id: i64) -> Result<Option<CaseInfo>> {
        let sql = format!("SELECT {CASE_COLUMNS} FROM cases WHERE id = ?1");
        Ok(self.conn.query_row(&sql, [id], case_row).optional()?)
    }

    pub fn case_private(&self, id: i64) -> Result<Option<Encrypted>> {
        Ok(self
            .conn
            .query_row("SELECT nonce, ciphertext FROM cases WHERE id = ?1", [id], |r| {
                Ok(encrypted(r.get(0)?, r.get(1)?))
            })
            .optional()?
            .flatten())
    }

    pub fn set_case_title(&self, id: i64, title: &str) -> Result<()> {
        self.conn.execute("UPDATE cases SET title = ?1, updated_at = ?2 WHERE id = ?3", params![title, now(), id])?;
        Ok(())
    }

    pub fn set_case_private(&self, id: i64, private: &Encrypted) -> Result<()> {
        self.conn.execute(
            "UPDATE cases SET nonce = ?1, ciphertext = ?2, updated_at = ?3 WHERE id = ?4",
            params![private.nonce, private.ciphertext, now(), id],
        )?;
        Ok(())
    }

    /// Something in the case changed: it moves to the top of the list.
    pub fn touch_case(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE cases SET updated_at = ?1 WHERE id = ?2", params![now(), id])?;
        Ok(())
    }

    /// The case, its links, files, documents and fill sets, in one transaction; its requests stay as activity
    /// without a case. Returns the ids of the files removed, whose blobs the caller deletes.
    pub fn delete_case(&mut self, id: i64) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
        let files: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM collected_files WHERE case_id = ?1")?;
            let rows = stmt.query_map([id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        tx.execute("DELETE FROM collected_files WHERE case_id = ?1", [id])?;
        tx.execute("DELETE FROM documents WHERE case_id = ?1", [id])?;
        tx.execute("DELETE FROM fill_sets WHERE case_id = ?1", [id])?;
        tx.execute("UPDATE requests SET case_id = NULL WHERE case_id = ?1", [id])?;
        tx.execute("DELETE FROM case_profiles WHERE case_id = ?1", [id])?;
        tx.execute("DELETE FROM cases WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(files)
    }

    pub fn case_counts(&self, id: i64) -> Result<CaseCounts> {
        let count = |table: &str| -> Result<i64> {
            let sql = format!("SELECT count(*) FROM {table} WHERE case_id = ?1");
            Ok(self.conn.query_row(&sql, [id], |r| r.get(0))?)
        };
        Ok(CaseCounts {
            files: count("collected_files")?,
            fill_sets: count("fill_sets")?,
            documents: count("documents")?,
            requests: count("requests")?,
        })
    }

    /// case id -> number of files, for the cases that have any.
    pub fn case_file_counts(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT case_id, count(*) FROM collected_files WHERE case_id IS NOT NULL GROUP BY case_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(collect(rows)?.into_iter().collect())
    }

    // -- profiles in cases ---------------------------------------------------------

    /// Every link, by case and in the order the profiles were added.
    pub fn case_links(&self) -> Result<Vec<CaseLink>> {
        let mut stmt = self
            .conn
            .prepare("SELECT case_id, profile_id, role FROM case_profiles ORDER BY case_id, added_at, rowid")?;
        let rows =
            stmt.query_map([], |r| Ok(CaseLink { case_id: r.get(0)?, profile_id: r.get(1)?, role: r.get(2)? }))?;
        collect(rows)
    }

    /// Links the profile, or changes its role when it is linked already.
    pub fn link_profile(&self, case_id: i64, profile_id: i64, role: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO case_profiles (case_id, profile_id, role, added_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (case_id, profile_id) DO UPDATE SET role = excluded.role",
            params![case_id, profile_id, role, now()],
        )?;
        Ok(())
    }

    pub fn unlink_profile(&self, case_id: i64, profile_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM case_profiles WHERE case_id = ?1 AND profile_id = ?2",
            params![case_id, profile_id],
        )?;
        Ok(())
    }

    /// A profile that shows only in this case, linked with its role, in one transaction.
    pub fn insert_local_profile(&mut self, name: &str, kind: &str, case_id: i64, role: &str) -> Result<i64> {
        let stamp = now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO profiles (name, kind, created_at, global) VALUES (?1, ?2, ?3, 0)",
            params![name, kind, stamp],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO case_profiles (case_id, profile_id, role, added_at) VALUES (?1, ?2, ?3, ?4)",
            params![case_id, id, role, stamp],
        )?;
        tx.execute("UPDATE cases SET updated_at = ?1 WHERE id = ?2", params![stamp, case_id])?;
        tx.commit()?;
        Ok(id)
    }
}
