//! A case's chat with the AI: its messages in order and a summary of the older ones, both encrypted under the DEK.

use serde_json::Value;

use super::encrypt;
use super::error::Result;
use super::{StoredMessage, Vault};

impl Vault {
    /// Stores the messages after the case's last one, in one transaction, and moves the case to the top of the list.
    /// Returns their seqs. Needs the vault unlocked.
    pub fn append_case_messages(&mut self, case_id: i64, messages: &[Value]) -> Result<Vec<i64>> {
        self.get_case(case_id)?;
        let dek = self.require_dek()?.clone();
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let plain = messages.iter().map(serde_json::to_vec).collect::<serde_json::Result<Vec<_>>>()?;
        self.repo
            .insert_case_messages(case_id, &plain, |message, seq| encrypt::case_message(&dek, case_id, seq, message))
    }

    /// The case's messages by seq. Needs the vault unlocked.
    pub fn case_messages(&self, case_id: i64) -> Result<Vec<StoredMessage>> {
        self.get_case(case_id)?;
        let dek = self.require_dek()?;
        self.repo
            .case_messages(case_id)?
            .into_iter()
            .map(|(seq, created_at, e)| {
                let plain = encrypt::open_case_message(dek, case_id, seq, &e.nonce, &e.ciphertext)?;
                Ok(StoredMessage { seq, created_at, message: serde_json::from_slice(&plain)? })
            })
            .collect()
    }

    /// (summary, the last seq it covers), None before the first one. Needs the vault unlocked.
    pub fn case_summary(&self, case_id: i64) -> Result<Option<(String, i64)>> {
        self.get_case(case_id)?;
        let dek = self.require_dek()?;
        let Some((upto, e)) = self.repo.case_summary(case_id)? else { return Ok(None) };
        let plain = encrypt::open_case_summary(dek, case_id, &e.nonce, &e.ciphertext)?;
        Ok(Some((String::from_utf8_lossy(&plain).into_owned(), upto)))
    }

    /// Replaces the case's summary; `upto` is the last seq it covers. Needs the vault unlocked.
    pub fn set_case_summary(&mut self, case_id: i64, summary: &str, upto: i64) -> Result<()> {
        self.get_case(case_id)?;
        let sealed = encrypt::case_summary(self.require_dek()?, case_id, summary.as_bytes());
        self.repo.set_case_summary(case_id, upto, &sealed)
    }
}
