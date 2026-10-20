//! Encrypted profile store over the Python app's SQLite schema (user_version 3), plus later migrations.
//!
//! - `repo/`: the database: schema and every query.
//! - `encrypt.rs`: what is encrypted under which associated data.
//! - `keys.rs`: the data key and how it is wrapped (password, recovery key, keychain), the request key pair.
//! - one file per area for the operations: profiles, documents (and fill sets), requests, relay, links, audit.

mod audit;
mod documents;
mod encrypt;
mod error;
mod keys;
mod links;
mod profiles;
mod relay;
mod repo;
mod requests;
mod types;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::crypto::Key;
use crate::keychain::SecretStore;
use repo::Repo;

pub use error::{Result, VaultError};
pub use types::*;

pub struct Vault {
    path: PathBuf,
    repo: Repo,
    secrets: Arc<dyn SecretStore>,
    dek: Option<Key>,
    seal_private: Option<Key>,
    /// resolved results for the MCP client, readable while locked for the life of this process
    results: HashMap<i64, Value>,
}

impl Vault {
    pub fn open(path: &Path, secrets: Box<dyn SecretStore>) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            repo: Repo::open(path)?,
            secrets: secrets.into(),
            dek: None,
            seal_private: None,
            results: HashMap::new(),
        })
    }
}

pub fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

/// The months the relay keeps numbers for: the current one and the three before it, UTC, newest first.
pub fn kept_months() -> Vec<String> {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let (mut year, mut month) = (now.year(), now.month());
    (0..4)
        .map(|_| {
            let name = format!("{year:04}-{month:02}");
            (year, month) = if month == 1 { (year - 1, 12) } else { (year, month - 1) };
            name
        })
        .collect()
}

#[cfg(test)]
mod tests;
