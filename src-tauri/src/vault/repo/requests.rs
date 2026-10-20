//! `requests`. Kind, status and timestamps are plaintext; payload and result are sealed (plaintext only in rows from
//! before sealing existed, until the next unlock seals them).

use rusqlite::{params, OptionalExtension, Row};

use super::{collect, Repo};
use crate::vault::error::Result;
use crate::vault::now;

pub struct StoredRequest {
    pub id: i64,
    pub kind: String,
    pub status: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    /// legacy plaintext JSON; "{}" once sealed
    pub payload: String,
    pub result: Option<String>,
    pub sealed_payload: Option<Vec<u8>>,
    pub sealed_result: Option<Vec<u8>>,
    pub grant_id: Option<String>,
}

/// A request's payload and result in sealed form, replacing their plaintext.
pub struct SealedRequest {
    pub id: i64,
    pub payload: Vec<u8>,
    pub result: Option<Vec<u8>>,
}

const COLUMNS: &str =
    "id, kind, status, created_at, resolved_at, payload, result, sealed_payload, sealed_result, grant_id";

fn row(r: &Row) -> rusqlite::Result<StoredRequest> {
    Ok(StoredRequest {
        id: r.get(0)?,
        kind: r.get(1)?,
        status: r.get(2)?,
        created_at: r.get(3)?,
        resolved_at: r.get(4)?,
        payload: r.get(5)?,
        result: r.get(6)?,
        sealed_payload: r.get(7)?,
        sealed_result: r.get(8)?,
        grant_id: r.get(9)?,
    })
}

impl Repo {
    /// A pending request with an empty payload; seal it with `set_sealed_payload`, which needs the id.
    pub fn insert_request(&self, kind: &str, grant: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO requests (kind, payload, status, created_at, grant_id) VALUES (?1, '{}', 'pending', ?2, ?3)",
            params![kind, now(), grant],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn set_sealed_payload(&self, id: i64, sealed: &[u8]) -> Result<()> {
        self.conn.execute("UPDATE requests SET sealed_payload = ?1 WHERE id = ?2", params![sealed, id])?;
        Ok(())
    }

    pub fn request(&self, id: i64) -> Result<Option<StoredRequest>> {
        let sql = format!("SELECT {COLUMNS} FROM requests WHERE id = ?1");
        Ok(self.conn.query_row(&sql, [id], row).optional()?)
    }

    /// Oldest first.
    pub fn requests_with_status(&self, status: &str) -> Result<Vec<StoredRequest>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {COLUMNS} FROM requests WHERE status = ?1 ORDER BY id"))?;
        let rows = stmt.query_map([status], row)?;
        collect(rows)
    }

    /// Newest first.
    pub fn latest_requests(&self, limit: i64) -> Result<Vec<StoredRequest>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {COLUMNS} FROM requests ORDER BY id DESC LIMIT ?1"))?;
        let rows = stmt.query_map([limit], row)?;
        collect(rows)
    }

    pub fn count_requests(&self, status: &str) -> Result<i64> {
        Ok(self.conn.query_row("SELECT count(*) FROM requests WHERE status = ?1", [status], |r| r.get(0))?)
    }

    pub fn resolve_request(&self, id: i64, status: &str, sealed_result: &[u8]) -> Result<()> {
        self.conn.execute(
            "UPDATE requests SET status = ?1, result = NULL, sealed_result = ?2, resolved_at = ?3 WHERE id = ?4",
            params![status, sealed_result, now(), id],
        )?;
        Ok(())
    }

    /// Rows from before sealing existed: (id, payload, result) in plaintext.
    pub fn plaintext_requests(&self) -> Result<Vec<(i64, String, Option<String>)>> {
        let mut stmt = self.conn.prepare("SELECT id, payload, result FROM requests WHERE sealed_payload IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        collect(rows)
    }

    /// Replaces plaintext payloads and results with their sealed forms, in one transaction.
    pub fn seal_requests(&mut self, sealed: &[SealedRequest]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for SealedRequest { id, payload, result } in sealed {
            tx.execute(
                "UPDATE requests SET payload = '{}', result = NULL, sealed_payload = ?1, sealed_result = ?2 WHERE id = ?3",
                params![payload, result, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
