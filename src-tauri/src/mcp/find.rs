//! Searching catalogs for the assistant: criteria in, one small page out, so results stay cheap at thousands of items.

use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;

pub fn fold(text: &str) -> String {
    text.nfc().collect::<String>().to_lowercase()
}

pub fn words(query: Option<&str>) -> Vec<String> {
    query.map(|q| fold(q).split_whitespace().map(str::to_string).collect()).unwrap_or_default()
}

/// Every word appears in one of the attributes.
pub fn hit(words: &[String], attrs: &[&str]) -> bool {
    if words.is_empty() {
        return true;
    }
    let folded: Vec<String> = attrs.iter().map(|a| fold(a)).collect();
    words.iter().all(|w| folded.iter().any(|a| a.contains(w.as_str())))
}

/// Exact id or name first, then names starting with the query; stable, so the default order holds within each.
pub fn rank_by<T>(items: &mut [T], query: Option<&str>, key: impl Fn(&T) -> (String, String)) {
    let query = fold(query.unwrap_or("").trim());
    if query.is_empty() {
        return;
    }
    items.sort_by_key(|item| {
        let (id, name) = key(item);
        let (id, name) = (fold(&id), fold(&name));
        if id == query || name == query {
            0
        } else if name.starts_with(&query) {
            1
        } else {
            2
        }
    });
}

pub fn window(limit: Option<i64>, offset: Option<i64>) -> (usize, usize) {
    let limit = limit.map_or(DEFAULT_LIMIT, |l| l.clamp(1, MAX_LIMIT as i64) as usize);
    (limit, offset.unwrap_or(0).max(0) as usize)
}

/// `{"total", "next_offset"?, "items"}` as compact JSON, one item per line.
pub fn found<T>(matched: &[T], limit: Option<i64>, offset: Option<i64>, view: impl Fn(&T) -> Value) -> String {
    let (limit, offset) = window(limit, offset);
    let items: Vec<String> = matched.iter().skip(offset).take(limit).map(|item| view(item).to_string()).collect();
    let mut head = json!({"total": matched.len()});
    if offset + limit < matched.len() {
        head["next_offset"] = json!(offset + limit);
    }
    let head = head.to_string();
    let head = head.strip_suffix('}').expect("an object");
    if items.is_empty() {
        format!("{head},\"items\":[]}}")
    } else {
        format!("{head},\"items\":[\n{}\n]}}", items.join(",\n"))
    }
}

/// The first `max` characters, with an ellipsis when cut.
pub fn clip(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end())
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_match_any_attribute_ignoring_case() {
        let w = words(Some("  Оферта ИВАН "));
        assert_eq!(w, vec!["оферта", "иван"]);
        assert!(hit(&w, &["Покупка – оферта", "Иван Петров"]));
        assert!(!hit(&w, &["Покупка – оферта"]));
        assert!(hit(&[], &[]));
    }

    #[test]
    fn ranking_puts_exact_and_prefix_names_first() {
        let mut names = vec!["Big offer", "Offer letter", "offer"];
        rank_by(&mut names, Some("Offer"), |n| (String::new(), n.to_string()));
        assert_eq!(names, vec!["offer", "Offer letter", "Big offer"]);
    }

    #[test]
    fn pages_report_the_total_and_the_next_offset() {
        let items: Vec<i64> = (0..5).collect();
        let first: Value = serde_json::from_str(&found(&items, Some(2), None, |i| json!(i))).unwrap();
        assert_eq!(first, json!({"total": 5, "next_offset": 2, "items": [0, 1]}));
        let last: Value = serde_json::from_str(&found(&items, Some(2), Some(4), |i| json!(i))).unwrap();
        assert_eq!(last, json!({"total": 5, "items": [4]}));
        let none: Value = serde_json::from_str(&found(&Vec::<i64>::new(), None, None, |i| json!(i))).unwrap();
        assert_eq!(none, json!({"total": 0, "items": []}));
        assert_eq!(window(Some(1000), Some(-3)), (MAX_LIMIT, 0));
        assert_eq!(clip("abcdef", 3), "abc…");
        assert_eq!(clip("abc", 3), "abc");
    }
}
