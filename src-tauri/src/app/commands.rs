//! Commands the window calls. Real values only ever go to this window, never through MCP.

use std::sync::atomic::Ordering;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

use super::{fill, platform, tray, AppState};
use crate::core::{FILL, PROFILE_INPUT};
use crate::fieldtypes;
use crate::keychain;
use crate::templates::preview::{self, Preview};
use crate::templates::{docx, TemplateKind};
use crate::vault::{FieldInput, Request};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

const MIN_PASSWORD: usize = 8;

fn check_password(password: &str) -> CmdResult<()> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(format!("The password must be at least {MIN_PASSWORD} characters."));
    }
    Ok(())
}

// -- state & lock ------------------------------------------------------------

#[derive(Serialize)]
pub struct Status {
    initialized: bool,
    unlocked: bool,
    pending: i64,
    keyring_enabled: bool,
    biometric: bool,
    data_dir: String,
    mcp_url: String,
    mcp_running: bool,
    field_types: &'static [&'static str],
}

#[tauri::command]
fn status(state: State<AppState>) -> CmdResult<Status> {
    let app = &state.app;
    let vault = app.vault();
    Ok(Status {
        initialized: vault.initialized(),
        unlocked: vault.unlocked(),
        pending: vault.count_requests("pending").map_err(err)?,
        keyring_enabled: vault.keyring_enabled(),
        biometric: keychain::biometric_available(),
        data_dir: app.config.data_dir.display().to_string(),
        mcp_url: app.config.mcp_url(),
        mcp_running: state.mcp_running.load(Ordering::SeqCst),
        field_types: fieldtypes::TYPES,
    })
}

#[tauri::command]
async fn setup(
    handle: AppHandle,
    state: State<'_, AppState>,
    password: String,
    use_keyring: bool,
) -> CmdResult<String> {
    check_password(&password)?;
    let recovery = state.app.vault().initialize(&password, use_keyring).map_err(err)?;
    state.app.audit("user", "initialize", "");
    tray::refresh(&handle);
    Ok(recovery)
}

#[tauri::command]
async fn unlock(handle: AppHandle, state: State<'_, AppState>, password: String) -> CmdResult<()> {
    state.app.vault().unlock(&password).map_err(|_| "Wrong password.".to_string())?;
    state.app.audit("user", "unlock", "");
    state.touch();
    tray::refresh(&handle);
    Ok(())
}

#[tauri::command]
async fn unlock_recovery(
    handle: AppHandle,
    state: State<'_, AppState>,
    recovery_key: String,
    new_password: String,
) -> CmdResult<()> {
    check_password(&new_password)?;
    state
        .app
        .vault()
        .unlock_with_recovery(&recovery_key, &new_password)
        .map_err(|_| "Invalid recovery key.".to_string())?;
    state.app.audit("user", "unlock_recovery", "");
    state.touch();
    tray::refresh(&handle);
    Ok(())
}

/// Touch ID / keychain; false when it is not set up or was cancelled.
#[tauri::command]
async fn unlock_quick(handle: AppHandle, reason: Option<String>) -> bool {
    let reason = reason.unwrap_or_else(|| "unlock the Blankey vault".into());
    tauri::async_runtime::spawn_blocking(move || super::quick_unlock(&handle, &reason)).await.unwrap_or(false)
}

#[tauri::command]
fn lock(handle: AppHandle) {
    super::lock(&handle);
}

#[tauri::command]
fn touch(state: State<AppState>) {
    state.touch();
}

#[tauri::command]
fn set_busy(state: State<AppState>, busy: bool) {
    state.busy.store(busy, Ordering::SeqCst);
    state.touch();
}

#[tauri::command]
fn copy_text(handle: AppHandle, text: String) -> CmdResult<()> {
    handle.clipboard().write_text(text).map_err(err)
}

// -- profiles ----------------------------------------------------------------

#[tauri::command]
fn list_profiles(state: State<AppState>) -> CmdResult<Vec<crate::vault::Profile>> {
    state.app.vault().list_profiles().map_err(err)
}

#[tauri::command]
fn create_profile(state: State<AppState>, name: String, kind: String) -> CmdResult<i64> {
    let id = state.app.vault().create_profile(name.trim(), kind.trim()).map_err(err)?;
    state.app.audit("user", "create_profile", &id.to_string());
    Ok(id)
}

#[tauri::command]
fn update_profile(state: State<AppState>, id: i64, name: String, kind: String) -> CmdResult<()> {
    state.app.vault().update_profile(id, name.trim(), kind.trim()).map_err(err)
}

#[tauri::command]
fn delete_profile(state: State<AppState>, id: i64) -> CmdResult<()> {
    state.app.vault().delete_profile(id).map_err(err)?;
    state.app.audit("user", "delete_profile", &id.to_string());
    Ok(())
}

#[tauri::command]
fn profile_fields(state: State<AppState>, id: i64) -> CmdResult<Vec<crate::vault::Field>> {
    state.app.vault().fields(id).map_err(err)
}

#[derive(Deserialize)]
struct FieldEdit {
    key: String,
    label: String,
    #[serde(rename = "type")]
    field_type: String,
    value: String,
}

/// Validates every row first; returns the problems (key: message) without saving when there are any.
#[tauri::command]
fn save_fields(
    state: State<AppState>,
    id: i64,
    fields: Vec<FieldEdit>,
    removed: Vec<String>,
) -> CmdResult<Vec<String>> {
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut inputs = Vec::new();
    for f in &fields {
        let key = f.key.trim();
        if key.is_empty() {
            continue;
        }
        if key.starts_with('.') || key.ends_with('.') || key.contains(char::is_whitespace) {
            errors.push(format!("{key}: use dotted words without spaces"));
        }
        if !seen.insert(key.to_string()) {
            errors.push(format!("{key}: duplicate key"));
        }
        let field_type = if fieldtypes::is_type(&f.field_type) { f.field_type.as_str() } else { "text" };
        let value = f.value.trim();
        if let Some(message) = fieldtypes::validate(field_type, value) {
            errors.push(format!("{key}: {message}"));
        }
        inputs.push(FieldInput::new(key, value, f.label.trim(), field_type));
    }
    if !errors.is_empty() {
        return Ok(errors);
    }
    let mut vault = state.app.vault();
    for key in removed.iter().filter(|k| !seen.contains(*k)) {
        vault.delete_field(id, key).map_err(err)?;
    }
    vault.set_values(id, &inputs).map_err(err)?;
    let _ = vault.audit("user", "save_profile", &id.to_string());
    Ok(errors)
}

// -- templates -----------------------------------------------------------------

#[derive(Serialize)]
struct TemplateCard {
    id: String,
    name: String,
    kind: &'static str,
    description: String,
    blanks: usize,
    used_by: Vec<String>,
}

fn blanks(template: &crate::templates::Template) -> usize {
    match template.kind {
        TemplateKind::Docx => docx::variables(&template.source_path()).map(|v| v.len()).unwrap_or(0),
        _ => template.fields.len(),
    }
}

#[tauri::command]
fn list_templates(state: State<AppState>) -> CmdResult<Vec<TemplateCard>> {
    let sets = state.app.vault().list_fill_sets().map_err(err)?;
    Ok(state
        .app
        .templates
        .all()
        .iter()
        .map(|t| TemplateCard {
            id: t.id.clone(),
            name: t.name.clone(),
            kind: t.kind.as_str(),
            description: t.description.clone(),
            blanks: blanks(t),
            used_by: sets.iter().filter(|s| s.templates.contains(&t.id)).map(|s| s.name.clone()).collect(),
        })
        .collect())
}

#[tauri::command]
async fn template_preview(state: State<'_, AppState>, id: String) -> CmdResult<Value> {
    let template = state.app.templates.get(&id)?;
    let card_roles = template.roles.clone();
    let fields = template.fields.clone();
    let labels = template.labels.clone();
    let examples = state.app.templates.examples(&id)?;
    let blank_count = blanks(&template);
    let preview = tauri::async_runtime::spawn_blocking(move || preview::preview(&template, None)).await.map_err(err)?;
    Ok(json!({
        "roles": card_roles, "fields": fields, "labels": labels, "examples": examples,
        "blanks": blank_count, "preview": preview,
    }))
}

#[tauri::command]
fn delete_template(state: State<AppState>, id: String) -> CmdResult<()> {
    state.app.templates.delete(&id)?;
    state.app.audit("user", "delete_template", &id);
    Ok(())
}

#[tauri::command]
fn show_template_folder(state: State<AppState>, id: Option<String>) -> CmdResult<()> {
    let path = match id {
        Some(id) => state.app.templates.get(&id)?.directory,
        None => state.app.templates.root().to_path_buf(),
    };
    platform::reveal(&path)
}

// -- documents -----------------------------------------------------------------

#[tauri::command]
fn list_documents(state: State<AppState>) -> CmdResult<Vec<crate::vault::DocumentInfo>> {
    state.app.vault().list_documents().map_err(err)
}

#[tauri::command]
fn open_documents(state: State<AppState>, ids: Vec<i64>) -> CmdResult<()> {
    for id in ids {
        let (filename, content) = state.app.vault().load_document(id).map_err(err)?;
        platform::open_document(&filename, &content)?;
        state.app.audit("user", "open_document", &id.to_string());
    }
    Ok(())
}

#[tauri::command]
async fn export_document(handle: AppHandle, state: State<'_, AppState>, id: i64) -> CmdResult<Option<String>> {
    let (filename, content) = state.app.vault().load_document(id).map_err(err)?;
    let target = handle.dialog().file().set_title("Export").set_file_name(&filename).blocking_save_file();
    let Some(target) = target.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    std::fs::write(&target, content).map_err(err)?;
    state.app.audit("user", "export_document", &id.to_string());
    Ok(Some(target.display().to_string()))
}

#[tauri::command]
fn delete_documents(state: State<AppState>, ids: Vec<i64>) -> CmdResult<()> {
    let vault = state.app.vault();
    for id in ids {
        vault.delete_document(id).map_err(err)?;
        let _ = vault.audit("user", "delete_document", &id.to_string());
    }
    Ok(())
}

// -- fill sets -----------------------------------------------------------------

#[derive(Serialize)]
struct FillSetRow {
    id: i64,
    name: String,
    templates: Vec<String>,
    profiles: Vec<String>,
    updated_at: String,
}

#[tauri::command]
fn list_fill_sets(state: State<AppState>) -> CmdResult<Vec<FillSetRow>> {
    let vault = state.app.vault();
    let names: IndexMap<i64, String> =
        vault.list_profiles().map_err(err)?.into_iter().map(|p| (p.id, p.name)).collect();
    Ok(vault
        .list_fill_sets()
        .map_err(err)?
        .into_iter()
        .map(|info| FillSetRow {
            id: info.id,
            name: info.name,
            templates: info.templates,
            profiles: info
                .profiles
                .values()
                .map(|id| names.get(id).cloned().unwrap_or_else(|| format!("#{id}")))
                .collect(),
            updated_at: info.updated_at,
        })
        .collect())
}

#[tauri::command]
async fn generate_fill_set(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<i64>> {
    let app = state.app.clone();
    let ids = tauri::async_runtime::spawn_blocking(move || app.generate_fill(id)).await.map_err(err)??;
    for doc in &ids {
        let (filename, content) = state.app.vault().load_document(*doc).map_err(err)?;
        platform::open_document(&filename, &content)?;
    }
    Ok(ids)
}

#[tauri::command]
async fn export_fill_set(
    handle: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    bundle: bool,
) -> CmdResult<Option<String>> {
    let (info, _) = state.app.vault().get_fill_set(id).map_err(err)?;
    let extension = if bundle { "zip" } else { "yaml" };
    let target = handle
        .dialog()
        .file()
        .set_title("Export")
        .set_file_name(format!("{}.{extension}", info.name))
        .blocking_save_file();
    let Some(target) = target.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    if bundle {
        state.app.export_fill_bundle(id, &target)?;
    } else {
        std::fs::write(&target, state.app.export_fill_set(id)?).map_err(err)?;
    }
    Ok(Some(target.display().to_string()))
}

#[tauri::command]
async fn import_fill_set(handle: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<i64>> {
    let source =
        handle.dialog().file().set_title("Import fill set").add_filter("YAML", &["yaml", "yml"]).blocking_pick_file();
    let Some(source) = source.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    let text = std::fs::read_to_string(&source).map_err(err)?;
    state.app.import_fill_set(&text).map(Some).map_err(|e| format!("Import failed: {e}"))
}

#[tauri::command]
fn delete_fill_set(state: State<AppState>, id: i64) -> CmdResult<()> {
    state.app.vault().delete_fill_set(id).map_err(err)?;
    state.app.audit("user", "delete_fill_set", &id.to_string());
    Ok(())
}

// -- requests from Claude --------------------------------------------------------

fn request_label(kind: &str) -> &str {
    match kind {
        PROFILE_INPUT => "Profile data",
        "generate" => "Generate document",
        FILL => "Fill documents",
        other => other,
    }
}

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// One-line outcome of a request, from its metadata-only result.
fn request_outcome(request: &Request) -> String {
    let result = request.result.clone().unwrap_or_else(|| json!({}));
    let outcome = result.get("outcome").and_then(Value::as_str).unwrap_or("");
    if request.status == "pending" {
        return "Waiting for you".into();
    }
    if request.status == "cancelled" || outcome == "cancelled" {
        return "Cancelled".into();
    }
    match request.kind.as_str() {
        PROFILE_INPUT => {
            let fields = result.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
            let saved = fields.iter().filter(|f| f.get("status").and_then(Value::as_str) == Some("saved")).count();
            let skipped = fields.len() - saved;
            let mut text = format!("Saved {}", plural(saved, "field"));
            if skipped > 0 {
                text.push_str(&format!(", {skipped} skipped"));
            }
            text
        }
        "generate" => "Document generated".into(),
        FILL => {
            let documents = result.get("document_ids").and_then(Value::as_array).map_or(0, Vec::len);
            let mut text = if documents > 0 {
                format!("{} generated", plural(documents, "document"))
            } else {
                "Fill set saved".into()
            };
            let empty = result.get("empty").and_then(Value::as_array).map_or(0, Vec::len);
            if empty > 0 {
                text.push_str(&format!(", {empty} empty"));
            }
            text
        }
        _ => {
            if outcome.is_empty() {
                request.status.clone()
            } else {
                outcome.to_string()
            }
        }
    }
}

fn request_details(request: &Request, names: &IndexMap<String, String>) -> String {
    let payload = &request.payload;
    if let Some(reason) = payload.get("reason").and_then(Value::as_str).filter(|r| !r.is_empty()) {
        return reason.to_string();
    }
    match request.kind.as_str() {
        PROFILE_INPUT => plural(payload.get("fields").and_then(Value::as_array).map_or(0, Vec::len), "field"),
        FILL => {
            let templates: Vec<String> = payload
                .get("templates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|t| names.get(t).cloned().unwrap_or_else(|| t.to_string()))
                .collect();
            format!("{} · {}", templates.join(", "), payload.get("name").and_then(Value::as_str).unwrap_or(""))
        }
        _ => String::new(),
    }
}

#[derive(Serialize)]
struct ActivityRow {
    id: i64,
    kind: String,
    label: String,
    details: String,
    outcome: String,
    time: String,
    status: String,
    fill_set_id: Option<i64>,
    document_ids: Vec<i64>,
}

/// Everything Claude asked for, waiting requests first. Metadata only.
#[tauri::command]
fn list_activity(state: State<AppState>) -> CmdResult<Vec<ActivityRow>> {
    let names: IndexMap<String, String> = state.app.templates.all().into_iter().map(|t| (t.id, t.name)).collect();
    let vault = state.app.vault();
    let fill_sets: Vec<i64> = vault.list_fill_sets().map_err(err)?.iter().map(|f| f.id).collect();
    let documents: Vec<i64> = vault.list_documents().map_err(err)?.iter().map(|d| d.id).collect();
    let mut requests = vault.list_requests(None).map_err(err)?;
    requests.sort_by_key(|r| (r.status != "pending", -r.id));
    Ok(requests
        .iter()
        .map(|r| {
            let result = r.result.clone().unwrap_or_default();
            let fill_set_id = result.get("fill_set_id").and_then(Value::as_i64).filter(|id| fill_sets.contains(id));
            let mut document_ids: Vec<i64> = result
                .get("document_ids")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
                .collect();
            document_ids.extend(result.get("document_id").and_then(Value::as_i64));
            document_ids.retain(|id| documents.contains(id));
            ActivityRow {
                id: r.id,
                kind: r.kind.clone(),
                label: request_label(&r.kind).to_string(),
                details: request_details(r, &names),
                outcome: request_outcome(r),
                time: r.resolved_at.clone().unwrap_or_else(|| r.created_at.clone()),
                status: r.status.clone(),
                fill_set_id,
                document_ids,
            }
        })
        .collect())
}

#[derive(Serialize)]
struct RequestDetail {
    request: Request,
    profile_name: Option<String>,
    /// key -> current value of the target profile
    existing: IndexMap<String, String>,
}

/// The oldest pending request with its payload (needs the vault unlocked); None when nothing waits.
#[tauri::command]
fn next_request(state: State<AppState>) -> CmdResult<Option<RequestDetail>> {
    let vault = state.app.vault();
    let pending = vault.list_requests(Some("pending")).map_err(err)?;
    let Some(request) = pending.into_iter().next() else { return Ok(None) };
    if request.hidden {
        return Err("Vault is locked".into());
    }
    if request.kind == "generate" {
        // retired: documents are generated from fill sets only
        drop(vault);
        state.app.vault().resolve_request(request.id, "cancelled", json!({"outcome": "cancelled"})).map_err(err)?;
        return next_request(state);
    }
    let profile_id = request.payload.get("profile_id").and_then(Value::as_i64);
    let profile_name = profile_id.and_then(|id| vault.get_profile(id).ok()).map(|p| p.name);
    let existing = match profile_id {
        Some(id) if request.kind == PROFILE_INPUT => {
            vault.get_values(id).map_err(err)?.into_iter().map(|(key, (_, value))| (key, value)).collect()
        }
        _ => IndexMap::new(),
    };
    Ok(Some(RequestDetail { request, profile_name, existing }))
}

#[tauri::command]
fn cancel_request(handle: AppHandle, state: State<AppState>, id: i64) -> CmdResult<()> {
    state.app.vault().resolve_request(id, "cancelled", json!({"outcome": "cancelled"})).map_err(err)?;
    tray::refresh(&handle);
    Ok(())
}

/// Saves what the user typed for a profile_input request. Returns problems without saving when there are any.
#[tauri::command]
fn resolve_profile_input(
    handle: AppHandle,
    state: State<AppState>,
    id: i64,
    profile_name: Option<String>,
    values: IndexMap<String, String>,
) -> CmdResult<Vec<String>> {
    let app = &state.app;
    let request = app.vault().get_request(id).map_err(err)?;
    let specs = request.payload.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut errors = Vec::new();
    let mut items = Vec::new();
    for spec in &specs {
        let key = spec.get("key").and_then(Value::as_str).unwrap_or("");
        let label = spec.get("label").and_then(Value::as_str).unwrap_or("");
        let shown = if label.is_empty() { key } else { label };
        let field_type = spec.get("type").and_then(Value::as_str).filter(|t| fieldtypes::is_type(t)).unwrap_or("text");
        let value = values.get(key).map(|v| v.trim().to_string()).unwrap_or_default();
        if spec.get("required").and_then(Value::as_bool).unwrap_or(false) && value.is_empty() {
            errors.push(format!("{shown}: required"));
        } else if let Some(message) = fieldtypes::validate(field_type, &value) {
            errors.push(format!("{shown}: {message}"));
        }
        items.push(FieldInput::new(key, &value, label, field_type));
    }
    let mut profile_id = request.payload.get("profile_id").and_then(Value::as_i64);
    let new_name = profile_name.unwrap_or_default().trim().to_string();
    if profile_id.is_none() && new_name.is_empty() {
        errors.push("Enter a profile name".into());
    }
    if !errors.is_empty() {
        return Ok(errors);
    }
    let mut vault = app.vault();
    if profile_id.is_none() {
        let kind = request.payload.get("new_profile").and_then(|p| p.get("kind")).and_then(Value::as_str).unwrap_or("");
        profile_id = Some(vault.create_profile(&new_name, kind).map_err(err)?);
    }
    let profile_id = profile_id.expect("profile chosen");
    vault.set_values(profile_id, &items).map_err(err)?;
    let lengths: IndexMap<String, i64> =
        vault.describe(profile_id).map_err(err)?.into_iter().map(|f| (f.key, f.length)).collect();
    let statuses: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "key": item.key,
                "length": lengths.get(&item.key).copied().unwrap_or(0),
                "status": if item.value.is_empty() { "skipped" } else { "saved" },
            })
        })
        .collect();
    vault
        .resolve_request(id, "done", json!({"outcome": "saved", "profile_id": profile_id, "fields": statuses}))
        .map_err(err)?;
    let _ = vault.audit("user", "profile_input", &format!("request={id} profile={profile_id}"));
    drop(vault);
    tray::refresh(&handle);
    let _ = handle.emit("requests-changed", ());
    Ok(Vec::new())
}

// -- fill form -------------------------------------------------------------------

#[tauri::command]
async fn fill_open(state: State<'_, AppState>, args: fill::OpenArgs) -> CmdResult<fill::Session> {
    let app = state.app.clone();
    tauri::async_runtime::spawn_blocking(move || fill::open(&app, args)).await.map_err(err)?
}

#[derive(Deserialize)]
struct FillState {
    templates: Vec<String>,
    profiles: crate::vault::Profiles,
    bindings: serde_json::Map<String, Value>,
}

#[tauri::command]
async fn fill_evaluate(state: State<'_, AppState>, args: FillState) -> CmdResult<fill::Evaluation> {
    let app = state.app.clone();
    tauri::async_runtime::spawn_blocking(move || fill::evaluate(&app, &args.templates, &args.profiles, &args.bindings))
        .await
        .map_err(err)?
}

#[tauri::command]
async fn fill_preview(state: State<'_, AppState>, args: FillState) -> CmdResult<IndexMap<String, Preview>> {
    let app = state.app.clone();
    tauri::async_runtime::spawn_blocking(move || fill::previews(&app, &args.templates, &args.profiles, &args.bindings))
        .await
        .map_err(err)?
}

#[tauri::command]
async fn fill_save(handle: AppHandle, state: State<'_, AppState>, args: fill::SaveArgs) -> CmdResult<fill::Saved> {
    let app = state.app.clone();
    let generate = args.generate;
    let saved = tauri::async_runtime::spawn_blocking(move || fill::save(&app, args)).await.map_err(err)??;
    if generate {
        for doc in &saved.document_ids {
            let (filename, content) = state.app.vault().load_document(*doc).map_err(err)?;
            platform::open_document(&filename, &content)?;
        }
    }
    tray::refresh(&handle);
    let _ = handle.emit("requests-changed", ());
    Ok(saved)
}

#[tauri::command]
fn fill_close(handle: AppHandle, state: State<AppState>, args: fill::CloseArgs) -> CmdResult<()> {
    fill::close(&state.app, args)?;
    tray::refresh(&handle);
    let _ = handle.emit("requests-changed", ());
    Ok(())
}

pub fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        status,
        setup,
        unlock,
        unlock_recovery,
        unlock_quick,
        lock,
        touch,
        set_busy,
        copy_text,
        list_profiles,
        create_profile,
        update_profile,
        delete_profile,
        profile_fields,
        save_fields,
        list_templates,
        template_preview,
        delete_template,
        show_template_folder,
        list_documents,
        open_documents,
        export_document,
        delete_documents,
        list_fill_sets,
        generate_fill_set,
        export_fill_set,
        import_fill_set,
        delete_fill_set,
        list_activity,
        next_request,
        cancel_request,
        resolve_profile_input,
        fill_open,
        fill_evaluate,
        fill_preview,
        fill_save,
        fill_close,
    ]
}
