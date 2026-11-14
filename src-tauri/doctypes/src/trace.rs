//! What the engine saw on the pages and tried, beside what it read: for whoever writes a document config set, to
//! see why a page was or was not taken for a document and why a field was not found. The app never asks for it.

use std::collections::BTreeMap;

use serde::Serialize;

use super::Reading;

/// One thing a side's `match` asks of a page. For `codes_none` it held when the code is not printed.
#[derive(Serialize, Clone, Debug)]
pub struct Condition {
    /// the key of `match` it comes from: "text_all", "codes_any", "mrz"
    pub kind: &'static str,
    pub wanted: String,
    pub held: bool,
    /// what the page has for it: the line that says the phrase, the state its zone names
    pub seen: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SideSurvey {
    pub page: usize,
    pub side: String,
    pub holds: bool,
    pub conditions: Vec<Condition>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DocumentSurvey {
    pub id: String,
    pub priority: i32,
    /// every side against every page
    pub sides: Vec<SideSurvey>,
    /// what it is ranked by when a page is a side of it: (pages, priority, conditions asked)
    pub score: Option<(usize, i32, usize)>,
    pub picked: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct PageSurvey {
    /// the state its machine-readable zone names
    pub mrz: Option<String>,
    /// per table of the set: its codes printed on the page
    pub codes: BTreeMap<String, Vec<String>>,
}

/// How the document of a file was picked among all of the set.
#[derive(Serialize, Clone, Debug)]
pub struct Survey {
    pub pages: Vec<PageSurvey>,
    pub documents: Vec<DocumentSurvey>,
}

#[derive(Serialize, Clone, Debug)]
pub struct LabelTrace {
    pub field: String,
    /// chars `start..end` of the line's text
    pub start: usize,
    pub end: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct LineTrace {
    pub index: usize,
    pub text: String,
    pub rect: [f64; 4],
    pub confidence: f32,
    /// what the line was taken for: "value", "code", "bracket", "item", "print", "zone"
    pub role: &'static str,
    pub code: Option<String>,
    pub labels: Vec<LabelTrace>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ZoneTrace {
    pub name: String,
    pub value: String,
    pub checked: Option<bool>,
}

/// The machine-readable zone of a page.
#[derive(Serialize, Clone, Debug)]
pub struct ZoneFound {
    /// the state it names: "BGR"
    pub state: String,
    /// the lines of the page it takes
    pub lines: Vec<usize>,
    pub parts: Vec<ZoneTrace>,
}

/// A page as the picked document's config divides it.
#[derive(Serialize, Clone, Debug)]
pub struct ScanTrace {
    pub lines: Vec<LineTrace>,
    /// per printed code: the lines of its value
    pub codes: BTreeMap<String, Vec<String>>,
    pub mrz: Vec<ZoneTrace>,
}

/// A piece of text a locator tried as the field's value.
#[derive(Serialize, Clone, Debug)]
pub struct Candidate {
    pub text: String,
    pub rect: Option<[f64; 4]>,
    /// the text shaped as the field's type or pattern
    pub value: String,
    pub checked: Option<bool>,
    /// why it was not taken, when it was not
    pub note: String,
}

/// One locator's try at a field on a page.
#[derive(Serialize, Clone, Debug)]
pub struct Attempt {
    /// "mrz", "code", "label", "pattern"
    pub locator: &'static str,
    /// the line it started from: the label, the code, the zone's line
    pub anchor: Option<usize>,
    pub candidates: Vec<Candidate>,
    /// why there was nothing to try
    pub note: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct PageTry {
    pub page: usize,
    pub side: String,
    pub attempts: Vec<Attempt>,
}

#[derive(Serialize, Clone, Debug)]
pub struct FieldTrace {
    pub key: String,
    /// "ok", "unchecked", "failed", or "not_found"
    pub status: &'static str,
    pub value: Option<String>,
    pub tries: Vec<PageTry>,
}

/// A reading with how it came about.
#[derive(Serialize, Clone, Debug)]
pub struct Explained {
    pub survey: Survey,
    /// the document the fields were read as: the one picked, or the one asked for
    pub document: Option<String>,
    /// the document was asked for, not picked
    pub forced: bool,
    pub reading: Option<Reading>,
    pub scans: Vec<ScanTrace>,
    pub fields: Vec<FieldTrace>,
}

#[derive(Serialize, Clone, Debug)]
pub struct StepTrace {
    /// "fold", "squeeze", "replace"
    pub step: &'static str,
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CheckTrace {
    pub check: String,
    pub held: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct PatternCandidate {
    pub value: String,
    pub checks: Vec<CheckTrace>,
}

/// One rule of a pattern run over a text, step by step.
#[derive(Serialize, Clone, Debug)]
pub struct RuleTrace {
    pub steps: Vec<StepTrace>,
    pub candidates: Vec<PatternCandidate>,
    pub value: Option<String>,
}

/// Collects the attempts at a field, or nothing at all when nobody asked.
pub struct Tracer(Option<Vec<Attempt>>);

impl Tracer {
    pub fn off() -> Tracer {
        Tracer(None)
    }

    pub fn on() -> Tracer {
        Tracer(Some(Vec::new()))
    }

    pub fn attempt(&mut self, make: impl FnOnce() -> Attempt) {
        if let Some(attempts) = &mut self.0 {
            attempts.push(make());
        }
    }

    /// A candidate of the attempt under way.
    pub fn candidate(&mut self, make: impl FnOnce() -> Candidate) {
        if let Some(attempt) = self.0.as_mut().and_then(|attempts| attempts.last_mut()) {
            attempt.candidates.push(make());
        }
    }

    /// The line the attempt under way started from; the first one stays.
    pub fn anchor(&mut self, line: usize) {
        if let Some(attempt) = self.0.as_mut().and_then(|attempts| attempts.last_mut()) {
            attempt.anchor.get_or_insert(line);
        }
    }

    pub fn note(&mut self, note: &str) {
        if let Some(attempt) = self.0.as_mut().and_then(|attempts| attempts.last_mut()) {
            attempt.note = note.to_string();
        }
    }

    pub fn done(self) -> Vec<Attempt> {
        self.0.unwrap_or_default()
    }
}
