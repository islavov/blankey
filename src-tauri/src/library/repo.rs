//! Every access to `library.db`: the connection, migrations and queries. The rest of the module works with the
//! typed rows below and never sees SQL.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{LibraryFile, Root};
use crate::vault::now;

/// Schema steps; the step at index i brings a database to user_version i + 1. Append only.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE IF NOT EXISTS "roots" ("id" INTEGER NOT NULL PRIMARY KEY, "path" TEXT NOT NULL UNIQUE, "added_at" TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS "files" ("id" INTEGER NOT NULL PRIMARY KEY, "root_id" INTEGER NOT NULL REFERENCES "roots" ("id") ON DELETE CASCADE, "rel_path" TEXT NOT NULL, "kind" TEXT NOT NULL, "size" INTEGER NOT NULL, "mtime" INTEGER NOT NULL, "sha256" TEXT NOT NULL, "title" TEXT NOT NULL, "pages" INTEGER, "fields" INTEGER NOT NULL, "blanks" INTEGER NOT NULL, "readiness" TEXT NOT NULL, "error" TEXT, "category" TEXT, "tags" TEXT NOT NULL, "indexed_at" TEXT NOT NULL, "missing_at" TEXT, UNIQUE ("root_id", "rel_path"));
CREATE INDEX IF NOT EXISTS "files_sha256" ON "files" ("sha256");
"#];

const FILE_COLUMNS: &str = "f.id, f.root_id, f.rel_path, f.kind, f.size, f.sha256, f.title, f.pages, f.fields, \
                            f.blanks, f.readiness, f.category, f.tags, f.missing_at IS NOT NULL, r.path";

/// What a scan compares the disk against.
pub struct Known {
    pub id: i64,
    pub size: i64,
    pub mtime: i64,
    pub sha256: String,
    pub missing: bool,
}

/// One indexed file as a scan read it.
pub struct FileRow<'a> {
    pub rel_path: &'a str,
    pub kind: &'a str,
    pub size: i64,
    pub mtime: i64,
    pub sha256: &'a str,
    pub title: &'a str,
    pub pages: Option<i64>,
    pub fields: i64,
    pub blanks: i64,
    pub readiness: &'a str,
    pub error: Option<&'a str>,
    pub category: Option<&'a str>,
    pub tags: &'a [String],
}

fn string(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn json_tags(tags: &[String]) -> String {
    serde_json::to_string(tags).expect("tags serialize")
}

fn file_row(row: &Row) -> rusqlite::Result<LibraryFile> {
    let rel_path: String = row.get(2)?;
    let root: String = row.get(14)?;
    let tags: String = row.get(12)?;
    Ok(LibraryFile {
        id: row.get(0)?,
        root_id: row.get(1)?,
        path: rel_path.split('/').fold(PathBuf::from(root), |p, part| p.join(part)),
        rel_path,
        kind: row.get(3)?,
        size: row.get(4)?,
        sha256: row.get(5)?,
        title: row.get(6)?,
        pages: row.get(7)?,
        fields: row.get(8)?,
        blanks: row.get(9)?,
        readiness: row.get(10)?,
        category: row.get(11)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        missing: row.get(13)?,
    })
}

/// Each method locks the connection for its own statements only, so scans read and hash files without holding it.
pub struct Repo {
    conn: Mutex<Connection>,
}

impl Repo {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut conn = Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        conn.pragma_update(None, "foreign_keys", true).map_err(string)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(string)?;
        for (index, step) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
            let tx = conn.transaction().map_err(string)?;
            tx.execute_batch(step).map_err(string)?;
            tx.pragma_update(None, "user_version", index as i64 + 1).map_err(string)?;
            tx.commit().map_err(string)?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("library connection")
    }

    // -- roots -------------------------------------------------------------

    pub fn roots(&self) -> Result<Vec<Root>, String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT r.id, r.path, r.added_at, \
                 (SELECT COUNT(*) FROM files f WHERE f.root_id = r.id AND f.missing_at IS NULL) \
                 FROM roots r ORDER BY r.path",
            )
            .map_err(string)?;
        let rows = stmt
            .query_map([], |r| Ok(Root { id: r.get(0)?, path: r.get(1)?, added_at: r.get(2)?, files: r.get(3)? }))
            .map_err(string)?;
        rows.collect::<rusqlite::Result<_>>().map_err(string)
    }

    pub fn insert_root(&self, path: &str) -> Result<i64, String> {
        let conn = self.conn();
        conn.execute("INSERT INTO roots (path, added_at) VALUES (?1, ?2)", params![path, now()]).map_err(string)?;
        Ok(conn.last_insert_rowid())
    }

    /// Its files go with it.
    pub fn delete_root(&self, id: i64) -> Result<(), String> {
        self.conn().execute("DELETE FROM roots WHERE id = ?1", [id]).map_err(string)?;
        Ok(())
    }

    // -- files -------------------------------------------------------------

    /// Files on disk (not missing), by root and path.
    pub fn files(&self) -> Result<Vec<LibraryFile>, String> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {FILE_COLUMNS} FROM files f JOIN roots r ON r.id = f.root_id WHERE f.missing_at IS NULL \
             ORDER BY r.path, f.rel_path"
        );
        let mut stmt = conn.prepare(&sql).map_err(string)?;
        let rows = stmt.query_map([], file_row).map_err(string)?;
        rows.collect::<rusqlite::Result<_>>().map_err(string)
    }

    pub fn file(&self, id: i64) -> Result<Option<LibraryFile>, String> {
        let sql = format!("SELECT {FILE_COLUMNS} FROM files f JOIN roots r ON r.id = f.root_id WHERE f.id = ?1");
        self.conn().query_row(&sql, [id], file_row).optional().map_err(string)
    }

    // -- scanning ----------------------------------------------------------

    /// Every row of a root, missing ones included, by relative path.
    pub fn known(&self, root_id: i64) -> Result<HashMap<String, Known>, String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT rel_path, id, size, mtime, sha256, missing_at IS NOT NULL FROM files WHERE root_id = ?1")
            .map_err(string)?;
        let rows = stmt
            .query_map([root_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Known { id: r.get(1)?, size: r.get(2)?, mtime: r.get(3)?, sha256: r.get(4)?, missing: r.get(5)? },
                ))
            })
            .map_err(string)?;
        rows.collect::<rusqlite::Result<_>>().map_err(string)
    }

    /// Inserts the file, or updates the row at the same path and keeps its id; either way it is not missing.
    pub fn upsert_file(&self, root_id: i64, file: &FileRow) -> Result<(), String> {
        self.conn()
            .execute(
                "INSERT INTO files (root_id, rel_path, kind, size, mtime, sha256, title, pages, fields, blanks, \
                 readiness, error, category, tags, indexed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
                 ON CONFLICT (root_id, rel_path) DO UPDATE SET kind = excluded.kind, size = excluded.size, \
                 mtime = excluded.mtime, sha256 = excluded.sha256, title = excluded.title, pages = excluded.pages, \
                 fields = excluded.fields, blanks = excluded.blanks, readiness = excluded.readiness, \
                 error = excluded.error, category = excluded.category, tags = excluded.tags, \
                 indexed_at = excluded.indexed_at, missing_at = NULL",
                params![
                    root_id,
                    file.rel_path,
                    file.kind,
                    file.size,
                    file.mtime,
                    file.sha256,
                    file.title,
                    file.pages,
                    file.fields,
                    file.blanks,
                    file.readiness,
                    file.error,
                    file.category,
                    json_tags(file.tags),
                    now(),
                ],
            )
            .map_err(string)?;
        Ok(())
    }

    /// The same content found at another path: the row (and its id) follows it.
    pub fn move_file(
        &self,
        id: i64,
        rel_path: &str,
        size: i64,
        mtime: i64,
        category: Option<&str>,
        tags: &[String],
    ) -> Result<(), String> {
        self.conn()
            .execute(
                "UPDATE files SET rel_path = ?2, size = ?3, mtime = ?4, category = ?5, tags = ?6, missing_at = NULL \
                 WHERE id = ?1",
                params![id, rel_path, size, mtime, category, json_tags(tags)],
            )
            .map_err(string)?;
        Ok(())
    }

    pub fn set_missing(&self, id: i64, missing: bool) -> Result<(), String> {
        let at = missing.then(now);
        self.conn().execute("UPDATE files SET missing_at = ?2 WHERE id = ?1", params![id, at]).map_err(string)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(rel_path: &'a str, sha256: &'a str, title: &'a str, tags: &'a [String]) -> FileRow<'a> {
        FileRow {
            rel_path,
            kind: "docx",
            size: 10,
            mtime: 1,
            sha256,
            title,
            pages: None,
            fields: 0,
            blanks: 2,
            readiness: "blanks",
            error: None,
            category: Some("lease"),
            tags,
        }
    }

    #[test]
    fn files_keep_their_id_through_updates_moves_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo::open(&dir.path().join("library.db")).unwrap();
        let root = repo.insert_root("/forms").unwrap();
        let tags = vec!["leases".to_string()];
        repo.upsert_file(root, &row("Leases/a.docx", "aa", "Lease", &tags)).unwrap();
        let id = repo.known(root).unwrap()["Leases/a.docx"].id;
        repo.upsert_file(root, &row("Leases/a.docx", "bb", "Lease v2", &tags)).unwrap();
        let file = repo.file(id).unwrap().unwrap();
        assert_eq!((file.title.as_str(), file.sha256.as_str(), file.tags.clone()), ("Lease v2", "bb", tags.clone()));
        assert_eq!(file.path, Path::new("/forms/Leases/a.docx"));

        repo.set_missing(id, true).unwrap();
        assert!(repo.file(id).unwrap().unwrap().missing && repo.files().unwrap().is_empty());
        repo.move_file(id, "Archive/a.docx", 11, 2, None, &[]).unwrap();
        let moved = repo.file(id).unwrap().unwrap();
        assert_eq!((moved.rel_path.as_str(), moved.missing, moved.category), ("Archive/a.docx", false, None));
        assert_eq!(repo.roots().unwrap()[0].files, 1);

        repo.delete_root(root).unwrap();
        assert!(repo.file(id).unwrap().is_none());
    }

    /// New library code adds queries here, not next to the logic that uses them.
    #[test]
    fn only_the_repository_touches_the_database() {
        let sources = [
            ("mod.rs", include_str!("mod.rs")),
            ("scan.rs", include_str!("scan.rs")),
            ("extract.rs", include_str!("extract.rs")),
            ("categorize.rs", include_str!("categorize.rs")),
            ("watch.rs", include_str!("watch.rs")),
        ];
        let banned = ["rusqlite", ".execute(", ".prepare(", "query_row", "query_map"];
        for (name, source) in sources {
            for word in banned {
                assert!(!source.contains(word), "library/{name} uses {word}; put the query in library/repo.rs");
            }
        }
    }
}
