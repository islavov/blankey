//! A first draft of a document file from the lines of its specimen pages: what could tell each page apart, and the
//! fields its labels, printed codes and machine-readable zone suggest. Every guess is marked; the draft is where
//! writing a document starts, not what is deployed.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use blankey_doctypes::{self as doctypes, canonical, Field, Line, Page, Script, Set};
use regex::Regex;

use crate::Sheet;

static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{2}\s?[.,/]\s?\d{2}\s?[.,/]\s?\d{4}").unwrap());

/// A table is the page's when this many of its codes are printed on it.
const TABLE_CODES: usize = 3;
/// A line read less surely than this is no label and tells no page apart.
const SURE: f32 = 0.8;

/// A YAML scalar for inside `{ }` and `[ ]`: quoted when it holds anything those read as their own.
fn scalar(text: &str) -> String {
    let plain = !text.is_empty()
        && text.trim() == text
        && !text.contains([',', '[', ']', '{', '}', ':', '#', '"', '\'', '&', '*', '!', '|', '>', '%', '@', '`'])
        && !text.starts_with(['-', '?'])
        && serde_yaml::from_str::<serde_yaml::Value>(text).is_ok_and(|value| value.is_string());
    if plain {
        text.to_string()
    } else {
        serde_json::to_string(text).unwrap_or_else(|_| format!("{text:?}"))
    }
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|item| scalar(item)).collect::<Vec<_>>().join(", "))
}

/// A field as a line of a document file.
fn field_line(field: &Field) -> String {
    let mut parts = vec![format!("key: {}", field.key)];
    if field.field_type != "text" {
        parts.push(format!("type: {}", field.field_type));
    }
    if let Some(part) = &field.mrz {
        parts.push(format!("mrz: {part}"));
    }
    if !field.label_of.is_empty() {
        parts.push(format!("label_of: {}", list(&field.label_of)));
    }
    if let Some(pattern) = &field.pattern {
        parts.push(format!("pattern: {pattern}"));
    }
    if field.part > 0 {
        parts.push(format!("part: {}", field.part));
    }
    match field.script {
        Some(Script::Cyrillic) => parts.push("script: cyrillic".into()),
        Some(Script::Latin) => parts.push("script: latin".into()),
        None => {}
    }
    if field.below {
        parts.push("below: true".into());
    }
    parts.push(format!("label: {{ bg: {}, en: {} }}", scalar(&field.label.bg), scalar(&field.label.en)));
    format!("  - {{ {} }}", parts.join(", "))
}

fn middle(rect: &[f64; 4]) -> f64 {
    (rect[1] + rect[3]) / 2.0
}

/// Where the value starts in a line that opens with its label: the first run of capitals or digits, as values are
/// printed. The label before it, and the value.
fn label_and_value(text: &str) -> (&str, &str) {
    let valued = |c: char| c.is_uppercase() || c.is_ascii_digit();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    match chars.windows(2).find(|pair| valued(pair[0].1) && valued(pair[1].1)) {
        Some(pair) => (text[..pair[0].0].trim(), text[pair[0].0..].trim()),
        None => (text.trim(), ""),
    }
}

/// The printed forms of a label, "Фамилия/Surname": each half, lower case.
fn aliases(label: &str) -> Vec<String> {
    let clean = |half: &str| half.trim().trim_end_matches([':', '.', ',']).trim().to_lowercase();
    label.split('/').map(clean).filter(|half| half.chars().filter(|c| c.is_alphabetic()).count() >= 3).collect()
}

fn is_label(label: &str) -> bool {
    let letters: Vec<char> = label.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 3 && letters.iter().filter(|c| c.is_lowercase()).count() * 2 >= letters.len()
}

/// The nearest line on the same row to the right.
fn beside<'p>(page: &'p Page, line: &Line) -> Option<&'p Line> {
    let height = line.rect[3] - line.rect[1];
    let same_row = |other: &&Line| (middle(&other.rect) - middle(&line.rect)).abs() < 0.7 * height;
    let right = page.lines.iter().filter(same_row).filter(|other| other.rect[0] >= line.rect[2]);
    right.min_by(|a, b| a.rect[0].total_cmp(&b.rect[0]))
}

/// What the draft says of one page.
struct Side {
    id: String,
    matches: Vec<String>,
    notes: Vec<String>,
}

pub fn draft(
    set: &Set,
    id: &str,
    sheets: &[Sheet],
    others: &[(String, Page)],
    masked: &dyn Fn(&str) -> String,
) -> String {
    // the fields the set's documents already have, by the labels and zone parts they are found by: a new document
    // that prints the same label should fill the same key
    let known: Vec<&Field> = set.documents.iter().flat_map(|d| &d.fields).collect();
    let by_label = |alias: &str| known.iter().find(|f| f.label_of.iter().any(|l| l.to_lowercase() == alias)).copied();
    let by_part = |part: &str| known.iter().find(|f| f.mrz.as_deref() == Some(part)).copied();

    let mut sides: Vec<Side> = Vec::new();
    let mut fields: Vec<(Field, String)> = Vec::new();
    let mut codes: Vec<String> = Vec::new();
    let mut uses: Option<String> = None;
    let mut todo = 0;
    let has = |fields: &[(Field, String)], key: &str| fields.iter().any(|(f, _)| f.key == key);

    for (at, sheet) in sheets.iter().enumerate() {
        let page = &sheet.page;
        let survey = doctypes::survey(set, std::slice::from_ref(page));
        let mut side = Side { id: format!("side{}", at + 1), matches: Vec::new(), notes: Vec::new() };
        side.notes.push(format!("{}: {} lines", sheet.name, page.lines.len()));
        let zone = doctypes::zone(page);
        let mut taken: Vec<usize> = zone.iter().flat_map(|zone| zone.lines.clone()).collect();

        if let Some(zone) = &zone {
            side.matches.push(format!("mrz: {}", zone.state));
            for part in &zone.parts {
                let field = match by_part(&part.name) {
                    Some(field) => Field { label_of: Vec::new(), ..field.clone() },
                    None => {
                        todo += 1;
                        Field {
                            key: format!("todo_{todo}"),
                            mrz: Some(part.name.clone()),
                            label: doctypes::Text { en: part.name.replace('_', " "), bg: part.name.replace('_', " ") },
                            ..blank()
                        }
                    }
                };
                if !has(&fields, &field.key) {
                    fields.push((field, format!("zone: {}", masked(&part.value))));
                }
            }
        }

        // the table most of whose codes the page prints
        let table = survey.pages[0].codes.iter().max_by_key(|(_, printed)| printed.len());
        if let Some((table, printed)) = table.filter(|(_, printed)| printed.len() >= TABLE_CODES) {
            uses = Some(table.clone());
            let elsewhere = |code: &String| {
                let on = |page: &Page| {
                    doctypes::survey(set, std::slice::from_ref(page)).pages[0].codes[table].contains(code)
                };
                sheets.iter().enumerate().any(|(other, sheet)| other != at && on(&sheet.page))
                    || others.iter().any(|(_, page)| on(page))
            };
            let own: Vec<String> = printed.iter().filter(|code| !elsewhere(code)).take(3).cloned().collect();
            let all: Vec<String> = if own.is_empty() { printed.iter().take(3).cloned().collect() } else { own };
            side.matches.push(format!("codes_all: {}", list(&all)));
            let of_table: Vec<String> = set
                .tables
                .iter()
                .filter(|t| &t.id == table)
                .flat_map(|t| t.codes.iter().filter_map(|f| f.code.clone()))
                .collect();
            let mut none: Vec<String> = of_table.into_iter().filter(|c| !printed.contains(c) && elsewhere(c)).collect();
            none.dedup();
            if !none.is_empty() {
                none.truncate(4);
                side.matches.push(format!("codes_none: {}", list(&none)));
            }
            if let Some(scan) = doctypes::table_scan(set, table, page) {
                taken.extend(scan.lines.iter().filter(|line| line.role != "value").map(|line| line.index));
                for code in printed {
                    let valued = scan.codes.get(&canonical(code)).is_some_and(|rows| !rows.is_empty());
                    if valued && !codes.contains(code) {
                        codes.push(code.clone());
                    }
                }
                let empty: Vec<&String> = printed.iter().filter(|code| !codes.contains(code)).collect();
                if !empty.is_empty() {
                    let empty: Vec<&str> = empty.iter().map(|code| code.as_str()).collect();
                    side.notes.push(format!("codes printed with no value beside them: {}", empty.join(" ")));
                }
            }
        }

        // labels: a line that opens in lower case, with its value after it or to its right
        let mut labelled: Vec<usize> = Vec::new();
        for (index, line) in page.lines.iter().enumerate() {
            if taken.contains(&index) || line.confidence < SURE {
                continue;
            }
            // a label in capitals, "ЕГН/Personal No", looks like a value: a document of the set knows it for a label
            let whole = aliases(&line.text).iter().any(|alias| by_label(alias).is_some());
            let (label, mut value) = if whole { (line.text.as_str(), "") } else { label_and_value(&line.text) };
            if !whole && !is_label(label) {
                continue;
            }
            let found = aliases(label);
            // a short word no document knows as a label is more often a piece of a value: "бул." of an address
            let known = found.iter().any(|alias| by_label(alias).is_some());
            if found.is_empty() || (!known && found.iter().all(|alias| alias.chars().count() < 4)) {
                continue;
            }
            if value.is_empty() {
                value = beside(page, line).map_or("", |other| other.text.as_str());
            }
            // with nothing after it or beside it, it is a caption: something to tell the page by, not a field
            if value.is_empty() && !known {
                continue;
            }
            labelled.push(index);
            let field = match found.iter().find_map(|alias| by_label(alias)) {
                Some(field) => Field { mrz: None, code: None, ..field.clone() },
                None => {
                    todo += 1;
                    let fits: Vec<&str> = set
                        .patterns
                        .names()
                        .into_iter()
                        .filter(|name| set.patterns.extract(name, value).is_some())
                        .collect();
                    let cyrillic = |text: &&String| text.chars().any(|c| !c.is_ascii() && c.is_alphabetic());
                    let bg = found.iter().find(cyrillic).unwrap_or(&found[0]);
                    let en = found.iter().find(|text| !cyrillic(text)).unwrap_or(&found[0]);
                    Field {
                        key: format!("todo_{todo}"),
                        field_type: if DATE.is_match(value) { "date".into() } else { "text".into() },
                        label_of: found.clone(),
                        pattern: (fits.len() == 1).then(|| fits[0].to_string()),
                        label: doctypes::Text { en: en.clone(), bg: bg.clone() },
                        ..blank()
                    }
                }
            };
            match fields.iter_mut().find(|(known, _)| known.key == field.key) {
                // the zone already gives it: the label is where it is read when the zone is not
                Some((known, _)) if known.label_of.is_empty() => known.label_of = field.label_of,
                Some(_) => {}
                None => fields.push((field, format!("line {index}: {}", masked(value)))),
            }
        }
        // phrases this page prints and no other page does
        let mut phrases: Vec<&Line> = page
            .lines
            .iter()
            .enumerate()
            .filter(|(index, line)| !taken.contains(index) && !labelled.contains(index) && line.confidence >= SURE)
            .map(|(_, line)| line)
            .filter(|line| {
                let letters = line.text.chars().filter(|c| c.is_alphabetic()).count();
                letters >= 6 && !line.text.chars().any(|c| c.is_ascii_digit())
            })
            .filter(|line| {
                let said = |page: &Page| doctypes::says(page, &line.text);
                !others.iter().any(|(_, page)| said(page))
                    && !sheets.iter().enumerate().any(|(other, sheet)| other != at && said(&sheet.page))
            })
            .collect();
        phrases.sort_by(|a, b| (b.rect[3] - b.rect[1]).total_cmp(&(a.rect[3] - a.rect[1])));
        let phrases: Vec<String> = phrases.iter().take(2).map(|line| line.text.trim().to_lowercase()).collect();
        if !phrases.is_empty() {
            side.matches.push(format!("text_all: {}", list(&phrases)));
        }
        if side.matches.is_empty() {
            side.matches.push("text_all: [TODO a phrase only this page prints]".into());
        }

        sides.push(side);
    }

    if fields.is_empty() && codes.is_empty() {
        let label = doctypes::Text { en: "TODO".into(), bg: "TODO".into() };
        fields.push((
            Field { key: "todo_1".into(), label_of: vec!["todo".into()], label, ..blank() },
            "no field found".into(),
        ));
    }
    let table_fields: Vec<&Field> = set
        .tables
        .iter()
        .filter(|table| Some(&table.id) == uses.as_ref())
        .flat_map(|table| &table.codes)
        .filter(|field| field.code.as_ref().is_some_and(|code| codes.contains(code)))
        .collect();
    let keys: Vec<&str> =
        table_fields.iter().map(|f| f.key.as_str()).chain(fields.iter().map(|(f, _)| f.key.as_str())).collect();
    // what tells one card or car from another: a field with a pattern, as numbers have, else the first
    let numbered = table_fields.iter().copied().chain(fields.iter().map(|(f, _)| f)).find(|f| f.pattern.is_some());
    let number = numbered.map_or(keys[0], |f| f.key.as_str());

    let mut out = String::new();
    out += "# DRAFT by docset scaffold: every TODO and todo_N is a guess to settle. Keys shared with other documents\n";
    out += "# of the set were taken from them where a label or a zone part is the same.\n";
    out += &format!("id: {id}\n");
    out += "name: { bg: TODO, en: TODO }\n";
    out += &format!("family: {id}\n");
    out += &format!("profile: {{ kind: person, name: \"{{{}}}\", number: {number} }}  # TODO kind, name\n", keys[0]);
    if let Some(table) = &uses {
        out += &format!("uses: {table}\n");
    }
    out += "sides:\n";
    for side in &sides {
        for note in &side.notes {
            out += &format!("  # {note}\n");
        }
        out += &format!("  - id: {}\n    match: {{ {} }}\n", side.id, side.matches.join(", "));
    }
    if !codes.is_empty() {
        out += &format!("codes: {}\n", list(&codes));
    }
    if !fields.is_empty() {
        out += "fields:\n";
        for (field, read) in &fields {
            out += &format!("{}  # {read}\n", field_line(field));
        }
    }
    out
}

fn blank() -> Field {
    Field {
        key: String::new(),
        label: doctypes::Text::default(),
        field_type: "text".into(),
        code: None,
        label_of: Vec::new(),
        mrz: None,
        pattern: None,
        script: None,
        line: 1,
        part: 0,
        below: false,
        items: false,
    }
}

/// What the set makes of the draft: whether it parses as a document of it, and whether its own pages read as it.
pub fn verdict(set_dir: &std::path::Path, id: &str, draft: &str, pages: &[Page]) -> String {
    let mut files: BTreeMap<String, String> = Set::files_in(set_dir);
    files.retain(|_, text| !text.lines().any(|line| line.trim() == format!("id: {id}")));
    files.insert(format!("documents/zz_draft/{id}.yaml"), draft.to_string());
    let set = match Set::parse("draft", &files) {
        Ok(set) => set,
        Err(error) => return format!("# check: the set does not load with it: {error}\n"),
    };
    if let Some((_, why)) = set.skipped.iter().find(|(name, _)| name == id || name.ends_with(&format!("{id}.yaml"))) {
        return format!("# check: the engine leaves it out: {why}\n");
    }
    match doctypes::read(&set, pages, "en") {
        Some(reading) if reading.doctype == id => format!(
            "# check: it parses, and its pages read as it: sides {:?}, {} fields\n",
            reading.sides,
            reading.fields.len()
        ),
        Some(reading) => format!("# check: it parses, but its pages read as {}: sharpen `match`\n", reading.doctype),
        None => "# check: it parses, but its pages are not recognized: fix `match`\n".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page of lines, each (text, x, y); a line is 14 high and 9 wide a character.
    fn page(lines: &[(&str, f64, f64)]) -> Page {
        let lines = lines.iter().map(|(text, x, y)| Line {
            text: text.to_string(),
            rect: [*x, *y, x + 9.0 * text.chars().count() as f64, y + 14.0],
            confidence: 0.95,
        });
        Page { width: 600.0, height: 400.0, lines: lines.collect(), turned: 0, skew: 0.0 }
    }

    /// The example set in a folder, as a checkout keeps one.
    fn example_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in doctypes::example_set_files() {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        dir
    }

    fn sheet(name: &str, page: Page) -> Sheet {
        Sheet { name: name.into(), page, picture: None }
    }

    #[test]
    fn a_page_of_labels_drafts_a_document_that_reads_it() {
        let dir = example_dir();
        let set = Set::from_dir(dir.path()).unwrap();
        let pass = page(&[
            ("EXAMPLE SWIMMING POOL", 40.0, 20.0),
            ("Season pass", 40.0, 40.0),
            ("Holder/Притежател JOHN EXAMPLE", 40.0, 80.0),
            ("Member no M123456", 40.0, 100.0),
            ("Valid until: 31.12.2030", 40.0, 120.0),
            ("Size, cm", 40.0, 140.0),
            ("LARGE 180", 200.0, 140.0),
        ]);
        let sheets = [sheet("pool_pass", pass.clone())];
        let draft = draft(&set, "example_pool_pass", &sheets, &[], &|text| text.to_string());
        assert!(draft.contains("text_all: [example swimming pool"), "{draft}");
        assert!(
            draft.contains("key: member.number, label_of: [member no], pattern: member_no"),
            "a known label: {draft}"
        );
        assert!(draft.contains("label_of: [holder, притежател], label: { bg: притежател, en: holder }"), "{draft}");
        assert!(draft.contains("type: date, label_of: [valid until]"), "{draft}");
        assert!(draft.contains(r#"label: { bg: "size, cm", en: "size, cm" }"#), "a comma is quoted: {draft}");
        assert!(draft.contains("number: member.number"), "the field with a pattern identifies it: {draft}");
        let verdict = verdict(dir.path(), "example_pool_pass", &draft, &[pass]);
        assert!(verdict.contains("its pages read as it") && verdict.contains("4 fields"), "{verdict}");
    }

    #[test]
    fn a_page_of_codes_drafts_a_document_of_its_table() {
        let dir = example_dir();
        let set = Set::from_dir(dir.path()).unwrap();
        let licence = page(&[
            ("EXAMPLE LICENCE BUREAU", 40.0, 20.0),
            ("(A) P-9876", 40.0, 60.0),
            ("(B) 01.02.2020", 40.0, 80.0),
            ("(D) 900 kg", 40.0, 100.0),
            ("(F) NONE", 40.0, 120.0),
        ]);
        let draft = draft(&set, "example_licence", &[sheet("licence", licence.clone())], &[], &|text| text.to_string());
        assert!(draft.contains("uses: example_codes") && draft.contains("codes: [A, B, D, F]"), "{draft}");
        assert!(draft.contains("codes_all: [A, B, D]"), "{draft}");
        let verdict = verdict(dir.path(), "example_licence", &draft, &[licence]);
        assert!(verdict.contains("its pages read as it"), "more pages asked for than the permit: {verdict}");
    }
}
