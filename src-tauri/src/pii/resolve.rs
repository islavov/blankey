//! From raw hits to entities: overlaps, then one entity per distinct value.

use indexmap::IndexMap;
use regex::Regex;

use super::t2::ends_word;
use super::types::{Analysis, Block, Detection, Entity, Kind, Occ, Source};
use super::REDACT;

/// Shorter values are not searched elsewhere: too many accidental matches.
const ECHO_MIN: usize = 4;

fn rank(source: &Source) -> u8 {
    match source {
        Source::User => 4,
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
            .replace(['„', '“', '”', '«', '»', '"'], "")
            .trim_matches(|c: char| c.is_ascii_punctuation())
            .to_lowercase(),
    }
}

/// One entity per value, whatever kind detected it: "Васил Стоянов Йовчев" as a party and as a company's manager is
/// one value. Its kind is internal, personal when any of its detections is. Personal values are numbered pii:1,
/// pii:2 in order of first appearance; the others (dates, amounts) v:1, v:2.
pub fn group(blocks: &[Block], per_block: Vec<Vec<Detection>>) -> Analysis {
    let mut by_key: IndexMap<String, Entity> = IndexMap::new();
    let mut entity_of = Vec::new();
    for (b, dets) in per_block.iter().enumerate() {
        let mut row = Vec::new();
        for d in dets {
            let surface = &blocks[b].text[d.start..d.end];
            let entry = by_key.entry(normalize(d.kind, surface));
            row.push(entry.index());
            let e = entry.or_insert_with_key(|norm| Entity {
                id: String::new(),
                kind: d.kind,
                norm: norm.clone(),
                surfaces: Vec::new(),
                occurrences: Vec::new(),
                score: 0.0,
                crosses: false,
            });
            if d.kind.personal() && !e.kind.personal() {
                e.kind = d.kind;
            }
            if !e.surfaces.iter().any(|s| s == surface) {
                e.surfaces.push(surface.to_string());
            }
            e.occurrences.push(Occ { block: b, start: d.start, end: d.end });
            e.score = e.score.max(d.score);
            e.crosses |= blocks[b].crosses(d.start, d.end);
        }
        entity_of.push(row);
    }
    let mut entities: Vec<Entity> = by_key.into_values().collect();
    let (mut personal, mut other) = (0, 0);
    for e in &mut entities {
        e.id = if e.kind.personal() {
            personal += 1;
            format!("pii:{personal}")
        } else {
            other += 1;
            format!("v:{other}")
        };
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
