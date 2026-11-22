//! Workspaces: separate sets of data, each a data dir of its own with its own vault and master password. One is
//! open at a time; the app relaunches into another. `workspaces.json` in the root lists them and names the open one.
//!
//! The first workspace ("default") is the root dir itself, where Blankey always kept its data, so its keychain
//! entries, named after that path, go on working. Every other workspace is `workspaces/<id>/` under the root.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_ID: &str = "default";
const DEFAULT_NAME: &str = "Personal";
const FILE: &str = "workspaces.json";
const FOLDER: &str = "workspaces";
/// Local data on this computer; online workspaces, shared by a team, come later.
pub const LOCAL: &str = "local";
/// What a workspace keeps in its dir: removed from the root when the first workspace is deleted.
const ENTRIES: &[&str] = &[
    "vault.db",
    "vault.db-wal",
    "vault.db-shm",
    "vault.db.1",
    "vault.db.2",
    "vault.db.3",
    "library.db",
    "library.db-journal",
    "doc-configs.json",
    "files",
    "templates",
    "uploads",
    "inbox",
    "previews",
    "scratch",
    crate::backup::TMP_FOLDER,
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    active: String,
    workspaces: Vec<Workspace>,
}

pub struct Workspaces {
    root: PathBuf,
    pub active: String,
    pub list: Vec<Workspace>,
}

impl Workspaces {
    /// Without the file, or with one that cannot be read: the first workspace alone.
    pub fn load(root: &Path) -> Self {
        let stored = std::fs::read_to_string(root.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok());
        let Stored { active, workspaces } = stored.filter(|s: &Stored| !s.workspaces.is_empty()).unwrap_or_else(|| {
            let first = Workspace { id: DEFAULT_ID.into(), name: DEFAULT_NAME.into(), kind: LOCAL.into() };
            Stored { active: DEFAULT_ID.into(), workspaces: vec![first] }
        });
        let active = if workspaces.iter().any(|w| w.id == active) { active } else { workspaces[0].id.clone() };
        Self { root: root.to_path_buf(), active, list: workspaces }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let stored = Stored { active: self.active.clone(), workspaces: self.list.clone() };
        let text = serde_json::to_string_pretty(&stored).map_err(std::io::Error::other)?;
        let tmp = self.root.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, self.root.join(FILE))
    }

    /// Where the workspace keeps its data.
    pub fn dir(&self, id: &str) -> PathBuf {
        dir(&self.root, id)
    }

    pub fn get(&self, id: &str) -> Option<&Workspace> {
        self.list.iter().find(|w| w.id == id)
    }

    /// An id for a workspace that is not listed yet, so its dir can be filled before `add`.
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    pub fn add(&mut self, id: &str, name: &str) {
        self.list.push(Workspace { id: id.into(), name: name.into(), kind: LOCAL.into() });
    }

    /// Takes the workspace off the list, the last one never; its files go at the next start (`cleanup`). The open
    /// workspace becomes the first one left.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        if self.list.len() < 2 {
            return Err("The only workspace cannot be deleted.".into());
        }
        self.list.retain(|w| w.id != id);
        if self.active == id {
            self.active = self.list[0].id.clone();
        }
        Ok(())
    }

    pub fn set_active(&mut self, id: &str) -> Result<(), String> {
        if self.get(id).is_none() {
            return Err(format!("unknown workspace {id}"));
        }
        self.active = id.into();
        Ok(())
    }
}

fn dir(root: &Path, id: &str) -> PathBuf {
    if id == DEFAULT_ID {
        root.to_path_buf()
    } else {
        root.join(FOLDER).join(id)
    }
}

/// At startup, before anything is open: deletes the files of workspaces that are no longer listed, which are the
/// deleted ones and a restore that did not finish.
pub fn cleanup(root: &Path) {
    let workspaces = Workspaces::load(root);
    for entry in std::fs::read_dir(root.join(FOLDER)).into_iter().flatten().flatten() {
        if workspaces.get(&entry.file_name().to_string_lossy()).is_none() {
            remove(&entry.path());
        }
    }
    if workspaces.get(DEFAULT_ID).is_none() {
        ENTRIES.iter().for_each(|name| remove(&root.join(name)));
    }
}

fn remove(path: &Path) {
    let result = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    if let Err(e) = result {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("workspaces: cannot remove {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_file_there_is_the_first_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let workspaces = Workspaces::load(dir.path());
        assert_eq!(workspaces.active, DEFAULT_ID);
        assert_eq!(workspaces.list, [Workspace { id: DEFAULT_ID.into(), name: "Personal".into(), kind: LOCAL.into() }]);
        assert_eq!(workspaces.dir(DEFAULT_ID), dir.path());
    }

    #[test]
    fn workspaces_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspaces = Workspaces::load(dir.path());
        let id = Workspaces::new_id();
        workspaces.add(&id, "Oak Street Office");
        workspaces.set_active(&id).unwrap();
        assert!(workspaces.set_active("nope").is_err());
        workspaces.save().unwrap();

        let mut loaded = Workspaces::load(dir.path());
        assert_eq!(loaded.active, id);
        assert_eq!(loaded.get(&id).unwrap().name, "Oak Street Office");
        assert_eq!(loaded.dir(&id), dir.path().join("workspaces").join(&id));

        loaded.remove(&id).unwrap();
        assert_eq!(loaded.active, DEFAULT_ID);
        assert!(loaded.remove(DEFAULT_ID).is_err(), "the last one stays");
    }

    #[test]
    fn an_open_workspace_that_is_gone_falls_back_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let text = r#"{"active":"gone","workspaces":[{"id":"default","name":"Personal","kind":"local"}]}"#;
        std::fs::write(dir.path().join(FILE), text).unwrap();
        assert_eq!(Workspaces::load(dir.path()).active, DEFAULT_ID);
    }

    #[test]
    fn cleanup_removes_what_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut workspaces = Workspaces::load(root);
        let kept = Workspaces::new_id();
        workspaces.add(&kept, "Kept");
        workspaces.save().unwrap();
        for folder in [kept.as_str(), "unfinished"] {
            std::fs::create_dir_all(root.join("workspaces").join(folder).join("files")).unwrap();
        }
        std::fs::create_dir_all(root.join("templates")).unwrap();
        std::fs::write(root.join("vault.db"), "x").unwrap();

        cleanup(root);
        assert!(root.join("workspaces").join(&kept).join("files").is_dir());
        assert!(!root.join("workspaces/unfinished").exists());
        assert!(root.join("vault.db").exists() && root.join("templates").is_dir(), "the first workspace is listed");

        workspaces.remove(DEFAULT_ID).unwrap();
        workspaces.save().unwrap();
        std::fs::write(root.join("config.toml"), "port = 9001\n").unwrap();
        cleanup(root);
        assert!(!root.join("vault.db").exists() && !root.join("templates").exists());
        assert!(root.join("config.toml").exists() && root.join(FILE).exists());
        assert!(root.join("workspaces").join(&kept).join("files").is_dir());
    }
}
