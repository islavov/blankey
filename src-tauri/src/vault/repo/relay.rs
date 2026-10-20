//! `relay_grants`: plaintext, so the relay client works while the vault is locked.

use rusqlite::params;

use super::{collect, Repo};
use crate::vault::error::Result;
use crate::vault::{now, RelayGrant};

impl Repo {
    /// Adds the grant, or accepts it again if it was revoked.
    pub fn insert_relay_grant(&self, grant: &RelayGrant) -> Result<()> {
        self.conn.execute(
            "REPLACE INTO relay_grants (grant_id, relay_url, client_name, redirect_host, accepted_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![grant.grant_id, grant.relay_url, grant.client_name, grant.redirect_host, grant.accepted_at],
        )?;
        Ok(())
    }

    /// Not revoked, newest first.
    pub fn relay_grants(&self, relay_url: &str) -> Result<Vec<RelayGrant>> {
        let mut stmt = self.conn.prepare(
            "SELECT grant_id, relay_url, client_name, redirect_host, accepted_at FROM relay_grants
             WHERE relay_url = ?1 AND revoked_at IS NULL ORDER BY accepted_at DESC",
        )?;
        let rows = stmt.query_map([relay_url], |r| {
            Ok(RelayGrant {
                grant_id: r.get(0)?,
                relay_url: r.get(1)?,
                client_name: r.get(2)?,
                redirect_host: r.get(3)?,
                accepted_at: r.get(4)?,
            })
        })?;
        collect(rows)
    }

    pub fn relay_grant_allowed(&self, grant_id: &str, relay_url: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM relay_grants WHERE grant_id = ?1 AND relay_url = ?2 AND revoked_at IS NULL)",
            [grant_id, relay_url],
            |r| r.get(0),
        )?)
    }

    pub fn revoke_relay_grant(&self, grant_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE relay_grants SET revoked_at = ?1 WHERE grant_id = ?2 AND revoked_at IS NULL",
            params![now(), grant_id],
        )?;
        Ok(())
    }

    pub fn clear_relay_grants(&self, relay_url: &str) -> Result<()> {
        self.conn.execute("DELETE FROM relay_grants WHERE relay_url = ?1", [relay_url])?;
        Ok(())
    }
}
