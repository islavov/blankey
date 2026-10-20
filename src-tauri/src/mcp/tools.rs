//! MCP tools. Nothing here may return a vault value or real-data document bytes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Extensions, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::core::{Blankey, FILE, FILE_KINDS, FILL, LIBRARY_SYNC_AGE, PROFILE_INPUT};
use crate::fieldtypes;
use crate::library::{self, LibraryFile};
use crate::links::{link_state, LinkDraft, SpecField, COLLECTION_LINK, DEFAULT_MAX_FILES, LINK_SUBMISSION};
use crate::mcp::{find, remote, Remote};
use crate::render::{pdf_form, preview};
use crate::templates::bindings::example_context;
use crate::templates::engine::{self, dump};
use crate::templates::{docx, fill, Origin, SaveTemplate, StrMap, Template, TemplateKind, ID_RE};
use crate::vault::{FieldInfo, Profile, Profiles, Request};

pub const INSTRUCTIONS: &str = r#"Blankey fills document templates from personal data in an encrypted local vault. You never see values,
only profiles' field keys, labels, types and value lengths: work from profile ids, never ask for values in chat.
A locked vault: tools wait up to 50 s for the user to unlock it; if still locked, ask them to and retry.

Filling, in one request_fill if you can: find_templates, find_profiles with the keys the template needs, then
request_fill with a source for every var; wait_request returns the documents. copy_fill_set_id repeats a fill for
another party. request_profile_input only to keep data in a profile for later.

New template, named for the kind of document and party (not this case): inspect_pdf, then save_template (form)
or save_pdf_overlay_template (flat); inspect_docx, then save_docx_template; or save_template with Typst
source_text. Bindings are Jinja in {{ }}: "{{ applicant.address.city }}", "{{ loan.amount | money }}".
Check it with render_example and check_bindings.
find_templates also lists the user's own files (status candidate, source lib:<id>): templatize one only when a fill
needs it, passing lib:<id> as path and source_path. Blankey rebuilds the template when the file changes.
"#;

/// Added to the instructions when the user is signed in to Blankey Online.
pub const LINK_INSTRUCTIONS: &str = r#"Collection links collect data someone else holds: create_collection_link; once the user confirms it,
wait_request returns the url to send. list_collection_links shows submissions and the profiles they went into.
"#;

pub const TOOL_TIMEOUT_MAX: u64 = 1800;
/// Relays drop long requests; agents call wait_request again while pending.
pub const REMOTE_TIMEOUT_MAX: u64 = 50;

type ToolResult = Result<CallToolResult, String>;

fn ok(content: Vec<ContentBlock>) -> ToolResult {
    Ok(CallToolResult::success(content))
}

fn text(value: &impl serde::Serialize) -> ToolResult {
    ok(vec![ContentBlock::text(dump(value))])
}

fn raw(body: String) -> ToolResult {
    ok(vec![ContentBlock::text(body)])
}

fn png(bytes: Vec<u8>) -> ContentBlock {
    ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(bytes), "image/png")
}

pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// A local path, a `file:<uuid>` id from request_file or a `lib:<id>` file from find_templates; remote sessions
/// only get ids.
fn source_path(app: &Blankey, remote: Option<&Remote>, path: &str) -> Result<PathBuf, String> {
    if path.starts_with("lib:") {
        app.library.path(path)
    } else if path.starts_with("file:") {
        app.upload_path(path, remote.map(|r| r.grant.as_str()))
    } else if remote.is_some() {
        Err(format!(
            "Remote sessions cannot read local paths ({path}): call request_file and pass the file id it returns"
        ))
    } else {
        Ok(expand(path))
    }
}

/// Where a saved template's source came from: a library file with its current hash, nothing for another source,
/// the existing origin when the source stays.
fn origin(
    app: &Blankey,
    source: Option<&str>,
    recipe: Vec<Value>,
    existing: Option<&Template>,
) -> Result<Option<Origin>, String> {
    match source {
        Some(file) if file.starts_with("lib:") => {
            let sha256 = library::hash_file(&app.library.path(file)?).map_err(string)?;
            Ok(Some(Origin { file: file.to_string(), sha256, recipe }))
        }
        Some(_) => Ok(None),
        None => Ok(existing.and_then(|t| t.origin.clone())),
    }
}

fn read_file(app: &Blankey, remote: Option<&Remote>, path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(source_path(app, remote, path)?).map_err(|e| format!("{path}: {e}"))
}

/// A find_templates hit: a template, or a library file nobody templatized yet.
enum Found<'a> {
    Template(&'a Template),
    File(&'a LibraryFile),
}

fn template_status(template: &Template) -> &'static str {
    if template.stale.is_empty() {
        "template"
    } else {
        "stale"
    }
}

/// What an agent does next with a collection link; it never gets the submitted values.
fn next_step(received: i64, pending: usize, saved_into: &[i64]) -> String {
    let saved = match saved_into {
        [] => String::new(),
        [id] => format!("The values are saved in profile {id}"),
        ids => format!("The values are saved in profiles {ids:?}"),
    };
    let use_them = "use find_profiles with ids and with_fields, then request_fill with paths mapping each template var to these keys. \
                    Do not ask the user to paste the values.";
    match (pending, saved.is_empty()) {
        (0, true) if received == 0 => "Nothing received yet.".into(),
        (0, true) => "Received, but not saved into a profile (discarded or still in Blankey).".into(),
        (0, false) => format!("{saved}: {use_them}"),
        (n, true) => format!("{n} waiting: ask the user to review it in Blankey and save it into a profile."),
        (n, false) => format!("{n} waiting for the user to review in Blankey. {saved}: {use_them}"),
    }
}

/// Remote agents only see their own requests; nobody sees submissions to collection links.
fn visible(request: Request, remote: Option<&Remote>) -> Result<Request, String> {
    if request.kind == LINK_SUBMISSION {
        return Err(format!("Unknown request {}", request.id));
    }
    match remote {
        Some(r) if request.grant_id.as_deref() != Some(r.grant.as_str()) => {
            Err(format!("Unknown request {}", request.id))
        }
        _ => Ok(request),
    }
}

fn wait_timeout(requested: Option<i64>, remote: bool) -> u64 {
    let max = if remote { REMOTE_TIMEOUT_MAX } else { TOOL_TIMEOUT_MAX };
    requested.unwrap_or(300).clamp(0, max as i64) as u64
}

fn string(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn dpi(value: Option<i64>) -> f32 {
    value.unwrap_or(80).clamp(30, 300) as f32
}

fn check_id(id: &str) -> Result<(), String> {
    if ID_RE.is_match(id) {
        Ok(())
    } else {
        Err("Template id must be lowercase letters, digits, '-' or '_'".into())
    }
}

/// The fields under a key: the key itself, or its nested keys ("address" holds "address.city").
fn key_fields<'a>(fields: &'a [FieldInfo], key: &str) -> Vec<&'a FieldInfo> {
    let prefix = format!("{key}.");
    fields.iter().filter(|i| !key.is_empty() && (i.key == key || i.key.starts_with(&prefix))).collect()
}

fn request_view(request: &Request) -> Value {
    json!({
        "id": request.id,
        "kind": request.kind,
        "status": request.status,
        "result": request.result,
        "created_at": request.created_at,
        "resolved_at": request.resolved_at,
    })
}

fn tempfile_path(extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!("blankey-{}.{extension}", uuid::Uuid::new_v4()))
}

/// Agents send loosely typed JSON: numbers as strings, non-string map values, nulls for absent arguments.
mod lenient {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    use crate::templates::StrMap;
    use crate::vault::Profiles;

    fn as_int(v: &Value) -> Option<i64> {
        v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    }

    pub fn int<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        opt_int(d)?.ok_or_else(|| D::Error::custom("expected an integer"))
    }

    pub fn opt_int<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            v => as_int(&v).map(Some).ok_or_else(|| D::Error::custom(format!("expected an integer, got {v}"))),
        }
    }

    pub fn str_map<'de, D: Deserializer<'de>>(d: D) -> Result<StrMap, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(StrMap::new()),
            Value::Object(map) => Ok(map
                .into_iter()
                .map(|(k, v)| (k, v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
                .collect()),
            _ => Err(D::Error::custom("expected an object of strings")),
        }
    }

    pub fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<i64>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(Vec::new()),
            Value::Array(items) => items
                .iter()
                .map(|v| as_int(v).ok_or_else(|| D::Error::custom(format!("expected an id, got {v}"))))
                .collect(),
            v => as_int(&v).map(|id| vec![id]).ok_or_else(|| D::Error::custom("expected a list of ids")),
        }
    }

    pub fn profiles<'de, D: Deserializer<'de>>(d: D) -> Result<Profiles, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(Profiles::new()),
            Value::Object(map) => map
                .into_iter()
                .map(|(role, id)| {
                    let id = as_int(&id).ok_or_else(|| D::Error::custom(format!("{role} must be a profile id")))?;
                    Ok((role, id))
                })
                .collect(),
            _ => Err(D::Error::custom("expected an object mapping roles to profile ids")),
        }
    }
}

// -- arguments ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct FindProfilesArgs {
    /// Words to match in the name, kind, field keys and labels.
    query: Option<String>,
    kind: Option<String>,
    /// Field keys the profile must hold ("address" also matches "address.city").
    #[serde(default)]
    keys: Vec<String>,
    /// With keys: only profiles where they are filled. Default true.
    filled: Option<bool>,
    #[serde(default, deserialize_with = "lenient::ids")]
    ids: Vec<i64>,
    /// List every field (key, label, type, length) instead of counts; for a few profiles, not hundreds.
    #[serde(default)]
    with_fields: bool,
    /// Default 20, at most 100.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    offset: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindTemplatesArgs {
    /// Words to match in the id, name, description, roles, vars and labels; for the user's own files in the
    /// path, title, category and tags.
    query: Option<String>,
    #[schemars(extend("enum" = ["pdf_form", "typst", "docx"]))]
    kind: Option<String>,
    /// template, stale (its library file changed and the rebuild failed) or candidate (a file not templatized yet).
    #[schemars(extend("enum" = ["template", "stale", "candidate"]))]
    status: Option<String>,
    /// Only templates with this role.
    role: Option<String>,
    /// Only templates with this var.
    var: Option<String>,
    /// Default 20, at most 100.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    offset: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindFillSetsArgs {
    /// Words to match in the name, template ids and names, and profile names.
    query: Option<String>,
    template_id: Option<String>,
    /// Only fill sets that use this profile for any role.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    profile_id: Option<i64>,
    /// Default 20, at most 100.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    offset: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FillSetArgs {
    #[serde(deserialize_with = "lenient::int")]
    fill_set_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindDocumentsArgs {
    /// Words to match in the title, file name and template id.
    query: Option<String>,
    template_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    profile_id: Option<i64>,
    /// Only documents made on or after this date (YYYY-MM-DD).
    since: Option<String>,
    /// Default 20, at most 100.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    offset: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListLinksArgs {
    /// Words to match in the title.
    query: Option<String>,
    /// open, full, expired or revoked
    status: Option<String>,
    /// Default 20, at most 100.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    offset: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TemplateArgs {
    template_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct PathArgs {
    /// A local path, a file id from request_file, or a lib:<id> from find_templates.
    path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct InspectPdfArgs {
    /// A local path, a file id from request_file, or a lib:<id> from find_templates.
    path: String,
    /// 1-based pages to render: a form's pages with its fields outlined (default none), a flat PDF's pages
    /// with the grid (default all).
    pages: Option<Vec<usize>>,
    /// Image resolution, default 80 (110 for outlined form pages).
    #[serde(default, deserialize_with = "lenient::opt_int")]
    dpi: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SavePdfOverlayArgs {
    template_id: String,
    name: String,
    /// [{name, page, rect: [x0, y0, x1, y1], type: text|check, max_len?, comb?}]
    boxes: Vec<Map<String, Value>>,
    #[serde(deserialize_with = "lenient::str_map")]
    roles: StrMap,
    #[serde(deserialize_with = "lenient::str_map")]
    fields: StrMap,
    /// A local path, a file id from request_file, or a lib:<id> from find_templates.
    source_path: Option<String>,
    #[serde(default, deserialize_with = "lenient::str_map")]
    labels: StrMap,
    description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SaveTemplateArgs {
    template_id: String,
    name: String,
    #[schemars(extend("enum" = ["pdf_form", "typst", "docx"]))]
    kind: String,
    #[serde(deserialize_with = "lenient::str_map")]
    fields: StrMap,
    #[serde(deserialize_with = "lenient::str_map")]
    roles: StrMap,
    description: Option<String>,
    /// A local path, a file id from request_file, or a lib:<id> from find_templates.
    source_path: Option<String>,
    source_text: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SaveDocxArgs {
    template_id: String,
    name: String,
    /// A local path, a file id from request_file, or a lib:<id> from find_templates.
    source_path: String,
    /// [{find, var, occurrence?}]
    replacements: Vec<Map<String, Value>>,
    #[serde(deserialize_with = "lenient::str_map")]
    roles: StrMap,
    #[serde(deserialize_with = "lenient::str_map")]
    fields: StrMap,
    #[serde(default, deserialize_with = "lenient::str_map")]
    labels: StrMap,
    description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RenderExampleArgs {
    template_id: String,
    /// A saved example name or an inline dict keyed by role.
    example: Value,
    /// Keep an inline example under this name for later renders.
    save_as: Option<String>,
    /// 1-based pages; default all.
    pages: Option<Vec<usize>>,
    /// Image resolution, default 80.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    dpi: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CheckBindingsArgs {
    template_id: String,
    /// {role: profile_id}
    #[serde(deserialize_with = "lenient::profiles")]
    profiles: Profiles,
}

#[derive(Deserialize, JsonSchema)]
pub struct RequestProfileInputArgs {
    /// [{key, label, type, required, hint}]
    fields: Vec<Map<String, Value>>,
    reason: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    profile_id: Option<i64>,
    new_profile_name: Option<String>,
    new_profile_kind: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RequestFillArgs {
    templates: Vec<String>,
    /// {role: profile_id} for every role.
    #[serde(deserialize_with = "lenient::profiles")]
    profiles: Profiles,
    /// Name of the fill set.
    name: String,
    /// Non-personal literals per var.
    #[serde(default, deserialize_with = "lenient::str_map")]
    values: StrMap,
    /// Vault fields per var: "role.key" for a role's profile, or "profile:ID.key" for any profile.
    #[serde(default, deserialize_with = "lenient::str_map")]
    paths: StrMap,
    /// var -> short hint: the user types these in the form; the value stays in this fill set only.
    #[serde(default, deserialize_with = "lenient::str_map")]
    ask: StrMap,
    /// Reopen this fill set and save over it.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    fill_set_id: Option<i64>,
    /// Start from this fill set (its profiles, sources and typed values) and save a new one under name.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    copy_fill_set_id: Option<i64>,
    reason: Option<String>,
    /// Open the form even though some vars fall back to a template default that gives no vault value.
    #[serde(default)]
    allow_defaults: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct RequestFileArgs {
    #[schemars(extend("enum" = ["pdf", "docx"]))]
    kind: String,
    /// Shown to the user: which document you need and what for.
    reason: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateCollectionLinkArgs {
    /// The form's title, shown to the person who fills it in.
    title: String,
    /// Shown above the form.
    note: Option<String>,
    /// Who is asking, shown next to the user's account email.
    from: Option<String>,
    /// The profile the answers go into; or new_profile_name.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    profile_id: Option<i64>,
    new_profile_name: Option<String>,
    /// [{key, label, type, required, hint}]
    #[serde(default)]
    fields: Vec<SpecField>,
    /// Files to ask for, 0 to 10 (default 5), up to 50 MB each.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    max_files: Option<i64>,
    /// Allowed file kinds as extensions or types (".pdf", "image/*"); empty allows any.
    #[serde(default)]
    kinds: Vec<String>,
    /// 1 to 90, default 7.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    expires_days: Option<i64>,
    /// 1 to 100, default 1.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    max_submissions: Option<i64>,
    /// Shown to the user when confirming the link.
    reason: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitRequestArgs {
    #[serde(deserialize_with = "lenient::int")]
    request_id: i64,
    /// Seconds, default 300; at most 50 over Blankey Online.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    timeout_s: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListRequestsArgs {
    /// pending, done or cancelled
    status: Option<String>,
    /// Default 20, at most 100.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    offset: Option<i64>,
}

// -- tools -------------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Tools {
    app: Arc<Blankey>,
    unlock_wait: Duration,
}

/// Tools that need a Blankey Online sign-in; hidden from the tool list without one.
const ONLINE_TOOLS: &[&str] = &["create_collection_link", "list_collection_links"];

impl Tools {
    pub fn new(app: Arc<Blankey>) -> Self {
        Self { app, unlock_wait: UNLOCK_WAIT }
    }

    fn online(&self) -> bool {
        self.app.vault().relay_account().ok().flatten().is_some()
    }

    fn router(&self) -> &'static ToolRouter<Self> {
        static ALL: OnceLock<ToolRouter<Tools>> = OnceLock::new();
        static OFFLINE: OnceLock<ToolRouter<Tools>> = OnceLock::new();
        if self.online() {
            ALL.get_or_init(Self::tool_router)
        } else {
            OFFLINE.get_or_init(|| {
                let mut router = Self::tool_router();
                for name in ONLINE_TOOLS {
                    router.remove_route(name);
                }
                router
            })
        }
    }

    /// Reading or changing data needs the vault unlocked: a locked vault opens Blankey's unlock screen and the
    /// call waits for the user. A client can still connect, list the tools and send requests while locked.
    async fn run(&self, f: impl FnOnce(&Arc<Blankey>) -> ToolResult + Send + 'static) -> ToolResult {
        let app = self.app.clone();
        let wait = self.unlock_wait;
        let unlocked = tokio::task::spawn_blocking(move || app.wait_for_unlock(UNLOCK_REASON, wait)).await;
        if !unlocked.unwrap_or(false) {
            return Err(LOCKED.into());
        }
        self.run_any(f).await
    }

    /// Vault, PDF and Typst work is blocking.
    async fn run_any(&self, f: impl FnOnce(&Arc<Blankey>) -> ToolResult + Send + 'static) -> ToolResult {
        let app = self.app.clone();
        tokio::task::spawn_blocking(move || f(&app)).await.unwrap_or_else(|e| Err(format!("Tool failed: {e}")))
    }
}

/// How long a tool call waits for the user to unlock the vault.
const UNLOCK_WAIT: Duration = Duration::from_secs(50);
/// Shown on the unlock screen, which translates it.
const UNLOCK_REASON: &str = crate::i18n::mark("Your AI assistant needs the vault. Unlock Blankey to continue.");
const LOCKED: &str =
    "The vault is locked and the user has not unlocked Blankey yet. Ask them to unlock it, then call again.";

#[tool_router]
impl Tools {
    #[tool(
        description = "Find profiles (people, companies, deals...) by name, kind or the field keys they hold. keys shows where\ndata already is: keys: [\"offered_price\"] returns the profiles with that key filled, with its value length.\nEach result has field counts; with_fields lists the fields (no values). Paged: total, next_offset."
    )]
    async fn find_profiles(&self, Parameters(a): Parameters<FindProfilesArgs>) -> ToolResult {
        self.run(move |app| {
            app.audit("mcp", "find_profiles", a.query.as_deref().unwrap_or(""));
            let (profiles, index) = {
                let vault = app.vault();
                (vault.list_profiles().map_err(string)?, vault.field_index().map_err(string)?)
            };
            let words = find::words(a.query.as_deref());
            let kind = a.kind.as_deref().map(find::fold);
            let filled = a.filled.unwrap_or(true);
            let none = Vec::new();
            let mut matched: Vec<(Profile, &Vec<FieldInfo>)> = profiles
                .into_iter()
                .map(|p| {
                    let fields = index.get(&p.id).unwrap_or(&none);
                    (p, fields)
                })
                .filter(|(p, fields)| {
                    let mut attrs = vec![p.name.as_str(), p.kind.as_str()];
                    attrs.extend(fields.iter().flat_map(|i| [i.key.as_str(), i.label.as_str()]));
                    (a.ids.is_empty() || a.ids.contains(&p.id))
                        && kind.as_ref().is_none_or(|k| find::fold(&p.kind) == *k)
                        && a.keys.iter().all(|key| {
                            let held = key_fields(fields, key);
                            !held.is_empty() && (!filled || held.iter().any(|i| i.length > 0))
                        })
                        && find::hit(&words, &attrs)
                })
                .collect();
            find::rank_by(&mut matched, a.query.as_deref(), |(p, _)| (p.id.to_string(), p.name.clone()));
            raw(find::found(&matched, a.limit, a.offset, |(p, fields)| {
                let mut item = json!({"id": p.id, "name": p.name, "kind": p.kind, "fields": fields.len(),
                                      "filled": fields.iter().filter(|i| i.length > 0).count()});
                if !a.keys.is_empty() {
                    let keys: Map<String, Value> = a
                        .keys
                        .iter()
                        .flat_map(|key| key_fields(fields, key))
                        .map(|i| (i.key.clone(), json!(i.length)))
                        .collect();
                    item["keys"] = Value::Object(keys);
                }
                if a.with_fields {
                    item["fields"] = json!(fields);
                }
                item
            }))
        })
        .await
    }

    #[tool(
        description = "Find saved templates by words in their id, name, description, roles, vars and labels, or by kind, role\nor var. Look for an existing template before making a new one. Show the user each template's id next to its\nname. get_template gives one template's fields. The user's own folders come after the templates: status candidate,\nsource lib:<id>, with a category from the file name and readiness (ready: has fields or tags, blanks: dots or\nunderscores where values go, needs_ai, scan). Paged: total, next_offset."
    )]
    async fn find_templates(&self, Parameters(a): Parameters<FindTemplatesArgs>) -> ToolResult {
        self.run(move |app| {
            app.sync_library(LIBRARY_SYNC_AGE);
            let words = find::words(a.query.as_deref());
            let wanted = |status: &str| a.status.as_deref().is_none_or(|s| s == status);
            let all = app.templates.all();
            let templatized: HashSet<&str> =
                all.iter().filter_map(|t| t.origin.as_ref()).map(|o| o.file.as_str()).collect();
            let mut templates: Vec<&Template> = all
                .iter()
                .filter(|t| {
                    let mut attrs = vec![t.id.as_str(), t.name.as_str(), t.description.as_str()];
                    attrs.extend(t.roles.iter().flat_map(|(role, about)| [role.as_str(), about.as_str()]));
                    attrs.extend(t.fields.keys().map(String::as_str));
                    attrs.extend(t.labels.values().map(String::as_str));
                    a.kind.as_deref().is_none_or(|k| t.kind.as_str() == k)
                        && a.role.as_ref().is_none_or(|r| t.roles.contains_key(r))
                        && a.var.as_ref().is_none_or(|v| t.fields.contains_key(v))
                        && wanted(template_status(t))
                        && find::hit(&words, &attrs)
                })
                .collect();
            templates.sort_by_cached_key(|t| find::fold(&t.name));
            let file_kind = match a.kind.as_deref() {
                None => None,
                Some("docx") => Some("docx"),
                Some("pdf_form") => Some("pdf"),
                Some(_) => Some(""),
            };
            let files: Vec<LibraryFile> = if a.role.is_none() && a.var.is_none() && wanted("candidate") {
                app.library
                    .files()?
                    .into_iter()
                    .filter(|f| {
                        let mut attrs = vec![f.rel_path.as_str(), f.title.as_str(), f.kind.as_str()];
                        attrs.extend(f.category.as_deref());
                        attrs.extend(f.tags.iter().map(String::as_str));
                        !templatized.contains(f.source().as_str())
                            && file_kind.is_none_or(|k| f.kind == k)
                            && find::hit(&words, &attrs)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let mut matched: Vec<Found> =
                templates.into_iter().map(Found::Template).chain(files.iter().map(Found::File)).collect();
            find::rank_by(&mut matched, a.query.as_deref(), |item| match item {
                Found::Template(t) => (t.id.clone(), t.name.clone()),
                Found::File(f) => (f.source(), f.name().to_string()),
            });
            raw(find::found(&matched, a.limit, a.offset, |item| match item {
                Found::Template(t) => json!({"id": t.id, "name": t.name, "kind": t.kind.as_str(),
                       "status": template_status(t), "roles": t.roles.keys().collect::<Vec<_>>(),
                       "vars": t.fields.len(), "description": find::clip(&t.description, 120)}),
                Found::File(f) => json!({"source": f.source(), "status": "candidate", "name": f.name(),
                       "kind": f.kind, "path": f.rel_path, "category": f.category, "tags": f.tags,
                       "readiness": f.readiness, "fields": f.fields, "blanks": f.blanks, "pages": f.pages}),
            }))
        })
        .await
    }

    #[tool(description = "Full template manifest (roles, field bindings) and its example names.")]
    async fn get_template(&self, Parameters(a): Parameters<TemplateArgs>) -> ToolResult {
        self.run(move |app| {
            let template = app.templates.get(&a.template_id)?;
            let mut out = engine::describe_template(&template);
            out["examples"] = json!(app.templates.examples(&a.template_id)?);
            text(&out)
        })
        .await
    }

    #[tool(
        description = "Read a PDF to make a template from it. A form (has AcroForm fields): its fields (name, type, page,\nrect in PDF points, comb length, button states), plus the given pages with every field outlined; next\nsave_template. A flat or scanned PDF: per page its size and printed text lines with [x0, y0, x1, y1] boxes\n(PDF points, origin bottom-left), and each page as an image with a coordinate grid (lines every 50 pt,\nlabels every 100 pt; default all pages); next save_pdf_overlay_template."
    )]
    async fn inspect_pdf(&self, Parameters(a): Parameters<InspectPdfArgs>, ext: Extensions) -> ToolResult {
        let remote = remote(&ext);
        self.run(move |app| {
            let pdf = read_file(app, remote.as_ref(), &a.path)?;
            let fields = pdf_form::inspect_form(&pdf)?;
            let sizes = preview::page_sizes(&pdf)?;
            let mut selected = a.pages.clone().unwrap_or_default();
            if let Some(bad) = selected.iter().find(|p| **p == 0 || **p > sizes.len()) {
                return Err(format!("page must be 1..{} (got {bad})", sizes.len()));
            }
            if !fields.is_empty() {
                let mut out = vec![ContentBlock::text(dump(&json!({"kind": "form", "form_fields": fields})))];
                for page in selected {
                    out.push(png(preview::annotate_fields(
                        &pdf,
                        &fields,
                        page,
                        a.dpi.map_or(110.0, |d| d.clamp(30, 300) as f32),
                    )?));
                }
                return ok(out);
            }
            if selected.is_empty() {
                selected = (1..=sizes.len()).collect();
            }
            let round = |v: f32| ((v as f64) * 10.0).round() / 10.0;
            let pages: Vec<Value> = selected
                .iter()
                .map(|&page| {
                    let (width, height) = sizes[page - 1];
                    Ok(json!({
                        "page": page,
                        "width": round(width),
                        "height": round(height),
                        "lines": preview::layout_lines(&pdf, page, 300)?,
                    }))
                })
                .collect::<Result<_, String>>()?;
            let mut out = vec![ContentBlock::text(dump(&json!({"kind": "flat", "pages": pages})))];
            for page in selected {
                out.push(png(preview::grid_page(&pdf, page, dpi(a.dpi))?));
            }
            ok(out)
        })
        .await
    }

    #[tool(
        description = "Make a PDF form template from a flat PDF: each box {name, page, rect: [x0, y0, x1, y1], type: text|check,\nmax_len?, comb?} (PDF points, origin bottom-left) becomes a form field named `name`. Place boxes where values go,\nright of or below their printed labels; a check box gets \"X\" when its expression is truthy. fields maps the\nnames to Jinja expressions over roles; labels maps them to labels for the fill form. Omit source_path to rebuild\nfrom the flat PDF saved earlier (replaces all boxes). Returns the pages with the boxes outlined: check them and\ncall again to correct."
    )]
    async fn save_pdf_overlay_template(
        &self,
        Parameters(a): Parameters<SavePdfOverlayArgs>,
        ext: Extensions,
    ) -> ToolResult {
        let remote = remote(&ext);
        self.run(move |app| {
            check_id(&a.template_id)?;
            let flat = match &a.source_path {
                Some(path) => read_file(app, remote.as_ref(), path)?,
                None => {
                    let saved = app.templates.root().join(&a.template_id).join("flat.pdf");
                    std::fs::read(&saved)
                        .map_err(|_| "Pass source_path: no flat PDF saved for this template yet".to_string())?
                }
            };
            let boxes: Vec<Value> = a.boxes.into_iter().map(Value::Object).collect();
            let form = pdf_form::add_fields(&flat, &boxes)?;
            let existing = app.templates.get(&a.template_id).ok();
            let origin = origin(app, a.source_path.as_deref(), Vec::new(), existing.as_ref())?;
            let template = app.templates.save(SaveTemplate {
                id: &a.template_id,
                name: &a.name,
                kind: TemplateKind::PdfForm,
                roles: a.roles,
                fields: a.fields.clone(),
                description: &a.description.unwrap_or_default(),
                source_file: None,
                source_bytes: Some(&form),
                source_text: None,
                labels: a.labels,
                boxes: boxes.clone(),
                origin,
            })?;
            std::fs::write(template.directory.join("flat.pdf"), &flat).map_err(string)?;
            app.audit("mcp", "save_template", &a.template_id);
            let names: Vec<String> =
                boxes.iter().filter_map(|b| b.get("name").and_then(Value::as_str)).map(str::to_string).collect();
            let mut warnings: Vec<String> = names
                .iter()
                .filter(|n| !a.fields.contains_key(*n))
                .map(|n| format!("Box without binding: {n}"))
                .collect();
            let mut known: Vec<String> = names.clone();
            known.extend(pdf_form::inspect_form(&flat)?.into_iter().map(|f| f.name));
            warnings
                .extend(a.fields.keys().filter(|f| !known.contains(f)).map(|f| format!("Binding without field: {f}")));
            let mut summary = engine::describe_template(&template);
            summary["warnings"] = json!(warnings);
            let all_fields = pdf_form::inspect_form(&form)?;
            let mut pages: Vec<usize> =
                boxes.iter().filter_map(|b| b.get("page").and_then(Value::as_u64)).map(|p| p as usize).collect();
            pages.sort_unstable();
            pages.dedup();
            let mut out = vec![ContentBlock::text(dump(&summary))];
            for page in pages {
                out.push(png(preview::annotate_fields(&form, &all_fields, page, 110.0)?));
            }
            ok(out)
        })
        .await
    }

    #[tool(
        description = "Create or replace a template. kind: pdf_form (source_path = blank PDF form), typst (source_text,\nreads `json(bytes(sys.inputs.data))` for the field values) or docx (source_path, docxtpl Jinja tags).\nfields maps PDF field names / template variables to Jinja expressions over the roles.\nroles maps role names (e.g. applicant, loan) to a short description. Name the template, its roles\nand its variables for the kind of document and party (applicant, counterparty, amount, start_date),\nnot for the particular case in front of you, so the template and its fill sets work for other\ncases too. Checkboxes check on any truthy value; radio groups take the state name (\"/Choice2\").\nOmit the source to keep the existing one."
    )]
    async fn save_template(&self, Parameters(a): Parameters<SaveTemplateArgs>, ext: Extensions) -> ToolResult {
        let remote = remote(&ext);
        self.run(move |app| {
            let source = a.source_path.as_deref().map(|path| source_path(app, remote.as_ref(), path)).transpose()?;
            let existing = app.templates.get(&a.template_id).ok();
            let origin = origin(app, a.source_path.as_deref(), Vec::new(), existing.as_ref())?;
            let template = app.templates.save(SaveTemplate {
                id: &a.template_id,
                name: &a.name,
                kind: TemplateKind::parse(&a.kind)?,
                roles: a.roles,
                fields: a.fields,
                description: &a.description.unwrap_or_default(),
                source_file: source.as_deref(),
                source_bytes: None,
                source_text: a.source_text.as_deref(),
                labels: existing.as_ref().map(|t| t.labels.clone()).unwrap_or_default(),
                boxes: existing.map(|t| t.boxes).unwrap_or_default(),
                origin,
            })?;
            app.audit("mcp", "save_template", &a.template_id);
            text(&engine::describe_template(&template))
        })
        .await
    }

    #[tool(
        description = "Paragraphs of a local .docx (body and table cells) with their run texts and existing {{ }} tags.\nUse the run texts to pick spans for save_docx_template: a span cannot cross a run boundary."
    )]
    async fn inspect_docx(&self, Parameters(a): Parameters<PathArgs>, ext: Extensions) -> ToolResult {
        let remote = remote(&ext);
        self.run(move |app| text(&docx::inspect(&source_path(app, remote.as_ref(), &a.path)?)?)).await
    }

    #[tool(
        description = "Create a docx template from a filled .docx: each replacement {find, var, occurrence?} replaces the\ntext with {{ var }} (occurrence is 1-based across the document; omit it to replace all). Replace the value, not\nits label (\"ЕГН ......\": find the dots, with occurrence); a span must sit inside one run. Replacements apply in\norder, so put longer spans first (\".......2026\" before \"......\"). fields maps var -> default Jinja expression\nover roles; labels maps var -> label shown in the fill form. Use generic var names (counterparty_name, amount,\nsigning_date), the same across related documents so one fill covers them all."
    )]
    async fn save_docx_template(&self, Parameters(a): Parameters<SaveDocxArgs>, ext: Extensions) -> ToolResult {
        let remote = remote(&ext);
        self.run(move |app| {
            check_id(&a.template_id)?;
            let source = source_path(app, remote.as_ref(), &a.source_path)?;
            let scratch = tempfile_path("docx");
            let replacements: Vec<Value> = a.replacements.into_iter().map(Value::Object).collect();
            let origin = origin(app, Some(&a.source_path), replacements.clone(), None)?;
            let saved = docx::tokenize(&source, &scratch, &replacements).and_then(|report| {
                let template = app.templates.save(SaveTemplate {
                    id: &a.template_id,
                    name: &a.name,
                    kind: TemplateKind::Docx,
                    roles: a.roles,
                    fields: a.fields,
                    description: &a.description.unwrap_or_default(),
                    source_file: Some(&scratch),
                    source_bytes: None,
                    source_text: None,
                    labels: a.labels,
                    boxes: Vec::new(),
                    origin,
                })?;
                Ok((template, report))
            });
            let _ = std::fs::remove_file(&scratch);
            let (template, report) = saved?;
            app.audit("mcp", "save_template", &a.template_id);
            let mut out = engine::describe_template(&template);
            out["replaced"] = json!(report);
            text(&out)
        })
        .await
    }

    #[tool(
        description = "Render a template with example data (a saved example name or an inline dict keyed by role).\nReturns warnings, the preview PDF path and PNGs of the selected pages (default: all)."
    )]
    async fn render_example(&self, Parameters(a): Parameters<RenderExampleArgs>, ext: Extensions) -> ToolResult {
        let remote = remote(&ext).is_some();
        self.run(move |app| {
            let template = app.templates.get(&a.template_id)?;
            let data = match a.example {
                Value::String(name) => app.templates.get_example(&a.template_id, &name)?,
                data @ Value::Object(_) => data,
                _ => return Err("example must be a saved example name or a dict keyed by role".into()),
            };
            if let Some(name) = a.save_as.as_deref().filter(|n| !n.is_empty()) {
                app.templates.save_example(&a.template_id, name, &data)?;
            }
            let rendered = engine::render_example(&template, &example_context(&data))?;
            let out_path = app.config.previews_dir().join(format!("{}-example.{}", a.template_id, rendered.extension));
            std::fs::write(&out_path, &rendered.content).map_err(string)?;
            // a remote agent cannot open the file, and the absolute path reveals the user's home folder
            let path = if remote { json!(null) } else { json!(out_path) };
            let mut out = vec![ContentBlock::text(dump(&json!({"path": path, "warnings": rendered.warnings})))];
            if rendered.is_pdf() {
                for image in preview::render_pages(&rendered.content, &a.pages.unwrap_or_default(), dpi(a.dpi))? {
                    out.push(png(image));
                }
            }
            ok(out)
        })
        .await
    }

    #[tool(
        description = "Check a template against real profiles ({role: profile_id}) without revealing values:\nmissing/empty keys and plain-text bindings (status literal) always; fit_ratio (> 1 = text too wide) and\nmissing glyphs when the vault is unlocked."
    )]
    async fn check_bindings(&self, Parameters(a): Parameters<CheckBindingsArgs>) -> ToolResult {
        self.run(move |app| {
            app.audit(
                "mcp",
                "check_bindings",
                &format!(
                    "template={} profiles={}",
                    a.template_id,
                    serde_json::to_string(&a.profiles).unwrap_or_default()
                ),
            );
            let reports = app.check_bindings(&a.template_id, &a.profiles)?;
            let problems: Vec<_> = reports.iter().filter(|r| r.status != "ok").collect();
            text(&json!({
                "vault_unlocked": app.vault().unlocked(),
                "checked": reports.len(),
                "ok": reports.len() - problems.len(),
                "problems": problems,
            }))
        })
        .await
    }

    #[tool(
        description = "Ask the user to type data into the app. fields: [{key, label, type, required, hint, suggested}] with type one of\ntext, number, date, bool, egn, iban, email, phone. suggested: a value you already know (e.g. public company data);\nthe form starts with it and the user confirms or changes it. hint is only a placeholder. Give profile_id to extend a\nprofile, or new_profile_name to create one. Returns a request id; call wait_request for the outcome."
    )]
    async fn request_profile_input(
        &self,
        Parameters(a): Parameters<RequestProfileInputArgs>,
        ext: Extensions,
    ) -> ToolResult {
        let grant = remote(&ext).map(|r| r.grant);
        self.run_any(move |app| {
            let new_name = a.new_profile_name.filter(|n| !n.trim().is_empty());
            if a.profile_id.is_none() && new_name.is_none() {
                return Err("Pass profile_id or new_profile_name".into());
            }
            if let Some(id) = a.profile_id {
                app.vault().get_profile(id).map_err(string)?;
            }
            for item in &a.fields {
                if item.get("key").and_then(Value::as_str).is_none_or(str::is_empty) {
                    return Err("Every field needs a key".into());
                }
                let field_type = item.get("type").and_then(Value::as_str).unwrap_or("text");
                if !fieldtypes::is_type(field_type) {
                    return Err(format!("Unknown field type {field_type:?}; use {}", fieldtypes::TYPES.join(", ")));
                }
                if item.get("suggested").is_some_and(|v| v.is_array() || v.is_object()) {
                    return Err("suggested must be a string, number or bool".into());
                }
            }
            let new_profile = a
                .profile_id
                .is_none()
                .then(|| json!({"name": new_name, "kind": a.new_profile_kind.unwrap_or_default()}));
            let payload = json!({
                "profile_id": a.profile_id,
                "new_profile": new_profile,
                "fields": a.fields,
                "reason": a.reason.unwrap_or_default(),
            });
            let id = app.create_request(PROFILE_INPUT, payload, grant.as_deref())?;
            text(&json!({"request_id": id, "status": "pending"}))
        })
        .await
    }

    #[tool(
        description = "Open the fill form; the user checks it and generates the documents. profiles maps each role the bindings\nread to a profile id: pick them yourself instead of asking. Give every var a source:\n- values: non-personal data you know (amounts, dates, listing details).\n- paths: data already in a profile, \"role.key\", or \"profile:ID.key\" for any profile. Not needed when the\n  template default is exactly \"{{ role.key }}\".\n- ask: {var: hint} for the rest; the user types it in the form and it stays in this fill set only. Never put\n  personal data in values.\nSpeed beats a perfect guess: the user can change any source. A var left on a default that gives no vault value is\nrefused unless allow_defaults. fill_set_id reopens a fill set and saves over it; copy_fill_set_id starts from one\nand saves a new fill set under name. Returns a request id; call wait_request."
    )]
    async fn request_fill(&self, Parameters(a): Parameters<RequestFillArgs>, ext: Extensions) -> ToolResult {
        let grant = remote(&ext).map(|r| r.grant);
        self.run_any(move |app| {
            let name = a.name.trim().to_string();
            if a.fill_set_id.is_some() && a.copy_fill_set_id.is_some() {
                return Err("Pass fill_set_id or copy_fill_set_id, not both".into());
            }
            let base = a.fill_set_id.or(a.copy_fill_set_id);
            let mut profiles = Profiles::new();
            let mut bindings = Map::new();
            let mut templates = a.templates.clone();
            if let Some(id) = base {
                let sets = app.vault().list_fill_sets().map_err(string)?;
                let info = sets.into_iter().find(|f| f.id == id).ok_or_else(|| format!("Unknown fill set {id}"))?;
                if templates.is_empty() {
                    templates = info.templates;
                }
                profiles = info.profiles;
                let vault = app.vault();
                if vault.unlocked() {
                    bindings = vault.get_fill_set(id).map_err(string)?.1;
                }
            }
            profiles.extend(a.profiles.clone());
            let loaded = app.fill_templates(&templates)?;
            if loaded.is_empty() || name.is_empty() {
                return Err("Pass at least one template and a name".into());
            }
            let known = fill::variables(&loaded)?;
            for var in a.values.keys().chain(a.paths.keys()).chain(a.ask.keys()) {
                if !known.contains_key(var) {
                    let names: Vec<&str> = known.keys().map(String::as_str).collect();
                    return Err(format!("Unknown variable {var:?}; known: {}", names.join(", ")));
                }
            }
            for var in a.ask.keys() {
                if a.values.contains_key(var) || a.paths.contains_key(var) {
                    return Err(format!("{var}: pass it in one of values, paths or ask"));
                }
            }
            let role_names: HashSet<String> = fill::roles(&loaded).into_keys().collect();
            let fields = app.profile_fields(&profiles)?;
            for (var, path) in &a.paths {
                let binding = fill::path_binding(path).map_err(|e| format!("{var}: {e}"))?;
                let key = binding.get(fill::PATH).and_then(Value::as_str).unwrap_or_default();
                let found = match binding.get(fill::PROFILE).and_then(Value::as_i64) {
                    Some(id) => {
                        let infos = app.vault().describe(id).map_err(|e| format!("{var}: profile {id}: {e}"))?;
                        fill::has_field(&infos, key)
                    }
                    None => {
                        let (role, key) = key.split_once('.').unwrap_or((key, ""));
                        fields.get(role).is_some_and(|infos| fill::has_field(infos, key))
                    }
                };
                if !found {
                    return Err(format!("{var}: no vault field {path:?}"));
                }
                bindings.insert(var.clone(), binding);
            }
            for var in a.values.keys() {
                bindings.insert(var.clone(), json!({fill::VALUE: ""}));
            }
            for var in a.ask.keys() {
                bindings.entry(var.clone()).or_insert_with(|| json!({fill::VALUE: ""}));
            }
            let mut missing: Vec<String> = fill::used_roles(&known, &bindings, &role_names)
                .into_iter()
                .filter(|r| !profiles.contains_key(r))
                .collect();
            if !missing.is_empty() {
                missing.sort();
                return Err(format!(
                    "Missing profiles for roles: {}. Pass them in profiles, or bind those vars with paths \
                     (profile:ID.key), values or ask",
                    missing.join(", ")
                ));
            }
            let saved = base.is_some() || app.vault().find_fill_set(&name).map_err(string)?.is_some();
            if !a.allow_defaults && !saved {
                let uncovered: Vec<String> = known
                    .values()
                    .filter(|v| !bindings.contains_key(&v.name))
                    .filter(|v| fill::uncovered(v, &role_names, &fields))
                    .map(|v| {
                        format!("{} (default {:?})", v.name, v.defaults.values().next().cloned().unwrap_or_default())
                    })
                    .collect();
                if !uncovered.is_empty() {
                    let keys: Vec<String> = fields
                        .iter()
                        .map(|(role, infos)| {
                            format!("{role}: {}", infos.iter().map(|i| i.key.as_str()).collect::<Vec<_>>().join(", "))
                        })
                        .collect();
                    return Err(format!(
                        "These vars fall back to a template default that gives no vault value: {}. \
                         Map each to a profile key in paths (\"buyer.full_name\" or \"profile:ID.key\" for any \
                         profile), put it in ask for the user to type in the form, or pass allow_defaults: true. \
                         Profile keys: {}",
                        uncovered.join(", "),
                        keys.join("; ")
                    ));
                }
            }
            let reason = a.reason.filter(|r| !r.is_empty()).unwrap_or_else(|| {
                format!("Fill {}", loaded.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "))
            });
            let payload = json!({
                "templates": a.templates,
                "profiles": a.profiles,
                "name": name,
                "values": a.values,
                "paths": a.paths,
                "ask": a.ask,
                "fill_set_id": a.fill_set_id,
                "copy_fill_set_id": a.copy_fill_set_id,
                "reason": reason,
            });
            let id = app.create_request(FILL, payload, grant.as_deref())?;
            text(&json!({"request_id": id, "status": "pending"}))
        })
        .await
    }

    #[tool(
        description = "Ask the user to pick a pdf or docx file in the app (remote sessions cannot read local paths).\nReturns a request id; wait_request then gives {file_id, kind, size}. Pass file_id as path or source_path."
    )]
    async fn request_file(&self, Parameters(a): Parameters<RequestFileArgs>, ext: Extensions) -> ToolResult {
        let grant = remote(&ext).map(|r| r.grant);
        self.run(move |app| {
            let kind = a.kind.trim().trim_start_matches('.').to_lowercase();
            if !FILE_KINDS.contains(&kind.as_str()) {
                return Err(format!("kind must be one of {}", FILE_KINDS.join(", ")));
            }
            if a.reason.trim().is_empty() {
                return Err("Pass a reason: the user sees it when picking the file".into());
            }
            let id = app.create_request(FILE, json!({"kind": kind, "reason": a.reason.trim()}), grant.as_deref())?;
            text(&json!({"request_id": id, "status": "pending"}))
        })
        .await
    }

    #[tool(
        description = "Find saved fill sets by words in their name, templates and profile names, or by template_id or\nprofile_id; newest first. get_fill_set shows where each var comes from. Paged: total, next_offset."
    )]
    async fn find_fill_sets(&self, Parameters(a): Parameters<FindFillSetsArgs>) -> ToolResult {
        self.run(move |app| {
            let (sets, profile_names) = {
                let vault = app.vault();
                let names: std::collections::HashMap<i64, String> =
                    vault.list_profiles().map_err(string)?.into_iter().map(|p| (p.id, p.name)).collect();
                (vault.list_fill_sets().map_err(string)?, names)
            };
            let template_names: std::collections::HashMap<String, String> =
                app.templates.all().into_iter().map(|t| (t.id, t.name)).collect();
            let words = find::words(a.query.as_deref());
            let mut matched: Vec<_> = sets
                .into_iter()
                .filter(|set| {
                    let mut attrs = vec![set.name.as_str()];
                    attrs.extend(set.templates.iter().map(String::as_str));
                    attrs.extend(set.templates.iter().filter_map(|t| template_names.get(t)).map(String::as_str));
                    attrs.extend(set.profiles.values().filter_map(|id| profile_names.get(id)).map(String::as_str));
                    a.template_id.as_ref().is_none_or(|t| set.templates.contains(t))
                        && a.profile_id.is_none_or(|id| set.profiles.values().any(|p| *p == id))
                        && find::hit(&words, &attrs)
                })
                .collect();
            find::rank_by(&mut matched, a.query.as_deref(), |set| (set.id.to_string(), set.name.clone()));
            raw(find::found(&matched, a.limit, a.offset, |set| json!(set)))
        })
        .await
    }

    #[tool(
        description = "One fill set: templates, profiles per role and the source kind per var (path:role.key,\npath:profile:ID.key, value or default). No values."
    )]
    async fn get_fill_set(&self, Parameters(a): Parameters<FillSetArgs>) -> ToolResult {
        self.run(move |app| {
            let (info, bindings) = app.vault().get_fill_set(a.fill_set_id).map_err(string)?;
            let mut out = json!(info);
            let vars: Map<String, Value> =
                bindings.iter().map(|(v, b)| (v.clone(), json!(fill::describe_binding(b)))).collect();
            out["vars"] = Value::Object(vars);
            text(&out)
        })
        .await
    }

    /// Polls without holding the vault between checks.
    #[tool(
        description = "Wait until the user resolves a request, or the timeout passes (timeout_s 0 returns the current status).\nReturns status and metadata only."
    )]
    async fn wait_request(&self, Parameters(a): Parameters<WaitRequestArgs>, ext: Extensions) -> ToolResult {
        let remote = remote(&ext);
        let timeout = wait_timeout(a.timeout_s, remote.is_some());
        let deadline = Instant::now() + Duration::from_secs(timeout);
        loop {
            let request = visible(self.app.vault().get_request(a.request_id).map_err(string)?, remote.as_ref())?;
            if request.status != "pending" || Instant::now() >= deadline {
                return text(&request_view(&request));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    #[tool(
        description = "List requests, newest first, optionally filtered by status (pending, done, cancelled). Paged: total,\nnext_offset."
    )]
    async fn list_requests(&self, Parameters(a): Parameters<ListRequestsArgs>, ext: Extensions) -> ToolResult {
        let remote = remote(&ext);
        self.run_any(move |app| {
            let mut requests: Vec<Request> = app
                .vault()
                .list_requests(a.status.as_deref())
                .map_err(string)?
                .into_iter()
                .filter_map(|r| visible(r, remote.as_ref()).ok())
                .collect();
            requests.sort_by_key(|r| std::cmp::Reverse(r.id));
            raw(find::found(&requests, a.limit, a.offset, request_view))
        })
        .await
    }

    #[tool(
        description = "Ask the user to create a collection link: a short URL to a web form where someone else types the\nfields (and attaches files) for a profile. fields: [{key, label, type, required, hint}] with type one of\ntext, number, date, bool, egn, iban, email, phone. Give profile_id to fill an existing profile or new_profile_name.\nThe user confirms it in the app; wait_request then returns {link_id, url}. The answers never reach you: the user\nsaves them into the profile, and you fill documents from that profile (request_fill). Never ask for the values in chat."
    )]
    async fn create_collection_link(
        &self,
        Parameters(a): Parameters<CreateCollectionLinkArgs>,
        ext: Extensions,
    ) -> ToolResult {
        let grant = remote(&ext).map(|r| r.grant);
        self.run_any(move |app| {
            let new_profile = a.new_profile_name.filter(|n| !n.trim().is_empty());
            if a.profile_id.is_none() && new_profile.is_none() {
                return Err("Pass profile_id or new_profile_name".into());
            }
            if let Some(id) = a.profile_id {
                app.vault().get_profile(id).map_err(string)?;
            }
            let count =
                |value: Option<i64>, default: u32| value.map_or(default, |v| v.clamp(0, u32::MAX as i64) as u32);
            let draft = LinkDraft {
                title: a.title,
                from: a.from.unwrap_or_default(),
                note: a.note.unwrap_or_default(),
                profile_id: a.profile_id,
                new_profile: new_profile.filter(|_| a.profile_id.is_none()),
                fields: a.fields,
                max_files: count(a.max_files, DEFAULT_MAX_FILES),
                kinds: a.kinds,
                expires_days: count(a.expires_days, 7),
                max_submissions: count(a.max_submissions, 1),
            };
            let spec = draft.spec()?;
            let mut payload = serde_json::to_value(&draft).map_err(string)?;
            let reason = a.reason.filter(|r| !r.trim().is_empty()).unwrap_or_else(|| format!("Collect {}", spec.title));
            payload["reason"] = json!(reason);
            let id = app.create_request(COLLECTION_LINK, payload, grant.as_deref())?;
            text(&json!({"request_id": id, "status": "pending"}))
        })
        .await
    }

    #[tool(
        description = "Collection links made on this computer, newest first: url, status, the fields they ask for, how\nmany submissions came in and how many wait for the user. Filter by words in the title or by status (open,\nfull, expired, revoked). No submitted values. Paged: total, next_offset."
    )]
    async fn list_collection_links(&self, Parameters(a): Parameters<ListLinksArgs>) -> ToolResult {
        self.run_any(move |app| {
            let (links, submissions) = {
                let vault = app.vault();
                (vault.list_links().map_err(string)?, vault.list_submissions(None).map_err(string)?)
            };
            let words = find::words(a.query.as_deref());
            let links: Vec<_> = links
                .into_iter()
                .filter(|link| {
                    a.status.as_deref().is_none_or(|s| link_state(link) == s) && find::hit(&words, &[&link.title])
                })
                .collect();
            raw(find::found(&links, a.limit, a.offset, |link| {
                let fields: Vec<Value> = link.spec["fields"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|f| json!({"key": f["key"], "label": f["label"]}))
                    .collect();
                let mine: Vec<_> = submissions.iter().filter(|r| r.link_id == link.link_id).collect();
                let pending = mine.iter().filter(|r| r.status == "pending").count();
                let mut saved_into: Vec<i64> =
                    mine.iter().filter(|r| r.status == "saved").filter_map(|r| r.profile_id).collect();
                saved_into.sort_unstable();
                saved_into.dedup();
                json!({
                    "id": link.link_id,
                    "title": link.title,
                    "url": link.url,
                    "status": link_state(link),
                    "fields": fields,
                    "max_files": link.spec["files"]["max_files"],
                    "received": link.received,
                    "pending": pending,
                    "saved_into": saved_into,
                    "max_submissions": link.max_submissions,
                    "profile_id": link.profile_id,
                    "expires_at": link.expires_at,
                    "next": next_step(link.received, pending, &saved_into),
                })
            }))
        })
        .await
    }

    #[tool(
        description = "Find generated real documents (metadata only; the user exports them from the app), newest first, by\nwords in the title or file name, template_id, profile_id or since (YYYY-MM-DD). Paged: total, next_offset."
    )]
    async fn find_documents(&self, Parameters(a): Parameters<FindDocumentsArgs>) -> ToolResult {
        self.run(move |app| {
            let words = find::words(a.query.as_deref());
            let documents: Vec<_> = app
                .vault()
                .list_documents()
                .map_err(string)?
                .into_iter()
                .filter(|d| {
                    a.template_id.as_ref().is_none_or(|t| d.template_id == *t)
                        && a.profile_id.is_none_or(|id| d.profiles.values().any(|p| p.as_i64() == Some(id)))
                        && a.since.as_deref().is_none_or(|since| d.created_at.as_str() >= since)
                        && find::hit(&words, &[&d.title, &d.filename, &d.template_id])
                })
                .collect();
            raw(find::found(&documents, a.limit, a.offset, |d| json!(d)))
        })
        .await
    }
}

#[tool_handler(router = self.router())]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        let instructions =
            if self.online() { format!("{INSTRUCTIONS}\n{LINK_INSTRUCTIONS}") } else { INSTRUCTIONS.into() };
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("blankey", env!("CARGO_PKG_VERSION")))
            .with_instructions(instructions)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::config::Config;
    use crate::keychain::MemoryStore;
    use crate::vault::FieldInput;
    use serde::de::DeserializeOwned;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub const PASSWORD: &str = "correct horse battery staple";
    const SENTINEL_NAME: &str = "Sentinel Hiddenwell";
    const SENTINEL_EGN: &str = "7501020018";

    pub fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
    }

    pub fn app(dir: &Path) -> Arc<Blankey> {
        let config = Config::load_from(dir.join("data")).unwrap();
        let app = Blankey::new(config, Box::new(MemoryStore::default())).unwrap();
        app.vault().initialize(PASSWORD, false).unwrap();
        Arc::new(app)
    }

    fn setup(app: &Arc<Blankey>, dir: &Path) -> (i64, String) {
        let source = dir.join("form.pdf");
        std::fs::copy(fixture("form.pdf"), &source).unwrap();
        let mut vault = app.vault();
        let pid = vault.create_profile("Applicant", "person").unwrap();
        vault
            .set_values(
                pid,
                &[
                    FieldInput::new("name", SENTINEL_NAME, "Name", "text"),
                    FieldInput::new("egn", SENTINEL_EGN, "EGN", "egn"),
                ],
            )
            .unwrap();
        (pid, source.display().to_string())
    }

    pub fn local() -> Extensions {
        Extensions::new()
    }

    /// What rmcp hands a tool for a request the relay client forwarded.
    pub fn remote() -> Extensions {
        remote_as("g1")
    }

    fn remote_as(grant: &str) -> Extensions {
        let (mut parts, ()) = axum::http::Request::new(()).into_parts();
        parts.extensions.insert(crate::mcp::Remote { grant: grant.into(), client_name: "Claude".into() });
        let mut extensions = Extensions::new();
        extensions.insert(parts);
        extensions
    }

    fn args<T: DeserializeOwned>(value: Value) -> Parameters<T> {
        Parameters(serde_json::from_value(value).unwrap())
    }

    /// Text of a successful result and the number of images.
    fn out(result: ToolResult) -> (String, usize) {
        let result = result.unwrap();
        let texts = result.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.as_str()).collect();
        let images = result.content.iter().filter(|c| c.as_image().is_some()).count();
        (texts, images)
    }

    #[tokio::test]
    async fn locked_vault_allows_status_and_requests_only() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        app.vault().lock();
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = asked.clone();
        app.set_unlock_hook(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let t = Tools { app: app.clone(), unlock_wait: Duration::ZERO };
        for result in [
            t.find_profiles(args(json!({}))).await,
            t.find_templates(args(json!({}))).await,
            t.find_fill_sets(args(json!({}))).await,
        ] {
            let err = result.err().unwrap();
            assert!(err.contains("locked"), "{err}");
        }
        assert_eq!(asked.load(Ordering::SeqCst), 3);
        let request = json!({"new_profile_name": "Someone", "fields": [{"key": "name"}], "reason": "unlock"});
        let (created, _) = out(t.request_profile_input(args(request), local()).await);
        assert!(created.contains("pending"), "{created}");
        let (listed, _) = out(t.list_requests(args(json!({})), local()).await);
        assert!(listed.contains("pending"), "{listed}");
    }

    #[tokio::test]
    async fn no_tool_leaks_vault_values() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let (pid, source) = setup(&app, dir.path());
        let docx = fixture("filled.docx").display().to_string();
        let save = json!({"template_id": "t", "name": "T", "kind": "pdf_form", "source_path": source,
                          "fields": {"name": "{{ applicant.name }}", "egn": "{{ applicant.egn }}"},
                          "roles": {"applicant": "person"}});
        let mut outputs = vec![
            out(t.find_profiles(args(json!({}))).await),
            out(t.find_profiles(args(json!({"with_fields": true, "keys": ["name", "egn"]}))).await),
            out(t.find_profiles(args(json!({"ids": [pid], "with_fields": true}))).await),
            out(t.inspect_pdf(args(json!({"path": source, "pages": [1]})), local()).await),
            out(t.save_template(args(save), local()).await),
            out(t.find_templates(args(json!({"query": "t"}))).await),
            out(t.get_template(args(json!({"template_id": "t"}))).await),
            out(t
                .render_example(
                    args(json!({"template_id": "t", "example": {"applicant": {"name": "Example"}}, "save_as": "ex"})),
                    local(),
                )
                .await),
            out(t.render_example(args(json!({"template_id": "t", "example": "ex"})), local()).await),
            out(t.check_bindings(args(json!({"template_id": "t", "profiles": {"applicant": pid}}))).await),
            out(t
                .request_fill(
                    args(json!({"templates": ["t"], "profiles": {"applicant": pid}, "name": "leak"})),
                    local(),
                )
                .await),
            out(t
                .request_profile_input(
                    args(json!({"fields": [{"key": "email", "type": "email"}], "reason": "r", "profile_id": pid})),
                    local(),
                )
                .await),
            out(t.request_file(args(json!({"kind": "pdf", "reason": "The blank form"})), local()).await),
            out(t.inspect_docx(args(json!({"path": docx})), local()).await),
            out(t.inspect_pdf(args(json!({"path": fixture("flat.pdf").display().to_string()})), local()).await),
            out(t
                .save_docx_template(
                    args(json!({"template_id": "d", "name": "D", "source_path": docx,
                "replacements": [{"find": "John Smith", "var": "who"}, {"find": "34 000", "var": "amount"}],
                "roles": {"applicant": "person"}, "fields": {"who": "{{ applicant.name }}"}})),
                    local(),
                )
                .await),
        ];
        let profiles: Profiles = [("applicant".to_string(), pid)].into_iter().collect();
        let fill_set = app.vault().save_fill_set("leak", &["t".into()], &profiles, &Map::new(), None).unwrap();
        let docs = app.generate_fill(fill_set).unwrap();
        app.vault().resolve_request(1, "done", json!({"fill_set_id": fill_set, "document_ids": docs})).unwrap();
        let bindings = json!({"amount": {"value": SENTINEL_NAME}}).as_object().cloned().unwrap();
        app.vault().save_fill_set("f", &["d".into()], &profiles, &bindings, None).unwrap();

        use crate::links::tests::{plaintext, stored_link, submit};
        let public = stored_link(&app, "Ab3dE9z");
        let sent = plaintext(
            json!({"name": SENTINEL_NAME, "email": "sentinel@example.com"}),
            &[("Hiddenwell.pdf", "application/pdf", SENTINEL_EGN.as_bytes())],
        );
        let saved = submit(&app, "Ab3dE9z", &public, &sent, "sub-saved");
        let values = app.open_submission("sub-saved").unwrap().values;
        let target = crate::core::Target::Existing(pid);
        assert!(app.resolve_submission("sub-saved", target, &values, &[0]).unwrap().is_empty());
        let waiting = submit(&app, "Ab3dE9z", &public, &sent, "sub-waiting");
        for id in [saved, waiting] {
            let hidden = t.wait_request(args(json!({"request_id": id, "timeout_s": 0})), local()).await.unwrap_err();
            assert!(hidden.contains("Unknown request"), "{hidden}");
        }
        let link = json!({"title": "More details", "profile_id": pid, "fields": [{"key": "phone", "type": "phone"}]});
        outputs.extend([
            out(t.list_collection_links(args(json!({}))).await),
            out(t.create_collection_link(args(link), local()).await),
        ]);
        outputs.extend([
            out(t.find_documents(args(json!({}))).await),
            out(t.wait_request(args(json!({"request_id": 1, "timeout_s": 0})), local()).await),
            out(t.list_requests(args(json!({})), local()).await),
            out(t.find_fill_sets(args(json!({}))).await),
            out(t.get_fill_set(args(json!({"fill_set_id": fill_set}))).await),
        ]);

        let preview = std::fs::read(app.config.previews_dir().join("t-example.pdf")).unwrap();
        let audit: Vec<String> = {
            let vault = app.vault();
            let mut stmt = vault.conn().prepare("SELECT action || target FROM audit").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        let all = [outputs.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join("\n"), audit.join("\n")].join("\n")
            + &String::from_utf8_lossy(&preview);
        for secret in [SENTINEL_NAME, SENTINEL_EGN, "Sentinel", "Hiddenwell", "sentinel@"] {
            assert!(!all.contains(secret), "leaked {secret}");
        }
        let tools = outputs.iter().map(|(t, _)| t.as_str()).collect::<String>();
        assert!(!tools.contains("link_submission"), "submissions are not listed");
    }

    #[tokio::test]
    async fn collection_links_are_confirmed_by_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        // agents always get English, whatever the user's language
        app.set_language("bg");
        let t = Tools::new(app.clone());
        let (pid, _) = setup(&app, dir.path());
        let bad = [
            json!({"title": "x", "fields": [{"key": "a"}]}),
            json!({"title": "", "profile_id": pid, "fields": [{"key": "a"}]}),
            json!({"title": "x", "profile_id": pid, "fields": [{"key": "a b"}]}),
            json!({"title": "x", "profile_id": pid, "fields": [{"key": "a", "type": "colour"}]}),
            json!({"title": "x", "profile_id": pid, "max_files": 0}),
            json!({"title": "x", "profile_id": pid, "fields": [{"key": "a"}], "max_files": 11}),
            json!({"title": "x", "profile_id": 999, "fields": [{"key": "a"}]}),
        ];
        for link in bad {
            assert!(t.create_collection_link(args(link.clone()), local()).await.is_err(), "{link}");
        }
        let link = json!({"title": "Tenant details", "new_profile_name": "Tenant", "max_files": "2",
                          "fields": [{"key": "name", "label": "Full name", "required": true}]});
        let (reply, _) = out(t.create_collection_link(args(link), remote()).await);
        let id = serde_json::from_str::<Value>(&reply).unwrap()["request_id"].as_i64().unwrap();
        let request = app.vault().get_request(id).unwrap();
        assert_eq!(request.kind, COLLECTION_LINK);
        let draft: LinkDraft = serde_json::from_value(Value::Object(request.payload.clone())).unwrap();
        assert_eq!(
            (draft.new_profile.as_deref(), draft.max_files, draft.fields[0].field_type.as_str()),
            (Some("Tenant"), 2, "text")
        );

        let url = "https://relay.example.com/s/Ab3dE9z#qqj_9wO1CyKX9Pbh";
        app.vault()
            .resolve_request(id, "done", json!({"outcome": "created", "link_id": "Ab3dE9z", "url": url}))
            .unwrap();
        let (waited, _) = out(t.wait_request(args(json!({"request_id": id, "timeout_s": 0})), remote()).await);
        assert!(waited.contains(url), "{waited}");

        crate::links::tests::stored_link(&app, "Ab3dE9z");
        let (listed, _) = out(t.list_collection_links(args(json!({}))).await);
        let listed: Value = serde_json::from_str(&listed).unwrap();
        let first = &listed["items"][0];
        assert_eq!(first["status"], "open");
        assert_eq!(first["fields"][0], json!({"key": "name", "label": "Full name"}));
        assert!(first["url"].as_str().unwrap().contains("/s/Ab3dE9z#"));
        assert_eq!(first["next"], "Nothing received yet.");

        use crate::links::tests::{plaintext, stored_link, submit};
        let public = stored_link(&app, "Zz9yX8w");
        submit(&app, "Zz9yX8w", &public, &plaintext(json!({"name": "Jane"}), &[]), "sub-1");
        let link = |listed: &Value| {
            listed["items"].as_array().unwrap().iter().find(|l| l["id"] == "Zz9yX8w").cloned().unwrap()
        };
        let listed: Value = serde_json::from_str(&out(t.list_collection_links(args(json!({}))).await).0).unwrap();
        assert_eq!((link(&listed)["pending"].as_u64(), link(&listed)["saved_into"].clone()), (Some(1), json!([])));
        assert!(link(&listed)["next"].as_str().unwrap().contains("ask the user to review it in Blankey"));

        let values = app.open_submission("sub-1").unwrap().values;
        app.resolve_submission("sub-1", crate::core::Target::Existing(pid), &values, &[]).unwrap();
        let listed: Value = serde_json::from_str(&out(t.list_collection_links(args(json!({}))).await).0).unwrap();
        assert_eq!(link(&listed)["saved_into"], json!([pid]));
        let next = link(&listed)["next"].as_str().unwrap().to_string();
        assert!(next.contains(&format!("profile {pid}")) && next.contains("request_fill"), "{next}");
        assert!(!listed.to_string().contains("Jane"));
    }

    fn page(result: ToolResult) -> Value {
        serde_json::from_str(&out(result).0).unwrap()
    }

    fn names(page: &Value) -> Vec<String> {
        page["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn find_tools_filter_and_page() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let (pid, _) = setup(&app, dir.path());
        let (filled, empty) = {
            let mut vault = app.vault();
            let filled = vault.create_profile("Deal Иван", "deal").unwrap();
            vault.set_values(filled, &[FieldInput::new("offered_price", "500000", "Offered price", "number")]).unwrap();
            let empty = vault.create_profile("Deal two", "deal").unwrap();
            vault.set_values(empty, &[FieldInput::new("offered_price", "", "Offered price", "number")]).unwrap();
            (filled, empty)
        };

        let found = page(t.find_profiles(args(json!({"keys": ["offered_price"]}))).await);
        assert_eq!((found["total"].clone(), names(&found)), (json!(1), vec!["Deal Иван".to_string()]));
        assert_eq!(found["items"][0]["keys"], json!({"offered_price": 6}));
        let found = page(t.find_profiles(args(json!({"keys": ["offered_price"], "filled": false}))).await);
        assert_eq!(found["total"], 2);
        assert_eq!(names(&page(t.find_profiles(args(json!({"query": "ИВАН"}))).await)), vec!["Deal Иван"]);
        assert_eq!(page(t.find_profiles(args(json!({"query": "offered price"}))).await)["total"], 2);
        assert_eq!(page(t.find_profiles(args(json!({"kind": "Deal", "ids": [empty, pid]}))).await)["total"], 1);
        let first = page(t.find_profiles(args(json!({"limit": 1}))).await);
        assert_eq!((first["total"].clone(), first["next_offset"].clone()), (json!(3), json!(1)));
        let last = page(t.find_profiles(args(json!({"limit": 1, "offset": 2}))).await);
        assert!(last.get("next_offset").is_none(), "{last}");
        assert_eq!(page(t.find_profiles(args(json!({"limit": 1000}))).await)["items"].as_array().unwrap().len(), 3);

        let template = |name: &str| {
            args(json!({"template_id": "offer", "name": name, "kind": "typst", "source_text": "",
                        "fields": {"price": "{{ buyer.offered_price }}"}, "roles": {"buyer": "person"}}))
        };
        out(t.save_template(template("Purchase offer"), local()).await);
        assert_eq!(names(&page(t.find_templates(args(json!({"query": "price"}))).await)), vec!["Purchase offer"]);
        assert_eq!(page(t.find_templates(args(json!({"role": "seller"}))).await)["total"], 0);
        assert_eq!(page(t.find_templates(args(json!({"var": "price", "kind": "typst"}))).await)["total"], 1);
        out(t.save_template(template("Buyer offer"), local()).await);
        assert_eq!(page(t.find_templates(args(json!({"query": "purchase"}))).await)["total"], 0);
        assert_eq!(names(&page(t.find_templates(args(json!({"query": "buyer offer"}))).await)), vec!["Buyer offer"]);

        let (older, newer) = {
            let mut vault = app.vault();
            let bindings = json!({"price": {"path": "offered_price", "profile": filled}}).as_object().cloned().unwrap();
            let buyer = |id: i64| -> Profiles { [("buyer".to_string(), id)].into_iter().collect() };
            let older = vault.save_fill_set("first", &["offer".into()], &buyer(filled), &bindings, None).unwrap();
            let newer = vault.save_fill_set("second", &["offer".into()], &buyer(pid), &Map::new(), None).unwrap();
            (older, newer)
        };
        assert_eq!(
            names(&page(t.find_fill_sets(args(json!({"template_id": "offer"}))).await)),
            vec!["second", "first"]
        );
        assert_eq!(names(&page(t.find_fill_sets(args(json!({"profile_id": filled}))).await)), vec!["first"]);
        assert_eq!(names(&page(t.find_fill_sets(args(json!({"query": "applicant"}))).await)), vec!["second"]);
        let (set, _) = out(t.get_fill_set(args(json!({"fill_set_id": older}))).await);
        assert!(set.contains(&format!("path:profile:{filled}.offered_price")), "{set}");

        app.generate_fill(newer).unwrap();
        let documents = page(t.find_documents(args(json!({"template_id": "offer", "profile_id": pid}))).await);
        assert_eq!(documents["total"], 1);
        assert_eq!(page(t.find_documents(args(json!({"since": "2999-01-01"}))).await)["total"], 0);
        assert_eq!(page(t.find_documents(args(json!({"query": "nothing like it"}))).await)["total"], 0);
    }

    #[tokio::test]
    async fn library_files_become_templates_on_demand() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let forms = dir.path().join("Forms");
        std::fs::create_dir_all(forms.join("Leases")).unwrap();
        let original = forms.join("Leases/Lease agreement.docx");
        std::fs::copy(fixture("filled.docx"), &original).unwrap();
        std::fs::copy(fixture("form.pdf"), forms.join("Application.pdf")).unwrap();
        let root = app.library.add_root(&forms).unwrap();
        app.library.scan_root(root).unwrap();

        assert_eq!(page(t.find_templates(args(json!({}))).await)["total"], 2);
        let found = page(t.find_templates(args(json!({"query": "lease"}))).await);
        let lease = &found["items"][0];
        assert_eq!((lease["status"].clone(), lease["readiness"].clone()), (json!("candidate"), json!("blanks")));
        assert_eq!((lease["category"].clone(), lease["kind"].clone()), (json!("lease"), json!("docx")));
        let source = lease["source"].as_str().unwrap().to_string();
        assert_eq!(page(t.find_templates(args(json!({"kind": "typst"}))).await)["total"], 0);
        assert_eq!(page(t.find_templates(args(json!({"kind": "pdf_form"}))).await)["total"], 1);
        assert_eq!(page(t.find_templates(args(json!({"status": "template"}))).await)["total"], 0);

        let fill = json!({"templates": [source], "profiles": {}, "name": "n"});
        let error = t.request_fill(args(fill), local()).await.unwrap_err();
        assert!(error.contains("not a template yet"), "{error}");

        let (inspected, _) = out(t.inspect_docx(args(json!({"path": source})), remote()).await);
        assert!(inspected.contains("John Smith"));
        let save = json!({"template_id": "lease", "name": "Lease", "source_path": source,
                          "replacements": [{"find": "John Smith", "var": "manager_name"}],
                          "fields": {"manager_name": ""}, "roles": {}});
        out(t.save_docx_template(args(save), remote()).await);
        let template = app.templates.get("lease").unwrap();
        let origin = template.origin.clone().unwrap();
        assert_eq!((origin.file.as_str(), origin.recipe.len()), (source.as_str(), 1));
        let found = page(t.find_templates(args(json!({"query": "lease"}))).await);
        assert_eq!((found["total"].clone(), found["items"][0]["status"].clone()), (json!(1), json!("template")));

        // the user edits the original elsewhere: the recipe still applies, so the copy follows
        let edited = dir.path().join("edited.docx");
        docx::tokenize(
            &fixture("filled.docx"),
            &edited,
            json!([{"find": "34 000", "var": "amount"}]).as_array().unwrap(),
        )
        .unwrap();
        std::fs::copy(&edited, &original).unwrap();
        app.sync_library(Duration::ZERO);
        let rebuilt = app.templates.get("lease").unwrap();
        assert_eq!(rebuilt.stale, "");
        assert_ne!(rebuilt.origin.as_ref().unwrap().sha256, origin.sha256);
        assert_eq!(docx::variables(&rebuilt.source_path()).unwrap(), ["manager_name", "amount"]);

        // the span the recipe needs is gone: the template keeps its copy and says why
        let broken = dir.path().join("broken.docx");
        docx::tokenize(
            &fixture("filled.docx"),
            &broken,
            json!([{"find": "John Smith", "var": "x"}]).as_array().unwrap(),
        )
        .unwrap();
        std::fs::copy(&broken, &original).unwrap();
        app.sync_library(Duration::ZERO);
        let stale = app.templates.get("lease").unwrap();
        assert!(stale.stale.contains("John Smith"), "{}", stale.stale);
        assert_eq!(docx::variables(&stale.source_path()).unwrap(), ["manager_name", "amount"]);
        assert_eq!(page(t.find_templates(args(json!({"status": "stale"}))).await)["total"], 1);

        std::fs::remove_file(&original).unwrap();
        app.sync_library(Duration::ZERO);
        assert_eq!(app.templates.get("lease").unwrap().stale, "The source file is gone");
        assert!(t.inspect_docx(args(json!({"path": source})), local()).await.is_err());
    }

    #[tokio::test]
    async fn request_profile_input_passes_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let ask = |field: Value| args(json!({"new_profile_name": "Example Ltd", "fields": [field]}));
        let error = t.request_profile_input(ask(json!({"key": "eik", "suggested": {"a": 1}})), local()).await;
        assert!(error.unwrap_err().contains("suggested"));
        out(t.request_profile_input(ask(json!({"key": "eik", "suggested": "123456789"})), local()).await);
        let request = app.vault().get_request(1).unwrap();
        assert_eq!(request.payload["fields"][0]["suggested"], "123456789");
    }

    #[tokio::test]
    async fn request_fill_validates_input() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let (pid, source) = setup(&app, dir.path());
        out(t
            .save_template(
                args(json!({"template_id": "t", "name": "T", "kind": "pdf_form", "source_path": source,
                "fields": {"name": "{{ applicant.name }}"}, "roles": {"applicant": "person"}})),
                local(),
            )
            .await);
        let error =
            t.request_fill(args(json!({"templates": ["t"], "profiles": {}, "name": "n"})), local()).await.unwrap_err();
        assert!(error.contains("Missing profiles"), "{error}");
        // profile ids as strings are accepted
        let fill = json!({"templates": ["t"], "profiles": {"applicant": pid.to_string()}, "name": "n",
                          "paths": {"name": "applicant.nope"}});
        let error = t.request_fill(args(fill), local()).await.unwrap_err();
        assert!(error.contains("no vault field"), "{error}");

        out(t
            .save_template(
                args(json!({"template_id": "u", "name": "U", "kind": "typst", "source_text": "",
                "fields": {"who": "{{ applicant.full_name }}", "egn": "{{ applicant.egn }}"},
                "roles": {"applicant": "person"}})),
                local(),
            )
            .await);
        let fill = |extra: Value| {
            let mut base = json!({"templates": ["u"], "profiles": {"applicant": pid}, "name": "u"});
            base.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            args(base)
        };
        let error = t.request_fill(fill(json!({})), local()).await.unwrap_err();
        assert!(error.contains("who (default") && !error.contains("egn (default"), "{error}");
        assert!(error.contains("applicant: egn, name"), "{error}");
        out(t.request_fill(fill(json!({"paths": {"who": "applicant.name"}})), local()).await);
        out(t.request_fill(fill(json!({"allow_defaults": true})), local()).await);

        // any profile, by id, and vars the user types
        let other = {
            let mut vault = app.vault();
            let id = vault.create_profile("Deal", "").unwrap();
            vault.set_values(id, &[FieldInput::new("full_name", "Example Name", "Name", "text")]).unwrap();
            id
        };
        out(t.request_fill(fill(json!({"paths": {"who": format!("profile:{other}.full_name")}})), local()).await);
        let error =
            t.request_fill(fill(json!({"paths": {"who": format!("profile:{other}.egn")}})), local()).await.unwrap_err();
        assert!(error.contains("no vault field"), "{error}");
        let error =
            t.request_fill(fill(json!({"paths": {"who": "profile:9999.full_name"}})), local()).await.unwrap_err();
        assert!(error.contains("profile 9999"), "{error}");
        out(t.request_fill(fill(json!({"ask": {"who": "full name as in the ID card"}})), local()).await);
        let error =
            t.request_fill(fill(json!({"ask": {"who": "name"}, "values": {"who": "x"}})), local()).await.unwrap_err();
        assert!(error.contains("one of values, paths or ask"), "{error}");

        // a role whose vars all have other sources needs no profile
        let unbound = json!({"templates": ["u"], "profiles": {}, "name": "free",
                             "paths": {"who": format!("profile:{other}.full_name")}, "ask": {"egn": "EGN"}});
        out(t.request_fill(args(unbound), local()).await);
        let error = t
            .request_fill(
                args(json!({"templates": ["u"], "profiles": {}, "name": "x", "ask": {"egn": "EGN"}})),
                local(),
            )
            .await
            .unwrap_err();
        assert!(error.contains("Missing profiles for roles: applicant"), "{error}");
        let error = t.request_fill(fill(json!({"copy_fill_set_id": 9999})), local()).await.unwrap_err();
        assert!(error.contains("Unknown fill set"), "{error}");

        let bindings = json!({"who": {"path": "full_name", "profile": other}, "egn": {"value": "typed"}});
        let bindings = bindings.as_object().cloned().unwrap();
        let context = app.fill_context(&Profiles::new(), &bindings).unwrap();
        let resolved = fill::resolve(&app.fill_templates(&["u".into()]).unwrap(), &bindings, &context).unwrap();
        assert_eq!(resolved["u"].0["who"], json!("Example Name"));
        let set = app.vault().save_fill_set("pinned", &["u".into()], &Profiles::new(), &bindings, None).unwrap();
        assert_eq!(app.generate_fill(set).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn flat_pdf_becomes_a_template() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let source = fixture("flat.pdf").display().to_string();
        let (layout, grids) = out(t.inspect_pdf(args(json!({"path": source})), local()).await);
        assert!(layout.contains("\"kind\": \"flat\"") && layout.contains("\"Name:\"") && grids == 1, "{layout}");
        let save = json!({
            "template_id": "flat", "name": "Flat",
            "boxes": [{"name": "name", "page": 1, "rect": [90, 785, 400, 803]}],
            "roles": {"applicant": "person"},
            "fields": {"name": "{{ applicant.name }}", "ghost": "x"},
            "source_path": source,
        });
        let (summary, images) = out(t.save_pdf_overlay_template(args(save.clone()), local()).await);
        assert!(summary.contains("Binding without field: ghost") && images == 1);

        let mut moved = save;
        moved["boxes"][0]["rect"] = json!([95, 785, 400, 803]);
        moved["source_path"] = Value::Null;
        out(t.save_pdf_overlay_template(args(moved), local()).await);
        let fields = pdf_form::inspect_form(&app.templates.get("flat").unwrap().source().unwrap()).unwrap();
        assert_eq!(fields[0].rects, [[95.0, 785.0, 400.0, 803.0]]);
    }

    #[tokio::test]
    async fn remote_sessions_read_only_requested_files() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let source = fixture("form.pdf").display().to_string();
        let docx = fixture("filled.docx").display().to_string();
        let save = json!({"template_id": "t", "name": "T", "kind": "pdf_form", "source_path": source,
                          "fields": {}, "roles": {}});
        let overlay = json!({"template_id": "o", "name": "O", "boxes": [], "roles": {}, "fields": {},
                             "source_path": source});
        let docx_save = json!({"template_id": "d", "name": "D", "source_path": docx, "replacements": [],
                               "roles": {}, "fields": {}});
        let refused = [
            t.inspect_pdf(args(json!({"path": source})), remote()).await,
            t.save_template(args(save.clone()), remote()).await,
            t.save_pdf_overlay_template(args(overlay), remote()).await,
            t.inspect_docx(args(json!({"path": docx})), remote()).await,
            t.save_docx_template(args(docx_save), remote()).await,
        ];
        for result in refused {
            let error = result.unwrap_err();
            assert!(error.contains("call request_file"), "{error}");
        }
        assert!(app.templates.all().is_empty());
        let (form, images) = out(t.inspect_pdf(args(json!({"path": source})), local()).await);
        assert!(form.contains("\"kind\": \"form\"") && images == 0, "{form}");

        let error = t.request_file(args(json!({"kind": "xlsx", "reason": "r"})), local()).await.unwrap_err();
        assert!(error.contains("pdf, docx"), "{error}");
        let (reply, _) = out(t.request_file(args(json!({"kind": "pdf", "reason": "The blank form"})), remote()).await);
        let id = serde_json::from_str::<Value>(&reply).unwrap()["request_id"].as_i64().unwrap();
        let request = app.vault().get_request(id).unwrap();
        assert_eq!((request.kind.as_str(), request.status.as_str()), ("file", "pending"));
        assert_eq!(request.payload["kind"], "pdf");

        assert!(app.store_upload(id, &fixture("filled.docx")).unwrap_err().contains(".pdf"));
        let picked = app.store_upload(id, &fixture("form.pdf")).unwrap();
        assert_eq!(picked["name"], "form.pdf");
        assert_eq!(picked["size"], std::fs::metadata(fixture("form.pdf")).unwrap().len());
        assert!(app.store_upload(id, &fixture("form.pdf")).is_err());
        let (waited, _) = out(t.wait_request(args(json!({"request_id": id, "timeout_s": 0})), remote()).await);
        let file_id =
            serde_json::from_str::<Value>(&waited).unwrap()["result"]["file_id"].as_str().unwrap().to_string();
        assert!(file_id.starts_with("file:") && !waited.contains(&dir.path().display().to_string()), "{waited}");
        assert!(!waited.contains("form.pdf"), "the file name stays with the user: {waited}");

        let other = || remote_as("g2");
        let wait = json!({"request_id": id, "timeout_s": 0});
        assert!(t.wait_request(args(wait), other()).await.unwrap_err().contains("Unknown request"));
        assert!(out(t.list_requests(args(json!({})), other()).await).0.contains("\"total\":0"));
        assert!(out(t.list_requests(args(json!({})), remote()).await).0.contains(&file_id));
        assert!(out(t.list_requests(args(json!({})), local()).await).0.contains(&file_id));
        let error = t.inspect_pdf(args(json!({"path": file_id})), other()).await.unwrap_err();
        assert!(error.contains("Unknown file id"), "{error}");
        out(t.inspect_pdf(args(json!({"path": file_id})), local()).await);

        let (fields, _) = out(t.inspect_pdf(args(json!({"path": file_id})), remote()).await);
        assert!(fields.contains("name"), "{fields}");
        let mut save = save;
        save["source_path"] = json!(file_id);
        out(t.save_template(args(save), remote()).await);

        let upper = file_id.to_uppercase().replace("FILE:", "file:");
        let simple = file_id.replace('-', "");
        for bad in ["file:../x", "file:abc", "file:../../data/vault.db", &upper, &simple, &format!("{file_id}/..")] {
            let error = t.inspect_pdf(args(json!({"path": bad})), remote()).await.unwrap_err();
            assert!(error.contains("Unknown file id"), "{bad}: {error}");
        }
        let missing = format!("file:{}", uuid::Uuid::new_v4());
        assert!(t.inspect_pdf(args(json!({"path": missing})), local()).await.unwrap_err().contains("Unknown"));
    }

    #[test]
    fn remote_waits_are_capped() {
        assert_eq!(wait_timeout(None, false), 300);
        assert_eq!(wait_timeout(Some(5000), false), TOOL_TIMEOUT_MAX);
        assert_eq!(wait_timeout(None, true), 50);
        assert_eq!(wait_timeout(Some(1800), true), 50);
        assert_eq!(wait_timeout(Some(10), true), 10);
        assert_eq!(wait_timeout(Some(-3), true), 0);
    }
}
