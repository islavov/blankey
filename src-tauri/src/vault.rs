//! Encrypted profile store over the Python app's SQLite schema (user_version 3), plus later migrations.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::crypto::{self, KdfParams, Key};
use crate::keychain::SecretStore;

const REQUEST_PUBLIC_KEY: &str = "request_public_key";
const REQUEST_PRIVATE_KEY: &str = "request_private_key";
const RELAY_DEVICE_ID: &str = "relay_device_id";
const RELAY_CLIENT: &str = "relay_client";
const RELAY_ACCOUNT: &str = "relay_account";
const RELAY_LANGUAGE: &str = "relay_language";
/// Audit actions the app reports to the relay as monthly numbers, and the kind each is reported as.
const REPORTED_ACTIONS: [(&str, &str); 4] = [
    ("generate", "doc_generated"),
    ("save_template", "template_saved"),
    ("save_fill_set", "fill_set_saved"),
    ("create_profile", "profile_created"),
];
const BACKUPS: usize = 3;

/// Schema steps; the step at index i brings a vault to user_version i + 1, the numbering the Python app uses.
/// Every step must be idempotent (IF NOT EXISTS, add_column), so re-running one after a partial upgrade is safe.
/// Append only: a shipped step never changes. New vaults run every step, so they match upgraded ones.
const MIGRATIONS: &[fn(&Connection) -> rusqlite::Result<()>] =
    &[initial_schema, fill_sets, sealed_requests, relay_grants, request_grants, collection_links, link_submissions];

/// The Python app's DDL (peewee): NOT NULL without SQL defaults, so every insert names all its columns.
fn initial_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS "audit" ("id" INTEGER NOT NULL PRIMARY KEY, "ts" TEXT NOT NULL, "actor" TEXT NOT NULL, "action" TEXT NOT NULL, "target" TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS "documents" ("id" INTEGER NOT NULL PRIMARY KEY, "template_id" TEXT NOT NULL, "title" TEXT NOT NULL, "profiles" TEXT NOT NULL, "pages" INTEGER NOT NULL, "filename" TEXT NOT NULL, "created_at" TEXT NOT NULL, "nonce" BLOB NOT NULL, "ciphertext" BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS "profiles" ("id" INTEGER NOT NULL PRIMARY KEY, "name" TEXT NOT NULL, "kind" TEXT NOT NULL, "created_at" TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS "fields" ("profile_id" INTEGER NOT NULL, "key" TEXT NOT NULL, "label" TEXT NOT NULL, "type" TEXT NOT NULL, "value_length" INTEGER NOT NULL, "nonce" BLOB, "ciphertext" BLOB, "updated_at" TEXT NOT NULL, PRIMARY KEY ("profile_id", "key"), FOREIGN KEY ("profile_id") REFERENCES "profiles" ("id") ON DELETE CASCADE);
CREATE INDEX IF NOT EXISTS "fieldrow_profile_id" ON "fields" ("profile_id");
CREATE TABLE IF NOT EXISTS "requests" ("id" INTEGER NOT NULL PRIMARY KEY, "kind" TEXT NOT NULL, "payload" TEXT NOT NULL, "status" TEXT NOT NULL, "result" TEXT, "created_at" TEXT NOT NULL, "resolved_at" TEXT);
CREATE TABLE IF NOT EXISTS "meta" ("key" TEXT NOT NULL PRIMARY KEY, "value" BLOB NOT NULL);
"#,
    )
}

fn fill_sets(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS "fill_sets" ("id" INTEGER NOT NULL PRIMARY KEY, "name" TEXT NOT NULL, "templates" TEXT NOT NULL, "profiles" TEXT NOT NULL, "created_at" TEXT NOT NULL, "updated_at" TEXT NOT NULL, "nonce" BLOB NOT NULL, "ciphertext" BLOB NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS "fillsetrow_name" ON "fill_sets" ("name");
"#,
    )
}

fn sealed_requests(conn: &Connection) -> rusqlite::Result<()> {
    add_column(conn, "requests", "sealed_payload", "BLOB")?;
    add_column(conn, "requests", "sealed_result", "BLOB")
}

/// Web agents the user accepted through a relay; plaintext, so the relay client works while the vault is locked.
fn relay_grants(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS "relay_grants" ("grant_id" TEXT NOT NULL PRIMARY KEY, "relay_url" TEXT NOT NULL, "client_name" TEXT NOT NULL, "redirect_host" TEXT NOT NULL, "accepted_at" TEXT NOT NULL, "revoked_at" TEXT);
"#,
    )
}

/// The relay grant of the agent that made a request; NULL for local clients.
fn request_grants(conn: &Connection) -> rusqlite::Result<()> {
    add_column(conn, "requests", "grant_id", "TEXT")
}

/// Links are plaintext (their spec is public on the relay); the private key of each is a secret. Collected files
/// keep their bytes in `files/<id>.bin` under a per-file key, and their name, type and key here under the DEK.
fn collection_links(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS "links" ("link_id" TEXT NOT NULL PRIMARY KEY, "relay_url" TEXT NOT NULL, "url" TEXT NOT NULL, "title" TEXT NOT NULL, "spec" TEXT NOT NULL, "profile_id" INTEGER, "new_profile" TEXT, "created_at" TEXT NOT NULL, "expires_at" TEXT NOT NULL, "max_submissions" INTEGER NOT NULL, "revoked_at" TEXT, "received" INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS "collected_files" ("id" INTEGER NOT NULL PRIMARY KEY, "profile_id" INTEGER, "link_id" TEXT NOT NULL, "size" INTEGER NOT NULL, "created_at" TEXT NOT NULL, "nonce" BLOB NOT NULL, "ciphertext" BLOB NOT NULL, FOREIGN KEY ("profile_id") REFERENCES "profiles" ("id") ON DELETE SET NULL);
"#,
    )
}

/// Each submission pulled from the relay, kept so it can be opened again; plaintext metadata, the blob stays
/// sealed to its link's key in `inbox/<sid>.bin`.
fn link_submissions(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS "link_submissions" ("sid" TEXT NOT NULL PRIMARY KEY, "link_id" TEXT NOT NULL, "request_id" INTEGER, "size" INTEGER NOT NULL, "received_at" TEXT NOT NULL, "status" TEXT NOT NULL, "handled_at" TEXT, "profile_id" INTEGER);
CREATE INDEX IF NOT EXISTS "link_submissions_link_id" ON "link_submissions" ("link_id");
"#,
    )
}

fn add_column(conn: &Connection, table: &str, column: &str, declaration: &str) -> rusqlite::Result<()> {
    let exists: bool = conn.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1)"),
        [column],
        |r| r.get(0),
    )?;
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"))?;
    }
    Ok(())
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > MIGRATIONS.len() as i64 {
        return Err(VaultError::Schema(version));
    }
    for (index, step) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        step(&tx)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("Vault is locked")]
    Locked,
    #[error("Wrong password or key")]
    WrongSecret,
    #[error("Vault is not initialized")]
    NotInitialized,
    #[error("Vault is already initialized")]
    AlreadyInitialized,
    #[error("Unlock Blankey once to enable requests from AI assistants")]
    NoRequestKey,
    #[error("{0}")]
    NotFound(String),
    #[error("Vault schema version {0} is newer than this app supports")]
    Schema(i64),
    #[error("Keychain: {0}")]
    Keychain(String),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Crypto(#[from] crypto::CryptoError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, VaultError>;

#[derive(Serialize, Clone, Debug)]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub kind: String,
}

/// Metadata only: safe for MCP.
#[derive(Serialize, Clone, Debug)]
pub struct FieldInfo {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub length: i64,
}

#[derive(Serialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub length: i64,
    pub value: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FieldInput {
    pub key: String,
    pub value: String,
    pub label: String,
    pub field_type: String,
}

impl FieldInput {
    pub fn new(key: &str, value: &str, label: &str, field_type: &str) -> Self {
        Self { key: key.into(), value: value.into(), label: label.into(), field_type: field_type.into() }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct DocumentInfo {
    pub id: i64,
    pub template_id: String,
    pub title: String,
    pub profiles: Map<String, Value>,
    pub pages: i64,
    pub filename: String,
    pub created_at: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct FillSetInfo {
    pub id: i64,
    pub name: String,
    pub templates: Vec<String>,
    pub profiles: Profiles,
    pub updated_at: String,
}

/// role -> profile id
pub type Profiles = indexmap::IndexMap<String, i64>;
/// var -> {"path": "role.key"} | {"value": "..."} | {"default": true}
pub type Bindings = Map<String, Value>;

#[derive(Serialize, Clone, Debug)]
pub struct Request {
    pub id: i64,
    pub kind: String,
    pub payload: Map<String, Value>,
    pub status: String,
    pub result: Option<Value>,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub grant_id: Option<String>,
    /// payload sealed and the vault is locked
    pub hidden: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RelayGrant {
    pub grant_id: String,
    pub relay_url: String,
    pub client_name: String,
    pub redirect_host: String,
    pub accepted_at: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Link {
    pub link_id: String,
    pub relay_url: String,
    /// the short URL with the key fingerprint in its fragment
    pub url: String,
    pub title: String,
    pub spec: Value,
    pub profile_id: Option<i64>,
    /// name of the profile to create when the link has no profile_id
    pub new_profile: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub max_submissions: i64,
    pub revoked_at: Option<String>,
    /// submissions this computer has pulled from the relay
    pub received: i64,
}

/// A submission to a collection link. status: pending, saved or discarded.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SubmissionInfo {
    pub sid: String,
    pub link_id: String,
    pub request_id: Option<i64>,
    pub size: i64,
    pub received_at: String,
    pub status: String,
    pub handled_at: Option<String>,
    /// the profile it was last saved into
    pub profile_id: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CollectedFile {
    pub id: i64,
    pub profile_id: Option<i64>,
    pub link_id: String,
    pub size: i64,
    pub created_at: String,
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: String,
}

pub fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

fn field_aad(profile_id: i64, key: &str) -> Vec<u8> {
    format!("field:{profile_id}|{key}").into_bytes()
}

fn request_aad(request_id: i64, part: &str) -> Vec<u8> {
    format!("request:{request_id}|{part}").into_bytes()
}

fn collected_aad(id: i64) -> Vec<u8> {
    format!("collected:{id}").into_bytes()
}

fn collected_data_aad(id: i64) -> Vec<u8> {
    format!("collected:{id}|data").into_bytes()
}

fn link_secret(link_id: &str) -> String {
    format!("link:{link_id}")
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

pub struct Vault {
    path: PathBuf,
    conn: Connection,
    secrets: Box<dyn SecretStore>,
    dek: Option<Key>,
    seal_private: Option<Key>,
    /// resolved results for the MCP client, readable while locked for the life of this process
    results: HashMap<i64, Value>,
}

impl Vault {
    pub fn open(path: &Path, secrets: Box<dyn SecretStore>) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "foreign_keys", 1)?;
        migrate(&mut conn)?;
        Ok(Self { path: path.to_path_buf(), conn, secrets, dek: None, seal_private: None, results: HashMap::new() })
    }

    // -- key management ----------------------------------------------------

    pub fn initialized(&self) -> bool {
        self.meta("kdf").ok().flatten().is_some()
    }

    pub fn unlocked(&self) -> bool {
        self.dek.is_some()
    }

    /// Creates the data key and returns the one-time recovery key.
    pub fn initialize(&mut self, password: &str, use_keyring: bool) -> Result<String> {
        if self.initialized() {
            return Err(VaultError::AlreadyInitialized);
        }
        let dek = crypto::new_key();
        let recovery_key = crypto::new_recovery_key();
        self.set_password(&dek, password)?;
        let params = KdfParams::new();
        self.set_meta("recovery_kdf", &serde_json::to_vec(&params)?)?;
        let kek = crypto::derive_key(&recovery_key, &params)?;
        self.set_meta("wrapped_recovery", &crypto::wrap_key(kek.as_ref(), dek.as_ref()))?;
        self.dek = Some(dek);
        self.after_unlock()?;
        if use_keyring {
            self.enable_keyring()?;
        }
        Ok(recovery_key)
    }

    pub fn unlock(&mut self, password: &str) -> Result<()> {
        let params = self.load_kdf("kdf")?;
        let kek = crypto::derive_key(password, &params)?;
        self.unwrap_dek(&kek, "wrapped_password")
    }

    pub fn unlock_with_recovery(&mut self, recovery_key: &str, new_password: &str) -> Result<()> {
        let params = self.load_kdf("recovery_kdf")?;
        let kek = crypto::derive_key(&crypto::normalize_recovery_key(recovery_key), &params)?;
        self.unwrap_dek(&kek, "wrapped_recovery")?;
        self.change_password(new_password)
    }

    /// False when there is no keychain key or it does not open the vault.
    pub fn unlock_with_keyring(&mut self) -> bool {
        let Ok(Some(stored)) = self.secrets.get() else { return false };
        if self.meta("wrapped_keyring").ok().flatten().is_none() {
            return false;
        }
        let Ok(kek) = crypto::decode_b64(&stored) else { return false };
        let Ok(kek) = crypto::key_from(&kek) else { return false };
        self.unwrap_dek(&kek, "wrapped_keyring").is_ok()
    }

    pub fn enable_keyring(&mut self) -> Result<()> {
        let dek = self.require_dek()?.clone();
        let kek = crypto::new_key();
        self.secrets.set(&crypto::encode_b64(kek.as_ref())).map_err(VaultError::Keychain)?;
        self.set_meta("wrapped_keyring", &crypto::wrap_key(kek.as_ref(), dek.as_ref()))
    }

    pub fn disable_keyring(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM meta WHERE key = 'wrapped_keyring'", [])?;
        let _ = self.secrets.delete();
        Ok(())
    }

    pub fn keyring_enabled(&self) -> bool {
        self.meta("wrapped_keyring").ok().flatten().is_some()
    }

    pub fn change_password(&mut self, new_password: &str) -> Result<()> {
        let dek = self.require_dek()?.clone();
        self.set_password(&dek, new_password)
    }

    pub fn lock(&mut self) {
        self.dek = None;
        self.seal_private = None;
    }

    fn set_password(&self, dek: &Key, password: &str) -> Result<()> {
        let params = KdfParams::new();
        let kek = crypto::derive_key(password, &params)?;
        self.set_meta("kdf", &serde_json::to_vec(&params)?)?;
        self.set_meta("wrapped_password", &crypto::wrap_key(kek.as_ref(), dek.as_ref()))
    }

    fn unwrap_dek(&mut self, kek: &Key, meta_key: &str) -> Result<()> {
        let wrapped = self.meta(meta_key)?.ok_or(VaultError::WrongSecret)?;
        let dek = crypto::unwrap_key(kek.as_ref(), &wrapped).map_err(|_| VaultError::WrongSecret)?;
        self.dek = Some(dek);
        self.after_unlock()
    }

    fn require_dek(&self) -> Result<&Key> {
        self.dek.as_ref().ok_or(VaultError::Locked)
    }

    fn load_kdf(&self, key: &str) -> Result<KdfParams> {
        let raw = self.meta(key)?.ok_or(VaultError::NotInitialized)?;
        Ok(serde_json::from_slice(&raw)?)
    }

    fn meta(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    fn set_meta(&self, key: &str, value: &[u8]) -> Result<()> {
        self.conn.execute("REPLACE INTO meta (key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }

    fn get_secret(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(raw) = self.meta(&format!("secret:{name}"))? else { return Ok(None) };
        let aad = format!("secret:{name}");
        Ok(Some(crypto::decrypt_joined(self.require_dek()?.as_ref(), &raw, aad.as_bytes())?))
    }

    fn put_secret(&self, name: &str, value: &[u8]) -> Result<()> {
        let aad = format!("secret:{name}");
        let blob = crypto::encrypt_joined(self.require_dek()?.as_ref(), value, aad.as_bytes());
        self.set_meta(&format!("secret:{name}"), &blob)
    }

    /// Creates the request key pair on first unlock, then seals requests stored before sealing existed.
    fn after_unlock(&mut self) -> Result<()> {
        let private = match self.get_secret(REQUEST_PRIVATE_KEY)? {
            Some(bytes) => crypto::key_from(&bytes)?,
            None => {
                let (private, public) = crypto::new_seal_keypair();
                self.put_secret(REQUEST_PRIVATE_KEY, private.as_ref())?;
                self.set_meta(REQUEST_PUBLIC_KEY, &public)?;
                private
            }
        };
        self.seal_private = Some(private);
        let public = self.meta(REQUEST_PUBLIC_KEY)?.ok_or(VaultError::NoRequestKey)?;
        let legacy: Vec<(i64, String, Option<String>)> = {
            let mut stmt =
                self.conn.prepare("SELECT id, payload, result FROM requests WHERE sealed_payload IS NULL")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if legacy.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        for (id, payload, result) in legacy {
            let sealed_payload = crypto::seal(&public, payload.as_bytes(), &request_aad(id, "payload"))?;
            let sealed_result = match result {
                Some(r) if !r.is_empty() => Some(crypto::seal(&public, r.as_bytes(), &request_aad(id, "result"))?),
                _ => None,
            };
            tx.execute(
                "UPDATE requests SET payload = '{}', result = NULL, sealed_payload = ?1, sealed_result = ?2 WHERE id = ?3",
                params![sealed_payload, sealed_result, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // -- profiles ----------------------------------------------------------

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self.conn.prepare("SELECT id, name, kind FROM profiles ORDER BY name")?;
        let rows = stmt.query_map([], |r| Ok(Profile { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_profile(&self, id: i64) -> Result<Profile> {
        self.conn
            .query_row("SELECT id, name, kind FROM profiles WHERE id = ?1", [id], |r| {
                Ok(Profile { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)? })
            })
            .optional()?
            .ok_or_else(|| VaultError::NotFound(format!("Unknown profile {id}")))
    }

    pub fn create_profile(&self, name: &str, kind: &str) -> Result<i64> {
        self.conn
            .execute("INSERT INTO profiles (name, kind, created_at) VALUES (?1, ?2, ?3)", params![name, kind, now()])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_profile(&self, id: i64, name: &str, kind: &str) -> Result<()> {
        self.conn.execute("UPDATE profiles SET name = ?1, kind = ?2 WHERE id = ?3", params![name, kind, id])?;
        Ok(())
    }

    pub fn delete_profile(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn describe(&self, profile_id: i64) -> Result<Vec<FieldInfo>> {
        self.get_profile(profile_id)?;
        let mut stmt = self
            .conn
            .prepare("SELECT key, label, type, value_length FROM fields WHERE profile_id = ?1 ORDER BY key")?;
        let rows = stmt.query_map([profile_id], |r| {
            Ok(FieldInfo { key: r.get(0)?, label: r.get(1)?, field_type: r.get(2)?, length: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Field metadata, plus decrypted values when unlocked.
    pub fn fields(&self, profile_id: i64) -> Result<Vec<Field>> {
        let values = if self.unlocked() { Some(self.get_values(profile_id)?) } else { None };
        Ok(self
            .describe(profile_id)?
            .into_iter()
            .map(|info| {
                let value =
                    values.as_ref().map(|v| v.get(&info.key).map(|(_, value)| value.clone()).unwrap_or_default());
                Field { key: info.key, label: info.label, field_type: info.field_type, length: info.length, value }
            })
            .collect())
    }

    /// key -> (type, plaintext)
    pub fn get_values(&self, profile_id: i64) -> Result<indexmap::IndexMap<String, (String, String)>> {
        let dek = self.require_dek()?;
        let mut stmt =
            self.conn.prepare("SELECT key, type, nonce, ciphertext FROM fields WHERE profile_id = ?1 ORDER BY key")?;
        type Row = (String, String, Option<Vec<u8>>, Option<Vec<u8>>);
        let rows: Vec<Row> = stmt
            .query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = indexmap::IndexMap::new();
        for (key, field_type, nonce, ct) in rows {
            let plain = match (nonce, ct) {
                (Some(nonce), Some(ct)) => {
                    let bytes = crypto::decrypt(dek.as_ref(), &nonce, &ct, &field_aad(profile_id, &key))?;
                    String::from_utf8_lossy(&bytes).into_owned()
                }
                _ => String::new(),
            };
            out.insert(key, (field_type, plain));
        }
        Ok(out)
    }

    /// Upserts field definitions (key, label, type) without touching values.
    pub fn define_fields(&self, profile_id: i64, fields: &[(String, String, String)]) -> Result<()> {
        for (key, label, field_type) in fields {
            self.conn.execute(
                "INSERT INTO fields (profile_id, key, label, type, value_length, updated_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)
                 ON CONFLICT (profile_id, key) DO NOTHING",
                params![profile_id, key, label, field_type, now()],
            )?;
        }
        Ok(())
    }

    pub fn set_values(&mut self, profile_id: i64, values: &[FieldInput]) -> Result<()> {
        let dek = self.require_dek()?.clone();
        let tx = self.conn.transaction()?;
        for item in values {
            let value: String = item.value.nfc().collect();
            let (nonce, ct) = if value.is_empty() {
                (None, None)
            } else {
                let (n, c) = crypto::encrypt(dek.as_ref(), value.as_bytes(), &field_aad(profile_id, &item.key));
                (Some(n), Some(c))
            };
            tx.execute(
                "REPLACE INTO fields (profile_id, key, label, type, value_length, nonce, ciphertext, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    profile_id,
                    item.key,
                    item.label,
                    item.field_type,
                    value.chars().count() as i64,
                    nonce,
                    ct,
                    now()
                ],
            )?;
        }
        tx.commit()?;
        self.backup()
    }

    pub fn delete_field(&self, profile_id: i64, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM fields WHERE profile_id = ?1 AND key = ?2", params![profile_id, key])?;
        Ok(())
    }

    // -- documents ---------------------------------------------------------

    pub fn store_document(
        &mut self,
        template_id: &str,
        title: &str,
        profiles: &Profiles,
        pages: i64,
        filename: &str,
        content: &[u8],
    ) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO documents (template_id, title, profiles, pages, filename, created_at, nonce, ciphertext)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, x'', x'')",
            params![template_id, title, serde_json::to_string(profiles)?, pages, filename, now()],
        )?;
        let id = tx.last_insert_rowid();
        let (nonce, ct) = crypto::encrypt(dek.as_ref(), content, format!("document:{id}").as_bytes());
        tx.execute("UPDATE documents SET nonce = ?1, ciphertext = ?2 WHERE id = ?3", params![nonce, ct, id])?;
        tx.commit()?;
        Ok(id)
    }

    pub fn list_documents(&self) -> Result<Vec<DocumentInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, template_id, title, profiles, pages, filename, created_at FROM documents ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DocumentInfo {
                id: r.get(0)?,
                template_id: r.get(1)?,
                title: r.get(2)?,
                profiles: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                pages: r.get(4)?,
                filename: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// (filename, content)
    pub fn load_document(&self, id: i64) -> Result<(String, Vec<u8>)> {
        let dek = self.require_dek()?;
        let (filename, nonce, ct): (String, Vec<u8>, Vec<u8>) = self
            .conn
            .query_row("SELECT filename, nonce, ciphertext FROM documents WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?
            .ok_or_else(|| VaultError::NotFound(format!("Unknown document {id}")))?;
        let content = crypto::decrypt(dek.as_ref(), &nonce, &ct, format!("document:{id}").as_bytes())?;
        Ok((filename, content))
    }

    pub fn delete_document(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM documents WHERE id = ?1", [id])?;
        Ok(())
    }

    // -- fill sets ---------------------------------------------------------
    // bindings are encrypted because literal values may be personal data

    pub fn save_fill_set(
        &mut self,
        name: &str,
        templates: &[String],
        profiles: &Profiles,
        bindings: &Bindings,
        fill_set_id: Option<i64>,
    ) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let stamp = now();
        let templates_json = serde_json::to_string(templates)?;
        let profiles_json = serde_json::to_string(profiles)?;
        let tx = self.conn.transaction()?;
        let id = match fill_set_id {
            None => {
                tx.execute(
                    "INSERT INTO fill_sets (name, templates, profiles, created_at, updated_at, nonce, ciphertext)
                     VALUES (?1, ?2, ?3, ?4, ?4, x'', x'')",
                    params![name, templates_json, profiles_json, stamp],
                )?;
                tx.last_insert_rowid()
            }
            Some(id) => {
                let changed = tx.execute(
                    "UPDATE fill_sets SET name = ?1, templates = ?2, profiles = ?3, updated_at = ?4 WHERE id = ?5",
                    params![name, templates_json, profiles_json, stamp, id],
                )?;
                if changed == 0 {
                    return Err(VaultError::NotFound(format!("Unknown fill set {id}")));
                }
                id
            }
        };
        let plain = serde_json::to_vec(bindings)?;
        let (nonce, ct) = crypto::encrypt(dek.as_ref(), &plain, format!("fillset:{id}").as_bytes());
        tx.execute("UPDATE fill_sets SET nonce = ?1, ciphertext = ?2 WHERE id = ?3", params![nonce, ct, id])?;
        tx.commit()?;
        Ok(id)
    }

    pub fn list_fill_sets(&self) -> Result<Vec<FillSetInfo>> {
        let mut stmt =
            self.conn.prepare("SELECT id, name, templates, profiles, updated_at FROM fill_sets ORDER BY id DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(FillSetInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                templates: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                profiles: parse_profiles(&r.get::<_, String>(3)?),
                updated_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_fill_set(&self, id: i64) -> Result<(FillSetInfo, Bindings)> {
        let dek = self.require_dek()?;
        let info = self
            .list_fill_sets()?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or_else(|| VaultError::NotFound(format!("Unknown fill set {id}")))?;
        let (nonce, ct): (Vec<u8>, Vec<u8>) =
            self.conn.query_row("SELECT nonce, ciphertext FROM fill_sets WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        let plain = crypto::decrypt(dek.as_ref(), &nonce, &ct, format!("fillset:{id}").as_bytes())?;
        Ok((info, serde_json::from_slice(&plain)?))
    }

    pub fn find_fill_set(&self, name: &str) -> Result<Option<i64>> {
        Ok(self.conn.query_row("SELECT id FROM fill_sets WHERE name = ?1", [name], |r| r.get(0)).optional()?)
    }

    pub fn delete_fill_set(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM fill_sets WHERE id = ?1", [id])?;
        Ok(())
    }

    // -- requests ----------------------------------------------------------
    // Payloads and results are sealed to the request public key, so a request can be stored while the vault is
    // locked and only an unlocked vault can read it. Kind, status and timestamps stay readable.

    pub fn create_request(&self, kind: &str, payload: &Value, grant: Option<&str>) -> Result<i64> {
        let public = self.meta(REQUEST_PUBLIC_KEY)?.ok_or(VaultError::NoRequestKey)?;
        self.conn.execute(
            "INSERT INTO requests (kind, payload, status, created_at, grant_id) VALUES (?1, '{}', 'pending', ?2, ?3)",
            params![kind, now(), grant],
        )?;
        let id = self.conn.last_insert_rowid();
        let sealed = crypto::seal(&public, &serde_json::to_vec(payload)?, &request_aad(id, "payload"))?;
        self.conn.execute("UPDATE requests SET sealed_payload = ?1 WHERE id = ?2", params![sealed, id])?;
        Ok(id)
    }

    pub fn get_request(&self, id: i64) -> Result<Request> {
        self.query_requests("WHERE id = ?1", params![id])?
            .pop()
            .ok_or_else(|| VaultError::NotFound(format!("Unknown request {id}")))
    }

    /// With a status: those requests oldest first; without: the latest 100.
    pub fn list_requests(&self, status: Option<&str>) -> Result<Vec<Request>> {
        match status {
            Some(status) => self.query_requests("WHERE status = ?1 ORDER BY id", params![status]),
            None => self.query_requests("ORDER BY id DESC LIMIT 100", params![]),
        }
    }

    pub fn count_requests(&self, status: &str) -> Result<i64> {
        Ok(self.conn.query_row("SELECT count(*) FROM requests WHERE status = ?1", [status], |r| r.get(0))?)
    }

    pub fn resolve_request(&mut self, id: i64, status: &str, result: Value) -> Result<()> {
        let public = self.meta(REQUEST_PUBLIC_KEY)?.ok_or(VaultError::NoRequestKey)?;
        let sealed = crypto::seal(&public, &serde_json::to_vec(&result)?, &request_aad(id, "result"))?;
        self.conn.execute(
            "UPDATE requests SET status = ?1, result = NULL, sealed_result = ?2, resolved_at = ?3 WHERE id = ?4",
            params![status, sealed, now(), id],
        )?;
        self.results.insert(id, result);
        Ok(())
    }

    fn query_requests(&self, clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Request>> {
        type Row = (
            i64,
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
            Option<String>,
        );
        let sql = format!(
            "SELECT id, kind, status, created_at, resolved_at, payload, result, sealed_payload, sealed_result, grant_id
             FROM requests {clause}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<Row> = stmt
            .query_map(args, |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let private = self.seal_private.as_ref();
        let open = |id: i64, sealed: &[u8], part: &str| -> Result<Value> {
            let plain = crypto::unseal(private.ok_or(VaultError::Locked)?.as_ref(), sealed, &request_aad(id, part))?;
            Ok(serde_json::from_slice(&plain)?)
        };
        let mut out = Vec::new();
        for (id, kind, status, created_at, resolved_at, payload, result, sealed_payload, sealed_result, grant_id) in
            rows
        {
            let mut hidden = false;
            let payload = match &sealed_payload {
                Some(_) if private.is_none() => {
                    hidden = true;
                    Value::Object(Map::new())
                }
                Some(sealed) => open(id, sealed, "payload")?,
                None => serde_json::from_str(&payload).unwrap_or_default(),
            };
            let result = match (self.results.get(&id), &sealed_result) {
                (Some(cached), _) => Some(cached.clone()),
                (None, Some(sealed)) if private.is_some() => Some(open(id, sealed, "result")?),
                (None, Some(_)) => None,
                (None, None) => result.and_then(|r| serde_json::from_str(&r).ok()),
            };
            let payload = match payload {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            out.push(Request { id, kind, payload, status, result, created_at, resolved_at, grant_id, hidden });
        }
        Ok(out)
    }

    // -- relay -------------------------------------------------------------
    // Plaintext metadata: the relay client runs while the vault is locked.

    pub fn add_relay_grant(&self, grant: &RelayGrant) -> Result<()> {
        self.conn.execute(
            "REPLACE INTO relay_grants (grant_id, relay_url, client_name, redirect_host, accepted_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![grant.grant_id, grant.relay_url, grant.client_name, grant.redirect_host, grant.accepted_at],
        )?;
        Ok(())
    }

    /// Grants on this relay that are not revoked, newest first.
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
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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

    pub fn relay_device_id(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_DEVICE_ID)
    }

    pub fn set_relay_device_id(&self, device_id: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_DEVICE_ID, device_id)
    }

    /// The account the app is signed in to on the relay, as the relay names it (an email address).
    pub fn relay_account(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_ACCOUNT)
    }

    pub fn set_relay_account(&self, account: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_ACCOUNT, account)
    }

    /// The UI language of the relay account, as its device profile last named it.
    pub fn relay_language(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_LANGUAGE)
    }

    pub fn set_relay_language(&self, language: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_LANGUAGE, language)
    }

    /// The app's own OAuth client registration on the relay (JSON).
    pub fn relay_client(&self) -> Result<Option<String>> {
        self.text_meta(RELAY_CLIENT)
    }

    pub fn set_relay_client(&self, client: Option<&str>) -> Result<()> {
        self.set_text_meta(RELAY_CLIENT, client)
    }

    fn text_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.meta(key)?.map(|v| String::from_utf8_lossy(&v).into_owned()))
    }

    fn set_text_meta(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => self.set_meta(key, value.as_bytes()),
            None => {
                self.conn.execute("DELETE FROM meta WHERE key = ?1", [key])?;
                Ok(())
            }
        }
    }

    // -- collection links --------------------------------------------------

    /// Stores the link and its private key; needs the vault unlocked.
    pub fn add_link(&self, link: &Link, private: &[u8]) -> Result<()> {
        self.require_dek()?;
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
        self.put_secret(&link_secret(&link.link_id), private)
    }

    /// Newest first.
    pub fn list_links(&self) -> Result<Vec<Link>> {
        self.query_links("ORDER BY created_at DESC", params![])
    }

    pub fn get_link(&self, link_id: &str) -> Result<Link> {
        self.query_links("WHERE link_id = ?1", params![link_id])?
            .pop()
            .ok_or_else(|| VaultError::NotFound(format!("Unknown link {link_id}")))
    }

    fn query_links(&self, clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Link>> {
        let sql = format!(
            "SELECT link_id, relay_url, url, title, spec, profile_id, new_profile, created_at, expires_at,
             max_submissions, revoked_at, received FROM links {clause}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(args, |r| {
            Ok(Link {
                link_id: r.get(0)?,
                relay_url: r.get(1)?,
                url: r.get(2)?,
                title: r.get(3)?,
                spec: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                profile_id: r.get(5)?,
                new_profile: r.get(6)?,
                created_at: r.get(7)?,
                expires_at: r.get(8)?,
                max_submissions: r.get(9)?,
                revoked_at: r.get(10)?,
                received: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count_received(&self, link_id: &str) -> Result<()> {
        self.conn.execute("UPDATE links SET received = received + 1 WHERE link_id = ?1", [link_id])?;
        Ok(())
    }

    /// The link's private key; None once the link is revoked.
    pub fn link_key(&self, link_id: &str) -> Result<Option<Key>> {
        self.get_secret(&link_secret(link_id))?.map(|bytes| Ok(crypto::key_from(&bytes)?)).transpose()
    }

    /// The key stays, so submissions already received still open; anything pulled for the link later is dropped.
    pub fn revoke_link(&self, link_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE links SET revoked_at = ?1 WHERE link_id = ?2 AND revoked_at IS NULL",
            params![now(), link_id],
        )?;
        Ok(())
    }

    /// Removes the link, its key and its submission rows; files already kept stay. The caller deletes the blobs.
    pub fn delete_link(&mut self, link_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM link_submissions WHERE link_id = ?1", [link_id])?;
        tx.execute("DELETE FROM links WHERE link_id = ?1", [link_id])?;
        tx.execute("DELETE FROM meta WHERE key = ?1", [format!("secret:{}", link_secret(link_id))])?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_submission(&self, sid: &str, link_id: &str, request_id: i64, size: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO link_submissions (sid, link_id, request_id, size, received_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
            params![sid, link_id, request_id, size, now()],
        )?;
        Ok(())
    }

    /// Newest first; all links when `link_id` is None.
    pub fn list_submissions(&self, link_id: Option<&str>) -> Result<Vec<SubmissionInfo>> {
        match link_id {
            Some(id) => self.query_submissions("WHERE link_id = ?1 ORDER BY received_at DESC, rowid DESC", params![id]),
            None => self.query_submissions("ORDER BY received_at DESC, rowid DESC", params![]),
        }
    }

    pub fn get_submission(&self, sid: &str) -> Result<SubmissionInfo> {
        self.query_submissions("WHERE sid = ?1", params![sid])?
            .pop()
            .ok_or_else(|| VaultError::NotFound(format!("Unknown submission {sid}")))
    }

    fn query_submissions(&self, clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<SubmissionInfo>> {
        let sql = format!(
            "SELECT sid, link_id, request_id, size, received_at, status, handled_at, profile_id
             FROM link_submissions {clause}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(args, |r| {
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
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_submission_status(&self, sid: &str, status: &str, profile_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE link_submissions SET status = ?1, handled_at = ?2, profile_id = coalesce(?3, profile_id)
             WHERE sid = ?4",
            params![status, now(), profile_id, sid],
        )?;
        Ok(())
    }

    fn files_dir(&self) -> PathBuf {
        self.path.with_file_name("files")
    }

    fn collected_path(&self, id: i64) -> PathBuf {
        self.files_dir().join(format!("{id}.bin"))
    }

    /// Encrypts the bytes under a fresh key into `files/<id>.bin`; the name, type and key go in the row.
    pub fn add_collected_file(
        &mut self,
        profile_id: Option<i64>,
        link_id: &str,
        name: &str,
        file_type: &str,
        content: &[u8],
    ) -> Result<i64> {
        let dek = self.require_dek()?.clone();
        let dir = self.files_dir();
        std::fs::create_dir_all(&dir)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO collected_files (profile_id, link_id, size, created_at, nonce, ciphertext)
             VALUES (?1, ?2, ?3, ?4, x'', x'')",
            params![profile_id, link_id, content.len() as i64, now()],
        )?;
        let id = tx.last_insert_rowid();
        let key = crypto::new_key();
        let meta = serde_json::json!({"name": name, "type": file_type, "key": crypto::encode_b64(key.as_ref())});
        let (nonce, ct) = crypto::encrypt(dek.as_ref(), &serde_json::to_vec(&meta)?, &collected_aad(id));
        tx.execute("UPDATE collected_files SET nonce = ?1, ciphertext = ?2 WHERE id = ?3", params![nonce, ct, id])?;
        std::fs::write(
            dir.join(format!("{id}.bin")),
            crypto::encrypt_joined(key.as_ref(), content, &collected_data_aad(id)),
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Newest first; needs the vault unlocked for the names.
    pub fn list_collected_files(&self) -> Result<Vec<CollectedFile>> {
        Ok(self.query_collected("ORDER BY id DESC", params![])?.into_iter().map(|(file, _)| file).collect())
    }

    fn query_collected(&self, clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<(CollectedFile, Key)>> {
        let dek = self.require_dek()?;
        let sql = format!(
            "SELECT id, profile_id, link_id, size, created_at, nonce, ciphertext FROM collected_files {clause}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        type Row = (i64, Option<i64>, String, i64, String, Vec<u8>, Vec<u8>);
        let rows: Vec<Row> = stmt
            .query_map(args, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(id, profile_id, link_id, size, created_at, nonce, ct)| {
                let meta: Value =
                    serde_json::from_slice(&crypto::decrypt(dek.as_ref(), &nonce, &ct, &collected_aad(id))?)?;
                let text = |name: &str| meta.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
                let key = crypto::key_from(&crypto::decode_b64(&text("key"))?)?;
                let file = CollectedFile {
                    id,
                    profile_id,
                    link_id,
                    size,
                    created_at,
                    name: text("name"),
                    file_type: text("type"),
                };
                Ok((file, key))
            })
            .collect()
    }

    pub fn load_collected_file(&self, id: i64) -> Result<(CollectedFile, Vec<u8>)> {
        let (file, key) = self
            .query_collected("WHERE id = ?1", params![id])?
            .pop()
            .ok_or_else(|| VaultError::NotFound(format!("Unknown file {id}")))?;
        let blob = std::fs::read(self.collected_path(id))?;
        let content = crypto::decrypt_joined(key.as_ref(), &blob, &collected_data_aad(id))?;
        Ok((file, content))
    }

    pub fn delete_collected_file(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM collected_files WHERE id = ?1", [id])?;
        let _ = std::fs::remove_file(self.collected_path(id));
        Ok(())
    }

    // -- audit & maintenance -----------------------------------------------

    pub fn audit(&self, actor: &str, action: &str, target: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit (ts, actor, action, target) VALUES (?1, ?2, ?3, ?4)",
            params![now(), actor, action, target],
        )?;
        Ok(())
    }

    /// How often each reported action happened per month (`YYYY-MM`, UTC) since the first of `months`'s oldest one,
    /// every month and kind present with a zero when nothing happened. Plain metadata, so it works while locked.
    pub fn usage_months(&self, months: &[String]) -> Result<BTreeMap<String, BTreeMap<&'static str, u64>>> {
        let mut report: BTreeMap<String, BTreeMap<&'static str, u64>> = months
            .iter()
            .map(|month| (month.clone(), REPORTED_ACTIONS.iter().map(|(_, kind)| (*kind, 0)).collect()))
            .collect();
        let Some(oldest) = months.iter().min() else { return Ok(report) };
        let placeholders =
            REPORTED_ACTIONS.iter().enumerate().map(|(i, _)| format!("?{}", i + 2)).collect::<Vec<_>>().join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT substr(ts, 1, 7), action, count(*) FROM audit WHERE ts >= ?1 AND action IN ({placeholders}) GROUP BY 1, 2"
        ))?;
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![oldest];
        values.extend(REPORTED_ACTIONS.iter().map(|(action, _)| action as &dyn rusqlite::ToSql));
        let rows = stmt.query_map(values.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u64>(2)?))
        })?;
        for row in rows {
            let (month, action, n) = row?;
            let Some(counts) = report.get_mut(&month) else { continue };
            if let Some((_, kind)) = REPORTED_ACTIONS.iter().find(|(a, _)| *a == action) {
                counts.insert(kind, n);
            }
        }
        Ok(report)
    }

    /// Rotates vault.db.1 .. vault.db.3 and writes a fresh copy to vault.db.1.
    pub fn backup(&self) -> Result<()> {
        let name = self.path.file_name().and_then(|n| n.to_str()).unwrap_or("vault.db").to_string();
        let numbered = |i: usize| self.path.with_file_name(format!("{name}.{i}"));
        for i in (2..=BACKUPS).rev() {
            if numbered(i - 1).exists() {
                let _ = std::fs::rename(numbered(i - 1), numbered(i));
            }
        }
        let mut target = Connection::open(numbered(1))?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut target)?;
        backup.run_to_completion(64, std::time::Duration::ZERO, None)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::MemoryStore;
    use serde_json::json;

    pub const PASSWORD: &str = "correct horse battery staple";

    fn vault(dir: &Path) -> Vault {
        let mut vault = Vault::open(&dir.join("vault.db"), Box::new(MemoryStore::default())).unwrap();
        vault.initialize(PASSWORD, false).unwrap();
        vault
    }

    fn input(key: &str, value: &str) -> FieldInput {
        FieldInput::new(key, value, "", "text")
    }

    #[test]
    fn values_are_encrypted_at_rest() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        let pid = vault.create_profile("Иван", "person").unwrap();
        vault.set_values(pid, &[input("name", "Sentinel Hiddenwell")]).unwrap();
        let raw: Vec<u8> = vault.conn().query_row("SELECT ciphertext FROM fields", [], |r| r.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Sentinel"));
        let db = std::fs::read(dir.path().join("vault.db")).unwrap();
        assert!(!String::from_utf8_lossy(&db).contains("Sentinel"));
    }

    #[test]
    fn lock_unlock_metadata_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
        let recovery = vault.initialize("pw", false).unwrap();
        let pid = vault.create_profile("Иван", "person").unwrap();
        vault.set_values(pid, &[input("name", "Иван Иванов"), input("empty", "")]).unwrap();
        vault.lock();
        drop(vault);

        let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
        let locked = vault.fields(pid).unwrap();
        assert!(locked.iter().all(|f| f.value.is_none()));
        assert_eq!(locked.iter().find(|f| f.key == "name").unwrap().length, 11);
        assert!(matches!(vault.get_values(pid), Err(VaultError::Locked)));
        assert!(matches!(vault.unlock("nope"), Err(VaultError::WrongSecret)));
        vault.unlock_with_recovery(&recovery.to_lowercase(), "pw2").unwrap();
        vault.lock();
        assert!(vault.unlock("pw").is_err());
        vault.unlock("pw2").unwrap();
        assert_eq!(vault.get_values(pid).unwrap()["name"].1, "Иван Иванов");
    }

    #[test]
    fn length_counts_nfc_characters() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        let pid = vault.create_profile("x", "").unwrap();
        vault.set_values(pid, &[input("name", "Йордан")]).unwrap();
        vault.set_values(pid, &[input("decomposed", "Йордан".nfd().collect::<String>().as_str())]).unwrap();
        let lengths: Vec<i64> = vault.describe(pid).unwrap().iter().map(|f| f.length).collect();
        assert_eq!(lengths, [6, 6]);
    }

    #[test]
    fn keyring_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        assert!(!vault.unlock_with_keyring());
        vault.enable_keyring().unwrap();
        vault.lock();
        assert!(vault.unlock_with_keyring());
        vault.disable_keyring().unwrap();
        vault.lock();
        assert!(!vault.unlock_with_keyring());
    }

    #[test]
    fn ciphertext_cannot_be_moved_between_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        let pid = vault.create_profile("x", "").unwrap();
        vault.set_values(pid, &[input("a", "secret a"), input("b", "secret b")]).unwrap();
        vault
            .conn()
            .execute_batch(
                "UPDATE fields SET nonce = (SELECT nonce FROM fields WHERE key = 'a'),
                 ciphertext = (SELECT ciphertext FROM fields WHERE key = 'a') WHERE key = 'b'",
            )
            .unwrap();
        assert!(vault.get_values(pid).is_err());
    }

    #[test]
    fn documents_and_fill_sets_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        let profiles: Profiles = [("applicant".to_string(), 1)].into_iter().collect();
        let doc = vault.store_document("t", "Title", &profiles, 1, "t.pdf", b"%PDF-1.7").unwrap();
        assert_eq!(vault.load_document(doc).unwrap().1, b"%PDF-1.7");
        assert_eq!(vault.list_documents().unwrap()[0].profiles["applicant"], json!(1));

        let bindings: Bindings = serde_json::from_value(json!({"amount": {"value": "34 000"}})).unwrap();
        let id = vault.save_fill_set("acme", &["loan".into()], &profiles, &bindings, None).unwrap();
        assert_eq!(vault.find_fill_set("acme").unwrap(), Some(id));
        let (info, loaded) = vault.get_fill_set(id).unwrap();
        assert_eq!((info.templates, loaded), (vec!["loan".to_string()], bindings));
        vault.lock();
        assert!(vault.get_fill_set(id).is_err());
        assert_eq!(vault.list_fill_sets().unwrap().len(), 1);
    }

    #[test]
    fn backup_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        let pid = vault.create_profile("x", "").unwrap();
        for i in 0..5 {
            vault.set_values(pid, &[input("n", &i.to_string())]).unwrap();
        }
        for i in 1..=3 {
            assert!(dir.path().join(format!("vault.db.{i}")).exists());
        }
        assert!(!dir.path().join("vault.db.4").exists());
    }

    #[test]
    fn requests_are_sealed_and_need_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        vault.lock();
        let id = vault.create_request("fill", &json!({"reason": "Needed"}), None).unwrap();
        let raw: Vec<u8> = vault.conn().query_row("SELECT sealed_payload FROM requests", [], |r| r.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Needed"));
        let request = vault.get_request(id).unwrap();
        assert!(request.hidden && request.payload.is_empty() && request.status == "pending");
        vault.resolve_request(id, "done", json!({"outcome": "saved"})).unwrap();
        assert_eq!(vault.get_request(id).unwrap().result, Some(json!({"outcome": "saved"})));
        vault.unlock(PASSWORD).unwrap();
        assert_eq!(vault.get_request(id).unwrap().payload["reason"], json!("Needed"));
        assert_eq!(vault.count_requests("done").unwrap(), 1);
    }

    #[test]
    fn requests_need_a_key_pair() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(&dir.path().join("vault.db"), Box::new(MemoryStore::default())).unwrap();
        assert!(matches!(vault.create_request("fill", &json!({}), None), Err(VaultError::NoRequestKey)));
    }

    #[test]
    fn plaintext_requests_from_before_sealing_are_sealed_on_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        vault.lock();
        vault
            .conn()
            .execute(
                "INSERT INTO requests (kind, payload, status, created_at) VALUES ('fill', '{\"reason\": \"Old\"}', 'pending', 'x')",
                [],
            )
            .unwrap();
        vault.unlock(PASSWORD).unwrap();
        let payload: String = vault.conn().query_row("SELECT payload FROM requests", [], |r| r.get(0)).unwrap();
        assert_eq!(payload, "{}");
        assert_eq!(vault.list_requests(None).unwrap()[0].payload["reason"], json!("Old"));
    }

    #[test]
    fn migrations_rerun_safely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
        vault.initialize(PASSWORD, false).unwrap();
        vault.create_profile("John", "person").unwrap();
        vault.conn().pragma_update(None, "user_version", 0).unwrap();
        drop(vault);

        let vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
        let version: i64 = vault.conn().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(vault.list_profiles().unwrap()[0].name, "John");
    }

    #[test]
    fn relay_grants_work_while_locked() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(dir.path());
        vault.lock();
        let grant = |id: &str, relay: &str| RelayGrant {
            grant_id: id.into(),
            relay_url: relay.into(),
            client_name: "Claude".into(),
            redirect_host: "claude.ai".into(),
            accepted_at: now(),
        };
        vault.add_relay_grant(&grant("g-1", "https://a")).unwrap();
        vault.add_relay_grant(&grant("g-2", "https://a")).unwrap();
        vault.add_relay_grant(&grant("g-3", "https://b")).unwrap();
        assert!(vault.relay_grant_allowed("g-1", "https://a").unwrap());
        assert!(!vault.relay_grant_allowed("g-1", "https://b").unwrap());

        vault.revoke_relay_grant("g-1").unwrap();
        assert!(!vault.relay_grant_allowed("g-1", "https://a").unwrap());
        let ids: Vec<String> = vault.relay_grants("https://a").unwrap().into_iter().map(|g| g.grant_id).collect();
        assert_eq!(ids, ["g-2"]);

        vault.clear_relay_grants("https://a").unwrap();
        assert!(vault.relay_grants("https://a").unwrap().is_empty());
        assert_eq!(vault.relay_grants("https://b").unwrap().len(), 1);

        assert_eq!(vault.relay_device_id().unwrap(), None);
        vault.set_relay_device_id(Some("d-1")).unwrap();
        assert_eq!(vault.relay_device_id().unwrap().as_deref(), Some("d-1"));
        vault.set_relay_device_id(None).unwrap();
        assert_eq!(vault.relay_device_id().unwrap(), None);
    }

    /// A vault written by the Python app opens and decrypts unchanged.
    #[test]
    fn reads_python_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/vault_v3.db"), &path).unwrap();
        let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
        let profiles = vault.list_profiles().unwrap();
        assert_eq!((profiles[0].name.as_str(), profiles[0].kind.as_str()), ("John", "person"));
        let lengths: Vec<(String, i64)> = vault.describe(1).unwrap().into_iter().map(|f| (f.key, f.length)).collect();
        assert_eq!(lengths, [("egn".to_string(), 0), ("name".to_string(), 11)]);
        let requests = vault.list_requests(None).unwrap();
        assert!(requests[0].hidden && requests[0].status == "done");

        vault.unlock("legacy-pw").unwrap();
        assert_eq!(vault.get_values(1).unwrap()["name"].1, "John Watson");
        let id = vault.find_fill_set("acme").unwrap().unwrap();
        let (info, bindings) = vault.get_fill_set(id).unwrap();
        assert_eq!(info.templates, ["loan"]);
        assert_eq!(info.profiles["manager"], 1);
        assert_eq!(Value::Object(bindings), json!({"amount": {"value": "34 000"}}));
        let doc = &vault.list_documents().unwrap()[0];
        assert_eq!(doc.title, "Contract");
        assert_eq!(vault.load_document(doc.id).unwrap().1, b"%PDF-legacy");
        let request = &vault.list_requests(None).unwrap()[0];
        assert_eq!(Value::Object(request.payload.clone()), json!({"reason": "Fill in"}));
        assert_eq!(request.result, Some(json!({"outcome": "saved"})));

        // the Python schema has no default for requests.payload
        let id = vault.create_request("fill", &json!({"reason": "New"}), None).unwrap();
        assert_eq!(vault.get_request(id).unwrap().payload["reason"], json!("New"));
    }
}
