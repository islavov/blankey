//! UI languages. One catalog per language (`ui/i18n/<code>.json`), keyed by the English text and shared with the
//! window, so the tray and the window always agree. English needs no catalog. MCP output is never translated.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{Map, Value};

pub const DEFAULT: &str = "en";

const LANGUAGES: &str = include_str!("../../ui/i18n/languages.json");
const CATALOGS: &[(&str, &str)] = &[("bg", include_str!("../../ui/i18n/bg.json"))];

type Catalog = Map<String, Value>;

fn catalogs() -> &'static HashMap<&'static str, Catalog> {
    static CATALOGS_PARSED: OnceLock<HashMap<&'static str, Catalog>> = OnceLock::new();
    CATALOGS_PARSED.get_or_init(|| {
        CATALOGS.iter().map(|(code, text)| (*code, serde_json::from_str(text).expect("valid catalog"))).collect()
    })
}

/// Language code -> its own name, in the order the window lists them.
pub fn languages() -> Map<String, Value> {
    serde_json::from_str(LANGUAGES).expect("valid languages.json")
}

/// The catalog the window loads; empty for English and unknown languages.
pub fn catalog(lang: &str) -> Map<String, Value> {
    catalogs().get(lang).cloned().unwrap_or_default()
}

pub fn is_supported(code: &str) -> bool {
    languages().contains_key(code)
}

/// Marks text that is translated later, by the window or another `t` call; the catalog check finds it.
pub const fn mark(text: &'static str) -> &'static str {
    text
}

fn entry(lang: &str, key: &str) -> Option<&'static Value> {
    catalogs().get(lang)?.get(key)
}

/// The translation of `text`, or `text` itself when there is none.
pub fn t(lang: &str, text: &str) -> String {
    match entry(lang, text) {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => text.to_string(),
    }
}

/// `t` with `{name}` placeholders filled in.
pub fn tf(lang: &str, text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(t(lang, text), args)
}

/// The singular or plural form for `n`, keyed by the English singular; `{n}` is filled in. English and Bulgarian
/// both use one and other.
pub fn tn(lang: &str, n: usize, one: &str, other: &str) -> String {
    let form = if n == 1 { "one" } else { "other" };
    let text = match entry(lang, one) {
        Some(Value::Object(forms)) => forms.get(form).and_then(Value::as_str).map(str::to_string),
        _ => None,
    };
    let text = text.unwrap_or_else(|| if n == 1 { one } else { other }.to_string());
    fill(text, &[("n", &n)])
}

fn fill(mut text: String, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

/// The computer's language, when it is one we have.
pub fn system_language() -> Option<String> {
    let locale = sys_locale::get_locale()?;
    let code = locale.split(['-', '_']).next()?.to_lowercase();
    is_supported(&code).then_some(code)
}

/// The language picked on this computer, else the account's when the relay gave one, else the computer's, else
/// English; codes we have no catalog for are skipped.
pub fn resolve(device: Option<&str>, profile: Option<&str>) -> String {
    [device, profile]
        .into_iter()
        .flatten()
        .find(|code| is_supported(code))
        .map(str::to_string)
        .or_else(system_language)
        .unwrap_or_else(|| DEFAULT.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_parse_and_name_their_languages() {
        let names = languages();
        assert_eq!(names["en"], "English");
        for (code, _) in CATALOGS {
            assert!(names.contains_key(*code), "{code} is not in languages.json");
            assert!(catalogs().contains_key(code));
        }
    }

    #[test]
    fn lookup_falls_back_to_english() {
        assert_eq!(t("en", "Lock"), "Lock");
        assert_eq!(t("xx", "Lock"), "Lock");
        assert_eq!(t("bg", "No such text, ever"), "No such text, ever");
        assert_eq!(tf("en", "Open Blankey ({n} waiting)", &[("n", &3)]), "Open Blankey (3 waiting)");
        assert_eq!(tn("en", 1, "{n} field", "{n} fields"), "1 field");
        assert_eq!(tn("en", 2, "{n} field", "{n} fields"), "2 fields");
    }

    #[test]
    fn bulgarian_forms() {
        assert_eq!(t("bg", "Lock"), "Заключи");
        assert_eq!(tn("bg", 1, "{n} field", "{n} fields"), "1 поле");
        assert_eq!(tn("bg", 5, "{n} field", "{n} fields"), "5 полета");
    }

    #[test]
    fn precedence() {
        assert_eq!(resolve(None, Some("bg")), "bg");
        assert_eq!(resolve(Some("en"), Some("bg")), "en");
        assert_eq!(resolve(Some("xx"), Some("bg")), "bg");
        let fallback = system_language().unwrap_or_else(|| DEFAULT.into());
        assert_eq!(resolve(None, Some("xx")), fallback);
        assert_eq!(resolve(None, None), fallback);
    }
}
