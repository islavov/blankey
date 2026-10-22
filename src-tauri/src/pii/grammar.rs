//! Value grammars for T2: where a name, an address or a free value after a label ends.

use std::sync::LazyLock;

use regex::Regex;

use super::t2::{find_words, key, Pack};
use super::types::{Detection, Kind, Source};
use super::validate::phone_digits;

static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\p{Lu}\p{Ll}+(?:-\p{Lu}\p{Ll}+)?|\p{Lu}{2,}(?:-\p{Lu}{2,})?)$").unwrap());
static INITIAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{Lu}\.$").unwrap());
static PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+\d][\d \-()/]{4,18}\d").unwrap());

const NAME_TOKENS: usize = 4;
const ADDRESS_MAX: usize = 160;
const ADDRESS_BACK: usize = 3;
const PERSON: f32 = 0.88;
const ADDRESS: f32 = 0.9;
const CHAIN: f32 = 0.8;
const UNTIL_DELIM: f32 = 0.8;
const PHONE_SCORE: f32 = 0.9;

/// Whitespace-separated tokens from `from` on, as byte ranges.
fn tokens(text: &str, from: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut at = from;
    std::iter::from_fn(move || {
        let rest = &text[at..];
        let s = at + rest.len() - rest.trim_start().len();
        if s >= text.len() {
            return None;
        }
        let e = text[s..].find(char::is_whitespace).map_or(text.len(), |i| s + i);
        at = e;
        Some((s, e))
    })
}

/// A blank line between two tokens ends any value.
fn paragraph_break(gap: &str) -> bool {
    gap.matches('\n').count() > 1
}

pub fn take(grammar: &str, text: &str, from: usize, max: usize, pack: &Pack) -> Option<(usize, usize, f32)> {
    match grammar {
        "person_name" => person_name(text, from, pack).map(|(s, e)| (s, e, PERSON)),
        "address" => {
            address(text, from, pack).filter(|a| a.has_token || a.has_digit).map(|a| (a.start, a.end, ADDRESS))
        }
        "until_delim" => until_delim(text, from, max).map(|(s, e)| (s, e, UNTIL_DELIM)),
        "phone_any" => {
            let m = PHONE.find(&text[from..]).filter(|m| phone_digits(m.as_str()))?;
            Some((from, from + m.end(), PHONE_SCORE))
        }
        other => panic!("unknown pii grammar {other}"),
    }
}

/// Is `token` (without trailing punctuation) a part of a name? Initials count, but not as a full name part.
fn name_part(token: &str, pack: &Pack) -> Option<bool> {
    if INITIAL.is_match(token) {
        return Some(false);
    }
    (NAME.is_match(token) && !pack.is_suffix(token)).then_some(true)
}

/// Two to four capitalized words from `from`: "Иван Петров Иванов", "IVAN P. PETROV".
pub fn person_name(text: &str, from: usize, pack: &Pack) -> Option<(usize, usize)> {
    let (mut start, mut end, mut prev) = (None, from, from);
    let (mut count, mut full) = (0, 0);
    for (s, e) in tokens(text, from) {
        if count == NAME_TOKENS || paragraph_break(&text[prev..s]) || pack.stops_at(text, s) {
            break;
        }
        let token = &text[s..e];
        let core = token.trim_end_matches(|c: char| ",;:)".contains(c));
        let (core, is_full) = match name_part(core, pack) {
            Some(is_full) => (core, is_full),
            None => {
                let bare = core.trim_end_matches('.');
                match name_part(bare, pack) {
                    Some(true) => (bare, true),
                    _ => break,
                }
            }
        };
        count += 1;
        full += is_full as usize;
        start.get_or_insert(s);
        end = s + core.len();
        prev = e;
        if core.len() < token.len() {
            break;
        }
    }
    (full >= 2).then_some((start?, end))
}

/// The same, ending at `end` and read backwards: the name right before "ЕГН".
pub fn person_name_before(text: &str, end: usize, pack: &Pack) -> Option<(usize, usize)> {
    let all: Vec<(usize, usize)> = tokens(&text[..end], 0).collect();
    let (mut start, mut count, mut full, mut next) = (None, 0, 0, end);
    for &(s, e) in all.iter().rev() {
        let token = &text[s..e];
        if count == NAME_TOKENS || paragraph_break(&text[e..next]) || pack.stops_at(text, s) {
            break;
        }
        let Some(is_full) = name_part(token, pack) else { break };
        count += 1;
        full += is_full as usize;
        start = Some(s);
        next = s;
    }
    (full >= 2).then_some((start?, end))
}

pub struct Address {
    pub start: usize,
    pub end: usize,
    pub has_token: bool,
    pub has_digit: bool,
    tokens: usize,
}

/// Street, city and building tokens, names and numbers from `from` until a label, a lowercase word or `;`.
pub fn address(text: &str, from: usize, pack: &Pack) -> Option<Address> {
    let mut a = Address { start: from, end: from, has_token: false, has_digit: false, tokens: 0 };
    let mut prev = from;
    for (s, e) in tokens(text, from) {
        if a.tokens > 0 && (paragraph_break(&text[prev..s]) || pack.stops_at(text, s) || s - a.start > ADDRESS_MAX) {
            break;
        }
        let token = &text[s..e];
        let t = token.trim_end_matches([',', ';', ')']);
        let Some(first) = t.chars().next() else { break };
        let is_token = pack.address_tokens.contains(&key(t)) || first == '№';
        if !(is_token || first.is_ascii_digit() || first.is_uppercase() || "„\"«(-–".contains(first)) {
            break;
        }
        let mut stop = token.ends_with(';');
        let mut t_end = s + t.len();
        if !is_token && t.ends_with('.') {
            t_end -= 1;
            stop = true;
        }
        if a.tokens == 0 {
            a.start = s;
        }
        a.tokens += 1;
        a.end = t_end;
        a.has_token |= is_token;
        a.has_digit |= t.chars().any(|c| c.is_ascii_digit());
        prev = e;
        if stop {
            break;
        }
    }
    (a.tokens > 0).then_some(a)
}

/// Addresses with no label: two or more tokens from a strong start ("гр. София, ул. ..."), also extended back over
/// a house number and street name ("12 Main Street").
pub fn address_chains(text: &str, pack: &Pack, starts: &Regex) -> Vec<Detection> {
    let mut out: Vec<Detection> = Vec::new();
    let mut covered = 0;
    for (s, _) in find_words(starts, text) {
        if s < covered {
            continue;
        }
        let Some(mut a) = address(text, s, pack).filter(|a| a.tokens >= 2) else { continue };
        let before: Vec<(usize, usize)> = tokens(&text[..s], 0).collect();
        let mut back = Vec::new();
        for &(ts, te) in before.iter().rev().take(ADDRESS_BACK) {
            let token = &text[ts..te];
            let first = token.chars().next().unwrap_or(' ');
            if !(first.is_ascii_digit() || first.is_uppercase())
                || token.ends_with([',', ';', '.', ':'])
                || pack.stops_at(text, ts)
            {
                break;
            }
            back.push(ts);
        }
        if let Some(&digit_at) = back.iter().rev().find(|&&ts| text[ts..].starts_with(|c: char| c.is_ascii_digit())) {
            a.start = digit_at;
            a.has_digit = true;
        }
        if a.has_token && a.has_digit {
            covered = a.end;
            out.push(Detection {
                kind: Kind::Address,
                start: a.start,
                end: a.end,
                score: CHAIN,
                source: Source::Chain,
            });
        }
    }
    out
}

fn until_delim(text: &str, from: usize, max: usize) -> Option<(usize, usize)> {
    let rest = &text[from..];
    let mut end = rest.find([',', ';', '\n']).unwrap_or(rest.len());
    if let Some((i, _)) = rest.char_indices().nth(max) {
        end = end.min(i);
    }
    let value = rest[..end].trim_end_matches(|c: char| c.is_whitespace() || c == '.');
    (!value.trim().is_empty()).then_some((from, from + value.len()))
}
