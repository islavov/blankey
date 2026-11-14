//! Writes and checks a document config set with the app's own engine: what the relay's set is run through while a
//! document is added to it and before it is deployed. A developer's tool; the app does not ship it.
//!
//!     docset [--set <folder>] [--model <name>] [--show] [--json] <command> ...
//!
//! The folder (default `doc-configs`) holds `set.yaml`, `tables/`, `documents/` and `tests/`. A page is named by a
//! picture (read now), a `tests/lines` name (`id_card_back`), or a lines file; several pages are the sides of one
//! file. Capitals and digits of what was read print as dots unless `--show` is given: pass it for public specimens.
//!
//!     ocr <picture>... [--write]      the page's lines; --write keeps them as tests/lines/<name>.<model>.json
//!     ocr --all                       reads every picture of tests/images anew into tests/lines
//!     read <page>...                  what the pages read as
//!     explain <page>...               why: every document's sides against the pages, every field's locators
//!             [--document <id>]       read as this document, picked or not
//!             [--field <key>]         only this field
//!     view <page>... [-o <file>]      the same on the picture, as one HTML file; --watch serves it and follows edits
//!     matrix                          every tests/lines page against every document
//!     pattern <name> <text>           a pattern step by step; `pattern --all <text>`: which patterns find a value
//!     scaffold <page>... --id <id>    a draft document file for the pages
//!     accept <page>... --name <text>  a test case that expects what the pages read as now
//!     check                           every test case; fails on a document left out or untested
//!     sweep <picture> --vary k=a,b    reads under other `ocr:` settings; `sweep --all`: the cases under each

mod cases;
mod scaffold;
mod view;

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use blankey_doctypes::ocr::{self, OcrSettings};
use blankey_doctypes::trace::{Explained, RuleTrace, Survey};
use blankey_doctypes::{self as doctypes, Page, Reading, Set};

const VALUED: &[&str] = &["--set", "--model", "-o", "--id", "--name", "--field", "--document", "--vary", "--port"];
const FLAGS: &[&str] = &["--show", "--json", "--write", "--all", "--watch"];
const PICTURES: &[&str] = &["png", "jpg", "jpeg", "webp"];
/// A sweep reads the pictures once per combination: more than this many is a mistake.
const MAX_COMBINATIONS: usize = 24;

/// A page to work on, with where it came from.
pub struct Sheet {
    /// its name under `tests/lines` and `tests/images`
    pub name: String,
    pub page: Page,
    /// the picture the lines were read from, when there is one
    pub picture: Option<PathBuf>,
}

pub struct Tool {
    pub dir: PathBuf,
    pub model: String,
    pub show: bool,
    pub json: bool,
    flags: HashSet<String>,
    options: HashMap<String, Vec<String>>,
    rest: Vec<String>,
    engine: OnceCell<Result<ocr::Engine, String>>,
}

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn is_picture(path: &Path) -> bool {
    PICTURES.contains(&extension(path).as_str())
}

/// The file's name without its folder, its extension and a model before the extension.
fn page_name(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if extension(path) == "json" {
        stem.rsplit_once('.').map_or(stem.clone(), |(name, _)| name.to_string())
    } else {
        stem
    }
}

impl Tool {
    fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    fn option(&self, name: &str) -> Option<&str> {
        self.options.get(name).and_then(|values| values.last()).map(String::as_str)
    }

    /// Capitals and digits as dots, unless the values may show.
    pub fn masked(&self, text: &str) -> String {
        if self.show {
            return text.to_string();
        }
        text.chars().map(|c| if c.is_uppercase() || c.is_ascii_digit() { '•' } else { c }).collect()
    }

    pub fn set(&self) -> Result<Set, String> {
        Set::from_dir(&self.dir)
    }

    /// The OCR engine with the model asked for, loaded on first use.
    fn engine(&self) -> Result<&ocr::Engine, String> {
        let loaded = self.engine.get_or_init(|| {
            let dir = ocr::model_dir().ok_or("The OCR models are missing: run scripts/fetch-ocr.sh in the app")?;
            let file = format!("{}_PP-OCRv5_rec_mobile.onnx", self.model);
            if !dir.join(&file).is_file() {
                return Err(format!("No recognition model {file} in {}", dir.display()));
            }
            ocr::Engine::load_model(&dir, &file)
        });
        loaded.as_ref().map_err(String::clone)
    }

    pub fn ocr(&self, picture: &Path, settings: &OcrSettings) -> Result<Page, String> {
        let content = std::fs::read(picture).map_err(|e| format!("{}: {e}", picture.display()))?;
        self.engine()?.read(&content, settings)
    }

    /// The picture kept for a page of the set.
    pub fn picture_of(&self, name: &str) -> Option<PathBuf> {
        PICTURES.iter().map(|ext| self.dir.join("tests/images").join(format!("{name}.{ext}"))).find(|p| p.is_file())
    }

    /// The page an argument names: a picture, a lines file, or a name under `tests/lines`.
    pub fn sheet(&self, arg: &str, settings: &OcrSettings) -> Result<Sheet, String> {
        let path = Path::new(arg);
        if path.is_file() && is_picture(path) {
            return Ok(Sheet { name: page_name(path), page: self.ocr(path, settings)?, picture: Some(path.into()) });
        }
        if path.is_file() {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{arg}: {e}"))?;
            let page = serde_json::from_str::<Page>(&text).map_err(|e| format!("{arg}: {e}"))?;
            let name = page_name(path);
            return Ok(Sheet { picture: self.picture_of(&name), name, page });
        }
        let page = cases::kept_lines(&self.dir, arg, Some(&self.model))
            .or_else(|with_model| cases::kept_lines(&self.dir, arg, None).map_err(|_| with_model))
            .map_err(|e| format!("{arg} is no file, and no page of the set: {e}"))?;
        Ok(Sheet { name: arg.to_string(), page, picture: self.picture_of(arg) })
    }

    pub fn sheets(&self, settings: &OcrSettings) -> Result<Vec<Sheet>, String> {
        if self.rest.is_empty() {
            return Err("Name a page: a picture, a lines file, or a name under tests/lines".into());
        }
        self.rest.iter().map(|arg| self.sheet(arg, settings)).collect()
    }

    /// Every page kept under `tests/lines` as the model read it, by name.
    pub fn kept_pages(&self) -> Vec<(String, Page)> {
        let mut names: Vec<String> = std::fs::read_dir(self.dir.join("tests/lines"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| extension(path) == "json")
            .filter(|path| {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                stem.rsplit_once('.').is_none_or(|(_, model)| model == self.model)
            })
            .map(|path| page_name(&path))
            .collect();
        names.sort();
        names.dedup();
        names
            .into_iter()
            .filter_map(|name| {
                let page = cases::kept_lines(&self.dir, &name, Some(&self.model))
                    .or_else(|_| cases::kept_lines(&self.dir, &name, None))
                    .ok()?;
                Some((name, page))
            })
            .collect()
    }

    /// Every picture kept under `tests/images`, in name order.
    fn kept_pictures(&self) -> Vec<PathBuf> {
        let mut pictures: Vec<PathBuf> = std::fs::read_dir(self.dir.join("tests/images"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_picture(path))
            .collect();
        pictures.sort();
        pictures
    }
}

/// A page's lines as a `tests/lines` file: a line of text per line of the file, so a change shows as one.
fn lines_json(page: &Page) -> String {
    let quoted = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let mut out = format!("{{\n\"width\": {}, \"height\": {}, ", page.width, page.height);
    if page.turned != 0 || page.skew != 0.0 {
        out += &format!("\"turned\": {}, \"skew\": {:.4}, ", page.turned, page.skew);
    }
    out += "\"lines\": [\n";
    let lines = page.lines.iter().map(|line| {
        let [x0, y0, x1, y1] = line.rect;
        format!(
            "{{\"text\": {}, \"rect\": [{x0:.1}, {y0:.1}, {x1:.1}, {y1:.1}], \"confidence\": {:.3}}}",
            quoted(&line.text),
            line.confidence
        )
    });
    out += &lines.collect::<Vec<_>>().join(",\n");
    out + "\n]}\n"
}

fn print_json<T: serde::Serialize>(value: &T) {
    println!("{}", serde_json::to_string_pretty(value).unwrap_or_default());
}

fn mask_reading(tool: &Tool, reading: &mut Reading) {
    reading.name = tool.masked(&reading.name);
    reading.number = reading.number.as_deref().map(|number| tool.masked(number));
    for field in &mut reading.fields {
        field.value = tool.masked(&field.value);
    }
}

fn mask_survey(tool: &Tool, survey: &mut Survey) {
    let conditions = survey.documents.iter_mut().flat_map(|d| &mut d.sides).flat_map(|s| &mut s.conditions);
    for condition in conditions.filter(|c| c.kind != "mrz") {
        condition.seen = condition.seen.as_deref().map(|seen| tool.masked(seen));
    }
}

/// Everything read from the pages as dots; what the set itself says (codes, labels, phrases) stays.
fn mask_explained(tool: &Tool, told: &mut Explained) {
    mask_survey(tool, &mut told.survey);
    if let Some(reading) = &mut told.reading {
        mask_reading(tool, reading);
    }
    for scan in &mut told.scans {
        for line in &mut scan.lines {
            line.text = tool.masked(&line.text);
        }
        for rows in scan.codes.values_mut() {
            rows.iter_mut().for_each(|row| *row = tool.masked(row));
        }
        scan.mrz.iter_mut().for_each(|part| part.value = tool.masked(&part.value));
    }
    for field in &mut told.fields {
        field.value = field.value.as_deref().map(|value| tool.masked(value));
        let candidates = field.tries.iter_mut().flat_map(|t| &mut t.attempts).flat_map(|a| &mut a.candidates);
        for candidate in candidates {
            candidate.text = tool.masked(&candidate.text);
            candidate.value = tool.masked(&candidate.value);
        }
    }
}

fn print_reading(tool: &Tool, reading: Option<&Reading>) {
    let Some(reading) = reading else {
        println!("not a known document");
        return;
    };
    println!(
        "{} {:?}: {} fields, number {}",
        reading.doctype,
        reading.sides,
        reading.fields.len(),
        if reading.number.is_some() { "read" } else { "not read" }
    );
    for field in &reading.fields {
        println!(
            "  {:<24} {:<10} {}",
            field.key,
            format!("{:?}", field.status).to_lowercase(),
            tool.masked(&field.value)
        );
    }
}

fn command_ocr(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    let all = tool.flag("--all");
    let pictures: Vec<PathBuf> = if all { tool.kept_pictures() } else { tool.rest.iter().map(PathBuf::from).collect() };
    if pictures.is_empty() {
        return Err(if all { "No pictures under tests/images".into() } else { "Name a picture".into() });
    }
    for picture in &pictures {
        let started = std::time::Instant::now();
        let page = tool.ocr(picture, &set.ocr)?;
        if !(all || tool.flag("--write")) {
            print!("{}", lines_json(&page));
            continue;
        }
        let folder = tool.dir.join("tests/lines");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let file = folder.join(format!("{}.{}.json", page_name(picture), tool.model));
        std::fs::write(&file, lines_json(&page)).map_err(|e| format!("{}: {e}", file.display()))?;
        println!("{}: {} lines in {:.1?}", file.display(), page.lines.len(), started.elapsed());
    }
    Ok(0)
}

fn command_read(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    let sheets = tool.sheets(&set.ocr)?;
    let pages: Vec<Page> = sheets.into_iter().map(|sheet| sheet.page).collect();
    let mut reading = doctypes::read(&set, &pages, "en");
    if tool.json {
        if let Some(reading) = &mut reading {
            mask_reading(tool, reading);
        }
        print_json(&reading);
    } else {
        print_reading(tool, reading.as_ref());
    }
    Ok(0)
}

fn print_survey(tool: &Tool, survey: &Survey) {
    for (at, page) in survey.pages.iter().enumerate() {
        let codes = page.codes.iter().filter(|(_, codes)| !codes.is_empty());
        let codes: Vec<String> = codes.map(|(table, codes)| format!("{table}: {}", codes.join(" "))).collect();
        println!("page {at}: zone {}; codes {}", page.mrz.as_deref().unwrap_or("none"), codes.join("; "));
    }
    for document in &survey.documents {
        let score = document.score.map_or("no side on any page".to_string(), |(pages, priority, asked)| {
            format!("{pages} pages, priority {priority}, {asked} asked")
        });
        println!("{} {:<34} {score}", if document.picked { "*" } else { " " }, document.id);
        for side in &document.sides {
            let asked = side.conditions.iter().map(|c| {
                let seen = c.seen.as_deref().filter(|_| c.kind != "mrz").map(|seen| tool.masked(seen));
                let seen = seen.or(c.seen.clone().filter(|_| c.kind == "mrz"));
                format!(
                    "{} {:?} {}{}",
                    c.kind,
                    c.wanted,
                    if c.held { "yes" } else { "NO" },
                    seen.map(|seen| format!(" ({seen:?})")).unwrap_or_default()
                )
            });
            println!(
                "    page {} {:<10} {:<6} {}",
                side.page,
                side.side,
                if side.holds { "holds" } else { "-" },
                asked.collect::<Vec<_>>().join("; ")
            );
        }
    }
}

fn print_explained(tool: &Tool, told: &Explained, only: Option<&str>) {
    if only.is_none() {
        print_survey(tool, &told.survey);
        println!();
    }
    let Some(document) = &told.document else {
        println!("not a known document: no side of any document holds on any page");
        return;
    };
    println!("fields of {document}{}", if told.forced { " (asked for, not picked)" } else { "" });
    for field in told.fields.iter().filter(|f| only.is_none_or(|key| f.key == key)) {
        println!("  {:<24} {:<10} {}", field.key, field.status, tool.masked(field.value.as_deref().unwrap_or("")));
        for tried in &field.tries {
            for attempt in &tried.attempts {
                let anchor = attempt.anchor.map(|line| format!(" at line {line}")).unwrap_or_default();
                let note = if attempt.note.is_empty() { String::new() } else { format!(": {}", attempt.note) };
                println!("      page {} [{}] {}{anchor}{note}", tried.page, tried.side, attempt.locator);
                for candidate in &attempt.candidates {
                    let checked = match candidate.checked {
                        Some(true) => "holds",
                        Some(false) => "fails its check",
                        None => "nothing to check",
                    };
                    let note = if candidate.note.is_empty() { "taken" } else { &candidate.note };
                    println!(
                        "          {:?} -> {:?} ({checked}; {note})",
                        tool.masked(&candidate.text),
                        tool.masked(&candidate.value)
                    );
                }
            }
        }
    }
}

fn command_explain(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    let sheets = tool.sheets(&set.ocr)?;
    let pages: Vec<Page> = sheets.into_iter().map(|sheet| sheet.page).collect();
    let document = tool.option("--document");
    if let Some(id) = document.filter(|id| !set.documents.iter().any(|d| d.id == *id)) {
        let why = set.skipped.iter().find(|(name, _)| name == id).map(|(_, why)| format!(": left out, {why}"));
        return Err(format!("The set has no document {id}{}", why.unwrap_or_default()));
    }
    let mut told = doctypes::explain(&set, &pages, "en", document);
    if tool.json {
        mask_explained(tool, &mut told);
        if let Some(key) = tool.option("--field") {
            told.fields.retain(|field| field.key == key);
        }
        print_json(&told);
    } else {
        print_explained(tool, &told, tool.option("--field"));
    }
    Ok(0)
}

/// Every kept page read on its own: the side of each document it is, and the document picked.
fn command_matrix(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    let pages = tool.kept_pages();
    if pages.is_empty() {
        return Err(format!("No pages under tests/lines for the model {}", tool.model));
    }
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let width = pages.iter().map(|(name, _)| name.len()).max().unwrap_or(0).max(4);
    if !tool.json {
        println!("{:<width$}  {:<34} also a side of", "page", "picked");
    }
    let mut unclear = 0;
    for (name, page) in &pages {
        let survey = doctypes::survey(&set, std::slice::from_ref(page));
        let side = |d: &doctypes::trace::DocumentSurvey| d.sides.iter().find(|s| s.holds).map(|s| s.side.clone());
        let picked = survey.documents.iter().find(|d| d.picked);
        let others: Vec<String> = survey
            .documents
            .iter()
            .filter(|d| !d.picked)
            .filter_map(|d| Some(format!("{} [{}]", d.id, side(d)?)))
            .collect();
        unclear += usize::from(picked.is_none() || !others.is_empty());
        let shown = picked.map_or("none".to_string(), |d| format!("{} [{}]", d.id, side(d).unwrap_or_default()));
        if !tool.json {
            println!("{name:<width$}  {shown:<34} {}", others.join(", "));
        }
        rows.push(serde_json::json!({
            "page": name,
            "picked": picked.map(|d| &d.id),
            "side": picked.and_then(side),
            "also": others,
        }));
    }
    if tool.json {
        print_json(&rows);
    } else {
        println!("\n{} pages, {unclear} picked by none or claimed by more than one document", pages.len());
    }
    Ok(0)
}

fn print_rules(tool: &Tool, rules: &[RuleTrace]) {
    for (at, rule) in rules.iter().enumerate() {
        if rules.len() > 1 {
            println!("alternative {}", at + 1);
        }
        for step in &rule.steps {
            println!("  {:<8} {:?}", step.step, tool.masked(&step.text));
        }
        if rule.candidates.is_empty() {
            println!("  nothing in the text matches");
        }
        for candidate in &rule.candidates {
            let checks =
                candidate.checks.iter().map(|c| format!("{} {}", c.check, if c.held { "holds" } else { "FAILS" }));
            println!("  found    {:?} {}", tool.masked(&candidate.value), checks.collect::<Vec<_>>().join(", "));
        }
        println!(
            "  value    {}",
            rule.value.as_deref().map_or("none".to_string(), |v| format!("{:?}", tool.masked(v)))
        );
    }
}

fn mask_rules(tool: &Tool, rules: &mut [RuleTrace]) {
    for rule in rules {
        rule.steps.iter_mut().for_each(|step| step.text = tool.masked(&step.text));
        rule.candidates.iter_mut().for_each(|c| c.value = tool.masked(&c.value));
        rule.value = rule.value.as_deref().map(|value| tool.masked(value));
    }
}

fn command_pattern(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    if tool.flag("--all") {
        let text = tool.rest.join(" ");
        let mut found: BTreeMap<&str, Option<String>> = BTreeMap::new();
        for name in set.patterns.names() {
            found.insert(name, set.patterns.extract(name, &text).map(|value| tool.masked(&value)));
        }
        if tool.json {
            print_json(&found);
        } else {
            found.iter().for_each(|(name, value)| println!("{name:<20} {}", value.as_deref().unwrap_or("-")));
        }
        return Ok(0);
    }
    let (Some(name), text) = (tool.rest.first(), tool.rest.iter().skip(1).cloned().collect::<Vec<_>>().join(" "))
    else {
        return Err(format!("Name a pattern: {}", set.patterns.names().join(", ")));
    };
    let mut rules = doctypes::explain_pattern(&set, name, &text)
        .ok_or(format!("The set has no pattern {name}: {}", set.patterns.names().join(", ")))?;
    if tool.json {
        mask_rules(tool, &mut rules);
        print_json(&rules);
    } else {
        print_rules(tool, &rules);
    }
    Ok(0)
}

/// A test case that expects what the pages read as now, as YAML for a file under `tests/cases`.
fn command_accept(tool: &Tool) -> Result<i32, String> {
    use serde_yaml::{Mapping, Value};
    let set = tool.set()?;
    let sheets = tool.sheets(&set.ocr)?;
    let pages: Vec<Page> = sheets.iter().map(|sheet| sheet.page.clone()).collect();
    let text = |text: &str| Value::String(text.to_string());
    let list = |items: &[String]| Value::Sequence(items.iter().map(|item| text(item)).collect());
    let mut case = Mapping::new();
    case.insert(text("name"), text(tool.option("--name").unwrap_or("TODO what this case shows")));
    case.insert(text("models"), list(std::slice::from_ref(&tool.model)));
    let named = sheets.iter().map(|sheet| Value::Mapping(Mapping::from_iter([(text("lines"), text(&sheet.name))])));
    case.insert(text("pages"), Value::Sequence(named.collect()));
    let expect = match doctypes::read(&set, &pages, "en") {
        None => text("none"),
        Some(reading) => {
            let mut expect = Mapping::new();
            expect.insert(text("doctype"), text(&reading.doctype));
            expect.insert(text("kind"), text(&reading.kind));
            expect.insert(text("sides"), list(&reading.sides));
            match &reading.number {
                Some(number) => expect.insert(text("number"), text(number)),
                None => expect.insert(text("no_number"), Value::Bool(true)),
            };
            for (name, status) in [
                ("ok", doctypes::Status::Ok),
                ("unchecked", doctypes::Status::Unchecked),
                ("failed", doctypes::Status::Failed),
            ] {
                let of = reading.fields.iter().filter(|f| f.status == status);
                let values: Mapping = of.map(|f| (text(&f.key), text(&f.value))).collect();
                if !values.is_empty() {
                    expect.insert(text(name), Value::Mapping(values));
                }
            }
            Value::Mapping(expect)
        }
    };
    case.insert(text("expect"), expect);
    let yaml = serde_yaml::to_string(&vec![Value::Mapping(case)]).map_err(|e| e.to_string())?;
    for sheet in sheets.iter().filter(|sheet| cases::kept_lines(&tool.dir, &sheet.name, Some(&tool.model)).is_err()) {
        eprintln!("{}: its lines are not kept yet: docset ocr --write <picture>", sheet.name);
    }
    print!("{yaml}");
    Ok(0)
}

fn command_check(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    let masked = |text: &str| tool.masked(text);
    let report = cases::run(&tool.dir, &set, &masked, None);
    if tool.json {
        print_json(&report);
    } else {
        for outcome in &report.outcomes {
            match outcome.wrong.iter().filter(|w| !w.is_empty()).collect::<Vec<_>>() {
                wrong if outcome.wrong.is_empty() && wrong.is_empty() => println!("ok    {}", outcome.title),
                wrong if wrong.is_empty() => println!("FAIL  {}", outcome.title),
                wrong => {
                    let lines: Vec<&str> = wrong.iter().map(|w| w.as_str()).collect();
                    println!("FAIL  {}\n        {}", outcome.title, lines.join("\n        "));
                }
            }
        }
        println!("\n{} documents, {} cases, {} failed", report.documents, report.cases, report.failures);
    }
    Ok(i32::from(report.failures > 0))
}

/// The set's `ocr:` settings with some keys given other values.
fn settings_with(dir: &Path, changes: &[(String, String)]) -> Result<OcrSettings, String> {
    let source = std::fs::read_to_string(dir.join("set.yaml")).map_err(|e| format!("set.yaml: {e}"))?;
    let file: serde_yaml::Value = serde_yaml::from_str(&source).map_err(|e| format!("set.yaml: {e}"))?;
    let mut ocr = file.get("ocr").and_then(|ocr| ocr.as_mapping()).cloned().ok_or("set.yaml has no ocr settings")?;
    for (key, value) in changes {
        let key = serde_yaml::Value::String(key.clone());
        if !ocr.contains_key(&key) {
            return Err(format!("set.yaml has no ocr setting {}", key.as_str().unwrap_or_default()));
        }
        ocr.insert(key, serde_yaml::from_str(value).map_err(|e| e.to_string())?);
    }
    serde_yaml::from_value(serde_yaml::Value::Mapping(ocr)).map_err(|e| e.to_string())
}

/// Every combination of the values given with `--vary key=a,b`, the set's own settings first (no changes).
fn combinations(tool: &Tool) -> Result<Vec<Vec<(String, String)>>, String> {
    let mut all: Vec<Vec<(String, String)>> = vec![Vec::new()];
    for vary in tool.options.get("--vary").map(Vec::as_slice).unwrap_or_default() {
        let (key, values) = vary.split_once('=').ok_or(format!("--vary {vary}: write key=a,b"))?;
        let values: Vec<&str> = values.split(',').filter(|v| !v.is_empty()).collect();
        all = all
            .iter()
            .flat_map(|sofar| values.iter().map(|v| [sofar.clone(), vec![(key.to_string(), v.to_string())]].concat()))
            .collect();
    }
    if all.len() > MAX_COMBINATIONS {
        return Err(format!("{} combinations: at most {MAX_COMBINATIONS}", all.len()));
    }
    if all.len() > 1 {
        all.insert(0, Vec::new());
    }
    Ok(all)
}

fn named(changes: &[(String, String)]) -> String {
    if changes.is_empty() {
        return "the set's own".into();
    }
    changes.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join(" ")
}

fn command_sweep(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    let combinations = combinations(tool)?;
    let masked = |text: &str| tool.masked(text);
    if tool.flag("--all") {
        let pictures = tool.kept_pictures();
        if pictures.is_empty() {
            return Err("No pictures under tests/images".into());
        }
        for changes in &combinations {
            let settings = settings_with(&tool.dir, changes)?;
            let mut pages = HashMap::new();
            for picture in &pictures {
                pages.insert(page_name(picture), tool.ocr(picture, &settings)?);
            }
            let fresh = cases::Fresh { model: tool.model.clone(), pages };
            let report = cases::run(&tool.dir, &set, &masked, Some(&fresh));
            println!("{:<40} {} of {} cases pass", named(changes), report.cases - report.failures, report.cases);
            for outcome in report.outcomes.iter().filter(|o| !o.wrong.is_empty()) {
                println!("    FAIL {}: {}", outcome.title, outcome.wrong.join("; "));
            }
        }
        return Ok(0);
    }
    let pictures: Vec<&Path> = tool.rest.iter().map(Path::new).collect();
    if pictures.is_empty() || pictures.iter().any(|p| !p.is_file() || !is_picture(p)) {
        return Err("Name the pictures to read, or --all for those of tests/images".into());
    }
    let mut first: Option<Vec<(String, String)>> = None;
    for changes in &combinations {
        let settings = settings_with(&tool.dir, changes)?;
        let pages: Vec<Page> = pictures.iter().map(|p| tool.ocr(p, &settings)).collect::<Result<_, _>>()?;
        let lines: Vec<&doctypes::Line> = pages.iter().flat_map(|page| &page.lines).collect();
        let sure = lines.iter().map(|l| l.confidence as f64).sum::<f64>() / lines.len().max(1) as f64;
        let reading = doctypes::read(&set, &pages, "en");
        let values: Vec<(String, String)> = reading
            .iter()
            .flat_map(|r| &r.fields)
            .map(|f| (f.key.clone(), format!("{} ({:?})", f.value, f.status).to_lowercase()))
            .collect();
        let count = |status| reading.iter().flat_map(|r| &r.fields).filter(|f| f.status == status).count();
        println!(
            "{:<40} {} lines, confidence {sure:.3}; {}: {} ok, {} unchecked, {} failed",
            named(changes),
            lines.len(),
            reading.as_ref().map_or("not a known document", |r| r.doctype.as_str()),
            count(doctypes::Status::Ok),
            count(doctypes::Status::Unchecked),
            count(doctypes::Status::Failed),
        );
        let Some(first) = &first else {
            first = Some(values);
            continue;
        };
        let keys: Vec<&String> = first.iter().chain(&values).map(|(key, _)| key).collect();
        let mut told: HashSet<&String> = HashSet::new();
        for key in keys.into_iter().filter(|key| told.insert(key)) {
            let of = |values: &[(String, String)]| values.iter().find(|(k, _)| k == key).map(|(_, v)| tool.masked(v));
            if of(first) != of(&values) {
                let none = || "not read".to_string();
                println!("    {key}: {} -> {}", of(first).unwrap_or_else(none), of(&values).unwrap_or_else(none));
            }
        }
    }
    Ok(0)
}

fn command_scaffold(tool: &Tool) -> Result<i32, String> {
    let set = tool.set()?;
    let sheets = tool.sheets(&set.ocr)?;
    let id = tool.option("--id").ok_or("Name the document: --id <id>")?;
    let names: Vec<&str> = sheets.iter().map(|sheet| sheet.name.as_str()).collect();
    let others: Vec<(String, Page)> =
        tool.kept_pages().into_iter().filter(|(name, _)| !names.contains(&name.as_str())).collect();
    let masked = |text: &str| tool.masked(text);
    let draft = scaffold::draft(&set, id, &sheets, &others, &masked);
    let pages: Vec<Page> = sheets.into_iter().map(|sheet| sheet.page).collect();
    print!("{draft}{}", scaffold::verdict(&tool.dir, id, &draft, &pages));
    Ok(0)
}

fn usage() -> ! {
    let source = include_str!("main.rs");
    let help: Vec<&str> =
        source.lines().take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!")).collect();
    eprintln!("{}", help.join("\n"));
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut flags, mut options, mut rest) = (HashSet::new(), HashMap::<String, Vec<String>>::new(), Vec::new());
    while let Some(arg) = args.next() {
        if VALUED.contains(&arg.as_str()) {
            let Some(value) = args.next() else { usage() };
            options.entry(arg).or_default().push(value);
        } else if FLAGS.contains(&arg.as_str()) {
            flags.insert(arg);
        } else if arg.starts_with("--") {
            eprintln!("Unknown option {arg}\n");
            usage();
        } else {
            rest.push(arg);
        }
    }
    if rest.is_empty() {
        usage();
    }
    let command = rest.remove(0);
    let last = |name: &str| options.get(name).and_then(|values| values.last()).cloned();
    let tool = Tool {
        dir: PathBuf::from(last("--set").unwrap_or("doc-configs".into())),
        model: last("--model").unwrap_or("cyrillic".into()),
        show: flags.contains("--show"),
        json: flags.contains("--json"),
        flags,
        options,
        rest,
        engine: OnceCell::new(),
    };
    let done = match command.as_str() {
        "ocr" => command_ocr(&tool),
        "read" => command_read(&tool),
        "explain" => command_explain(&tool),
        "view" => view::command(&tool),
        "matrix" => command_matrix(&tool),
        "pattern" => command_pattern(&tool),
        "scaffold" => command_scaffold(&tool),
        "accept" => command_accept(&tool),
        "check" => command_check(&tool),
        "sweep" => command_sweep(&tool),
        _ => usage(),
    };
    match done {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
