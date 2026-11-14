//! Checks a document config set against its test cases, with this app's engine: what the relay's set is run through
//! before it is deployed.
//!
//!     cargo run --example check_doc_configs -- [--show] <set folder>
//!
//! The folder holds `set.yaml`, `tables/`, `documents/` and `tests/`: `tests/lines/<page>[.<model>].json` are the
//! text lines of a page, `tests/cases/*.yaml` what pages must read as. It fails when the set does not parse whole,
//! when a document has no case, and when a case reads as anything but what it expects, picked among all the
//! documents of the set. Capitals and digits of what was read print as dots unless `--show` is given.

use std::collections::BTreeMap;
use std::path::Path;

use blankey_lib::doctypes::{self, Line, Page, Reading, Set, Status};
use serde::Deserialize;

fn english() -> String {
    "en".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    /// the case runs once per model: its pages are `lines/<page>.<model>.json`
    #[serde(default)]
    models: Vec<String>,
    #[serde(default = "english")]
    language: String,
    pages: Vec<PageSpec>,
    expect: Expect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageSpec {
    /// a file under `tests/lines/`, without the model and `.json`
    #[serde(default)]
    lines: Option<String>,
    /// instead: lines of text, one under another
    #[serde(default)]
    text: Vec<String>,
    /// text swapped in every line
    #[serde(default)]
    replace: BTreeMap<String, String>,
    /// a value printed to the right of the line that starts with the key
    #[serde(default)]
    beside: BTreeMap<String, String>,
    #[serde(default)]
    add: Vec<Added>,
}

#[derive(Deserialize)]
struct Added {
    text: String,
    rect: [f64; 4],
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Expect {
    /// `none`: no document of the set
    Nothing(String),
    Reading(Box<Expected>),
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Expected {
    doctype: String,
    #[serde(default)]
    sides: Option<Vec<String>>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    number: Option<String>,
    #[serde(default)]
    no_number: bool,
    /// key -> value, with the status the map is named for
    #[serde(default)]
    ok: BTreeMap<String, String>,
    #[serde(default)]
    unchecked: BTreeMap<String, String>,
    #[serde(default)]
    failed: BTreeMap<String, String>,
    /// key -> a part of its value
    #[serde(default)]
    contains: BTreeMap<String, String>,
    /// key -> its label
    #[serde(default)]
    labels: BTreeMap<String, String>,
    /// keys that must not be read
    #[serde(default)]
    absent: Vec<String>,
    /// no field at all
    #[serde(default)]
    empty: bool,
}

fn build(dir: &Path, spec: &PageSpec, model: Option<&str>) -> Result<Page, String> {
    let mut page = match &spec.lines {
        Some(name) => {
            let file = match model {
                Some(model) => format!("{name}.{model}.json"),
                None => format!("{name}.json"),
            };
            let path = dir.join("tests/lines").join(file);
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            serde_json::from_str::<Page>(&text).map_err(|e| format!("{}: {e}", path.display()))?
        }
        None => Page { width: 595.0, height: 842.0, lines: Vec::new(), turned: 0, skew: 0.0 },
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show = args.iter().any(|a| a == "--show");
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")).map(Path::new) else {
        eprintln!("usage: check_doc_configs [--show] <set folder>");
        std::process::exit(2);
    };
    let masked = move |text: &str| -> String {
        if show {
            return text.to_string();
        }
        text.chars().map(|c| if c.is_uppercase() || c.is_ascii_digit() { '•' } else { c }).collect()
    };
    let mut failures = 0;
    let mut fail = |message: String| {
        println!("FAIL  {message}");
        failures += 1;
    };
    let set = match Set::from_dir(dir) {
        Ok(set) => set,
        Err(e) => {
            println!("FAIL  {e}");
            std::process::exit(1);
        }
    };
    for (name, why) in &set.skipped {
        fail(format!("{name} is left out: {why}"));
    }
    let mut tested: Vec<String> = Vec::new();
    let mut cases = 0;
    let mut files: Vec<_> =
        std::fs::read_dir(dir.join("tests/cases")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    files.sort();
    for path in files.iter().filter(|p| p.extension().is_some_and(|e| e == "yaml")) {
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        let parsed = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_yaml::from_str::<Vec<Case>>(&t).map_err(|e| e.to_string()));
        let list = match parsed {
            Ok(list) => list,
            Err(e) => {
                fail(format!("{file}: {e}"));
                continue;
            }
        };
        for case in list {
            let models: Vec<Option<&str>> = if case.models.is_empty() {
                vec![None]
            } else {
                case.models.iter().map(|m| Some(m.as_str())).collect()
            };
            for model in models {
                cases += 1;
                let title = format!("{file}: {}{}", case.name, model.map(|m| format!(" [{m}]")).unwrap_or_default());
                let pages: Result<Vec<Page>, String> = case.pages.iter().map(|spec| build(dir, spec, model)).collect();
                let pages = match pages {
                    Ok(pages) => pages,
                    Err(e) => {
                        fail(format!("{title}: {e}"));
                        continue;
                    }
                };
                let reading = doctypes::read(&set, &pages, &case.language);
                let wrong = match (&case.expect, &reading) {
                    (Expect::Nothing(word), None) if word == "none" => Vec::new(),
                    (Expect::Nothing(word), None) => vec![format!("expect: {word}?")],
                    (Expect::Nothing(_), Some(reading)) => {
                        vec![format!("expected no document, read {}", reading.doctype)]
                    }
                    (Expect::Reading(wanted), None) => vec![format!("expected {}, not recognized", wanted.doctype)],
                    (Expect::Reading(wanted), Some(reading)) => {
                        tested.push(wanted.doctype.clone());
                        compare(reading, wanted, &masked)
                    }
                };
                if wrong.is_empty() {
                    println!("ok    {title}");
                } else {
                    fail(format!("{title}\n        {}", wrong.join("\n        ")));
                }
            }
        }
    }
    for document in set.documents.iter().filter(|d| !tested.contains(&d.id)) {
        fail(format!("{} has no test case", document.id));
    }
    println!("\n{} documents, {cases} cases, {failures} failed", set.documents.len());
    if failures > 0 {
        std::process::exit(1);
    }
}
