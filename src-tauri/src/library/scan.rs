//! Walking one root: new files are indexed, changed ones re-read, a vanished file whose content shows up at another
//! path is a move (same id, so templates made from it follow), the rest are marked missing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use super::categorize::categorize;
use super::extract::{extract, kind_of};
use super::repo::{FileRow, Known};
use super::{hash_file, Library, Root, UNREADABLE};

const MAX_DEPTH: usize = 24;
const SKIPPED_DIRS: &[&str] = &["_blankey_trash", "node_modules", "$RECYCLE.BIN"];

#[derive(Serialize, Debug, Default)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub missing: usize,
    /// ids whose content changed or that went missing
    #[serde(skip)]
    pub changed: Vec<i64>,
}

impl ScanReport {
    pub fn absorb(&mut self, other: ScanReport) {
        self.added += other.added;
        self.updated += other.updated;
        self.moved += other.moved;
        self.missing += other.missing;
        self.changed.extend(other.changed);
    }

    /// Nothing the file list shows changed.
    pub fn is_empty(&self) -> bool {
        self.added + self.updated + self.moved + self.missing == 0 && self.changed.is_empty()
    }
}

struct OnDisk {
    rel_path: String,
    path: PathBuf,
    kind: &'static str,
    size: i64,
    mtime: i64,
}

fn walk(dir: &Path, rel: &str, depth: usize, out: &mut Vec<OnDisk>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else { continue };
        if name.starts_with('.') || name.starts_with("~$") || SKIPPED_DIRS.contains(&name.as_str()) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else { continue };
        let rel_path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let path = entry.path();
        if file_type.is_dir() {
            if depth < MAX_DEPTH {
                walk(&path, &rel_path, depth + 1, out);
            }
        } else if file_type.is_file() {
            let Some(kind) = kind_of(&path) else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            let mtime =
                meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
            out.push(OnDisk { rel_path, path, kind, size: meta.len() as i64, mtime });
        }
    }
}

pub fn scan(library: &Library, root: &Root) -> Result<ScanReport, String> {
    let mut on_disk = Vec::new();
    walk(Path::new(&root.path), "", 0, &mut on_disk);
    let repo = &library.repo;
    let known: HashMap<String, Known> = repo.known(root.id)?;
    let seen: HashSet<&str> = on_disk.iter().map(|f| f.rel_path.as_str()).collect();
    // a vanished row is where a new path with the same content moved from
    let mut vanished: HashMap<&str, &Known> = HashMap::new();
    for (rel_path, row) in &known {
        if !seen.contains(rel_path.as_str()) {
            vanished.entry(row.sha256.as_str()).or_insert(row);
        }
    }
    let mut report = ScanReport::default();
    let mut moved_ids = HashSet::new();
    for file in &on_disk {
        match known.get(&file.rel_path) {
            Some(row) if row.size == file.size && row.mtime == file.mtime => {
                if row.missing {
                    repo.set_missing(row.id, false)?;
                    report.changed.push(row.id);
                }
            }
            Some(row) => {
                let sha256 = hash_file(&file.path).map_err(|e| e.to_string())?;
                store(library, root.id, file, &sha256)?;
                report.updated += 1;
                if sha256 != row.sha256 || row.missing {
                    report.changed.push(row.id);
                }
            }
            None => {
                let sha256 = hash_file(&file.path).map_err(|e| e.to_string())?;
                match vanished.remove(sha256.as_str()) {
                    Some(row) => {
                        let (category, tags) = categorize(&file.rel_path);
                        repo.move_file(row.id, &file.rel_path, file.size, file.mtime, category.as_deref(), &tags)?;
                        moved_ids.insert(row.id);
                        report.moved += 1;
                        if row.missing {
                            report.changed.push(row.id);
                        }
                    }
                    None => {
                        store(library, root.id, file, &sha256)?;
                        report.added += 1;
                    }
                }
            }
        }
    }
    for (rel_path, row) in &known {
        if !seen.contains(rel_path.as_str()) && !row.missing && !moved_ids.contains(&row.id) {
            repo.set_missing(row.id, true)?;
            report.missing += 1;
            report.changed.push(row.id);
        }
    }
    Ok(report)
}

/// Reads the file's metadata (outside the connection lock) and writes its row, keeping the id of an existing one.
fn store(library: &Library, root_id: i64, file: &OnDisk, sha256: &str) -> Result<(), String> {
    let (category, tags) = categorize(&file.rel_path);
    let (meta, error) = match extract(&file.path, file.kind) {
        Ok(meta) => (meta, None),
        Err(e) => (Default::default(), Some(e)),
    };
    library.repo.upsert_file(
        root_id,
        &FileRow {
            rel_path: &file.rel_path,
            kind: file.kind,
            size: file.size,
            mtime: file.mtime,
            sha256,
            title: &meta.title,
            pages: meta.pages,
            fields: meta.fields,
            blanks: meta.blanks,
            readiness: if error.is_some() { UNREADABLE } else { meta.readiness },
            error: error.as_deref(),
            category: category.as_deref(),
            tags: &tags,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
    }

    #[test]
    fn add_change_move_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(&dir.path().join("library.db")).unwrap();
        let forms = dir.path().join("Forms");
        std::fs::create_dir_all(forms.join("Leases")).unwrap();
        std::fs::create_dir_all(forms.join(".hidden")).unwrap();
        std::fs::copy(fixture("filled.docx"), forms.join("Leases/Lease agreement.docx")).unwrap();
        std::fs::copy(fixture("form.pdf"), forms.join("Application.pdf")).unwrap();
        std::fs::copy(fixture("form.pdf"), forms.join(".hidden/skip.pdf")).unwrap();
        std::fs::write(forms.join("notes.txt"), "not indexed").unwrap();
        std::fs::write(forms.join("~$Lease agreement.docx"), "lock file").unwrap();
        let root = library.add_root(&forms).unwrap();

        let report = library.scan_root(root).unwrap();
        assert_eq!(report.added, 2, "{report:?}");
        let files = library.files().unwrap();
        let lease = files.iter().find(|f| f.rel_path == "Leases/Lease agreement.docx").unwrap();
        assert_eq!((lease.category.as_deref(), lease.tags.clone()), (Some("lease"), vec!["leases".to_string()]));
        assert_eq!(lease.readiness, "blanks");
        let form = files.iter().find(|f| f.rel_path == "Application.pdf").unwrap();
        assert_eq!((form.readiness.as_str(), form.category.as_deref()), ("ready", Some("application")));

        let again = library.scan_root(root).unwrap();
        assert_eq!((again.added, again.updated, again.changed.len()), (0, 0, 0));

        std::fs::write(forms.join("Application.pdf"), b"not a pdf any more").unwrap();
        let changed = library.scan_root(root).unwrap();
        assert_eq!((changed.updated, changed.changed.clone()), (1, vec![form.id]));
        assert_eq!(library.file(form.id).unwrap().readiness, UNREADABLE);

        std::fs::create_dir_all(forms.join("Archive")).unwrap();
        std::fs::rename(forms.join("Leases/Lease agreement.docx"), forms.join("Archive/Old lease.docx")).unwrap();
        let moved = library.scan_root(root).unwrap();
        assert_eq!((moved.moved, moved.missing), (1, 0), "{moved:?}");
        let lease_now = library.file(lease.id).unwrap();
        assert_eq!(lease_now.rel_path, "Archive/Old lease.docx");
        assert!(library.path(&lease.source()).unwrap().ends_with("Archive/Old lease.docx"));

        std::fs::remove_file(forms.join("Archive/Old lease.docx")).unwrap();
        let gone = library.scan_root(root).unwrap();
        assert_eq!((gone.missing, gone.changed.clone()), (1, vec![lease.id]));
        assert!(library.path(&lease.source()).is_err());
        assert_eq!(library.files().unwrap().len(), 1);
    }
}
