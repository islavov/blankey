//! The user's corrections for one file: values to hide that detection missed, and detected values that are not
//! personal. Applied before entities form, so redaction, placeholders and the review all agree.

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::resolve::normalize;
use super::t2::ends_word;
use super::types::{Detection, Kind, Source};

const QUOTES: &str = "„“”\"«»";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Corrections {
    #[serde(default)]
    pub hide: Vec<String>,
    #[serde(default)]
    pub show: Vec<String>,
    /// Detected values the user dismissed as not personal: visible to the assistant and not listed in the review.
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// Two spellings of one value: case, quotes and spacing aside ("„ПРИМЕР“ ЕООД" and "Пример ЕООД").
fn same(a: &str, b: &str) -> bool {
    normalize(Kind::Marked, a) == normalize(Kind::Marked, b)
}

/// Finds a value in any letter case and with or without quotes around its words, so hiding "ПРИМЕР“ ЕООД" (a
/// selection that left out the opening quote) also hides „ПРИМЕР“ ЕООД and ПРИМЕР ЕООД.
pub fn value_pattern(value: &str) -> Option<Regex> {
    let words: Vec<String> = value
        .split(|c: char| c.is_whitespace() || QUOTES.contains(c))
        .filter(|w| !w.is_empty())
        .map(regex::escape)
        .collect();
    if words.is_empty() {
        return None;
    }
    let q = format!("[{}]*", regex::escape(QUOTES));
    let gap = format!("(?:{q}\\s+{q}|[{}]+)", regex::escape(QUOTES));
    Regex::new(&format!("(?i){q}{}{q}", words.join(&gap))).ok()
}

/// Where a value occurs as whole words in `text`, by `value_pattern`.
pub fn find_value(text: &str, value: &str) -> Vec<(usize, usize)> {
    let Some(re) = value_pattern(value) else { return Vec::new() };
    re.find_iter(text)
        .map(|m| (m.start(), m.end()))
        .filter(|&(s, e)| (s == 0 || ends_word(text, s)) && ends_word(text, e))
        .collect()
}

impl Corrections {
    pub fn is_empty(&self) -> bool {
        self.hide.is_empty() && self.show.is_empty() && self.ignore.is_empty()
    }

    /// Drops the hits the user showed and adds every occurrence of what the user hid, in any letter case and with or
    /// without quotes.
    pub fn apply(&self, text: &str, hits: Vec<Detection>) -> Vec<Detection> {
        let mut hits: Vec<Detection> = hits.into_iter().filter(|d| !self.shown(&text[d.start..d.end])).collect();
        for value in &self.hide {
            for (start, end) in find_value(text, value) {
                hits.retain(|d| d.end <= start || end <= d.start);
                hits.push(Detection { kind: Kind::Marked, start, end, score: 1.0, source: Source::User });
            }
        }
        hits.sort_by_key(|d| d.start);
        hits
    }

    fn shown(&self, value: &str) -> bool {
        self.show.iter().chain(&self.ignore).any(|s| same(s, value))
    }

    /// Hides a value. A detected value only needs to stop being shown; `needed` adds it to the user's own.
    pub fn hide(&mut self, value: &str, needed: bool) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        self.show.retain(|s| !same(s, value));
        self.ignore.retain(|s| !same(s, value));
        if needed && !self.hide.iter().any(|h| same(h, value)) {
            self.hide.push(value.to_string());
        }
    }

    /// Lets the assistant see a value, detected or added by the user; it stays listed so it can be hidden again.
    pub fn show(&mut self, value: &str) {
        self.hide.retain(|h| !same(h, value));
        if !self.show.iter().any(|s| same(s, value)) {
            self.show.push(value.trim().to_string());
        }
    }

    /// Drops a value the user added, hidden or shown.
    pub fn forget(&mut self, value: &str) {
        self.hide.retain(|h| !same(h, value));
        self.show.retain(|s| !same(s, value));
        self.ignore.retain(|s| !same(s, value));
    }

    /// Takes a value off the review list: one the user added is forgotten, a detected one dismissed.
    pub fn remove(&mut self, value: &str) {
        if self.hide.iter().any(|h| same(h, value)) {
            self.forget(value);
        } else {
            self.dismiss(value);
        }
    }

    /// A detected value is not personal: the assistant sees it and the review no longer lists it.
    pub fn dismiss(&mut self, value: &str) {
        self.forget(value);
        self.ignore.push(value.trim().to_string());
    }
}
