//! Application service shared by the MCP server (metadata + examples) and the UI (real values).

use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, RwLock};

use indexmap::IndexMap;
use serde_json::{json, Map, Value};

use crate::config::Config;
use crate::keychain::SecretStore;
use crate::render::preview;
use crate::templates::engine::{self, BindingReport, Rendered};
use crate::templates::{bindings, fill, yaml_dump, Template, TemplateStore};
use crate::vault::{Bindings, FieldInfo, Profiles, Vault};

pub const PROFILE_INPUT: &str = "profile_input";
pub const FILL: &str = "fill";

type RequestHook = Box<dyn Fn(i64) + Send + Sync>;

pub struct Blankey {
    pub config: Config,
    vault: Mutex<Vault>,
    pub templates: TemplateStore,
    on_request: RwLock<RequestHook>,
}

impl Blankey {
    pub fn new(config: Config, secrets: Box<dyn SecretStore>) -> Result<Self, String> {
        let vault = Vault::open(&config.db_path(), secrets).map_err(|e| e.to_string())?;
        let templates = TemplateStore::new(config.templates_dir());
        Ok(Self { config, vault: Mutex::new(vault), templates, on_request: RwLock::new(Box::new(|_| {})) })
    }

    pub fn vault(&self) -> MutexGuard<'_, Vault> {
        self.vault.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_request_hook(&self, hook: impl Fn(i64) + Send + Sync + 'static) {
        *self.on_request.write().unwrap() = Box::new(hook);
    }

    pub fn audit(&self, actor: &str, action: &str, target: &str) {
        let _ = self.vault().audit(actor, action, target);
    }

    // -- requests ----------------------------------------------------------

    pub fn create_request(&self, kind: &str, payload: Value) -> Result<i64, String> {
        let id = {
            let vault = self.vault();
            let id = vault.create_request(kind, &payload).map_err(|e| e.to_string())?;
            let _ = vault.audit("mcp", &format!("request:{kind}"), &id.to_string());
            id
        };
        (self.on_request.read().unwrap())(id);
        Ok(id)
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
