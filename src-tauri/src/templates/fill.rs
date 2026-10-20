//! Fill sets: per-variable choice of source (vault path, literal value or template default) across templates.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Map, Value};

use super::bindings::{bare_path, evaluate, plain_path, referenced_paths};
use super::engine::Values;
use super::{docx, Template, TemplateKind};
use crate::render::{pdf_form, preview};
use crate::vault::FieldInfo;

pub const PATH: &str = "path";
pub const VALUE: &str = "value";
pub const DEFAULT: &str = "default";
/// A path binding with this key reads the key from that profile instead of a role's profile.
pub const PROFILE: &str = "profile";

/// Where a pinned profile's values sit in the fill context; never a valid Jinja name, so templates cannot reach it.
pub fn pinned_key(profile_id: i64) -> String {
    format!("#{profile_id}")
}

/// Profiles that path bindings read directly, outside the roles.
pub fn pinned_profiles(bindings: &Map<String, Value>) -> Vec<i64> {
    let mut ids: Vec<i64> = Vec::new();
    for binding in bindings.values() {
        if let Some(id) = binding.get(PROFILE).and_then(Value::as_i64) {
            if binding.get(PATH).is_some() && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Parses "role.key" or "profile:ID.key" into a path binding.
pub fn path_binding(path: &str) -> Result<Value, String> {
    let Some(rest) = path.strip_prefix("profile:") else { return Ok(json!({PATH: path})) };
    let (id, key) = rest.split_once('.').unwrap_or((rest, ""));
    let id: i64 = id.parse().map_err(|_| format!("{path:?}: expected profile:ID.key"))?;
    if key.is_empty() {
        return Err(format!("{path:?}: expected profile:ID.key"));
    }
    Ok(json!({PATH: key, PROFILE: id}))
}

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
        if let Some(path) = plain_path(expression, role_names) {
            return json!({PATH: path});
        }
    }
    if expressions.is_empty() {
        json!({VALUE: ""})
    } else {
        json!({DEFAULT: true})
    }
}

/// Roles the bindings still read: role paths, and the defaults of vars left to them.
pub fn used_roles(
    vars: &IndexMap<String, FillVar>,
    bindings: &Map<String, Value>,
    role_names: &HashSet<String>,
) -> HashSet<String> {
    let mut used = HashSet::new();
    for var in vars.values() {
        let binding = bindings
            .get(&var.name)
            .filter(|b| b.as_object().is_some_and(|o| !o.is_empty()))
            .cloned()
            .unwrap_or_else(|| default_binding(var, role_names));
        if let Some(path) = binding.get(PATH).and_then(Value::as_str) {
            if binding.get(PROFILE).is_none() {
                used.insert(path.split_once('.').map_or(path, |(role, _)| role).to_string());
            }
        } else if binding.get(VALUE).is_none() {
            for expression in var.defaults.values() {
                for path in referenced_paths(expression, role_names) {
                    used.insert(path.split_once('.').map_or(path.as_str(), |(role, _)| role).to_string());
                }
            }
        }
    }
    used.retain(|role| role_names.contains(role));
    used
}

pub fn has_field(infos: &[FieldInfo], key: &str) -> bool {
    !key.is_empty() && infos.iter().any(|i| i.key == key || i.key.starts_with(&format!("{key}.")))
}

/// A var left to its template default that cannot give a vault value: the default is a bare "role.key" (plain
/// text) or reads a key the role's profile does not have.
pub fn uncovered(var: &FillVar, role_names: &HashSet<String>, fields: &IndexMap<String, Vec<FieldInfo>>) -> bool {
    var.defaults.values().any(|expression| {
        bare_path(expression, role_names).is_some()
            || referenced_paths(expression, role_names).iter().any(|path| {
                let (role, key) = path.split_once('.').unwrap_or((path, ""));
                !fields.get(role).is_some_and(|infos| has_field(infos, key))
            })
    })
}

pub fn evaluate_binding(binding: &Value, context: &Value, expression: Option<&str>) -> Result<Value, String> {
    if let Some(path) = binding.get(PATH).and_then(Value::as_str) {
        if let Some(id) = binding.get(PROFILE).and_then(Value::as_i64) {
            let profile = context.get(pinned_key(id)).cloned().unwrap_or_else(|| json!({}));
            return evaluate(&format!("{{{{ p.{path} }}}}"), &json!({"p": profile}));
        }
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
        return match binding.get(PROFILE).and_then(Value::as_i64) {
            Some(id) => format!("path:profile:{id}.{path}"),
            None => format!("path:{path}"),
        };
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
        assert_eq!(describe_binding(&json!({"path": "phone", "profile": 9})), "path:profile:9.phone");
    }

    #[test]
    fn path_bindings_parse_profiles() {
        assert_eq!(path_binding("buyer.phone").unwrap(), json!({"path": "buyer.phone"}));
        assert_eq!(path_binding("profile:9.address.city").unwrap(), json!({"path": "address.city", "profile": 9}));
        assert!(path_binding("profile:x.phone").is_err());
        assert!(path_binding("profile:9").is_err());
        let bindings: Map<String, Value> = [
            ("a".to_string(), json!({"path": "phone", "profile": 9})),
            ("b".to_string(), json!({"path": "egn", "profile": 9})),
            ("c".to_string(), json!({"path": "buyer.egn"})),
        ]
        .into_iter()
        .collect();
        assert_eq!(pinned_profiles(&bindings), vec![9]);
    }

    #[test]
    fn used_roles_follow_the_bindings() {
        let roles: HashSet<String> = ["buyer".to_string(), "seller".to_string()].into_iter().collect();
        let vars: IndexMap<String, FillVar> =
            [("name", "{{ buyer.name }}"), ("phone", "{{ buyer.phone }}"), ("seller", "{{ seller.name | upper }}")]
                .into_iter()
                .map(|(n, e)| {
                    (
                        n.to_string(),
                        FillVar { name: n.into(), defaults: [("t".into(), e.into())].into(), ..Default::default() },
                    )
                })
                .collect();
        let all = used_roles(&vars, &Map::new(), &roles);
        assert_eq!(all, roles);
        let bindings: Map<String, Value> = [
            ("name".to_string(), json!({"path": "name", "profile": 3})),
            ("phone".to_string(), json!({"value": ""})),
            ("seller".to_string(), json!({"path": "buyer.company"})),
        ]
        .into_iter()
        .collect();
        assert_eq!(used_roles(&vars, &bindings, &roles), ["buyer".to_string()].into());
    }

    #[test]
    fn uncovered_defaults() {
        let roles: HashSet<String> = ["buyer".to_string()].into_iter().collect();
        let info =
            |key: &str| FieldInfo { key: key.into(), label: String::new(), field_type: "text".into(), length: 1 };
        let fields: IndexMap<String, Vec<FieldInfo>> =
            [("buyer".to_string(), vec![info("full_name"), info("address.city")])].into_iter().collect();
        assert!(uncovered(&var(&[("a", "buyer.full_name")]), &roles, &fields));
        assert!(uncovered(&var(&[("a", "{{ buyer.phone }}")]), &roles, &fields));
        assert!(!uncovered(&var(&[("a", "{{ buyer.full_name | upper }}")]), &roles, &fields));
        assert!(!uncovered(&var(&[("a", "{{ buyer.address }}")]), &roles, &fields));
        assert!(!uncovered(&var(&[("a", "{{ today() }}")]), &roles, &fields));
    }

    #[test]
    fn bindings_pick_their_source() {
        let context = json!({"manager": {"egn": "7501020018", "name": "Иван"}});
        assert_eq!(evaluate_binding(&json!({"path": "manager.egn"}), &context, None).unwrap(), json!(7501020018_i64));
        let pinned = json!({"manager": {"egn": "1"}, "#9": {"phone": "0888"}});
        assert_eq!(evaluate_binding(&json!({"path": "phone", "profile": 9}), &pinned, None).unwrap(), json!("0888"));
        assert_eq!(evaluate_binding(&json!({"path": "phone", "profile": 4}), &pinned, None).unwrap(), Value::Null);
        assert_eq!(evaluate_binding(&json!({"value": "34 000"}), &context, Some("x")).unwrap(), json!("34 000"));
        assert_eq!(
            evaluate_binding(&json!({"default": true}), &context, Some("{{ manager.name | upper }}")).unwrap(),
            json!("ИВАН")
        );
    }
}
