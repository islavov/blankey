//! From raw hits to entities: overlaps, one entity per distinct value, and the party each entity belongs to.

use std::collections::HashMap;

use indexmap::IndexMap;

use regex::Regex;

use super::t2::{ends_word, Pack, PACKS};
use super::types::{Analysis, Block, Detection, Entity, Kind, Occ, Source};
use super::REDACT;

/// Shorter values are not searched elsewhere: too many accidental matches.
const ECHO_MIN: usize = 4;

/// Blocks after a role heading ("ПРОДАВАЧ:" alone on its line) whose parties take that role.
const HEADING_REACH: usize = 3;

fn rank(source: &Source) -> u8 {
    match source {
        Source::Anchor(_) => 3,
        Source::Pattern(_) => 2,
        Source::Suffix => 1,
        Source::Chain | Source::Echo => 0,
    }
}

/// Keeps the best of overlapping hits: higher score, then a label over a bare pattern, then the longer span.
pub fn resolve(mut hits: Vec<Detection>) -> Vec<Detection> {
    hits.retain(|d| d.score > 0.0 && d.end > d.start);
    hits.sort_by(|a, b| {
        b.score.total_cmp(&a.score).then(rank(&b.source).cmp(&rank(&a.source))).then(b.width().cmp(&a.width()))
    });
    let mut kept: Vec<Detection> = Vec::new();
    for d in hits {
        if !kept.iter().any(|k| k.overlaps(&d)) {
            kept.push(d);
        }
    }
    kept.sort_by_key(|d| d.start);
    kept
}

/// The form two spellings of one value share.
pub fn normalize(kind: Kind, surface: &str) -> String {
    match kind {
        Kind::Egn | Kind::Lnch | Kind::Eik | Kind::IdCard | Kind::Ssn | Kind::Phone | Kind::Postcode => {
            surface.chars().filter(char::is_ascii_digit).collect()
        }
        Kind::Iban | Kind::Bic | Kind::VatId | Kind::Passport | Kind::Nino | Kind::Plate => {
            surface.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
        }
        _ => surface
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(['„', '“', '”', '«', '»'], "\"")
            .trim_matches(|c: char| c.is_ascii_punctuation() && c != '"')
            .to_lowercase(),
    }
}

/// One entity per (kind, normalized value), numbered per kind in order of first appearance.
pub fn group(blocks: &[Block], per_block: Vec<Vec<Detection>>) -> Analysis {
    let mut by_key: IndexMap<(Kind, String), Entity> = IndexMap::new();
    let mut entity_of = Vec::new();
    for (b, dets) in per_block.iter().enumerate() {
        let mut row = Vec::new();
        for d in dets {
            let surface = &blocks[b].text[d.start..d.end];
            let entry = by_key.entry((d.kind, normalize(d.kind, surface)));
            row.push(entry.index());
            let e = entry.or_insert_with_key(|(kind, norm)| Entity {
                id: String::new(),
                kind: *kind,
                norm: norm.clone(),
                surfaces: Vec::new(),
                occurrences: Vec::new(),
                role: None,
                label: None,
                score: 0.0,
                crosses: false,
            });
            if !e.surfaces.iter().any(|s| s == surface) {
                e.surfaces.push(surface.to_string());
            }
            e.occurrences.push(Occ { block: b, start: d.start, end: d.end });
            e.score = e.score.max(d.score);
            e.crosses |= blocks[b].crosses(d.start, d.end);
            if e.label.is_none() {
                e.label = d.label().map(|l| l.trim().to_string());
            }
        }
        entity_of.push(row);
    }
    let mut counts: HashMap<Kind, usize> = HashMap::new();
    let mut entities: Vec<Entity> = by_key.into_values().collect();
    for e in &mut entities {
        let n = counts.entry(e.kind).or_default();
        *n += 1;
        e.id = format!("{}:{n}", e.kind.name());
    }
    Analysis { per_block, entity_of, entities }
}

/// A value found once is found everywhere: the name detected after "represented by" also in the signature line,
/// in any letter case.
pub fn propagate(blocks: &[Block], a: &mut Analysis) {
    let mut found: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (i, e) in a.entities.iter().enumerate() {
        if !e.kind.personal() || e.score < REDACT {
            continue;
        }
        for surface in e.surfaces.iter().filter(|s| s.chars().count() >= ECHO_MIN) {
            let Ok(re) = Regex::new(&format!("(?i){}", regex::escape(surface))) else { continue };
            for (b, block) in blocks.iter().enumerate() {
                for m in re.find_iter(&block.text) {
                    let (s, end) = (m.start(), m.end());
                    let word = ends_word(&block.text, end) && (s == 0 || ends_word(&block.text, s));
                    let taken = a.per_block[b].iter().any(|d| d.start < end && s < d.end)
                        || found.iter().any(|&(fb, fs, fe, _)| fb == b && fs < end && s < fe);
                    if word && !taken {
                        found.push((b, s, end, i));
                    }
                }
            }
        }
    }
    for (b, start, end, i) in found {
        let e = &mut a.entities[i];
        let d = Detection { kind: e.kind, start, end, score: e.score, source: Source::Echo };
        let at = a.per_block[b].partition_point(|x| x.start < start);
        a.per_block[b].insert(at, d);
        a.entity_of[b].insert(at, i);
        let surface = &blocks[b].text[start..end];
        if !e.surfaces.iter().any(|s| s == surface) {
            e.surfaces.push(surface.to_string());
        }
        e.occurrences.push(Occ { block: b, start, end });
        e.occurrences.sort_by_key(|o| (o.block, o.start));
        e.crosses |= blocks[b].crosses(start, end);
    }
}

enum Event {
    /// "..., наричан по-долу ПРОДАВАЧ": the parties before it take the role.
    Closing(String),
    /// "ПРОДАВАЧ: ...": the parties after it take the role.
    Opening(String),
    /// A role named in passing; used when nothing else assigns one.
    Plain(String),
}

/// The term a defined-term phrase introduces: quoted ("„Довереник“") or one capitalized word.
fn defined(rest: &str) -> Option<&str> {
    let term = match rest.strip_prefix(['„', '"', '«', '“', '(']) {
        Some(inner) => inner[..inner.find(['”', '“', '"', '»', ')'])?].trim(),
        None => rest
            .split(|c: char| !c.is_alphanumeric() && c != '-')
            .next()
            .filter(|w| w.starts_with(char::is_uppercase))?,
    };
    (term.chars().count() >= 2 && term.chars().count() <= 30).then_some(term)
}

const TRANSLIT: &[(char, &str)] = &[
    ('а', "a"),
    ('б', "b"),
    ('в', "v"),
    ('г', "g"),
    ('д', "d"),
    ('е', "e"),
    ('ж', "zh"),
    ('з', "z"),
    ('и', "i"),
    ('й', "y"),
    ('к', "k"),
    ('л', "l"),
    ('м', "m"),
    ('н', "n"),
    ('о', "o"),
    ('п', "p"),
    ('р', "r"),
    ('с', "s"),
    ('т', "t"),
    ('у', "u"),
    ('ф', "f"),
    ('х', "h"),
    ('ц', "ts"),
    ('ч', "ch"),
    ('ш', "sh"),
    ('щ', "sht"),
    ('ъ', "a"),
    ('ь', "y"),
    ('ю', "yu"),
    ('я', "ya"),
];

/// A var-safe id for a defined term the packs do not know: "Довереник" -> "doverenik".
pub fn slug(term: &str) -> String {
    let mut out = String::new();
    for c in term.to_lowercase().chars() {
        match TRANSLIT.iter().find(|(k, _)| *k == c) {
            Some((_, latin)) => out.push_str(latin),
            None if c.is_ascii_alphanumeric() => out.push(c),
            None if !out.ends_with('_') && !out.is_empty() => out.push('_'),
            None => {}
        }
    }
    out.trim_end_matches('_').to_string()
}

fn role_word(pack: &Pack, text: &str, at: usize) -> Option<(String, usize)> {
    let rest = &text[at..];
    let rest = rest.strip_prefix("the ").or_else(|| rest.strip_prefix("The ")).unwrap_or(rest);
    let offset = text.len() - at - rest.len();
    let m = pack.role_any.captures(rest).and_then(|c| c.get(1)).filter(|m| m.start() == 0)?;
    let role = pack.role_of(m.as_str())?;
    Some((role.to_string(), at + offset + m.end()))
}

fn events(text: &str) -> Vec<(usize, Event)> {
    let mut out = Vec::new();
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for pack in PACKS.iter() {
        for m in pack.defined_term.find_iter(text) {
            let after = &text[m.end()..];
            let skip =
                after.len() - after.trim_start_matches(|c: char| c.is_whitespace() || "„\"«“(".contains(c)).len();
            let at = m.end() + skip;
            if let Some((role, end)) = role_word(pack, text, at) {
                taken.push((at, end));
                out.push((m.start(), Event::Closing(role)));
            } else if let Some(term) = defined(after.trim_start()) {
                out.push((m.start(), Event::Closing(slug(term))));
            }
        }
        for c in pack.role_any.captures_iter(text) {
            let m = c.get(1).unwrap();
            if !ends_word(text, m.end()) || taken.iter().any(|&(s, e)| m.start() >= s && m.start() < e) {
                continue;
            }
            let Some(role) = pack.role_of(m.as_str()) else { continue };
            let after = text[m.end()..].trim_start_matches(|c: char| c == ' ' || "“”\"»".contains(c));
            if after.starts_with([':', '-', '–', '—']) {
                out.push((m.start(), Event::Opening(role.to_string())));
            } else {
                out.push((m.start(), Event::Plain(role.to_string())));
            }
        }
    }
    out.sort_by_key(|(at, _)| *at);
    out
}

/// A block that is nothing but a role: "ПРОДАВАЧ:", "The Landlord".
fn heading_role(text: &str) -> Option<String> {
    let t = text.trim().trim_matches(|c: char| !c.is_alphanumeric());
    PACKS.iter().find_map(|pack| role_word(pack, t, 0).filter(|(_, end)| *end == t.len()).map(|(role, _)| role))
}

/// Gives each personal entity the role of the party it belongs to; parties with no role become party_1, party_2.
pub fn assign_roles(blocks: &[Block], a: &mut Analysis) {
    let mut roles: Vec<Option<String>> = vec![None; a.entities.len()];
    let mut orphans: Vec<Vec<usize>> = Vec::new();
    let (mut heading, mut reach) = (None::<String>, 0);
    for (b, block) in blocks.iter().enumerate() {
        if let Some(role) = heading_role(&block.text) {
            heading = Some(role);
            reach = HEADING_REACH + 1;
        }
        let mut items: Vec<(usize, Option<Event>, Option<usize>)> =
            events(&block.text).into_iter().map(|(at, e)| (at, Some(e), None)).collect();
        for (i, d) in a.per_block[b].iter().enumerate() {
            if d.kind.personal() {
                items.push((d.start, None, Some(a.entity_of[b][i])));
            }
        }
        items.sort_by_key(|(at, _, _)| *at);
        let (mut open, mut pending, mut plain) = (None::<String>, Vec::new(), Vec::<String>::new());
        let set = |e: usize, role: &str, roles: &mut Vec<Option<String>>| {
            roles[e].get_or_insert_with(|| role.to_string());
        };
        for (_, event, entity) in items {
            match (event, entity) {
                (_, Some(e)) => match &open {
                    Some(role) => set(e, role, &mut roles),
                    None => pending.push(e),
                },
                (Some(Event::Closing(role)), _) => {
                    pending.drain(..).for_each(|e| set(e, &role, &mut roles));
                    open = None;
                }
                (Some(Event::Opening(role)), _) => open = Some(role),
                (Some(Event::Plain(role)), _) => plain.push(role),
                _ => {}
            }
        }
        plain.dedup();
        if !pending.is_empty() {
            match (&heading, plain.as_slice()) {
                (Some(role), _) if reach > 0 => pending.iter().for_each(|&e| set(e, role, &mut roles)),
                (_, [role]) => pending.iter().for_each(|&e| set(e, role, &mut roles)),
                _ => orphans.push(pending),
            }
        }
        reach = reach.saturating_sub(1);
    }
    let mut n = 0;
    for group in orphans {
        let unassigned: Vec<usize> = group.into_iter().filter(|&e| roles[e].is_none()).collect();
        if !unassigned.is_empty() {
            n += 1;
            unassigned.iter().for_each(|&e| roles[e] = Some(format!("party_{n}")));
        }
    }
    for (e, role) in a.entities.iter_mut().zip(roles) {
        if e.kind.personal() {
            e.role = role;
        }
    }
}
