//! The vault schema: the Python app's tables (user_version 3) plus the steps added since.

use rusqlite::Connection;

use super::super::{Result, VaultError};

/// Schema steps; the step at index i brings a vault to user_version i + 1, the numbering the Python app uses.
/// Every step must be idempotent (IF NOT EXISTS, add_column), so re-running one after a partial upgrade is safe.
/// Append only: a shipped step never changes. New vaults run every step, so they match upgraded ones.
pub const MIGRATIONS: &[fn(&Connection) -> rusqlite::Result<()>] = &[
    initial_schema,
    fill_sets,
    sealed_requests,
    relay_grants,
    request_grants,
    collection_links,
    link_submissions,
    field_keys,
];

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

/// Finding profiles by the keys they hold.
fn field_keys(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(r#"CREATE INDEX IF NOT EXISTS "fieldrow_key" ON "fields" ("key");"#)
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

pub fn migrate(conn: &mut Connection) -> Result<()> {
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
