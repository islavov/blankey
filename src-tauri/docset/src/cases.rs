//! The test cases of a document config set: `tests/cases/*.yaml` say what pages must read as, the pages being
//! `tests/lines/<page>[.<model>].json` or lines written in the case itself.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use blankey_doctypes::{self as doctypes, Line, Page, Reading, Set, Status};
use serde::Deserialize;

fn english() -> String {
    "en".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    /// the case runs once per model: its pages are `lines/<page>.<model>.json`
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default = "english")]
    pub language: String,
    pub pages: Vec<PageSpec>,
    pub expect: Expect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSpec {
    /// a file under `tests/lines/`, without the model and `.json`
    #[serde(default)]
    pub lines: Option<String>,
    /// instead: lines of text, one under another
    #[serde(default)]
    pub text: Vec<String>,
    /// text swapped in every line
    #[serde(default)]
    pub replace: BTreeMap<String, String>,
    /// a value printed to the right of the line that starts with the key
    #[serde(default)]
    pub beside: BTreeMap<String, String>,
    #[serde(default)]
    pub add: Vec<Added>,
}

#[derive(Deserialize)]
pub struct Added {
    pub text: String,
    pub rect: [f64; 4],
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Expect {
    /// `none`: no document of the set
    Nothing(String),
    Reading(Box<Expected>),
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub doctype: String,
    #[serde(default)]
    pub sides: Option<Vec<String>>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub number: Option<String>,
    #[serde(default)]
    pub no_number: bool,
    /// key -> value, with the status the map is named for
    #[serde(default)]
    pub ok: BTreeMap<String, String>,
    #[serde(default)]
    pub unchecked: BTreeMap<String, String>,
    #[serde(default)]
    pub failed: BTreeMap<String, String>,
    /// key -> a part of its value
    #[serde(default)]
    pub contains: BTreeMap<String, String>,
    /// key -> its label
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// keys that must not be read
    #[serde(default)]
    pub absent: Vec<String>,
    /// no field at all
    #[serde(default)]
    pub empty: bool,
}

/// Pages read anew from the pictures, to run the cases on in place of the lines kept: by page name, as one model
/// read them.
pub struct Fresh {
    pub model: String,
    pub pages: HashMap<String, Page>,
}

/// The lines kept for a page: `tests/lines/<name>.<model>.json`, or `<name>.json` when no model is named.
pub fn kept_lines(dir: &Path, name: &str, model: Option<&str>) -> Result<Page, String> {
    let file = match model {
        Some(model) => format!("{name}.{model}.json"),
        None => format!("{name}.json"),
    };
    let path = dir.join("tests/lines").join(file);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str::<Page>(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn build(dir: &Path, spec: &PageSpec, model: Option<&str>, fresh: Option<&Fresh>) -> Result<Page, String> {
    let mut page = match (&spec.lines, fresh) {
        (Some(name), Some(fresh)) => {
            fresh.pages.get(name).cloned().ok_or(format!("no picture tests/images/{name}.* to read anew"))?
        }
        (Some(name), None) => kept_lines(dir, name, model)?,
        (None, _) => Page { width: 595.0, height: 842.0, lines: Vec::new(), turned: 0, skew: 0.0 },
    };
    for (at, text) in spec.text.iter().enumerate() {
        let y = 60.0 + 30.0 * at as f64;
        page.lines.push(Line { text: text.clone(), rect: [40.0, y, 400.0, y + 14.0], confidence: 0.99 });
    }
    for line in &mut page.lines {
        for (from, to) in &spec.replace {
            line.text = line.text.replace(from, to);
        }
    }
    for (start, value) in &spec.beside {
        let at =
            page.lines.iter().find(|l| l.text.starts_with(start)).ok_or(format!("no line starts with {start}"))?.rect;
        let width = 9.0 * value.chars().count() as f64;
        page.lines.push(Line {
            text: value.clone(),
            rect: [at[2] + 12.0, at[1], at[2] + 12.0 + width, at[3]],
            confidence: 0.95,
        });
    }
    page.lines.extend(spec.add.iter().map(|a| Line { text: a.text.clone(), rect: a.rect, confidence: 0.95 }));
    Ok(page)
}

fn compare(reading: &Reading, wanted: &Expected, masked: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut wrong = Vec::new();
    let mut same = |what: &str, got: &str, expected: &str| {
        if got != expected {
            wrong.push(format!("{what}: expected {expected}, read {}", masked(got)));
        }
    };
    same("doctype", &reading.doctype, &wanted.doctype);
    if let Some(sides) = &wanted.sides {
        same("sides", &reading.sides.join(", "), &sides.join(", "));
    }
    for (what, got, expected) in [
        ("kind", &reading.kind, &wanted.kind),
        ("family", &reading.family, &wanted.family),
        ("name", &reading.name, &wanted.name),
    ] {
        if let Some(expected) = expected {
            same(what, got, expected);
        }
    }
    if let Some(number) = &wanted.number {
        same("number", reading.number.as_deref().unwrap_or("(none)"), number);
    }
    if wanted.no_number && reading.number.is_some() {
        wrong.push("number: expected none".into());
    }
    let field = |key: &str| reading.fields.iter().find(|f| f.key == key);
    for (status, values) in
        [(Status::Ok, &wanted.ok), (Status::Unchecked, &wanted.unchecked), (Status::Failed, &wanted.failed)]
    {
        for (key, expected) in values {
            match field(key) {
                Some(f) if f.value == *expected && f.status == status => {}
                Some(f) => wrong.push(format!(
                    "{key}: expected {expected} ({status:?}), read {} ({:?})",
                    masked(&f.value),
                    f.status
                )),
                None => wrong.push(format!("{key}: expected {expected} ({status:?}), not read")),
            }
        }
    }
    for (key, part) in &wanted.contains {
        if !field(key).is_some_and(|f| f.value.contains(part.as_str())) {
            wrong.push(format!(
                "{key}: expected to contain {part}, read {}",
                field(key).map_or("nothing".into(), |f| masked(&f.value))
            ));
        }
    }
    for (key, label) in &wanted.labels {
        if field(key).map(|f| f.label.as_str()) != Some(label.as_str()) {
            wrong.push(format!("{key}: expected the label {label}, got {:?}", field(key).map(|f| &f.label)));
        }
    }
    for key in &wanted.absent {
        if let Some(f) = field(key) {
            wrong.push(format!("{key}: expected nothing, read {}", masked(&f.value)));
        }
    }
    if wanted.empty && !reading.fields.is_empty() {
        wrong.push(format!("expected no fields, read {}", reading.fields.len()));
    }
    wrong
}

/// One case as one model's lines read, or a thing about the set itself, with what is wrong with it.
#[derive(serde::Serialize)]
pub struct Outcome {
    pub title: String,
    pub wrong: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct Report {
    pub documents: usize,
    pub cases: usize,
    pub failures: usize,
    pub outcomes: Vec<Outcome>,
}

/// The case files of the set, by file name, in name order: (file, its cases or why it does not parse).
pub fn load(dir: &Path) -> Vec<(String, Result<Vec<Case>, String>)> {
    let mut files: Vec<_> =
        std::fs::read_dir(dir.join("tests/cases")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    files.sort();
    let yaml = files.iter().filter(|p| p.extension().is_some_and(|e| e == "yaml"));
    yaml.map(|path| {
        let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let parsed = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_yaml::from_str::<Vec<Case>>(&t).map_err(|e| e.to_string()));
        (file, parsed)
    })
    .collect()
}

/// Runs every case of the set. It fails when the set leaves something out, when a document has no case, and when a
/// case reads as anything but what it expects, picked among all the documents of the set. With `fresh`, the cases
/// run on those pages and only for their model; documents left without a case are then not counted.
pub fn run(dir: &Path, set: &Set, masked: &dyn Fn(&str) -> String, fresh: Option<&Fresh>) -> Report {
    let mut outcomes: Vec<Outcome> = Vec::new();
    for (name, why) in &set.skipped {
        outcomes.push(Outcome { title: format!("{name} is left out"), wrong: vec![why.clone()] });
    }
    let mut tested: Vec<String> = Vec::new();
    let mut cases = 0;
    for (file, parsed) in load(dir) {
        let list = match parsed {
            Ok(list) => list,
            Err(e) => {
                outcomes.push(Outcome { title: file, wrong: vec![e] });
                continue;
            }
        };
        for case in list {
            let models: Vec<Option<&str>> = match fresh {
                Some(fresh) if case.models.is_empty() || case.models.contains(&fresh.model) => vec![Some(&fresh.model)],
                Some(_) => Vec::new(),
                None if case.models.is_empty() => vec![None],
                None => case.models.iter().map(|m| Some(m.as_str())).collect(),
            };
            for model in models {
                cases += 1;
                let title = format!("{file}: {}{}", case.name, model.map(|m| format!(" [{m}]")).unwrap_or_default());
                let pages: Result<Vec<Page>, String> =
                    case.pages.iter().map(|spec| build(dir, spec, model, fresh)).collect();
                let pages = match pages {
                    Ok(pages) => pages,
                    Err(e) => {
                        outcomes.push(Outcome { title, wrong: vec![e] });
                        continue;
                    }
                };
                let reading = doctypes::read(set, &pages, &case.language);
                let wrong = match (&case.expect, &reading) {
                    (Expect::Nothing(word), None) if word == "none" => Vec::new(),
                    (Expect::Nothing(word), None) => vec![format!("expect: {word}?")],
                    (Expect::Nothing(_), Some(reading)) => {
                        vec![format!("expected no document, read {}", reading.doctype)]
                    }
                    (Expect::Reading(wanted), None) => vec![format!("expected {}, not recognized", wanted.doctype)],
                    (Expect::Reading(wanted), Some(reading)) => {
                        tested.push(wanted.doctype.clone());
                        compare(reading, wanted, masked)
                    }
                };
                outcomes.push(Outcome { title, wrong });
            }
        }
    }
    if fresh.is_none() {
        for document in set.documents.iter().filter(|d| !tested.contains(&d.id)) {
            outcomes.push(Outcome { title: format!("{} has no test case", document.id), wrong: vec![String::new()] });
        }
    }
    let failures = outcomes.iter().filter(|o| !o.wrong.is_empty()).count();
    Report { documents: set.documents.len(), cases, failures, outcomes }
}
