//! The fill form: every blank of one or more templates with a choice of source, next to a preview.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::core::Blankey;
use crate::templates::bindings::{as_text, is_empty};
use crate::templates::fill::{self, FillVar};
use crate::templates::preview::{self, Preview};
use crate::vault::{Profile, Profiles};

#[derive(Deserialize, Default)]
pub struct OpenArgs {
    #[serde(default)]
    pub templates: Vec<String>,
    #[serde(default)]
    pub profiles: Profiles,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub values: IndexMap<String, String>,
    #[serde(default)]
    pub paths: IndexMap<String, String>,
    pub fill_set_id: Option<i64>,
    pub request_id: Option<i64>,
}

#[derive(Serialize)]
pub struct TemplateRef {
    id: String,
    name: String,
    kind: &'static str,
}

#[derive(Serialize)]
pub struct Session {
    request_id: Option<i64>,
    reason: String,
    fill_set_id: Option<i64>,
    name: String,
    templates: Vec<TemplateRef>,
    roles: IndexMap<String, String>,
    profiles: Profiles,
    all_profiles: Vec<Profile>,
    vars: Vec<FillVar>,
    bindings: Map<String, Value>,
    claude_values: IndexMap<String, String>,
}

pub fn open(app: &Blankey, args: OpenArgs) -> Result<Session, String> {
    let mut templates = args.templates;
    let mut profiles = args.profiles;
    let mut name = args.name;
    let vault_fill_set = match args.fill_set_id {
        Some(id) => Some(id),
        None if !name.is_empty() => app.vault().find_fill_set(&name).map_err(|e| e.to_string())?,
        None => None,
    };
    let mut bindings = Map::new();
    if let Some(id) = vault_fill_set {
        let (info, saved) = app.vault().get_fill_set(id).map_err(|e| e.to_string())?;
        if templates.is_empty() {
            templates = info.templates;
        }
        let mut merged = info.profiles;
        merged.extend(profiles);
        profiles = merged;
        if name.is_empty() {
            name = info.name;
        }
        bindings = saved;
    }
    for (var, value) in &args.values {
        bindings.insert(var.clone(), json!({fill::VALUE: value}));
    }
    for (var, path) in &args.paths {
        bindings.insert(var.clone(), json!({fill::PATH: path}));
    }
    let loaded = app.fill_templates(&templates)?;
    let roles = fill::roles(&loaded);
    let role_names: HashSet<String> = roles.keys().cloned().collect();
    let vars = fill::variables(&loaded)?;
    for var in vars.values() {
        if !bindings.contains_key(&var.name) {
            bindings.insert(var.name.clone(), fill::default_binding(var, &role_names));
        }
    }
    let reason = match args.request_id {
        Some(id) => app
            .vault()
            .get_request(id)
            .ok()
            .and_then(|r| r.payload.get("reason").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default(),
        None => String::new(),
    };
    Ok(Session {
        request_id: args.request_id,
        reason,
        fill_set_id: vault_fill_set,
        name,
        templates: loaded
            .iter()
            .map(|t| TemplateRef { id: t.id.clone(), name: t.name.clone(), kind: t.kind.as_str() })
            .collect(),
        roles,
        profiles,
        all_profiles: app.vault().list_profiles().map_err(|e| e.to_string())?,
        vars: vars.into_values().collect(),
        bindings,
        claude_values: args.values,
    })
}

#[derive(Serialize)]
pub struct SourceEntry {
    path: String,
    value: String,
}

#[derive(Serialize)]
pub struct RowState {
    /// the value the current binding gives, as text
    value: String,
    empty: bool,
    /// what the template default gives
    default_value: Option<String>,
}

#[derive(Serialize)]
pub struct Evaluation {
    /// role -> vault fields of its profile with their values
    sources: IndexMap<String, Vec<SourceEntry>>,
    rows: IndexMap<String, RowState>,
    empty: Vec<String>,
}

fn row_value(binding: &Value, context: &Value, var: &FillVar) -> String {
    let expression = var.defaults.values().next().map(String::as_str);
    match fill::evaluate_binding(binding, context, expression) {
        Ok(value) => as_text(&value),
        Err(e) => format!("<{e}>"),
    }
}

pub fn evaluate(
    app: &Blankey,
    templates: &[String],
    profiles: &Profiles,
    bindings: &Map<String, Value>,
) -> Result<Evaluation, String> {
    let loaded = app.fill_templates(templates)?;
    let vars = fill::variables(&loaded)?;
    let context = app.real_context(profiles)?;
    let mut sources = IndexMap::new();
    {
        let vault = app.vault();
        for (role, id) in profiles {
            let entries = vault
                .get_values(*id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|(key, (_, value))| SourceEntry { path: format!("{role}.{key}"), value })
                .collect();
            sources.insert(role.clone(), entries);
        }
    }
    let default = json!({fill::DEFAULT: true});
    let mut rows = IndexMap::new();
    let mut empty = Vec::new();
    for var in vars.values() {
        let binding = bindings.get(&var.name).unwrap_or(&default);
        let value = row_value(binding, &context, var);
        let is_blank = is_empty(&Value::String(value.clone()));
        if is_blank {
            empty.push(var.name.clone());
        }
        let default_value = (!var.defaults.is_empty()).then(|| row_value(&default, &context, var));
        rows.insert(var.name.clone(), RowState { value, empty: is_blank, default_value });
    }
    Ok(Evaluation { sources, rows, empty })
}

/// Previews of every template with the chosen values in place.
pub fn previews(
    app: &Blankey,
    templates: &[String],
    profiles: &Profiles,
    bindings: &Map<String, Value>,
) -> Result<IndexMap<String, Preview>, String> {
    let loaded = app.fill_templates(templates)?;
    let context = app.real_context(profiles)?;
    let resolved = fill::resolve(&loaded, bindings, &context)?;
    Ok(loaded
        .iter()
        .map(|template| {
            let values = resolved.get(&template.id).map(|(values, _)| values);
            (template.id.clone(), preview::preview(template, values))
        })
        .collect())
}

#[derive(Deserialize)]
pub struct SaveArgs {
    pub request_id: Option<i64>,
    pub fill_set_id: Option<i64>,
    pub name: String,
    pub templates: Vec<String>,
    pub profiles: Profiles,
    pub bindings: Map<String, Value>,
    pub generate: bool,
}

#[derive(Serialize)]
pub struct Saved {
    pub fill_set_id: i64,
    pub document_ids: Vec<i64>,
}

pub fn save(app: &Blankey, args: SaveArgs) -> Result<Saved, String> {
    let name = args.name.trim().to_string();
    let mut problems = Vec::new();
    if name.is_empty() {
        problems.push("Enter a fill set name".to_string());
    }
    let loaded = app.fill_templates(&args.templates)?;
    let mut missing: Vec<String> =
        fill::roles(&loaded).into_keys().filter(|r| !args.profiles.contains_key(r)).collect();
    if !missing.is_empty() {
        missing.sort();
        problems.push(format!("Choose profiles for: {}", missing.join(", ")));
    }
    let existing = if name.is_empty() { None } else { app.vault().find_fill_set(&name).map_err(|e| e.to_string())? };
    if existing.is_some() && existing != args.fill_set_id {
        problems.push(format!("A fill set named {name:?} already exists"));
    }
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let fill_set_id = {
        let mut vault = app.vault();
        let id = vault
            .save_fill_set(&name, &args.templates, &args.profiles, &args.bindings, args.fill_set_id)
            .map_err(|e| e.to_string())?;
        let _ = vault.audit("user", "save_fill_set", &id.to_string());
        id
    };
    if !args.generate {
        return Ok(Saved { fill_set_id, document_ids: Vec::new() });
    }
    let document_ids = app.generate_fill(fill_set_id).map_err(|e| format!("Generation failed: {e}"))?;
    if let Some(request_id) = args.request_id {
        let empty = evaluate(app, &args.templates, &args.profiles, &args.bindings).map(|e| e.empty).unwrap_or_default();
        let result =
            json!({"outcome": "generated", "fill_set_id": fill_set_id, "document_ids": document_ids, "empty": empty});
        app.vault().resolve_request(request_id, "done", result).map_err(|e| e.to_string())?;
    }
    Ok(Saved { fill_set_id, document_ids })
}

#[derive(Deserialize)]
pub struct CloseArgs {
    pub request_id: i64,
    pub fill_set_id: Option<i64>,
    pub templates: Vec<String>,
    pub profiles: Profiles,
    pub bindings: Map<String, Value>,
}

/// Closing the form without generating: "saved" when a fill set was saved, else "cancelled".
pub fn close(app: &Blankey, args: CloseArgs) -> Result<(), String> {
    let pending = app.vault().get_request(args.request_id).map(|r| r.status == "pending").unwrap_or(false);
    if !pending {
        return Ok(());
    }
    let (status, result) = match args.fill_set_id {
        Some(id) => {
            let empty =
                evaluate(app, &args.templates, &args.profiles, &args.bindings).map(|e| e.empty).unwrap_or_default();
            ("done", json!({"outcome": "saved", "fill_set_id": id, "document_ids": [], "empty": empty}))
        }
        None => ("cancelled", json!({"outcome": "cancelled"})),
    };
    app.vault().resolve_request(args.request_id, status, result).map_err(|e| e.to_string())
}
