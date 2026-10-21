//! A docx or PDF read into blocks and analyzed once: redacted views for the assistant, entity suggestions,
//! and marks compiled into template replacements.

use std::cmp::Reverse;
use std::path::Path;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value};

use super::marks::{At, Draft, Mark};
use super::redact::{self, Span, PLACEHOLDER};
use super::t2::ends_word;
use super::types::{Analysis, Block, Entity, Occ};
use super::{analyze, keys, REDACT, SUGGEST};
use crate::render::preview::{self, Place};
use crate::templates::docx::{self, ParagraphInfo};

const PDF_LINES: usize = 2000;

#[derive(Serialize, Clone, Debug)]
pub struct LineText {
    pub text: String,
    pub rect: [f64; 4],
}

#[derive(Serialize, Clone, Debug)]
pub struct PageText {
    pub page: usize,
    pub width: f64,
    pub height: f64,
    pub lines: Vec<LineText>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// What the assistant and the UI see of an entity: never its value, only its length.
#[derive(Serialize, Debug)]
pub struct EntityView {
    pub id: String,
    pub kind: &'static str,
    pub role: Option<String>,
    pub var: Option<String>,
    pub binding: Option<String>,
    pub label: Option<String>,
    pub length: usize,
    pub occurrences: usize,
    pub score: f32,
    #[serde(skip_serializing_if = "is_false")]
    pub crosses_runs: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<Place>,
}

enum Source {
    Docx(Vec<ParagraphInfo>),
    Pdf { pages: Vec<PageText>, places: Vec<Vec<Place>> },
}

pub struct Document {
    blocks: Vec<Block>,
    analysis: Analysis,
    source: Source,
}

fn round(v: f32) -> f64 {
    ((v as f64) * 10.0).round() / 10.0
}

fn overlaps(a: &Occ, b: &Occ) -> bool {
    a.block == b.block && a.start < b.end && b.start < a.end
}

impl Document {
    pub fn open(path: &Path, kind: &str) -> Result<Document, String> {
        match kind {
            "docx" => Document::docx(path),
            "pdf" => Document::pdf(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?),
            other => Err(format!("Cannot read a {other} file for personal data")),
        }
    }

    pub fn docx(path: &Path) -> Result<Document, String> {
        let paragraphs = docx::inspect(path)?;
        let blocks: Vec<Block> =
            paragraphs.iter().map(|p| Block { text: p.text.clone(), pieces: p.run_spans.clone() }).collect();
        Ok(Document { analysis: analyze(&blocks), blocks, source: Source::Docx(paragraphs) })
    }

    /// The text layer, one block per page with its lines joined by newlines. A scan has empty blocks.
    pub fn pdf(bytes: &[u8]) -> Result<Document, String> {
        let (mut blocks, mut pages) = (Vec::new(), Vec::new());
        for (i, (width, height)) in preview::page_sizes(bytes)?.into_iter().enumerate() {
            let lines = preview::layout_lines(bytes, i + 1, PDF_LINES)?;
            let mut block = Block::default();
            for line in &lines {
                if !block.text.is_empty() {
                    block.text.push('\n');
                }
                let start = block.text.len();
                block.text.push_str(&line.text);
                block.pieces.push((start, block.text.len()));
            }
            blocks.push(block);
            let lines = lines.into_iter().map(|l| LineText { text: l.text, rect: l.rect }).collect();
            pages.push(PageText { page: i + 1, width: round(width), height: round(height), lines });
        }
        let analysis = analyze(&blocks);
        let places = entity_places(bytes, &analysis.entities)?;
        Ok(Document { blocks, analysis, source: Source::Pdf { pages, places } })
    }

    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The read_body index of each block of a docx (inspect skips empty paragraphs).
    pub fn paragraph_index(&self, block: usize) -> Option<usize> {
        match &self.source {
            Source::Docx(paragraphs) => paragraphs.get(block).map(|p| p.index),
            Source::Pdf { .. } => None,
        }
    }

    fn surface(&self, occ: &Occ) -> &str {
        &self.blocks[occ.block].text[occ.start..occ.end]
    }

    /// The redacted spans of a block.
    fn spans(&self, b: usize) -> Vec<Span<'_>> {
        let a = &self.analysis;
        a.per_block[b]
            .iter()
            .zip(&a.entity_of[b])
            .filter(|(d, _)| d.kind.personal() && d.score >= REDACT)
            .map(|(d, &e)| (d.start, d.end, a.entities[e].id.as_str()))
            .collect()
    }

    fn redacted_entities(&self) -> impl Iterator<Item = (usize, &Entity)> {
        self.analysis.entities.iter().enumerate().filter(|(_, e)| e.kind.personal() && e.score >= REDACT)
    }

    pub fn entities(&self) -> Vec<EntityView> {
        let places = match &self.source {
            Source::Pdf { places, .. } => Some(places),
            Source::Docx(_) => None,
        };
        self.analysis
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.score >= SUGGEST)
            .map(|(i, e)| {
                let key = keys::profile_key(e.kind);
                let named = e.role.as_deref().zip(key);
                EntityView {
                    id: e.id.clone(),
                    kind: e.kind.name(),
                    role: e.role.clone(),
                    var: named.map(|(r, k)| keys::var(r, k)),
                    binding: named.map(|(r, k)| keys::binding(r, k)),
                    label: e.label.clone(),
                    length: e.surfaces.first().map_or(0, |s| s.chars().count()),
                    occurrences: e.occurrences.len(),
                    score: (e.score * 100.0).round() / 100.0,
                    crosses_runs: e.crosses,
                    places: places.map(|p| p[i].clone()).unwrap_or_default(),
                }
            })
            .collect()
    }

    /// One mark per suggested entity that has a var.
    pub fn suggested(&self) -> Vec<Mark> {
        self.entities()
            .into_iter()
            .filter_map(|v| {
                Some(Mark { at: At::Entity(v.id), var: v.var?, binding: v.binding, label: v.label, all: true })
            })
            .collect()
    }

    pub fn redacted_paragraphs(&self) -> Vec<ParagraphInfo> {
        let Source::Docx(paragraphs) = &self.source else { return Vec::new() };
        paragraphs
            .iter()
            .enumerate()
            .map(|(b, p)| {
                let spans = self.spans(b);
                let mut p = p.clone();
                if !spans.is_empty() {
                    p.runs = redact::redact_pieces(&p.text, &p.run_spans, &spans);
                    p.text = redact::redact_text(&p.text, &spans);
                }
                p
            })
            .collect()
    }

    /// The pages as printed, for when redaction is off.
    pub fn pages(&self) -> Vec<PageText> {
        match &self.source {
            Source::Pdf { pages, .. } => pages.clone(),
            Source::Docx(_) => Vec::new(),
        }
    }

    pub fn redacted_pages(&self) -> Vec<PageText> {
        let Source::Pdf { pages, .. } = &self.source else { return Vec::new() };
        pages
            .iter()
            .enumerate()
            .map(|(b, page)| {
                let block = &self.blocks[b];
                let texts = redact::redact_pieces(&block.text, &block.pieces, &self.spans(b));
                let mut page = page.clone();
                for (line, text) in page.lines.iter_mut().zip(texts) {
                    line.text = text;
                }
                page
            })
            .collect()
    }

    /// Where redacted values are printed on a 1-based page.
    pub fn masks(&self, page: usize) -> Vec<[f64; 4]> {
        let Source::Pdf { places, .. } = &self.source else { return Vec::new() };
        self.redacted_entities().flat_map(|(i, _)| places[i].iter().filter(|p| p.0 == page).map(|p| p.1)).collect()
    }

    /// The text with every redacted value replaced by its placeholder.
    pub fn scrub(&self, text: &str) -> String {
        let values: Vec<(String, String)> =
            self.redacted_entities().flat_map(|(_, e)| e.surfaces.iter().map(|s| (s.clone(), e.id.clone()))).collect();
        redact::scrub(text, &values)
    }

    /// Where `find` occurs as tokenize sees it: inside one run, without overlaps, in document order.
    fn run_occurrences(&self, find: &str) -> Vec<Occ> {
        let mut out = Vec::new();
        if find.is_empty() {
            return out;
        }
        for (b, block) in self.blocks.iter().enumerate() {
            for &(ps, pe) in &block.pieces {
                let piece = &block.text[ps..pe];
                let mut at = 0;
                while let Some(i) = piece[at..].find(find) {
                    let start = ps + at + i;
                    out.push(Occ { block: b, start, end: start + find.len() });
                    at += i + find.len();
                }
            }
        }
        out
    }

    /// The assistant's replacements with placeholders turned back into the text they stand for. A lone placeholder
    /// replaces every spelling of the entity, or its n-th occurrence; a placeholder inside a longer find is
    /// substituted. Replacements without placeholders pass unchanged.
    pub fn resolve_finds(&self, replacements: Vec<Value>) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        for r in replacements {
            let Some(find) = r.get("find").and_then(Value::as_str) else {
                out.push(r);
                continue;
            };
            let ids: Vec<String> = PLACEHOLDER.captures_iter(find).map(|c| c[1].to_string()).collect();
            if ids.is_empty() {
                out.push(r);
                continue;
            }
            let entities = ids
                .iter()
                .map(|id| {
                    self.analysis.entity(id).ok_or_else(|| format!("{find}: no such placeholder in this document"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let occurrence = r.get("occurrence").and_then(Value::as_u64).map(|n| n as usize);
            if ids.len() > 1 || find != redact::placeholder(&ids[0]) {
                let mut text = find.to_string();
                for (id, e) in ids.iter().zip(&entities) {
                    text = text.replace(&redact::placeholder(id), &e.surfaces[0]);
                }
                out.push(with_find(&r, &text, occurrence));
                continue;
            }
            let e = entities[0];
            match occurrence {
                Some(n) => {
                    let occ = n
                        .checked_sub(1)
                        .and_then(|i| e.occurrences.get(i))
                        .ok_or_else(|| format!("{find}: occurs {}x, no occurrence {n}", e.occurrences.len()))?;
                    let surface = self.surface(occ);
                    let nth = self
                        .run_occurrences(surface)
                        .iter()
                        .position(|o| o == occ)
                        .ok_or_else(|| format!("{find}: occurrence {n} spans several runs (formatting change)"))?;
                    out.push(with_find(&r, surface, Some(nth + 1)));
                }
                None => {
                    let mut surfaces = e.surfaces.clone();
                    surfaces.sort_by_key(|s| Reverse(s.len()));
                    out.extend(surfaces.iter().map(|s| with_find(&r, s, None)));
                }
            }
        }
        Ok(out)
    }

    fn utf16_to_byte(text: &str, unit: usize) -> usize {
        let mut seen = 0;
        for (i, c) in text.char_indices() {
            if seen >= unit {
                return i;
            }
            seen += c.len_utf16();
        }
        text.len()
    }

    /// The occurrences a mark covers, in byte offsets.
    pub fn expand(&self, mark: &Mark) -> Result<Vec<Occ>, String> {
        match &mark.at {
            At::Entity(id) => self
                .analysis
                .entity(id)
                .map(|e| e.occurrences.clone())
                .ok_or_else(|| format!("{}: unknown {id}", mark.var)),
            At::Span { block, start, end } => {
                let text = &self.blocks.get(*block).ok_or_else(|| format!("{}: no block {block}", mark.var))?.text;
                let (s, e) = snap(text, Self::utf16_to_byte(text, *start), Self::utf16_to_byte(text, *end));
                if s >= e {
                    return Err(format!("{}: the selection is empty", mark.var));
                }
                let occ = Occ { block: *block, start: s, end: e };
                if !mark.all {
                    return Ok(vec![occ]);
                }
                if let Some(entity) = self.analysis.entities.iter().find(|en| en.occurrences.contains(&occ)) {
                    return Ok(entity.occurrences.clone());
                }
                Ok(self.word_occurrences(&text[s..e]))
            }
            At::Rect { .. } => Ok(Vec::new()),
        }
    }

    /// Occurrences of `needle` that do not sit inside a longer word.
    fn word_occurrences(&self, needle: &str) -> Vec<Occ> {
        let mut out = Vec::new();
        for (b, block) in self.blocks.iter().enumerate() {
            for (i, _) in block.text.match_indices(needle) {
                let end = i + needle.len();
                if ends_word(&block.text, i) && ends_word(&block.text, end) {
                    out.push(Occ { block: b, start: i, end });
                }
            }
        }
        out
    }

    /// Marks to a draft: tokenize replacements for a docx, overlay boxes for a PDF.
    pub fn compile(&self, marks: &[Mark]) -> Result<Draft, String> {
        match &self.source {
            Source::Docx(paragraphs) => self.compile_docx(marks, paragraphs),
            Source::Pdf { .. } => compile_boxes(marks),
        }
    }

    /// Replacements that hit exactly the marked occurrences under tokenize's rules: longer finds first, then for
    /// each find either all of it or its occurrences from last to first, so earlier counts never shift.
    fn compile_docx(&self, marks: &[Mark], paragraphs: &[ParagraphInfo]) -> Result<Draft, String> {
        let mut draft = Draft::default();
        let mut targets: Vec<(Occ, &Mark)> = Vec::new();
        for mark in marks {
            draft.add_meta(&mark.var, mark);
            for occ in self.expand(mark)? {
                if targets.iter().any(|(t, _)| overlaps(t, &occ)) {
                    continue;
                }
                let block = &self.blocks[occ.block];
                if !block.pieces.iter().any(|&(ps, pe)| ps <= occ.start && occ.end <= pe) {
                    return Err(format!(
                        "{}: the marked text in paragraph {} spans a formatting change; mark it within one run",
                        mark.var, paragraphs[occ.block].index
                    ));
                }
                targets.push((occ, mark));
            }
        }
        let mut groups: IndexMap<String, Vec<(Occ, &Mark)>> = IndexMap::new();
        for (occ, mark) in targets {
            groups.entry(self.surface(&occ).to_string()).or_default().push((occ, mark));
        }
        groups.sort_by(|a, _, b, _| b.len().cmp(&a.len()));
        let mut consumed: Vec<Occ> = Vec::new();
        for (find, group) in &groups {
            let live: Vec<Occ> =
                self.run_occurrences(find).into_iter().filter(|o| !consumed.iter().any(|c| overlaps(c, o))).collect();
            let mut hits = group
                .iter()
                .map(|(occ, mark)| {
                    live.iter()
                        .position(|o| o == occ)
                        .map(|i| (i, *mark))
                        .ok_or_else(|| format!("{}: the marked text overlaps another mark", mark.var))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if hits.len() == live.len() && hits.iter().all(|(_, m)| m.var == hits[0].1.var) {
                draft.replacements.push(json!({"find": find, "var": hits[0].1.var}));
            } else {
                hits.sort_by_key(|(i, _)| Reverse(*i));
                for (i, mark) in hits {
                    draft.replacements.push(json!({"find": find, "var": mark.var, "occurrence": i + 1}));
                }
            }
            consumed.extend(group.iter().map(|(o, _)| o.clone()));
        }
        Ok(draft)
    }
}

/// Boxes drawn on a PDF become overlay fields. Values printed on a flat PDF stay printed, so only drawn boxes
/// (on blank areas) are supported.
fn compile_boxes(marks: &[Mark]) -> Result<Draft, String> {
    let mut draft = Draft::default();
    let mut used: IndexMap<String, usize> = IndexMap::new();
    for mark in marks {
        let At::Rect { page, rect } = &mark.at else {
            return Err(format!("{}: on a PDF, draw a box where the value goes", mark.var));
        };
        let n = used.entry(mark.var.clone()).or_default();
        *n += 1;
        let name = if *n == 1 { mark.var.clone() } else { format!("{}_{n}", mark.var) };
        draft.add_meta(&name, mark);
        draft.boxes.push(json!({"name": name, "page": page, "rect": rect, "type": "text"}));
    }
    Ok(draft)
}

fn with_find(r: &Value, find: &str, occurrence: Option<usize>) -> Value {
    let mut r = r.clone();
    r["find"] = json!(find);
    match occurrence {
        Some(n) => r["occurrence"] = json!(n),
        None => {
            if let Some(map) = r.as_object_mut() {
                map.remove("occurrence");
            }
        }
    }
    r
}

/// A quick selection made clean: widened to whole words, trimmed of surrounding spaces and punctuation.
pub fn snap(text: &str, start: usize, end: usize) -> (usize, usize) {
    let (mut s, mut e) = (start.min(text.len()), end.min(text.len()));
    while !text.is_char_boundary(s) {
        s -= 1;
    }
    while !text.is_char_boundary(e) {
        e += 1;
    }
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    while s > 0 && word(text[..s].chars().next_back()) && word(text[s..].chars().next()) {
        s -= text[..s].chars().next_back().map_or(1, char::len_utf8);
    }
    while e < text.len() && word(text[e..].chars().next()) && word(text[..e].chars().next_back()) {
        e += text[e..].chars().next().map_or(1, char::len_utf8);
    }
    let junk = |c: char| c.is_whitespace() || ",;:!?".contains(c);
    let inner = text[s..e.max(s)].trim_matches(junk);
    let lead = text[s..e.max(s)].len() - text[s..e.max(s)].trim_start_matches(junk).len();
    (s + lead, s + lead + inner.len())
}

/// Where each entity's spellings are printed, per page. A spelling broken over lines is searched line by line.
fn entity_places(pdf: &[u8], entities: &[Entity]) -> Result<Vec<Vec<Place>>, String> {
    let mut needles: IndexMap<String, String> = IndexMap::new();
    for (i, e) in entities.iter().enumerate() {
        for (k, part) in e.surfaces.iter().flat_map(|s| s.split('\n')).map(str::trim).enumerate() {
            if part.chars().count() >= 2 {
                needles.insert(format!("{i}/{k}"), part.to_string());
            }
        }
    }
    let found = preview::find_text(pdf, &needles)?;
    let mut places = vec![Vec::new(); entities.len()];
    for (key, list) in found {
        let i: usize = key.split('/').next().and_then(|i| i.parse().ok()).unwrap_or(0);
        for place in list {
            if !places[i].contains(&place) {
                places[i].push(place);
            }
        }
    }
    Ok(places)
}
