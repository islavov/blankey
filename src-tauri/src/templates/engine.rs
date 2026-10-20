//! Render templates to documents and check bindings against profiles.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value};

use super::bindings::{as_text, bare_path, evaluate, plain_path, referenced_paths};
use super::{docx, typst, Template, TemplateKind};
use crate::render::{fonts, pdf_form};
use crate::vault::FieldInfo;

pub struct Rendered {
    pub content: Vec<u8>,
    /// "pdf" or "docx"
    pub extension: &'static str,
    pub warnings: Vec<String>,
}

impl Rendered {
    pub fn is_pdf(&self) -> bool {
        self.extension == "pdf"
    }
}

/// What the assistant learns about one binding. Only statuses, never measurements or characters of a value: the
/// assistant writes the expressions, so anything computed from real values would let it read them.
#[derive(Serialize, Debug, Clone)]
pub struct BindingReport {
    pub field: String,
    /// ok | literal | missing | empty | overflow | glyphs
    pub status: String,
    pub missing: Vec<String>,
    pub empty: Vec<String>,
    pub error: Option<String>,
}

impl BindingReport {
    fn new(field: &str) -> Self {
        Self { field: field.to_string(), status: "ok".into(), missing: Vec::new(), empty: Vec::new(), error: None }
    }
}

pub type Values = IndexMap<String, Value>;

pub fn resolve(template: &Template, context: &Value) -> (Values, Vec<String>) {
    let (mut values, mut errors) = (Values::new(), Vec::new());
    for (name, expression) in &template.fields {
        match evaluate(expression, context) {
            Ok(value) => {
                values.insert(name.clone(), value);
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    (values, errors)
}

/// Renders invented example data: the only case where Typst may download packages (see `typst::compile`).
pub fn render_example(template: &Template, context: &Value) -> Result<Rendered, String> {
    let (values, errors) = resolve(template, context);
    render_with(template, &values, errors, true)
}

/// Renders values that may be real vault data: no network access.
pub fn render_values(template: &Template, values: &Values, errors: Vec<String>) -> Result<Rendered, String> {
    render_with(template, values, errors, false)
}

fn render_with(template: &Template, values: &Values, errors: Vec<String>, downloads: bool) -> Result<Rendered, String> {
    let mut warnings = errors;
    let source = template.source().map_err(|e| format!("{}: {e}", template.source_path().display()))?;
    match template.kind {
        TemplateKind::PdfForm => {
            let known: HashSet<String> = pdf_form::inspect_form(&source)?.into_iter().map(|f| f.name).collect();
            let mut unknown: Vec<&String> = values.keys().filter(|k| !known.contains(*k)).collect();
            unknown.sort();
            if !unknown.is_empty() {
                let names: Vec<&str> = unknown.iter().map(|s| s.as_str()).collect();
                warnings.push(format!("Fields not in the PDF form: {}", names.join(", ")));
            }
            let text: IndexMap<String, String> =
                values.iter().filter(|(k, _)| known.contains(*k)).map(|(k, v)| (k.clone(), as_text(v))).collect();
            Ok(Rendered { content: pdf_form::fill_form(&source, &text)?, extension: "pdf", warnings })
        }
        TemplateKind::Typst => {
            let data = serde_json::to_string(values).map_err(|e| e.to_string())?;
            Ok(Rendered { content: typst::compile(&template.directory, &data, downloads)?, extension: "pdf", warnings })
        }
        TemplateKind::Docx => {
            let tags = docx::variables(&template.source_path())?;
            let unbound: Vec<&str> = tags.iter().filter(|t| !values.contains_key(*t)).map(String::as_str).collect();
            if !unbound.is_empty() {
                let mut sorted = unbound.clone();
                sorted.sort();
                warnings.push(format!("Tags without a value: {}", sorted.join(", ")));
            }
            let text: IndexMap<String, String> = values.iter().map(|(k, v)| (k.clone(), as_text(v))).collect();
            let rendered = docx::render(&source, &text)?;
            match docx::to_pdf(&rendered)? {
                Some(pdf) => Ok(Rendered { content: pdf, extension: "pdf", warnings }),
                None => {
                    warnings.push("LibreOffice not found: output is .docx and cannot be previewed".into());
                    Ok(Rendered { content: rendered, extension: "docx", warnings })
                }
            }
        }
    }
}

/// Checks each binding. `profile_fields` is metadata per role; `context` holds real values when the vault is
/// unlocked and enables fit / glyph checks, for plain "{{ role.key }}" bindings only. Those measure the stored value
/// as it is, so the expression cannot choose what is measured; computed expressions are never evaluated on real data
/// (a conditional error or a character mapping would turn the report into a channel for the value). What remains:
/// whether a stored value fits a box the assistant sized, which narrows down its width, never its characters.
pub fn check(
    template: &Template,
    profile_fields: &IndexMap<String, Vec<FieldInfo>>,
    context: Option<&Value>,
) -> Result<Vec<BindingReport>, String> {
    let roles: HashSet<String> = template.roles.keys().chain(profile_fields.keys()).cloned().collect();
    let form_fields: IndexMap<String, pdf_form::FormField> = if template.kind == TemplateKind::PdfForm {
        let source = template.source().map_err(|e| e.to_string())?;
        pdf_form::inspect_form(&source)?.into_iter().map(|f| (f.name.clone(), f)).collect()
    } else {
        IndexMap::new()
    };
    let mut reports = Vec::new();
    for (name, expression) in &template.fields {
        let mut report = BindingReport::new(name);
        let bare = bare_path(expression, &roles);
        let paths = match &bare {
            Some(path) => [path.clone()].into(),
            None => referenced_paths(expression, &roles),
        };
        for path in paths {
            let (role, key) = path.split_once('.').unwrap_or((&path, ""));
            let Some(infos) = profile_fields.get(role) else {
                report.missing.push(path.clone());
                continue;
            };
            let matches: Vec<&FieldInfo> = infos
                .iter()
                .filter(|i| !key.is_empty() && (i.key == key || i.key.starts_with(&format!("{key}."))))
                .collect();
            if matches.is_empty() {
                report.missing.push(path.clone());
            } else if !matches.iter().any(|i| i.length > 0) {
                report.empty.push(path.clone());
            }
        }
        if !report.missing.is_empty() {
            report.status = "missing".into();
        } else if !report.empty.is_empty() {
            report.status = "empty".into();
        }
        if let Some(path) = bare {
            report.status = "literal".into();
            report.error = Some(format!("plain text, not a binding; write \"{{{{ {path} }}}}\""));
            reports.push(report);
            continue;
        }
        if let (Some(context), Some(_)) = (context, plain_path(expression, &roles)) {
            let text = evaluate(expression, context).map(|v| as_text(&v)).unwrap_or_default();
            if let Some(field) = form_fields.get(name) {
                if field.field_type == "text" && !text.is_empty() {
                    let comb_len = if field.comb { field.max_len.unwrap_or(0).max(0) as usize } else { 0 };
                    if !pdf_form::measure(field.rects[0], &text, comb_len).fits && report.status == "ok" {
                        report.status = "overflow".into();
                    }
                }
            }
            if !fonts::missing_glyphs(&text).is_empty() && report.status == "ok" {
                report.status = "glyphs".into();
            }
        }
        reports.push(report);
    }
    Ok(reports)
}

pub fn describe_template(template: &Template) -> Value {
    let mut out = template.manifest();
    out["source"] = json!(template.kind.source_name());
    if template.kind == TemplateKind::Docx {
        let tags = docx::variables(&template.source_path()).unwrap_or_default();
        let mut warnings: Vec<String> = tags
            .iter()
            .filter(|t| !template.fields.contains_key(*t))
            .map(|t| format!("Tag without binding: {t}"))
            .collect();
        warnings
            .extend(template.fields.keys().filter(|f| !tags.contains(f)).map(|f| format!("Binding without tag: {f}")));
        out["tags"] = json!(tags);
        out["warnings"] = json!(warnings);
    }
    out
}

/// Pretty JSON as the tools return it (one-space indent, real Unicode).
pub fn dump(data: &impl Serialize) -> String {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    data.serialize(&mut serializer).expect("JSON serializes");
    String::from_utf8(out).expect("JSON is UTF-8")
}
