//! Every access to `vault.db`: the connection, the schema and the queries, one file per area. Rows go in and out as
//! plain data with their encrypted columns as bytes; the repository never holds a key.

mod audit;
mod documents;
mod links;
mod meta;
mod profiles;
mod relay;
mod requests;
pub mod schema;

use std::path::Path;

use rusqlite::{Connection, Row};

pub use documents::NewDocument;
pub use links::StoredCollected;
pub use profiles::FieldRow;
pub use requests::{SealedRequest, StoredRequest};

use super::encrypt::Encrypted;
use super::error::Result;

pub struct Repo {
    conn: Connection,
}

/// Two nullable columns that hold one value only together.
fn encrypted(nonce: Option<Vec<u8>>, ciphertext: Option<Vec<u8>>) -> Option<Encrypted> {
    Some(Encrypted { nonce: nonce?, ciphertext: ciphertext? })
}

fn collect<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>> {
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A JSON text column; unreadable JSON reads as the default, as it always did.
fn json<T: serde::de::DeserializeOwned + Default>(row: &Row, index: usize) -> rusqlite::Result<T> {
    Ok(serde_json::from_str(&row.get::<_, String>(index)?).unwrap_or_default())
}

impl Repo {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "foreign_keys", 1)?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// A consistent copy of the whole database at `target`.
    pub fn backup_to(&self, target: &Path) -> Result<()> {
        let mut copy = Connection::open(target)?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut copy)?;
        backup.run_to_completion(64, std::time::Duration::ZERO, None)?;
        Ok(())
    }

    /// For tests that check what is stored at rest.
    #[cfg(test)]
    pub fn raw(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    /// New vault code adds queries in repo/, not next to the logic that uses them.
    #[test]
    fn only_the_repository_touches_the_database() {
        let sources = [
            ("mod.rs", include_str!("../mod.rs")),
            ("types.rs", include_str!("../types.rs")),
            ("encrypt.rs", include_str!("../encrypt.rs")),
            ("keys.rs", include_str!("../keys.rs")),
            ("profiles.rs", include_str!("../profiles.rs")),
            ("documents.rs", include_str!("../documents.rs")),
            ("requests.rs", include_str!("../requests.rs")),
            ("relay.rs", include_str!("../relay.rs")),
            ("links.rs", include_str!("../links.rs")),
            ("audit.rs", include_str!("../audit.rs")),
        ];
        let banned = ["rusqlite", ".execute(", ".prepare(", "query_row", "query_map", "Connection"];
        for (name, source) in sources {
            for word in banned {
                assert!(!source.contains(word), "vault/{name} uses {word}; put the query in vault/repo/");
            }
        }
    }
}
