//! T2: per-language label packs. A label claims the T1 hit after it ("ЕГН 7501020018") or delimits a value by
//! a grammar ("с адрес гр. София, ул. ...").

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;

use super::grammar;
use super::types::{Detection, Kind, Source};

const PACK_FILES: &[&str] = &[include_str!("packs/bg.yaml"), include_str!("packs/en.yaml")];

/// How far after its label a claimed value may start.
const WINDOW: usize = 48;
const ANCHORED: f32 = 0.98;
/// A label with something ID-like after it that no recognizer validated: redacted, not suggested.
const UNCHECKED: f32 = 0.5;
const ROLE_PERSON: f32 = 0.8;
const PERSON_BEFORE: f32 = 0.85;

#[derive(Deserialize)]
struct PackFile {
    lang: String,
    #[serde(default)]
    months: Vec<String>,
    anchors: Vec<AnchorFile>,
    #[serde(default)]
    person_before: Vec<String>,
    #[serde(default)]
    glue: Vec<String>,
    address_tokens: Vec<String>,
    address_starts: Vec<String>,
    org_suffixes: Vec<String>,
    /// Party words ("продавач", "landlord"): they end a name or an address, and "ПРОДАВАЧ: <name>" is a person.
    roles: IndexMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct AnchorFile {
    kind: String,
    labels: Vec<String>,
    value: ValueFile,
    #[serde(default)]
    include_label: bool,
}

#[derive(Deserialize)]
struct ValueFile {
    pattern: Option<String>,
    grammar: Option<String>,
    regex: Option<String>,
    max: Option<usize>,
}

pub enum Value {
    Pattern(String),
    Grammar(String, usize),
    Regex(Regex),
}

pub struct Anchor {
    pub kind: Kind,
    pub value: Value,
    pub include_label: bool,
}

pub struct Pack {
    labels: Regex,
    anchor_of: HashMap<String, usize>,
    anchors: Vec<Anchor>,
    labels_at: Regex,
    person_before: Option<Regex>,
    glue: Vec<String>,
    suffixes: HashSet<String>,
    pub address_tokens: HashSet<String>,
    address_starts: Regex,
    orgs: Vec<(Regex, f32)>,
    dates: Option<Regex>,
    pub role_any: Regex,
    role_at: Regex,
}

/// Lowercase with single spaces, the form labels are looked up in.
pub fn key(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn alternation(words: &[String]) -> String {
    let mut sorted: Vec<&String> = words.iter().collect();
    sorted.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    sorted.iter().map(|w| regex::escape(&key(w)).replace(' ', r"\s+")).collect::<Vec<_>>().join("|")
}

/// Words matched case-insensitively, not inside a longer word. Group 1 is the word.
fn words_re(words: &[String]) -> Regex {
    Regex::new(&format!(r"(?i)(?:^|[^\p{{L}}\p{{N}}])({})", alternation(words))).unwrap()
}

/// The match does not run into a letter or digit ("адрес" is not a label inside "адреса").
pub fn ends_word(text: &str, end: usize) -> bool {
    let last = text[..end].chars().next_back();
    let next = text[end..].chars().next();
    match (last, next) {
        (Some(l), Some(n)) => !(l.is_alphanumeric() && n.is_alphanumeric()),
        _ => true,
    }
}

/// Matches of a `words_re` regex as (start, end) of the word itself.
pub fn find_words<'a>(re: &'a Regex, text: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
    re.captures_iter(text)
        .filter_map(move |c| c.get(1))
        .map(|m| (m.start(), m.end()))
        .filter(|&(_, e)| ends_word(text, e))
}

impl Pack {
    fn compile(file: PackFile) -> Pack {
        let mut anchor_of = HashMap::new();
        let mut labels = Vec::new();
        let mut anchors = Vec::new();
        for (i, a) in file.anchors.into_iter().enumerate() {
            let kind =
                Kind::from_name(&a.kind).unwrap_or_else(|| panic!("pack {}: unknown kind {}", file.lang, a.kind));
            for label in &a.labels {
                anchor_of.entry(key(label)).or_insert(i);
            }
            labels.extend(a.labels);
            let value = match (a.value.pattern, a.value.grammar, a.value.regex) {
                (Some(p), _, _) => Value::Pattern(p),
                (_, Some(g), _) => Value::Grammar(g, a.value.max.unwrap_or(60)),
                (_, _, Some(r)) => Value::Regex(Regex::new(&format!("^(?:{r})")).unwrap()),
                _ => panic!("pack {}: anchor {} has no value", file.lang, a.kind),
            };
            anchors.push(Anchor { kind, value, include_label: a.include_label });
        }
        let suffixes = file.org_suffixes.iter().map(|s| regex::escape(s)).collect::<Vec<_>>().join("|");
        let quoted = r#"„[^“”"„\n]{1,60}[“”"]|“[^”\n]{1,60}”|"[^"\n]{1,60}"|«[^»\n]{1,60}»"#;
        let capitalized = r"\p{Lu}[\p{L}\d&.\-]*(?:[ ]\p{Lu}[\p{L}\d&.\-]*){0,3}";
        let orgs = vec![
            (Regex::new(&format!(r"(?:{quoted})[ \u{{a0}}]*(?:{suffixes})\b\.?")).unwrap(), 0.9),
            (Regex::new(&format!(r"\b(?:{suffixes})[ \u{{a0}}]+(?:{quoted})")).unwrap(), 0.9),
            (Regex::new(&format!(r"{capitalized}[ \u{{a0}}]+(?:{suffixes})\b\.?")).unwrap(), 0.75),
        ];
        let dates = (!file.months.is_empty())
            .then(|| Regex::new(&format!(r"(?i)\b\d{{1,2}}\s+(?:{})\s+\d{{4}}\b", alternation(&file.months))).unwrap());
        let all_stems: Vec<String> = file.roles.values().flatten().cloned().collect();
        Pack {
            labels_at: Regex::new(&format!("(?i)^(?:{})", alternation(&labels))).unwrap(),
            labels: words_re(&labels),
            anchor_of,
            anchors,
            person_before: (!file.person_before.is_empty()).then(|| words_re(&file.person_before)),
            glue: file.glue.iter().map(|g| key(g)).collect(),
            suffixes: file.org_suffixes.iter().cloned().collect(),
            address_tokens: file.address_tokens.iter().map(|t| key(t)).collect(),
            address_starts: words_re(&file.address_starts),
            orgs,
            dates,
            role_any: Regex::new(&format!(r"(?i)(?:^|[^\p{{L}}])(({})\p{{L}}*)", alternation(&all_stems))).unwrap(),
            role_at: Regex::new(&format!(r"(?i)^(?:{})\p{{L}}*", alternation(&all_stems))).unwrap(),
        }
    }

    /// A label or a role word starts exactly at `at`: it ends a name or an address.
    pub fn stops_at(&self, text: &str, at: usize) -> bool {
        let rest = &text[at..];
        [&self.labels_at, &self.role_at].iter().any(|re| re.find(rest).is_some_and(|m| ends_word(rest, m.end())))
    }

    pub fn is_suffix(&self, word: &str) -> bool {
        self.suffixes.contains(word)
    }
}

pub static PACKS: LazyLock<Vec<Pack>> = LazyLock::new(|| {
    PACK_FILES.iter().map(|text| Pack::compile(serde_yaml::from_str(text).expect("pii pack"))).collect()
});

fn skip_separators(text: &str, from: usize) -> usize {
    from + text[from..].len()
        - text[from..].trim_start_matches(|c: char| c.is_whitespace() || ":-–—№.(".contains(c)).len()
}

/// The label is followed by a fill-in blank ("ЕГН ..........."): nothing to claim.
fn blank_after(text: &str, from: usize) -> bool {
    let rest = text[from..].trim_start_matches(|c: char| c.is_whitespace() || ":-–—№".contains(c));
    rest.starts_with("...") || rest.starts_with('…') || rest.starts_with("___")
}

/// Something ID-like right after a label that no recognizer validated, so redaction still covers it.
fn id_like(text: &str, from: usize) -> Option<(usize, usize)> {
    static ID_LIKE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z0-9]{2,}(?:[ \-/]?[A-Z0-9]{2,})*").unwrap());
    let m = ID_LIKE.find(&text[from..])?;
    m.as_str().chars().any(|c| c.is_ascii_digit()).then(|| (from + m.start(), from + m.end()))
}

fn id_kind(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Egn
            | Kind::Lnch
            | Kind::Eik
            | Kind::IdCard
            | Kind::Passport
            | Kind::Ssn
            | Kind::Iban
            | Kind::VatId
            | Kind::Nino
    )
}

/// Adds T2 detections and lifts the T1 hits that labels claim.
pub fn scan(text: &str, t1: &mut Vec<Detection>) -> Vec<Detection> {
    let mut out = Vec::new();
    for pack in PACKS.iter() {
        if let Some(dates) = &pack.dates {
            for m in dates.find_iter(text) {
                t1.push(Detection {
                    kind: Kind::Date,
                    start: m.start(),
                    end: m.end(),
                    score: 0.9,
                    source: Source::Pattern("date"),
                });
            }
        }
    }
    for pack in PACKS.iter() {
        for (ls, le) in find_words(&pack.labels, text) {
            let label = &text[ls..le];
            let Some(&i) = pack.anchor_of.get(&key(label)) else { continue };
            let anchor = &pack.anchors[i];
            if blank_after(text, le) {
                continue;
            }
            let from = skip_separators(text, le);
            let source = Source::Anchor(label.to_string());
            match &anchor.value {
                Value::Pattern(id) => {
                    let claimed = t1
                        .iter_mut()
                        .filter(|d| d.start >= from && d.start <= le + WINDOW)
                        .filter(|d| d.kind == anchor.kind || matches!(d.source, Source::Pattern(p) if p == id))
                        .min_by_key(|d| (d.start, std::cmp::Reverse(d.score.to_bits())));
                    match claimed {
                        Some(d) => {
                            d.kind = anchor.kind;
                            d.score = d.score.max(ANCHORED);
                            d.source = source;
                        }
                        None if id_kind(anchor.kind) => {
                            if let Some((s, e)) = id_like(text, from) {
                                out.push(Detection { kind: anchor.kind, start: s, end: e, score: UNCHECKED, source });
                            }
                        }
                        None => {}
                    }
                }
                Value::Grammar(name, max) => {
                    if let Some((s, e, score)) = grammar::take(name, text, from, *max, pack) {
                        let s = if anchor.include_label { ls } else { s };
                        out.push(Detection { kind: anchor.kind, start: s, end: e, score, source });
                    }
                }
                Value::Regex(re) => {
                    if let Some(m) = re.find(&text[from..]) {
                        out.push(Detection { kind: anchor.kind, start: from, end: from + m.end(), score: 0.9, source });
                    }
                }
            }
        }
        out.extend(role_people(text, pack));
        out.extend(people_before(text, pack));
        out.extend(orgs(text, pack));
        out.extend(grammar::address_chains(text, pack, &pack.address_starts));
    }
    out
}

/// "ПРОДАВАЧ: Иван Петров Иванов"
fn role_people(text: &str, pack: &Pack) -> Vec<Detection> {
    let mut out = Vec::new();
    for c in pack.role_any.captures_iter(text) {
        let m = c.get(1).unwrap();
        let rest = &text[m.end()..];
        let after = rest.trim_start_matches(|c: char| c == ' ' || "“”\"»".contains(c));
        let Some(after) = after.strip_prefix([':', '-', '–', '—']) else { continue };
        let from = m.end() + rest.len() - after.len();
        let from = from + after.len() - after.trim_start().len();
        if let Some((s, e)) = grammar::person_name(text, from, pack) {
            out.push(Detection {
                kind: Kind::Person,
                start: s,
                end: e,
                score: ROLE_PERSON,
                source: Source::Anchor(m.as_str().to_string()),
            });
        }
    }
    out
}

/// "Иван Петров Иванов, ЕГН ..."
fn people_before(text: &str, pack: &Pack) -> Vec<Detection> {
    let Some(re) = &pack.person_before else { return Vec::new() };
    let mut out = Vec::new();
    for (ls, le) in find_words(re, text) {
        let mut left = text[..ls].trim_end();
        if let Some((head, last)) = left.rsplit_once(char::is_whitespace) {
            if pack.glue.contains(&last.to_lowercase()) {
                left = head.trim_end();
            }
        }
        let left = left.trim_end_matches(',').trim_end();
        if let Some((s, e)) = grammar::person_name_before(text, left.len(), pack) {
            out.push(Detection {
                kind: Kind::Person,
                start: s,
                end: e,
                score: PERSON_BEFORE,
                source: Source::Anchor(text[ls..le].to_string()),
            });
        }
    }
    out
}

fn orgs(text: &str, pack: &Pack) -> Vec<Detection> {
    pack.orgs
        .iter()
        .flat_map(|(re, score)| {
            re.find_iter(text).map(|m| Detection {
                kind: Kind::Org,
                start: m.start(),
                end: m.end(),
                score: *score,
                source: Source::Suffix,
            })
        })
        .collect()
}
