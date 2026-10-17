//! Application service shared by the MCP server (metadata + examples) and the UI (real values).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::fieldtypes;
use crate::keychain::SecretStore;
use crate::render::preview;
use crate::templates::engine::{self, BindingReport, Rendered};
use crate::templates::{bindings, fill, yaml_dump, Template, TemplateStore};
use crate::vault::{Bindings, FieldInfo, FieldInput, Profiles, Vault};

pub const PROFILE_INPUT: &str = "profile_input";
pub const FILL: &str = "fill";
pub const FILE: &str = "file";
/// Extensions a `file` request can ask for.
pub const FILE_KINDS: &[&str] = &["pdf", "docx"];
const UPLOAD_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// The outcome of saving typed values into a profile.
pub enum ProfileSave {
    /// nothing was saved
    Problems(Vec<String>),
    /// per field: {key, length, status: saved|skipped}
    Saved { profile_id: i64, fields: Vec<Value> },
}

/// Where typed values go: an existing profile, or a new one with this name and kind.
pub enum Target<'a> {
    Existing(i64),
    New { name: &'a str, kind: &'a str },
}

type RequestHook = Box<dyn Fn(i64) + Send + Sync>;
type UnlockHook = Box<dyn Fn(&str) + Send + Sync>;

pub struct Blankey {
    pub config: Config,
    vault: Mutex<Vault>,
    pub templates: TemplateStore,
    on_request: RwLock<RequestHook>,
    on_unlock_needed: RwLock<UnlockHook>,
}

impl Blankey {
    pub fn new(config: Config, secrets: Box<dyn SecretStore>) -> Result<Self, String> {
        let vault = Vault::open(&config.db_path(), secrets).map_err(|e| e.to_string())?;
        let templates = TemplateStore::new(config.templates_dir());
        Ok(Self {
            config,
            vault: Mutex::new(vault),
            templates,
            on_request: RwLock::new(Box::new(|_| {})),
            on_unlock_needed: RwLock::new(Box::new(|_| {})),
        })
    }

    pub fn vault(&self) -> MutexGuard<'_, Vault> {
        self.vault.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_request_hook(&self, hook: impl Fn(i64) + Send + Sync + 'static) {
        *self.on_request.write().unwrap() = Box::new(hook);
    }

    /// Called with the reason when a caller needs the vault unlocked; the app shows the unlock screen.
    pub fn set_unlock_hook(&self, hook: impl Fn(&str) + Send + Sync + 'static) {
        *self.on_unlock_needed.write().unwrap() = Box::new(hook);
    }

    /// Asks the user to unlock and waits up to `timeout` for it. True once the vault is unlocked.
    pub fn wait_for_unlock(&self, reason: &str, timeout: Duration) -> bool {
        if self.vault().unlocked() {
            return true;
        }
        (self.on_unlock_needed.read().unwrap())(reason);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
            if self.vault().unlocked() {
                return true;
            }
        }
        false
    }

    pub fn audit(&self, actor: &str, action: &str, target: &str) {
        let _ = self.vault().audit(actor, action, target);
    }

    // -- requests ----------------------------------------------------------

    /// grant: the relay grant of a remote agent; only that agent sees the request and its uploads.
    pub fn create_request(&self, kind: &str, payload: Value, grant: Option<&str>) -> Result<i64, String> {
        let id = {
            let vault = self.vault();
            let id = vault.create_request(kind, &payload, grant).map_err(|e| e.to_string())?;
            let _ = vault.audit("mcp", &format!("request:{kind}"), &id.to_string());
            id
        };
        (self.on_request.read().unwrap())(id);
        Ok(id)
    }

    /// Validates `values` against `specs` ([{key, label, type, required}]) and saves them into the target profile.
    pub fn save_profile_values(
        &self,
        specs: &[Value],
        values: &IndexMap<String, String>,
        target: Target,
    ) -> Result<ProfileSave, String> {
        let mut errors = Vec::new();
        let mut items = Vec::new();
        for spec in specs {
            let key = spec.get("key").and_then(Value::as_str).unwrap_or("");
            let label = spec.get("label").and_then(Value::as_str).unwrap_or("");
            let shown = if label.is_empty() { key } else { label };
            let field_type =
                spec.get("type").and_then(Value::as_str).filter(|t| fieldtypes::is_type(t)).unwrap_or("text");
            let value = values.get(key).map(|v| v.trim().to_string()).unwrap_or_default();
            if spec.get("required").and_then(Value::as_bool).unwrap_or(false) && value.is_empty() {
                errors.push(format!("{shown}: required"));
            } else if let Some(message) = fieldtypes::validate(field_type, &value) {
                errors.push(format!("{shown}: {message}"));
            }
            items.push(FieldInput::new(key, &value, label, field_type));
        }
        if matches!(target, Target::New { name, .. } if name.trim().is_empty()) {
            errors.push("Enter a profile name".into());
        }
        if !errors.is_empty() {
            return Ok(ProfileSave::Problems(errors));
        }
        let mut vault = self.vault();
        let profile_id = match target {
            Target::Existing(id) => vault.get_profile(id).map_err(|e| e.to_string())?.id,
            Target::New { name, kind } => vault.create_profile(name.trim(), kind).map_err(|e| e.to_string())?,
        };
        vault.set_values(profile_id, &items).map_err(|e| e.to_string())?;
        let lengths: IndexMap<String, i64> =
            vault.describe(profile_id).map_err(|e| e.to_string())?.into_iter().map(|f| (f.key, f.length)).collect();
        let fields = items
            .iter()
            .map(|item| {
                json!({
                    "key": item.key,
                    "length": lengths.get(&item.key).copied().unwrap_or(0),
                    "status": if item.value.is_empty() { "skipped" } else { "saved" },
                })
            })
            .collect();
        Ok(ProfileSave::Saved { profile_id, fields })
    }

    // -- uploads -----------------------------------------------------------
    // Files the user picked for a `file` request. MCP reads them only by their `file:<uuid>` id.

    pub fn uploads_dir(&self) -> PathBuf {
        self.config.data_dir.join("uploads")
    }

    /// One folder per relay grant (hashed: grant ids come from the relay), `local` for local clients.
    fn upload_owner_dir(&self, grant: Option<&str>) -> PathBuf {
        let owner = match grant {
            Some(grant) => data_encoding::HEXLOWER.encode(&Sha256::digest(grant.as_bytes())[..16]),
            None => "local".into(),
        };
        self.uploads_dir().join(owner)
    }

    /// Copies the picked file into uploads and resolves the request with its id and size. The file name stays
    /// out of the result (it can be personal data); the returned value adds it for the app.
    pub fn store_upload(&self, request_id: i64, source: &Path) -> Result<Value, String> {
        let request = self.vault().get_request(request_id).map_err(|e| e.to_string())?;
        if request.kind != FILE || request.status != "pending" {
            return Err(format!("Request {request_id} does not wait for a file"));
        }
        let kind = request.payload.get("kind").and_then(Value::as_str).unwrap_or_default();
        let extension = source.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
        if !FILE_KINDS.contains(&kind) || extension != kind {
            return Err(format!("Choose a .{kind} file"));
        }
        let size = std::fs::metadata(source).map_err(|e| e.to_string())?.len();
        if size > UPLOAD_MAX_BYTES {
            return Err(format!("The file is larger than {} MB", UPLOAD_MAX_BYTES / 1024 / 1024));
        }
        let uuid = uuid::Uuid::new_v4();
        let dir = self.upload_owner_dir(request.grant_id.as_deref());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::copy(source, dir.join(format!("{uuid}.{kind}"))).map_err(|e| e.to_string())?;
        let mut result = json!({"outcome": "uploaded", "file_id": format!("file:{uuid}"), "kind": kind, "size": size});
        let mut vault = self.vault();
        vault.resolve_request(request_id, "done", result.clone()).map_err(|e| e.to_string())?;
        let _ = vault.audit("user", "upload", &format!("request={request_id} file={uuid}"));
        result["name"] = json!(source.file_name().and_then(|n| n.to_str()).unwrap_or_default());
        Ok(result)
    }

    /// The upload behind an exact `file:<uuid>` id. A remote agent (grant) only finds files it asked for; local
    /// clients, which can read any path anyway, find all.
    pub fn upload_path(&self, file_id: &str, grant: Option<&str>) -> Result<PathBuf, String> {
        let unknown = || format!("Unknown file id {file_id:?}; call request_file to get one");
        let uuid = file_id
            .strip_prefix("file:")
            .and_then(|s| uuid::Uuid::parse_str(s).ok().filter(|u| u.hyphenated().to_string() == s))
            .ok_or_else(unknown)?;
        let dirs: Vec<PathBuf> = match grant {
            Some(_) => vec![self.upload_owner_dir(grant)],
            None => std::fs::read_dir(self.uploads_dir())
                .map(|entries| entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect())
                .unwrap_or_default(),
        };
        dirs.iter()
            .flat_map(|dir| FILE_KINDS.iter().map(move |extension| dir.join(format!("{uuid}.{extension}"))))
            .find(|path| path.is_file())
            .ok_or_else(unknown)
    }

    // -- metadata (safe for MCP) -------------------------------------------

    pub fn profile_fields(&self, profiles: &Profiles) -> Result<IndexMap<String, Vec<FieldInfo>>, String> {
        let vault = self.vault();
        profiles.iter().map(|(role, id)| Ok((role.clone(), vault.describe(*id).map_err(|e| e.to_string())?))).collect()
    }

    pub fn check_bindings(&self, template_id: &str, profiles: &Profiles) -> Result<Vec<BindingReport>, String> {
        let template = self.templates.get(template_id)?;
        let fields = self.profile_fields(profiles)?;
        let context = if self.vault().unlocked() { Some(self.real_context(profiles)?) } else { None };
        engine::check(&template, &fields, context.as_ref())
    }

    // -- real values (UI only, never returned through MCP) -----------------

    pub fn real_context(&self, profiles: &Profiles) -> Result<Value, String> {
        let vault = self.vault();
        let mut context = Map::new();
        for (role, id) in profiles {
            let values = vault.get_values(*id).map_err(|e| e.to_string())?;
            context.insert(role.clone(), bindings::profile_context(&values));
        }
        Ok(Value::Object(context))
    }

    pub fn fill_templates(&self, ids: &[String]) -> Result<Vec<Template>, String> {
        ids.iter().map(|id| self.templates.get(id)).collect()
    }

    /// Renders every template of a fill set and stores the documents.
    pub fn generate_fill(&self, fill_set_id: i64) -> Result<Vec<i64>, String> {
        let (info, bindings) = self.vault().get_fill_set(fill_set_id).map_err(|e| e.to_string())?;
        let templates = self.fill_templates(&info.templates)?;
        let context = self.real_context(&info.profiles)?;
        let resolved = fill::resolve(&templates, &bindings, &context)?;
        let mut rendered = Vec::new();
        for template in &templates {
            let (values, errors) = &resolved[&template.id];
            if !errors.is_empty() {
                return Err(format!("{}: {}", template.name, errors.join("; ")));
            }
            rendered.push((template, engine::render_values(template, values, Vec::new())?));
        }
        rendered
            .into_iter()
            .map(|(template, output)| {
                self.store(template, &output, &info.profiles, &format!("{} ({})", template.name, info.name))
            })
            .collect()
    }

    fn store(&self, template: &Template, rendered: &Rendered, profiles: &Profiles, title: &str) -> Result<i64, String> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M");
        let pages = if rendered.is_pdf() { preview::page_count(&rendered.content).unwrap_or(0) as i64 } else { 0 };
        let filename = format!("{}-{stamp}.{}", template.id, rendered.extension);
        let mut vault = self.vault();
        let id = vault
            .store_document(&template.id, title, profiles, pages, &filename, &rendered.content)
            .map_err(|e| e.to_string())?;
        let _ = vault.audit("user", "generate", &format!("template={} document={id}", template.id));
        Ok(id)
    }

    pub fn export_fill_set(&self, fill_set_id: i64) -> Result<String, String> {
        let vault = self.vault();
        let (info, bindings) = vault.get_fill_set(fill_set_id).map_err(|e| e.to_string())?;
        let data =
            json!({"name": info.name, "templates": info.templates, "profiles": info.profiles, "bindings": bindings});
        let _ = vault.audit("user", "export_fill_set", &fill_set_id.to_string());
        Ok(yaml_dump(&data))
    }

    /// Zip with each template's manifest + source and the fill set as fill.yaml.
    pub fn export_fill_bundle(&self, fill_set_id: i64, target: &Path) -> Result<(), String> {
        let (info, _) = self.vault().get_fill_set(fill_set_id).map_err(|e| e.to_string())?;
        let yaml = self.export_fill_set(fill_set_id)?;
        let file = std::fs::File::create(target).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for template in self.fill_templates(&info.templates)? {
            for path in [template.directory.join("manifest.yaml"), template.source_path()] {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                zip.start_file(format!("{}/{name}", template.id), options).map_err(|e| e.to_string())?;
                zip.write_all(&std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
        }
        zip.start_file("fill.yaml", options).map_err(|e| e.to_string())?;
        zip.write_all(yaml.as_bytes()).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn import_fill_set(&self, text: &str) -> Result<i64, String> {
        let data: Value = serde_yaml::from_str(text).map_err(|e| format!("Not a fill set: {e}"))?;
        let name = data.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let templates: Vec<String> = data
            .get("templates")
            .and_then(Value::as_array)
            .map(|t| t.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        if name.is_empty() || templates.is_empty() {
            return Err("A fill set needs a name and templates".into());
        }
        self.fill_templates(&templates)?;
        let mut profiles = Profiles::new();
        for (role, id) in data.get("profiles").and_then(Value::as_object).into_iter().flatten() {
            let id = id
                .as_i64()
                .or_else(|| id.as_str().and_then(|s| s.parse().ok()))
                .ok_or("Profile ids must be numbers")?;
            profiles.insert(role.clone(), id);
        }
        let bindings: Bindings = data.get("bindings").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut vault = self.vault();
        for id in profiles.values() {
            vault.get_profile(*id).map_err(|e| e.to_string())?;
        }
        let existing = vault.find_fill_set(&name).map_err(|e| e.to_string())?;
        let id = vault.save_fill_set(&name, &templates, &profiles, &bindings, existing).map_err(|e| e.to_string())?;
        let _ = vault.audit("user", "import_fill_set", &id.to_string());
        Ok(id)
    }
}
