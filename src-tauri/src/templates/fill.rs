//! Fill sets: per-variable choice of source (vault path, literal value or template default) across templates.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Map, Value};

use super::bindings::{evaluate, referenced_paths};
use super::engine::Values;
use super::{docx, Template, TemplateKind};
use crate::render::{pdf_form, preview};

pub const PATH: &str = "path";
pub const VALUE: &str = "value";
pub const DEFAULT: &str = "default";

#[derive(Serialize, Clone, Debug, Default)]
pub struct FillVar {
    pub name: String,
    pub label: String,
    /// (template name, surrounding text)
    pub contexts: Vec<(String, String)>,
    /// template id -> expression
    pub defaults: IndexMap<String, String>,
}

pub fn roles(templates: &[Template]) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for template in templates {
        for (role, description) in &template.roles {
            out.entry(role.clone()).or_insert_with(|| description.clone());
        }
    }
    out
}

/// Union of variables in template order; a variable shared by several templates appears once.
pub fn variables(templates: &[Template]) -> Result<IndexMap<String, FillVar>, String> {
    let mut out: IndexMap<String, FillVar> = IndexMap::new();
    for template in templates {
        let mut printed: IndexMap<String, String> = IndexMap::new();
        let (names, contexts): (Vec<String>, IndexMap<String, Vec<String>>) = match template.kind {
            TemplateKind::Docx => {
                let path = template.source_path();
                (docx::variables(&path)?, docx::blank_contexts(&path, 400)?)
            }
            TemplateKind::PdfForm => {
                let source = template.source().map_err(|e| e.to_string())?;
                let fields: Vec<pdf_form::FormField> = pdf_form::inspect_form(&source)?
                    .into_iter()
                    .filter(|f| template.fields.contains_key(&f.name))
                    .collect();
                printed = preview::field_labels(&source, &fields, 60).unwrap_or_default();
                let contexts = printed.iter().map(|(name, text)| (name.clone(), vec![text.clone()])).collect();
                (template.fields.keys().cloned().collect(), contexts)
            }
            TemplateKind::Typst => (template.fields.keys().cloned().collect(), IndexMap::new()),
        };
        let extra: Vec<String> = template.fields.keys().filter(|f| !names.contains(f)).cloned().collect();
        for name in names.iter().chain(extra.iter()) {
            let var = out.entry(name.clone()).or_insert_with(|| FillVar { name: name.clone(), ..Default::default() });
            if var.label.is_empty() {
                var.label = template
                    .labels
                    .get(name)
                    .filter(|l| !l.is_empty())
                    .or_else(|| printed.get(name))
                    .cloned()
                    .unwrap_or_default();
            }
            for context in contexts.get(name).into_iter().flatten() {
                var.contexts.push((template.name.clone(), context.clone()));
            }
            if let Some(expression) = template.fields.get(name) {
                var.defaults.insert(template.id.clone(), expression.clone());
            }
        }
    }
    Ok(out)
}

fn unique_defaults(var: &FillVar) -> Vec<&String> {
    let mut seen: Vec<&String> = Vec::new();
    for expression in var.defaults.values() {
        if !seen.contains(&expression) {
            seen.push(expression);
        }
    }
    seen
}

/// A plain `{{ role.key }}` default becomes a path binding; anything else stays the template default.
pub fn default_binding(var: &FillVar, role_names: &HashSet<String>) -> Value {
    let expressions = unique_defaults(var);
    if let [expression] = expressions[..] {
        let paths = referenced_paths(expression, role_names);
        if paths.len() == 1 {
            let path = paths.iter().next().expect("one path");
            if expression.replace(' ', "") == format!("{{{{{path}}}}}") {
                return json!({PATH: path});
            }
        }
    }
    if expressions.is_empty() {
        json!({VALUE: ""})
    } else {
        json!({DEFAULT: true})
    }
}

pub fn evaluate_binding(binding: &Value, context: &Value, expression: Option<&str>) -> Result<Value, String> {
    if let Some(path) = binding.get(PATH).and_then(Value::as_str) {
        return evaluate(&format!("{{{{ {path} }}}}"), context);
    }
    if let Some(value) = binding.get(VALUE) {
        return Ok(value.clone());
    }
    match expression {
        Some(expression) => evaluate(expression, context),
        None => Ok(Value::Null),
    }
}

/// Values per template id, plus errors.
pub fn resolve(
    templates: &[Template],
    bindings: &Map<String, Value>,
    context: &Value,
) -> Result<IndexMap<String, (Values, Vec<String>)>, String> {
    let mut out = IndexMap::new();
    for template in templates {
        let (mut values, mut errors) = (Values::new(), Vec::new());
        let mut names: Vec<String> =
            if template.kind == TemplateKind::Docx { docx::variables(&template.source_path())? } else { Vec::new() };
        for field in template.fields.keys() {
            if !names.contains(field) {
                names.push(field.clone());
            }
        }
        for name in names {
            let default = json!({DEFAULT: true});
            let binding =
                bindings.get(&name).filter(|b| b.as_object().is_some_and(|o| !o.is_empty())).unwrap_or(&default);
            match evaluate_binding(binding, context, template.fields.get(&name).map(String::as_str)) {
                Ok(value) => {
                    values.insert(name, value);
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        out.insert(template.id.clone(), (values, errors));
    }
    Ok(out)
}

/// Metadata-only description (safe for MCP).
pub fn describe_binding(binding: &Value) -> String {
    if let Some(path) = binding.get(PATH).and_then(Value::as_str) {
        return format!("path:{path}");
    }
    if binding.get(VALUE).is_some() { VALUE } else { DEFAULT }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(defaults: &[(&str, &str)]) -> FillVar {
        FillVar {
            name: "x".into(),
            defaults: defaults.iter().map(|(t, e)| (t.to_string(), e.to_string())).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn default_binding_prefers_plain_paths() {
        let roles: HashSet<String> = ["manager".to_string()].into_iter().collect();
        assert_eq!(default_binding(&var(&[("a", "{{ manager.egn }}")]), &roles), json!({"path": "manager.egn"}));
        assert_eq!(default_binding(&var(&[("a", "{{ manager.name | upper }}")]), &roles), json!({"default": true}));
        assert_eq!(
            default_binding(&var(&[("a", "{{ manager.egn }}"), ("b", "{{ manager.name }}")]), &roles),
            json!({"default": true})
        );
        assert_eq!(default_binding(&var(&[]), &roles), json!({"value": ""}));
        assert_eq!(describe_binding(&json!({"path": "manager.egn"})), "path:manager.egn");
        assert_eq!(describe_binding(&json!({"value": "secret"})), "value");
    }

    #[test]
    fn bindings_pick_their_source() {
        let context = json!({"manager": {"egn": "7501020018", "name": "Иван"}});
        assert_eq!(evaluate_binding(&json!({"path": "manager.egn"}), &context, None).unwrap(), json!(7501020018_i64));
        assert_eq!(evaluate_binding(&json!({"value": "34 000"}), &context, Some("x")).unwrap(), json!("34 000"));
        assert_eq!(
            evaluate_binding(&json!({"default": true}), &context, Some("{{ manager.name | upper }}")).unwrap(),
            json!("ИВАН")
        );
    }
}
