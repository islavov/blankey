//! `links`, `link_submissions` and `collected_files`. Links and submissions are plaintext (a link's spec is public
//! on the relay); a collected file's name, type and key are encrypted in its row.

use rusqlite::{params, OptionalExtension, Row};

use super::{collect, json, Repo};
use crate::vault::encrypt::Encrypted;
use crate::vault::error::Result;
use crate::vault::{now, Link, SubmissionInfo};

/// A collected file row: plaintext columns and the encrypted metadata (name, type, file key).
pub struct StoredCollected {
    pub id: i64,
    pub profile_id: Option<i64>,
    pub link_id: String,
    pub size: i64,
    pub created_at: String,
    pub meta: Encrypted,
}

const LINK_COLUMNS: &str = "link_id, relay_url, url, title, spec, profile_id, new_profile, created_at, expires_at, \
                            max_submissions, revoked_at, received";
const SUBMISSION_COLUMNS: &str = "sid, link_id, request_id, size, received_at, status, handled_at, profile_id";
const COLLECTED_COLUMNS: &str = "id, profile_id, link_id, size, created_at, nonce, ciphertext";

fn link_row(r: &Row) -> rusqlite::Result<Link> {
    Ok(Link {
        link_id: r.get(0)?,
        relay_url: r.get(1)?,
        url: r.get(2)?,
        title: r.get(3)?,
        spec: json(r, 4)?,
        profile_id: r.get(5)?,
        new_profile: r.get(6)?,
        created_at: r.get(7)?,
        expires_at: r.get(8)?,
        max_submissions: r.get(9)?,
        revoked_at: r.get(10)?,
        received: r.get(11)?,
    })
}

fn submission_row(r: &Row) -> rusqlite::Result<SubmissionInfo> {
    Ok(SubmissionInfo {
        sid: r.get(0)?,
        link_id: r.get(1)?,
        request_id: r.get(2)?,
        size: r.get(3)?,
        received_at: r.get(4)?,
        status: r.get(5)?,
        handled_at: r.get(6)?,
        profile_id: r.get(7)?,
    })
}

fn collected_row(r: &Row) -> rusqlite::Result<StoredCollected> {
    Ok(StoredCollected {
        id: r.get(0)?,
        profile_id: r.get(1)?,
        link_id: r.get(2)?,
        size: r.get(3)?,
        created_at: r.get(4)?,
        meta: Encrypted { nonce: r.get(5)?, ciphertext: r.get(6)? },
    })
}

impl Repo {
    // -- links ---------------------------------------------------------------

    pub fn insert_link(&self, link: &Link) -> Result<()> {
        self.conn.execute(
            "INSERT INTO links (link_id, relay_url, url, title, spec, profile_id, new_profile, created_at, expires_at,
             max_submissions, revoked_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            params![
                link.link_id,
                link.relay_url,
                link.url,
                link.title,
                serde_json::to_string(&link.spec)?,
                link.profile_id,
                link.new_profile,
                link.created_at,
                link.expires_at,
                link.max_submissions
            ],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn links(&self) -> Result<Vec<Link>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {LINK_COLUMNS} FROM links ORDER BY created_at DESC"))?;
        let rows = stmt.query_map([], link_row)?;
        collect(rows)
    }

    pub fn link(&self, link_id: &str) -> Result<Option<Link>> {
        let sql = format!("SELECT {LINK_COLUMNS} FROM links WHERE link_id = ?1");
        Ok(self.conn.query_row(&sql, [link_id], link_row).optional()?)
    }

    pub fn count_received(&self, link_id: &str) -> Result<()> {
        self.conn.execute("UPDATE links SET received = received + 1 WHERE link_id = ?1", [link_id])?;
        Ok(())
    }

    pub fn revoke_link(&self, link_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE links SET revoked_at = ?1 WHERE link_id = ?2 AND revoked_at IS NULL",
            params![now(), link_id],
        )?;
        Ok(())
    }

    /// The link, its submission rows and the meta entry `secret_key` (its private key), in one transaction.
    pub fn delete_link(&mut self, link_id: &str, secret_key: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM link_submissions WHERE link_id = ?1", [link_id])?;
        tx.execute("DELETE FROM links WHERE link_id = ?1", [link_id])?;
        tx.execute("DELETE FROM meta WHERE key = ?1", [secret_key])?;
        tx.commit()?;
        Ok(())
    }

    // -- submissions -----------------------------------------------------------

    pub fn insert_submission(&self, sid: &str, link_id: &str, request_id: i64, size: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO link_submissions (sid, link_id, request_id, size, received_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
            params![sid, link_id, request_id, size, now()],
        )?;
        Ok(())
    }

    /// Newest first; every link's when `link_id` is None.
    pub fn submissions(&self, link_id: Option<&str>) -> Result<Vec<SubmissionInfo>> {
        let order = "ORDER BY received_at DESC, rowid DESC";
        match link_id {
            Some(id) => {
                let sql = format!("SELECT {SUBMISSION_COLUMNS} FROM link_submissions WHERE link_id = ?1 {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([id], submission_row)?;
                collect(rows)
            }
            None => {
                let sql = format!("SELECT {SUBMISSION_COLUMNS} FROM link_submissions {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([], submission_row)?;
                collect(rows)
            }
        }
    }

    pub fn submission(&self, sid: &str) -> Result<Option<SubmissionInfo>> {
        let sql = format!("SELECT {SUBMISSION_COLUMNS} FROM link_submissions WHERE sid = ?1");
        Ok(self.conn.query_row(&sql, [sid], submission_row).optional()?)
    }

    /// Keeps the profile it was saved into when `profile_id` is None.
    pub fn set_submission_status(&self, sid: &str, status: &str, profile_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE link_submissions SET status = ?1, handled_at = ?2, profile_id = coalesce(?3, profile_id)
             WHERE sid = ?4",
            params![status, now(), profile_id, sid],
        )?;
        Ok(())
    }

    // -- collected files -----------------------------------------------------------

    /// Inserts the row, then stores the metadata `finish` encrypts for the new id; `finish` also writes the file,
    /// so a failure there rolls the row back.
    pub fn insert_collected_file(
        &mut self,
        profile_id: Option<i64>,
        link_id: &str,
        size: i64,
        finish: impl FnOnce(i64) -> Result<Encrypted>,
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO collected_files (profile_id, link_id, size, created_at, nonce, ciphertext)
             VALUES (?1, ?2, ?3, ?4, x'', x'')",
            params![profile_id, link_id, size, now()],
        )?;
        let id = tx.last_insert_rowid();
        let meta = finish(id)?;
        tx.execute(
            "UPDATE collected_files SET nonce = ?1, ciphertext = ?2 WHERE id = ?3",
            params![meta.nonce, meta.ciphertext, id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Newest first.
    pub fn collected_files(&self) -> Result<Vec<StoredCollected>> {
        let sql = format!("SELECT {COLLECTED_COLUMNS} FROM collected_files ORDER BY id DESC");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], collected_row)?;
        collect(rows)
    }

    pub fn collected_file(&self, id: i64) -> Result<Option<StoredCollected>> {
        let sql = format!("SELECT {COLLECTED_COLUMNS} FROM collected_files WHERE id = ?1");
        Ok(self.conn.query_row(&sql, [id], collected_row).optional()?)
    }

    pub fn delete_collected_file(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM collected_files WHERE id = ?1", [id])?;
        Ok(())
    }
}
