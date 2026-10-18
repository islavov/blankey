//! Collection links: a public form on the relay whose answers and files the browser seals to a key only this computer
//! holds. The public spec, the sealed plaintext layout, and what the app does with a submission once it is pulled
//! down (a `link_submission` request pointing at `inbox/<sid>.bin`, still sealed).

use std::io::{Read, Write};
use std::path::PathBuf;

use base64::Engine;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{Blankey, Problem, ProfileSave, Target};
use crate::crypto;
use crate::fieldtypes;
use crate::vault::{Link, SubmissionInfo};

/// Same value as MAX_FILE_BYTES in the relay and its collect page.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub const DEFAULT_MAX_FILES: u32 = 5;
pub const MAX_FILES: u32 = 10;
pub const MAX_SUBMISSIONS: u32 = 100;
pub const MAX_LINK_DAYS: u32 = 90;
const MAX_FIELDS: usize = 50;
/// Images up to this size get an inline preview in the submission sheet.
const THUMBNAIL_MAX: usize = 8 * 1024 * 1024;

pub const LINK_SUBMISSION: &str = "link_submission";
pub const COLLECTION_LINK: &str = "collection_link";

/// The largest blob the relay accepts for a spec: the files plus room for the header and the seal.
pub fn blob_cap(max_files: u32) -> u64 {
    max_files as u64 * MAX_FILE_BYTES + 1024 * 1024
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpecField {
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(rename = "type", default = "text_type")]
    pub field_type: String,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub required: bool,
}

fn text_type() -> String {
    "text".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilesSpec {
    pub max_files: u32,
    pub kinds: Vec<String>,
}

/// What the relay serves to anyone with the link: no values, no profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub title: String,
    pub from: String,
    pub note: String,
    pub fields: Vec<SpecField>,
    pub files: FilesSpec,
}

/// A link as the user sets it up in the New link sheet, or as an agent drafts it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkDraft {
    pub title: String,
    /// the name the recipient sees next to the account email
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub profile_id: Option<i64>,
    #[serde(default)]
    pub new_profile: Option<String>,
    #[serde(default)]
    pub fields: Vec<SpecField>,
    #[serde(default)]
    pub max_files: u32,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default = "default_days")]
    pub expires_days: u32,
    #[serde(default = "default_submissions")]
    pub max_submissions: u32,
}

fn default_days() -> u32 {
    7
}

fn default_submissions() -> u32 {
    1
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

/// HTML `accept` tokens, as the relay checks them: `.pdf`, `image/*`, `application/pdf`.
fn valid_kind(kind: &str) -> bool {
    let token =
        |s: &str, extra: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    match kind.strip_prefix('.') {
        Some(extension) => extension.len() <= 10 && token(extension, ""),
        None => kind.split_once('/').is_some_and(|(top, sub)| token(top, "") && (sub == "*" || token(sub, ".+-"))),
    }
}

impl LinkDraft {
    /// The public spec, or what is wrong with the draft.
    pub fn spec(&self) -> Result<Spec, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Give the link a title".into());
        }
        if self.fields.len() > MAX_FIELDS {
            return Err(format!("A link can ask for up to {MAX_FIELDS} fields"));
        }
        let mut fields = Vec::new();
        for field in &self.fields {
            let key = field.key.trim();
            if !valid_key(key) {
                return Err(format!("{key:?}: field keys are letters, digits, '.', '_' or '-'"));
            }
            if fields.iter().any(|f: &SpecField| f.key == key) {
                return Err(format!("{key}: duplicate field"));
            }
            if !fieldtypes::is_type(&field.field_type) {
                return Err(format!("{key}: unknown type {:?}", field.field_type));
            }
            let label = field.label.trim();
            fields.push(SpecField {
                key: key.into(),
                label: if label.is_empty() { key.into() } else { label.into() },
                field_type: field.field_type.clone(),
                hint: field.hint.trim().into(),
                required: field.required,
            });
        }
        if self.max_files > MAX_FILES {
            return Err(format!("A link can ask for up to {MAX_FILES} files"));
        }
        let kinds: Vec<String> = self.kinds.iter().map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()).collect();
        if let Some(bad) = kinds.iter().find(|k| !valid_kind(k)) {
            return Err(format!("{bad}: use file extensions (.pdf) or types (image/*)"));
        }
        if fields.is_empty() && self.max_files == 0 {
            return Err("Ask for at least one field or file".into());
        }
        if !(1..=MAX_LINK_DAYS).contains(&self.expires_days) {
            return Err(format!("A link can stay open for 1 to {MAX_LINK_DAYS} days"));
        }
        if !(1..=MAX_SUBMISSIONS).contains(&self.max_submissions) {
            return Err(format!("A link can take 1 to {MAX_SUBMISSIONS} submissions"));
        }
        Ok(Spec {
            title: title.into(),
            from: self.from.trim().into(),
            note: self.note.trim().into(),
            fields,
            files: FilesSpec { max_files: self.max_files, kinds },
        })
    }

    pub fn expires_at(&self) -> String {
        let at = chrono::Utc::now() + chrono::Duration::days(self.expires_days as i64);
        at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }
}

/// open, expired, revoked or full, as far as this computer knows; the relay's count is more current.
pub fn link_state(link: &Link) -> &'static str {
    let expired = chrono::DateTime::parse_from_rfc3339(&link.expires_at).is_ok_and(|at| at < chrono::Utc::now());
    if link.revoked_at.is_some() {
        "revoked"
    } else if expired {
        "expired"
    } else if link.received >= link.max_submissions {
        "full"
    } else {
        "open"
    }
}

/// Where the relay holds a submission for this device (`GET /device/inbox`).
#[derive(Clone, Debug, Deserialize)]
pub struct InboxEntry {
    pub sid: String,
    pub link_id: String,
    pub size: u64,
    /// presigned GET, no bearer token
    pub url: String,
}

/// The sealed plaintext's header.
#[derive(Debug, Deserialize)]
pub struct Header {
    pub v: u32,
    #[serde(default)]
    pub values: IndexMap<String, Value>,
    #[serde(default)]
    pub files: Vec<HeaderFile>,
    #[serde(default)]
    pub submitted_at: String,
}

#[derive(Debug, Deserialize)]
pub struct HeaderFile {
    pub name: String,
    #[serde(rename = "type", default)]
    pub file_type: String,
    pub size: u64,
}

/// `u32 BE header length || header JSON || file bytes back to back`.
pub fn parse_plaintext(bytes: &[u8]) -> Result<(Header, Vec<&[u8]>), String> {
    let malformed = || "The submission is malformed".to_string();
    let length: [u8; 4] = bytes.get(..4).ok_or_else(malformed)?.try_into().unwrap();
    let end = 4usize.checked_add(u32::from_be_bytes(length) as usize).ok_or_else(malformed)?;
    let header: Header = serde_json::from_slice(bytes.get(4..end).ok_or_else(malformed)?).map_err(|_| malformed())?;
    if header.v != 1 {
        return Err(format!("Unknown submission version {}", header.v));
    }
    let mut offset = end;
    let mut files = Vec::new();
    for file in &header.files {
        if file.size > MAX_FILE_BYTES {
            return Err(malformed());
        }
        let next = offset + file.size as usize;
        files.push(bytes.get(offset..next).ok_or_else(malformed)?);
        offset = next;
    }
    if offset != bytes.len() {
        return Err(malformed());
    }
    Ok((header, files))
}

#[derive(Debug, Serialize)]
pub struct SubmittedFile {
    pub index: usize,
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub size: u64,
    /// data URL, for images small enough to show
    pub thumbnail: Option<String>,
}

/// What the submission sheet shows.
#[derive(Debug, Serialize)]
pub struct Submission {
    #[serde(flatten)]
    pub info: SubmissionInfo,
    pub link: Link,
    pub fields: Vec<SpecField>,
    pub values: IndexMap<String, String>,
    pub files: Vec<SubmittedFile>,
    pub submitted_at: String,
}

fn thumbnail(file_type: &str, bytes: &[u8]) -> Option<String> {
    let shown = ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&file_type);
    (shown && bytes.len() <= THUMBNAIL_MAX)
        .then(|| format!("data:{file_type};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

impl Blankey {
    /// Submissions pulled from the relay, still sealed to their link's key.
    pub fn inbox_dir(&self) -> PathBuf {
        self.config.data_dir.join("inbox")
    }

    fn inbox_path(&self, sid: &str) -> Result<PathBuf, String> {
        if sid.is_empty() || !sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("Bad submission id {sid:?}"));
        }
        Ok(self.inbox_dir().join(format!("{sid}.bin")))
    }

    /// `inbox/<sid>.link` holds the link id from the download until the request exists, so a submission the relay
    /// already deleted still becomes a request on the next drain.
    fn marker_path(&self, sid: &str) -> Result<PathBuf, String> {
        Ok(self.inbox_path(sid)?.with_extension("link"))
    }

    /// (link id, sid) of downloads that have no request yet.
    pub fn unaccepted_submissions(&self) -> Vec<(String, String)> {
        let Ok(entries) = std::fs::read_dir(self.inbox_dir()) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let sid = path.file_stem()?.to_str()?.to_string();
                (path.extension()? == "link" && path.with_extension("bin").exists()).then_some(())?;
                Some((std::fs::read_to_string(&path).ok()?.trim().to_string(), sid))
            })
            .collect()
    }

    /// Downloads the sealed blob to `inbox/<sid>.bin`; already there means an earlier drain got it. Blocking.
    pub fn download_submission(&self, agent: &ureq::Agent, entry: &InboxEntry) -> Result<(), String> {
        let path = self.inbox_path(&entry.sid)?;
        if path.exists() {
            return Ok(());
        }
        if entry.size > blob_cap(MAX_FILES) {
            return Err(format!("Submission {} is larger than any link allows", entry.sid));
        }
        std::fs::create_dir_all(self.inbox_dir()).map_err(|e| e.to_string())?;
        let part = path.with_extension("bin.part");
        let result = (|| {
            let response = agent.get(&entry.url).call().map_err(|e| e.to_string())?;
            let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let copied = std::io::copy(&mut response.into_reader().take(entry.size + 1), &mut file)
                .map_err(|e| e.to_string())?;
            if copied != entry.size {
                return Err(format!("Submission {}: got {copied} of {} bytes", entry.sid, entry.size));
            }
            file.flush().map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::write(self.marker_path(&entry.sid)?, &entry.link_id).map_err(|e| e.to_string())?;
            std::fs::rename(&part, &path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        result
    }

    /// Records a downloaded submission and creates the request the user answers; works while the vault is locked.
    /// A submission for a revoked or unknown link is dropped: None.
    pub fn accept_submission(&self, link_id: &str, sid: &str) -> Result<Option<i64>, String> {
        let path = self.inbox_path(sid)?;
        let link = self.vault().get_link(link_id).ok();
        let Some(link) = link.filter(|l| l.revoked_at.is_none()) else {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(self.marker_path(sid)?);
            self.audit("relay", "submission_revoked", &format!("link={link_id} sid={sid}"));
            return Ok(None);
        };
        let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len() as i64;
        let payload = json!({"link_id": link_id, "sid": sid, "reason": format!("Submission for {}", link.title)});
        let id = self.create_request(LINK_SUBMISSION, payload, None)?;
        {
            let vault = self.vault();
            vault.add_submission(sid, link_id, id, size).map_err(|e| e.to_string())?;
            let _ = vault.count_received(link_id);
        }
        let _ = std::fs::remove_file(self.marker_path(sid)?);
        Ok(Some(id))
    }

    /// The sid of a submission request.
    pub fn submission_of(&self, request_id: i64) -> Result<String, String> {
        let request = self.vault().get_request(request_id).map_err(|e| e.to_string())?;
        if request.kind != LINK_SUBMISSION {
            return Err(format!("Request {request_id} is not a submission"));
        }
        Ok(request.payload.get("sid").and_then(Value::as_str).unwrap_or_default().to_string())
    }

    /// Resolves the submission's request when it still waits.
    fn close_request(&self, info: &SubmissionInfo, status: &str, result: Value) -> Result<(), String> {
        let Some(id) = info.request_id else { return Ok(()) };
        let mut vault = self.vault();
        if vault.get_request(id).is_ok_and(|r| r.status == "pending") {
            vault.resolve_request(id, status, result).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Unseals a received submission; a blob that does not open is deleted and the submission marked damaged.
    fn unseal_submission(&self, sid: &str) -> Result<(SubmissionInfo, Link, Vec<u8>), String> {
        let (info, link, key) = {
            let vault = self.vault();
            let info = vault.get_submission(sid).map_err(|e| e.to_string())?;
            let link = vault.get_link(&info.link_id).map_err(|e| e.to_string())?;
            let key = vault.link_key(&link.link_id).map_err(|e| e.to_string())?;
            (info, link, key)
        };
        if info.status == "discarded" || info.status == "damaged" {
            return Err(format!("This submission was {}.", info.status));
        }
        let key = key.ok_or("The key of this link is gone, so its submissions no longer open.")?;
        let sealed = std::fs::read(self.inbox_path(sid)?).map_err(|e| e.to_string())?;
        let plain = crypto::unseal(key.as_ref(), &sealed, format!("link:{}", link.link_id).as_bytes()).ok();
        match plain.filter(|p| parse_plaintext(p).is_ok()) {
            Some(plain) => Ok((info, link, plain)),
            None => {
                self.forget_submission(&info, "damaged")?;
                Err("This submission could not be opened, so it was dropped.".into())
            }
        }
    }

    /// Any number of times: a saved submission stays until it is discarded.
    pub fn open_submission(&self, sid: &str) -> Result<Submission, String> {
        let (info, link, plain) = self.unseal_submission(sid)?;
        let (header, contents) = parse_plaintext(&plain)?;
        let fields: Vec<SpecField> = serde_json::from_value(link.spec["fields"].clone()).unwrap_or_default();
        let values = fields
            .iter()
            .map(|f| {
                let value = match header.values.get(&f.key) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                (f.key.clone(), value)
            })
            .collect();
        let files = header
            .files
            .iter()
            .zip(&contents)
            .enumerate()
            .map(|(index, (file, bytes))| SubmittedFile {
                index,
                name: file.name.clone(),
                file_type: file.file_type.clone(),
                size: file.size,
                thumbnail: thumbnail(&file.file_type, bytes),
            })
            .collect();
        self.audit("user", "open_submission", &format!("sid={sid} link={}", link.link_id));
        Ok(Submission { info, link, fields, values, files, submitted_at: header.submitted_at })
    }

    /// (name, bytes) of one submitted file, to open or export it without keeping it in the vault.
    pub fn submission_file(&self, sid: &str, index: usize) -> Result<(String, Vec<u8>), String> {
        let (_, _, plain) = self.unseal_submission(sid)?;
        let (header, contents) = parse_plaintext(&plain)?;
        let file = header.files.get(index).ok_or_else(|| format!("No file {index} in this submission"))?;
        self.audit("user", "open_submission_file", &format!("sid={sid} file={index}"));
        Ok((file.name.clone(), contents[index].to_vec()))
    }

    /// Saves the (edited) values into the target profile and keeps the ticked files, re-encrypted under the vault.
    /// Returns the problems without saving anything when there are any. Can be done again later.
    pub fn resolve_submission(
        &self,
        sid: &str,
        target: Target,
        values: &IndexMap<String, String>,
        keep: &[usize],
    ) -> Result<Vec<Problem>, String> {
        let (info, link) = {
            let vault = self.vault();
            let info = vault.get_submission(sid).map_err(|e| e.to_string())?;
            let link = vault.get_link(&info.link_id).map_err(|e| e.to_string())?;
            (info, link)
        };
        let specs: Vec<Value> = link.spec["fields"].as_array().cloned().unwrap_or_default();
        let (profile_id, fields) = match self.save_profile_values(&specs, values, target)? {
            ProfileSave::Problems(problems) => return Ok(problems),
            ProfileSave::Saved { profile_id, fields } => (profile_id, fields),
        };
        let (_, _, plain) = self.unseal_submission(sid)?;
        let (header, contents) = parse_plaintext(&plain)?;
        let mut kept = Vec::new();
        for (index, (file, bytes)) in header.files.iter().zip(&contents).enumerate() {
            if keep.contains(&index) {
                let id = self
                    .vault()
                    .add_collected_file(Some(profile_id), &link.link_id, &file.name, &file.file_type, bytes)
                    .map_err(|e| e.to_string())?;
                kept.push(json!({"file_id": id, "size": file.size}));
            }
        }
        self.vault().set_submission_status(sid, "saved", Some(profile_id)).map_err(|e| e.to_string())?;
        let result = json!({"outcome": "saved", "profile_id": profile_id, "fields": fields, "files": kept});
        self.close_request(&info, "done", result)?;
        self.audit(
            "user",
            "link_submission",
            &format!("sid={sid} link={} profile={profile_id} files={}", link.link_id, kept.len()),
        );
        Ok(Vec::new())
    }

    /// Removes the link and everything it received that was not kept: the submissions' blobs and rows, and their
    /// waiting requests. Values and files already saved stay. The relay is told separately.
    pub fn delete_link(&self, link_id: &str) -> Result<(), String> {
        let submissions = self.vault().list_submissions(Some(link_id)).map_err(|e| e.to_string())?;
        for info in &submissions {
            let _ = std::fs::remove_file(self.inbox_path(&info.sid)?);
            self.close_request(info, "cancelled", json!({"outcome": "discarded"}))?;
        }
        self.vault().delete_link(link_id).map_err(|e| e.to_string())?;
        self.audit("user", "delete_link", &format!("link={link_id} submissions={}", submissions.len()));
        Ok(())
    }

    /// Deletes the sealed blob for good; the submission stays listed as discarded.
    pub fn discard_submission(&self, sid: &str) -> Result<(), String> {
        let info = self.vault().get_submission(sid).map_err(|e| e.to_string())?;
        self.forget_submission(&info, "discarded")
    }

    fn forget_submission(&self, info: &SubmissionInfo, status: &str) -> Result<(), String> {
        let _ = std::fs::remove_file(self.inbox_path(&info.sid)?);
        self.vault().set_submission_status(&info.sid, status, None).map_err(|e| e.to_string())?;
        self.close_request(info, "cancelled", json!({"outcome": status}))?;
        self.audit("user", &format!("submission_{status}"), &format!("sid={} link={}", info.sid, info.link_id));
        Ok(())
    }

    pub fn export_collected_file(&self, id: i64, target: &std::path::Path) -> Result<(), String> {
        let (_, content) = self.vault().load_collected_file(id).map_err(|e| e.to_string())?;
        std::fs::write(target, content).map_err(|e| e.to_string())?;
        self.audit("user", "export_collected_file", &id.to_string());
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mcp::tools::tests::app;

    pub fn plaintext(values: Value, files: &[(&str, &str, &[u8])]) -> Vec<u8> {
        let header = json!({
            "v": 1,
            "values": values,
            "files": files.iter().map(|(name, kind, bytes)| json!({"name": name, "type": kind, "size": bytes.len()}))
                .collect::<Vec<_>>(),
            "submitted_at": "2026-10-17T12:00:00.000Z",
        })
        .to_string();
        let mut out = (header.len() as u32).to_be_bytes().to_vec();
        out.extend(header.as_bytes());
        for (_, _, bytes) in files {
            out.extend(*bytes);
        }
        out
    }

    pub fn draft() -> LinkDraft {
        serde_json::from_value(json!({
            "title": "Tenant details", "from": "Example Lettings", "new_profile": "New tenant",
            "fields": [{"key": "name", "label": "Full name", "type": "text", "required": true},
                       {"key": "email", "label": "Email", "type": "email"}],
            "max_files": 2,
        }))
        .unwrap()
    }

    /// A link as `Relay::create_link` stores it, without the relay.
    pub fn stored_link(app: &Blankey, link_id: &str) -> [u8; 32] {
        let (private, public) = crypto::new_seal_keypair();
        let spec = draft().spec().unwrap();
        let link = Link {
            link_id: link_id.into(),
            relay_url: "https://relay.example.com".into(),
            url: format!("https://relay.example.com/s/{link_id}#{}", crypto::fingerprint(&public)),
            title: spec.title.clone(),
            spec: serde_json::to_value(&spec).unwrap(),
            profile_id: None,
            new_profile: Some("New tenant".into()),
            created_at: crate::vault::now(),
            expires_at: draft().expires_at(),
            max_submissions: 1,
            revoked_at: None,
            received: 0,
        };
        app.vault().add_link(&link, private.as_ref()).unwrap();
        public
    }

    /// Puts a sealed submission in the inbox as a drain would; its request id, None when it was dropped.
    pub fn try_submit(app: &Blankey, link_id: &str, public: &[u8], plain: &[u8], sid: &str) -> Option<i64> {
        let sealed = crypto::seal(public, plain, format!("link:{link_id}").as_bytes()).unwrap();
        std::fs::create_dir_all(app.inbox_dir()).unwrap();
        std::fs::write(app.inbox_dir().join(format!("{sid}.bin")), sealed).unwrap();
        app.accept_submission(link_id, sid).unwrap()
    }

    pub fn submit(app: &Blankey, link_id: &str, public: &[u8], plain: &[u8], sid: &str) -> i64 {
        try_submit(app, link_id, public, plain, sid).expect("submission accepted")
    }

    #[test]
    fn plaintext_layout() {
        let plain = plaintext(json!({"name": "Иван"}), &[("a.bin", "application/octet-stream", &[0, 1, 2, 0xff])]);
        let (header, files) = parse_plaintext(&plain).unwrap();
        assert_eq!(header.values["name"], "Иван");
        assert_eq!((header.files[0].name.as_str(), files[0]), ("a.bin", &[0u8, 1, 2, 0xff][..]));

        assert!(parse_plaintext(&plain[..plain.len() - 1]).is_err());
        assert!(parse_plaintext(&[plain.as_slice(), &[0]].concat()).is_err());
        assert!(parse_plaintext(&[0, 0, 0xff, 0xff]).is_err());
        assert!(parse_plaintext(&[0, 0]).is_err());
    }

    #[test]
    fn drafts_become_public_specs() {
        let spec = draft().spec().unwrap();
        let value = serde_json::to_value(&spec).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["title", "from", "note", "fields", "files"]);
        assert!(!value.to_string().contains("New tenant"), "the profile stays out of the spec");

        let mut bad = draft();
        bad.fields[1].key = "name".into();
        assert!(bad.spec().unwrap_err().contains("duplicate"));
        let mut bad = draft();
        bad.kinds = vec!["exe".into()];
        assert!(bad.spec().is_err());
        let mut good = draft();
        good.kinds = vec![".pdf".into(), "image/*".into(), "application/pdf".into()];
        assert!(good.spec().is_ok());
        let mut empty = draft();
        (empty.fields, empty.max_files) = (Vec::new(), 0);
        assert!(empty.spec().is_err());
        let mut long = draft();
        long.expires_days = 91;
        assert!(long.spec().is_err());
    }

    #[test]
    fn submission_is_saved_into_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let public = stored_link(&app, "Ab3dE9z");
        // a 50 MB file: the largest a link takes
        let big: Vec<u8> = (0..=250u8).cycle().take(MAX_FILE_BYTES as usize).collect();
        let marker = b"PLAINTEXT-MARKER-0123456789";
        let mut photo = marker.to_vec();
        photo.extend(&big[..1000]);
        let plain = plaintext(
            json!({"name": "Jane Roe", "email": "jane@example.com", "extra": "ignored"}),
            &[("scan.bin", "application/octet-stream", &big), ("photo.png", "image/png", &photo)],
        );
        let id = submit(&app, "Ab3dE9z", &public, &plain, "sub-1");
        let info = app.vault().get_submission("sub-1").unwrap();
        assert_eq!((info.status.as_str(), info.request_id), ("pending", Some(id)));
        app.vault().lock();
        let request = app.vault().get_request(id).unwrap();
        assert!(request.hidden && request.kind == LINK_SUBMISSION);
        assert!(app.open_submission("sub-1").is_err());
        app.vault().unlock(crate::mcp::tools::tests::PASSWORD).unwrap();
        assert_eq!(app.submission_of(id).unwrap(), "sub-1");

        let opened = app.open_submission("sub-1").unwrap();
        assert_eq!(opened.values.keys().collect::<Vec<_>>(), ["name", "email"]);
        assert_eq!(opened.files.len(), 2);
        assert!(opened.files[0].thumbnail.is_none());
        assert!(opened.files[1].thumbnail.as_deref().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(app.submission_file("sub-1", 1).unwrap(), ("photo.png".to_string(), photo.clone()));

        let mut values = opened.values.clone();
        values["email"] = "not an email".into();
        let jane = || Target::New { name: "Jane", kind: "person" };
        let problems = app.resolve_submission("sub-1", jane(), &values, &[0]).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(app.vault().list_profiles().unwrap().is_empty());

        values["email"] = "jane@example.com".into();
        assert!(app.resolve_submission("sub-1", jane(), &values, &[0, 1]).unwrap().is_empty());
        let profile = app.vault().list_profiles().unwrap().pop().unwrap();
        assert_eq!(app.vault().get_values(profile.id).unwrap()["name"].1, "Jane Roe");
        assert_eq!(app.vault().get_request(id).unwrap().status, "done");
        let info = app.vault().get_submission("sub-1").unwrap();
        assert_eq!((info.status.as_str(), info.profile_id), ("saved", Some(profile.id)));
        assert_eq!(app.vault().get_link("Ab3dE9z").unwrap().received, 1);

        // still there, sealed: it opens again and can be saved again
        assert_eq!(std::fs::read_dir(app.inbox_dir()).unwrap().count(), 1);
        assert_eq!(app.open_submission("sub-1").unwrap().values["name"], "Jane Roe");
        assert!(app.resolve_submission("sub-1", Target::Existing(profile.id), &values, &[]).unwrap().is_empty());

        let files = app.vault().list_collected_files().unwrap();
        assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["photo.png", "scan.bin"]);
        assert!(files.iter().all(|f| f.profile_id == Some(profile.id)));
        for entry in std::fs::read_dir(dir.path().join("data/files")).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            let head = &bytes[..bytes.len().min(4096)];
            assert!(!head.windows(marker.len()).any(|w| w == marker), "a collected file is plaintext on disk");
            assert!(!head.windows(64).any(|w| w == &big[..64]), "a collected file is plaintext on disk");
        }
        let target = dir.path().join("export.bin");
        app.export_collected_file(files[1].id, &target).unwrap();
        assert!(std::fs::read(&target).unwrap() == big, "the export is byte-identical");

        app.vault().delete_collected_file(files[1].id).unwrap();
        assert_eq!(std::fs::read_dir(dir.path().join("data/files")).unwrap().count(), 1);

        app.discard_submission("sub-1").unwrap();
        assert_eq!(std::fs::read_dir(app.inbox_dir()).unwrap().count(), 0);
        assert_eq!(app.vault().get_submission("sub-1").unwrap().status, "discarded");
        assert!(app.open_submission("sub-1").unwrap_err().contains("discarded"));
    }

    #[test]
    fn submissions_after_a_revoke_are_dropped_and_earlier_ones_still_open() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let public = stored_link(&app, "XchaBj2");
        let id = submit(&app, "XchaBj2", &public, &plaintext(json!({"name": "Jane"}), &[]), "sub-2");
        app.vault().revoke_link("XchaBj2").unwrap();

        assert_eq!(app.open_submission("sub-2").unwrap().values["name"], "Jane");
        assert_eq!(app.vault().get_request(id).unwrap().status, "pending");

        let late = try_submit(&app, "XchaBj2", &public, &plaintext(json!({"name": "Late"}), &[]), "sub-late");
        assert_eq!(late, None);
        assert!(!app.inbox_dir().join("sub-late.bin").exists());
        assert!(app.vault().get_submission("sub-late").is_err());
        let audited: i64 = app
            .vault()
            .conn()
            .query_row("SELECT count(*) FROM audit WHERE action = 'submission_revoked'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audited, 1);
    }

    #[test]
    fn deleting_a_link_removes_what_it_received_but_not_what_was_kept() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let public = stored_link(&app, "Ab3dE9z");
        let plain = plaintext(json!({"name": "Jane Roe"}), &[("id.pdf", "application/pdf", b"%PDF-1.7")]);
        submit(&app, "Ab3dE9z", &public, &plain, "sub-saved");
        let waiting = submit(&app, "Ab3dE9z", &public, &plain, "sub-waiting");
        let values = app.open_submission("sub-saved").unwrap().values;
        let jane = Target::New { name: "Jane", kind: "person" };
        assert!(app.resolve_submission("sub-saved", jane, &values, &[0]).unwrap().is_empty());
        let other = stored_link(&app, "XchaBj2");
        submit(&app, "XchaBj2", &other, &plain, "sub-other");

        app.delete_link("Ab3dE9z").unwrap();
        assert!(app.vault().get_link("Ab3dE9z").is_err());
        assert!(app.vault().link_key("Ab3dE9z").unwrap().is_none());
        assert!(app.vault().list_submissions(Some("Ab3dE9z")).unwrap().is_empty());
        assert_eq!(app.vault().get_request(waiting).unwrap().status, "cancelled");
        let left: Vec<_> = std::fs::read_dir(app.inbox_dir()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["sub-other.bin"], "only the other link's submission is left");
        // the values and the kept file stay
        let profile = app.vault().list_profiles().unwrap().pop().unwrap();
        assert_eq!(app.vault().get_values(profile.id).unwrap()["name"].1, "Jane Roe");
        assert_eq!(app.vault().list_collected_files().unwrap().len(), 1);
        assert_eq!(app.open_submission("sub-other").unwrap().values["name"], "Jane Roe");
    }

    #[test]
    fn a_blob_sealed_to_another_key_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        stored_link(&app, "XchaBj2");
        let (_, other) = crypto::new_seal_keypair();
        let id = submit(&app, "XchaBj2", &other, &plaintext(json!({}), &[]), "sub-3");
        assert!(app.open_submission("sub-3").unwrap_err().contains("could not be opened"));
        assert_eq!(app.vault().get_request(id).unwrap().status, "cancelled");
        assert_eq!(app.vault().get_submission("sub-3").unwrap().status, "damaged");
    }
}
