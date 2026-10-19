//! Binding expressions: sandboxed Jinja (minijinja) with native result types, evaluated against role contexts.

use std::collections::{BTreeSet, HashSet};
use std::str::FromStr;
use std::sync::{LazyLock, OnceLock};

use indexmap::IndexMap;
use minijinja::{Environment, UndefinedBehavior};
use regex::Regex;
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Map, Value};

use crate::fieldtypes;

static TAG_REGION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\{\{(.*?)\}\}|\{%(.*?)%\}").unwrap());
static BARE_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+)(\.\w+)+$").unwrap());

fn environment() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Chainable);
        env.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
        env.add_filter("money", money_filter);
        env.add_function("today", today);
        env
    })
}

fn money_filter(value: minijinja::Value, decimals: Option<u32>) -> Result<String, minijinja::Error> {
    if value.is_undefined() || value.is_none() {
        return Ok(String::new());
    }
    money(&value.to_string(), decimals).map_err(|e| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e))
}

/// "1234.5" -> "1 234,50": spaces between thousands, a decimal comma, two decimals unless whole.
pub fn money(text: &str, decimals: Option<u32>) -> Result<String, String> {
    let cleaned = text.replace(' ', "").replace(',', ".");
    if cleaned.is_empty() {
        return Ok(String::new());
    }
    let number = Decimal::from_str(&cleaned)
        .or_else(|_| Decimal::from_scientific(&cleaned))
        .map_err(|_| format!("money: not a number: {text:?}"))?;
    let decimals = decimals.unwrap_or(if number.fract().is_zero() { 0 } else { 2 });
    let rounded = number.round_dp_with_strategy(decimals, RoundingStrategy::MidpointNearestEven);
    let formatted = format!("{:.*}", decimals as usize, rounded.abs());
    let (whole, fraction) = formatted.split_once('.').unwrap_or((&formatted, ""));
    let mut grouped = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(' ');
        }
        grouped.push(ch);
    }
    let sign = if rounded.is_sign_negative() && !rounded.is_zero() { "-" } else { "" };
    Ok(if fraction.is_empty() { format!("{sign}{grouped}") } else { format!("{sign}{grouped},{fraction}") })
}

pub fn today() -> String {
    chrono::Local::now().format("%d.%m.%Y").to_string()
}

/// The expression inside "{{ ... }}" when the whole template is one output tag (Jinja native single node).
fn single_expression(template: &str) -> Option<&str> {
    let inner = template.strip_prefix("{{")?.strip_suffix("}}")?;
    if inner.contains("{{") || inner.contains("}}") || inner.contains("{%") || inner.contains("{#") {
        return None;
    }
    let inner = inner.strip_prefix('-').unwrap_or(inner);
    Some(inner.strip_suffix('-').unwrap_or(inner))
}

/// What Python's ast.literal_eval made of a rendered string in Jinja's native mode (numbers, bools, None).
fn literal(text: String) -> Value {
    static INT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-?(0|[1-9](_?[0-9])*)$").unwrap());
    static FLOAT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^-?([0-9]+\.[0-9]*|\.[0-9]+|[0-9]+)([eE][+-]?[0-9]+)?$").unwrap());
    match text.as_str() {
        "True" => Value::Bool(true),
        "False" => Value::Bool(false),
        "None" => Value::Null,
        t if INT.is_match(t) => t.replace('_', "").parse::<i64>().map(Value::from).unwrap_or(Value::String(text)),
        t if FLOAT.is_match(t) && (t.contains('.') || t.contains(['e', 'E'])) => t
            .parse::<f64>()
            .ok()
            .and_then(|f| serde_json::Number::from_f64(f).map(Value::Number))
            .unwrap_or(Value::String(text)),
        _ => Value::String(text),
    }
}

fn to_json(value: minijinja::Value) -> Value {
    if value.is_undefined() || value.is_none() {
        return Value::Null;
    }
    match serde_json::to_value(&value) {
        Ok(Value::String(s)) => literal(s),
        Ok(other) => other,
        Err(_) => Value::String(value.to_string()),
    }
}

/// Evaluates a binding: a single "{{ expr }}" keeps its native type, anything else renders to text that is then
/// read as a literal when it looks like one. Undefined values become null.
pub fn evaluate(expression: &str, context: &Value) -> Result<Value, String> {
    let env = environment();
    let ctx = minijinja::Value::from_serialize(context);
    if expression.is_empty() {
        return Ok(Value::Null);
    }
    if let Some(inner) = single_expression(expression) {
        let compiled = env.compile_expression(inner.trim()).map_err(describe)?;
        return compiled.eval(ctx).map(to_json).map_err(describe);
    }
    env.render_str(expression, ctx).map(literal).map_err(describe)
}

fn describe(error: minijinja::Error) -> String {
    let mut text = error.kind().to_string();
    if let Some(detail) = error.detail() {
        text = format!("{text}: {detail}");
    }
    text
}

pub fn as_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Array(items) => items.iter().map(as_text).collect::<Vec<_>>().join(", "),
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e16 => format!("{f:.1}"),
            _ => n.to_string(),
        },
        Value::Object(_) => value.to_string(),
    }
}

pub fn is_empty(value: &Value) -> bool {
    as_text(value).trim().is_empty()
}

/// Dotted paths rooted at a role, e.g. "applicant.address.city"; only the longest chains are kept.
pub fn referenced_paths(expression: &str, roles: &HashSet<String>) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for region in TAG_REGION.captures_iter(expression) {
        let code = region.get(1).or_else(|| region.get(2)).map_or("", |m| m.as_str());
        scan_paths(code, roles, &mut paths);
    }
    let all: Vec<String> = paths.iter().cloned().collect();
    paths.retain(|p| !all.iter().any(|other| other != p && other.starts_with(&format!("{p}."))));
    paths
}

/// A binding written as plain text "role.key" instead of "{{ role.key }}": it would render as that text.
pub fn bare_path(expression: &str, roles: &HashSet<String>) -> Option<String> {
    let text = expression.trim();
    let role = BARE_PATH.captures(text)?.get(1)?.as_str();
    roles.contains(role).then(|| text.to_string())
}

fn scan_paths(code: &str, roles: &HashSet<String>, out: &mut BTreeSet<String>) {
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    let ident_start = |c: char| c.is_alphabetic() || c == '_';
    let ident_char = |c: char| c.is_alphanumeric() || c == '_';
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' {
            i = skip_string(&chars, i);
            continue;
        }
        if !ident_start(c) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && ident_char(chars[i]) {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        let previous = chars[..start].iter().rev().find(|c| !c.is_whitespace());
        if previous == Some(&'.') || !roles.contains(&name) {
            continue;
        }
        let mut parts = vec![name];
        loop {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '.' {
                j += 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                let attr_start = j;
                while j < chars.len() && ident_char(chars[j]) {
                    j += 1;
                }
                if j == attr_start {
                    break;
                }
                let attr: String = chars[attr_start..j].iter().collect();
                // a method call such as name.upper() is not part of the path
                let mut k = j;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                if k < chars.len() && chars[k] == '(' {
                    break;
                }
                parts.push(attr);
                i = j;
            } else if j < chars.len() && chars[j] == '[' {
                let Some((key, end)) = constant_index(&chars, j + 1) else { break };
                parts.push(key);
                i = end;
            } else {
                break;
            }
        }
        out.insert(parts.join("."));
    }
}

fn skip_string(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let mut i = start + 1;
    while i < chars.len() && chars[i] != quote {
        if chars[i] == '\\' {
            i += 1;
        }
        i += 1;
    }
    i + 1
}

/// `[0]` or `['key']` after a path; returns the key and the index after "]".
fn constant_index(chars: &[char], mut i: usize) -> Option<(String, usize)> {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    let key = if i < chars.len() && (chars[i] == '\'' || chars[i] == '"') {
        let end = skip_string(chars, i);
        let key: String = chars[i + 1..end - 1].iter().collect();
        i = end;
        key
    } else {
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if start == i {
            return None;
        }
        chars[start..i].iter().collect()
    };
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    (i < chars.len() && chars[i] == ']').then_some((key, i + 1))
}

/// {"a.b": 1, "list.0.x": 2} -> nested objects; all-numeric keys become lists.
pub fn nest(flat: &IndexMap<String, Value>) -> Value {
    let mut root = Map::new();
    for (key, value) in flat {
        let parts: Vec<&str> = key.split('.').collect();
        let (leaf, parents) = parts.split_last().expect("split yields one part");
        let mut node = &mut root;
        for part in parents {
            let entry = node.entry(part.to_string()).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            node = entry.as_object_mut().expect("object");
        }
        if !node.get(*leaf).is_some_and(Value::is_object) {
            node.insert(leaf.to_string(), value.clone());
        }
    }
    listify(Value::Object(root))
}

fn listify(node: Value) -> Value {
    let Value::Object(map) = node else { return node };
    let converted: Map<String, Value> = map.into_iter().map(|(k, v)| (k, listify(v))).collect();
    if !converted.is_empty() && converted.keys().all(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_digit())) {
        let mut items: Vec<(u64, Value)> =
            converted.into_iter().map(|(k, v)| (k.parse().unwrap_or(u64::MAX), v)).collect();
        items.sort_by_key(|(k, _)| *k);
        return Value::Array(items.into_iter().map(|(_, v)| v).collect());
    }
    Value::Object(converted)
}

fn flatten(data: &Value, prefix: &str, out: &mut IndexMap<String, Value>) {
    match data {
        Value::Object(map) => {
            for (key, value) in map {
                flatten(value, &format!("{prefix}{key}."), out);
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                flatten(value, &format!("{prefix}{index}."), out);
            }
        }
        _ => {
            out.insert(prefix.trim_end_matches('.').to_string(), data.clone());
        }
    }
}

/// key -> (type, value) of one profile as the nested object templates see.
pub fn profile_context(values: &IndexMap<String, (String, String)>) -> Value {
    let flat: IndexMap<String, Value> = values
        .iter()
        .filter(|(_, (_, value))| !value.is_empty())
        .map(|(key, (field_type, value))| (key.clone(), fieldtypes::coerce(field_type, value)))
        .collect();
    nest(&flat)
}

/// An invented example dataset keyed by role, normalized like real profiles.
pub fn example_context(example: &Value) -> Value {
    let Some(roles) = example.as_object() else { return Value::Object(Map::new()) };
    Value::Object(
        roles
            .iter()
            .map(|(role, data)| {
                let mut flat = IndexMap::new();
                flatten(data, "", &mut flat);
                (role.clone(), nest(&flat))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roles(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn referenced_paths_follow_the_chains() {
        let r = roles(&["applicant", "loan"]);
        let set = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(referenced_paths("{{ applicant.address.city }}", &r), set(&["applicant.address.city"]));
        assert_eq!(
            referenced_paths("{{ loan.obligations[0].creditor }} {{ x.y }}", &r),
            set(&["loan.obligations.0.creditor"])
        );
        assert_eq!(
            referenced_paths("{{ 'X' if applicant.a == 'b' else loan.c | money }}", &r),
            set(&["applicant.a", "loan.c"])
        );
        assert_eq!(referenced_paths("{{ 'applicant.name' }} applicant.x", &r), set(&[]));
        assert_eq!(referenced_paths("{{ applicant.name.upper() }}", &r), set(&["applicant.name"]));
    }

    #[test]
    fn bare_paths_are_detected() {
        let r = roles(&["applicant", "loan"]);
        assert_eq!(bare_path(" applicant.address.city ", &r).as_deref(), Some("applicant.address.city"));
        assert_eq!(bare_path("loan.items.0", &r).as_deref(), Some("loan.items.0"));
        assert_eq!(bare_path("other.name", &r), None);
        assert_eq!(bare_path("{{ applicant.name }}", &r), None);
        assert_eq!(bare_path("applicant.name | upper", &r), None);
        assert_eq!(bare_path("applicant", &r), None);
        assert_eq!(bare_path("гр. София", &r), None);
    }

    #[test]
    fn nest_builds_lists() {
        let flat: IndexMap<String, Value> = [("a.b", json!(1)), ("l.1.x", json!("b")), ("l.0.x", json!("a"))]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(nest(&flat), json!({"a": {"b": 1}, "l": [{"x": "a"}, {"x": "b"}]}));
    }

    #[test]
    fn evaluate_native_and_filters() {
        let ctx = example_context(
            &json!({"loan": {"amount": 150000, "items": [1, 2]}, "p": {"name": "Иван", "egn": "0041010101"}}),
        );
        assert_eq!(evaluate("{{ loan.amount | money }}", &ctx).unwrap(), json!("150 000"));
        assert_eq!(evaluate("{{ loan.missing.deep }}", &ctx).unwrap(), Value::Null);
        assert_eq!(evaluate("{{ loan.items | sum }}", &ctx).unwrap(), json!(3));
        assert_eq!(evaluate("{{ loan.amount > 1000 }}", &ctx).unwrap(), json!(true));
        assert_eq!(evaluate("{{ p.name | upper }} е", &ctx).unwrap(), json!("ИВАН е"));
        assert_eq!(evaluate("{{ p.name.upper() }}", &ctx).unwrap(), json!("ИВАН"));
        assert_eq!(evaluate("{{ p.egn }}", &ctx).unwrap(), json!("0041010101"));
        assert_eq!(evaluate("{{ 'X' if p.missing }}", &ctx).unwrap(), Value::Null);
        assert_eq!(evaluate("София", &ctx).unwrap(), json!("София"));
        assert_eq!(evaluate("", &ctx).unwrap(), Value::Null);
        assert!(evaluate("{{ p.name | nope }}", &ctx).is_err());
        assert_eq!(money("1234.5", None).unwrap(), "1 234,50");
        assert_eq!(money("1 234 567", None).unwrap(), "1 234 567");
        assert_eq!(money("-1234.567", Some(2)).unwrap(), "-1 234,57");
        assert_eq!(evaluate("{{ today() }}", &ctx).unwrap(), json!(today()));
    }

    #[test]
    fn text_of_values() {
        assert_eq!(as_text(&json!(null)), "");
        assert_eq!(as_text(&json!(true)), "true");
        assert_eq!(as_text(&json!([1, "a"])), "1, a");
        assert_eq!(as_text(&json!(2.5)), "2.5");
        assert_eq!(as_text(&json!(3.0)), "3.0");
    }

    #[test]
    fn profile_values_are_coerced() {
        let values: IndexMap<String, (String, String)> = [
            ("amount".to_string(), ("number".to_string(), "1 500".to_string())),
            ("address.city".to_string(), ("text".to_string(), "София".to_string())),
            ("married".to_string(), ("bool".to_string(), "да".to_string())),
            ("empty".to_string(), ("text".to_string(), String::new())),
        ]
        .into_iter()
        .collect();
        assert_eq!(profile_context(&values), json!({"amount": 1500, "address": {"city": "София"}, "married": true}));
    }
}
