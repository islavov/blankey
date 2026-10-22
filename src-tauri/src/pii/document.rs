//! A docx or PDF read into blocks and analyzed once: redacted views for the assistant, entity suggestions,
//! and marks compiled into template replacements.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value};

use super::redact::{self, Span, PLACEHOLDER};
use super::types::{Analysis, Block, Entity, Occ};
use super::{analyze, Corrections, REDACT};
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

/// What the assistant sees of a hidden value: never the value, only its length and where it is. What the value is
/// and whose it is the assistant reads from the text around it.
#[derive(Serialize, Debug)]
pub struct EntityView {
    pub id: String,
    pub length: usize,
    pub occurrences: usize,
    #[serde(skip_serializing_if = "is_false")]
    pub crosses_runs: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<Place>,
}

enum Source {
    Docx(Vec<ParagraphInfo>, PathBuf),
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

impl Document {
    pub fn open(path: &Path, kind: &str, corrections: &Corrections) -> Result<Document, String> {
        match kind {
            "docx" => Document::docx(path, corrections),
            "pdf" => Document::pdf(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?, corrections),
            other => Err(format!("Cannot read a {other} file for personal data")),
        }
    }

    pub fn docx(path: &Path, corrections: &Corrections) -> Result<Document, String> {
        let paragraphs = docx::inspect(path)?;
        let blocks: Vec<Block> =
            paragraphs.iter().map(|p| Block { text: p.text.clone(), pieces: p.run_spans.clone() }).collect();
        let analysis = analyze(&blocks, corrections);
        Ok(Document { analysis, blocks, source: Source::Docx(paragraphs, path.to_path_buf()) })
    }

    /// The text layer, one block per page with its lines joined by newlines. A scan has empty blocks.
    pub fn pdf(bytes: &[u8], corrections: &Corrections) -> Result<Document, String> {
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
        let analysis = analyze(&blocks, corrections);
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
            Source::Docx(paragraphs, _) => paragraphs.get(block).map(|p| p.index),
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

    /// The hidden values, in order of first appearance.
    pub fn entities(&self) -> Vec<EntityView> {
        let places = match &self.source {
            Source::Pdf { places, .. } => Some(places),
            Source::Docx(..) => None,
        };
        self.redacted_entities()
            .map(|(i, e)| EntityView {
                id: e.id.clone(),
                length: e.surfaces.first().map_or(0, |s| s.chars().count()),
                occurrences: e.occurrences.len(),
                crosses_runs: e.crosses,
                places: places.map(|p| p[i].clone()).unwrap_or_default(),
            })
            .collect()
    }

    pub fn redacted_paragraphs(&self) -> Vec<ParagraphInfo> {
        let Source::Docx(paragraphs, _) = &self.source else { return Vec::new() };
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
            Source::Docx(..) => Vec::new(),
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

    /// The docx preview with every hidden value highlighted (`data-e` its entity, class `user` for what the user
    /// hid). Every run is a `<span data-b="block" data-o="utf-16 offset">`, so a selection maps back to the text.
    pub fn html(&self, shown: &[String]) -> Result<String, String> {
        let Source::Docx(paragraphs, path) = &self.source else { return Err("Not a .docx".into()) };
        // (start, end, item id, class) per block, then numbered per item in document order as data-n, so the
        // review can step through an item's places even when one place spans several runs
        let mut counts: HashMap<String, usize> = HashMap::new();
        let marks: Vec<Vec<(usize, usize, String)>> = (0..self.blocks.len())
            .map(|b| {
                let mut row: Vec<(usize, usize, String, &str)> = self
                    .spans(b)
                    .into_iter()
                    .map(|(s, e, id)| {
                        (s, e, id.to_string(), if id.starts_with("marked:") { "pii user" } else { "pii" })
                    })
                    .collect();
                for (i, value) in shown.iter().enumerate() {
                    for (s, e) in self.occurrences_of(b, value) {
                        if !row.iter().any(|(rs, re, _, _)| *rs < e && s < *re) {
                            row.push((s, e, format!("shown:{i}"), "pii shown"));
                        }
                    }
                }
                row.sort_by_key(|(s, _, _, _)| *s);
                row.into_iter()
                    .map(|(s, e, id, class)| {
                        let n = counts.entry(id.clone()).or_default();
                        *n += 1;
                        let attr = match id.strip_prefix("shown:") {
                            Some(i) => format!(r#"data-v="{i}""#),
                            None => format!(r#"data-e="{id}""#),
                        };
                        (s, e, format!(r#"class="{class}" {attr} data-n="{}""#, *n - 1))
                    })
                    .collect()
            })
            .collect();
        let block_of: HashMap<usize, usize> = paragraphs.iter().enumerate().map(|(b, p)| (p.index, b)).collect();
        let hook = |paragraph: usize, run: usize, raw: &str| -> Option<String> {
            let &b = block_of.get(&paragraph)?;
            let &(ps, pe) = self.blocks[b].pieces.get(run)?;
            let text = &self.blocks[b].text;
            if &text[ps..pe] != raw {
                return None;
            }
            let mut out = format!(r#"<span data-b="{b}" data-o="{}">"#, text[..ps].encode_utf16().count());
            let mut at = ps;
            for (s, e, attrs) in marks[b].iter().filter(|(s, e, _)| *s < pe && ps < *e) {
                let (s, e) = ((*s).max(ps), (*e).min(pe));
                out.push_str(&escape(&text[at..s]));
                out.push_str(&format!("<mark {attrs}>{}</mark>", escape(&text[s..e])));
                at = e;
            }
            out.push_str(&escape(&text[at..pe]));
            out.push_str("</span>");
            Some(out)
        };
        docx::to_html_with(path, &hook)
    }

    /// Where a value occurs in a block as whole words, in any letter case and with or without quotes.
    fn occurrences_of(&self, b: usize, value: &str) -> Vec<(usize, usize)> {
        super::corrections::find_value(&self.blocks[b].text, value)
    }

    /// How often a value occurs in the document.
    pub fn count(&self, value: &str) -> usize {
        (0..self.blocks.len()).map(|b| self.occurrences_of(b, value).len()).sum()
    }

    /// What gets hidden, for the user's own review: id, kind, the value as first written and how often it occurs.
    /// `first`: where the value first occurs, [block, byte offset], to keep the review list in document order.
    pub fn hidden(&self) -> Vec<Value> {
        self.redacted_entities()
            .map(|(_, e)| {
                let first = e.occurrences.iter().map(|o| (o.block, o.start)).min();
                json!({"id": e.id, "value": e.surfaces[0], "occurrences": e.occurrences.len(), "first": first})
            })
            .collect()
    }

    /// Where a value first occurs as a whole word, [block, byte offset].
    pub fn first(&self, value: &str) -> Option<(usize, usize)> {
        (0..self.blocks.len()).find_map(|b| self.occurrences_of(b, value).first().map(|&(s, _)| (b, s)))
    }

    /// Where each hidden entity is printed in a PDF, by id.
    pub fn places(&self) -> IndexMap<String, Vec<Place>> {
        let Source::Pdf { places, .. } = &self.source else { return IndexMap::new() };
        self.redacted_entities().map(|(i, e)| (e.id.clone(), places[i].clone())).collect()
    }

    /// Every spelling of an entity, to stop hiding it.
    pub fn surfaces(&self, id: &str) -> Vec<String> {
        self.analysis.entity(id).map(|e| e.surfaces.clone()).unwrap_or_default()
    }

    /// The text of a selection (UTF-16 offsets in one block), widened to whole words and trimmed.
    pub fn selection(&self, block: usize, start: usize, end: usize) -> Option<String> {
        let text = &self.blocks.get(block)?.text;
        let (s, e) = snap(text, utf16_to_byte(text, start), utf16_to_byte(text, end));
        (s < e).then(|| text[s..e].to_string())
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
            let alone = PLACEHOLDER.find(find).is_some_and(|m| m.start() == 0 && m.end() == find.len());
            if !alone {
                let text = PLACEHOLDER.replace_all(find, |c: &regex::Captures| {
                    self.analysis.entity(&c[1]).map(|e| e.surfaces[0].clone()).unwrap_or_default()
                });
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
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
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
