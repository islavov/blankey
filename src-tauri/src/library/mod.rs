//! The user's own folders of forms and model contracts, indexed in place in `library.db`. Blankey never writes to
//! them; a file becomes a template only when the assistant templatizes it (`lib:<id>` sources).

pub mod categorize;
pub mod extract;
mod repo;
mod scan;
pub mod watch;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use repo::Repo;
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub use scan::ScanReport;

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
    repo: Repo,
    /// held for a whole scan, so scans never overlap; the time of the last full one
    scanned: Mutex<Option<Instant>>,
    /// bumped when a root is added or removed, so a watch knows to follow
    generation: AtomicU64,
}

impl Library {
    pub fn open(path: &Path) -> Result<Self, String> {
        Ok(Self { repo: Repo::open(path)?, scanned: Mutex::new(None), generation: AtomicU64::new(0) })
    }

    pub fn roots(&self) -> Result<Vec<Root>, String> {
        self.repo.roots()
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
        let id = self.repo.insert_root(&path.to_string_lossy())?;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(id)
    }

    pub fn remove_root(&self, id: i64) -> Result<(), String> {
        self.repo.delete_root(id)?;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Files on disk (not missing), by root and path.
    pub fn files(&self) -> Result<Vec<LibraryFile>, String> {
        self.repo.files()
    }

    pub fn file(&self, id: i64) -> Result<LibraryFile, String> {
        self.repo.file(id)?.ok_or_else(|| format!("Unknown library file lib:{id}"))
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
