//! The user's own folders of forms and model contracts, indexed in place in `library.db`. Blankey never writes to
//! them; a file becomes a template only when the assistant templatizes it (`lib:<id>` sources).

pub mod categorize;
pub mod extract;
mod scan;
pub mod watch;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub use scan::ScanReport;

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE IF NOT EXISTS "roots" ("id" INTEGER NOT NULL PRIMARY KEY, "path" TEXT NOT NULL UNIQUE, "added_at" TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS "files" ("id" INTEGER NOT NULL PRIMARY KEY, "root_id" INTEGER NOT NULL REFERENCES "roots" ("id") ON DELETE CASCADE, "rel_path" TEXT NOT NULL, "kind" TEXT NOT NULL, "size" INTEGER NOT NULL, "mtime" INTEGER NOT NULL, "sha256" TEXT NOT NULL, "title" TEXT NOT NULL, "pages" INTEGER, "fields" INTEGER NOT NULL, "blanks" INTEGER NOT NULL, "readiness" TEXT NOT NULL, "error" TEXT, "category" TEXT, "tags" TEXT NOT NULL, "indexed_at" TEXT NOT NULL, "missing_at" TEXT, UNIQUE ("root_id", "rel_path"));
CREATE INDEX IF NOT EXISTS "files_sha256" ON "files" ("sha256");
"#];

pub const UNREADABLE: &str = "unreadable";

#[derive(Serialize, Clone, Debug)]
pub struct Root {
    pub id: i64,
    pub path: String,
    pub files: i64,
    pub added_at: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct LibraryFile {
    pub id: i64,
    pub root_id: i64,
    /// relative to the root, '/' separated
    pub rel_path: String,
    #[serde(skip)]
    pub path: PathBuf,
    /// docx | pdf
    pub kind: String,
    pub size: i64,
    #[serde(skip)]
    pub sha256: String,
    pub title: String,
    pub pages: Option<i64>,
    pub fields: i64,
    pub blanks: i64,
    /// ready | blanks | needs_ai | scan | unreadable
    pub readiness: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub missing: bool,
}

impl LibraryFile {
    pub fn source(&self) -> String {
        format!("lib:{}", self.id)
    }

    /// The file name, or the title when the document has one.
    pub fn name(&self) -> &str {
        if self.title.is_empty() {
            self.rel_path.rsplit('/').next().unwrap_or(&self.rel_path)
        } else {
            &self.title
        }
    }
}

/// The file id in a `lib:<id>` source.
pub fn parse_source(source: &str) -> Option<i64> {
    source.strip_prefix("lib:")?.parse().ok()
}

pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(data_encoding::HEXLOWER.encode(&hasher.finalize()))
}

pub struct Library {
    conn: Mutex<Connection>,
    /// held for a whole scan, so scans never overlap; the time of the last full one
    scanned: Mutex<Option<Instant>>,
    /// bumped when a root is added or removed, so a watch knows to follow
    generation: AtomicU64,
}

const FILE_COLUMNS: &str =
    "f.id, f.root_id, f.rel_path, f.kind, f.size, f.sha256, f.title, f.pages, f.fields, f.blanks, \
                            f.readiness, f.category, f.tags, f.missing_at IS NOT NULL, r.path";

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

fn string(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Library {
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
        Ok(Self { conn: Mutex::new(conn), scanned: Mutex::new(None), generation: AtomicU64::new(0) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("library connection")
    }

    pub fn roots(&self) -> Result<Vec<Root>, String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT r.id, r.path, r.added_at, (SELECT COUNT(*) FROM files f WHERE f.root_id = r.id AND f.missing_at IS NULL) \
                 FROM roots r ORDER BY r.path",
            )
            .map_err(string)?;
        let rows = stmt
            .query_map([], |r| Ok(Root { id: r.get(0)?, path: r.get(1)?, added_at: r.get(2)?, files: r.get(3)? }))
            .map_err(string)?;
        rows.collect::<rusqlite::Result<_>>().map_err(string)
    }

    /// Adds a folder; refuses one that is inside an existing root or contains one. Call `scan` afterwards.
    pub fn add_root(&self, path: &Path) -> Result<i64, String> {
        let path = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        if !path.is_dir() {
            return Err(format!("{} is not a folder", path.display()));
        }
        for root in self.roots()? {
            let existing = Path::new(&root.path);
            if path.starts_with(existing) || existing.starts_with(&path) {
                return Err(format!("{} overlaps the folder {} already added", path.display(), root.path));
            }
        }
        let conn = self.conn();
        conn.execute(
            "INSERT INTO roots (path, added_at) VALUES (?1, ?2)",
            params![path.to_string_lossy(), crate::vault::now()],
        )
        .map_err(string)?;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(conn.last_insert_rowid())
    }

    pub fn remove_root(&self, id: i64) -> Result<(), String> {
        self.conn().execute("DELETE FROM roots WHERE id = ?1", [id]).map_err(string)?;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

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

    pub fn file(&self, id: i64) -> Result<LibraryFile, String> {
        let sql = format!("SELECT {FILE_COLUMNS} FROM files f JOIN roots r ON r.id = f.root_id WHERE f.id = ?1");
        self.conn()
            .query_row(&sql, [id], file_row)
            .optional()
            .map_err(string)?
            .ok_or_else(|| format!("Unknown library file lib:{id}"))
    }

    /// The path behind a `lib:<id>` source, when the file is still there.
    pub fn path(&self, source: &str) -> Result<PathBuf, String> {
        let id = parse_source(source).ok_or_else(|| format!("Not a library file id: {source:?}"))?;
        let file = self.file(id)?;
        if file.missing {
            return Err(format!("{source} ({}) is no longer in its folder", file.rel_path));
        }
        Ok(file.path)
    }

    /// Scans every root unless the last full scan is younger than `max_age`; the report of all of them.
    pub fn sync(&self, max_age: Duration) -> Result<ScanReport, String> {
        let mut scanned = self.scanned.lock().expect("library scan");
        let mut report = ScanReport::default();
        if scanned.is_some_and(|at| at.elapsed() < max_age) {
            return Ok(report);
        }
        for root in self.roots()? {
            report.absorb(scan::scan(self, &root)?);
        }
        *scanned = Some(Instant::now());
        Ok(report)
    }

    pub fn scan_root(&self, id: i64) -> Result<ScanReport, String> {
        let _scanning = self.scanned.lock().expect("library scan");
        let root = self.roots()?.into_iter().find(|r| r.id == id).ok_or_else(|| format!("Unknown folder {id}"))?;
        scan::scan(self, &root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_must_not_overlap() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(&dir.path().join("library.db")).unwrap();
        let forms = dir.path().join("forms");
        std::fs::create_dir_all(forms.join("leases")).unwrap();
        let id = library.add_root(&forms).unwrap();
        assert!(library.add_root(&forms.join("leases")).is_err());
        assert!(library.add_root(dir.path()).is_err());
        library.remove_root(id).unwrap();
        assert!(library.roots().unwrap().is_empty());
        assert!(library.add_root(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn sources() {
        assert_eq!(parse_source("lib:12"), Some(12));
        assert_eq!(parse_source("lib:x"), None);
        assert_eq!(parse_source("file:12"), None);
    }
}
