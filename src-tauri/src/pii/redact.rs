//! Placeholders in place of values: in block text, in runs or lines, in error messages and on page images.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

use super::t1;
use super::types::Kind;
use crate::render::raster;

/// `⟦egn:1 ##########⟧`: the entity id, then the value's shape. The shape is optional when the assistant writes one.
pub static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"⟦([a-z_]+:\d+)(?: [^⟧]*)?⟧").unwrap());
const GUARDED: &[Kind] = &[Kind::Egn, Kind::Iban, Kind::VatId];
const MASK_PAD: f64 = 1.5;

/// The value's shape: letters as X or x, digits as #, everything else kept ("Иван Петров" -> "Xxxx Xxxxxx").
pub fn shape(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            c if c.is_ascii_digit() => '#',
            c if c.is_uppercase() => 'X',
            c if c.is_alphabetic() => 'x',
            '\n' => ' ',
            c => c,
        })
        .collect()
}

pub fn placeholder(id: &str, value: &str) -> String {
    format!("⟦{id} {}⟧", shape(value))
}

/// A redacted span of a block: byte range and entity id. Spans are sorted and do not overlap.
pub type Span<'a> = (usize, usize, &'a str);

pub fn redact_text(text: &str, spans: &[Span]) -> String {
    let (mut out, mut at) = (String::new(), 0);
    for &(s, e, id) in spans {
        out.push_str(&text[at..s]);
        out.push_str(&placeholder(id, &text[s..e]));
        at = e;
    }
    out.push_str(&text[at..]);
    out
}

/// Each piece (run or line) of a block with the spans redacted. A span across pieces leaves its placeholder in
/// the first piece and removes the rest.
pub fn redact_pieces(text: &str, pieces: &[(usize, usize)], spans: &[Span]) -> Vec<String> {
    pieces
        .iter()
        .map(|&(ps, pe)| {
            let (mut out, mut at) = (String::new(), ps);
            for &(s, e, id) in spans.iter().filter(|(s, e, _)| *s < pe && ps < *e) {
                if s >= ps {
                    out.push_str(&text[at..s]);
                    out.push_str(&placeholder(id, &text[s..e]));
                }
                at = e.min(pe);
            }
            out.push_str(&text[at..pe]);
            out
        })
        .collect()
}

/// Replaces every known value in free text, longest first, so a message never echoes one.
pub fn scrub(text: &str, values: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = values.iter().filter(|(v, _)| !v.is_empty()).collect();
    sorted.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
    sorted.iter().fold(text.to_string(), |acc, (value, id)| acc.replace(value.as_str(), &placeholder(id, value)))
}

/// Masks identifiers that validate themselves (EGN, IBAN, VAT) in any text: a safety net for output that did not
/// go through a Document.
pub fn guard(text: &str) -> Cow<'_, str> {
    let mut spans: Vec<(usize, usize)> = t1::scan(text)
        .into_iter()
        .filter(|d| GUARDED.contains(&d.kind) && d.score >= 0.8)
        .map(|d| (d.start, d.end))
        .collect();
    if spans.is_empty() {
        return Cow::Borrowed(text);
    }
    spans.sort();
    spans.dedup();
    let (mut out, mut at) = (String::new(), 0);
    for (s, e) in spans {
        if s < at {
            continue;
        }
        out.push_str(&text[at..s]);
        out.push_str(&format!("⟦{}⟧", shape(&text[s..e])));
        at = e;
    }
    out.push_str(&text[at..]);
    Cow::Owned(out)
}

/// Paints the rects ([x0, y0, x1, y1] in points, origin bottom-left) opaque on a page image rendered at `dpi`.
pub fn mask_png(png: &[u8], page_height: f32, rects: &[[f64; 4]], dpi: f32) -> Result<Vec<u8>, String> {
    if rects.is_empty() {
        return Ok(png.to_vec());
    }
    let mut image = tiny_skia::Pixmap::decode_png(png).map_err(|e| e.to_string())?;
    let scale = dpi as f64 / 72.0;
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(40, 40, 40, 255);
    for [x0, y0, x1, y1] in rects {
        let left = (x0 - MASK_PAD) * scale;
        let top = (page_height as f64 - y1 - MASK_PAD) * scale;
        let width = (x1 - x0 + 2.0 * MASK_PAD) * scale;
        let height = (y1 - y0 + 2.0 * MASK_PAD) * scale;
        if let Some(rect) = tiny_skia::Rect::from_xywh(left as f32, top as f32, width as f32, height as f32) {
            image.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
        }
    }
    raster::png(&image)
}
