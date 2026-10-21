//! A mark says where a template var goes. The preview UI and suggestions speak marks; `Document::compile` turns
//! them into what the template save paths take.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::templates::StrMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum At {
    /// Every occurrence of a detected entity ("egn:1").
    Entity(String),
    /// A selection in one block, in UTF-16 code units as a browser selection gives them.
    Span { block: usize, start: usize, end: usize },
    /// A box drawn on a PDF page: [x0, y0, x1, y1] in points, origin bottom-left.
    Rect { page: usize, rect: [f64; 4] },
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mark {
    pub at: At,
    pub var: String,
    /// "seller.egn"; none when the value is typed at fill time
    #[serde(default)]
    pub binding: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// A span also marks every other occurrence of the same text.
    #[serde(default = "yes")]
    pub all: bool,
}

/// What the template save paths take: tokenize replacements for a docx, overlay boxes for a PDF.
#[derive(Debug, Default)]
pub struct Draft {
    pub replacements: Vec<Value>,
    pub boxes: Vec<Value>,
    pub fields: StrMap,
    pub labels: StrMap,
    pub roles: StrMap,
}

/// "party_1" -> "Party 1", "seller" -> "Seller"
pub fn role_title(role: &str) -> String {
    let words = role.replace('_', " ");
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

impl Draft {
    pub fn add_meta(&mut self, var: &str, mark: &Mark) {
        if let Some(binding) = &mark.binding {
            self.fields.entry(var.to_string()).or_insert_with(|| format!("{{{{ {binding} }}}}"));
            if let Some((role, _)) = binding.split_once('.') {
                self.roles.entry(role.to_string()).or_insert_with(|| role_title(role));
            }
        }
        if let Some(label) = &mark.label {
            self.labels.entry(var.to_string()).or_insert_with(|| label.clone());
        }
    }
}
