//! The user's corrections for one file: values to hide that detection missed, and detected values that are not
//! personal. Applied before entities form, so redaction, placeholders and the review all agree.

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::t2::{ends_word, key};
use super::types::{Detection, Kind, Source};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Corrections {
    #[serde(default)]
    pub hide: Vec<String>,
    #[serde(default)]
    pub show: Vec<String>,
}

fn same(a: &str, b: &str) -> bool {
    key(a) == key(b)
}

impl Corrections {
    pub fn is_empty(&self) -> bool {
        self.hide.is_empty() && self.show.is_empty()
    }

    /// Drops the hits the user showed and adds every occurrence of what the user hid, in any letter case.
    pub fn apply(&self, text: &str, hits: Vec<Detection>) -> Vec<Detection> {
        let mut hits: Vec<Detection> =
            hits.into_iter().filter(|d| !self.show.iter().any(|s| same(s, &text[d.start..d.end]))).collect();
        for value in &self.hide {
            let pattern = regex::escape(value.trim()).replace(' ', r"\s+");
            let Ok(re) = Regex::new(&format!("(?i){pattern}")) else { continue };
            for m in re.find_iter(text) {
                let (start, end) = (m.start(), m.end());
                if (start > 0 && !ends_word(text, start)) || !ends_word(text, end) {
                    continue;
                }
                hits.retain(|d| d.end <= start || end <= d.start);
                hits.push(Detection { kind: Kind::Marked, start, end, score: 1.0, source: Source::User });
            }
        }
        hits.sort_by_key(|d| d.start);
        hits
    }

    /// Hides a value. A detected value only needs to stop being shown; `needed` adds it to the user's own.
    pub fn hide(&mut self, value: &str, needed: bool) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        self.show.retain(|s| !same(s, value));
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
    }
}
