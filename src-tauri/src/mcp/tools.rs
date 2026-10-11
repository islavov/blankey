//! MCP tools. Nothing here may return a vault value or real-data document bytes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::core::{Blankey, FILL, PROFILE_INPUT};
use crate::fieldtypes;
use crate::render::{pdf_form, preview};
use crate::templates::bindings::example_context;
use crate::templates::engine::{self, dump};
use crate::templates::{docx, fill, SaveTemplate, StrMap, TemplateKind, ID_RE};
use crate::vault::{Profiles, Request};

pub const INSTRUCTIONS: &str = r#"Blankey keeps personal data (PII) encrypted in a local vault and renders document templates.
You never see real values. You see profiles, their field keys, labels, types and value lengths.

Workflow:
1. inspect_pdf_form on a blank PDF form (or write a Typst template) to learn its fields.
2. list_profiles / describe_profile to see which data keys exist.
3. save_template with Jinja bindings per field, e.g. "{{ applicant.address.city }}",
   "{{ 'X' if applicant.citizenship == 'българско' }}", "{{ loan.amount | money }}".
   Checkboxes: any truthy value checks them. Radio groups: return the state name, e.g. "/Choice2".
4. save_example + render_example to fill the template with invented example data and look at the result.
5. check_bindings against real profiles: reports missing/empty keys, overflow (fit_ratio > 1) and
   missing glyphs, without revealing values.
6. request_profile_input to ask the user to type missing data into the app, then wait_request.
7. request_fill to have the user fill the template into a named fill set and generate the real document.
   Documents are always generated from a fill set.
The user exports real documents from the app; you only get their metadata.

Filled .docx documents (contracts, decisions...):
1. inspect_docx to read the paragraphs and their runs.
2. save_docx_template with replacements [{find, var, occurrence?}] that turn the variable spans
   (names, ids, addresses, amounts, dates, dot blanks like "......") into {{ var }} tags. Replace only the
   value, not its label ("ЕГН ......" -> find the dots with occurrence). A span must sit inside one run.
   Use the same var names across related documents so one fill covers them all.
   fields gives a default per var: "{{ manager.egn }}", "{{ manager.name | upper }}", "{{ today() }}".
3. request_fill for one or more templates: the user sees every blank with its context and picks the source
   (vault path, your literal, or the template default). Pass non-personal data you know (amounts, rates,
   terms, dates, city) in values, and vault paths in paths. Never put personal data in values: ask for it
   with request_profile_input instead. wait_request returns the fill set id and document ids.
4. list_fill_sets shows saved fill sets (sources only); request_fill with fill_set_id reopens one.

Flat PDFs (no form fields, e.g. exports or scans):
1. inspect_pdf_layout: pages with a coordinate grid (PDF points, origin bottom-left) and the printed text lines
   with their boxes.
2. save_pdf_overlay_template with boxes [{name, page, rect: [x0, y0, x1, y1], type: text|check, max_len?, comb?}]
   placed where values go (right of / below their printed labels), plus fields (name -> Jinja expression) and labels.
   It returns the pages with the boxes outlined: check them and call again with corrected boxes (source_path can
   be omitted then; the original flat PDF is kept). A check box gets "X" when its expression is truthy text.
3. save_example + render_example, then request_fill as for any PDF form.
"#;

pub const TOOL_TIMEOUT_MAX: u64 = 1800;

type ToolResult = Result<CallToolResult, String>;

fn ok(content: Vec<ContentBlock>) -> ToolResult {
    Ok(CallToolResult::success(content))
}

fn text(value: &impl serde::Serialize) -> ToolResult {
    ok(vec![ContentBlock::text(dump(value))])
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

fn read_file(path: &str) -> Result<Vec<u8>, String> {
    let path = expand(path);
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
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
pub struct ProfileArgs {
    #[serde(deserialize_with = "lenient::int")]
    profile_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct TemplateArgs {
    template_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct PathArgs {
    path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct InspectPdfFormArgs {
    path: String,
    /// 1-based pages to render with every field outlined and named.
    annotate_pages: Option<Vec<usize>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct InspectPdfLayoutArgs {
    path: String,
    /// 1-based pages; default all.
    pages: Option<Vec<usize>>,
    /// Image resolution, default 80.
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
    source_path: Option<String>,
    source_text: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SaveDocxArgs {
    template_id: String,
    name: String,
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
pub struct SaveExampleArgs {
    template_id: String,
    name: String,
    data: Option<Map<String, Value>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RenderExampleArgs {
    template_id: String,
    /// A saved example name or an inline dict keyed by role.
    example: Value,
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
    /// Vault fields per var, as role.key.
    #[serde(default, deserialize_with = "lenient::str_map")]
    paths: StrMap,
    #[serde(default, deserialize_with = "lenient::opt_int")]
    fill_set_id: Option<i64>,
    reason: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RequestArgs {
    #[serde(deserialize_with = "lenient::int")]
    request_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitRequestArgs {
    #[serde(deserialize_with = "lenient::int")]
    request_id: i64,
    /// Seconds, default 300.
    #[serde(default, deserialize_with = "lenient::opt_int")]
    timeout_s: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListRequestsArgs {
    /// pending, done or cancelled
    status: Option<String>,
}

// -- tools -------------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Tools {
    app: Arc<Blankey>,
}

impl Tools {
    pub fn new(app: Arc<Blankey>) -> Self {
        Self { app }
    }

    /// Vault, PDF and Typst work is blocking.
    async fn run(&self, f: impl FnOnce(&Arc<Blankey>) -> ToolResult + Send + 'static) -> ToolResult {
        let app = self.app.clone();
        tokio::task::spawn_blocking(move || f(&app)).await.unwrap_or_else(|e| Err(format!("Tool failed: {e}")))
    }
}

#[tool_router]
impl Tools {
    #[tool(description = "Whether the vault exists and is unlocked. Real-value checks need it unlocked.")]
    async fn vault_status(&self) -> ToolResult {
        self.run(|app| {
            let vault = app.vault();
            text(&json!({"initialized": vault.initialized(), "unlocked": vault.unlocked()}))
        })
        .await
    }

    #[tool(description = "List profiles (people, loans, companies...) stored in the vault.")]
    async fn list_profiles(&self) -> ToolResult {
        self.run(|app| {
            app.audit("mcp", "list_profiles", "");
            text(&app.vault().list_profiles().map_err(string)?)
        })
        .await
    }

    #[tool(
        description = "Field keys of a profile with label, type and value length in characters (0 = empty). No values."
    )]
    async fn describe_profile(&self, Parameters(a): Parameters<ProfileArgs>) -> ToolResult {
        self.run(move |app| {
            app.audit("mcp", "describe_profile", &a.profile_id.to_string());
            let vault = app.vault();
            let profile = vault.get_profile(a.profile_id).map_err(string)?;
            let fields = vault.describe(a.profile_id).map_err(string)?;
            text(&json!({"id": profile.id, "name": profile.name, "kind": profile.kind, "fields": fields}))
        })
        .await
    }

    #[tool(description = "List saved templates.")]
    async fn list_templates(&self) -> ToolResult {
        self.run(|app| {
            let list: Vec<Value> = app
                .templates
                .all()
                .iter()
                .map(|t| json!({"id": t.id, "name": t.name, "kind": t.kind.as_str(), "roles": t.roles, "fields": t.fields.len()}))
                .collect();
            text(&list)
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
        description = "List the AcroForm fields of a local PDF (name, type, page, rects in PDF points, comb length,\nbutton states). annotate_pages renders those 1-based pages with every field outlined and named."
    )]
    async fn inspect_pdf_form(&self, Parameters(a): Parameters<InspectPdfFormArgs>) -> ToolResult {
        self.run(move |_| {
            let pdf = read_file(&a.path)?;
            let fields = pdf_form::inspect_form(&pdf)?;
            let mut out = vec![ContentBlock::text(dump(&fields))];
            for page in a.annotate_pages.unwrap_or_default() {
                out.push(png(preview::annotate_fields(&pdf, &fields, page, 110.0)?));
            }
            ok(out)
        })
        .await
    }

    #[tool(
        description = "Read a local (flat or scanned) PDF to place form boxes: per page its size in points, printed text lines\nwith [x0, y0, x1, y1] boxes (PDF points, origin bottom-left) and existing form fields, plus each page as an\nimage with a coordinate grid (lines every 50 pt, labels every 100 pt). Default pages: all."
    )]
    async fn inspect_pdf_layout(&self, Parameters(a): Parameters<InspectPdfLayoutArgs>) -> ToolResult {
        self.run(move |_| {
            let pdf = read_file(&a.path)?;
            let sizes = preview::page_sizes(&pdf)?;
            let mut selected = a.pages.unwrap_or_default();
            if selected.is_empty() {
                selected = (1..=sizes.len()).collect();
            }
            if let Some(bad) = selected.iter().find(|p| **p == 0 || **p > sizes.len()) {
                return Err(format!("page must be 1..{} (got {bad})", sizes.len()));
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
            let info = json!({"pages": pages, "form_fields": pdf_form::inspect_form(&pdf)?});
            let mut out = vec![ContentBlock::text(dump(&info))];
            for page in selected {
                out.push(png(preview::grid_page(&pdf, page, dpi(a.dpi))?));
            }
            ok(out)
        })
        .await
    }

    #[tool(
        description = "Make a PDF form template from a flat PDF: each box {name, page, rect: [x0, y0, x1, y1], type: text|check,\nmax_len?, comb?} (PDF points, origin bottom-left) becomes a form field named `name`. fields maps those names\nto Jinja expressions over roles; labels maps them to human labels for the fill form. Omit source_path to\nrebuild from the flat PDF saved earlier (replaces all boxes). Returns the template and its pages with the\nboxes outlined, to check placement."
    )]
    async fn save_pdf_overlay_template(&self, Parameters(a): Parameters<SavePdfOverlayArgs>) -> ToolResult {
        self.run(move |app| {
            check_id(&a.template_id)?;
            let flat = match &a.source_path {
                Some(path) => read_file(path)?,
                None => {
                    let saved = app.templates.root().join(&a.template_id).join("flat.pdf");
                    std::fs::read(&saved)
                        .map_err(|_| "Pass source_path: no flat PDF saved for this template yet".to_string())?
                }
            };
            let boxes: Vec<Value> = a.boxes.into_iter().map(Value::Object).collect();
            let form = pdf_form::add_fields(&flat, &boxes)?;
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
        description = "Create or replace a template. kind: pdf_form (source_path = blank PDF form), typst (source_text,\nreads `json(bytes(sys.inputs.data))` for the field values) or docx (source_path, docxtpl Jinja tags).\nfields maps PDF field names / template variables to Jinja expressions over the roles.\nroles maps role names (e.g. applicant, loan) to a short description. Omit the source to keep\nthe existing one."
    )]
    async fn save_template(&self, Parameters(a): Parameters<SaveTemplateArgs>) -> ToolResult {
        self.run(move |app| {
            let source = a.source_path.as_deref().map(expand);
            let existing = app.templates.get(&a.template_id).ok();
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
            })?;
            app.audit("mcp", "save_template", &a.template_id);
            text(&engine::describe_template(&template))
        })
        .await
    }

    #[tool(
        description = "Paragraphs of a local .docx (body and table cells) with their run texts and existing {{ }} tags.\nUse the run texts to pick spans for save_docx_template: a span cannot cross a run boundary."
    )]
    async fn inspect_docx(&self, Parameters(a): Parameters<PathArgs>) -> ToolResult {
        self.run(move |_| text(&docx::inspect(&expand(&a.path))?)).await
    }

    #[tool(
        description = "Create a docx template from a filled .docx: each replacement {find, var, occurrence?} replaces the\ntext with {{ var }} (occurrence is 1-based across the document; omit it to replace all). Replacements\napply in order, so put longer spans first (\".......2026\" before \"......\"). fields maps\nvar -> default Jinja expression over roles; labels maps var -> human label shown in the fill form."
    )]
    async fn save_docx_template(&self, Parameters(a): Parameters<SaveDocxArgs>) -> ToolResult {
        self.run(move |app| {
            check_id(&a.template_id)?;
            let scratch = tempfile_path("docx");
            let replacements: Vec<Value> = a.replacements.into_iter().map(Value::Object).collect();
            let saved = docx::tokenize(&expand(&a.source_path), &scratch, &replacements).and_then(|report| {
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
        description = "Store an invented example dataset keyed by role, e.g. {\"applicant\": {\"name\": \"Иван Иванов\"}}."
    )]
    async fn save_example(&self, Parameters(a): Parameters<SaveExampleArgs>) -> ToolResult {
        self.run(move |app| {
            let data = Value::Object(a.data.unwrap_or_default());
            app.templates.save_example(&a.template_id, &a.name, &data)?;
            ok(vec![ContentBlock::text(format!("Saved example '{}'", a.name))])
        })
        .await
    }

    #[tool(
        description = "Render a template with example data (a saved example name or an inline dict keyed by role).\nReturns warnings, the preview PDF path and PNGs of the selected pages (default: all)."
    )]
    async fn render_example(&self, Parameters(a): Parameters<RenderExampleArgs>) -> ToolResult {
        self.run(move |app| {
            let template = app.templates.get(&a.template_id)?;
            let data = match a.example {
                Value::String(name) => app.templates.get_example(&a.template_id, &name)?,
                data @ Value::Object(_) => data,
                _ => return Err("example must be a saved example name or a dict keyed by role".into()),
            };
            let rendered = engine::render(&template, &example_context(&data))?;
            let out_path = app.config.previews_dir().join(format!("{}-example.{}", a.template_id, rendered.extension));
            std::fs::write(&out_path, &rendered.content).map_err(string)?;
            let mut out = vec![ContentBlock::text(dump(&json!({"path": out_path, "warnings": rendered.warnings})))];
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
        description = "Check a template against real profiles ({role: profile_id}) without revealing values:\nmissing/empty keys always; fit_ratio (> 1 = text too wide) and missing glyphs when the vault is unlocked."
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
        description = "Ask the user to type data into the app. fields: [{key, label, type, required, hint}] with type one of\ntext, number, date, bool, egn, iban, email, phone. Give profile_id to extend a profile, or\nnew_profile_name to create one. Returns a request id; call wait_request for the outcome."
    )]
    async fn request_profile_input(&self, Parameters(a): Parameters<RequestProfileInputArgs>) -> ToolResult {
        self.run(move |app| {
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
            let id = app.create_request(PROFILE_INPUT, payload)?;
            text(&json!({"request_id": id, "status": "pending"}))
        })
        .await
    }

    #[tool(
        description = "Open the fill form for one or more templates. profiles maps every role to a profile id. values are\nnon-personal literals per var (e.g. {\"loan_amount\": \"34 000\"}); paths pick vault fields per var\n(e.g. {\"borrower_egn\": \"manager.egn\"}). Other vars start from the template default, or from the saved\nfill set when fill_set_id is given. name names the fill set. Returns a request id; call wait_request."
    )]
    async fn request_fill(&self, Parameters(a): Parameters<RequestFillArgs>) -> ToolResult {
        self.run(move |app| {
            let name = a.name.trim().to_string();
            let loaded = app.fill_templates(&a.templates)?;
            if loaded.is_empty() || name.is_empty() {
                return Err("Pass at least one template and a name".into());
            }
            let mut missing: Vec<String> =
                fill::roles(&loaded).into_keys().filter(|r| !a.profiles.contains_key(r)).collect();
            if !missing.is_empty() {
                missing.sort();
                return Err(format!("Missing profiles for roles: {}", missing.join(", ")));
            }
            let fields = app.profile_fields(&a.profiles)?;
            let known = fill::variables(&loaded)?;
            for var in a.values.keys().chain(a.paths.keys()) {
                if !known.contains_key(var) {
                    let names: Vec<&str> = known.keys().map(String::as_str).collect();
                    return Err(format!("Unknown variable {var:?}; known: {}", names.join(", ")));
                }
            }
            for (var, path) in &a.paths {
                let (role, key) = path.split_once('.').unwrap_or((path, ""));
                let found = fields
                    .get(role)
                    .is_some_and(|infos| infos.iter().any(|i| i.key == key || i.key.starts_with(&format!("{key}."))));
                if !found {
                    return Err(format!("{var}: no vault field {path:?}"));
                }
            }
            if let Some(id) = a.fill_set_id {
                if !app.vault().list_fill_sets().map_err(string)?.iter().any(|f| f.id == id) {
                    return Err(format!("Unknown fill set {id}"));
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
                "fill_set_id": a.fill_set_id,
                "reason": reason,
            });
            let id = app.create_request(FILL, payload)?;
            text(&json!({"request_id": id, "status": "pending"}))
        })
        .await
    }

    #[tool(description = "Saved fill sets with the source kind per var (path:role.key, value or default). No values.")]
    async fn list_fill_sets(&self) -> ToolResult {
        self.run(|app| {
            let vault = app.vault();
            let sets = vault.list_fill_sets().map_err(string)?;
            let unlocked = vault.unlocked();
            let list: Vec<Value> = sets
                .iter()
                .map(|info| {
                    let mut out = serde_json::to_value(info).unwrap_or_default();
                    if unlocked {
                        if let Ok((_, bindings)) = vault.get_fill_set(info.id) {
                            let vars: Map<String, Value> =
                                bindings.iter().map(|(v, b)| (v.clone(), json!(fill::describe_binding(b)))).collect();
                            out["vars"] = Value::Object(vars);
                        }
                    }
                    out
                })
                .collect();
            text(&list)
        })
        .await
    }

    /// Polls without holding the vault between checks.
    #[tool(
        description = "Wait until the user resolves a request (or the timeout passes). Returns status and metadata only."
    )]
    async fn wait_request(&self, Parameters(a): Parameters<WaitRequestArgs>) -> ToolResult {
        let timeout = a.timeout_s.unwrap_or(300).clamp(0, TOOL_TIMEOUT_MAX as i64) as u64;
        let deadline = Instant::now() + Duration::from_secs(timeout);
        loop {
            let request = self.app.vault().get_request(a.request_id).map_err(string)?;
            if request.status != "pending" || Instant::now() >= deadline {
                return text(&request_view(&request));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    #[tool(description = "Current status of a request.")]
    async fn get_request(&self, Parameters(a): Parameters<RequestArgs>) -> ToolResult {
        self.run(move |app| text(&request_view(&app.vault().get_request(a.request_id).map_err(string)?))).await
    }

    #[tool(description = "List requests, optionally filtered by status (pending, done, cancelled).")]
    async fn list_requests(&self, Parameters(a): Parameters<ListRequestsArgs>) -> ToolResult {
        self.run(move |app| {
            let requests = app.vault().list_requests(a.status.as_deref()).map_err(string)?;
            text(&requests.iter().map(request_view).collect::<Vec<_>>())
        })
        .await
    }

    #[tool(description = "Generated real documents (metadata only; the user exports them from the app).")]
    async fn list_documents(&self) -> ToolResult {
        self.run(|app| text(&app.vault().list_documents().map_err(string)?)).await
    }
}

#[tool_handler]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("blankey", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
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
            out(t.vault_status().await),
            out(t.list_profiles().await),
            out(t.describe_profile(args(json!({"profile_id": pid}))).await),
            out(t.inspect_pdf_form(args(json!({"path": source}))).await),
            out(t.save_template(args(save)).await),
            out(t.list_templates().await),
            out(t.get_template(args(json!({"template_id": "t"}))).await),
            out(t
                .save_example(args(
                    json!({"template_id": "t", "name": "ex", "data": {"applicant": {"name": "Example"}}}),
                ))
                .await),
            out(t.render_example(args(json!({"template_id": "t", "example": "ex"}))).await),
            out(t.check_bindings(args(json!({"template_id": "t", "profiles": {"applicant": pid}}))).await),
            out(t
                .request_fill(args(json!({"templates": ["t"], "profiles": {"applicant": pid}, "name": "leak"})))
                .await),
            out(t
                .request_profile_input(args(
                    json!({"fields": [{"key": "email", "type": "email"}], "reason": "r", "profile_id": pid}),
                ))
                .await),
            out(t.inspect_docx(args(json!({"path": docx}))).await),
            out(t.inspect_pdf_layout(args(json!({"path": source}))).await),
            out(t
                .save_docx_template(args(json!({"template_id": "d", "name": "D", "source_path": docx,
                "replacements": [{"find": "John Smith", "var": "who"}, {"find": "34 000", "var": "amount"}],
                "roles": {"applicant": "person"}, "fields": {"who": "{{ applicant.name }}"}})))
                .await),
        ];
        let profiles: Profiles = [("applicant".to_string(), pid)].into_iter().collect();
        let fill_set = app.vault().save_fill_set("leak", &["t".into()], &profiles, &Map::new(), None).unwrap();
        let docs = app.generate_fill(fill_set).unwrap();
        app.vault().resolve_request(1, "done", json!({"fill_set_id": fill_set, "document_ids": docs})).unwrap();
        let bindings = json!({"amount": {"value": SENTINEL_NAME}}).as_object().cloned().unwrap();
        app.vault().save_fill_set("f", &["d".into()], &profiles, &bindings, None).unwrap();
        outputs.extend([
            out(t.list_documents().await),
            out(t.get_request(args(json!({"request_id": 1}))).await),
            out(t.wait_request(args(json!({"request_id": 1, "timeout_s": 0}))).await),
            out(t.list_requests(args(json!({}))).await),
            out(t.list_fill_sets().await),
        ]);

        let preview = std::fs::read(app.config.previews_dir().join("t-example.pdf")).unwrap();
        let audit: Vec<String> = {
            let vault = app.vault();
            let mut stmt = vault.conn().prepare("SELECT action || target FROM audit").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        let all = [outputs.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join("\n"), audit.join("\n")].join("\n")
            + &String::from_utf8_lossy(&preview);
        for secret in [SENTINEL_NAME, SENTINEL_EGN, "Sentinel", "Hiddenwell"] {
            assert!(!all.contains(secret), "leaked {secret}");
        }
    }

    #[tokio::test]
    async fn request_fill_validates_input() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let (pid, source) = setup(&app, dir.path());
        out(t
            .save_template(args(json!({"template_id": "t", "name": "T", "kind": "pdf_form", "source_path": source,
                "fields": {"name": "{{ applicant.name }}"}, "roles": {"applicant": "person"}})))
            .await);
        let error = t.request_fill(args(json!({"templates": ["t"], "profiles": {}, "name": "n"}))).await.unwrap_err();
        assert!(error.contains("Missing profiles"), "{error}");
        // profile ids as strings are accepted
        let fill = json!({"templates": ["t"], "profiles": {"applicant": pid.to_string()}, "name": "n",
                          "paths": {"name": "applicant.nope"}});
        let error = t.request_fill(args(fill)).await.unwrap_err();
        assert!(error.contains("no vault field"), "{error}");
    }

    #[tokio::test]
    async fn flat_pdf_becomes_a_template() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let t = Tools::new(app.clone());
        let source = fixture("flat.pdf").display().to_string();
        let (layout, grids) = out(t.inspect_pdf_layout(args(json!({"path": source}))).await);
        assert!(layout.contains("\"Name:\"") && grids == 1, "{layout}");
        let save = json!({
            "template_id": "flat", "name": "Flat",
            "boxes": [{"name": "name", "page": 1, "rect": [90, 785, 400, 803]}],
            "roles": {"applicant": "person"},
            "fields": {"name": "{{ applicant.name }}", "ghost": "x"},
            "source_path": source,
        });
        let (summary, images) = out(t.save_pdf_overlay_template(args(save.clone())).await);
        assert!(summary.contains("Binding without field: ghost") && images == 1);

        let mut moved = save;
        moved["boxes"][0]["rect"] = json!([95, 785, 400, 803]);
        moved["source_path"] = Value::Null;
        out(t.save_pdf_overlay_template(args(moved)).await);
        let fields = pdf_form::inspect_form(&app.templates.get("flat").unwrap().source().unwrap()).unwrap();
        assert_eq!(fields[0].rects, [[95.0, 785.0, 400.0, 803.0]]);
    }
}
