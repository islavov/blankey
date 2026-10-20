//! Keeps the library index current: the OS file watcher (FSEvents, ReadDirectoryChangesW, inotify through `notify`)
//! triggers a rescan shortly after a change, and a poll covers what it misses (network shares, sync clients, a
//! platform without a watcher). The owner only says which folders to watch and how to rescan them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

/// Quiet time after the last event before rescanning: editors and sync clients write a file in several steps.
const DEBOUNCE: Duration = Duration::from_millis(300);
const TICK: Duration = Duration::from_millis(100);

pub trait Folders: Send + Sync + 'static {
    /// The folders to watch, recursively.
    fn roots(&self) -> Vec<PathBuf>;
    /// Changes whenever `roots` would return something else.
    fn generation(&self) -> u64;
    /// `changed`: files changed on disk, scan now; otherwise only if the last scan is older than the poll.
    fn rescan(&self, changed: bool);
}

/// Starts the watch on its own thread for the life of the app.
pub fn start(folders: Arc<dyn Folders>, poll: Duration) {
    std::thread::spawn(move || run(folders.as_ref(), poll));
}

fn run(folders: &dyn Folders, poll: Duration) {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .map_err(|e| eprintln!("library: no file watcher, polling only: {e}"))
    .ok();
    let mut watched: HashSet<PathBuf> = HashSet::new();
    let mut generation = None;
    let mut pending: Option<Instant> = None;
    let mut last_scan = Instant::now();
    folders.rescan(false);
    loop {
        if let Some(watcher) = watcher.as_mut().filter(|_| generation != Some(folders.generation())) {
            generation = Some(folders.generation());
            follow(watcher, &mut watched, folders.roots());
        }
        match rx.recv_timeout(TICK) {
            Ok(Ok(event)) if relevant(&event) => {
                pending.get_or_insert_with(Instant::now);
            }
            Ok(Err(e)) => eprintln!("library: watch error: {e}"),
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(TICK),
            _ => {}
        }
        if pending.is_some_and(|since| since.elapsed() >= DEBOUNCE) {
            // drain what arrived meanwhile: the scan sees it anyway
            while rx.try_recv().is_ok() {}
            pending = None;
            folders.rescan(true);
            last_scan = Instant::now();
        } else if last_scan.elapsed() >= poll {
            folders.rescan(false);
            last_scan = Instant::now();
        }
    }
}

/// Watches exactly `roots`: stops on removed ones, starts on new ones.
fn follow(watcher: &mut impl Watcher, watched: &mut HashSet<PathBuf>, roots: Vec<PathBuf>) {
    let wanted: HashSet<PathBuf> = roots.into_iter().collect();
    for gone in watched.difference(&wanted) {
        let _ = watcher.unwatch(gone);
    }
    watched.retain(|path| wanted.contains(path));
    for root in wanted {
        if watched.contains(&root) {
            continue;
        }
        match watcher.watch(&root, RecursiveMode::Recursive) {
            Ok(()) => {
                watched.insert(root);
            }
            Err(e) => eprintln!("library: cannot watch {}, polling it: {e}", root.display()),
        }
    }
}

/// Changes that can alter the index: not reads, not hidden or Office lock files, only folders and pdf / docx.
fn relevant(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.need_rescan() || event.paths.is_empty() || event.paths.iter().any(|p| indexed_path(p))
}

fn indexed_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    if name.starts_with('.') || name.starts_with("~$") {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        None => true,
        Some(extension) => matches!(extension.to_lowercase().as_str(), "pdf" | "docx"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        roots: Mutex<Vec<PathBuf>>,
        generation: AtomicU64,
        changed: AtomicUsize,
    }

    impl Folders for Fake {
        fn roots(&self) -> Vec<PathBuf> {
            self.roots.lock().unwrap().clone()
        }
        fn generation(&self) -> u64 {
            self.generation.load(Ordering::SeqCst)
        }
        fn rescan(&self, changed: bool) {
            if changed {
                self.changed.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    fn wait_for(fake: &Fake, count: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if fake.changed.load(Ordering::SeqCst) >= count {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn a_new_file_triggers_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let fake = Arc::new(Fake::default());
        *fake.roots.lock().unwrap() = vec![root.clone()];
        start(fake.clone(), Duration::from_secs(3600));
        // the watch starts asynchronously; keep touching until it reports
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut n = 0;
        while fake.changed.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::fs::create_dir_all(root.join("Leases")).unwrap();
            std::fs::write(root.join(format!("Leases/lease-{n}.docx")), b"x").unwrap();
            n += 1;
            std::thread::sleep(Duration::from_millis(400));
        }
        assert!(wait_for(&fake, 1), "no rescan after a change");
    }

    #[test]
    fn only_indexed_paths_count() {
        assert!(indexed_path(Path::new("/f/Lease.DOCX")));
        assert!(indexed_path(Path::new("/f/Leases")));
        assert!(!indexed_path(Path::new("/f/~$Lease.docx")));
        assert!(!indexed_path(Path::new("/f/.DS_Store")));
        assert!(!indexed_path(Path::new("/f/notes.txt")));
    }
}
