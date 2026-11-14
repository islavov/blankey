//! Finds a config's fields on one page: beside printed codes, after printed labels, in the machine-readable zone,
//! or by pattern. Positions are the page's own: x to the right, y down.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use super::mrz::{self, Mrz};
use super::patterns::Patterns;
use super::set::{canonical, Config, Field, Script};
use super::trace::{Attempt, Candidate, LabelTrace, LineTrace, ScanTrace, Tracer, ZoneTrace};
use super::{fold, patterns, Line, Page};

/// A field's value on the page. `checked`: whether its check holds; None when it has none.
pub struct Found {
    pub value: String,
    pub place: Option<[f64; 4]>,
    pub checked: Option<bool>,
}

/// A text tried as a field's value, for the tracer.
fn tried(text: &str, found: &Found, note: &str) -> Candidate {
    Candidate {
        text: text.to_string(),
        rect: found.place,
        value: found.value.clone(),
        checked: found.checked,
        note: note.to_string(),
    }
}

/// A printed label of a field inside a line: chars `start..end` of its text.
struct Hit {
    field: usize,
    start: usize,
    end: usize,
}

/// Lines OCR is this unsure of are not taken as values.
const MIN_CONFIDENCE: f32 = 0.6;

static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{2})\s?[.,/]\s?(\d{2})\s?[.,/]\s?(\d{4})").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:[.,]\d+)?").unwrap());
static STARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[*·.\s]*$").unwrap());
static TRANSLATED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\([^()]*\)\s*$").unwrap());
static LATIN_HALF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/[A-Za-z .'-]+").unwrap());
static NEXT_LABEL_HALF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\S+/\s*$").unwrap());

fn middle(rect: &[f64; 4]) -> f64 {
    (rect[1] + rect[3]) / 2.0
}

fn height(rect: &[f64; 4]) -> f64 {
    rect[3] - rect[1]
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// How many wrong characters a printed phrase of this length may be read with.
pub(super) fn slack(chars: usize) -> usize {
    (chars / 6).min(2)
}

/// A line that starts with a printed code, "(F.1) 1732": the code as the table has it, and what follows it.
/// OCR drops dots and reads look-alikes ("(F1)", "(0.1)", "(G.1.1.)"), so the code is compared without them.
pub(super) fn anchor(text: &str, known: &[String]) -> Option<(String, String)> {
    let text = text.trim_start();
    // the opening bracket is sometimes lost: "V.1)" is still a code when a bracket closes it at once
    let closed = |rest: &str| rest.chars().take(7).take_while(|c| *c != ')').all(|c| c.is_alphanumeric() || c == '.');
    let rest =
        text.strip_prefix('(').or(Some(text).filter(|rest| rest.chars().take(7).any(|c| c == ')') && closed(rest)))?;
    let (raw, after) = match rest.char_indices().take(9).find(|(_, c)| *c == ')') {
        Some((at, _)) => (&rest[..at], &rest[at + 1..]),
        None => {
            let end = rest
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '.' | '$')))
                .map_or(rest.len(), |(at, _)| at);
            (&rest[..end], &rest[end..])
        }
    };
    if raw.chars().next()?.is_lowercase() {
        return None;
    }
    let mut code = canonical(&fold::latin(&raw.replace('$', "S")));
    let numbered = |code: &str| code.len() > 1 && code[1..].chars().all(|c| c.is_ascii_digit());
    if code.starts_with('0') {
        code.replace_range(0..1, "O");
    }
    // only C has two digits after it, and no G has any
    if (code.starts_with('G') && numbered(&code)) || (code.starts_with('O') && code.len() == 3 && numbered(&code)) {
        code.replace_range(0..1, "C");
    }
    known.contains(&code).then(|| (code, after.trim().to_string()))
}

/// The text from its first run of capitals or digits: values are printed so, labels are not.
fn value_start(text: &str) -> &str {
    let valued = |c: char| c.is_uppercase() || c.is_ascii_digit();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    chars.windows(2).find(|pair| valued(pair[0].1) && valued(pair[1].1)).map_or(text, |pair| &text[pair[0].0..])
}

/// What a line is, for a line that cannot be a value.
#[derive(Clone, PartialEq)]
enum Role {
    Value,
    /// starts with a code of the table: the code, and what follows it in the line
    Code(String, String),
    /// starts with something in brackets that is no code of the table: a code OCR misread
    Bracket,
    /// a "(a)" item of a list: the value of a code whose value is one
    Item,
    /// a title, a caption or a label
    Print,
    Zone,
}

/// A line that opens with a bracket and is no known code: a misread code, or with a lowercase letter a list item.
fn bracket(text: &str) -> Option<Role> {
    let inside = text.trim_start().strip_prefix('(')?;
    Some(if inside.chars().next().is_some_and(char::is_lowercase) { Role::Item } else { Role::Bracket })
}

pub struct Scan<'a> {
    page: &'a Page,
    patterns: &'a Patterns,
    roles: Vec<Role>,
    mrz: Option<Mrz>,
    /// canonical code -> the lines of its value, top to bottom
    codes: HashMap<String, Vec<(String, [f64; 4])>>,
    /// per line: the labels in it, left to right
    labels: Vec<Vec<Hit>>,
}

impl<'a> Scan<'a> {
    pub fn new(page: &'a Page, config: &Config, patterns: &'a Patterns) -> Scan<'a> {
        let skeletons: Vec<Vec<char>> = page.lines.iter().map(|l| fold::skeleton(&l.text)).collect();
        let mrz = mrz::find(&page.lines);
        let in_mrz = |line: usize| mrz.as_ref().is_some_and(|m| m.lines.contains(&line));
        let printed = |skeleton: &Vec<char>| {
            config
                .print
                .iter()
                .map(|phrase| fold::skeleton(phrase))
                .any(|needle| fold::find(skeleton, &needle, slack(needle.len())).is_some())
        };
        let mut labels: Vec<Vec<Hit>> = Vec::new();
        let mut roles: Vec<Role> = Vec::new();
        for (i, line) in page.lines.iter().enumerate() {
            let coded = anchor(&line.text, &config.known_codes).map(|(code, rest)| Role::Code(code, rest));
            let role = if in_mrz(i) { Some(Role::Zone) } else { coded.or_else(|| bracket(&line.text)) };
            let hits = if role.is_some() {
                Vec::new()
            } else {
                label_hits(&skeletons[i], &fold::case_breaks(&line.text), &config.fields)
            };
            let print = !hits.is_empty() || printed(&skeletons[i]);
            roles.push(role.unwrap_or(if print { Role::Print } else { Role::Value }));
            labels.push(hits);
        }
        let codes = code_values(page, &roles, &config.item_codes);
        Scan { page, patterns, roles, mrz, codes, labels }
    }

    /// The field on this page: the first locator that finds a value that holds, else the first that failed.
    pub fn field(&self, index: usize, field: &Field) -> Option<Found> {
        self.field_traced(index, field, &mut Tracer::off())
    }

    /// `field`, telling the tracer what each locator had to go by and tried.
    pub fn field_traced(&self, index: usize, field: &Field, tracer: &mut Tracer) -> Option<Found> {
        let mut failed = None;
        let mut keep = |found: Option<Found>| match found {
            Some(found) if found.checked != Some(false) => Some(found),
            other => {
                failed = failed.take().or(other);
                None
            }
        };
        if let Some(name) = &field.mrz {
            let part = self.mrz.as_ref().and_then(|zone| zone.parts.iter().find(|p| p.name == name));
            let found = part.map(|p| Found {
                value: p.value.clone(),
                place: Some(self.page.lines[p.line].rect),
                checked: p.checked,
            });
            tracer.attempt(|| Attempt {
                locator: "mrz",
                anchor: part.map(|p| p.line),
                candidates: found.iter().map(|found| tried(&found.value, found, "")).collect(),
                note: match (&self.mrz, part) {
                    (None, _) => "no machine-readable zone on the page".into(),
                    (Some(_), None) => format!("the zone has no part {name}"),
                    _ => String::new(),
                },
            });
            if let Some(found) = keep(found) {
                return Some(found);
            }
        }
        if let Some(code) = &field.code {
            let code = canonical(code);
            let rows = self.codes.get(&code);
            let row = rows.and_then(|rows| rows.get(field.line - 1));
            let found = row.map(|(text, rect)| shape(field, text, *rect, self.patterns));
            tracer.attempt(|| Attempt {
                locator: "code",
                anchor: self.roles.iter().position(|role| matches!(role, Role::Code(printed, _) if *printed == code)),
                candidates: row
                    .zip(found.as_ref())
                    .map(|((text, _), found)| tried(text, found, ""))
                    .into_iter()
                    .collect(),
                note: match (rows, row) {
                    (None, _) => "the code is not printed on the page".into(),
                    (Some(rows), None) if rows.is_empty() => "the code has no value beside it".into(),
                    (Some(rows), None) => format!("its value has {} lines, not {}", rows.len(), field.line),
                    _ => String::new(),
                },
            });
            if let Some(found) = keep(found) {
                return Some(found);
            }
        }
        if !field.label_of.is_empty() {
            tracer.attempt(|| Attempt { locator: "label", anchor: None, candidates: Vec::new(), note: String::new() });
            if let Some(found) = keep(self.labelled(index, field, tracer)) {
                return Some(found);
            }
        }
        if let Some(pattern) = field.pattern.as_ref().filter(|_| field.code.is_none()) {
            let lines = self.page.lines.iter().enumerate().filter(|(_, l)| l.confidence >= MIN_CONFIDENCE);
            let hit = lines.filter_map(|(at, l)| Some((at, l, self.patterns.extract(pattern, &l.text)?))).next();
            let found = hit.as_ref().map(|(_, l, value)| Found {
                value: value.clone(),
                place: Some(l.rect),
                checked: Some(true),
            });
            tracer.attempt(|| Attempt {
                locator: "pattern",
                anchor: hit.as_ref().map(|(at, ..)| *at),
                candidates: hit
                    .iter()
                    .zip(found.as_ref())
                    .map(|((_, l, _), found)| tried(&l.text, found, ""))
                    .collect(),
                note: if hit.is_none() { format!("no line holds the pattern {pattern}") } else { String::new() },
            });
            return keep(found).or(failed);
        }
        failed
    }

    /// The value after the field's label in its line, else in the line to its right; with `below`, also the lines
    /// under it up to the next label.
    fn labelled(&self, index: usize, field: &Field, tracer: &mut Tracer) -> Option<Found> {
        let mut failed = None;
        let mut printed = false;
        for (at, hits) in self.labels.iter().enumerate() {
            let Some(last) = hits.iter().rposition(|h| h.field == index) else { continue };
            printed = true;
            tracer.anchor(at);
            let line = &self.page.lines[at];
            let chars: Vec<char> = line.text.chars().collect();
            let next = hits.get(last + 1).map(|h| h.start);
            let mut raw: String = chars[hits[last].end..next.unwrap_or(chars.len())].iter().collect();
            if next.is_some() {
                // the next label's half in the other language, "ЕГН/" before "Personal No"
                raw = NEXT_LABEL_HALF.replace(&raw, "").into_owned();
            }
            // this label's own other half, "/Surname"
            let rest = match raw.trim().strip_prefix('/') {
                Some(half) => half.split_once(char::is_whitespace).map_or("", |(_, value)| value),
                None => raw.trim(),
            };
            let beside = self.beside(at);
            if field.below {
                let mut parts: Vec<(String, [f64; 4])> = Vec::new();
                parts.extend(
                    (!rest.trim().is_empty())
                        .then(|| (rest.trim().to_string(), line.rect))
                        .or(beside.map(|l| (l.text.clone(), l.rect))),
                );
                parts.extend(self.under(at).into_iter().map(|l| (l.text.clone(), l.rect)));
                let text = parts
                    .iter()
                    .map(|(text, _)| clean(field, text))
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                let place = parts.iter().map(|(_, rect)| *rect).reduce(union);
                if text.is_empty() {
                    tracer.note("nothing beside the label or under it");
                    return None;
                }
                let found = Found { value: text, place, checked: None };
                let read = parts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join(" | ");
                tracer.candidate(|| tried(&read, &found, ""));
                return Some(found);
            }
            let candidates = [(raw.as_str(), line.rect), (rest, line.rect)]
                .into_iter()
                .chain(beside.map(|l| (l.text.as_str(), l.rect)));
            let mut any = false;
            for (text, rect) in candidates.filter(|(text, _)| text.chars().any(char::is_alphanumeric)) {
                any = true;
                let found = shape(field, value_start(text), rect, self.patterns);
                let enough =
                    found.value.chars().filter(|c| c.is_alphanumeric()).count() >= if field.part > 0 { 1 } else { 2 };
                if found.checked != Some(false) && enough {
                    tracer.candidate(|| tried(text, &found, ""));
                    return Some(found);
                }
                tracer.candidate(|| {
                    tried(text, &found, if found.checked == Some(false) { "not a valid value" } else { "too short" })
                });
                failed = failed.or(Some(found).filter(|f| f.checked == Some(false)));
            }
            if !any {
                tracer.note("nothing after the label in its line or to its right");
            }
        }
        if !printed {
            tracer.note("none of its labels is on the page");
        }
        failed
    }

    /// The page as this config divides it: what each line was taken for, the values beside the codes, the zone.
    pub fn describe(&self, config: &Config) -> ScanTrace {
        let lines = self.page.lines.iter().enumerate().map(|(index, line)| LineTrace {
            index,
            text: line.text.clone(),
            rect: line.rect,
            confidence: line.confidence,
            role: match &self.roles[index] {
                Role::Value => "value",
                Role::Code(..) => "code",
                Role::Bracket => "bracket",
                Role::Item => "item",
                Role::Print => "print",
                Role::Zone => "zone",
            },
            code: match &self.roles[index] {
                Role::Code(code, _) => Some(code.clone()),
                _ => None,
            },
            labels: self.labels[index]
                .iter()
                .map(|hit| LabelTrace { field: config.fields[hit.field].key.clone(), start: hit.start, end: hit.end })
                .collect(),
        });
        let codes =
            self.codes.iter().map(|(code, rows)| (code.clone(), rows.iter().map(|(text, _)| text.clone()).collect()));
        let parts = self.mrz.iter().flat_map(|zone| &zone.parts);
        ScanTrace {
            lines: lines.collect(),
            codes: codes.collect(),
            mrz: parts
                .map(|p| ZoneTrace { name: p.name.to_string(), value: p.value.clone(), checked: p.checked })
                .collect(),
        }
    }

    /// The nearest line on the same row to the right that is not a label itself.
    fn beside(&self, at: usize) -> Option<&Line> {
        let line = &self.page.lines[at];
        let same_row = |other: &Line| (middle(&other.rect) - middle(&line.rect)).abs() < 0.7 * height(&line.rect);
        let lines = self
            .page
            .lines
            .iter()
            .enumerate()
            .filter(|(i, other)| *i != at && self.roles[*i] == Role::Value && same_row(other));
        let half = (line.rect[0] + line.rect[2]) / 2.0;
        lines
            .map(|(_, other)| other)
            .filter(|other| other.rect[0] > half)
            .min_by(|a, b| a.rect[0].total_cmp(&b.rect[0]))
    }

    /// The lines under a label that go on with its value: until the next label or the machine-readable zone.
    fn under(&self, at: usize) -> Vec<&Line> {
        let line = &self.page.lines[at];
        let mut below: Vec<usize> =
            (0..self.page.lines.len()).filter(|&i| middle(&self.page.lines[i].rect) > line.rect[3]).collect();
        below.sort_by(|&a, &b| middle(&self.page.lines[a].rect).total_cmp(&middle(&self.page.lines[b].rect)));
        // a value that goes on starts where its label does; what starts further in is another field
        let margin = 0.04 * self.page.width;
        below
            .into_iter()
            .map(|i| (i, &self.page.lines[i]))
            .take_while(|(i, l)| self.roles[*i] == Role::Value && (l.rect[0] - line.rect[0]).abs() < margin)
            .map(|(_, l)| l)
            .take(4)
            .collect()
    }
}

/// The labels of the config's fields in one line. Where two overlap the longer wins, so "Father's name" is not "name".
fn label_hits(skeleton: &[char], breaks: &[bool], fields: &[Field]) -> Vec<Hit> {
    let mut found: Vec<(usize, Hit)> = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        for alias in &field.label_of {
            let needle = fold::skeleton(alias);
            if let Some((_, start, end)) = fold::find_with(skeleton, &needle, slack(needle.len()), breaks) {
                found.push((needle.len(), Hit { field: index, start, end }));
            }
        }
    }
    found.sort_by_key(|(length, _)| std::cmp::Reverse(*length));
    let mut hits: Vec<Hit> = Vec::new();
    for (_, hit) in found {
        if hits.iter().all(|kept| hit.end <= kept.start || hit.start >= kept.end) {
            hits.push(hit);
        }
    }
    hits.sort_by_key(|hit| hit.start);
    hits
}

/// For each code on the page, the lines of its value: what follows it in its own line, then the lines on its row
/// to its right. A value belongs to the nearest code on its left. A code sits level with a one-line value and
/// mid-height of a two-line one, so a value may start a little above its code and end a line below it; boxes
/// come padded, so the reach is measured in the heights of both. A caption or a misread code on the left takes the
/// value away from the code before it: what follows "ЕГН / ID" is not the surname.
fn code_values(page: &Page, roles: &[Role], item_codes: &[String]) -> HashMap<String, Vec<(String, [f64; 4])>> {
    let codes: Vec<usize> = (0..page.lines.len()).filter(|&i| matches!(roles[i], Role::Code(..))).collect();
    let claims: Vec<usize> =
        (0..page.lines.len()).filter(|&i| matches!(roles[i], Role::Code(..) | Role::Print | Role::Bracket)).collect();
    let mut taken: Vec<Vec<usize>> = vec![Vec::new(); page.lines.len()];
    for (i, line) in page.lines.iter().enumerate() {
        if !matches!(roles[i], Role::Value | Role::Item) || line.confidence < MIN_CONFIDENCE {
            continue;
        }
        let near = |a: &usize| {
            let code = &page.lines[*a].rect;
            let down = middle(&line.rect) - middle(code);
            let reach = (height(code) + height(&line.rect)) * if down < 0.0 { 0.3 } else { 0.45 };
            let status = matches!(&roles[*a], Role::Code(code, _) if item_codes.contains(code));
            let fits = code[0] < line.rect[0] && down.abs() <= reach && (roles[i] == Role::Value || status);
            fits.then_some((down.abs() > 0.3 * height(code), -code[0], down.abs()))
        };
        let on_left = claims.iter().filter_map(|a| Some((*a, near(a)?)));
        if let Some((a, _)) = on_left.min_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal)) {
            taken[a].push(i);
        }
    }
    let mut values: HashMap<String, Vec<(String, [f64; 4])>> = HashMap::new();
    for a in codes {
        let Role::Code(code, inline) = &roles[a] else { continue };
        let mut lines = std::mem::take(&mut taken[a]);
        lines.sort_by(|&x, &y| middle(&page.lines[x].rect).total_cmp(&middle(&page.lines[y].rect)));
        // pieces of one printed line come as separate boxes on the same row
        let mut rows: Vec<(String, [f64; 4])> = Vec::new();
        if !inline.is_empty() {
            rows.push((inline.clone(), page.lines[a].rect));
        }
        for i in lines {
            let line = &page.lines[i];
            match rows
                .last_mut()
                .filter(|(_, rect)| (middle(rect) - middle(&line.rect)).abs() < 0.25 * height(&line.rect))
            {
                Some((text, rect)) => {
                    *text = if line.rect[0] < rect[0] {
                        format!("{} {text}", line.text)
                    } else {
                        format!("{text} {}", line.text)
                    };
                    *rect = union(*rect, line.rect);
                }
                None => rows.push((line.text.clone(), line.rect)),
            }
        }
        // "***" marks a field the certificate leaves empty
        let rows = rows
            .into_iter()
            .map(|(text, rect)| (TRANSLATED.replace(&text, "").trim().to_string(), rect))
            .filter(|(text, _)| !STARS.is_match(text));
        values.entry(code.clone()).or_default().extend(rows);
    }
    values
}

/// What the slash between the halves of a bilingual value is read as, when not as a slash.
fn slash_like(c: char) -> bool {
    matches!(c, '/' | '|' | 'I' | 'l' | '1' | '!')
}

/// The Cyrillic half of "БЪЛГАРИЯ/BGR" whose slash was read as a letter, "БЪЛГАРИЯIBGR": it ends where Latin
/// begins. A Latin letter that looks like a Cyrillic one may be either, so the half ends at the last thing that
/// could be the slash before the first letter that can only be Latin.
fn cyrillic_half(text: &str) -> &str {
    let only_latin = |c: char| c.is_ascii_alphabetic() && fold::cyrillic(&c.to_string()) == c.to_string();
    let Some((cut, _)) = text.char_indices().find(|(_, c)| only_latin(*c)) else { return text };
    let before = &text[..cut];
    let tail = before.trim_end_matches(|c: char| c.is_ascii_alphabetic() && !slash_like(c));
    let end = if tail.ends_with(slash_like) { tail.trim_end_matches(slash_like).len() } else { cut };
    // with nothing Cyrillic before it there is no half to cut off
    if !text[..end].is_ascii() {
        &text[..end]
    } else {
        text
    }
}

/// The Latin half of a bilingual value whose slash was read as a letter, "ЖIF": what follows the last Cyrillic
/// letter, without the slash.
fn latin_half(text: &str) -> &str {
    let after = text.char_indices().rfind(|(_, c)| !c.is_ascii()).map_or(0, |(at, c)| at + c.len_utf8());
    let half = text[after..].trim();
    let bare = half.trim_start_matches(slash_like);
    if bare.is_empty() {
        half
    } else {
        bare
    }
}

/// A text value as its field prints it: one half of a bilingual value, in the field's script.
fn clean(field: &Field, text: &str) -> String {
    let parted = text.contains(['/', '|']);
    let half = match field.part {
        0 if field.script == Some(Script::Cyrillic) => LATIN_HALF.replace_all(text, "").into_owned(),
        0 => text.to_string(),
        part if parted => text.split(['/', '|']).nth(part - 1).unwrap_or("").to_string(),
        1 if field.script == Some(Script::Cyrillic) => cyrillic_half(text).to_string(),
        2 if field.script == Some(Script::Latin) => latin_half(text).to_string(),
        _ => text.to_string(),
    };
    let folded = match field.script {
        Some(Script::Cyrillic) => fold::cyrillic(&half),
        Some(Script::Latin) => fold::latin(&half),
        None => half,
    };
    folded.trim_matches(|c: char| c.is_whitespace() || matches!(c, '/' | ',' | ':' | ';' | '-')).to_string()
}

/// The raw text as a value of the field's type, and whether it holds as one.
fn shape(field: &Field, raw: &str, rect: [f64; 4], patterns: &Patterns) -> Found {
    let found = |value: Option<String>| match value {
        Some(value) => Found { value, place: Some(rect), checked: Some(true) },
        None => Found { value: raw.trim().to_string(), place: Some(rect), checked: Some(false) },
    };
    if let Some(pattern) = &field.pattern {
        return found(patterns.extract(pattern, raw));
    }
    match field.field_type.as_str() {
        "date" => {
            let date = DATE.captures(raw).map(|c| format!("{}.{}.{}", &c[1], &c[2], &c[3]));
            found(date.filter(|date| crate::values::parse_date(date).is_some()))
        }
        "number" => found(NUMBER.find(raw).map(|m| m.as_str().to_string())),
        "egn" => found(patterns::egn(raw)),
        _ => Found { value: clean(field, raw), place: Some(rect), checked: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_read_through_ocr_noise() {
        let known: Vec<String> =
            ["A", "C11", "C23", "F1", "G", "J", "O1", "S2"].iter().map(|c| c.to_string()).collect();
        let code = |text: &str| anchor(text, &known);
        assert_eq!(code("(F.1)"), Some(("F1".into(), "".into())));
        assert_eq!(code("(F1)1732"), Some(("F1".into(), "1732".into())));
        assert_eq!(code("(J) M1"), Some(("J".into(), "M1".into())));
        assert_eq!(code("(0.1)"), Some(("O1".into(), "".into())), "a zero for the letter O");
        assert_eq!(code("($.2)"), Some(("S2".into(), "".into())));
        assert_eq!(code("(G.1.1.)"), Some(("C11".into(), "".into())), "G with digits can only be C");
        assert_eq!(code("(C23"), Some(("C23".into(), "".into())), "the closing bracket was lost");
        assert_eq!(code("F.1) 1732"), Some(("F1".into(), "1732".into())), "the opening bracket was lost");
        assert_eq!(code("ГАЗ (LPG)"), None);
        assert_eq!(code("(G) 1200"), Some(("G".into(), "1200".into())));
        assert_eq!(code("(a) СОБСТВЕНИК"), None, "a lowercase letter is a list item, not a code");
        assert_eq!(code("(Δ)"), None);
        assert_eq!(code("1732"), None);
    }

    #[test]
    fn a_bilingual_value_parts_where_the_script_changes_when_its_slash_was_misread() {
        assert_eq!(cyrillic_half("БЪЛГАРИЯIBGP"), "БЪЛГАРИЯ", "the slash read as the letter I");
        assert_eq!(cyrillic_half("БЪЛГАРИЯ1BGR"), "БЪЛГАРИЯ", "as the digit one, before a letter both scripts have");
        assert_eq!(cyrillic_half("СОФИЯlSOFIA"), "СОФИЯ");
        assert_eq!(cyrillic_half("БЪЛГAPИЯIBGR"), "БЪЛГAPИЯ", "Latin look-alikes inside the word stay in it");
        assert_eq!(cyrillic_half("КАФЯВИ"), "КАФЯВИ");
        assert_eq!(cyrillic_half("BGR"), "BGR", "nothing Cyrillic: kept whole");
        assert_eq!(latin_half("ЖIF"), "F");
        assert_eq!(latin_half("ЖF"), "F");
        assert_eq!(latin_half("M"), "M");
    }

    #[test]
    fn a_value_starts_at_its_capitals() {
        assert_eq!(value_start("of bith СОФИЯ/SOFIA"), "СОФИЯ/SOFIA");
        assert_eq!(value_start("168"), "168");
        assert_eq!(value_start("K/F"), "K/F", "nothing to go by: kept whole");
    }
}
