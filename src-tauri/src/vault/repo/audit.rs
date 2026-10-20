//! `audit`: plaintext actions, no values.

use rusqlite::params;

use super::{collect, Repo};
use crate::vault::error::Result;
use crate::vault::now;

impl Repo {
    pub fn insert_audit(&self, actor: &str, action: &str, target: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit (ts, actor, action, target) VALUES (?1, ?2, ?3, ?4)",
            params![now(), actor, action, target],
        )?;
        Ok(())
    }

    /// (month `YYYY-MM`, action, count) for the given actions since `since` (a timestamp prefix).
    pub fn audit_counts(&self, since: &str, actions: &[&str]) -> Result<Vec<(String, String, u64)>> {
        let placeholders = (0..actions.len()).map(|i| format!("?{}", i + 2)).collect::<Vec<_>>().join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT substr(ts, 1, 7), action, count(*) FROM audit WHERE ts >= ?1 AND action IN ({placeholders}) GROUP BY 1, 2"
        ))?;
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&since];
        values.extend(actions.iter().map(|action| action as &dyn rusqlite::ToSql));
        let rows = stmt.query_map(values.as_slice(), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        collect(rows)
    }

    /// (action, target) of every entry, oldest first.
    #[cfg(test)]
    pub fn audit_log(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT action, target FROM audit ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        collect(rows)
    }
}
