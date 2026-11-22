//! A local backup: one workspace in one file, encrypted with its master password, to restore as a workspace on
//! this or another computer.
//!
//! The file is a tar of the workspace's data, not compressed (its files already are, or are encrypted), cut into
//! chunks that are each encrypted on their way to disk, so a backup of several GB is one pass and costs about a copy:
//!
//! ```text
//! "BLANKEYBAK1\n"
//! u32 LE length, header JSON { version, kdf, vault_schema }
//! chunks: u8 last (0 or 1), u32 LE length, nonce(12) || AES-256-GCM ciphertext+tag     1 MiB of tar per chunk
//! ```
//!
//! The key is Argon2id of the password with the header's parameters. A chunk's associated data is its index and
//! whether it is the last, so a cut, reordered or spliced file does not open.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::Blankey;
use crate::crypto::{self, KdfParams, Key};
use crate::i18n::mark;
use crate::keychain::MemoryStore;
use crate::vault::{self, Vault};

/// The extension of a backup file.
pub const EXTENSION: &str = "blankey";
/// In the data dir while a backup is written: the database snapshots.
pub const TMP_FOLDER: &str = "backup-tmp";
const MAGIC: &[u8] = b"BLANKEYBAK1\n";
const VERSION: u32 = 1;
const CHUNK: usize = 1024 * 1024;
const MAX_HEADER: usize = 4096;
/// nonce and tag around a chunk
const SEAL_OVERHEAD: usize = crypto::NONCE_SIZE + 16;
const DATABASES: &[&str] = &["vault.db", "library.db"];
const FOLDERS: &[&str] = &["files", "templates", "uploads", "inbox"];

// the window translates these messages
const NOT_A_BACKUP: &str = mark("This file is not a Blankey backup.");
const WRONG_PASSWORD: &str = mark("Wrong password.");
const DAMAGED: &str = mark("The backup file is damaged.");
const NEWER: &str = mark("This backup was made by a newer version of Blankey. Update Blankey first.");

#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    kdf: KdfParams,
    /// the vault's database layout: a build that does not know it yet cannot open the backup
    vault_schema: i64,
}

/// What a backup is encrypted with, made from the master password.
pub struct BackupKey {
    header: Header,
    key: Key,
}

fn aad(index: u64, last: bool) -> Vec<u8> {
    format!("blankey:backup|{index}|{}", last as u8).into_bytes()
}

/// Encrypts what is written to it, a chunk at a time; `finish` writes the last chunk.
struct SealWriter<W: Write> {
    inner: W,
    key: Key,
    buffer: Vec<u8>,
    index: u64,
}

impl<W: Write> SealWriter<W> {
    fn new(mut inner: W, backup: &BackupKey) -> std::io::Result<Self> {
        let header = serde_json::to_vec(&backup.header)?;
        inner.write_all(MAGIC)?;
        inner.write_all(&(header.len() as u32).to_le_bytes())?;
        inner.write_all(&header)?;
        Ok(Self { inner, key: backup.key.clone(), buffer: Vec::with_capacity(CHUNK), index: 0 })
    }

    fn emit(&mut self, last: bool) -> std::io::Result<()> {
        let sealed = crypto::encrypt_joined(self.key.as_ref(), &self.buffer, &aad(self.index, last));
        self.inner.write_all(&[last as u8])?;
        self.inner.write_all(&(sealed.len() as u32).to_le_bytes())?;
        self.inner.write_all(&sealed)?;
        self.index += 1;
        self.buffer.clear();
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<W> {
        self.emit(true)?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for SealWriter<W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        // a full chunk waits for more data, so the last one is known when it is written
        if self.buffer.len() == CHUNK {
            self.emit(false)?;
        }
        let taken = data.len().min(CHUNK - self.buffer.len());
        self.buffer.extend_from_slice(&data[..taken]);
        Ok(taken)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Reads a backup's tar back, a chunk at a time. `damaged` is set when a chunk does not open.
struct OpenReader<R: Read> {
    inner: R,
    key: Key,
    chunk: Vec<u8>,
    at: usize,
    index: u64,
    done: bool,
    damaged: bool,
}

impl<R: Read> OpenReader<R> {
    /// Reads the header and the first chunk, which tells a wrong password from a damaged file.
    fn open(mut inner: R, password: &str) -> Result<Self, String> {
        let header = read_header(&mut inner).ok_or(NOT_A_BACKUP)?;
        if header.version > VERSION || header.vault_schema > vault::schema_version() {
            return Err(NEWER.into());
        }
        let key = crypto::derive_key(password, &header.kdf).map_err(|e| e.to_string())?;
        let mut reader = Self { inner, key, chunk: Vec::new(), at: 0, index: 0, done: false, damaged: false };
        reader.next_chunk().map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidData => WRONG_PASSWORD,
            _ => DAMAGED,
        })?;
        Ok(reader)
    }

    /// A chunk that does not decrypt is `InvalidData`; a cut or malformed file any other error.
    fn next_chunk(&mut self) -> std::io::Result<()> {
        let mut frame = [0u8; 5];
        self.inner.read_exact(&mut frame)?;
        let length = u32::from_le_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        if frame[0] > 1 || length > CHUNK + SEAL_OVERHEAD {
            return Err(std::io::Error::other("malformed chunk"));
        }
        let last = frame[0] == 1;
        let mut sealed = vec![0u8; length];
        self.inner.read_exact(&mut sealed)?;
        self.chunk = crypto::decrypt_joined(self.key.as_ref(), &sealed, &aad(self.index, last))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.at = 0;
        self.index += 1;
        self.done = last;
        Ok(())
    }
}

impl<R: Read> Read for OpenReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.chunk.len() {
            if self.done {
                return Ok(0);
            }
            if let Err(e) = self.next_chunk() {
                self.damaged = true;
                return Err(e);
            }
        }
        let taken = out.len().min(self.chunk.len() - self.at);
        out[..taken].copy_from_slice(&self.chunk[self.at..self.at + taken]);
        self.at += taken;
        Ok(taken)
    }
}

fn read_header(reader: &mut impl Read) -> Option<Header> {
    let mut magic = [0u8; MAGIC.len()];
    reader.read_exact(&mut magic).ok()?;
    let mut length = [0u8; 4];
    reader.read_exact(&mut length).ok()?;
    let length = u32::from_le_bytes(length) as usize;
    if magic != MAGIC || length > MAX_HEADER {
        return None;
    }
    let mut header = vec![0u8; length];
    reader.read_exact(&mut header).ok()?;
    serde_json::from_slice(&header).ok()
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    target.with_file_name(name)
}

impl Blankey {
    /// The key for a backup of this workspace; refuses anything but its master password.
    pub fn backup_key(&self, password: &str) -> Result<BackupKey, String> {
        if !self.vault().verify_password(password) {
            return Err(WRONG_PASSWORD.into());
        }
        let kdf = KdfParams::new();
        let key = crypto::derive_key(password, &kdf).map_err(|e| e.to_string())?;
        Ok(BackupKey { header: Header { version: VERSION, kdf, vault_schema: vault::schema_version() }, key })
    }

    /// Writes this workspace to `target`: the databases as consistent snapshots, the folders as they are.
    pub fn create_backup(&self, key: &BackupKey, target: &Path) -> Result<(), String> {
        let tmp = self.config.data_dir.join(TMP_FOLDER);
        let part = part_path(target);
        let result = self
            .write_backup(key, &tmp, &part)
            .and_then(|()| std::fs::rename(&part, target).map_err(|e| format!("{}: {e}", target.display())));
        let _ = std::fs::remove_dir_all(&tmp);
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        result?;
        self.audit("user", "create_backup", "");
        Ok(())
    }

    fn write_backup(&self, key: &BackupKey, tmp: &Path, part: &Path) -> Result<(), String> {
        let io = |e: std::io::Error| e.to_string();
        let _ = std::fs::remove_dir_all(tmp);
        std::fs::create_dir_all(tmp).map_err(io)?;
        self.vault().snapshot_to(&tmp.join("vault.db")).map_err(|e| e.to_string())?;
        self.library.snapshot_to(&tmp.join("library.db"))?;

        let file = File::create(part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut tar = tar::Builder::new(SealWriter::new(BufWriter::new(file), key).map_err(io)?);
        tar.follow_symlinks(false);
        for name in DATABASES {
            tar.append_path_with_name(tmp.join(name), name).map_err(io)?;
        }
        for name in FOLDERS {
            let folder = self.config.data_dir.join(name);
            if folder.is_dir() {
                tar.append_dir_all(name, &folder).map_err(io)?;
            }
        }
        tar.into_inner().and_then(SealWriter::finish).map(drop).map_err(io)
    }
}

/// Unpacks the backup at `source` into `dir`, a new workspace's data dir, which then opens with `password`. On any
/// failure `dir` is removed.
pub fn restore_into(source: &Path, password: &str, dir: &Path) -> Result<(), String> {
    let result = unpack(source, password, dir);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(dir);
    }
    result
}

fn unpack(source: &Path, password: &str, dir: &Path) -> Result<(), String> {
    let file = File::open(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut reader = OpenReader::open(BufReader::new(file), password)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let unpacked = unpack_entries(&mut reader, dir);
    if reader.damaged {
        return Err(DAMAGED.into());
    }
    unpacked?;
    if !dir.join("vault.db").is_file() {
        return Err(DAMAGED.into());
    }
    let mut vault = Vault::open(&dir.join("vault.db"), Box::new(MemoryStore::default())).map_err(|e| match e {
        vault::VaultError::Schema(_) => NEWER.to_string(),
        e => e.to_string(),
    })?;
    if !vault.verify_password(password) {
        return Err(WRONG_PASSWORD.into());
    }
    // the keychain key that opened it is on the computer the backup came from, under another name
    vault.disable_keyring().map_err(|e| e.to_string())
}

/// Only what a backup holds is unpacked: plain files and folders under the known names.
fn unpack_entries(reader: &mut impl Read, dir: &Path) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let mut archive = tar::Archive::new(&mut *reader);
    for entry in archive.entries().map_err(io)? {
        let mut entry = entry.map_err(io)?;
        let path = entry.path().map_err(io)?.into_owned();
        let top = match path.components().next() {
            Some(Component::Normal(name)) => name.to_string_lossy().into_owned(),
            _ => return Err(DAMAGED.into()),
        };
        let kind = entry.header().entry_type();
        let known = if FOLDERS.contains(&top.as_str()) {
            kind.is_file() || kind.is_dir()
        } else {
            DATABASES.contains(&top.as_str()) && kind.is_file() && path.components().count() == 1
        };
        if !known || !entry.unpack_in(dir).map_err(io)? {
            return Err(DAMAGED.into());
        }
    }
    // tar stops at its end marker: the rest is read so a cut file is noticed
    std::io::copy(reader, &mut std::io::sink()).map(drop).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::tests::{app, PASSWORD};
    use crate::vault::FieldInput;

    /// An app with a profile, a template folder and an upload larger than one chunk; returns the backup's path.
    fn backed_up(dir: &Path) -> PathBuf {
        let app = app(dir);
        let id = app.vault().create_profile("Maria Example", "person").unwrap();
        let field = FieldInput::new("name", "Maria Example", "Full name", "text");
        app.vault().set_values(id, &[field]).unwrap();
        let data = &app.config.data_dir;
        std::fs::create_dir_all(data.join("templates/lease")).unwrap();
        std::fs::write(data.join("templates/lease/manifest.yaml"), "name: Lease\n").unwrap();
        std::fs::create_dir_all(data.join("uploads/local")).unwrap();
        let big: Vec<u8> = (0..CHUNK * 2 + 4321).map(|i| (i % 251) as u8).collect();
        std::fs::write(data.join("uploads/local/scan.pdf"), &big).unwrap();
        std::fs::create_dir_all(data.join("scratch")).unwrap();
        std::fs::write(data.join("scratch/open.pdf"), "temporary").unwrap();

        let target = dir.join("Personal 2026-11-21.blankey");
        app.create_backup(&app.backup_key(PASSWORD).unwrap(), &target).unwrap();
        assert!(!data.join(TMP_FOLDER).exists() && !part_path(&target).exists());
        target
    }

    #[test]
    fn a_backup_restores_as_a_workspace_that_opens_with_its_password() {
        let dir = tempfile::tempdir().unwrap();
        let backup = backed_up(dir.path());
        let bytes = std::fs::read(&backup).unwrap();
        assert!(bytes.len() > CHUNK * 2);
        assert!(!bytes.windows(13).any(|w| w == b"Maria Example") && !bytes.windows(11).any(|w| w == b"name: Lease"));

        let restored = dir.path().join("restored");
        restore_into(&backup, PASSWORD, &restored).unwrap();
        let mut vault = Vault::open(&restored.join("vault.db"), Box::new(MemoryStore::default())).unwrap();
        assert!(!vault.keyring_enabled());
        vault.unlock(PASSWORD).unwrap();
        let profiles = vault.list_profiles().unwrap();
        assert_eq!(profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Maria Example"]);
        assert_eq!(std::fs::read_to_string(restored.join("templates/lease/manifest.yaml")).unwrap(), "name: Lease\n");
        let big = std::fs::read(restored.join("uploads/local/scan.pdf")).unwrap();
        assert_eq!(big.len(), CHUNK * 2 + 4321);
        assert!(big.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
        assert!(restored.join("library.db").is_file());
        assert!(!restored.join("scratch").exists() && !restored.join("config.toml").exists());
    }

    #[test]
    fn a_backup_needs_the_master_password() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        assert_eq!(app.backup_key("not the password").err().unwrap(), WRONG_PASSWORD);
    }

    #[test]
    fn what_is_not_a_good_backup_is_refused_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let backup = backed_up(dir.path());
        let bytes = std::fs::read(&backup).unwrap();
        let restored = dir.path().join("restored");
        let refused = |name: &str, content: &[u8], password: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, content).unwrap();
            let error = restore_into(&path, password, &restored).unwrap_err();
            assert!(!restored.exists(), "{name}");
            error
        };

        assert_eq!(refused("wrong-password", &bytes, "not the password"), WRONG_PASSWORD);
        assert_eq!(refused("other-file", b"%PDF-1.7 not a backup at all", PASSWORD), NOT_A_BACKUP);
        assert_eq!(refused("cut", &bytes[..bytes.len() - 100], PASSWORD), DAMAGED);
        assert_eq!(refused("cut-at-a-chunk", &bytes[..bytes.len() / 2], PASSWORD), DAMAGED);
        let mut flipped = bytes.clone();
        let middle = flipped.len() / 2;
        flipped[middle] ^= 1;
        assert_eq!(refused("flipped", &flipped, PASSWORD), DAMAGED);

        let header_length = u32::from_le_bytes(bytes[MAGIC.len()..MAGIC.len() + 4].try_into().unwrap()) as usize;
        let body = &bytes[MAGIC.len() + 4 + header_length..];
        let mut header: Header = serde_json::from_slice(&bytes[MAGIC.len() + 4..][..header_length]).unwrap();
        header.vault_schema += 1;
        let header = serde_json::to_vec(&header).unwrap();
        let newer = [MAGIC, &(header.len() as u32).to_le_bytes(), &header, body].concat();
        assert_eq!(refused("newer", &newer, PASSWORD), NEWER);
    }

    #[test]
    fn an_entry_a_backup_never_holds_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let key = app.backup_key(PASSWORD).unwrap();
        let path = dir.path().join("odd.blankey");
        let mut tar = tar::Builder::new(SealWriter::new(File::create(&path).unwrap(), &key).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_cksum();
        tar.append_data(&mut header, "config.toml", &b"port"[..]).unwrap();
        tar.into_inner().unwrap().finish().unwrap();

        let restored = dir.path().join("restored");
        assert_eq!(restore_into(&path, PASSWORD, &restored).unwrap_err(), DAMAGED);
        assert!(!restored.exists());
    }
}
