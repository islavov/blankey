//! A document config set: which known documents there are, how a page is told to be one, where its fields are, how
//! their values are shaped and how the OCR pipeline runs. The engine holds none of this: a set is data it is given
//! (`Set::parse`), as the text of its files by path: `set.yaml`, `tables/*.yaml` for codes several documents share,
//! and one file per document under `documents/`.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::ocr::OcrSettings;
use super::patterns::Patterns;

/// What this engine can run. A set, or a document in one, that names a higher number needs a newer app.
pub const ENGINE: u32 = 1;

static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]*$").unwrap());
static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$").unwrap());
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([^{}]*)\}").unwrap());

/// A text in the languages of the UI.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Text {
    pub en: String,
    pub bg: String,
}

impl Text {
    pub fn get(&self, language: &str) -> &str {
        if language == "bg" {
            &self.bg
        } else {
            &self.en
        }
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Script {
    Cyrillic,
    Latin,
}

fn text_type() -> String {
    "text".into()
}

fn first() -> usize {
    1
}

/// One profile field and where the document has it. The locators, tried in this order: `mrz` (a part of the
/// machine-readable zone), `code` (the value beside a printed code), `label_of` (the value after or beside a
/// printed label, or under it with `below`), `pattern` alone (anywhere on the page).
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub key: String,
    pub label: Text,
    #[serde(rename = "type", default = "text_type")]
    pub field_type: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub label_of: Vec<String>,
    #[serde(default)]
    pub mrz: Option<String>,
    /// a named pattern the value must match; with `label_of` it is looked for near the label
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub script: Option<Script>,
    /// which line of a code's value: 2 is the line printed under it
    #[serde(default = "first")]
    pub line: usize,
    /// which side of a bilingual "БЪЛГАРИЯ/BGR" value; 0 keeps it whole
    #[serde(default)]
    pub part: usize,
    /// the value goes on under the label, until the next label
    #[serde(default)]
    pub below: bool,
    /// a code whose value is a list of lettered items, "(a) ...": the items beside it are its value
    #[serde(default)]
    pub items: bool,
}

/// What a page must show to be a side of the document. Every condition given must hold.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Match {
    #[serde(default)]
    pub text_all: Vec<String>,
    #[serde(default)]
    pub text_any: Vec<String>,
    #[serde(default)]
    pub codes_all: Vec<String>,
    #[serde(default)]
    pub codes_any: Vec<String>,
    #[serde(default)]
    pub codes_none: Vec<String>,
    /// the state its machine-readable zone names: "BGR"
    #[serde(default)]
    pub mrz: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Side {
    pub id: String,
    /// false for a side that is recognized and holds no fields
    #[serde(default = "yes")]
    pub read: bool,
    #[serde(rename = "match")]
    pub matches: Match,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ProfileSpec {
    pub kind: String,
    /// the profile's name, with {key} for a field's value
    pub name: String,
    /// the field that identifies the physical thing, so its other files join the same profile
    pub number: String,
}

/// A check across fields; a field that fails it is not saved.
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// [egn, date]: the EGN starts with this date
    pub egn_birth: [String; 2],
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    id: String,
    /// the lowest engine that can run this document
    #[serde(default = "one")]
    engine: u32,
    name: Text,
    family: String,
    #[serde(default)]
    priority: i32,
    profile: ProfileSpec,
    #[serde(default)]
    uses: Option<String>,
    sides: Vec<Side>,
    #[serde(default)]
    codes: Vec<String>,
    #[serde(default)]
    fields: Vec<Field>,
    #[serde(default)]
    checks: Vec<Check>,
    #[serde(default)]
    print: Vec<String>,
}

/// The codes several documents print, each with the field it is: a file under `tables/`.
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub id: String,
    /// what documents of the table print besides their values
    #[serde(default)]
    pub print: Vec<String>,
    pub codes: Vec<Field>,
}

#[derive(Deserialize)]
struct SetFile {
    engine: u32,
    ocr: OcrSettings,
    #[serde(default)]
    patterns: BTreeMap<String, serde_yaml::Value>,
}

pub struct Config {
    pub id: String,
    pub name: Text,
    /// documents of one family with the same number describe one thing and share a profile
    pub family: String,
    /// among the documents a file could be, the higher one is taken
    pub priority: i32,
    pub profile: ProfileSpec,
    pub sides: Vec<Side>,
    /// in profile order: the coded fields, then the rest
    pub fields: Vec<Field>,
    pub checks: Vec<Check>,
    /// every code the document's table knows, canonical: codes it does not carry still mark where a value ends
    pub known_codes: Vec<String>,
    /// the codes, canonical, whose value is a list of lettered items
    pub item_codes: Vec<String>,
    /// what the document prints besides its values: titles and captions, never taken for a value
    pub print: Vec<String>,
}

/// The code without dots, as it is compared: "C.1.1" is "C11".
pub fn canonical(code: &str) -> String {
    code.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_uppercase()
}

/// One document of a set, or (its id, why this engine cannot run it).
fn compile(source: &str, tables: &[Table], patterns: &Patterns) -> Result<Config, (String, String)> {
    let file: ConfigFile = serde_yaml::from_str(source).map_err(|e| (String::new(), e.to_string()))?;
    let refuse = |reason: String| (file.id.clone(), reason);
    if !ID.is_match(&file.id) {
        return Err(refuse("its id is not a lowercase name".into()));
    }
    if file.engine > ENGINE {
        return Err(refuse(format!("it needs engine {}", file.engine)));
    }
    let table = match &file.uses {
        Some(id) => Some(tables.iter().find(|t| &t.id == id).ok_or_else(|| refuse(format!("no table {id}")))?),
        None => None,
    };
    let mut fields = Vec::new();
    for code in &file.codes {
        let found: Vec<&Field> =
            table.iter().flat_map(|t| &t.codes).filter(|f| f.code.as_ref() == Some(code)).collect();
        if found.is_empty() {
            return Err(refuse(format!("code {code} is not in its table")));
        }
        fields.extend(found.into_iter().cloned());
    }
    fields.extend(file.fields.iter().cloned());
    for (at, field) in fields.iter().enumerate() {
        let problem = if !KEY.is_match(&field.key) {
            Some("is not a field key")
        } else if fields[..at].iter().any(|f| f.key == field.key) {
            Some("is there twice")
        } else if !crate::values::is_type(&field.field_type) {
            Some("has an unknown type")
        } else if field.pattern.as_ref().is_some_and(|pattern| !patterns.has(pattern)) {
            Some("names a pattern this engine does not have")
        } else if field.line == 0 {
            Some("counts lines from 1")
        } else {
            None
        };
        if let Some(problem) = problem {
            return Err(refuse(format!("{} {problem}", field.key)));
        }
    }
    let is_field = |key: &str| fields.iter().any(|f| f.key == key);
    if !is_field(&file.profile.number) {
        return Err(refuse(format!("its number {} is not a field", file.profile.number)));
    }
    if let Some(unknown) = PLACEHOLDER.captures_iter(&file.profile.name).find(|c| !is_field(&c[1])) {
        return Err(refuse(format!("its name uses {}, which is not a field", &unknown[1])));
    }
    if let Some(unknown) = file.checks.iter().flat_map(|c| &c.egn_birth).find(|key| !is_field(key)) {
        return Err(refuse(format!("a check uses {unknown}, which is not a field")));
    }
    if file.sides.is_empty() || !ID.is_match(&file.profile.kind) || !ID.is_match(&file.family) {
        return Err(refuse("it needs a side, a profile kind and a family".into()));
    }
    let table_codes = || table.iter().flat_map(|t| &t.codes);
    let known_codes = table_codes().filter_map(|f| f.code.as_deref()).map(canonical).collect();
    let item_codes = table_codes().filter(|f| f.items).filter_map(|f| f.code.as_deref()).map(canonical).collect();
    let print = table.iter().flat_map(|t| t.print.iter().cloned()).chain(file.print.iter().cloned()).collect();
    Ok(Config {
        id: file.id,
        name: file.name,
        family: file.family,
        priority: file.priority,
        profile: file.profile,
        sides: file.sides,
        fields,
        checks: file.checks,
        known_codes,
        item_codes,
        print,
    })
}

pub struct Set {
    /// what the relay names this set by; a set with other files has another
    pub version: String,
    pub ocr: OcrSettings,
    pub patterns: Patterns,
    /// in the order of their paths
    pub tables: Vec<Table>,
    /// in the order of their paths
    pub documents: Vec<Config>,
    /// what this engine left out: (a document's id, a pattern's name or a file's path, why)
    pub skipped: Vec<(String, String)>,
}

impl Set {
    /// The set in `files` (path -> YAML). It is refused whole when `set.yaml` is missing, does not parse or needs a
    /// newer engine. A document, table or pattern that this engine cannot run is left out and listed in `skipped`,
    /// and the rest works: a set may hold documents written for a newer app.
    pub fn parse(version: &str, files: &BTreeMap<String, String>) -> Result<Set, String> {
        let source = files.get("set.yaml").ok_or("The document config set has no set.yaml")?;
        let file: SetFile = serde_yaml::from_str(source).map_err(|e| format!("set.yaml: {e}"))?;
        if file.engine > ENGINE {
            return Err(format!("The document config set needs engine {}; this is {ENGINE}", file.engine));
        }
        let (patterns, mut skipped) = Patterns::compile(file.patterns);
        let mut tables: Vec<Table> = Vec::new();
        for (path, source) in files.iter().filter(|(path, _)| path.starts_with("tables/")) {
            match serde_yaml::from_str::<Table>(source) {
                Ok(table) => tables.push(table),
                Err(e) => skipped.push((path.clone(), e.to_string())),
            }
        }
        let mut documents: Vec<Config> = Vec::new();
        for (path, source) in files.iter().filter(|(path, _)| path.starts_with("documents/")) {
            match compile(source, &tables, &patterns) {
                Ok(config) if documents.iter().any(|d| d.id == config.id) => {
                    skipped.push((config.id, "its id is there twice".into()));
                }
                Ok(config) => documents.push(config),
                Err((id, reason)) => skipped.push((if id.is_empty() { path.clone() } else { id }, reason)),
            }
        }
        Ok(Set { version: version.to_string(), ocr: file.ocr, patterns, tables, documents, skipped })
    }

    /// The set in a folder laid out as the relay keeps it (`set.yaml`, `tables/`, `documents/`): for the checker
    /// and the diagnostic, which read a set from a checkout.
    pub fn from_dir(dir: &std::path::Path) -> Result<Set, String> {
        fn collect(root: &std::path::Path, dir: &std::path::Path, files: &mut BTreeMap<String, String>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let name = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                if path.is_dir() {
                    if name != "tests" {
                        collect(root, &path, files);
                    }
                } else if name.ends_with(".yaml") {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        files.insert(name, text);
                    }
                }
            }
        }
        let mut files = BTreeMap::new();
        collect(dir, dir, &mut files);
        Set::parse("local", &files)
    }
}
