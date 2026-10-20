//! A category and tags for a library file from its name and folders alone, no content and no model.

use std::sync::LazyLock;

use regex::Regex;

use crate::mcp::find::fold;

static TABLE: LazyLock<Vec<(String, Vec<String>)>> = LazyLock::new(|| {
    let data: serde_yaml::Mapping = serde_yaml::from_str(include_str!("categories.yaml")).expect("categories.yaml");
    data.into_iter()
        .filter_map(|(category, stems)| {
            let stems = stems.as_sequence()?.iter().filter_map(|s| s.as_str()).map(fold).collect();
            Some((category.as_str()?.to_string(), stems))
        })
        .collect()
});

static CAMEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\p{Ll})(\p{Lu})").unwrap());
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\d+|v\d+|rev\d*|final|draft|copy|new|old|копие|нов|нова|ново|стар|стара|окончателен|проект|bg|en)$")
        .unwrap()
});

/// Words of one name: camel case and separators split, folded, numbers and version noise dropped.
pub fn words(name: &str) -> Vec<String> {
    let spaced = CAMEL.replace_all(name, "$1 $2");
    fold(&spaced)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !NOISE.is_match(w))
        .map(str::to_string)
        .collect()
}

fn category_of(words: &[String]) -> Option<String> {
    TABLE
        .iter()
        .find(|(_, stems)| words.iter().any(|w| stems.iter().any(|s| w.starts_with(s.as_str()))))
        .map(|(category, _)| category.clone())
}

/// The category from the file name, else from the nearest folder that has one; the folders as tags.
pub fn categorize(rel_path: &str) -> (Option<String>, Vec<String>) {
    let mut parts: Vec<&str> = rel_path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let file = parts.pop().unwrap_or_default();
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let category = std::iter::once(stem).chain(parts.iter().rev().copied()).find_map(|name| category_of(&words(name)));
    let mut tags: Vec<String> = parts.iter().map(|p| words(p).join(" ")).filter(|t| !t.is_empty()).collect();
    tags.dedup();
    (category, tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_drop_noise() {
        assert_eq!(words("LeaseAgreement_v2 FINAL (copy) 2026-03-01"), ["lease", "agreement"]);
        assert_eq!(words("Договор-за-наем_нов"), ["договор", "за", "наем"]);
    }

    #[test]
    fn specific_categories_win() {
        assert_eq!(categorize("Договор за наем на апартамент.docx").0.as_deref(), Some("lease"));
        assert_eq!(categorize("Договор за услуги.docx").0.as_deref(), Some("services"));
        assert_eq!(categorize("Договор.docx").0.as_deref(), Some("contract"));
        assert_eq!(categorize("Пълномощно - общо.pdf").0.as_deref(), Some("power_of_attorney"));
        assert_eq!(categorize("Employment contract template.docx").0.as_deref(), Some("employment"));
        assert_eq!(categorize("notes.docx").0, None);
    }

    #[test]
    fn folders_give_tags_and_a_fallback_category() {
        let (category, tags) = categorize("Models/Leases/Sample 3.docx");
        assert_eq!(category.as_deref(), Some("lease"));
        assert_eq!(tags, ["models", "leases"]);
        assert_eq!(categorize("Declarations\\GDPR consent.pdf").0.as_deref(), Some("declaration"));
    }
}
