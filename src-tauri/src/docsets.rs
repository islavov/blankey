//! The document config set this app holds: what its engine (`doctypes`) runs. It comes from the relay
//! (`relay/doc_configs.rs`) and is kept in a file beside the vault, so it is there on the next start. A set is used
//! for a day after the relay last confirmed it; without one no file is read as a document.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::core::Blankey;
use crate::doctypes::{Set, ENGINE};

/// How long a set is used after the relay last served or confirmed it.
const VALID_HOURS: i64 = 24;
/// A set is the text of some config files; more than this is not one.
const MAX_BYTES: usize = 5 * 1024 * 1024;

/// A set as the relay serves it.
#[derive(Serialize, Deserialize)]
struct Served {
    engine: u32,
    version: String,
    files: BTreeMap<String, String>,
}

/// The file: the set as served, and when the relay last served or confirmed it.
#[derive(Serialize, Deserialize)]
struct Stored {
    #[serde(flatten)]
    served: Served,
    checked_at: DateTime<Utc>,
}

pub struct Held {
    set: Arc<Set>,
    checked_at: DateTime<Utc>,
}

impl Held {
    fn valid(&self) -> bool {
        Utc::now() - self.checked_at < Duration::hours(VALID_HOURS)
    }
}

/// What Settings shows of the set.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DocConfigsStatus {
    /// "valid", "stale" (held, not confirmed for a day: not used) or "none"
    pub state: &'static str,
    pub version: String,
    pub checked_at: Option<DateTime<Utc>>,
    /// how many documents this app reads with the set
    pub documents: usize,
    /// how many of the set's it cannot run: they need a newer app
    pub skipped: usize,
}

fn string(e: impl ToString) -> String {
    e.to_string()
}

impl Blankey {
    fn read_stored(&self) -> Option<Stored> {
        let text = std::fs::read_to_string(self.config.doc_configs_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write_stored(&self, stored: &Stored) -> Result<(), String> {
        let path = self.config.doc_configs_path();
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(stored).map_err(string)?).map_err(string)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&temporary, &path).map_err(string)
    }

    /// Takes the set from its file, at start and after the file changed, and tells the window.
    pub fn reload_doc_set(&self) {
        let held = self.read_stored().and_then(|stored| {
            let set = Set::parse(&stored.served.version, &stored.served.files).ok()?;
            Some(Held { set: Arc::new(set), checked_at: stored.checked_at })
        });
        *self.doc_set.write().unwrap() = held;
        (self.on_doc_configs_change.read().unwrap())();
    }

    pub fn set_doc_configs_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.on_doc_configs_change.write().unwrap() = Box::new(hook);
    }

    /// The set to read documents with; None when there is none or the relay has not confirmed it for a day.
    pub fn doc_set(&self) -> Option<Arc<Set>> {
        self.doc_set.read().unwrap().as_ref().filter(|held| held.valid()).map(|held| held.set.clone())
    }

    /// The version of the set held, valid or stale: what the relay is asked about.
    pub fn doc_set_version(&self) -> Option<String> {
        self.doc_set.read().unwrap().as_ref().map(|held| held.set.version.clone())
    }

    /// Keeps a set the relay served. One this engine cannot parse is refused, and the set held stays.
    pub fn store_doc_set(&self, body: &str) -> Result<(), String> {
        if body.len() > MAX_BYTES {
            return Err("The document config set is too large".into());
        }
        let served: Served = serde_json::from_str(body).map_err(|e| format!("The document config set: {e}"))?;
        if served.engine > ENGINE {
            return Err(format!("The document config set needs engine {}", served.engine));
        }
        Set::parse(&served.version, &served.files)?;
        self.write_stored(&Stored { served, checked_at: Utc::now() })?;
        self.reload_doc_set();
        Ok(())
    }

    /// The relay confirmed the set held is the current one.
    pub fn confirm_doc_set(&self) {
        let Some(mut stored) = self.read_stored() else { return };
        stored.checked_at = Utc::now();
        if self.write_stored(&stored).is_ok() {
            self.reload_doc_set();
        }
    }

    /// The account has no set any more: signed out, or the relay refused it.
    pub fn drop_doc_set(&self) {
        let _ = std::fs::remove_file(self.config.doc_configs_path());
        self.reload_doc_set();
    }

    pub fn doc_configs_status(&self) -> DocConfigsStatus {
        match self.doc_set.read().unwrap().as_ref() {
            None => {
                DocConfigsStatus { state: "none", version: String::new(), checked_at: None, documents: 0, skipped: 0 }
            }
            Some(held) => DocConfigsStatus {
                state: if held.valid() { "valid" } else { "stale" },
                version: held.set.version.clone(),
                checked_at: Some(held.checked_at),
                documents: if held.valid() { held.set.documents.len() } else { 0 },
                skipped: held.set.skipped.len(),
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mcp::tools::tests::app as test_app;

    /// A small set as the relay would serve it, of invented documents.
    pub(crate) fn served(version: &str) -> String {
        let files = crate::doctypes::example_set_files();
        serde_json::json!({"engine": 1, "version": version, "files": files}).to_string()
    }

    #[test]
    fn a_served_set_is_kept_used_for_a_day_and_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let app = test_app(dir.path());
        let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = changes.clone();
        app.set_doc_configs_hook(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        assert_eq!(app.doc_configs_status().state, "none");
        assert!(app.doc_set().is_none());

        app.store_doc_set(&served("v1")).unwrap();
        let status = app.doc_configs_status();
        assert_eq!((status.state, status.version.as_str(), status.documents), ("valid", "v1", 2));
        assert_eq!(app.doc_set().unwrap().version, "v1");
        assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 1);

        // it is there after a restart
        app.reload_doc_set();
        assert_eq!(app.doc_set_version().as_deref(), Some("v1"));

        // not confirmed for a day: held, not used; one confirmation revives it
        let mut stored = app.read_stored().unwrap();
        stored.checked_at = Utc::now() - Duration::hours(25);
        app.write_stored(&stored).unwrap();
        app.reload_doc_set();
        assert!(app.doc_set().is_none());
        let status = app.doc_configs_status();
        assert_eq!((status.state, status.documents), ("stale", 0));
        assert_eq!(app.doc_set_version().as_deref(), Some("v1"), "the relay is still asked about it");
        app.confirm_doc_set();
        assert_eq!(app.doc_configs_status().state, "valid");

        app.drop_doc_set();
        assert_eq!(app.doc_configs_status().state, "none");
        assert!(!app.config.doc_configs_path().exists());
    }

    #[test]
    fn a_set_this_engine_cannot_use_is_refused_and_the_one_held_stays() {
        let dir = tempfile::tempdir().unwrap();
        let app = test_app(dir.path());
        app.store_doc_set(&served("v1")).unwrap();
        assert!(app.store_doc_set("not json").is_err());
        assert!(app.store_doc_set(r#"{"engine": 1, "version": "v2", "files": {}}"#).is_err(), "no set.yaml");
        let newer = served("v3").replace(r#""engine":1"#, r#""engine":99"#);
        assert!(app.store_doc_set(&newer).unwrap_err().contains("engine 99"));
        assert_eq!(app.doc_set().unwrap().version, "v1");
    }
}
