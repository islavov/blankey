//! `case_messages` and `case_summaries`: a case's chat. Each message's ciphertext is bound to its seq, so appends
//! number the messages and encrypt them inside the transaction.

use rusqlite::{params, OptionalExtension};

use super::{collect, Repo};
use crate::vault::encrypt::Encrypted;
use crate::vault::error::Result;
use crate::vault::now;

impl Repo {
    /// Numbers the messages after the case's last seq, stores what `encrypt` makes of each with its seq and moves
    /// the case to the top of the list, in one transaction. Returns the new seqs.
    pub fn insert_case_messages<T>(
        &mut self,
        case_id: i64,
        messages: &[T],
        encrypt: impl Fn(&T, i64) -> Encrypted,
    ) -> Result<Vec<i64>> {
        let stamp = now();
        let tx = self.conn.transaction()?;
        let last: i64 =
            tx.query_row("SELECT coalesce(max(seq), 0) FROM case_messages WHERE case_id = ?1", [case_id], |r| {
                r.get(0)
            })?;
        let mut seqs = Vec::with_capacity(messages.len());
        for (seq, message) in (last + 1..).zip(messages) {
            let sealed = encrypt(message, seq);
            tx.execute(
                "INSERT INTO case_messages (case_id, seq, created_at, nonce, ciphertext) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![case_id, seq, stamp, sealed.nonce, sealed.ciphertext],
            )?;
            seqs.push(seq);
        }
        tx.execute("UPDATE cases SET updated_at = ?1 WHERE id = ?2", params![stamp, case_id])?;
        tx.commit()?;
        Ok(seqs)
    }

    /// (seq, created_at, encrypted message) by seq.
    pub fn case_messages(&self, case_id: i64) -> Result<Vec<(i64, String, Encrypted)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, created_at, nonce, ciphertext FROM case_messages WHERE case_id = ?1 ORDER BY seq")?;
        let rows = stmt.query_map([case_id], |r| {
            Ok((r.get(0)?, r.get(1)?, Encrypted { nonce: r.get(2)?, ciphertext: r.get(3)? }))
        })?;
        collect(rows)
    }

    /// (upto, encrypted summary)
    pub fn case_summary(&self, case_id: i64) -> Result<Option<(i64, Encrypted)>> {
        Ok(self
            .conn
            .query_row("SELECT upto, nonce, ciphertext FROM case_summaries WHERE case_id = ?1", [case_id], |r| {
                Ok((r.get(0)?, Encrypted { nonce: r.get(1)?, ciphertext: r.get(2)? }))
            })
            .optional()?)
    }

    /// Writes the case's summary, replacing the one it had.
    pub fn set_case_summary(&self, case_id: i64, upto: i64, summary: &Encrypted) -> Result<()> {
        self.conn.execute(
            "INSERT INTO case_summaries (case_id, upto, nonce, ciphertext) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (case_id) DO UPDATE SET upto = excluded.upto, nonce = excluded.nonce,
             ciphertext = excluded.ciphertext",
            params![case_id, upto, summary.nonce, summary.ciphertext],
        )?;
        Ok(())
    }
}
